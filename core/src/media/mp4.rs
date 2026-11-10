//! MP4: reading fragmented MP4 (HLS and DASH pieces) into samples, and writing tracks of samples into one MP4 file
//! with its index (`moov`) before the media, so it plays before it's read to the end.

use std::io::Write;
use std::path::Path;

use super::bits::Boxes;
use super::{JoinError, Kind, Sample, Spool, Track};

/// The boxes inside `data`: (type, body).
pub fn boxes(data: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 8 <= data.len() {
        let mut size = u32::from_be_bytes(data[i..i + 4].try_into().unwrap()) as u64;
        let kind: [u8; 4] = data[i + 4..i + 8].try_into().unwrap();
        let mut header = 8;
        if size == 1 && i + 16 <= data.len() {
            size = u64::from_be_bytes(data[i + 8..i + 16].try_into().unwrap());
            header = 16;
        } else if size == 0 {
            size = (data.len() - i) as u64;
        }
        if size < header as u64 || i as u64 + size > data.len() as u64 {
            break;
        }
        out.push((kind, &data[i + header..i + size as usize]));
        i += size as usize;
    }
    out
}

fn child<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(data).into_iter().find(|(k, _)| k == kind).map(|(_, b)| b)
}

fn be32(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn be64(d: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(d.get(at..at + 8)?.try_into().ok()?))
}

/// A track an init section describes, and the defaults its fragments lean on.
#[derive(Clone, Debug)]
pub struct FragmentedTrack {
    pub id: u32,
    pub track: Track,
    default_duration: u32,
    default_size: u32,
    default_flags: u32,
}

/// Reads an init section (`ftyp` + `moov`) into its tracks, with no samples yet.
pub fn read_init(data: &[u8]) -> Result<Vec<FragmentedTrack>, JoinError> {
    let bad = || JoinError::Damaged("The stream's init section can't be read".into());
    let moov = child(data, b"moov").ok_or_else(bad)?;
    let movie_timescale = child(moov, b"mvhd").and_then(|m| if m[0] == 1 { be32(m, 20) } else { be32(m, 12) }).unwrap_or(1000).max(1);
    let mut tracks = Vec::new();
    let trex: Vec<&[u8]> = child(moov, b"mvex").map(|m| boxes(m).into_iter().filter(|(k, _)| k == b"trex").map(|(_, b)| b).collect()).unwrap_or_default();
    for (kind, trak) in boxes(moov) {
        if &kind != b"trak" {
            continue;
        }
        let tkhd = child(trak, b"tkhd").ok_or_else(bad)?;
        let id = if tkhd[0] == 1 { be32(tkhd, 20) } else { be32(tkhd, 12) }.ok_or_else(bad)?;
        let wh_at = tkhd.len().saturating_sub(8);
        let (width, height) = (be32(tkhd, wh_at).unwrap_or(0) >> 16, be32(tkhd, wh_at + 4).unwrap_or(0) >> 16);
        let mdia = child(trak, b"mdia").ok_or_else(bad)?;
        let mdhd = child(mdia, b"mdhd").ok_or_else(bad)?;
        let (timescale, lang_at) = if mdhd[0] == 1 { (be32(mdhd, 20), 32) } else { (be32(mdhd, 12), 20) };
        let language = mdhd.get(lang_at..lang_at + 2).map(|l| {
            let code = u16::from_be_bytes([l[0], l[1]]);
            (0..3).rev().map(|i| (((code >> (5 * i)) & 0x1f) as u8 + 0x60) as char).collect::<String>()
        });
        let handler = child(mdia, b"hdlr").and_then(|h| h.get(8..12)).ok_or_else(bad)?;
        let kind = match handler {
            b"vide" => Kind::Video,
            b"soun" => Kind::Audio,
            _ => continue, // subtitles, metadata
        };
        let stsd = child(mdia, b"minf").and_then(|m| child(m, b"stbl")).and_then(|s| child(s, b"stsd")).ok_or_else(bad)?;
        // stsd: version and flags, a count, then the entries: the first is the one fragments use.
        let entries = stsd.get(8..).ok_or_else(bad)?;
        let first = be32(entries, 0).ok_or_else(bad)? as usize;
        let entry = entries.get(..first).ok_or_else(bad)?.to_vec();
        if entry.get(4..8).is_some_and(|k| k == b"encv" || k == b"enca") {
            return Err(JoinError::Unsupported("protected (DRM) video".into()));
        }
        let defaults = trex.iter().find(|t| be32(t, 4) == Some(id));
        let edit = child(trak, b"edts").and_then(|e| child(e, b"elst")).and_then(|elst| read_elst(elst, movie_timescale));
        tracks.push(FragmentedTrack {
            id,
            track: Track {
                kind,
                timescale: timescale.ok_or_else(bad)?,
                entry,
                width,
                height,
                language: language.filter(|l| l.chars().all(|c| c.is_ascii_lowercase())).unwrap_or_else(|| "und".into()),
                samples: Vec::new(),
                edit,
            },
            default_duration: defaults.and_then(|t| be32(t, 12)).unwrap_or(0),
            default_size: defaults.and_then(|t| be32(t, 16)).unwrap_or(0),
            default_flags: defaults.and_then(|t| be32(t, 20)).unwrap_or(0),
        });
    }
    if tracks.is_empty() {
        return Err(bad());
    }
    Ok(tracks)
}

/// An edit list as (seconds of empty edits before it, the media time the first real edit starts at).
fn read_elst(elst: &[u8], movie_timescale: u32) -> Option<(f64, i64)> {
    let version = elst[0];
    let count = be32(elst, 4)?;
    let mut empty = 0u64;
    let mut at = 8;
    for _ in 0..count {
        let (duration, media_time) = if version == 1 {
            (be64(elst, at)?, be64(elst, at + 8)? as i64)
        } else {
            (be32(elst, at)? as u64, be32(elst, at + 4)? as i32 as i64)
        };
        at += if version == 1 { 20 } else { 12 };
        if media_time == -1 {
            empty += duration;
        } else {
            return Some((empty as f64 / movie_timescale as f64, media_time));
        }
    }
    None
}

/// Reads one piece's fragments (`moof` + `mdat`, any number) into its tracks' samples.
pub fn read_fragments(data: &[u8], tracks: &mut [FragmentedTrack], spool: &mut Spool) -> Result<(), JoinError> {
    let bad = || JoinError::Damaged("A piece of the stream can't be read".into());
    let mut i = 0;
    while i + 8 <= data.len() {
        let size = be32(data, i).ok_or_else(bad)? as usize;
        let kind = &data[i + 4..i + 8];
        let size = if size == 1 { be64(data, i + 8).ok_or_else(bad)? as usize } else { size };
        if size < 8 || i + size > data.len() {
            break;
        }
        if kind == b"moof" {
            let moof_start = i;
            for (k, traf) in boxes(&data[i + 8..i + size]) {
                if &k != b"traf" {
                    continue;
                }
                read_traf(traf, moof_start, data, tracks, spool)?;
            }
        }
        i += size;
    }
    Ok(())
}

fn read_traf(traf: &[u8], moof_start: usize, data: &[u8], tracks: &mut [FragmentedTrack], spool: &mut Spool) -> Result<(), JoinError> {
    let bad = || JoinError::Damaged("A piece of the stream can't be read".into());
    let tfhd = child(traf, b"tfhd").ok_or_else(bad)?;
    let flags = be32(tfhd, 0).ok_or_else(bad)? & 0x00ff_ffff;
    let id = be32(tfhd, 4).ok_or_else(bad)?;
    let Some(t) = tracks.iter_mut().find(|t| t.id == id) else { return Ok(()) };
    let mut at = 8;
    let mut base = moof_start as u64;
    if flags & 0x01 != 0 {
        base = be64(tfhd, at).ok_or_else(bad)?;
        at += 8;
    }
    if flags & 0x02 != 0 {
        at += 4;
    }
    let mut default_duration = t.default_duration;
    let mut default_size = t.default_size;
    let mut default_flags = t.default_flags;
    if flags & 0x08 != 0 {
        default_duration = be32(tfhd, at).ok_or_else(bad)?;
        at += 4;
    }
    if flags & 0x10 != 0 {
        default_size = be32(tfhd, at).ok_or_else(bad)?;
        at += 4;
    }
    if flags & 0x20 != 0 {
        default_flags = be32(tfhd, at).ok_or_else(bad)?;
    }
    let mut dts = match child(traf, b"tfdt") {
        Some(tfdt) if tfdt[0] == 1 => be64(tfdt, 4).ok_or_else(bad)? as i64,
        Some(tfdt) => be32(tfdt, 4).ok_or_else(bad)? as i64,
        None => t.track.samples.last().map(|s| s.dts + s.duration as i64).unwrap_or(0),
    };
    for (k, trun) in boxes(traf) {
        if &k != b"trun" {
            continue;
        }
        let version = trun[0];
        let tf = be32(trun, 0).ok_or_else(bad)? & 0x00ff_ffff;
        let count = be32(trun, 4).ok_or_else(bad)?;
        let mut at = 8;
        let mut offset = base;
        if tf & 0x01 != 0 {
            offset = (base as i64 + be32(trun, at).ok_or_else(bad)? as i32 as i64) as u64;
            at += 4;
        }
        let mut first_flags = None;
        if tf & 0x04 != 0 {
            first_flags = Some(be32(trun, at).ok_or_else(bad)?);
            at += 4;
        }
        for n in 0..count {
            let mut field = |on: bool| -> Result<Option<u32>, JoinError> {
                if !on {
                    return Ok(None);
                }
                let v = be32(trun, at).ok_or_else(bad)?;
                at += 4;
                Ok(Some(v))
            };
            let duration = field(tf & 0x100 != 0)?.unwrap_or(default_duration);
            let size = field(tf & 0x200 != 0)?.unwrap_or(default_size);
            let flags = field(tf & 0x400 != 0)?.or(if n == 0 { first_flags } else { None }).unwrap_or(default_flags);
            let cts = field(tf & 0x800 != 0)?.map(|c| if version == 0 { c as i64 } else { c as i32 as i64 }).unwrap_or(0);
            let bytes = data.get(offset as usize..offset as usize + size as usize).ok_or_else(bad)?;
            let at_spool = spool.put(bytes)?;
            t.track.samples.push(Sample {
                dts,
                cts: cts as i32,
                duration,
                // sample_is_non_sync_sample; sound is always sync.
                sync: t.track.kind == Kind::Audio || flags & 0x0001_0000 == 0,
                offset: at_spool,
                size,
            });
            dts += duration as i64;
            offset += size as u64;
        }
    }
    Ok(())
}

/// Runs of equal values: (count, value).
fn runs<T: PartialEq + Copy>(values: impl Iterator<Item = T>) -> Vec<(u32, T)> {
    let mut out: Vec<(u32, T)> = Vec::new();
    for v in values {
        match out.last_mut() {
            Some((n, last)) if *last == v => *n += 1,
            _ => out.push((1, v)),
        }
    }
    out
}

const MOVIE_TIMESCALE: u64 = 1000;

/// The first presentation time in the track's media time.
fn first_pts(t: &Track) -> i64 {
    t.samples.iter().take(64).map(|s| s.dts + s.cts as i64).min().unwrap_or(0)
}

/// Where a track starts on the timeline, in seconds: its edit list's empty time, or its first presentation time.
pub(crate) fn start_of(t: &Track) -> f64 {
    match t.edit {
        Some((empty, _)) => empty,
        None => first_pts(t) as f64 / t.timescale as f64,
    }
}

/// The media time presentation starts at, counted from the first sample's decode time: the first picture (B-frames
/// decode earlier), or where the source's edit list starts (after a sound encoder's lead-in samples).
fn media_start(t: &Track) -> i64 {
    let from = t.edit.map(|(_, m)| m).unwrap_or_else(|| first_pts(t));
    (from - t.samples[0].dts).max(0)
}

/// Writes `tracks` into one MP4 file at `path`, their bytes read from `spool`: `ftyp`, `moov`, then `mdat` with the
/// samples interleaved in time.
pub fn write(path: &Path, tracks: &[Track], spool: &mut Spool) -> Result<(), JoinError> {
    let tracks: Vec<&Track> = tracks.iter().filter(|t| !t.samples.is_empty()).collect();
    if tracks.is_empty() {
        return Err(JoinError::Damaged("The stream has nothing to save".into()));
    }
    let origin = tracks.iter().map(|t| start_of(t)).fold(f64::INFINITY, f64::min);
    // The order samples go into mdat: by decode time across tracks.
    let mut order: Vec<(f64, usize, usize)> = Vec::new();
    for (ti, t) in tracks.iter().enumerate() {
        for (si, s) in t.samples.iter().enumerate() {
            order.push((s.dts as f64 / t.timescale as f64, ti, si));
        }
    }
    order.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mdat_len: u64 = order.iter().map(|&(_, ti, si)| tracks[ti].samples[si].size as u64).sum();
    let large = mdat_len + 16 > u32::MAX as u64;
    let ftyp = {
        let mut b = Boxes::default();
        b.begin(b"ftyp").bytes(b"isom").u32(0x200).bytes(b"isomiso2avc1mp41").end();
        b.out
    };
    let mdat_header = if large { 16 } else { 8 };
    // The moov's size doesn't depend on the offsets (64-bit ones throughout): build it once to measure, then for real.
    let mut offsets: Vec<Vec<u64>> = tracks.iter().map(|t| vec![0; t.samples.len()]).collect();
    let probe = moov(&tracks, &offsets, origin);
    let mut at = ftyp.len() as u64 + probe.len() as u64 + mdat_header;
    for &(_, ti, si) in &order {
        offsets[ti][si] = at;
        at += tracks[ti].samples[si].size as u64;
    }
    let moov = moov(&tracks, &offsets, origin);
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    out.write_all(&ftyp)?;
    out.write_all(&moov)?;
    if large {
        out.write_all(&1u32.to_be_bytes())?;
        out.write_all(b"mdat")?;
        out.write_all(&(mdat_len + 16).to_be_bytes())?;
    } else {
        out.write_all(&((mdat_len + 8) as u32).to_be_bytes())?;
        out.write_all(b"mdat")?;
    }
    let mut buf = Vec::new();
    for &(_, ti, si) in &order {
        let s = tracks[ti].samples[si];
        spool.read(s.offset, s.size, &mut buf)?;
        out.write_all(&buf)?;
    }
    out.flush()?;
    out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    Ok(())
}

fn moov(tracks: &[&Track], offsets: &[Vec<u64>], origin: f64) -> Vec<u8> {
    let mut b = Boxes::default();
    let movie_duration = |t: &Track| -> u64 {
        let media: u64 = t.samples.iter().map(|s| s.duration as u64).sum::<u64>().saturating_sub(media_start(t) as u64);
        let delay = ((start_of(t) - origin) * MOVIE_TIMESCALE as f64).round() as u64;
        delay + (media * MOVIE_TIMESCALE).div_ceil(t.timescale as u64)
    };
    let total = tracks.iter().map(|t| movie_duration(t)).max().unwrap_or(0);
    b.begin(b"moov");
    b.begin_full(b"mvhd", 1, 0).u64(0).u64(0).u32(MOVIE_TIMESCALE as u32).u64(total);
    b.u32(0x0001_0000).u16(0x0100).zeros(10);
    matrix(&mut b);
    b.zeros(24).u32(tracks.len() as u32 + 1).end();
    for (i, t) in tracks.iter().enumerate() {
        trak(&mut b, t, i as u32 + 1, &offsets[i], origin, movie_duration(t));
    }
    b.end();
    b.out
}

fn matrix(b: &mut Boxes) {
    for v in [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        b.u32(v);
    }
}

fn trak(b: &mut Boxes, t: &Track, id: u32, offsets: &[u64], origin: f64, duration: u64) {
    let media_duration: u64 = t.samples.iter().map(|s| s.duration as u64).sum();
    let video = t.kind == Kind::Video;
    let text = t.kind == Kind::Text;
    b.begin(b"trak");
    b.begin_full(b"tkhd", 1, 3).u64(0).u64(0).u32(id).u32(0).u64(duration).zeros(8);
    // Layer, alternate group (subtitle tracks are alternatives to each other), volume.
    b.u16(0).u16(if text { 2 } else { 0 }).u16(if video || text { 0 } else { 0x0100 }).u16(0);
    matrix(b);
    b.u32(t.width << 16).u32(t.height << 16).end();
    // The edit list: a pause before a track that starts later, then the track from its first presentation time (the
    // decode time of a video's first frame comes before it when B-frames reorder pictures).
    let delay = ((start_of(t) - origin) * MOVIE_TIMESCALE as f64).round() as u64;
    let media_start = media_start(t);
    // Presentation runs to the last sample's end: as long as the decode times, less anything the edit list skips
    // beyond the first picture (a lead-in). Rounded up: rounded down, the player cuts the last few sound samples.
    let skipped = (media_start - (first_pts(t) - t.samples[0].dts).max(0)).max(0) as u64;
    let shown = ((media_duration - skipped.min(media_duration)) * MOVIE_TIMESCALE).div_ceil(t.timescale as u64);
    b.begin(b"edts").begin_full(b"elst", 1, 0).u32(if delay > 0 { 2 } else { 1 });
    if delay > 0 {
        b.u64(delay).u64(u64::MAX).u32(0x0001_0000);
    }
    b.u64(shown).u64(media_start as u64).u32(0x0001_0000).end().end();
    b.begin(b"mdia");
    let lang = t.language.bytes().chain(std::iter::repeat(b'a')).take(3).fold(0u16, |acc, c| (acc << 5) | ((c.wrapping_sub(0x60)) as u16 & 0x1f));
    b.begin_full(b"mdhd", 1, 0).u64(0).u64(0).u32(t.timescale).u64(media_duration).u16(lang).u16(0).end();
    let (handler, name): (&[u8; 4], &[u8]) = match t.kind {
        Kind::Video => (b"vide", b"VideoHandler\0"),
        Kind::Audio => (b"soun", b"SoundHandler\0"),
        Kind::Text => (b"sbtl", b"SubtitleHandler\0"),
    };
    b.begin_full(b"hdlr", 0, 0).u32(0).bytes(handler).zeros(12).bytes(name).end();
    b.begin(b"minf");
    match t.kind {
        Kind::Video => b.begin_full(b"vmhd", 0, 1).zeros(8).end(),
        Kind::Audio => b.begin_full(b"smhd", 0, 0).u32(0).end(),
        Kind::Text => b.begin_full(b"nmhd", 0, 0).end(),
    };
    b.begin(b"dinf").begin_full(b"dref", 0, 0).u32(1).begin_full(b"url ", 0, 1).end().end().end();
    b.begin(b"stbl");
    b.begin_full(b"stsd", 0, 0).u32(1).bytes(&t.entry).end();
    let stts = runs(t.samples.iter().map(|s| s.duration));
    b.begin_full(b"stts", 0, 0).u32(stts.len() as u32);
    for (n, d) in stts {
        b.u32(n).u32(d);
    }
    b.end();
    if t.samples.iter().any(|s| s.cts != 0) {
        let ctts = runs(t.samples.iter().map(|s| s.cts));
        let version = if ctts.iter().any(|&(_, c)| c < 0) { 1 } else { 0 };
        b.begin_full(b"ctts", version, 0).u32(ctts.len() as u32);
        for (n, c) in ctts {
            b.u32(n).u32(c as u32);
        }
        b.end();
    }
    if t.samples.iter().any(|s| !s.sync) {
        let sync: Vec<u32> = t.samples.iter().enumerate().filter(|(_, s)| s.sync).map(|(i, _)| i as u32 + 1).collect();
        b.begin_full(b"stss", 0, 0).u32(sync.len() as u32);
        for n in sync {
            b.u32(n);
        }
        b.end();
    }
    // One sample per chunk: samples of different tracks are interleaved in time.
    b.begin_full(b"stsc", 0, 0).u32(1).u32(1).u32(1).u32(1).end();
    b.begin_full(b"stsz", 0, 0).u32(0).u32(t.samples.len() as u32);
    for s in &t.samples {
        b.u32(s.size);
    }
    b.end();
    b.begin_full(b"co64", 0, 0).u32(offsets.len() as u32);
    for &o in offsets {
        b.u64(o);
    }
    b.end();
    b.end().end().end().end();
}
