//! Every control explains itself (§3, §4): hover text through one tooltip control per window, and a name for screen
//! readers through Windows' accessibility service (for glyph buttons, whose text is an icon character, and fields
//! without a label before them). `--a11y-report FILE` checks every control of every window of the app.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, Interface, PCWSTR, PWSTR};

thread_local! {
    static TIPS: RefCell<HashMap<isize, HWND>> = RefCell::new(HashMap::new());
    static PROPS: RefCell<Option<IAccPropServices>> = const { RefCell::new(None) };
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The tooltip control of `control`'s top-level window, made on first use.
fn tooltips_for(control: HWND) -> HWND {
    unsafe {
        let top = GetAncestor(control, GA_ROOT);
        if let Some(t) = TIPS.with(|t| t.borrow().get(&(top.0 as isize)).copied())
            && IsWindow(Some(t)).as_bool()
        {
            return t;
        }
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let t = CreateWindowExW(WS_EX_TOPMOST, TOOLTIPS_CLASSW, PCWSTR::null(), WINDOW_STYLE(TTS_ALWAYSTIP),
                                CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, Some(top), None, Some(instance.into()), None)
            .unwrap_or_default();
        // Long hover texts wrap instead of running off the screen.
        SendMessageW(t, TTM_SETMAXTIPWIDTH, None, Some(LPARAM(420)));
        TIPS.with(|m| m.borrow_mut().insert(top.0 as isize, t));
        t
    }
}

/// Hover text for a control.
pub fn tip(control: HWND, text: &str) {
    unsafe {
        let tips = tooltips_for(control);
        let text = wide(text);
        let info = TTTOOLINFOW {
            cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
            uFlags: TTF_IDISHWND | TTF_SUBCLASS,
            hwnd: GetParent(control).unwrap_or_default(),
            uId: control.0 as usize,
            lpszText: PWSTR(text.as_ptr() as *mut _),
            ..Default::default()
        };
        // Replace an earlier text for the same control.
        SendMessageW(tips, TTM_DELTOOLW, None, Some(LPARAM(&info as *const _ as isize)));
        SendMessageW(tips, TTM_ADDTOOLW, None, Some(LPARAM(&info as *const _ as isize)));
    }
}

/// The name screen readers say for a control (instead of its text, for a glyph button).
pub fn name(control: HWND, text: &str) {
    unsafe {
        let props = PROPS.with(|p| {
            if p.borrow().is_none() {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                *p.borrow_mut() = CoCreateInstance(&CLSID_AccPropServices, None, CLSCTX_INPROC_SERVER).ok();
            }
            p.borrow().clone()
        });
        if let Some(props) = props {
            let text = wide(text);
            let _ = props.SetHwndPropStr(control, OBJID_CLIENT.0 as u32, CHILDID_SELF, PROPID_ACC_NAME, PCWSTR(text.as_ptr()));
        }
    }
}

/// Both: what screen readers say, and the hover text.
pub fn explain(control: HWND, label: &str, help: &str) {
    name(control, label);
    tip(control, help);
}

// --- The audit -----------------------------------------------------------------------------------------------------

fn class_of(h: HWND) -> String {
    let mut buffer = [0u16; 64];
    let n = unsafe { GetClassNameW(h, &mut buffer) };
    String::from_utf16_lossy(&buffer[..n.max(0) as usize])
}

fn text_of(h: HWND) -> String {
    let mut buffer = [0u16; 256];
    let n = unsafe { GetWindowTextW(h, &mut buffer) };
    String::from_utf16_lossy(&buffer[..n.max(0) as usize])
}

/// What a screen reader would say for the control (its accessible name).
fn accessible_name(h: HWND) -> String {
    unsafe {
        let mut object: Option<IAccessible> = None;
        if AccessibleObjectFromWindow(h, OBJID_CLIENT.0 as u32, &IAccessible::IID, &mut object as *mut _ as *mut _).is_err() {
            return String::new();
        }
        object.and_then(|o| o.get_accName(&VARIANT::from(CHILDID_SELF as i32)).ok()).map(|b| b.to_string()).unwrap_or_default()
    }
}

fn has_tip(h: HWND) -> bool {
    unsafe {
        let top = GetAncestor(h, GA_ROOT);
        let mut tips = vec![];
        if let Some(t) = TIPS.with(|t| t.borrow().get(&(top.0 as isize)).copied()) {
            tips.push(t);
        }
        tips.extend(crate::app::main_tooltips());
        tips.iter().any(|&t| {
            let mut info = TTTOOLINFOW {
                cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
                hwnd: GetParent(h).unwrap_or_default(),
                uId: h.0 as usize,
                ..Default::default()
            };
            SendMessageW(t, TTM_GETTOOLINFOW, None, Some(LPARAM(&mut info as *mut _ as isize))).0 != 0
        })
    }
}

/// One line per interactive control without hover text or a name a screen reader can say, in every visible window
/// of this thread; the first line lists the windows checked.
pub fn report() -> Vec<String> {
    let mut windows: Vec<HWND> = Vec::new();
    unsafe extern "system" fn top(h: HWND, data: LPARAM) -> BOOL {
        // A program started over ssh gets its first window hidden (its startup info says so), so windows are taken by
        // their own WS_VISIBLE flag where they have one, and by having a title: tooltips and menus have none.
        let list = unsafe { &mut *(data.0 as *mut Vec<HWND>) };
        if !text_of(h).is_empty() && class_of(h) != "tooltips_class32" {
            list.push(h);
        }
        TRUE
    }
    unsafe {
        let _ = EnumThreadWindows(windows::Win32::System::Threading::GetCurrentThreadId(), Some(top), LPARAM(&mut windows as *mut _ as isize));
    }
    let mut out = vec![format!("# checked: {}", windows.iter().map(|&h| text_of(h)).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(", "))];
    for &window in &windows {
        let mut children: Vec<HWND> = Vec::new();
        unsafe extern "system" fn child(h: HWND, data: LPARAM) -> BOOL {
            unsafe { (*(data.0 as *mut Vec<HWND>)).push(h) };
            TRUE
        }
        unsafe {
            let _ = EnumChildWindows(Some(window), Some(child), LPARAM(&mut children as *mut _ as isize));
        }
        for h in children {
            let class = class_of(h).to_ascii_lowercase();
            let style = unsafe { GetWindowLongW(h, GWL_STYLE) } as u32;
            if style & WS_VISIBLE.0 == 0 {
                continue;
            }
            let interactive = match class.as_str() {
                // Group boxes, separators and owner-drawn bars (style 0x7, 0x10, 0xD on a static) aren't controls.
                "button" => style & 0xF != 0x7,
                "edit" => style & ES_READONLY as u32 == 0,
                "combobox" | "syslistview32" | "systreeview32" | "systabcontrol32" => true,
                _ => false,
            };
            if !interactive || unsafe { GetParent(h) }.ok() != Some(window) && class != "edit" {
                continue;
            }
            let name = accessible_name(h);
            let glyph = name.chars().all(|c| ('\u{E000}'..='\u{F8FF}').contains(&c));
            let what = format!("{} | {} “{}”", text_of(window), class, text_of(h));
            if name.trim().is_empty() || glyph {
                out.push(format!("{what} | no name a screen reader can say"));
            }
            if !has_tip(h) {
                out.push(format!("{what} | no hover text"));
            }
        }
    }
    out
}
