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
        crate::theme::control(t);
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

/// Holds back the hover texts of `control`'s window while `on` (the list's cells show their own), and lets them show
/// again after.
pub fn pause(control: HWND, on: bool) {
    unsafe {
        SendMessageW(tooltips_for(control), TTM_ACTIVATE, Some(WPARAM(!on as usize)), None);
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

thread_local! {
    /// Each list with cell hover texts: its own tooltip control and the cell (row, column) its one tool is on.
    static CELL_TIPS: RefCell<HashMap<isize, (HWND, Option<(i32, i32)>)>> = RefCell::new(HashMap::new());
}

/// Every cell of a report-view list shows its own text as its hover text, as the Mac's lists do (a long link cut
/// short in its column can still be read). While the pointer is on a cell, the list's own hover text waits.
pub fn cell_tips(list: HWND) {
    unsafe {
        let _ = windows::Win32::UI::Shell::SetWindowSubclass(list, Some(cell_tip_proc), 0x4844_4354, 0);
    }
}

thread_local! {
    /// Lists whose checkboxes have their own hover text: (when ticked, when not).
    static CHECK_TIPS: std::cell::RefCell<std::collections::HashMap<isize, (&'static str, &'static str)>> = Default::default();
}

/// `cell_tips`, and each row's checkbox explains itself: `on` while it is ticked, `off` while not (the Mac's
/// per-source checkboxes).
pub fn cell_tips_with_checks(list: HWND, on: &'static str, off: &'static str) {
    CHECK_TIPS.with(|c| c.borrow_mut().insert(list.0 as isize, (on, off)));
    cell_tips(list);
}

unsafe extern "system" fn cell_tip_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _: usize, _: usize) -> LRESULT {
    use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
    unsafe {
        match msg {
            WM_MOUSEMOVE => {
                cell_tip(hwnd, Some(POINT { x: (lparam.0 & 0xffff) as i16 as i32, y: ((lparam.0 >> 16) & 0xffff) as i16 as i32 }));
                let mut track = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                let _ = TrackMouseEvent(&mut track);
            }
            WM_MOUSELEAVE => cell_tip(hwnd, None),
            WM_DESTROY => {
                CHECK_TIPS.with(|c| c.borrow_mut().remove(&(hwnd.0 as isize)));
                if let Some((tips, _)) = CELL_TIPS.with(|c| c.borrow_mut().remove(&(hwnd.0 as isize))) {
                    let _ = DestroyWindow(tips);
                }
            }
            _ => {}
        }
        windows::Win32::UI::Shell::DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}

/// Puts the list's cell tooltip on the cell under `point` (list pixels), with that cell's text, or takes it away.
fn cell_tip(list: HWND, point: Option<POINT>) {
    unsafe {
        let key = list.0 as isize;
        let (tips, shown) = match CELL_TIPS.with(|c| c.borrow().get(&key).copied()) {
            Some(entry) if IsWindow(Some(entry.0)).as_bool() => entry,
            _ => {
                let instance = GetModuleHandleW(None).unwrap_or_default();
                let t = CreateWindowExW(WS_EX_TOPMOST, TOOLTIPS_CLASSW, PCWSTR::null(), WINDOW_STYLE(TTS_ALWAYSTIP | TTS_NOPREFIX),
                                        CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, Some(list), None, Some(instance.into()), None)
                    .unwrap_or_default();
                SendMessageW(t, TTM_SETMAXTIPWIDTH, None, Some(LPARAM(420)));
                crate::theme::control(t);
                (t, None)
            }
        };
        let mut hit = LVHITTESTINFO { pt: point.unwrap_or_default(), ..Default::default() };
        let checks = CHECK_TIPS.with(|c| c.borrow().get(&key).copied());
        let cell = point.and_then(|_| {
            let item = SendMessageW(list, LVM_SUBITEMHITTEST, None, Some(LPARAM(&mut hit as *mut _ as isize))).0;
            // Column -1: the row's checkbox (LVHT_ONITEMSTATEICON).
            let on_check = checks.is_some() && hit.flags.0 & 0x8 != 0;
            (item >= 0 && hit.iSubItem >= 0).then_some((item as i32, if on_check { -1 } else { hit.iSubItem }))
        });
        pause(list, cell.is_some());
        CELL_TIPS.with(|c| c.borrow_mut().insert(key, (tips, cell)));
        if cell == shown {
            return;
        }
        let mut info = TTTOOLINFOW { cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32, hwnd: list, uId: 1, ..Default::default() };
        SendMessageW(tips, TTM_POP, None, None);
        SendMessageW(tips, TTM_DELTOOLW, None, Some(LPARAM(&info as *const _ as isize)));
        let Some((row, column)) = cell else { return };
        if column == -1
            && let Some((on, off)) = checks
        {
            let ticked = (SendMessageW(list, LVM_GETITEMSTATE, Some(WPARAM(row as usize)), Some(LPARAM(LVIS_STATEIMAGEMASK.0 as isize))).0 >> 12) == 2;
            let text: Vec<u16> = (if ticked { on } else { off }).encode_utf16().chain(std::iter::once(0)).collect();
            // The checkbox: from the row's left edge to its label.
            let mut rect = RECT { left: LVIR_BOUNDS as i32, ..Default::default() };
            SendMessageW(list, LVM_GETITEMRECT, Some(WPARAM(row as usize)), Some(LPARAM(&mut rect as *mut _ as isize)));
            let mut label = RECT { left: LVIR_LABEL as i32, ..Default::default() };
            SendMessageW(list, LVM_GETITEMRECT, Some(WPARAM(row as usize)), Some(LPARAM(&mut label as *mut _ as isize)));
            rect.right = label.left;
            info.uFlags = TTF_SUBCLASS;
            info.rect = rect;
            info.lpszText = PWSTR(text.as_ptr() as *mut _);
            SendMessageW(tips, TTM_ADDTOOLW, None, Some(LPARAM(&info as *const _ as isize)));
            return;
        }
        let mut buffer = [0u16; 2048];
        let get = LVITEMW { iSubItem: column, pszText: PWSTR(buffer.as_mut_ptr()), cchTextMax: buffer.len() as i32, ..Default::default() };
        let n = SendMessageW(list, LVM_GETITEMTEXTW, Some(WPARAM(row as usize)), Some(LPARAM(&get as *const _ as isize))).0.max(0) as usize;
        if n == 0 {
            return;
        }
        // The cell's own rectangle (column 0's label, not the whole row).
        let mut rect = RECT { top: column, left: if column == 0 { LVIR_LABEL as i32 } else { LVIR_BOUNDS as i32 }, ..Default::default() };
        SendMessageW(list, LVM_GETSUBITEMRECT, Some(WPARAM(row as usize)), Some(LPARAM(&mut rect as *mut _ as isize)));
        let text: Vec<u16> = buffer[..n].iter().copied().chain(std::iter::once(0)).collect();
        info.uFlags = TTF_SUBCLASS;
        info.rect = rect;
        info.lpszText = PWSTR(text.as_ptr() as *mut _);
        SendMessageW(tips, TTM_ADDTOOLW, None, Some(LPARAM(&info as *const _ as isize)));
    }
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
                "combobox" | "syslistview32" | "systreeview32" | "systabcontrol32" | "herbsdmsegments" => true,
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
