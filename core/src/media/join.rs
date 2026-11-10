//! Joins a stream's pieces into one MP4: MPEG-TS pieces or MP4 fragments, from one playlist or two (a separate
//! sound track), in order. Nothing is re-encoded.

use std::collections::BTreeMap;
use std::path::Path;

use super::mp4::{self, FragmentedTrack};
use super::ts::{Codec, Demuxer, Pes, Stream};
use super::{JoinError, Kind, Spool, Track};

/// One playlist's pieces.
enum Source {
    /// Not known until the first piece.
    Unknown,
    Ts { demuxer: Demuxer, streams: BTreeMap<u16, Stream> },
    Fragments { tracks: Vec<FragmentedTrack> },
}

pub struct Joiner {
    spool: Spool,
    sources: Vec<Source>,
    /// Only the sound is kept (the picture's tracks are left out of the file).
    sound_only: bool,
    /// Only the first whole picture is kept: a still for a preview.
    still: bool,
}

/// What `finish` wrote.
#[derive(Debug, PartialEq, Eq)]
pub enum Written {
    Mp4,
    /// Sound only, in MPEG audio (MP3): its frames one after the other, as the stream had them.
    Mp3,
}

/// A piece whose timestamps jumped by more than this from where the last one ended starts again (a new clip, an
/// advert's own clock): it's placed right after the previous one instead.
const JUMP_SECONDS: f64 = 10.0;

impl Joiner {
    /// The pieces' bytes wait in `spool_path` until `finish`.
    pub fn new(spool_path: &Path) -> Result<Joiner, JoinError> {
        Ok(Joiner { spool: Spool::create(spool_path)?, sources: Vec::new(), sound_only: false, still: false })
    }

    /// Keeps only the sound: the file is an `.m4a` (or `.mp3`), with no picture.
    pub fn sound_only(&mut self) {
        self.sound_only = true;
    }

    /// Keeps only the first whole picture, with no sound: a still for a preview.
    pub fn still(&mut self) {
        self.still = true;
    }

    /// A new playlist (the video's, then a separate sound track's); returns its number.
    pub fn source(&mut self) -> usize {
        self.sources.push(Source::Unknown);
        self.sources.len() - 1
    }

    /// An MP4 init section (EXT-X-MAP) for the source's fragments.
    pub fn init(&mut self, source: usize, data: &[u8]) -> Result<(), JoinError> {
        let mut tracks = mp4::read_init(data)?;
        // A new init section (after a discontinuity) keeps the samples read so far.
        if let Source::Fragments { tracks: old } = &mut self.sources[source] {
            for t in &mut tracks {
                if let Some(o) = old.iter_mut().find(|o| o.id == t.id) {
                    t.track.samples = std::mem::take(&mut o.track.samples);
                }
            }
        }
        self.sources[source] = Source::Fragments { tracks };
        Ok(())
    }

    /// The next piece of `source`, in playlist order.
    pub fn piece(&mut self, source: usize, data: &[u8], discontinuity: bool) -> Result<(), JoinError> {
        if matches!(self.sources[source], Source::Unknown) {
            if data.first() != Some(&0x47) {
                return Err(JoinError::Damaged("A piece is neither MPEG-TS nor MP4 with an init section".into()));
            }
            self.sources[source] = Source::Ts { demuxer: Demuxer::default(), streams: BTreeMap::new() };
        }
        let spool = &mut self.spool;
        match &mut self.sources[source] {
            Source::Unknown => unreachable!(),
            Source::Fragments { tracks } => {
                let before: Vec<usize> = tracks.iter().map(|t| t.track.samples.len()).collect();
                mp4::read_fragments(data, tracks, spool)?;
                if discontinuity {
                    let mut views: Vec<&mut Track> = tracks.iter_mut().map(|t| &mut t.track).collect();
                    place_after(&mut views, &before);
                }
            }
            Source::Ts { demuxer, streams } => {
                let mut packets = Vec::new();
                demuxer.feed(data, &mut packets)?;
                demuxer.flush(&mut packets);
                if discontinuity {
                    streams.values_mut().for_each(Stream::restart_clock);
                }
                let before: BTreeMap<u16, usize> = streams.iter().map(|(&pid, s)| (pid, s.samples.len())).collect();
                for pes in packets {
                    push(streams, &demuxer.streams, pes, spool)?;
                }
                // Timestamps that went back or jumped ahead: the piece follows the previous one.
                let mut shift: Option<f64> = None;
                let (mut prev_end, mut new_start) = (f64::NEG_INFINITY, f64::INFINITY);
                for (pid, s) in streams.iter() {
                    let from = before.get(pid).copied().unwrap_or(0);
                    let ts = s.timescale() as f64;
                    if let Some(last) = from.checked_sub(1).map(|i| s.samples[i]) {
                        prev_end = prev_end.max((last.dts + last.duration.max(1) as i64) as f64 / ts);
                    }
                    if let Some(first) = s.samples.get(from) {
                        new_start = new_start.min(first.dts as f64 / ts);
                    }
                }
                if prev_end.is_finite() && new_start.is_finite() && (new_start < prev_end - 0.5 || new_start > prev_end + JUMP_SECONDS) {
                    shift = Some(prev_end - new_start);
                }
                if let Some(shift) = shift {
                    for (pid, s) in streams.iter_mut() {
                        let from = before.get(pid).copied().unwrap_or(0);
                        let by = (shift * s.timescale() as f64).round() as i64;
                        for sample in &mut s.samples[from..] {
                            sample.dts += by;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Writes the MP4 (or, sound only in MP3, the MP3) to `path`. The spool file can go afterwards.
    pub fn finish(mut self, path: &Path) -> Result<Written, JoinError> {
        let sound_only = self.sound_only;
        let wanted = |kind: Kind| !sound_only || kind == Kind::Audio;
        // Each track, and whether it's MPEG audio from MPEG-TS pieces.
        let mut tracks = Vec::new();
        for source in std::mem::take(&mut self.sources) {
            match source {
                Source::Unknown => {}
                Source::Fragments { tracks: t } => {
                    tracks.extend(t.into_iter().map(|t| (t.track, false)).filter(|(t, _)| !t.samples.is_empty() && wanted(t.kind)))
                }
                Source::Ts { streams, .. } => {
                    for s in streams.into_values().filter(|s| wanted(s.kind())) {
                        let mp3 = s.codec == Codec::Mpeg;
                        tracks.push((s.track()?, mp3));
                    }
                }
            }
        }
        if sound_only {
            // One sound track: the first (a separate sound playlist comes after the video's).
            tracks.truncate(1);
            match tracks.first() {
                None => return Err(JoinError::NoSound),
                Some((track, true)) => {
                    // MP3 frames carry their own headers: saved one after the other, as an .mp3 file.
                    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
                    let mut buf = Vec::new();
                    for sample in &track.samples {
                        buf.clear();
                        self.spool.read(sample.offset, sample.size, &mut buf)?;
                        std::io::Write::write_all(&mut out, &buf)?;
                    }
                    std::io::Write::flush(&mut out)?;
                    return Ok(Written::Mp3);
                }
                Some(_) => {}
            }
        }
        let mut tracks: Vec<Track> = tracks.into_iter().map(|(t, _)| t).collect();
        if self.still {
            // The first picture that stands on its own (a key frame), shown for a moment.
            let mut video = tracks.into_iter().find(|t| t.kind == Kind::Video).ok_or_else(|| JoinError::Damaged("This stream has no picture".into()))?;
            let first = *video.samples.iter().find(|s| s.sync).ok_or_else(|| JoinError::Damaged("The piece has no whole picture".into()))?;
            video.samples = vec![super::Sample { dts: 0, cts: 0, duration: video.timescale.max(1), ..first }];
            video.edit = None;
            tracks = vec![video];
        }
        // Durations still unknown at a piece's edge come from the next sample.
        for t in &mut tracks {
            for i in 1..t.samples.len() {
                if t.samples[i - 1].duration == 0 {
                    t.samples[i - 1].duration = (t.samples[i].dts - t.samples[i - 1].dts).clamp(1, u32::MAX as i64) as u32;
                }
            }
        }
        mp4::write(path, &tracks, &mut self.spool)?;
        Ok(Written::Mp4)
    }
}

fn push(streams: &mut BTreeMap<u16, Stream>, types: &std::collections::HashMap<u16, u8>, pes: Pes, spool: &mut Spool) -> Result<(), JoinError> {
    if !streams.contains_key(&pes.pid) {
        let Some(codec) = Codec::of(*types.get(&pes.pid).unwrap_or(&0))? else { return Ok(()) };
        streams.insert(pes.pid, Stream::new(codec));
    }
    streams.get_mut(&pes.pid).unwrap().push(pes, spool)
}

/// MP4 fragments after a discontinuity: placed right after the previous ones when their times went back or jumped.
fn place_after(tracks: &mut [&mut Track], before: &[usize]) {
    let (mut prev_end, mut new_start) = (f64::NEG_INFINITY, f64::INFINITY);
    for (t, &from) in tracks.iter().zip(before) {
        let ts = t.timescale as f64;
        if let Some(last) = from.checked_sub(1).map(|i| t.samples[i]) {
            prev_end = prev_end.max((last.dts + last.duration as i64) as f64 / ts);
        }
        if let Some(first) = t.samples.get(from) {
            new_start = new_start.min(first.dts as f64 / ts);
        }
    }
    if !(prev_end.is_finite() && new_start.is_finite()) || (new_start >= prev_end - 0.5 && new_start <= prev_end + JUMP_SECONDS) {
        return;
    }
    let shift = prev_end - new_start;
    for (t, &from) in tracks.iter_mut().zip(before) {
        let by = (shift * t.timescale as f64).round() as i64;
        for s in &mut t.samples[from..] {
            s.dts += by;
        }
    }
}

/// Decrypts an AES-128 piece (CBC, PKCS#7 padding).
pub fn decrypt_aes128(data: &[u8], key: &[u8; 16], iv: &[u8; 16]) -> Result<Vec<u8>, JoinError> {
    use aes::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
    cbc::Decryptor::<aes::Aes128>::new(key.into(), iv.into())
        .decrypt_padded_vec_mut::<Pkcs7>(data)
        .map_err(|_| JoinError::Damaged("A piece couldn't be decrypted with its key".into()))
}
