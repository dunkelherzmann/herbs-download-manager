//! The Swift-facing API of hd-core, generated into Swift by UniFFI for the macOS app.
//! The Windows and Linux apps are Rust and call hd-core directly.

uniffi::setup_scaffolding!();

use hd_core::settings;

#[derive(uniffi::Enum)]
pub enum WhenFileExists {
    Ask,
    NumberedName,
    Replace,
}

#[derive(uniffi::Record)]
pub struct Settings {
    pub save_to: Option<String>,
    pub connections: u32,
    pub connections_per_server: u32,
    pub downloads_at_once: u32,
    pub speed_limit: Option<u64>,
    pub retries: u32,
    pub when_file_exists: WhenFileExists,
    pub look_for_checksum_files: bool,
    pub notify_when_finished: bool,
}

impl From<settings::Settings> for Settings {
    fn from(s: settings::Settings) -> Self {
        Settings {
            save_to: s.save_to,
            connections: s.connections,
            connections_per_server: s.connections_per_server,
            downloads_at_once: s.downloads_at_once,
            speed_limit: s.speed_limit,
            retries: s.retries,
            when_file_exists: match s.when_file_exists {
                settings::WhenFileExists::Ask => WhenFileExists::Ask,
                settings::WhenFileExists::NumberedName => WhenFileExists::NumberedName,
                settings::WhenFileExists::Replace => WhenFileExists::Replace,
            },
            look_for_checksum_files: s.look_for_checksum_files,
            notify_when_finished: s.notify_when_finished,
        }
    }
}

#[uniffi::export]
pub fn core_version() -> String {
    hd_core::version().to_string()
}

#[uniffi::export]
pub fn default_settings() -> Settings {
    settings::Settings::default().into()
}

/// The choices in the connections picker (1 to 32).
#[uniffi::export]
pub fn connection_choices() -> Vec<u32> {
    settings::CONNECTIONS.collect()
}
