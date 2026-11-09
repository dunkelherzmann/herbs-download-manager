//! Text the list shows, the same on every platform (SPECIFICATION.md §2.1, §2.2).

use crate::manager::Status;

/// The fixed file-type groups under each sidebar filter (§2.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileKind {
    Compressed,
    Documents,
    Music,
    Pictures,
    Video,
    Programs,
    Other,
}

impl FileKind {
    pub const ALL: [FileKind; 7] = [
        FileKind::Compressed,
        FileKind::Documents,
        FileKind::Music,
        FileKind::Pictures,
        FileKind::Video,
        FileKind::Programs,
        FileKind::Other,
    ];

    pub fn of(name: &str) -> FileKind {
        let lower = name.to_ascii_lowercase();
        let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        // Two-part endings first.
        if lower.ends_with(".tar.gz") || lower.ends_with(".tar.xz") || lower.ends_with(".tar.bz2") {
            return FileKind::Compressed;
        }
        match ext {
            "zip" | "rar" | "7z" | "gz" | "tgz" | "xz" | "bz2" | "zst" | "tar" | "lz" | "lzma" | "cab" | "z" => {
                FileKind::Compressed
            }
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp" | "txt" | "rtf" | "md"
            | "csv" | "epub" | "pages" | "numbers" | "key" => FileKind::Documents,
            "mp3" | "flac" | "wav" | "aac" | "m4a" | "ogg" | "opus" | "wma" | "aiff" | "alac" => FileKind::Music,
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "heic" | "tif" | "tiff" | "bmp" | "svg" | "raw" | "psd" => {
                FileKind::Pictures
            }
            "mp4" | "mkv" | "mov" | "avi" | "webm" | "m4v" | "wmv" | "flv" | "mpg" | "mpeg" | "ts" => FileKind::Video,
            "dmg" | "pkg" | "app" | "exe" | "msi" | "msix" | "deb" | "rpm" | "appimage" | "flatpak" | "apk" | "iso"
            | "img" | "jar" => FileKind::Programs,
            _ => FileKind::Other,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            FileKind::Compressed => "Compressed",
            FileKind::Documents => "Documents",
            FileKind::Music => "Music",
            FileKind::Pictures => "Pictures",
            FileKind::Video => "Video",
            FileKind::Programs => "Programs",
            FileKind::Other => "Other",
        }
    }
}

/// The sidebar's status filters (§2.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Filter {
    All,
    Downloading,
    Paused,
    Finished,
    Unfinished,
}

impl Filter {
    pub const ALL: [Filter; 5] = [Filter::All, Filter::Downloading, Filter::Paused, Filter::Finished, Filter::Unfinished];

    pub fn title(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Downloading => "Downloading",
            Filter::Paused => "Paused",
            Filter::Finished => "Finished",
            Filter::Unfinished => "Unfinished",
        }
    }

    pub fn matches(self, status: &Status) -> bool {
        match self {
            Filter::All => true,
            Filter::Downloading => status.is_running(),
            Filter::Paused => matches!(status, Status::Paused | Status::Waiting),
            Filter::Finished => *status == Status::Finished,
            Filter::Unfinished => *status != Status::Finished,
        }
    }
}

/// L2: Waiting, Connecting, Downloading 47%, Finishing…, Paused 47%, Finished, Failed (reason).
pub fn status_text(status: &Status, fraction: Option<f64>) -> String {
    let pct = |word: &str| match fraction {
        Some(f) => format!("{word} {}%", (f * 100.0).floor() as u32),
        None => word.to_string(),
    };
    match status {
        Status::Waiting => "Waiting".into(),
        Status::Connecting => "Connecting".into(),
        Status::Downloading => pct("Downloading"),
        Status::Finishing => "Finishing…".into(),
        Status::Paused => pct("Paused"),
        Status::Finished => "Finished".into(),
        Status::Failed(reason) => format!("Failed ({reason})"),
    }
}

/// "1.2 GB", "340 KB", "12 bytes" (decimal units, as Finder shows them).
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if n < 1000 {
        return if n == 1 { "1 byte".into() } else { format!("{n} bytes") };
    }
    let mut value = n as f64 / 1000.0;
    let mut unit = 0;
    while value >= 999.95 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if value >= 100.0 {
        format!("{} {}", value.round() as u64, UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// L3: "1.2 GB / 4.0 GB", "1.2 GB / Unknown"; a finished download shows only its size.
pub fn size_text(downloaded: u64, size: Option<u64>, finished: bool) -> String {
    match size {
        Some(s) if finished => bytes(s),
        Some(s) => format!("{} / {}", bytes(downloaded), bytes(s)),
        None => format!("{} / Unknown", bytes(downloaded)),
    }
}

/// L4: "2.4 MB/s"; empty when 0.
pub fn speed_text(speed: u64) -> String {
    if speed == 0 { String::new() } else { format!("{}/s", bytes(speed)) }
}

/// L5: "45 s", "12 min", "1 h 05 min", "3 d 4 h".
pub fn time_left_text(seconds: Option<u64>) -> String {
    let Some(s) = seconds else { return String::new() };
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min", s.div_ceil(60).min(59)),
        3600..86400 => format!("{} h {:02} min", s / 3600, (s % 3600) / 60),
        _ => format!("{} d {} h", s / 86400, (s % 86400) / 3600),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_files_into_groups() {
        assert_eq!(FileKind::of("a.ZIP"), FileKind::Compressed);
        assert_eq!(FileKind::of("linux.tar.gz"), FileKind::Compressed);
        assert_eq!(FileKind::of("Report.pdf"), FileKind::Documents);
        assert_eq!(FileKind::of("song.flac"), FileKind::Music);
        assert_eq!(FileKind::of("cat.heic"), FileKind::Pictures);
        assert_eq!(FileKind::of("movie.mkv"), FileKind::Video);
        assert_eq!(FileKind::of("Setup.exe"), FileKind::Programs);
        assert_eq!(FileKind::of("download"), FileKind::Other);
    }

    #[test]
    fn writes_sizes_speeds_and_times() {
        assert_eq!(bytes(12), "12 bytes");
        assert_eq!(bytes(1_234), "1.2 KB");
        assert_eq!(bytes(340_000), "340 KB");
        assert_eq!(bytes(1_200_000_000), "1.2 GB");
        assert_eq!(bytes(999_999), "1.0 MB");
        assert_eq!(size_text(1_200_000_000, Some(4_000_000_000), false), "1.2 GB / 4.0 GB");
        assert_eq!(size_text(5_000, None, false), "5.0 KB / Unknown");
        assert_eq!(size_text(4_000, Some(4_000), true), "4.0 KB");
        assert_eq!(speed_text(2_400_000), "2.4 MB/s");
        assert_eq!(speed_text(0), "");
        assert_eq!(time_left_text(Some(45)), "45 s");
        assert_eq!(time_left_text(Some(700)), "12 min");
        assert_eq!(time_left_text(Some(3900)), "1 h 05 min");
        assert_eq!(time_left_text(Some(277_200)), "3 d 5 h");
    }

    #[test]
    fn writes_status() {
        assert_eq!(status_text(&Status::Downloading, Some(0.479)), "Downloading 47%");
        assert_eq!(status_text(&Status::Finishing, Some(1.0)), "Finishing…");
        assert!(Filter::Downloading.matches(&Status::Finishing));
        assert_eq!(status_text(&Status::Paused, None), "Paused");
        assert_eq!(status_text(&Status::Failed("404 Not Found".into()), None), "Failed (404 Not Found)");
        assert!(Filter::Paused.matches(&Status::Waiting));
        assert!(Filter::Unfinished.matches(&Status::Failed("x".into())));
        assert!(!Filter::Unfinished.matches(&Status::Finished));
    }
}
