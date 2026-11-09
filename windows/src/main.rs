//! Herb's Download Manager for Windows (SPECIFICATION.md §1): a Win32 window over the shared core, laid out as the
//! Mac's, with the window controls on the right and the Main Menu (☰) in the top-left corner.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod add;
#[cfg(windows)]
mod app;
#[cfg(windows)]
mod detail;

use std::path::PathBuf;

/// The list lives in %LOCALAPPDATA%\Herb's Download Manager, or in %HD_DATA_DIR% for tests, so they never touch the
/// user's list.
pub fn data_folder() -> PathBuf {
    match std::env::var_os("HD_DATA_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("Herb's Download Manager"),
    }
}

/// S1 when unset: the Downloads folder (%HD_DOWNLOADS_DIR% in tests).
pub fn downloads_folder() -> PathBuf {
    match std::env::var_os("HD_DOWNLOADS_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default()).join("Downloads"),
    }
}

/// Where new downloads go: S1, or when it's unset or its folder is gone, the Downloads folder.
pub fn save_folder(manager: &hd_core::manager::Manager) -> PathBuf {
    manager.settings().save_to.map(PathBuf::from).filter(|p| p.is_dir()).unwrap_or_else(downloads_folder)
}

fn main() {
    // `--self-check` prints what the core reports and exits without a window (the build script checks the build).
    if std::env::args().any(|a| a == "--self-check") {
        let s = hd_core::settings::Settings::default();
        println!("core {} connections {} per-server {}", hd_core::version(), s.connections, s.connections_per_server);
        return;
    }
    #[cfg(windows)]
    app::run();
}
