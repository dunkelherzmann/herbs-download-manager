//! The scheduler's "When the queue is done" (SPECIFICATION.md §11.1): a minute's countdown with Cancel, then the app
//! quits, or Windows sleeps or shuts down. A test copy (HD_DATA_DIR set) never sleeps or shuts down the computer.

use std::cell::Cell;

use hd_core::schedule::WhenDone;
use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

/// Posted to the main window with the action (`code`) once the queue is done; the countdown runs from its own
/// message, never inside another handler.
pub const WM_QUEUE_DONE: u32 = WM_APP + 22;
const SECONDS: u32 = 60;

thread_local! {
    /// The countdown's dialog and action while it shows.
    static DIALOG: Cell<Option<(isize, WhenDone)>> = const { Cell::new(None) };
}

pub fn code(action: WhenDone) -> usize {
    match action {
        WhenDone::Nothing => 0,
        WhenDone::Quit => 1,
        WhenDone::Sleep => 2,
        WhenDone::ShutDown => 3,
    }
}

fn action_of(code: usize) -> WhenDone {
    match code {
        1 => WhenDone::Quit,
        2 => WhenDone::Sleep,
        3 => WhenDone::ShutDown,
        _ => WhenDone::Nothing,
    }
}

fn sentence(action: WhenDone, seconds: u32) -> String {
    match action {
        WhenDone::Quit => format!("Herb's Download Manager will quit in {seconds} seconds."),
        WhenDone::Sleep => format!("The computer will go to sleep in {seconds} seconds."),
        WhenDone::ShutDown => format!("The computer will shut down in {seconds} seconds."),
        WhenDone::Nothing => String::new(),
    }
}

/// The countdown (from WM_QUEUE_DONE); `code` is the action.
pub fn countdown(main: HWND, code: usize) {
    let action = action_of(code);
    if action == WhenDone::Nothing || DIALOG.get().is_some() {
        return;
    }
    let label = match action {
        WhenDone::Quit => "Quit Now",
        WhenDone::Sleep => "Sleep Now",
        _ => "Shut Down Now",
    };
    // Cancel is the default: pressing Enter by chance never shuts the computer down.
    let mut alert = crate::alert::Alert::new("The queue is done", sentence(action, SECONDS), &[label, "Cancel"], 1, Some(1));
    alert.tick = Some(Box::new(move |seconds| {
        let left = SECONDS.saturating_sub(seconds);
        if left == 0 { crate::alert::Tick::Press(0) } else { crate::alert::Tick::Body(sentence(action, left)) }
    }));
    unsafe {
        if IsIconic(main).as_bool() {
            let _ = ShowWindow(main, SW_RESTORE);
        }
        let _ = FlashWindow(main, true);
    }
    DIALOG.set(Some((0, action)));
    let pressed = crate::alert::run(main, alert).0;
    DIALOG.set(None);
    if pressed == Some(0) {
        perform(main, action);
    }
}

/// Feature tests: answers the countdown for `action` with Cancel.
pub fn cancel(action: WhenDone) -> std::result::Result<(), String> {
    match DIALOG.get() {
        Some((_, a)) if a == action => {
            // Posted: the alert closes from its own loop once this step has answered (its close box is Cancel).
            if let Some((hwnd, _)) = crate::alert::current() {
                unsafe {
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
            Ok(())
        }
        _ => Err(format!("the app isn't counting down to {}", action.word())),
    }
}

#[link(name = "powrprof")]
unsafe extern "system" {
    fn SetSuspendState(hibernate: u8, force: u8, wake_events_disabled: u8) -> u8;
}

fn perform(main: HWND, action: WhenDone) {
    if std::env::var_os("HD_DATA_DIR").is_some() && action != WhenDone::Quit {
        eprintln!("Test copy: the queue is done; the computer would {} now", action.word());
        return;
    }
    match action {
        WhenDone::Quit => unsafe {
            let _ = PostMessageW(Some(main), WM_CLOSE, WPARAM(0), LPARAM(0));
        },
        WhenDone::Sleep => unsafe {
            SetSuspendState(0, 0, 0);
        },
        WhenDone::ShutDown => {
            use std::os::windows::process::CommandExt;
            // shutdown.exe asks Windows to shut down as the Start menu does; CREATE_NO_WINDOW keeps a console away.
            let _ = std::process::Command::new("shutdown").args(["/s", "/t", "0"]).creation_flags(0x0800_0000).spawn();
        }
        WhenDone::Nothing => {}
    }
}
