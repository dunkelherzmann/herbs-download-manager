//! Keep on Top (§3 T11, View ▸ Keep on Top, Ctrl+Super+T: Ubuntu's Ctrl+Alt+T opens a terminal). GNOME on Wayland lets no app keep its own windows above
//! others, so the app installs a small GNOME Shell extension of its own (`linux/gnome-extension/`) and asks it over
//! the session bus. GNOME loads a newly installed extension only at the next login: until then Keep on Top stays
//! on as chosen and says to log out and back in once; at that login's first launch the app turns the helper on.
//! Remembered in the data folder, as on Windows (a test copy with $HD_DATA_DIR keeps its own).

use std::path::PathBuf;
use std::time::Duration;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;

pub const UUID: &str = "keep-on-top@herbsdownloadmanager.org";
const BUS_NAME: &str = "org.herbsdownloadmanager.KeepOnTop";
const OBJECT_PATH: &str = "/org/herbsdownloadmanager/KeepOnTop";
const SHELL: &str = "org.gnome.Shell.Extensions";
const SHELL_PATH: &str = "/org/gnome/Shell/Extensions";
const FILES: [(&str, &str); 2] = [
    ("metadata.json", include_str!("../gnome-extension/keep-on-top@herbsdownloadmanager.org/metadata.json")),
    ("extension.js", include_str!("../gnome-extension/keep-on-top@herbsdownloadmanager.org/extension.js")),
];

/// What turning it on or off came to.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Done,
    /// The helper is installed but GNOME loads it only at the next login.
    NeedsLogin,
    /// No GNOME Shell on this desktop.
    NotGnome,
    Failed(String),
}

fn remembered_file() -> PathBuf {
    crate::data_folder().join("keep-on-top")
}

pub fn remembered() -> bool {
    remembered_file().exists()
}

pub fn remember(on: bool) {
    if on {
        let _ = std::fs::create_dir_all(crate::data_folder());
        let _ = std::fs::write(remembered_file(), b"");
    } else {
        let _ = std::fs::remove_file(remembered_file());
    }
}

/// Where GNOME looks for a user's extensions (the Flatpak sees the real home folder).
fn helper_folder() -> PathBuf {
    glib::home_dir().join(".local/share/gnome-shell/extensions").join(UUID)
}

/// Writes the helper's files where they differ from this build's.
fn install_helper() -> std::io::Result<()> {
    let dir = helper_folder();
    std::fs::create_dir_all(&dir)?;
    for (name, text) in FILES {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text) {
            std::fs::write(&path, text)?;
        }
    }
    Ok(())
}

fn call(bus: &gio::DBusConnection, name: &str, path: &str, iface: &str, method: &str, args: &glib::Variant) -> Result<glib::Variant, glib::Error> {
    bus.call_sync(Some(name), path, iface, method, Some(args), None, gio::DBusCallFlags::NONE, 2000, gio::Cancellable::NONE)
}

fn set_above(bus: &gio::DBusConnection, on: bool) -> Result<(), glib::Error> {
    call(bus, BUS_NAME, OBJECT_PATH, BUS_NAME, "SetAbove", &(crate::app_id(), on).to_variant()).map(|_| ())
}

/// Keeps this copy's windows above others (or stops), installing and turning on the helper when needed. `done`
/// runs on the main loop once it's known how it went.
pub fn apply(on: bool, done: impl FnOnce(Outcome) + 'static) {
    let bus = match gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) {
        Ok(bus) => bus,
        Err(e) => return done(Outcome::Failed(e.message().to_string())),
    };
    if set_above(&bus, on).is_ok() {
        // Keep the helper's files current; a newer one runs from the next login.
        let _ = install_helper();
        return done(Outcome::Done);
    }
    if !on {
        return done(Outcome::Done);
    }
    let info = match call(&bus, SHELL, SHELL_PATH, SHELL, "GetExtensionInfo", &(UUID,).to_variant()) {
        Ok(info) => info,
        Err(_) => return done(Outcome::NotGnome),
    };
    if let Err(e) = install_helper() {
        return done(Outcome::Failed(format!("The GNOME helper couldn't be installed: {e}.")));
    }
    let info = glib::VariantDict::new(Some(&info.child_value(0)));
    // GNOME only knows extensions it found at login (an empty answer): this one loads at the next.
    let Some(state) = info.lookup::<f64>("state").ok().flatten() else { return done(Outcome::NeedsLogin) };
    // ExtensionState: 3 error, 4 out of date for this GNOME.
    if state == 3.0 || state == 4.0 {
        let why = info.lookup::<String>("error").ok().flatten().filter(|e| !e.is_empty());
        return done(Outcome::Failed(match why {
            Some(e) => format!("GNOME couldn't run the Keep on Top helper: {e}."),
            None if state == 4.0 => "GNOME says the Keep on Top helper isn't made for this GNOME version.".into(),
            None => "GNOME couldn't run the Keep on Top helper.".into(),
        }));
    }
    let _ = call(&bus, SHELL, SHELL_PATH, SHELL, "EnableExtension", &(UUID,).to_variant());
    // The helper takes its name on the bus just after it's turned on: ask again for up to 3 seconds.
    let mut tries = 0;
    let mut done = Some(done);
    glib::timeout_add_local(Duration::from_millis(150), move || {
        tries += 1;
        if set_above(&bus, true).is_ok() {
            done.take().unwrap()(Outcome::Done);
            return glib::ControlFlow::Break;
        }
        if tries >= 20 {
            done.take().unwrap()(Outcome::Failed("The GNOME helper for Keep on Top didn't answer.".into()));
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}

/// The message for an outcome that needs the user to know or do something; None when it just worked.
pub fn message(outcome: &Outcome) -> Option<(&'static str, String)> {
    match outcome {
        Outcome::Done => None,
        Outcome::NeedsLogin => Some((
            "Log out and back in once to finish Keep on Top",
            "GNOME doesn't let an app keep itself above other windows, so Herb's DM has added a small GNOME helper \
             that does. GNOME starts it at your next login; Keep on Top stays on and works from then on."
                .into(),
        )),
        Outcome::NotGnome => Some((
            "Keep on Top needs GNOME",
            "This desktop doesn't let an app keep itself above other windows. Its title bar's menu may have Always \
             on Top."
                .into(),
        )),
        Outcome::Failed(why) => Some(("Keep on Top didn't turn on", why.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_helper_s_files_name_this_uuid_and_are_installed_where_gnome_looks() {
        assert!(FILES[0].1.contains(&format!("\"uuid\": \"{UUID}\"")));
        assert!(FILES[1].1.contains(BUS_NAME) && FILES[1].1.contains(OBJECT_PATH));
        assert!(helper_folder().ends_with(format!(".local/share/gnome-shell/extensions/{UUID}")));
    }

    #[test]
    fn only_a_plain_success_says_nothing() {
        assert_eq!(message(&Outcome::Done), None);
        assert!(message(&Outcome::NeedsLogin).unwrap().0.contains("Log out"));
        assert!(message(&Outcome::Failed("x".into())).unwrap().1 == "x");
    }
}
