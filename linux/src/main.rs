//! Herb's Download Manager for Linux (SPECIFICATION.md §1): GTK 4 + libadwaita over the shared core, the same
//! window as the Mac's with the window controls on the right and the Main Menu (☰) in the title bar's left corner.

mod add;
mod detail;
mod features;
mod list;
mod settings;
mod signins;
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

/// S1 when unset: the Downloads folder ($HD_DOWNLOADS_DIR in tests).
pub fn downloads_folder() -> PathBuf {
    match std::env::var_os("HD_DOWNLOADS_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => gtk::glib::user_special_dir(gtk::glib::UserDirectory::Downloads)
            .unwrap_or_else(|| gtk::glib::home_dir().join("Downloads")),
    }
}

/// Every control explains itself: a name for screen readers and hover text (what it is, what it does). GTK uses
/// hover text as the name when there's none, and then reads no description, so both are set.
pub fn explain(widget: &impl gtk::prelude::IsA<gtk::Widget>, name: &str, help: &str) {
    use gtk::prelude::{AccessibleExtManual, Cast, EditableExt, WidgetExt};
    let widget = widget.upcast_ref::<gtk::Widget>();
    // A "labelled by" or "described by" relation outranks the name and description set here, so it goes first.
    let set = |w: &gtk::Widget| {
        w.reset_relation(gtk::AccessibleRelation::LabelledBy);
        w.reset_relation(gtk::AccessibleRelation::DescribedBy);
        w.update_property(&[gtk::accessible::Property::Label(name), gtk::accessible::Property::Description(help)]);
        w.set_tooltip_text(Some(help));
    };
    set(widget);
    // An entry's accessible part is the text widget inside it.
    if let Some(inner) = widget.dynamic_cast_ref::<gtk::Editable>().and_then(|e| e.delegate()) {
        set(inner.upcast_ref());
    }
}

/// Hover text for a libadwaita row and the switch or control inside it, which doesn't inherit the row's.
pub fn explain_row(row: &impl gtk::prelude::IsA<gtk::Widget>, help: &str) {
    use gtk::prelude::{AccessibleExtManual, Cast, WidgetExt};
    let row = row.upcast_ref::<gtk::Widget>();
    let mut stack = vec![row.clone()];
    while let Some(w) = stack.pop() {
        if &w == row || w.is::<gtk::Switch>() || w.is::<gtk::CheckButton>() {
            w.reset_relation(gtk::AccessibleRelation::DescribedBy);
            w.update_property(&[gtk::accessible::Property::Description(help)]);
            w.set_tooltip_text(Some(help));
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            child = c.next_sibling();
            stack.push(c);
        }
    }
}

/// Where new downloads go: S1, or when it's unset or its folder is gone, the Downloads folder.
pub fn save_folder(manager: &hd_core::manager::Manager) -> PathBuf {
    manager.settings().save_to.map(PathBuf::from).filter(|p| p.is_dir()).unwrap_or_else(downloads_folder)
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
                // A test copy never fills the desktop's Trash.
                if std::env::var_os("HD_DATA_DIR").is_some() {
                    manager.use_trash_folder(&data_folder().join("Trash"));
                }
                // Test hook: `--add URL` (repeatable) adds and starts a download, as on the Mac.
                let args: Vec<String> = std::env::args().collect();
                for (i, a) in args.iter().enumerate() {
                    if a == "--add" && let Some(url) = args.get(i + 1) {
                        let s = manager.settings();
                        manager.add(hd_core::manager::NewDownload {
                            url: url.clone(),
                            folder: save_folder(&manager),
                            connections: s.connections,
                            connections_per_server: s.connections_per_server,
                            retries: s.retries,
                            start: true,
                            ..Default::default()
                        });
                    }
                }
                // The "Download finished" notification's Open and Show in Folder buttons.
                for (name, show) in [("open-file", false), ("show-file", true)] {
                    let action = gtk::gio::SimpleAction::new(name, Some(gtk::glib::VariantTy::UINT64));
                    let manager = manager.clone();
                    let app2 = app.clone();
                    action.connect_activate(move |_, id| {
                        let id = id.and_then(|v| v.get::<u64>());
                        // The file may have been renamed or moved since: look it up now.
                        let Some(path) = manager.rows().into_iter().find(|r| Some(r.id) == id).and_then(|r| r.path).filter(|p| p.exists()) else { return };
                        let file = gtk::gio::File::for_path(&path);
                        if show {
                            gtk::FileLauncher::new(Some(&file)).open_containing_folder(app2.active_window().as_ref(), None::<&gtk::gio::Cancellable>, |_| {});
                        } else {
                            let _ = gtk::gio::AppInfo::launch_default_for_uri(&file.uri(), None::<&gtk::gio::AppLaunchContext>);
                        }
                    });
                    app.add_action(&action);
                }
                let window = window::build(app, manager.clone());
                features::run_if_asked(app, &manager);
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
