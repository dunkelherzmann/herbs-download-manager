//! The main window's own drawing, matching the Mac window (§2, §3): one title row with the title at the left, the
//! toolbar's symbols, Search and the Main Menu (☰) at the right and Windows' window controls after them; the sidebar's
//! lines (chevron, symbol, name, count at the right, a rounded selection); the list's rows (30 pixels, every other one
//! shaded, the file's icon, the name with a thin progress bar under it while unfinished, Failed in red); the column
//! titles; and the stripes and "No downloads yet" under the last row.

use std::cell::{Cell, RefCell};

use hd_core::format::{FileKind, Filter};
use hd_core::manager::{Row, Status};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetFocus, TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::theme::{self, Palette};

/// The title row (the Mac's title bar with its toolbar), the footer, a sidebar line, a list row: in 96-DPI pixels.
pub const HEADER: i32 = 52;
pub const FOOTER: i32 = 26;
pub const SIDEBAR_ROW: i32 = 28;
pub const LIST_ROW: i32 = 30;
pub const CAPTION_BUTTON: i32 = 46;
/// A toolbar button and the step from one to the next.
pub const TOOL: i32 = 28;
pub const TOOL_STEP: i32 = 30;

pub fn scale(hwnd: HWND) -> i32 {
    unsafe { GetDpiForWindow(hwnd).max(96) as i32 }
}

pub fn px(hwnd: HWND, v: i32) -> i32 {
    v * scale(hwnd) / 96
}

// --- The title row and the window controls --------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Caption {
    Minimize,
    Maximize,
    Close,
}

thread_local! {
    static HOT_CAPTION: Cell<Option<Caption>> = const { Cell::new(None) };
    static PRESSED_CAPTION: Cell<Option<Caption>> = const { Cell::new(None) };
    static HOT_TOOL: Cell<isize> = const { Cell::new(0) };
    /// What had the keyboard before a toolbar button was pressed: it gets it back once the click is done.
    static FOCUS_BEFORE_TOOL: Cell<isize> = const { Cell::new(0) };
}

/// The window controls' rectangles, right to left in the title row (client coordinates).
pub fn caption_rects(hwnd: HWND) -> [(Caption, RECT); 3] {
    let mut rect = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rect);
    }
    let (w, h) = (px(hwnd, CAPTION_BUTTON), px(hwnd, HEADER));
    let r = |i: i32| RECT { left: rect.right - w * (i + 1), top: 0, right: rect.right - w * i, bottom: h };
    [(Caption::Close, r(0)), (Caption::Maximize, r(1)), (Caption::Minimize, r(2))]
}

/// Forgets which window control was under the pointer (the window was hidden with the pointer on Close).
pub fn reset_captions() {
    HOT_CAPTION.set(None);
    PRESSED_CAPTION.set(None);
}

pub fn caption_at(hwnd: HWND, x: i32, y: i32) -> Option<Caption> {
    caption_rects(hwnd).iter().find(|(_, r)| x >= r.left && x < r.right && y >= r.top && y < r.bottom).map(|(c, _)| *c)
}

fn maximized(hwnd: HWND) -> bool {
    unsafe { IsZoomed(hwnd).as_bool() }
}

/// WM_NCCALCSIZE: the title row is part of the window's own area; the sides and bottom keep Windows' resize border.
pub fn calc_size(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    unsafe {
        let params = &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS);
        let top = params.rgrc[0].top;
        let result = DefWindowProcW(hwnd, WM_NCCALCSIZE, WPARAM(1), lparam);
        params.rgrc[0].top = top;
        // A maximized window reaches past the screen's edge by its border: keep the title row on the screen.
        if maximized(hwnd) {
            let dpi = GetDpiForWindow(hwnd);
            params.rgrc[0].top += GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
        }
        result
    }
}

/// WM_NCHITTEST: the top edge resizes, the window controls are themselves (so Windows 11 shows its snap layouts over
/// Maximize), and the rest of the title row moves the window.
pub fn hit_test(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        let hit = DefWindowProcW(hwnd, WM_NCHITTEST, wparam, lparam);
        if hit.0 != HTCLIENT as isize {
            return hit;
        }
        let mut pt = POINT { x: (lparam.0 & 0xffff) as i16 as i32, y: ((lparam.0 >> 16) & 0xffff) as i16 as i32 };
        let _ = ScreenToClient(hwnd, &mut pt);
        if !maximized(hwnd) && pt.y < px(hwnd, 5) {
            let mut rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut rect);
            let corner = px(hwnd, 12);
            return LRESULT(if pt.x < corner { HTTOPLEFT } else if pt.x >= rect.right - corner { HTTOPRIGHT } else { HTTOP } as isize);
        }
        match caption_at(hwnd, pt.x, pt.y) {
            Some(Caption::Minimize) => LRESULT(HTMINBUTTON as isize),
            Some(Caption::Maximize) => LRESULT(HTMAXBUTTON as isize),
            Some(Caption::Close) => LRESULT(HTCLOSE as isize),
            None if pt.y < px(hwnd, HEADER) => LRESULT(HTCAPTION as isize),
            None => hit,
        }
    }
}

fn caption_of(hit: usize) -> Option<Caption> {
    match hit as u32 {
        HTMINBUTTON => Some(Caption::Minimize),
        HTMAXBUTTON => Some(Caption::Maximize),
        HTCLOSE => Some(Caption::Close),
        _ => None,
    }
}

fn invalidate_captions(hwnd: HWND) {
    for (_, r) in caption_rects(hwnd) {
        unsafe {
            let _ = InvalidateRect(Some(hwnd), Some(&r), false);
        }
    }
}

/// The window controls' mouse messages. Returns Some when the message was one of theirs.
pub fn caption_mouse(hwnd: HWND, msg: u32, wparam: WPARAM) -> Option<LRESULT> {
    let hit = caption_of(wparam.0);
    match msg {
        WM_NCMOUSEMOVE => {
            if HOT_CAPTION.get() != hit {
                HOT_CAPTION.set(hit);
                invalidate_captions(hwnd);
            }
            if hit.is_some() {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE | TME_NONCLIENT,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                unsafe {
                    let _ = TrackMouseEvent(&mut track);
                }
            }
            None
        }
        WM_NCMOUSELEAVE | WM_MOUSEMOVE => {
            if HOT_CAPTION.get().is_some() || PRESSED_CAPTION.get().is_some() {
                HOT_CAPTION.set(None);
                PRESSED_CAPTION.set(None);
                invalidate_captions(hwnd);
            }
            None
        }
        WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK if hit.is_some() => {
            PRESSED_CAPTION.set(hit);
            invalidate_captions(hwnd);
            Some(LRESULT(0))
        }
        WM_NCLBUTTONUP if hit.is_some() => {
            let pressed = PRESSED_CAPTION.replace(None);
            invalidate_captions(hwnd);
            if pressed == hit {
                let command = match hit {
                    Some(Caption::Minimize) => SC_MINIMIZE,
                    Some(Caption::Maximize) if maximized(hwnd) => SC_RESTORE,
                    Some(Caption::Maximize) => SC_MAXIMIZE,
                    _ => SC_CLOSE,
                };
                unsafe {
                    let _ = PostMessageW(Some(hwnd), WM_SYSCOMMAND, WPARAM(command as usize), LPARAM(0));
                }
            }
            Some(LRESULT(0))
        }
        _ => None,
    }
}

/// The title row, the footer band and the lines between the window's parts.
pub fn paint_bands(hwnd: HWND, dc: HDC, side: i32, active: bool) {
    let p = theme::palette();
    let fonts = theme::fonts(unsafe { GetDpiForWindow(hwnd) });
    let mut rect = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rect);
    }
    let (w, h) = (rect.right, rect.bottom);
    let (bar, foot) = (px(hwnd, HEADER), px(hwnd, FOOTER));
    theme::fill(dc, RECT { left: 0, top: 0, right: w, bottom: bar }, p.chrome);
    theme::fill(dc, RECT { left: 0, top: h - foot, right: w, bottom: h }, p.chrome);
    theme::fill(dc, RECT { left: 0, top: bar - 1, right: w, bottom: bar }, p.edge);
    theme::fill(dc, RECT { left: 0, top: h - foot - 1, right: w, bottom: h - foot }, p.footer_line);
    if side > 0 {
        theme::fill(dc, RECT { left: side, top: bar, right: side + 1, bottom: h - foot - 1 }, p.edge);
    }
    // The title, in the Mac's place at the row's left (the window controls having moved to the right).
    let title = RECT { left: px(hwnd, 18), top: 0, right: w / 2, bottom: bar };
    theme::text(dc, "Herb's Download Manager", title, fonts.bold, if active { p.text } else { p.secondary },
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
    paint_captions(hwnd, dc, active, &p);
}

fn paint_captions(hwnd: HWND, dc: HDC, active: bool, p: &Palette) {
    let font = icon_font(hwnd, 10);
    for (c, r) in caption_rects(hwnd) {
        let hot = HOT_CAPTION.get() == Some(c);
        let pressed = PRESSED_CAPTION.get() == Some(c);
        let (back, fore) = match (c, hot, pressed) {
            (Caption::Close, _, true) => (theme::blend(COLORREF(0x1C2BC4), p.chrome, 0.2), COLORREF(0xFFFFFF)),
            (Caption::Close, true, _) => (COLORREF(0x1C2BC4), COLORREF(0xFFFFFF)),
            (_, _, true) => (p.pressed, p.text),
            (_, true, _) => (p.hover, p.text),
            _ => (p.chrome, if active { p.text } else { p.secondary }),
        };
        theme::fill(dc, RECT { bottom: r.bottom - 1, ..r }, back);
        let glyph = match c {
            Caption::Minimize => "\u{E921}",
            Caption::Maximize if maximized(hwnd) => "\u{E923}",
            Caption::Maximize => "\u{E922}",
            Caption::Close => "\u{E8BB}",
        };
        theme::text(dc, glyph, r, font, fore, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    }
}

/// Segoe Fluent Icons at a size, for this window's DPI (kept for the app's life).
fn icon_font(hwnd: HWND, size: i32) -> HFONT {
    thread_local! {
        static MADE: RefCell<Vec<(i32, isize)>> = const { RefCell::new(Vec::new()) };
    }
    let height = size * scale(hwnd) / 96;
    MADE.with(|m| {
        if let Some((_, f)) = m.borrow().iter().find(|(h, _)| *h == height) {
            return HFONT(*f as *mut _);
        }
        let mut lf = LOGFONTW { lfHeight: -height, lfQuality: CLEARTYPE_QUALITY, ..Default::default() };
        let face: Vec<u16> = "Segoe Fluent Icons".encode_utf16().collect();
        lf.lfFaceName[..face.len()].copy_from_slice(&face);
        let f = unsafe { CreateFontIndirectW(&lf) };
        m.borrow_mut().push((height, f.0 as isize));
        f
    })
}

// --- Toolbar ------------------------------------------------------------------------------------------------------

/// Follows the pointer over a toolbar button, so it shows its hover shape (owner-drawn buttons get no hot state).
unsafe extern "system" fn tool_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _: usize, _: usize) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEMOVE if HOT_TOOL.get() != hwnd.0 as isize => {
                let old = HWND(HOT_TOOL.replace(hwnd.0 as isize) as *mut _);
                if !old.is_invalid() {
                    let _ = InvalidateRect(Some(old), None, false);
                }
                let _ = InvalidateRect(Some(hwnd), None, false);
                let mut track = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                let _ = TrackMouseEvent(&mut track);
            }
            WM_MOUSELEAVE => {
                if HOT_TOOL.get() == hwnd.0 as isize {
                    HOT_TOOL.set(0);
                }
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            // No background flash under the drawn button.
            WM_ERASEBKGND => return LRESULT(1),
            // A toolbar button never keeps the keyboard, as on the Mac: it goes back to where it was once the click is
            // done. Not before: a button that loses the keyboard while pressed lets go and never reports the click.
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                FOCUS_BEFORE_TOOL.set(GetFocus().0 as isize);
            }
            WM_LBUTTONUP => {
                let r = DefSubclassProc(hwnd, msg, wparam, lparam);
                // Unless the command put the keyboard somewhere itself (Search's find field).
                let previous = HWND(FOCUS_BEFORE_TOOL.replace(0) as *mut _);
                if GetFocus() == hwnd && !previous.is_invalid() && previous != hwnd && IsWindow(Some(previous)).as_bool() {
                    let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(previous));
                }
                return r;
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}

pub fn track_tool(button: HWND) {
    unsafe {
        let _ = SetWindowSubclass(button, Some(tool_proc), 0x4844_544C, 0);
    }
}

/// Which symbol a toolbar button shows, after the Mac's SF Symbols.
#[derive(Clone, Copy)]
pub enum Symbol {
    /// link.badge.plus
    NewDownload,
    /// play.fill
    Resume,
    /// pause.fill
    Pause,
    /// text.line.first.and.arrowtriangle.forward
    StartQueue,
    /// stop.circle
    StopQueue,
    /// stop.fill
    StopAll,
    Delete,
    Settings,
    /// pin, or pin.fill tinted while on
    KeepOnTop(bool),
    Search,
    Menu,
}

/// WM_DRAWITEM for a toolbar button: a rounded shape under the pointer or while pressed, then the symbol in the
/// toolbar's grey (dimmed when the command doesn't apply).
pub fn draw_tool(item: &DRAWITEMSTRUCT, symbol: Symbol) {
    let p = theme::palette();
    let hwnd = item.hwndItem;
    let dc = item.hDC;
    let r = item.rcItem;
    let disabled = item.itemState.0 & ODS_DISABLED.0 != 0;
    let pressed = item.itemState.0 & ODS_SELECTED.0 != 0;
    let hot = HOT_TOOL.get() == hwnd.0 as isize && !disabled;
    theme::fill(dc, r, p.chrome);
    if pressed || hot {
        theme::round_rect(dc, r, px(hwnd, 6), if pressed { p.pressed } else { p.hover });
    }
    let color = if disabled { p.disabled } else { p.icon };
    let big = icon_font(hwnd, 16);
    let small = icon_font(hwnd, 9);
    let center = |s: &str, font: HFONT, color: COLORREF| theme::text(dc, s, r, font, color, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    let s = |v: i32| px(hwnd, v);
    let (cx, cy) = ((r.left + r.right) / 2, (r.top + r.bottom) / 2);
    match symbol {
        Symbol::NewDownload => {
            // The link a little up and left, a small plus at its lower right, cut out of the link.
            let link = RECT { left: r.left - s(3), top: r.top - s(2), right: r.right - s(3), bottom: r.bottom - s(2) };
            theme::text(dc, "\u{E71B}", link, big, color, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            let badge = RECT { left: cx + s(2), top: cy + s(1), right: cx + s(12), bottom: cy + s(11) };
            theme::round_rect(dc, badge, s(5), if pressed { p.pressed } else if hot { p.hover } else { p.chrome });
            theme::text(dc, "\u{E710}", badge, small, color, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
        Symbol::Resume => center("\u{F5B0}", icon_font(hwnd, 14), color),
        Symbol::Pause => center("\u{F8AE}", icon_font(hwnd, 14), color),
        Symbol::StartQueue => {
            // Three lines, the first shorter after a small triangle.
            let (left, top) = (cx - s(8), cy - s(7));
            let line = |x0: i32, y: i32| theme::fill(dc, RECT { left: x0, top: y, right: left + s(16), bottom: y + s(2).max(2) }, color);
            theme::text(dc, "\u{F5B0}", RECT { left, top: top - s(1), right: left + s(7), bottom: top + s(5) }, icon_font(hwnd, 6), color,
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            line(left + s(8), top + s(1));
            line(left, top + s(6));
            line(left, top + s(11));
        }
        Symbol::StopQueue => {
            center("\u{EA3A}", big, color);
            center("\u{E978}", icon_font(hwnd, 6), color);
        }
        Symbol::StopAll => center("\u{EE95}", icon_font(hwnd, 13), color),
        Symbol::Delete => center("\u{E74D}", big, color),
        Symbol::Settings => center("\u{E713}", big, color),
        Symbol::KeepOnTop(on) => center(if on { "\u{E841}" } else { "\u{E718}" }, big, if on && !disabled { p.accent } else { color }),
        Symbol::Search => center("\u{E721}", big, color),
        Symbol::Menu => center("\u{E700}", big, color),
    }
}

// --- Sidebar -------------------------------------------------------------------------------------------------------

thread_local! {
    /// Each sidebar line's count, by its item data (filter × 8 + file type + 1, 0 for the filter itself).
    pub static COUNTS: RefCell<Vec<usize>> = RefCell::new(vec![0; 40]);
}

fn filter_symbol(f: Filter) -> &'static str {
    match f {
        Filter::All => "\u{E8B7}",
        Filter::Downloading => "\u{E74B}",
        Filter::Paused => "\u{E769}",
        Filter::Finished => "\u{E930}",
        Filter::Unfinished => "\u{E81C}",
    }
}

fn kind_symbol(k: FileKind) -> &'static str {
    match k {
        FileKind::Compressed => "\u{F012}",
        FileKind::Documents => "\u{E8A5}",
        FileKind::Music => "\u{EC4F}",
        FileKind::Pictures => "\u{EB9F}",
        FileKind::Video => "\u{E8B2}",
        FileKind::Programs => "\u{ECAA}",
        FileKind::Other => "\u{E7C3}",
    }
}

/// NM_CUSTOMDRAW from the sidebar's tree: each line drawn here.
pub fn sidebar_draw(lparam: LPARAM) -> LRESULT {
    unsafe {
        let draw = &mut *(lparam.0 as *mut NMTVCUSTOMDRAW);
        let stage = draw.nmcd.dwDrawStage;
        if stage == CDDS_PREPAINT {
            return LRESULT(CDRF_NOTIFYITEMDRAW as isize);
        }
        if stage != CDDS_ITEMPREPAINT {
            return LRESULT(CDRF_DODEFAULT as isize);
        }
        let p = theme::palette();
        let tree = draw.nmcd.hdr.hwndFrom;
        let dc = draw.nmcd.hdc;
        let r = draw.nmcd.rc;
        let item = HTREEITEM(draw.nmcd.dwItemSpec as isize);
        let data = draw.nmcd.lItemlParam.0 as usize;
        let (filter, kind) = (Filter::ALL[(data / 8).min(4)], (data % 8 != 0).then(|| FileKind::ALL[data % 8 - 1]));
        let level = draw.iLevel;
        let s = |v: i32| px(tree, v);
        let selected = SendMessageW(tree, TVM_GETNEXTITEM, Some(WPARAM(TVGN_CARET as usize)), None).0 == item.0;
        let expanded = SendMessageW(tree, TVM_GETITEMSTATE, Some(WPARAM(item.0 as usize)), Some(LPARAM(TVIS_EXPANDED.0 as isize))).0 as u32
            & TVIS_EXPANDED.0
            != 0;
        theme::fill(dc, r, p.chrome);
        if selected {
            let shape = RECT { left: r.left + s(10), top: r.top + s(1), right: r.right - s(10), bottom: r.bottom - s(1) };
            theme::round_rect(dc, shape, s(5), p.sidebar_selection);
        }
        let fonts = theme::fonts(GetDpiForWindow(tree));
        let indent = s(16) * level;
        // The disclosure chevron of a filter line.
        if kind.is_none() {
            let chevron = RECT { left: r.left + s(12), top: r.top, right: r.left + s(24), bottom: r.bottom };
            theme::text(dc, if expanded { "\u{E70D}" } else { "\u{E76C}" }, chevron, icon_font(tree, 8), p.secondary,
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
        let x = r.left + s(26) + indent;
        let symbol = RECT { left: x, top: r.top, right: x + s(18), bottom: r.bottom };
        theme::text(dc, kind.map(kind_symbol).unwrap_or(filter_symbol(filter)), symbol, icon_font(tree, 14), p.text,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        let count = COUNTS.with(|c| c.borrow().get(data).copied().unwrap_or(0));
        let count_rect = RECT { left: r.right - s(70), top: r.top, right: r.right - s(16), bottom: r.bottom };
        theme::text(dc, &count.to_string(), count_rect, fonts.small, p.secondary, DT_RIGHT | DT_VCENTER | DT_SINGLELINE);
        let title = kind.map(|k| k.title()).unwrap_or(filter.title());
        let title_rect = RECT { left: x + s(24), top: r.top, right: r.right - s(44), bottom: r.bottom };
        theme::text(dc, title, title_rect, if kind.is_none() { fonts.bold } else { fonts.text }, p.text,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
        LRESULT(CDRF_SKIPDEFAULT as isize)
    }
}

// --- The download list ----------------------------------------------------------------------------------------------

thread_local! {
    /// The rows as the list shows them, in order, for drawing (kept apart from the window's state, which may be busy
    /// while the list paints).
    pub static ROWS: RefCell<Vec<Row>> = const { RefCell::new(Vec::new()) };
    /// The system's small file icons, and which column is sorted which way.
    pub static IMAGES: Cell<isize> = const { Cell::new(0) };
    pub static SORT: Cell<(usize, bool)> = const { Cell::new((7, false)) };
    pub static EMPTY_TEXT: RefCell<&'static str> = const { RefCell::new("No downloads yet") };
    pub static CELL_TEXT: Cell<Option<fn(usize, &Row) -> String>> = const { Cell::new(None) };
    pub static ICON_OF: Cell<Option<fn(&str) -> i32>> = const { Cell::new(None) };
}

/// The list column's left and right edge, in the list's coordinates (columns can be dragged into another order and
/// scrolled sideways, so their place comes from the column titles).
fn column_edges(list: HWND, header: HWND, column: usize) -> Option<(i32, i32)> {
    unsafe {
        let mut r = RECT::default();
        if SendMessageW(header, HDM_GETITEMRECT, Some(WPARAM(column)), Some(LPARAM(&mut r as *mut _ as isize))).0 == 0 || r.right <= r.left {
            return None;
        }
        let mut pts = [POINT { x: r.left, y: 0 }, POINT { x: r.right, y: 0 }];
        MapWindowPoints(Some(header), Some(list), &mut pts);
        Some((pts[0].x, pts[1].x))
    }
}

/// The middle of a long name gives way to "…", so its start and its extension both show (the Mac's name column).
fn middle_ellipsis(dc: HDC, text: &str, room: i32) -> String {
    let width = |s: &str| -> i32 {
        let w: Vec<u16> = s.encode_utf16().collect();
        let mut size = SIZE::default();
        unsafe {
            let _ = GetTextExtentPoint32W(dc, &w, &mut size);
        }
        size.cx
    };
    if width(text) <= room {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let keep = (lo + hi).div_ceil(2);
        let head = keep - keep / 2;
        let tail = keep / 2;
        let s: String = chars[..head].iter().chain(std::iter::once(&'…')).chain(&chars[chars.len() - tail..]).collect();
        if width(&s) <= room { lo = keep } else { hi = keep - 1 }
    }
    let (head, tail) = (lo - lo / 2, lo / 2);
    chars[..head].iter().chain(std::iter::once(&'…')).chain(&chars[chars.len() - tail..]).collect()
}

/// NM_CUSTOMDRAW from the list: every row drawn here, cell by cell.
pub fn list_draw(lparam: LPARAM, columns: usize) -> LRESULT {
    unsafe {
        let draw = &mut *(lparam.0 as *mut NMLVCUSTOMDRAW);
        let stage = draw.nmcd.dwDrawStage;
        if stage == CDDS_PREPAINT {
            return LRESULT(CDRF_NOTIFYITEMDRAW as isize);
        }
        if stage != CDDS_ITEMPREPAINT {
            return LRESULT(CDRF_DODEFAULT as isize);
        }
        let list = draw.nmcd.hdr.hwndFrom;
        let index = draw.nmcd.dwItemSpec;
        let Some(row) = ROWS.with(|r| r.borrow().get(index).cloned()) else { return LRESULT(CDRF_DODEFAULT as isize) };
        let p = theme::palette();
        let dc = draw.nmcd.hdc;
        let s = |v: i32| px(list, v);
        let mut bounds = RECT { left: LVIR_BOUNDS as i32, ..Default::default() };
        SendMessageW(list, LVM_GETITEMRECT, Some(WPARAM(index)), Some(LPARAM(&mut bounds as *mut _ as isize)));
        let mut client = RECT::default();
        let _ = GetClientRect(list, &mut client);
        let row_rect = RECT { left: client.left, right: client.right.max(bounds.right), ..bounds };
        let selected = SendMessageW(list, LVM_GETITEMSTATE, Some(WPARAM(index)), Some(LPARAM(LVIS_SELECTED.0 as isize))).0 != 0;
        let focused = GetFocus() == list;
        let back = match (selected, focused) {
            (true, true) => p.accent,
            (true, false) => p.sidebar_selection,
            _ if index % 2 == 1 => p.stripe,
            _ => p.list,
        };
        theme::fill(dc, row_rect, back);
        let fore = if selected && focused { COLORREF(0xFFFFFF) } else { p.text };
        let fonts = theme::fonts(GetDpiForWindow(list));
        let header = HWND(SendMessageW(list, LVM_GETHEADER, None, None).0 as *mut _);
        let cell_text = CELL_TEXT.get();
        for column in 0..columns {
            let Some((left, right)) = column_edges(list, header, column) else { continue };
            let cell = RECT { left: left + s(6), top: bounds.top, right: right - s(4), bottom: bounds.bottom };
            if column == 0 {
                // L1: icon, name, and a thin progress bar under the name while unfinished.
                let icon = ICON_OF.get().map(|f| f(&row.name)).unwrap_or(0);
                let images = IMAGES.get();
                let size = GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForWindow(list));
                if images != 0 {
                    let _ = ImageList_Draw(HIMAGELIST(images), icon, dc, cell.left, (cell.top + cell.bottom - size) / 2, ILD_TRANSPARENT);
                }
                let text_left = cell.left + size + s(6);
                let bar = row.status != Status::Finished && (row.fraction.is_some() || row.status.is_running());
                let old = SelectObject(dc, fonts.text.into());
                let name = middle_ellipsis(dc, &row.name, (cell.right - text_left).max(0));
                SelectObject(dc, old);
                if bar {
                    let name_rect = RECT { left: text_left, top: cell.top + s(3), right: cell.right, bottom: cell.top + s(19) };
                    theme::text(dc, &name, name_rect, fonts.text, fore, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                    let track = RECT { left: text_left, top: cell.top + s(20), right: cell.right, bottom: cell.top + s(24) };
                    let track_color = if selected && focused { theme::blend(p.accent, COLORREF(0xFFFFFF), 0.35) } else { p.track };
                    let fill_color = if selected && focused { COLORREF(0xFFFFFF) } else { p.accent };
                    theme::round_rect(dc, track, s(2), track_color);
                    match row.fraction {
                        Some(f) => {
                            let width = ((track.right - track.left) as f64 * f.clamp(0.0, 1.0)) as i32;
                            if width > 0 {
                                theme::round_rect(dc, RECT { right: track.left + width.max(s(4)), ..track }, s(2), fill_color);
                            }
                        }
                        None => {
                            // No size yet: a short piece moving along the track while it runs.
                            let span = (track.right - track.left).max(1);
                            let piece = span / 4;
                            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
                            let x = (t as i64 / 8 % (span + piece) as i64) as i32 - piece;
                            let left = (track.left + x).max(track.left);
                            let right = (track.left + x + piece).min(track.right);
                            if right > left {
                                theme::round_rect(dc, RECT { left, right, ..track }, s(2), fill_color);
                            }
                        }
                    }
                } else {
                    theme::text(dc, &name, RECT { left: text_left, ..cell }, fonts.text, fore, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                }
                continue;
            }
            let text = cell_text.map(|f| f(column, &row)).unwrap_or_default();
            let color = if column == 1 && matches!(row.status, Status::Failed(_)) && !(selected && focused) { p.red } else { fore };
            theme::text(dc, &text, cell, fonts.text, color, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
        }
        LRESULT(CDRF_SKIPDEFAULT as isize)
    }
}

/// The list's own messages: its column titles drawn as the Mac's, the stripes going on under the last row, and "No
/// downloads yet" (or "No downloads match") when it's empty.
unsafe extern "system" fn list_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _: usize, _: usize) -> LRESULT {
    unsafe {
        match msg {
            WM_NOTIFY => {
                let header = &*(lparam.0 as *const NMHDR);
                if header.code == NM_CUSTOMDRAW {
                    return header_draw(hwnd, lparam);
                }
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            WM_PAINT => {
                let r = DefSubclassProc(hwnd, msg, wparam, lparam);
                let dc = GetDC(Some(hwnd));
                paint_below_rows(hwnd, dc);
                ReleaseDC(Some(hwnd), dc);
                r
            }
            WM_PRINTCLIENT => {
                let r = DefSubclassProc(hwnd, msg, wparam, lparam);
                paint_below_rows(hwnd, HDC(wparam.0 as *mut _));
                r
            }
            // The selection's colour follows whether the list has the keyboard, as on the Mac.
            WM_SETFOCUS | WM_KILLFOCUS => {
                let r = DefSubclassProc(hwnd, msg, wparam, lparam);
                let _ = InvalidateRect(Some(hwnd), None, false);
                r
            }
            _ => DefSubclassProc(hwnd, msg, wparam, lparam),
        }
    }
}

pub fn subclass_list(list: HWND) {
    unsafe {
        let _ = SetWindowSubclass(list, Some(list_proc), 0x4844_4C53, 0);
    }
}

unsafe fn paint_below_rows(list: HWND, dc: HDC) {
    unsafe {
        let p = theme::palette();
        let mut client = RECT::default();
        let _ = GetClientRect(list, &mut client);
        let header = HWND(SendMessageW(list, LVM_GETHEADER, None, None).0 as *mut _);
        let mut hr = RECT::default();
        let _ = GetWindowRect(header, &mut hr);
        let header_bottom = hr.bottom - hr.top;
        let count = SendMessageW(list, LVM_GETITEMCOUNT, None, None).0 as usize;
        let top_index = SendMessageW(list, LVM_GETTOPINDEX, None, None).0 as usize;
        let row = px(list, LIST_ROW);
        let mut y = header_bottom;
        let mut i = top_index;
        if count > 0 {
            let mut last = RECT { left: LVIR_BOUNDS as i32, ..Default::default() };
            SendMessageW(list, LVM_GETITEMRECT, Some(WPARAM(count - 1)), Some(LPARAM(&mut last as *mut _ as isize)));
            y = last.bottom;
            i = count;
        }
        while y < client.bottom {
            theme::fill(dc, RECT { left: 0, top: y, right: client.right, bottom: (y + row).min(client.bottom) }, if i % 2 == 1 { p.stripe } else { p.list });
            y += row;
            i += 1;
        }
        if count == 0 {
            let fonts = theme::fonts(GetDpiForWindow(list));
            let _ = fonts;
            let text = EMPTY_TEXT.with(|t| *t.borrow());
            let area = RECT { left: 0, top: header_bottom, right: client.right, bottom: client.bottom };
            theme::text(dc, text, area, empty_font(list), p.secondary, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
    }
}

fn empty_font(hwnd: HWND) -> HFONT {
    thread_local! {
        static MADE: Cell<(i32, isize)> = const { Cell::new((0, 0)) };
    }
    let height = 17 * scale(hwnd) / 96;
    let (h, f) = MADE.get();
    if h == height {
        return HFONT(f as *mut _);
    }
    let mut lf = LOGFONTW { lfHeight: -height, lfWeight: 600, lfQuality: CLEARTYPE_QUALITY, ..Default::default() };
    let face: Vec<u16> = "Segoe UI Variable Text".encode_utf16().collect();
    lf.lfFaceName[..face.len()].copy_from_slice(&face);
    let font = unsafe { CreateFontIndirectW(&lf) };
    MADE.set((height, font.0 as isize));
    font
}

/// The column titles: on the list's colour, small, with faint lines between them, a line under them, and the sorted
/// column's chevron at its right.
unsafe fn header_draw(list: HWND, lparam: LPARAM) -> LRESULT {
    unsafe {
        let draw = &mut *(lparam.0 as *mut NMCUSTOMDRAW);
        let p = theme::palette();
        let header = draw.hdr.hwndFrom;
        if draw.dwDrawStage == CDDS_PREPAINT {
            let mut r = RECT::default();
            let _ = GetClientRect(header, &mut r);
            theme::fill(draw.hdc, r, p.list);
            theme::fill(draw.hdc, RECT { top: r.bottom - 1, ..r }, p.header_line);
            return LRESULT((CDRF_NOTIFYITEMDRAW | CDRF_NOTIFYPOSTPAINT) as isize);
        }
        if draw.dwDrawStage == CDDS_POSTPAINT {
            // The part right of the last column, which the header would paint in its own colour.
            let mut r = RECT::default();
            let _ = GetClientRect(header, &mut r);
            let count = SendMessageW(header, HDM_GETITEMCOUNT, None, None).0.max(0) as usize;
            let mut right = 0;
            for i in 0..count {
                let mut item = RECT::default();
                SendMessageW(header, HDM_GETITEMRECT, Some(WPARAM(i)), Some(LPARAM(&mut item as *mut _ as isize)));
                right = right.max(item.right);
            }
            if right < r.right {
                theme::fill(draw.hdc, RECT { left: right, top: r.top, right: r.right, bottom: r.bottom - 1 }, p.list);
                theme::fill(draw.hdc, RECT { left: right, top: r.bottom - 1, right: r.right, bottom: r.bottom }, p.header_line);
            }
            return LRESULT(CDRF_DODEFAULT as isize);
        }
        if draw.dwDrawStage != CDDS_ITEMPREPAINT {
            return LRESULT(CDRF_DODEFAULT as isize);
        }
        let column = draw.dwItemSpec;
        let r = draw.rc;
        let s = |v: i32| px(list, v);
        theme::fill(draw.hdc, RECT { bottom: r.bottom - 1, ..r }, p.list);
        if r.right - r.left > 0 {
            theme::fill(draw.hdc, RECT { left: r.right - 1, top: r.top + s(6), right: r.right, bottom: r.bottom - s(6) }, p.header_line);
        }
        let mut buffer = [0u16; 64];
        let mut item = HDITEMW { mask: HDI_TEXT, pszText: windows::core::PWSTR(buffer.as_mut_ptr()), cchTextMax: 64, ..Default::default() };
        SendMessageW(header, HDM_GETITEMW, Some(WPARAM(column)), Some(LPARAM(&mut item as *mut _ as isize)));
        let n = buffer.iter().position(|&c| c == 0).unwrap_or(0);
        let title = String::from_utf16_lossy(&buffer[..n]);
        let fonts = theme::fonts(GetDpiForWindow(list));
        let (sorted, ascending) = SORT.get();
        let room = RECT { left: r.left + s(6), top: r.top, right: r.right - s(if sorted == column { 20 } else { 4 }), bottom: r.bottom - 1 };
        theme::text(draw.hdc, &title, room, fonts.small, p.text, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
        if sorted == column {
            let chevron = RECT { left: r.right - s(20), top: r.top, right: r.right - s(6), bottom: r.bottom - 1 };
            theme::text(draw.hdc, if ascending { "\u{E70E}" } else { "\u{E70D}" }, chevron, icon_font(list, 8), p.secondary,
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
        LRESULT(CDRF_SKIPDEFAULT as isize)
    }
}

/// The list's rows are 30 pixels high: a list takes its row height from its small image list.
pub fn row_height(list: HWND) {
    unsafe {
        let images = ImageList_Create(1, px(list, LIST_ROW), ILC_COLOR32, 1, 0);
        SendMessageW(list, LVM_SETIMAGELIST, Some(WPARAM(LVSIL_SMALL as usize)), Some(LPARAM(images.0)));
    }
}

/// A few pixels at a control's edge pass the pointer through to the window (HTTRANSPARENT), so the sidebar's
/// one-pixel edge is easy to grab: `which` is 1 for the right edge, 2 for the left.
unsafe extern "system" fn edge_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _: usize, which: usize) -> LRESULT {
    unsafe {
        if msg == WM_NCHITTEST {
            let mut pt = POINT { x: (lparam.0 & 0xffff) as i16 as i32, y: ((lparam.0 >> 16) & 0xffff) as i16 as i32 };
            let _ = ScreenToClient(hwnd, &mut pt);
            let mut r = RECT::default();
            let _ = GetClientRect(hwnd, &mut r);
            let grip = px(hwnd, 3);
            let header_height = {
                let header = HWND(SendMessageW(hwnd, LVM_GETHEADER, None, None).0 as *mut _);
                let mut hr = RECT::default();
                if !header.is_invalid() {
                    let _ = GetWindowRect(header, &mut hr);
                }
                hr.bottom - hr.top
            };
            let near = if which == 1 { pt.x >= r.right - grip } else { pt.x < grip && pt.y > header_height };
            if near {
                return LRESULT(HTTRANSPARENT as isize);
            }
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}

pub fn grip_edge(control: HWND, right: bool) {
    unsafe {
        let _ = SetWindowSubclass(control, Some(edge_proc), 0x4844_4547, if right { 1 } else { 2 });
    }
}

// --- A segmented control (the Download window's Info | Connections | Mirrors | Options, as the Mac's) -------------

/// Its labels are its window text, split at "|"; it answers TCM_GETCURSEL and TCM_SETCURSEL and tells its parent of a
/// change with TCN_SELCHANGE, as a tab control does.
pub fn register_segments() {
    unsafe {
        let class = WNDCLASSW {
            lpfnWndProc: Some(segments_proc),
            hInstance: windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default().into(),
            lpszClassName: windows::core::w!("HerbsDMSegments"),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            style: CS_HREDRAW | CS_VREDRAW,
            ..Default::default()
        };
        RegisterClassW(&class);
    }
}

fn segment_labels(hwnd: HWND) -> Vec<String> {
    let mut buffer = [0u16; 256];
    let n = unsafe { GetWindowTextW(hwnd, &mut buffer) } as usize;
    String::from_utf16_lossy(&buffer[..n]).split('|').map(str::to_string).collect()
}

/// The segments' rectangles: as wide as their labels need, centred in the control.
fn segment_rects(hwnd: HWND) -> Vec<RECT> {
    let labels = segment_labels(hwnd);
    let mut r = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut r);
        let dc = GetDC(Some(hwnd));
        let old = SelectObject(dc, theme::fonts(GetDpiForWindow(hwnd)).text.into());
        let widths: Vec<i32> = labels
            .iter()
            .map(|l| {
                let w: Vec<u16> = l.encode_utf16().collect();
                let mut size = SIZE::default();
                let _ = GetTextExtentPoint32W(dc, &w, &mut size);
                size.cx + px(hwnd, 24)
            })
            .collect();
        SelectObject(dc, old);
        ReleaseDC(Some(hwnd), dc);
        let total: i32 = widths.iter().sum();
        let mut x = (r.right - total) / 2;
        widths
            .iter()
            .map(|w| {
                let seg = RECT { left: x, top: r.top, right: x + w, bottom: r.bottom };
                x += w;
                seg
            })
            .collect()
    }
}

unsafe extern "system" fn segments_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        let selected = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as usize;
        let select = |i: usize, notify: bool| {
            let count = segment_labels(hwnd).len();
            if i < count {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, i as isize);
                let _ = InvalidateRect(Some(hwnd), None, false);
                if notify && let Ok(parent) = GetParent(hwnd) {
                    let header = NMHDR { hwndFrom: hwnd, idFrom: GetDlgCtrlID(hwnd) as usize, code: TCN_SELCHANGE };
                    SendMessageW(parent, WM_NOTIFY, Some(WPARAM(header.idFrom)), Some(LPARAM(&header as *const _ as isize)));
                }
            }
        };
        match msg {
            TCM_GETCURSEL => LRESULT(selected as isize),
            TCM_SETCURSEL => {
                select(wparam.0, false);
                LRESULT(selected as isize)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = ((lparam.0 & 0xffff) as i16 as i32, ((lparam.0 >> 16) & 0xffff) as i16 as i32);
                if let Some(i) = segment_rects(hwnd).iter().position(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom) {
                    let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(hwnd));
                    select(i, true);
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                let count = segment_labels(hwnd).len();
                match wparam.0 as u16 {
                    0x25 if selected > 0 => select(selected - 1, true),
                    0x27 if selected + 1 < count => select(selected + 1, true),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_GETDLGCODE => LRESULT(DLGC_WANTARROWS as isize),
            WM_SETFOCUS | WM_KILLFOCUS => {
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT | WM_PRINTCLIENT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = if msg == WM_PAINT { BeginPaint(hwnd, &mut ps) } else { HDC(wparam.0 as *mut _) };
                let p = theme::palette();
                let mut r = RECT::default();
                let _ = GetClientRect(hwnd, &mut r);
                theme::fill(dc, r, p.chrome);
                let rects = segment_rects(hwnd);
                if let (Some(first), Some(last)) = (rects.first(), rects.last()) {
                    let whole = RECT { left: first.left, top: r.top, right: last.right, bottom: r.bottom };
                    let (track, chosen) = if p.dark { (COLORREF(0x00303030), COLORREF(0x00656565)) } else { (COLORREF(0x00E3E3E3), COLORREF(0x00FFFFFF)) };
                    theme::round_rect(dc, whole, px(hwnd, 6), track);
                    let fonts = theme::fonts(GetDpiForWindow(hwnd));
                    for (i, (seg, label)) in rects.iter().zip(segment_labels(hwnd)).enumerate() {
                        if i == selected {
                            let inner = RECT { left: seg.left + 2, top: seg.top + 2, right: seg.right - 2, bottom: seg.bottom - 2 };
                            theme::round_rect(dc, inner, px(hwnd, 5), chosen);
                        } else if i + 1 < rects.len() && i + 1 != selected {
                            theme::fill(dc, RECT { left: seg.right - 1, top: seg.top + px(hwnd, 6), right: seg.right, bottom: seg.bottom - px(hwnd, 6) }, p.header_line);
                        }
                        theme::text(dc, &label, *seg, fonts.text, p.text, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                    }
                    if windows::Win32::UI::Input::KeyboardAndMouse::GetFocus() == hwnd {
                        let focus = RECT { left: whole.left - 2, top: whole.top, right: whole.right + 2, bottom: whole.bottom };
                        let _ = FrameRect(dc, &focus, theme::brush(p.accent));
                    }
                }
                if msg == WM_PAINT {
                    let _ = EndPaint(hwnd, &ps);
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
