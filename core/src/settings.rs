//! Settings S1–S9 (SPECIFICATION.md §9): defaults and allowed ranges.

use std::ops::RangeInclusive;

/// Connections a download may use (S2, and the New Download window's picker, A5).
pub const CONNECTIONS: RangeInclusive<u32> = 1..=32;
/// Connections to any one server (S3).
pub const CONNECTIONS_PER_SERVER: RangeInclusive<u32> = 1..=32;
/// Downloads the queue runs at once (S4).
pub const DOWNLOADS_AT_ONCE: RangeInclusive<u32> = 1..=10;
/// Retries per source (S6).
pub const RETRIES: RangeInclusive<u32> = 0..=20;

/// What happens when the destination file already exists (S7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WhenFileExists {
    Ask,
    NumberedName,
    Replace,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
}
