//! Questions and messages laid out as the Mac's alerts, in the app's light or dark look: the app's icon at the top,
//! the question in bold and the explanation under it, both centred, an optional check box, and the buttons at the
//! bottom (side by side when there are two, one above the other otherwise), the default one in the accent colour.
//! As on the Mac, the arrow keys and Tab move the highlight (the blue button) from one button to the next, Return or
//! Enter presses the highlighted one, and Esc the cancel button. The owner window is disabled while it's open.

use std::cell::RefCell;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_SELECTED};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use crate::theme;

/// What a countdown alert does each second (`Alert::tick`).
pub enum Tick {
    /// New explanation text.
    Body(String),
    /// Press this button.
    Press(usize),
}

pub struct Alert {
    pub heading: String,
    pub body: String,
    /// Left to right (or top to bottom).
    pub buttons: Vec<String>,
    pub default: usize,
    /// The button Esc and the window's close box press; None: they do nothing.
    pub cancel: Option<usize>,
    pub checkbox: Option<String>,
    /// A web address shown under the text, opened in the browser when clicked.
    pub link: Option<String>,
    /// Called once a second with the seconds since it opened.
    pub tick: Option<Box<dyn FnMut(u32) -> Tick>>,
}

impl Alert {
    pub fn new(heading: impl Into<String>, body: impl Into<String>, buttons: &[&str], default: usize, cancel: Option<usize>) -> Self {
        Alert {
            heading: heading.into(),
            body: body.into(),
            buttons: buttons.iter().map(|b| b.to_string()).collect(),
            default,
            cancel,
            checkbox: None,
            link: None,
            tick: None,
        }
    }
}

struct Open {
    alert: Alert,
    pressed: Option<usize>,
    checked: bool,
    body: HWND,
    check: HWND,
    seconds: u32,
}

thread_local! {
    static OPEN: RefCell<Vec<(isize, Open)>> = const { RefCell::new(Vec::new()) };
}

const ID_BUTTON: u16 = 100;
const ID_CHECK: u16 = 90;
const ID_LINK: u16 = 91;

fn with<R>(hwnd: HWND, f: impl FnOnce(&mut Open) -> R) -> Option<R> {
    OPEN.with(|o| o.try_borrow_mut().ok()?.iter_mut().find(|(h, _)| *h == hwnd.0 as isize).map(|(_, a)| f(a)))
}

/// The open alert (the feature tests answer the queue's countdown through it): its window and heading.
pub fn current() -> Option<(HWND, String)> {
    OPEN.with(|o| o.borrow().last().map(|(h, a)| (HWND(*h as *mut _), a.alert.heading.clone())))
}

fn finish(hwnd: HWND, pressed: Option<usize>) {
    with(hwnd, |a| {
        a.pressed = pressed;
        a.checked = !a.check.is_invalid() && unsafe { SendMessageW(a.check, BM_GETCHECK, None, None).0 } == 1;
    });
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

/// Shows the alert over `owner` and waits: the pressed button's index (None when closed without one), and whether
/// the check box was ticked.
pub fn run(owner: HWND, alert: Alert) -> (Option<usize>, bool) {
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class = WNDCLASSW {
            lpfnWndProc: Some(proc_),
            hInstance: instance.into(),
            lpszClassName: w!("HerbsDMAlert"),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&class);
        let owner = if owner.is_invalid() { owner } else { GetAncestor(owner, GA_ROOT) };
        let dpi = if owner.is_invalid() { 96 } else { GetDpiForWindow(owner).max(96) as i32 };
        let s = |v: i32| v * dpi / 96;
        let fonts = theme::fonts(dpi as u32);

        // Measure the text at the alert's width to size the window.
        let width = s(300);
        let inner = width - s(40);
        let measure = |text: &str, font: HFONT| -> i32 {
            if text.is_empty() {
                return 0;
            }
            let dc = GetDC(None);
            let old = SelectObject(dc, font.into());
            let mut r = RECT { left: 0, top: 0, right: inner, bottom: 0 };
            let mut t: Vec<u16> = text.encode_utf16().collect();
            DrawTextW(dc, &mut t, &mut r, DT_CENTER | DT_WORDBREAK | DT_CALCRECT | DT_NOPREFIX);
            SelectObject(dc, old);
            ReleaseDC(None, dc);
            r.bottom
        };
        let icon = s(64);
        let heading_h = measure(&alert.heading, fonts.bold);
        let body_h = measure(&alert.body, fonts.small);
        let link_h = alert.link.as_ref().map(|l| measure(l, fonts.small)).unwrap_or(0);
        let side_by_side = alert.buttons.len() <= 2;
        let button_h = s(28);
        let rows = if side_by_side { 1 } else { alert.buttons.len() as i32 };
        let mut y = s(20) + icon + s(14);
        let heading_y = y;
        y += heading_h + s(6);
        let body_y = y;
        y += body_h + if link_h > 0 { s(6) } else { 0 };
        let link_y = y;
        y += link_h + s(16);
        let check_y = y;
        if alert.checkbox.is_some() {
            y += s(26);
        }
        let buttons_y = y;
        y += rows * button_h + (rows - 1) * s(8) + s(18);
        let height = y;

        // Centred over the owner (or the screen).
        let mut frame = RECT { left: 0, top: 0, right: width, bottom: height };
        let style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
        let _ = AdjustWindowRectExForDpi(&mut frame, style, false, WS_EX_DLGMODALFRAME, dpi as u32);
        let (fw, fh) = (frame.right - frame.left, frame.bottom - frame.top);
        let mut area = RECT::default();
        if owner.is_invalid() || !IsWindowVisible(owner).as_bool() || IsIconic(owner).as_bool() {
            let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut area as *mut _ as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
        } else {
            let _ = GetWindowRect(owner, &mut area);
        }
        let (x0, y0) = (area.left + (area.right - area.left - fw) / 2, area.top + (area.bottom - area.top - fh) / 3);
        let title: Vec<u16> = "Herb's Download Manager\0".encode_utf16().collect();
        let Ok(hwnd) = CreateWindowExW(WS_EX_DLGMODALFRAME, w!("HerbsDMAlert"), PCWSTR(title.as_ptr()), style, x0, y0, fw, fh,
                                       Some(owner).filter(|o| !o.is_invalid()), None, Some(instance.into()), None)
        else {
            return (None, false);
        };
        let child = |class: PCWSTR, text: &str, style: u32, id: u16, x: i32, y: i32, w: i32, h: i32| {
            let t: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
            CreateWindowExW(WINDOW_EX_STYLE::default(), class, PCWSTR(t.as_ptr()), WS_CHILD | WS_VISIBLE | WINDOW_STYLE(style), x, y, w, h,
                            Some(hwnd), Some(HMENU(id as usize as *mut _)), Some(instance.into()), None)
                .unwrap_or_default()
        };
        let pad = s(20);
        let heading = child(w!("STATIC"), &alert.heading, 0x1 /* SS_CENTER */ | 0x80 /* SS_NOPREFIX */, 0, pad, heading_y, inner, heading_h);
        SendMessageW(heading, WM_SETFONT, Some(WPARAM(fonts.bold.0 as usize)), Some(LPARAM(1)));
        let body = child(w!("STATIC"), &alert.body, 0x1 | 0x80, 0, pad, body_y, inner, body_h.max(1));
        SendMessageW(body, WM_SETFONT, Some(WPARAM(fonts.small.0 as usize)), Some(LPARAM(1)));
        if let Some(link) = &alert.link {
            let l = child(w!("STATIC"), link, 0x1 | 0x80 | 0x100 /* SS_NOTIFY */, ID_LINK, pad, link_y, inner, link_h);
            SendMessageW(l, WM_SETFONT, Some(WPARAM(fonts.small.0 as usize)), Some(LPARAM(1)));
            crate::a11y::tip(l, "Open the project's page in the browser");
        }
        let check = match &alert.checkbox {
            Some(text) => {
                let c = child(w!("BUTTON"), text, BS_AUTOCHECKBOX as u32 | WS_TABSTOP.0, ID_CHECK, pad, check_y, inner, s(20));
                SendMessageW(c, WM_SETFONT, Some(WPARAM(fonts.small.0 as usize)), Some(LPARAM(1)));
                c
            }
            None => HWND::default(),
        };
        let mut buttons = Vec::new();
        let n = alert.buttons.len() as i32;
        for (i, label) in alert.buttons.iter().enumerate() {
            let i32_ = i as i32;
            let (bx, by, bw) = if side_by_side {
                let w = (inner - s(8) * (n - 1)) / n;
                (pad + i32_ * (w + s(8)), buttons_y, w)
            } else {
                (pad, buttons_y + i32_ * (button_h + s(8)), inner)
            };
            let b = child(w!("BUTTON"), label, BS_OWNERDRAW as u32 | WS_TABSTOP.0, ID_BUTTON + i as u16, bx, by, bw, button_h);
            SendMessageW(b, WM_SETFONT, Some(WPARAM(fonts.text.0 as usize)), Some(LPARAM(1)));
            buttons.push(b);
        }
        let has_tick = alert.tick.is_some();
        let default = alert.default.min(buttons.len().saturating_sub(1));
        let mut alert = alert;
        alert.default = default;
        OPEN.with(|o| o.borrow_mut().push((hwnd.0 as isize, Open { alert, pressed: None, checked: false, body, check, seconds: 0 })));
        theme::apply(hwnd);
        if has_tick {
            SetTimer(Some(hwnd), 1, 1000, None);
        }
        if !owner.is_invalid() {
            let _ = EnableWindow(owner, false);
        }
        let _ = ShowWindow(hwnd, SW_SHOW);
        if let Some(b) = buttons.get(default) {
            let _ = SetFocus(Some(*b));
        }
        let _ = SetForegroundWindow(hwnd);
        let mut msg = MSG::default();
        while IsWindow(Some(hwnd)).as_bool() && GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_KEYDOWN && GetAncestor(msg.hwnd, GA_ROOT) == hwnd {
                let key = VIRTUAL_KEY(msg.wParam.0 as u16);
                let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
                let step = match key {
                    VK_RIGHT | VK_DOWN => 1,
                    VK_LEFT | VK_UP => -1,
                    VK_TAB if shift => -1,
                    VK_TAB => 1,
                    _ => 0,
                };
                // The arrows and Tab move the highlight among the buttons (Ctrl and Alt combinations are left alone).
                if step != 0 && buttons.len() > 1 && GetKeyState(VK_CONTROL.0 as i32) >= 0 && GetKeyState(VK_MENU.0 as i32) >= 0 {
                    let n = buttons.len() as i32;
                    let next = with(hwnd, |a| {
                        a.alert.default = ((a.alert.default as i32 + step + n) % n) as usize;
                        a.alert.default
                    });
                    if let Some(next) = next {
                        let _ = SetFocus(Some(buttons[next]));
                        for b in &buttons {
                            let _ = InvalidateRect(Some(*b), None, false);
                        }
                    }
                    continue;
                }
                match key {
                    VK_RETURN => {
                        let highlighted = with(hwnd, |a| a.alert.default).unwrap_or(default);
                        finish(hwnd, Some(highlighted));
                        continue;
                    }
                    VK_ESCAPE => {
                        if let Some(c) = with(hwnd, |a| a.alert.cancel).flatten() {
                            finish(hwnd, Some(c));
                        }
                        continue;
                    }
                    _ => {}
                }
            }
            if IsDialogMessageW(hwnd, &msg).as_bool() {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if msg.message == WM_QUIT {
            PostQuitMessage(msg.wParam.0 as i32);
        }
        if !owner.is_invalid() {
            let _ = EnableWindow(owner, true);
            let _ = SetForegroundWindow(owner);
        }
        let open = OPEN.with(|o| {
            let mut o = o.borrow_mut();
            let at = o.iter().position(|(h, _)| *h == hwnd.0 as isize)?;
            Some(o.remove(at).1)
        });
        open.map(|a| (a.pressed, a.checked)).unwrap_or((None, false))
    }
}

/// A message with an OK button.
pub fn tell(owner: HWND, heading: &str, body: &str) {
    run(owner, Alert::new(heading, body, &["OK"], 0, Some(0)));
}

extern "system" fn proc_(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if let Some(r) = theme::ctl_color(msg, wparam, lparam) {
            // The link in the accent colour.
            if msg == WM_CTLCOLORSTATIC && GetDlgCtrlID(HWND(lparam.0 as *mut _)) == ID_LINK as i32 {
                SetTextColor(HDC(wparam.0 as *mut _), theme::palette().accent);
            }
            return r;
        }
        match msg {
            WM_ERASEBKGND => theme::erase(hwnd, HDC(wparam.0 as *mut _)),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                let dpi = GetDpiForWindow(hwnd).max(96) as i32;
                let size = 64 * dpi / 96;
                let mut r = RECT::default();
                let _ = GetClientRect(hwnd, &mut r);
                let instance = GetModuleHandleW(None).unwrap_or_default();
                if let Ok(icon) = LoadImageW(Some(instance.into()), PCWSTR(1 as *const u16), IMAGE_ICON, size, size, LR_DEFAULTCOLOR) {
                    let _ = DrawIconEx(dc, (r.right - size) / 2, 20 * dpi / 96, HICON(icon.0), size, size, 0, None, DI_NORMAL);
                    let _ = DestroyIcon(HICON(icon.0));
                }
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_DRAWITEM => {
                let item = &*(lparam.0 as *const DRAWITEMSTRUCT);
                let index = item.CtlID as usize - ID_BUTTON as usize;
                let default = with(hwnd, |a| a.alert.default).unwrap_or(0) == index;
                draw_button(item, default);
                LRESULT(1)
            }
            WM_COMMAND => {
                let id = (wparam.0 & 0xffff) as u16;
                if id >= ID_BUTTON {
                    finish(hwnd, Some((id - ID_BUTTON) as usize));
                } else if id == ID_LINK
                    && let Some(link) = with(hwnd, |a| a.alert.link.clone()).flatten()
                {
                    let t: Vec<u16> = link.encode_utf16().chain(std::iter::once(0)).collect();
                    ShellExecuteW(None, w!("open"), PCWSTR(t.as_ptr()), None, None, SW_SHOWNORMAL);
                }
                LRESULT(0)
            }
            WM_SETCURSOR if GetDlgCtrlID(HWND(wparam.0 as *mut _)) == ID_LINK as i32 => {
                let _ = SetCursor(LoadCursorW(None, IDC_HAND).ok());
                LRESULT(1)
            }
            WM_TIMER => {
                let tick = with(hwnd, |a| {
                    a.seconds += 1;
                    let seconds = a.seconds;
                    a.alert.tick.as_mut().map(|t| t(seconds))
                })
                .flatten();
                match tick {
                    Some(Tick::Body(text)) => {
                        if let Some(body) = with(hwnd, |a| a.body) {
                            let t: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
                            let _ = SetWindowTextW(body, PCWSTR(t.as_ptr()));
                        }
                    }
                    Some(Tick::Press(i)) => finish(hwnd, Some(i)),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                if let Some(c) = with(hwnd, |a| a.alert.cancel).flatten() {
                    finish(hwnd, Some(c));
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// A push button drawn as the Mac's: rounded, the default one in the accent colour with white text.
pub fn draw_button(item: &DRAWITEMSTRUCT, default: bool) {
    let p = theme::palette();
    let dc = item.hDC;
    let r = item.rcItem;
    let hwnd = item.hwndItem;
    let dpi = unsafe { GetDpiForWindow(hwnd).max(96) as i32 };
    let pressed = item.itemState.0 & ODS_SELECTED.0 != 0;
    let disabled = item.itemState.0 & ODS_DISABLED.0 != 0;
    let focused = item.itemState.0 & ODS_FOCUS.0 != 0;
    theme::fill(dc, r, p.chrome);
    let face = if default && !disabled {
        if pressed { theme::blend(p.accent, COLORREF(0), 0.2) } else { p.accent }
    } else if p.dark {
        if pressed { COLORREF(0x7A7A7A) } else { COLORREF(0x656565) }
    } else if pressed {
        COLORREF(0xD8D8D8)
    } else {
        COLORREF(0xFFFFFF)
    };
    let shape = RECT { left: r.left + 1, top: r.top + 1, right: r.right - 1, bottom: r.bottom - 1 };
    if !p.dark && !(default && !disabled) {
        // A light button's thin outline.
        theme::round_rect(dc, RECT { left: r.left, top: r.top, right: r.right, bottom: r.bottom }, 6 * dpi / 96, COLORREF(0xC8C8C8));
    }
    theme::round_rect(dc, shape, 5 * dpi / 96, face);
    if focused && !default {
        theme::round_rect(dc, RECT { left: r.left, top: r.bottom - 2, right: r.right, bottom: r.bottom }, 1, p.accent);
    }
    let mut text = [0u16; 128];
    let n = unsafe { GetWindowTextW(hwnd, &mut text) } as usize;
    let label = String::from_utf16_lossy(&text[..n]);
    let color = if disabled { p.disabled } else if default { COLORREF(0xFFFFFF) } else { p.text };
    let font = HFONT(unsafe { SendMessageW(hwnd, WM_GETFONT, None, None).0 } as *mut _);
    theme::text(dc, &label, r, font, color, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
}
