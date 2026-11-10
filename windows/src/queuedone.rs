//! The scheduler's "When the queue is done" (SPECIFICATION.md §11.1): a minute's countdown with Cancel, then the app
//! quits, or Windows sleeps or shuts down. A test copy (HD_DATA_DIR set) never sleeps or shuts down the computer.

use std::cell::Cell;

use hd_core::schedule::WhenDone;
use windows::Win32::Foundation::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

use crate::add::wide;

/// Posted to the main window with the action (`code`) once the queue is done; the countdown runs from its own
/// message, never inside another handler.
pub const WM_QUEUE_DONE: u32 = WM_APP + 22;
const SECONDS: u32 = 60;
const NOW: i32 = 1001;

thread_local! {
    /// The countdown's dialog and action while it shows.
    static DIALOG: Cell<Option<(isize, WhenDone)>> = const { Cell::new(None) };
    static LEFT: Cell<u32> = const { Cell::new(0) };
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
    let heading = wide("The queue is done");
    let body = wide(&sentence(action, SECONDS));
    let label = wide(match action {
        WhenDone::Quit => "Quit Now",
        WhenDone::Sleep => "Sleep Now",
        _ => "Shut Down Now",
    });
    let buttons = [TASKDIALOG_BUTTON { nButtonID: NOW, pszButtonText: PCWSTR(label.as_ptr()) }];
    let config = TASKDIALOGCONFIG {
        cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: main,
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | TDF_CALLBACK_TIMER,
        dwCommonButtons: TDCBF_CANCEL_BUTTON,
        pszWindowTitle: w!("Herb's Download Manager"),
        pszMainInstruction: PCWSTR(heading.as_ptr()),
        pszContent: PCWSTR(body.as_ptr()),
        cButtons: 1,
        pButtons: buttons.as_ptr(),
        // Cancel is the default: pressing Enter by chance never shuts the computer down.
        nDefaultButton: IDCANCEL.0,
        pfCallback: Some(callback),
        lpCallbackData: code as isize,
        ..Default::default()
    };
    LEFT.set(SECONDS);
    let mut pressed = 0i32;
    unsafe {
        if IsIconic(main).as_bool() {
            let _ = ShowWindow(main, SW_RESTORE);
        }
        let _ = FlashWindow(main, true);
        let _ = TaskDialogIndirect(&config, Some(&mut pressed), None, None);
    }
    DIALOG.set(None);
    if pressed == NOW {
        perform(main, action);
    }
}

unsafe extern "system" fn callback(hwnd: HWND, msg: TASKDIALOG_NOTIFICATIONS, wparam: WPARAM, _: LPARAM, data: isize) -> HRESULT {
    let action = action_of(data as usize);
    if msg == TDN_CREATED {
        DIALOG.set(Some((hwnd.0 as isize, action)));
    } else if msg == TDN_TIMER {
        // wparam: milliseconds since the dialog opened.
        let left = SECONDS.saturating_sub((wparam.0 / 1000) as u32);
        if left != LEFT.get() {
            LEFT.set(left);
            unsafe {
                if left == 0 {
                    SendMessageW(hwnd, TDM_CLICK_BUTTON.0 as u32, Some(WPARAM(NOW as usize)), None);
                } else {
                    let text = wide(&sentence(action, left));
                    SendMessageW(hwnd, TDM_SET_ELEMENT_TEXT.0 as u32, Some(WPARAM(TDE_CONTENT.0 as usize)),
                                 Some(LPARAM(text.as_ptr() as isize)));
                }
            }
        }
    }
    S_OK
}

/// Feature tests: answers the countdown for `action` with Cancel.
pub fn cancel(action: WhenDone) -> std::result::Result<(), String> {
    match DIALOG.get() {
        Some((hwnd, a)) if a == action => {
            unsafe {
                // Posted: the dialog closes from its own loop once this step has answered.
                let _ = PostMessageW(Some(HWND(hwnd as *mut _)), TDM_CLICK_BUTTON.0 as u32, WPARAM(IDCANCEL.0 as usize), LPARAM(0));
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
