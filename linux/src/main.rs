//! Herb's Download Manager for Linux (SPECIFICATION.md §1): GTK 4 + libadwaita over the shared core, the same
//! window as the Mac's with the window controls on the right and the Main Menu (☰) in the title bar's left corner.

mod window;

use adw::prelude::*;
use std::path::PathBuf;

pub const APP_ID: &str = "org.herbsdownloadmanager.app";

/// The list lives in $XDG_DATA_HOME/herbs-download-manager (inside the Flatpak, ~/.var/app/<id>/data), or in
/// $HD_DATA_DIR for tests, so they never touch the user's list.
pub fn data_folder() -> PathBuf {
    match std::env::var_os("HD_DATA_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => gtk::glib::user_data_dir().join("herbs-download-manager"),
    }
}

/// Test hook: `--capture FILE` saves a picture of the main window as PNG once it is drawn, then quits.
fn capture_path() -> Option<PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == "--capture").and_then(|i| args.get(i + 1)).map(PathBuf::from)
}

fn main() -> gtk::glib::ExitCode {
    // `--self-check` prints what the core reports and exits without a window (scripts/build.sh checks the build).
    if std::env::args().any(|a| a == "--self-check") {
        let s = hd_core::settings::Settings::default();
        println!("core {} connections {} per-server {}", hd_core::version(), s.connections, s.connections_per_server);
        return gtk::glib::ExitCode::SUCCESS;
    }
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(|app| {
        if let Some(window) = app.active_window() {
            window.present();
            return;
        }
        match hd_core::manager::Manager::open(&data_folder()) {
            Ok(manager) => {
                let window = window::build(app, manager);
                window.present();
                if let Some(path) = capture_path() {
                    window::capture_then_quit(app, &window, path);
                }
            }
            Err(e) => {
                let dialog = adw::AlertDialog::new(
                    Some("Herb's Download Manager can't open its download list"),
                    Some(&e.to_string()),
                );
                dialog.add_response("quit", "Quit");
                let holder = adw::ApplicationWindow::new(app);
                let app = app.clone();
                dialog.connect_response(None, move |_, _| app.quit());
                dialog.present(Some(&holder));
            }
        }
    });
    // GTK must not see our own flags.
    app.run_with_args(&std::env::args().take(1).collect::<Vec<_>>())
}
