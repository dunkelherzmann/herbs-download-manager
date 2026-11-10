//! The scheduler's "When the queue is done" (SPECIFICATION.md §11.1): a minute's countdown with Cancel, then the app
//! quits, or the computer sleeps or shuts down through systemd-logind. A test copy (HD_DATA_DIR set) never sleeps
//! or shuts down the computer; it only prints it.

use std::cell::RefCell;

use adw::prelude::*;
use gtk::{gio, glib};
use hd_core::manager::Manager;
use hd_core::schedule::WhenDone;

const SECONDS: u32 = 60;

thread_local! {
    static PENDING: RefCell<Option<(WhenDone, adw::AlertDialog)>> = const { RefCell::new(None) };
}

fn sentence(action: WhenDone, seconds: u32) -> String {
    match action {
        WhenDone::Quit => format!("Herb's Download Manager will quit in {seconds} seconds."),
        WhenDone::Sleep => format!("The computer will go to sleep in {seconds} seconds."),
        WhenDone::ShutDown => format!("The computer will shut down in {seconds} seconds."),
        WhenDone::Nothing => String::new(),
    }
}

/// On every refresh: starts the countdown once the core says the queue is done.
pub fn check(manager: &Manager, window: &adw::ApplicationWindow) {
    if PENDING.with(|p| p.borrow().is_some()) {
        return;
    }
    let Some(action) = manager.take_queue_done() else { return };
    let dialog = adw::AlertDialog::new(Some("The queue is done"), Some(&sentence(action, SECONDS)));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("now", match action {
        WhenDone::Quit => "Quit Now",
        WhenDone::Sleep => "Sleep Now",
        _ => "Shut Down Now",
    });
    dialog.set_response_appearance("now", adw::ResponseAppearance::Destructive);
    // Cancel is the default: pressing Enter by chance never shuts the computer down.
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let left = std::rc::Rc::new(std::cell::Cell::new(SECONDS));
    let timer = {
        let (dialog, left) = (dialog.clone(), left.clone());
        glib::timeout_add_seconds_local(1, move || {
            left.set(left.get() - 1);
            if left.get() == 0 {
                dialog.close();
                glib::ControlFlow::Break
            } else {
                dialog.set_body(&sentence(action, left.get()));
                glib::ControlFlow::Continue
            }
        })
    };
    let timer = std::rc::Rc::new(RefCell::new(Some(timer)));
    {
        let window = window.clone();
        dialog.connect_response(None, move |_, response| {
            PENDING.with(|p| *p.borrow_mut() = None);
            if let Some(t) = timer.borrow_mut().take()
                && left.get() > 0
            {
                t.remove();
            }
            // The countdown ran out (closing the dialog answers "cancel"), or Now was chosen.
            if response == "now" || left.get() == 0 {
                perform(action, &window);
            }
        });
    }
    window.present();
    dialog.present(Some(window));
    PENDING.with(|p| *p.borrow_mut() = Some((action, dialog)));
}

/// Feature tests: answers the countdown for `action` with Cancel.
pub fn cancel(action: WhenDone) -> Result<(), String> {
    let dialog = PENDING.with(|p| match p.borrow().as_ref() {
        Some((a, d)) if *a == action => Ok(d.clone()),
        _ => Err(format!("the app isn't counting down to {}", action.word())),
    })?;
    dialog.close();
    Ok(())
}

fn perform(action: WhenDone, window: &adw::ApplicationWindow) {
    if std::env::var_os("HD_DATA_DIR").is_some() && action != WhenDone::Quit {
        eprintln!("Test copy: the queue is done; the computer would {} now", action.word());
        return;
    }
    let method = match action {
        WhenDone::Quit => return window.close(),
        WhenDone::Sleep => "Suspend",
        WhenDone::ShutDown => "PowerOff",
        WhenDone::Nothing => return,
    };
    // org.freedesktop.login1 is allowed in the Flatpak's manifest; `false` means don't ask for a password.
    let call = gio::bus_get_sync(gio::BusType::System, None::<&gio::Cancellable>).and_then(|bus| {
        bus.call_sync(Some("org.freedesktop.login1"), "/org/freedesktop/login1", "org.freedesktop.login1.Manager", method,
                      Some(&(false,).to_variant()), None, gio::DBusCallFlags::NONE, -1, None::<&gio::Cancellable>)
    });
    if let Err(e) = call {
        eprintln!("Couldn't {} the computer: {e}", action.word());
    }
}
