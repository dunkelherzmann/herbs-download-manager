//! Settings S1–S9 (SPECIFICATION.md §9): defaults and allowed ranges.

use std::ops::RangeInclusive;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Connections a download may use (S2, and the New Download window's picker, A5).
pub const CONNECTIONS: RangeInclusive<u32> = 1..=32;
/// Connections to any one server (S3).
pub const CONNECTIONS_PER_SERVER: RangeInclusive<u32> = 1..=32;
/// Downloads the queue runs at once (S4).
pub const DOWNLOADS_AT_ONCE: RangeInclusive<u32> = 1..=10;
/// Retries per source (S6).
pub const RETRIES: RangeInclusive<u32> = 0..=20;

/// What happens when a different file already has the new download's name (S7, §5.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WhenFileExists {
    /// Skip, Replace or Keep Both, asked each time.
    Ask,
    /// The new download is named "<name> - kept.<ext>".
    KeepBoth,
    /// The new link is not added.
    Skip,
    /// The existing file goes to the Trash and its download leaves the list.
    Replace,
}

/// Kept in `settings.json` in the app's data folder. A missing or unknown value takes its default.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// S1; `None` means the user's Downloads folder.
    pub save_to: Option<String>,
    pub connections: u32,
    pub connections_per_server: u32,
    pub downloads_at_once: u32,
    /// S5, in bytes per second; `None` is no limit.
    pub speed_limit: Option<u64>,
    pub retries: u32,
    pub when_file_exists: WhenFileExists,
    pub look_for_checksum_files: bool,
    pub notify_when_finished: bool,
    /// S12: a finished file gets the date the server gives it (`Last-Modified`) instead of the time it finished.
    pub keep_server_date: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            save_to: None,
            connections: 32,
            connections_per_server: 32,
            downloads_at_once: 3,
            speed_limit: None,
            retries: 5,
            when_file_exists: WhenFileExists::Ask,
            look_for_checksum_files: true,
            notify_when_finished: true,
            keep_server_date: false,
        }
    }
}

impl Settings {
    /// Brings every number into its allowed range, so a hand-edited settings file can't break a download.
    pub fn clamped(mut self) -> Self {
        let clamp = |v: u32, r: &RangeInclusive<u32>| v.clamp(*r.start(), *r.end());
        self.connections = clamp(self.connections, &CONNECTIONS);
        self.connections_per_server = clamp(self.connections_per_server, &CONNECTIONS_PER_SERVER);
        self.downloads_at_once = clamp(self.downloads_at_once, &DOWNLOADS_AT_ONCE);
        self.retries = clamp(self.retries, &RETRIES);
        if self.speed_limit == Some(0) {
            self.speed_limit = None;
        }
        self
    }

    /// Reads `settings.json` in `dir`; `None` when there is none yet.
    pub fn load(dir: &Path) -> Option<Settings> {
        let bytes = std::fs::read(dir.join("settings.json")).ok()?;
        serde_json::from_slice::<Settings>(&bytes)
            .ok()
            .map(Settings::clamped)
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let tmp = dir.join("settings.json.tmp");
        std::fs::write(
            &tmp,
            serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?,
        )?;
        std::fs::rename(tmp, dir.join("settings.json"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_the_spec() {
        let s = Settings::default();
        assert_eq!((s.connections, s.connections_per_server, s.downloads_at_once, s.retries), (32, 32, 3, 5));
        assert_eq!(s.when_file_exists, WhenFileExists::Ask);
        assert!(s.look_for_checksum_files && s.notify_when_finished);
        assert_eq!(s.clone().clamped(), s);
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let s = Settings { connections: 0, connections_per_server: 99, downloads_at_once: 50, retries: 100, speed_limit: Some(0), ..Settings::default() }
            .clamped();
        assert_eq!((s.connections, s.connections_per_server, s.downloads_at_once, s.retries), (1, 32, 10, 20));
        assert_eq!(s.speed_limit, None);
    }

    #[test]
    fn saved_settings_come_back_and_missing_ones_take_defaults() {
        let dir = std::env::temp_dir().join(format!("hd-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(Settings::load(&dir), None);
        let s = Settings {
            retries: 7,
            speed_limit: Some(500_000),
            when_file_exists: WhenFileExists::KeepBoth,
            ..Settings::default()
        };
        s.save(&dir).unwrap();
        assert_eq!(Settings::load(&dir), Some(s));
        std::fs::write(
            dir.join("settings.json"),
            r#"{"retries": 99, "when_file_exists": "skip"}"#,
        )
        .unwrap();
        let s = Settings::load(&dir).unwrap();
        assert_eq!(
            (s.retries, s.when_file_exists, s.connections),
            (20, WhenFileExists::Skip, 32)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
