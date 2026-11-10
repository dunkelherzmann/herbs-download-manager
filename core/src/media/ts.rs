//! MPEG transport streams, as HLS pieces carry them: 188-byte packets → each stream's PES packets → samples for MP4,
//! their bytes unchanged apart from the framing MP4 wants (start codes become lengths, ADTS headers go into the
//! track's configuration).

use std::collections::HashMap;

use super::bits::{nal_units, unescape, Bits, Boxes};
use super::{JoinError, Kind, Sample, Spool, Track};

const PACKET: usize = 188;

/// One PES packet: a frame (video) or some blocks of sound.
#[derive(Debug, Default)]
pub struct Pes {
    pub pid: u16,
    pub stream_type: u8,
    pub pts: Option<u64>,
    pub dts: Option<u64>,
    pub data: Vec<u8>,
    /// The packet that started it said "a picture can start here".
    pub random_access: bool,
}

/// Reads packets into PES packets, finding the streams through the PAT and PMT.
#[derive(Default)]
pub struct Demuxer {
    pmt_pids: Vec<u16>,
    /// PID → stream type (0x1B H.264, 0x24 H.265, 0x0F AAC, 0x03/0x04 MPEG audio, 0x81 AC-3…).
    pub streams: HashMap<u16, u8>,
    open: HashMap<u16, Pes>,
}

fn section_payload(payload: &[u8], start: bool) -> Option<&[u8]> {
    if !start {
        return None;
    }
    let pointer = *payload.first()? as usize;
    payload.get(1 + pointer..)
}

impl Demuxer {
    /// Reads one piece's bytes; complete PES packets go to `out`. A piece ends its last PES packet (`flush`).
    pub fn feed(&mut self, data: &[u8], out: &mut Vec<Pes>) -> Result<(), JoinError> {
        let mut i = 0;
        while i + PACKET <= data.len() {
            if data[i] != 0x47 {
                // Lost the packet boundary: find the next sync byte.
                i += 1;
                continue;
            }
            self.packet(&data[i..i + PACKET], out);
            i += PACKET;
        }
        Ok(())
    }

    pub fn flush(&mut self, out: &mut Vec<Pes>) {
        let mut pids: Vec<u16> = self.open.keys().copied().collect();
        pids.sort();
        for pid in pids {
            if let Some(pes) = self.open.remove(&pid)
                && let Some(done) = finish_pes(pes)
            {
                out.push(done);
            }
        }
    }

    fn packet(&mut self, p: &[u8], out: &mut Vec<Pes>) {
        let start = p[1] & 0x40 != 0;
        let pid = ((p[1] as u16 & 0x1f) << 8) | p[2] as u16;
        let control = (p[3] >> 4) & 3;
        let mut at = 4;
        let mut random_access = false;
        if control & 2 != 0 {
            let len = p[4] as usize;
            if len > 0 {
                random_access = p[5] & 0x40 != 0;
            }
            at += 1 + len;
        }
        if control & 1 == 0 || at >= PACKET {
            return;
        }
        let payload = &p[at..];
        if pid == 0 {
            if let Some(s) = section_payload(payload, start) {
                self.pat(s);
            }
        } else if self.pmt_pids.contains(&pid) {
            if let Some(s) = section_payload(payload, start) {
                self.pmt(s);
            }
        } else if let Some(&stream_type) = self.streams.get(&pid) {
            if start {
                if let Some(done) = self.open.remove(&pid).and_then(finish_pes) {
                    out.push(done);
                }
                self.open.insert(pid, Pes { pid, stream_type, random_access, ..Default::default() });
            }
            if let Some(pes) = self.open.get_mut(&pid) {
                pes.data.extend_from_slice(payload);
            }
        }
    }

    fn pat(&mut self, s: &[u8]) {
        if s.len() < 8 {
            return;
        }
        let len = (((s[1] as usize) & 0x0f) << 8 | s[2] as usize).min(s.len() - 3);
        let entries = &s[8..(3 + len).saturating_sub(4).max(8)];
        for e in entries.chunks_exact(4) {
            let program = u16::from_be_bytes([e[0], e[1]]);
            let pid = ((e[2] as u16 & 0x1f) << 8) | e[3] as u16;
            if program != 0 && !self.pmt_pids.contains(&pid) {
                self.pmt_pids.push(pid);
            }
        }
    }

    fn pmt(&mut self, s: &[u8]) {
        if s.len() < 12 {
            return;
        }
        let len = (((s[1] as usize) & 0x0f) << 8 | s[2] as usize).min(s.len() - 3);
        let end = (3 + len).saturating_sub(4);
        let info = (((s[10] as usize) & 0x0f) << 8) | s[11] as usize;
        let mut i = 12 + info;
        while i + 5 <= end {
            let stream_type = s[i];
            let pid = ((s[i + 1] as u16 & 0x1f) << 8) | s[i + 2] as u16;
            let es_info = (((s[i + 3] as usize) & 0x0f) << 8) | s[i + 4] as usize;
            // Private data (0x06) is often AC-3 or subtitles: told apart by its descriptors.
            let descriptors = s.get(i + 5..i + 5 + es_info).unwrap_or(&[]);
            let kind = if stream_type == 0x06 && has_descriptor(descriptors, &[0x6a, 0x81]) { 0x81 } else { stream_type };
            self.streams.insert(pid, kind);
            i += 5 + es_info;
        }
    }
}

fn has_descriptor(descriptors: &[u8], tags: &[u8]) -> bool {
    let mut i = 0;
    while i + 2 <= descriptors.len() {
        if tags.contains(&descriptors[i]) {
            return true;
        }
        i += 2 + descriptors[i + 1] as usize;
    }
    false
}

fn timestamp(b: &[u8]) -> u64 {
    ((b[0] as u64 >> 1) & 7) << 30 | (b[1] as u64) << 22 | (b[2] as u64 >> 1) << 15 | (b[3] as u64) << 7 | b[4] as u64 >> 1
}

/// Strips the PES header, keeping the timestamps.
fn finish_pes(mut pes: Pes) -> Option<Pes> {
    let d = &pes.data;
    if d.len() < 9 || d[0] != 0 || d[1] != 0 || d[2] != 1 {
        return None;
    }
    let flags = d[7] >> 6;
    let header = 9 + d[8] as usize;
    if d.len() < header {
        return None;
    }
    if flags & 2 != 0 && d.len() >= 14 {
        pes.pts = Some(timestamp(&d[9..14]));
    }
    if flags == 3 && d.len() >= 19 {
        pes.dts = Some(timestamp(&d[14..19]));
    }
    pes.data.drain(..header);
    Some(pes)
}

/// 33-bit timestamps that roll over every 26.5 hours, made into one rising count.
#[derive(Default, Clone, Copy)]
struct Unwrap {
    last: Option<u64>,
    offset: i64,
}

impl Unwrap {
    fn get(&mut self, raw: u64) -> i64 {
        const WRAP: i64 = 1 << 33;
        if let Some(last) = self.last {
            let (raw_i, last_i) = (raw as i64, last as i64);
            if last_i - raw_i > WRAP / 2 {
                self.offset += WRAP;
            } else if raw_i - last_i > WRAP / 2 {
                self.offset -= WRAP;
            }
        }
        self.last = Some(raw);
        raw as i64 + self.offset
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    H264,
    Hevc,
    Aac,
    /// MPEG-1/2 audio layer II or III.
    Mpeg,
}

impl Codec {
    pub fn of(stream_type: u8) -> Result<Option<Codec>, JoinError> {
        Ok(Some(match stream_type {
            0x1b => Codec::H264,
            0x24 => Codec::Hevc,
            0x0f => Codec::Aac,
            0x03 | 0x04 => Codec::Mpeg,
            0x81 | 0x87 => return Err(JoinError::Unsupported("Dolby (AC-3) sound".into())),
            // Timed metadata (ID3), subtitles and the like: not part of the picture or the sound.
            _ => return Ok(None),
        }))
    }
}

/// One stream turned into MP4 samples as its PES packets come.
pub struct Stream {
    pub codec: Codec,
    clock: Unwrap,
    /// H.264: SPS and PPS; H.265: VPS, SPS and PPS — the first of each, for the sample description.
    vps: Option<Vec<u8>>,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    /// Sound: (sample rate, channels, AudioSpecificConfig or MPEG version) once the first block is read.
    sound: Option<SoundFormat>,
    /// Sound bytes of a block cut by the end of a PES packet.
    pending: Vec<u8>,
    /// The next sound block's time, in the track's timescale.
    next_dts: Option<i64>,
    pub samples: Vec<Sample>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SoundFormat {
    rate: u32,
    channels: u16,
    /// AAC: the AudioSpecificConfig's two bytes; MPEG audio: [version (1, 2 or 25), layer].
    config: [u8; 2],
    /// Samples a block holds.
    per_block: u32,
}

const AAC_RATES: [u32; 13] = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];

/// One ADTS frame at the start of `d`: its format, header length and whole length.
fn adts(d: &[u8]) -> Option<(SoundFormat, usize, usize)> {
    if d.len() < 7 || d[0] != 0xff || d[1] & 0xf6 != 0xf0 {
        return None;
    }
    let header = if d[1] & 1 == 0 { 9 } else { 7 };
    let profile = (d[2] >> 6) + 1;
    let rate_index = (d[2] >> 2) & 0x0f;
    let channels = ((d[2] & 1) << 2) | (d[3] >> 6);
    let len = (((d[3] as usize) & 3) << 11) | (d[4] as usize) << 3 | (d[5] as usize) >> 5;
    let rate = *AAC_RATES.get(rate_index as usize)?;
    let asc = [(profile << 3) | (rate_index >> 1), ((rate_index & 1) << 7) | (channels << 3)];
    (len >= header).then_some((SoundFormat { rate, channels: channels.max(1) as u16, config: asc, per_block: 1024 }, header, len))
}

/// One MPEG audio frame at the start of `d`: its format and length.
fn mpeg_audio(d: &[u8]) -> Option<(SoundFormat, usize)> {
    if d.len() < 4 || d[0] != 0xff || d[1] & 0xe0 != 0xe0 {
        return None;
    }
    let version = match (d[1] >> 3) & 3 {
        3 => 1,
        2 => 2,
        0 => 25,
        _ => return None,
    };
    let layer = match (d[1] >> 1) & 3 {
        1 => 3,
        2 => 2,
        _ => return None, // Layer I: not in HLS
    };
    const V1_L3: [u32; 15] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
    const V1_L2: [u32; 15] = [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384];
    const V2: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
    let rate_bits = ((d[2] >> 2) & 3) as usize;
    let base = [44100u32, 48000, 32000].get(rate_bits)?;
    let rate = match version {
        1 => *base,
        2 => base / 2,
        _ => base / 4,
    };
    let bitrate = match (version, layer) {
        (1, 3) => V1_L3,
        (1, _) => V1_L2,
        _ => V2,
    }
    .get(((d[2] >> 4) & 0x0f) as usize)
    .copied()
    .filter(|&b| b > 0)?
        * 1000;
    let padding = ((d[2] >> 1) & 1) as u32;
    let per_block = if version == 1 || layer == 2 { 1152 } else { 576 };
    let len = (per_block / 8 * bitrate / rate + padding) as usize;
    let channels = if d[3] >> 6 == 3 { 1 } else { 2 };
    Some((SoundFormat { rate, channels, config: [version, layer], per_block }, len))
}

impl Stream {
    pub fn new(codec: Codec) -> Stream {
        Stream { codec, clock: Unwrap::default(), vps: None, sps: None, pps: None, sound: None, pending: Vec::new(), next_dts: None, samples: Vec::new() }
    }

    pub fn kind(&self) -> Kind {
        match self.codec {
            Codec::H264 | Codec::Hevc => Kind::Video,
            Codec::Aac | Codec::Mpeg => Kind::Audio,
        }
    }

    /// Where the timeline was before the next piece: so a piece that starts its timestamps again can be placed after.
    pub fn timescale(&self) -> u32 {
        match self.kind() {
            Kind::Video => 90_000,
            Kind::Audio | Kind::Text => self.sound.map(|s| s.rate).unwrap_or(90_000),
        }
    }

    /// Forgets the 33-bit history, for a piece whose timestamps start again (EXT-X-DISCONTINUITY).
    pub fn restart_clock(&mut self) {
        self.clock = Unwrap::default();
        self.next_dts = None;
        self.pending.clear();
    }

    pub fn push(&mut self, pes: Pes, spool: &mut Spool) -> Result<(), JoinError> {
        match self.codec {
            Codec::H264 | Codec::Hevc => self.video(pes, spool),
            Codec::Aac | Codec::Mpeg => self.sound(pes, spool),
        }
    }

    fn video(&mut self, pes: Pes, spool: &mut Spool) -> Result<(), JoinError> {
        let Some(pts) = pes.pts else { return Ok(()) };
        let pts = self.clock.get(pts);
        let dts = match pes.dts {
            Some(d) => pts - ((pes.pts.unwrap() as i64 - d as i64).rem_euclid(1 << 33)),
            None => pts,
        };
        let mut data = Vec::with_capacity(pes.data.len() + 16);
        let mut sync = false;
        for nal in nal_units(&pes.data) {
            let (kind, keep) = match self.codec {
                Codec::H264 => {
                    let t = nal[0] & 0x1f;
                    match t {
                        7 => {
                            self.sps.get_or_insert_with(|| nal.to_vec());
                        }
                        8 => {
                            self.pps.get_or_insert_with(|| nal.to_vec());
                        }
                        5 => sync = true,
                        _ => {}
                    }
                    (t, t != 9)
                }
                _ => {
                    let t = (nal[0] >> 1) & 0x3f;
                    match t {
                        32 => {
                            self.vps.get_or_insert_with(|| nal.to_vec());
                        }
                        33 => {
                            self.sps.get_or_insert_with(|| nal.to_vec());
                        }
                        34 => {
                            self.pps.get_or_insert_with(|| nal.to_vec());
                        }
                        16..=21 => sync = true,
                        _ => {}
                    }
                    (t, t != 35)
                }
            };
            let _ = kind;
            // Access unit delimiters say nothing MP4 needs; everything else is kept as it came.
            if keep {
                data.extend((nal.len() as u32).to_be_bytes());
                data.extend(nal);
            }
        }
        if data.is_empty() {
            return Ok(());
        }
        let offset = spool.put(&data)?;
        if let Some(prev) = self.samples.last_mut()
            && prev.duration == 0
        {
            prev.duration = (dts - prev.dts).clamp(1, u32::MAX as i64) as u32;
        }
        self.samples.push(Sample { dts, cts: (pts - dts) as i32, duration: 0, sync, offset, size: data.len() as u32 });
        Ok(())
    }

    fn sound(&mut self, pes: Pes, spool: &mut Spool) -> Result<(), JoinError> {
        let pes_time = pes.pts.map(|p| self.clock.get(p));
        let mut d = std::mem::take(&mut self.pending);
        let carried = !d.is_empty();
        d.extend_from_slice(&pes.data);
        let mut i = 0;
        let mut first = true;
        loop {
            let found = match self.codec {
                Codec::Aac => adts(&d[i..]).map(|(f, h, len)| (f, h, len)),
                _ => mpeg_audio(&d[i..]).map(|(f, len)| (f, 0, len)),
            };
            let Some((format, header, len)) = found else {
                if i < d.len() && d.len() - i >= 7 {
                    // Not a block start: skip a byte and look again.
                    i += 1;
                    continue;
                }
                break;
            };
            if i + len > d.len() {
                break;
            }
            let format = *self.sound.get_or_insert(format);
            // The block's time: counted on from the last one, set again from the PES time when it's off by more
            // than a block (a gap, or the first block).
            if first && !carried
                && let Some(t90) = pes_time
            {
                let t = (t90 as i128 * format.rate as i128 / 90_000) as i64;
                if self.next_dts.is_none_or(|n| (n - t).abs() > format.per_block as i64) {
                    self.next_dts = Some(t);
                }
            }
            first = false;
            let dts = self.next_dts.unwrap_or(0);
            let body = &d[i + header..i + len];
            let offset = spool.put(body)?;
            self.samples.push(Sample { dts, cts: 0, duration: format.per_block, sync: true, offset, size: body.len() as u32 });
            self.next_dts = Some(dts + format.per_block as i64);
            i += len;
        }
        self.pending = d[i.min(d.len())..].to_vec();
        Ok(())
    }

    /// The finished track: its sample description from the parameter sets or sound format it read.
    pub fn track(self) -> Result<Track, JoinError> {
        let kind = self.kind();
        let mut b = Boxes::default();
        let (width, height, timescale);
        match self.codec {
            Codec::H264 => {
                let (sps, pps) = (self.sps.ok_or_else(|| damaged("no H.264 SPS"))?, self.pps.ok_or_else(|| damaged("no H.264 PPS"))?);
                (width, height) = h264_size(&sps).ok_or_else(|| damaged("an H.264 SPS that can't be read"))?;
                visual_entry(&mut b, b"avc1", width, height);
                b.begin(b"avcC").u8(1).u8(sps[1]).u8(sps[2]).u8(sps[3]).u8(0xff).u8(0xe1).u16(sps.len() as u16).bytes(&sps);
                b.u8(1).u16(pps.len() as u16).bytes(&pps).end();
                b.end();
                timescale = 90_000;
            }
            Codec::Hevc => {
                let (vps, sps, pps) = (
                    self.vps.ok_or_else(|| damaged("no H.265 VPS"))?,
                    self.sps.ok_or_else(|| damaged("no H.265 SPS"))?,
                    self.pps.ok_or_else(|| damaged("no H.265 PPS"))?,
                );
                let info = hevc_sps(&sps).ok_or_else(|| damaged("an H.265 SPS that can't be read"))?;
                (width, height) = (info.width, info.height);
                visual_entry(&mut b, b"hvc1", width, height);
                b.begin(b"hvcC").u8(1).bytes(&info.profile).u16(0xf000).u8(0xfc).u8(0xfc | info.chroma);
                b.u8(0xf8 | info.luma_bits).u8(0xf8 | info.chroma_bits).u16(0);
                b.u8((info.sub_layers << 3) | (info.nested << 2) | 3).u8(3);
                for (t, nal) in [(32u8, &vps), (33, &sps), (34, &pps)] {
                    b.u8(0x80 | t).u16(1).u16(nal.len() as u16).bytes(nal);
                }
                b.end().end();
                timescale = 90_000;
            }
            Codec::Aac | Codec::Mpeg => {
                let s = self.sound.ok_or_else(|| damaged("no sound blocks"))?;
                (width, height, timescale) = (0, 0, s.rate);
                b.begin(b"mp4a").zeros(6).u16(1).zeros(8).u16(s.channels).u16(16).zeros(4).u32(s.rate << 16);
                let (object, config): (u8, &[u8]) = match self.codec {
                    Codec::Aac => (0x40, &s.config),
                    _ => (if s.config[0] == 1 { 0x6b } else { 0x69 }, &[]),
                };
                b.begin_full(b"esds", 0, 0);
                let dsi_len = if config.is_empty() { 0 } else { 2 + config.len() };
                let dcd_len = 13 + dsi_len;
                b.u8(3).u8((3 + 2 + dcd_len + 3) as u8).u16(0).u8(0);
                b.u8(4).u8(dcd_len as u8).u8(object).u8(0x15).bytes(&[0, 0, 0]).u32(0).u32(0);
                if !config.is_empty() {
                    b.u8(5).u8(config.len() as u8).bytes(config);
                }
                b.u8(6).u8(1).u8(2);
                b.end().end();
            }
        }
        let mut samples = self.samples;
        if let Some(last) = samples.len().checked_sub(2).map(|i| samples[i].duration)
            && let Some(s) = samples.last_mut()
            && s.duration == 0
        {
            s.duration = last.max(1);
        }
        if let Some(s) = samples.last_mut()
            && s.duration == 0
        {
            s.duration = 3600;
        }
        if samples.is_empty() {
            return Err(damaged("a stream with no samples"));
        }
        Ok(Track { kind, timescale, entry: b.out, width, height, language: "und".into(), samples, edit: None })
    }
}

fn damaged(what: &str) -> JoinError {
    JoinError::Damaged(format!("The stream has {what}"))
}

/// The start of a video sample entry (`avc1`, `hvc1`), left open for its configuration box.
fn visual_entry(b: &mut Boxes, kind: &[u8; 4], width: u32, height: u32) {
    b.begin(kind).zeros(6).u16(1).zeros(16).u16(width as u16).u16(height as u16).u32(0x0048_0000).u32(0x0048_0000);
    b.u32(0).u16(1).zeros(32).u16(0x18).u16(0xffff);
}

/// The picture size an H.264 SPS gives (after its cropping).
pub fn h264_size(nal: &[u8]) -> Option<(u32, u32)> {
    let rbsp = unescape(&nal[1..]);
    let mut b = Bits::new(&rbsp);
    let profile = b.bits(8)?;
    b.skip(16)?;
    b.ue()?;
    let mut chroma = 1;
    if [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135].contains(&profile) {
        chroma = b.ue()?;
        if chroma == 3 {
            b.skip(1)?;
        }
        b.ue()?;
        b.ue()?;
        b.skip(1)?;
        if b.bit()? == 1 {
            for i in 0..if chroma == 3 { 12 } else { 8 } {
                if b.bit()? == 1 {
                    let size = if i < 6 { 16 } else { 64 };
                    let (mut last, mut next) = (8i32, 8i32);
                    for _ in 0..size {
                        if next != 0 {
                            next = (last + b.se()? + 256) % 256;
                        }
                        last = if next == 0 { last } else { next };
                    }
                }
            }
        }
    }
    b.ue()?;
    match b.ue()? {
        0 => {
            b.ue()?;
        }
        1 => {
            b.skip(1)?;
            b.se()?;
            b.se()?;
            for _ in 0..b.ue()? {
                b.se()?;
            }
        }
        _ => {}
    }
    b.ue()?;
    b.skip(1)?;
    let w_mbs = b.ue()? + 1;
    let h_units = b.ue()? + 1;
    let frame_only = b.bit()?;
    if frame_only == 0 {
        b.skip(1)?;
    }
    b.skip(1)?;
    let (mut cl, mut cr, mut ct, mut cb) = (0, 0, 0, 0);
    if b.bit()? == 1 {
        (cl, cr, ct, cb) = (b.ue()?, b.ue()?, b.ue()?, b.ue()?);
    }
    let (sub_w, sub_h) = match chroma {
        1 => (2, 2),
        2 => (2, 1),
        _ => (1, 1),
    };
    let width = w_mbs * 16 - sub_w * (cl + cr);
    let height = (2 - frame_only) * h_units * 16 - sub_h * (2 - frame_only) * (ct + cb);
    Some((width, height))
}

struct HevcInfo {
    /// general_profile_space … general_level_idc, as hvcC wants them (12 bytes).
    profile: [u8; 12],
    width: u32,
    height: u32,
    chroma: u8,
    luma_bits: u8,
    chroma_bits: u8,
    sub_layers: u8,
    nested: u8,
}

fn hevc_sps(nal: &[u8]) -> Option<HevcInfo> {
    let rbsp = unescape(&nal[2..]);
    let mut b = Bits::new(&rbsp);
    b.skip(4)?;
    let max_sub = b.bits(3)?;
    let nested = b.bit()? as u8;
    let mut profile = [0u8; 12];
    for byte in &mut profile {
        *byte = b.bits(8)? as u8;
    }
    let mut present = Vec::new();
    for _ in 0..max_sub {
        present.push((b.bit()?, b.bit()?));
    }
    if max_sub > 0 {
        for _ in max_sub..8 {
            b.skip(2)?;
        }
    }
    for (profile_present, level_present) in present {
        if profile_present == 1 {
            b.skip(88)?;
        }
        if level_present == 1 {
            b.skip(8)?;
        }
    }
    b.ue()?;
    let chroma = b.ue()?;
    if chroma == 3 {
        b.skip(1)?;
    }
    let (mut width, mut height) = (b.ue()?, b.ue()?);
    if b.bit()? == 1 {
        let (sub_w, sub_h) = match chroma {
            1 => (2, 2),
            2 => (2, 1),
            _ => (1, 1),
        };
        let (l, r, t, bt) = (b.ue()?, b.ue()?, b.ue()?, b.ue()?);
        width -= sub_w * (l + r);
        height -= sub_h * (t + bt);
    }
    let luma = b.ue()? as u8;
    let chroma_bits = b.ue()? as u8;
    Some(HevcInfo { profile, width, height, chroma: chroma as u8, luma_bits: luma, chroma_bits, sub_layers: max_sub as u8 + 1, nested })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sound_frames_are_read() {
        // An ADTS header: AAC LC, 44.1 kHz (index 4), stereo, 20 bytes long.
        let frame = [0xff, 0xf1, 0x50, 0x80, 0x02, 0x9f, 0xfc];
        let (format, header, len) = adts(&frame).unwrap();
        assert_eq!((format.rate, format.channels, header, len), (44100, 2, 7, 20));
        assert_eq!(format.config, [0x12, 0x10], "AAC LC, 44.1 kHz, two channels");
        // MPEG-1 layer III, 128 kb/s, 44.1 kHz, no padding: 417 bytes.
        let (mp3, len) = mpeg_audio(&[0xff, 0xfb, 0x90, 0x00]).unwrap();
        assert_eq!((mp3.rate, mp3.per_block, len), (44100, 1152, 417));
    }

    #[test]
    fn timestamps_keep_rising_past_the_rollover() {
        let mut u = Unwrap::default();
        assert_eq!(u.get((1 << 33) - 10), (1 << 33) - 10);
        assert_eq!(u.get(5), (1 << 33) + 5);
    }
}
