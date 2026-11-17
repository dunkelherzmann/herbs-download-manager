//! Light and dark, as the Mac app follows the system's appearance (§2): the colours come from the Mac window in each
//! appearance, and Windows' "Choose your app mode" picks which. A change of mode applies at once to every open window.
//! Windows' own controls are switched to their dark themes (lists, trees, fields, menus, scroll bars, tooltips);
//! check boxes and radio buttons are drawn here, as their dark theme keeps black text.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, IsWindowEnabled, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, PCSTR, PCWSTR, w};

/// The colours of one appearance, as COLORREF (0x00BBGGRR).
#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// Title row, sidebar, footer and dialogs.
    pub chrome: COLORREF,
    /// The list's background and its column titles.
    pub list: COLORREF,
    /// Every other row of the list.
    pub stripe: COLORREF,
    pub text: COLORREF,
    pub secondary: COLORREF,
    pub disabled: COLORREF,
    /// Toolbar symbols.
    pub icon: COLORREF,
    /// A toolbar button under the pointer, and the chosen sidebar line.
    pub hover: COLORREF,
    pub pressed: COLORREF,
    pub sidebar_selection: COLORREF,
    /// The line under the title row and right of the sidebar.
    pub edge: COLORREF,
    /// The line above the footer.
    pub footer_line: COLORREF,
    /// The line under the column titles, and between them.
    pub header_line: COLORREF,
    pub field: COLORREF,
    /// Selected rows: the system's accent colour, with white text.
    pub accent: COLORREF,
    pub red: COLORREF,
    pub track: COLORREF,
}

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

const DARK: Palette = Palette {
    dark: true,
    chrome: rgb(58, 58, 58),
    list: rgb(48, 48, 48),
    stripe: rgb(58, 58, 58),
    text: rgb(236, 236, 236),
    secondary: rgb(176, 176, 176),
    disabled: rgb(110, 110, 110),
    icon: rgb(196, 196, 196),
    hover: rgb(76, 76, 76),
    pressed: rgb(90, 90, 90),
    sidebar_selection: rgb(84, 84, 84),
    edge: rgb(0, 0, 0),
    footer_line: rgb(77, 77, 77),
    header_line: rgb(90, 90, 90),
    field: rgb(40, 40, 40),
    accent: rgb(0, 88, 208),
    red: rgb(255, 105, 97),
    track: rgb(82, 82, 82),
};

const LIGHT: Palette = Palette {
    dark: false,
    chrome: rgb(236, 236, 236),
    list: rgb(255, 255, 255),
    stripe: rgb(244, 245, 245),
    text: rgb(38, 38, 38),
    secondary: rgb(128, 128, 128),
    disabled: rgb(182, 182, 182),
    icon: rgb(98, 98, 98),
    hover: rgb(222, 222, 222),
    pressed: rgb(208, 208, 208),
    sidebar_selection: rgb(212, 212, 212),
    edge: rgb(200, 200, 200),
    footer_line: rgb(214, 214, 214),
    header_line: rgb(224, 224, 224),
    field: rgb(255, 255, 255),
    accent: rgb(0, 100, 225),
    red: rgb(215, 0, 21),
    track: rgb(222, 222, 222),
};

thread_local! {
    static CURRENT: Cell<Palette> = Cell::new(read());
    static BRUSHES: RefCell<HashMap<u32, HBRUSH>> = RefCell::new(HashMap::new());
}

/// Whether Windows' app mode is dark (Settings ▸ Personalization ▸ Colors ▸ Choose your app mode).
fn system_dark() -> bool {
    let mut value = 1u32;
    let mut size = 4u32;
    unsafe {
        let _ = RegGetValueW(HKEY_CURRENT_USER, w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
                             w!("AppsUseLightTheme"), RRF_RT_REG_DWORD, None, Some(&mut value as *mut _ as *mut _), Some(&mut size));
    }
    value == 0
}

fn read() -> Palette {
    let mut p = if system_dark() { DARK } else { LIGHT };
    // Selected rows take the system's accent colour, as the Mac's take its accent colour.
    let accent = unsafe { GetSysColor(COLOR_HIGHLIGHT) };
    if accent != 0 {
        p.accent = COLORREF(accent);
    }
    p
}

pub fn palette() -> Palette {
    CURRENT.get()
}

pub fn dark() -> bool {
    CURRENT.get().dark
}

/// A solid brush of this colour, made once.
pub fn brush(color: COLORREF) -> HBRUSH {
    BRUSHES.with(|b| *b.borrow_mut().entry(color.0).or_insert_with(|| unsafe { CreateSolidBrush(color) }))
}

pub fn blend(a: COLORREF, b: COLORREF, t: f32) -> COLORREF {
    let mix = |shift: u32| {
        let (x, y) = ((a.0 >> shift) & 0xff, (b.0 >> shift) & 0xff);
        ((x as f32 + (y as f32 - x as f32) * t).round() as u32) << shift
    };
    COLORREF(mix(0) | mix(8) | mix(16))
}

type SetPreferredAppMode = unsafe extern "system" fn(i32) -> i32;
type AllowDarkModeForWindow = unsafe extern "system" fn(HWND, BOOL) -> BOOL;
type FlushMenuThemes = unsafe extern "system" fn();

/// uxtheme's unnamed exports for dark menus and scroll bars (by ordinal, as File Explorer and Notepad use them).
fn uxtheme(ordinal: usize) -> Option<unsafe extern "system" fn() -> isize> {
    unsafe {
        let module = LoadLibraryW(w!("uxtheme.dll")).ok()?;
        GetProcAddress(module, PCSTR(ordinal as *const u8))
    }
}

/// Once at launch: menus and scroll bars follow the app mode.
pub fn init() {
    unsafe {
        if let Some(f) = uxtheme(135) {
            let set: SetPreferredAppMode = std::mem::transmute(f);
            set(1); // AllowDark: follow the system
        }
        if let Some(f) = uxtheme(136) {
            let flush: FlushMenuThemes = std::mem::transmute(f);
            flush();
        }
    }
}

/// Reads the app mode again after Windows says it changed (WM_SETTINGCHANGE "ImmersiveColorSet"), and redraws every
/// window of the app in it. Returns whether it changed.
pub fn reload() -> bool {
    let before = dark();
    CURRENT.set(read());
    if before == dark() {
        return false;
    }
    unsafe {
        init();
        unsafe extern "system" fn each(hwnd: HWND, _: LPARAM) -> BOOL {
            apply(hwnd);
            let _ = unsafe { RedrawWindow(Some(hwnd), None, None, RDW_ERASE | RDW_INVALIDATE | RDW_FRAME | RDW_ALLCHILDREN) };
            BOOL(1)
        }
        let _ = EnumThreadWindows(windows::Win32::System::Threading::GetCurrentThreadId(), Some(each), LPARAM(0));
    }
    true
}

fn class_of(hwnd: HWND) -> String {
    let mut name = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd, &mut name) };
    String::from_utf16_lossy(&name[..n.max(0) as usize])
}

/// Puts a window and all its controls in the current mode: the title bar, then each control by its kind. Call it once
/// a window's controls exist; `reload` calls it again for every window when the mode changes.
pub fn apply(hwnd: HWND) {
    let p = palette();
    unsafe {
        let on = BOOL(p.dark as i32);
        if let Some(f) = uxtheme(133) {
            let allow: AllowDarkModeForWindow = std::mem::transmute(f);
            let _ = allow(hwnd, on);
        }
        // DWMWA_USE_IMMERSIVE_DARK_MODE
        let _ = DwmSetWindowAttribute(hwnd, windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE(20), &on as *const _ as *const _, 4);
        unsafe extern "system" fn child(hwnd: HWND, _: LPARAM) -> BOOL {
            control(hwnd);
            BOOL(1)
        }
        let _ = EnumChildWindows(Some(hwnd), Some(child), LPARAM(0));
        // A tooltip or other top-level helper window is a control itself.
        if class_of(hwnd) == "tooltips_class32" {
            control(hwnd);
        }
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

/// One control in the current mode.
pub fn control(hwnd: HWND) {
    let p = palette();
    let class = class_of(hwnd);
    let explorer = if p.dark { w!("DarkMode_Explorer") } else { w!("Explorer") };
    let cfd = if p.dark { w!("DarkMode_CFD") } else { PCWSTR::null() };
    unsafe {
        if let Some(f) = uxtheme(133) {
            let allow: AllowDarkModeForWindow = std::mem::transmute(f);
            let _ = allow(hwnd, BOOL(p.dark as i32));
        }
        match class.as_str() {
            "SysListView32" => {
                let _ = SetWindowTheme(hwnd, explorer, PCWSTR::null());
                SendMessageW(hwnd, LVM_SETBKCOLOR, None, Some(LPARAM(p.list.0 as isize)));
                SendMessageW(hwnd, LVM_SETTEXTBKCOLOR, None, Some(LPARAM(p.list.0 as isize)));
                SendMessageW(hwnd, LVM_SETTEXTCOLOR, None, Some(LPARAM(p.text.0 as isize)));
                let header = HWND(SendMessageW(hwnd, LVM_GETHEADER, None, None).0 as *mut _);
                if !header.is_invalid() {
                    let _ = SetWindowTheme(header, if p.dark { w!("DarkMode_ItemsView") } else { w!("ItemsView") }, PCWSTR::null());
                }
            }
            "SysTreeView32" => {
                let _ = SetWindowTheme(hwnd, explorer, PCWSTR::null());
                SendMessageW(hwnd, TVM_SETBKCOLOR, None, Some(LPARAM(p.chrome.0 as isize)));
                SendMessageW(hwnd, TVM_SETTEXTCOLOR, None, Some(LPARAM(p.text.0 as isize)));
            }
            "Edit" | "ComboBox" | "SysDateTimePick32" => {
                let _ = SetWindowTheme(hwnd, cfd, PCWSTR::null());
            }
            "ComboLBox" | "ListBox" | "ScrollBar" | "tooltips_class32" => {
                let _ = SetWindowTheme(hwnd, explorer, PCWSTR::null());
            }
            // A list's column titles (found again among the window's children after their list): the items view's
            // theme, the one that has a dark look; the Explorer theme draws them light.
            "SysHeader32" => {
                let _ = SetWindowTheme(hwnd, if p.dark { w!("DarkMode_ItemsView") } else { w!("ItemsView") }, PCWSTR::null());
            }
            "Button" => {
                let style = GetWindowLongW(hwnd, GWL_STYLE) as u32 & 0xf;
                let kind = style;
                // Push buttons: the dark Explorer theme. Check boxes and radio buttons: drawn here (`check_proc`).
                if matches!(kind, 2 | 3 | 4 | 5 | 6 | 9) {
                    let _ = SetWindowTheme(hwnd, explorer, PCWSTR::null());
                    let _ = SetWindowSubclass(hwnd, Some(check_proc), 0x4844_4348, 0);
                } else if kind != 11 {
                    let _ = SetWindowTheme(hwnd, explorer, PCWSTR::null());
                }
            }
            _ => {}
        }
    }
}

/// WM_CTLCOLOR… for every window of the app: labels and dialogs on the chrome colour, fields on the field colour, all
/// with the mode's text colour. Returns None for other messages.
pub fn ctl_color(msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    let p = palette();
    let dc = HDC(wparam.0 as *mut _);
    let control = HWND(lparam.0 as *mut _);
    unsafe {
        match msg {
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN | WM_CTLCOLORDLG => {
                // Read-only fields (WM_CTLCOLORSTATIC comes from those too) are plain text on the window's colour.
                let back = p.chrome;
                let color = match GetPropW(control, w!("hdm-tone")).0 as usize {
                    _ if !IsWindowEnabled(control).as_bool() => p.disabled,
                    1 => p.secondary,
                    2 => p.red,
                    _ => p.text,
                };
                SetTextColor(dc, color);
                SetBkColor(dc, back);
                SetBkMode(dc, TRANSPARENT);
                Some(LRESULT(brush(back).0 as isize))
            }
            WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
                SetTextColor(dc, p.text);
                SetBkColor(dc, p.field);
                Some(LRESULT(brush(p.field).0 as isize))
            }
            _ => None,
        }
    }
}

/// A label's text colour, other than the mode's text: the Mac's secondary grey (help and previews), or red (a mistake).
#[derive(Clone, Copy)]
pub enum Tone {
    Secondary = 1,
    Red = 2,
}

pub fn set_tone(control: HWND, tone: Tone) {
    unsafe {
        let _ = SetPropW(control, w!("hdm-tone"), Some(HANDLE(tone as usize as *mut _)));
        let _ = InvalidateRect(Some(control), None, true);
    }
}

/// Paints a window's background in the chrome colour (WM_ERASEBKGND).
pub fn erase(hwnd: HWND, dc: HDC) -> LRESULT {
    unsafe {
        let mut rect = RECT::default();
        let _ = GetClientRect(hwnd, &mut rect);
        FillRect(dc, &rect, brush(palette().chrome));
    }
    LRESULT(1)
}

/// Check boxes and radio buttons with the mode's text colour: the box from the theme, the text drawn here.
unsafe extern "system" fn check_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _: usize, _: usize) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                paint_check(hwnd, dc);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_PRINTCLIENT => {
                paint_check(hwnd, HDC(wparam.0 as *mut _));
                LRESULT(0)
            }
            // The state changes repaint at once (the default code would draw the old look over ours).
            BM_SETCHECK | BM_SETSTATE | WM_ENABLE | WM_SETTEXT | WM_UPDATEUISTATE => {
                let r = DefSubclassProc(hwnd, msg, wparam, lparam);
                let _ = InvalidateRect(Some(hwnd), None, false);
                r
            }
            WM_MOUSEMOVE | WM_MOUSELEAVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_SETFOCUS | WM_KILLFOCUS => {
                let r = DefSubclassProc(hwnd, msg, wparam, lparam);
                if msg == WM_MOUSEMOVE {
                    let mut track = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    let _ = TrackMouseEvent(&mut track);
                }
                let _ = InvalidateRect(Some(hwnd), None, false);
                r
            }
            _ => DefSubclassProc(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe fn paint_check(hwnd: HWND, dc: HDC) {
    unsafe {
        let p = palette();
        let mut rect = RECT::default();
        let _ = GetClientRect(hwnd, &mut rect);
        // Double-buffered, so a refresh never flickers.
        let mem = CreateCompatibleDC(Some(dc));
        let bitmap = CreateCompatibleBitmap(dc, rect.right, rect.bottom);
        let old = SelectObject(mem, bitmap.into());
        let parent = GetParent(hwnd).unwrap_or_default();
        // The parent's background colour behind it.
        let back = HBRUSH(SendMessageW(parent, WM_CTLCOLORBTN, Some(WPARAM(mem.0 as usize)), Some(LPARAM(hwnd.0 as isize))).0 as *mut _);
        FillRect(mem, &rect, if back.is_invalid() { brush(p.chrome) } else { back });
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let radio = matches!(style & 0xf, 4 | 9);
        let checked = SendMessageW(hwnd, BM_GETCHECK, None, None).0 as u32;
        let state = SendMessageW(hwnd, BM_GETSTATE, None, None).0 as u32;
        let enabled = IsWindowEnabled(hwnd).as_bool();
        let hot = {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let _ = ScreenToClient(hwnd, &mut pt);
            pt.x >= 0 && pt.y >= 0 && pt.x < rect.right && pt.y < rect.bottom
        };
        let pushed = state & 0x4 != 0; // BST_PUSHED
        let theme = OpenThemeData(Some(hwnd), w!("Button"));
        let scale = GetDpiForWindow(hwnd).max(96) as i32;
        let size = 13 * scale / 96;
        let top = (rect.bottom - size) / 2;
        let box_rect = RECT { left: 0, top, right: size, bottom: top + size };
        let base = match (enabled, pushed, hot) {
            (false, _, _) => 4,
            (true, true, _) => 3,
            (true, false, true) => 2,
            _ => 1,
        };
        let (part, s) = if radio {
            (BP_RADIOBUTTON.0, if checked == 1 { base + 4 } else { base })
        } else {
            (BP_CHECKBOX.0, match checked { 1 => base + 4, 2 => base + 8, _ => base })
        };
        if !theme.is_invalid() {
            let _ = DrawThemeBackground(theme, mem, part, s, &box_rect, None);
            let _ = CloseThemeData(theme);
        }
        let mut text = [0u16; 256];
        let n = GetWindowTextW(hwnd, &mut text) as usize;
        let font = HFONT(SendMessageW(hwnd, WM_GETFONT, None, None).0 as *mut _);
        let old_font = SelectObject(mem, font.into());
        SetBkMode(mem, TRANSPARENT);
        SetTextColor(mem, if enabled { p.text } else { p.disabled });
        let mut text_rect = RECT { left: size + 5 * scale / 96, top: 0, right: rect.right, bottom: rect.bottom };
        DrawTextW(mem, &mut text[..n], &mut text_rect, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
        if GetFocus() == hwnd && SendMessageW(hwnd, WM_QUERYUISTATE, None, None).0 as u32 & 0x1 /* UISF_HIDEFOCUS */ == 0 {
            let mut measure = text_rect;
            DrawTextW(mem, &mut text[..n], &mut measure, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_CALCRECT);
            let height = measure.bottom - measure.top;
            let focus = RECT { left: measure.left - 1, top: (rect.bottom - height) / 2 - 1, right: measure.right + 1, bottom: (rect.bottom + height) / 2 + 1 };
            let _ = DrawFocusRect(mem, &focus);
        }
        SelectObject(mem, old_font);
        let _ = BitBlt(dc, 0, 0, rect.right, rect.bottom, Some(mem), 0, 0, SRCCOPY);
        SelectObject(mem, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem);
    }
}

/// The app's fonts, as on the Mac: the system font at 13 points (Segoe UI Variable on Windows 11), bold for titles, 11
/// for the footer and the sidebar's counts. Sized for a window's DPI.
pub struct Fonts {
    pub text: HFONT,
    pub bold: HFONT,
    pub small: HFONT,
}

pub fn fonts(dpi: u32) -> Fonts {
    thread_local! {
        static MADE: RefCell<HashMap<u32, (isize, isize, isize)>> = RefCell::new(HashMap::new());
    }
    let made = MADE.with(|m| {
        *m.borrow_mut().entry(dpi).or_insert_with(|| {
            let font = |px: i32, weight: i32, face: &str| unsafe {
                let mut lf = LOGFONTW { lfHeight: -(px * dpi as i32 / 96), lfWeight: weight, lfQuality: CLEARTYPE_QUALITY, ..Default::default() };
                let f: Vec<u16> = face.encode_utf16().collect();
                lf.lfFaceName[..f.len().min(31)].copy_from_slice(&f[..f.len().min(31)]);
                CreateFontIndirectW(&lf).0 as isize
            };
            let face = if font_exists("Segoe UI Variable Text") { "Segoe UI Variable Text" } else { "Segoe UI" };
            (font(13, 400, face), font(13, 600, face), font(11, 400, face))
        })
    });
    Fonts {
        text: HFONT(made.0 as *mut _),
        bold: HFONT(made.1 as *mut _),
        small: HFONT(made.2 as *mut _),
    }
}

/// The Mac's small monospaced font (11 points) for previews of links.
pub fn mono(dpi: u32) -> HFONT {
    thread_local! {
        static MADE: RefCell<HashMap<u32, isize>> = RefCell::new(HashMap::new());
    }
    let made = MADE.with(|m| {
        *m.borrow_mut().entry(dpi).or_insert_with(|| unsafe {
            let face = if font_exists("Cascadia Mono") { "Cascadia Mono" } else { "Consolas" };
            let mut lf = LOGFONTW { lfHeight: -(11 * dpi as i32 / 96), lfWeight: 400, lfQuality: CLEARTYPE_QUALITY, ..Default::default() };
            let f: Vec<u16> = face.encode_utf16().collect();
            lf.lfFaceName[..f.len().min(31)].copy_from_slice(&f[..f.len().min(31)]);
            CreateFontIndirectW(&lf).0 as isize
        })
    });
    HFONT(made as *mut _)
}

fn font_exists(face: &str) -> bool {
    unsafe extern "system" fn found(_: *const LOGFONTW, _: *const TEXTMETRICW, _: u32, flag: LPARAM) -> i32 {
        unsafe { *(flag.0 as *mut bool) = true };
        0
    }
    let mut yes = false;
    unsafe {
        let dc = GetDC(None);
        let mut lf = LOGFONTW { lfCharSet: DEFAULT_CHARSET, ..Default::default() };
        let f: Vec<u16> = face.encode_utf16().collect();
        lf.lfFaceName[..f.len().min(31)].copy_from_slice(&f[..f.len().min(31)]);
        EnumFontFamiliesExW(dc, &lf, Some(found), LPARAM(&mut yes as *mut _ as isize), 0);
        ReleaseDC(None, dc);
    }
    yes
}

/// Text drawn in a colour and font, in a rectangle (DrawText flags).
pub fn text(dc: HDC, s: &str, rect: RECT, font: HFONT, color: COLORREF, flags: DRAW_TEXT_FORMAT) {
    // DrawText reads an empty slice's dangling pointer: nothing to draw anyway.
    if s.is_empty() {
        return;
    }
    unsafe {
        let mut buffer: Vec<u16> = s.encode_utf16().collect();
        let mut r = rect;
        let old = SelectObject(dc, font.into());
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, color);
        DrawTextW(dc, &mut buffer, &mut r, flags | DT_NOPREFIX);
        SelectObject(dc, old);
    }
}

/// A filled rectangle with rounded corners (the Mac's hover and selection shapes).
pub fn round_rect(dc: HDC, rect: RECT, radius: i32, color: COLORREF) {
    unsafe {
        let old_brush = SelectObject(dc, brush(color).into());
        let pen = CreatePen(PS_SOLID, 1, color);
        let old_pen = SelectObject(dc, pen.into());
        let _ = RoundRect(dc, rect.left, rect.top, rect.right, rect.bottom, radius * 2, radius * 2);
        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
        let _ = DeleteObject(pen.into());
    }
}

pub fn fill(dc: HDC, rect: RECT, color: COLORREF) {
    unsafe {
        FillRect(dc, &rect, brush(color));
    }
}

/// The shared part of every dialog-like window's procedure (Settings, New Download, Add Links, the Download window,
/// Sign-ins): its background, its controls' colours, and its controls put in the current mode when it first shows.
/// Call first; Some means the message was handled.
pub fn dialog(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    match msg {
        WM_ERASEBKGND => Some(erase(hwnd, HDC(wparam.0 as *mut _))),
        WM_SHOWWINDOW if wparam.0 != 0 => {
            apply(hwnd);
            None
        }
        _ => ctl_color(msg, wparam, lparam),
    }
}
