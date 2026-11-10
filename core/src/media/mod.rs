//! Streamed video and sound (SPECIFICATION.md §15): HLS playlists, and joining a stream's pieces into one MP4 file
//! without re-encoding anything. MPEG-TS pieces are read sample by sample and written into MP4 as they are; MP4
//! fragments are read the same way, so a separate sound track can join the video. Nothing is transcoded: every
//! sample keeps its bytes, so the file has the quality the site sent.

pub mod bits;
pub mod hls;
pub mod download;
pub mod join;
pub mod mp4;
pub mod ts;

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Video,
    Audio,
}

/// One frame of video or block of sound, its bytes kept in the spool file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    /// Decode time, in the track's timescale.
    pub dts: i64,
    /// Presentation time minus decode time (B-frames).
    pub cts: i32,
    /// 0 until the next sample's time is known.
    pub duration: u32,
    /// A frame the picture can start from (a key frame); every sound sample is one.
    pub sync: bool,
    /// Where its bytes are in the spool, and how many.
    pub offset: u64,
    pub size: u32,
}

/// A track ready for MP4: its sample description and its samples in decode order.
#[derive(Clone, Debug)]
pub struct Track {
    pub kind: Kind,
    pub timescale: u32,
    /// The sample entry box (`avc1`, `hvc1`, `mp4a`…) as it goes into `stsd`, with its configuration.
    pub entry: Vec<u8>,
    /// Pixels (video) for the track header.
    pub width: u32,
    pub height: u32,
    /// ISO 639-2 code ("und" when unknown).
    pub language: String,
    pub samples: Vec<Sample>,
    /// The source's own edit list, for MP4 fragments: (seconds of nothing before it, the media time it starts at).
    /// `None`: it starts at its first sample's presentation time.
    pub edit: Option<(f64, i64)>,
}

/// Where samples' bytes wait while a stream is read, so a long video never has to fit in memory.
pub struct Spool {
    file: std::fs::File,
    len: u64,
}

impl Spool {
    pub fn create(path: &Path) -> std::io::Result<Spool> {
        let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).open(path)?;
        Ok(Spool { file, len: 0 })
    }

    /// Keeps `bytes`; returns their offset.
    pub fn put(&mut self, bytes: &[u8]) -> std::io::Result<u64> {
        let at = self.len;
        self.file.seek(SeekFrom::Start(at))?;
        self.file.write_all(bytes)?;
        self.len += bytes.len() as u64;
        Ok(at)
    }

    pub fn read(&mut self, offset: u64, size: u32, into: &mut Vec<u8>) -> std::io::Result<()> {
        into.resize(size as usize, 0);
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(into)
    }
}

/// Why a stream couldn't be joined into MP4: the pieces are then kept as one `.ts` file instead.
#[derive(Debug)]
pub enum JoinError {
    Io(std::io::Error),
    /// A codec MP4 isn't given here (AC-3, for one): the stream is saved as it came.
    Unsupported(String),
    /// The pieces aren't what they should be.
    Damaged(String),
}

impl From<std::io::Error> for JoinError {
    fn from(e: std::io::Error) -> Self {
        JoinError::Io(e)
    }
}

impl std::fmt::Display for JoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JoinError::Io(e) => write!(f, "{e}"),
            JoinError::Unsupported(what) | JoinError::Damaged(what) => write!(f, "{what}"),
        }
    }
}
