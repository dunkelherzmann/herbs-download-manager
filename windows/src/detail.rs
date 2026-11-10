//! The Download window (§6), Add Mirror… (§7.1) and Settings (§9) on Windows.
//!
//! The Download window: the file's segments (an owner-drawn bar), its numbers, Pause/Resume and Close, and the Info,
//! Connections, Mirrors and Options tabs (a tab control whose pages show and hide their own controls). One window per
//! download; it refreshes twice a second and closing it never stops the download. Settings apply as they change.

use std::cell::RefCell;
use std::collections::HashMap;

use hd_core::format;
use hd_core::manager::{Detail, Status};
use hd_core::schedule::{self, WhenDone};
use hd_core::settings::{Settings, WhenFileExists};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, PWSTR, w};

use crate::add::{BUTTON, Ctx, DEFAULT, EDIT, LABEL, choose_folder, set_text, text_of, wide};

const TIMER_REFRESH: usize = 1;
// Download window controls.
const D_BAR: u16 = 10;
const D_PERCENT: u16 = 11;
const D_NUMBERS: u16 = 12;
const D_TOGGLE: u16 = 13;
const D_CLOSE: u16 = 2; // IDCANCEL: Esc closes
const D_TABS: u16 = 15;
const D_INFO: u16 = 20; // + row, values at D_INFO + 10 + row
const D_CONNS: u16 = 40;
const D_PLUS: u16 = 41;
const D_MINUS: u16 = 42;
const D_SOURCES: u16 = 43;
const D_ADD_MIRROR: u16 = 44;
const D_REMOVE: u16 = 45;
const D_CONN_LABEL: u16 = 46;
const D_CONN_COUNT: u16 = 47;
const D_LIMIT_ON: u16 = 48;
const D_LIMIT: u16 = 49;
const D_UNIT: u16 = 50;
const D_ALGO_LABEL: u16 = 51;
const D_ALGO: u16 = 52;
const D_VALUE: u16 = 53;
const D_NOTE: u16 = 54;
// Add Mirror.
const M_TEXT: u16 = 60;
const M_ADD: u16 = 1; // IDOK
const M_CANCEL: u16 = 2;
// Settings.
const S_FOLDER: u16 = 70;
const S_CHOOSE: u16 = 71;
const S_RESET: u16 = 72;
const S_AT_ONCE: u16 = 73;
const S_LIMIT_ON: u16 = 74;
const S_LIMIT: u16 = 75;
const S_UNIT: u16 = 76;
const S_CONNECTIONS: u16 = 77;
const S_PER_SERVER: u16 = 78;
const S_RETRIES: u16 = 79;
const S_EXISTS: u16 = 80;
const S_NOTIFY: u16 = 81;
const S_CHECKSUM_FILES: u16 = 82;
const S_LIMITS: u16 = 83;
const S_FORGET: u16 = 84;
const S_SIGN_INS: u16 = 85;
const S_MANAGE: u16 = 86;
const S_SERVER_DATE: u16 = 87;
const S_START_ON: u16 = 88;
const S_START: u16 = 89;
const S_STOP_ON: u16 = 90;
const S_STOP: u16 = 91;
const S_WHEN_DONE: u16 = 92;
/// Monday to Sunday: S_DAY..S_DAY + 6.
const S_DAY: u16 = 93;
const S_EXTENSION: u16 = 100;
const S_CLOSE: u16 = 2;

const INFO_KEYS: [&str; 7] = ["Link:", "Folder:", "Size:", "Resumable:", "Date Added:", "Date Finished:", "Checksum:"];
const ALGORITHMS: [&str; 5] = ["None", "MD5", "SHA-1", "SHA-256", "SHA-512"];
/// Which controls each tab shows.
const PAGES: [&[u16]; 4] = [
    &[D_INFO, D_INFO + 1, D_INFO + 2, D_INFO + 3, D_INFO + 4, D_INFO + 5, D_INFO + 6,
      D_INFO + 10, D_INFO + 11, D_INFO + 12, D_INFO + 13, D_INFO + 14, D_INFO + 15, D_INFO + 16],
    &[D_CONNS, D_PLUS, D_MINUS],
    &[D_SOURCES, D_ADD_MIRROR, D_REMOVE],
    &[D_CONN_LABEL, D_CONN_COUNT, D_LIMIT_ON, D_LIMIT, D_UNIT, D_ALGO_LABEL, D_ALGO, D_VALUE, D_NOTE],
];

enum Kind {
    Download { id: u64, detail: Option<Detail> },
    Mirror { id: u64 },
    Settings,
}

struct Win {
    ctx: Ctx,
    kind: Kind,
    controls: HashMap<u16, HWND>,
}

thread_local! {
    static WINDOWS: RefCell<HashMap<isize, Win>> = RefCell::new(HashMap::new());
    static OPEN: RefCell<HashMap<u64, HWND>> = RefCell::new(HashMap::new());
    static SETTINGS: RefCell<Option<HWND>> = const { RefCell::new(None) };
}

/// The window's state; skipped when a control notifies while it's being changed (see CLAUDE.md).
fn with_win<R>(hwnd: HWND, f: impl FnOnce(&mut Win) -> R) -> Option<R> {
    WINDOWS.with(|w| w.try_borrow_mut().ok()?.get_mut(&(hwnd.0 as isize)).map(f))
}

type Spec<'a> = (u16, PCWSTR, &'a str, u32, i32, i32, i32, i32);

fn create(ctx: &Ctx, kind: Kind, title: &str, size: (i32, i32), controls: &[Spec]) -> HWND {
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class = WNDCLASSW {
            lpfnWndProc: Some(proc_),
            hInstance: instance.into(),
            lpszClassName: w!("HerbsDMDetail"),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            ..Default::default()
        };
        RegisterClassW(&class);
        let scale = GetDpiForWindow(ctx.owner).max(96) as i32;
        let px = |v: i32| v * scale / 96;
        let mut rect = RECT { left: 0, top: 0, right: px(size.0), bottom: px(size.1) };
        let _ = AdjustWindowRectEx(&mut rect, WS_CAPTION | WS_SYSMENU, false, WINDOW_EX_STYLE::default());
        let t = wide(title);
        let hwnd = CreateWindowExW(WS_EX_CONTROLPARENT, w!("HerbsDMDetail"), PCWSTR(t.as_ptr()), WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN,
                                   CW_USEDEFAULT, CW_USEDEFAULT, rect.right - rect.left, rect.bottom - rect.top, Some(ctx.owner), None,
                                   Some(instance.into()), None)
            .unwrap_or_default();
        let mut map = HashMap::new();
        for &(id, class, text, style, x, y, w, h) in controls {
            let text = wide(text);
            let ex = if class == WC_LISTVIEWW || class == w!("EDIT") { WS_EX_CLIENTEDGE } else { WINDOW_EX_STYLE::default() };
            let c = CreateWindowExW(ex, class, PCWSTR(text.as_ptr()), WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_CLIPSIBLINGS | WINDOW_STYLE(style),
                                    px(x), px(y), px(w), px(h), Some(hwnd), Some(HMENU(id as usize as *mut _)), Some(instance.into()), None)
                .unwrap_or_default();
            SendMessageW(c, WM_SETFONT, Some(WPARAM(ctx.font.0 as usize)), Some(LPARAM(1)));
            map.insert(id, c);
        }
        WINDOWS.with(|w| w.borrow_mut().insert(hwnd.0 as isize, Win { ctx: ctx.clone(), kind, controls: map }));
        hwnd
    }
}

/// Names (where a control's text isn't one) and hover texts for a window's controls.
fn explain_ids(hwnd: HWND, items: &[(u16, &str, &str)]) {
    with_win(hwnd, |w| {
        for &(id, name, help) in items {
            if let Some(&c) = w.controls.get(&id) {
                if name.is_empty() { crate::a11y::tip(c, help) } else { crate::a11y::explain(c, name, help) }
            }
        }
    });
}

pub(crate) fn columns(list: HWND, names: &[(&str, i32)], checkboxes: bool) {
    unsafe {
        let mut style = LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER;
        if checkboxes {
            style |= LVS_EX_CHECKBOXES;
        }
        SendMessageW(list, LVM_SETEXTENDEDLISTVIEWSTYLE, Some(WPARAM(0)), Some(LPARAM(style as isize)));
        for (i, (name, width)) in names.iter().enumerate() {
            let text = wide(name);
            let c = LVCOLUMNW { mask: LVCF_TEXT | LVCF_WIDTH, cx: *width, pszText: PWSTR(text.as_ptr() as *mut _), ..Default::default() };
            SendMessageW(list, LVM_INSERTCOLUMNW, Some(WPARAM(i)), Some(LPARAM(&c as *const _ as isize)));
        }
    }
}

/// Rewrites a ListView's rows (adding or removing rows only when their number changes).
pub(crate) fn fill(list: HWND, rows: &[Vec<String>]) {
    unsafe {
        let have = SendMessageW(list, LVM_GETITEMCOUNT, None, None).0 as usize;
        if have != rows.len() {
            SendMessageW(list, LVM_DELETEALLITEMS, None, None);
        }
        for (i, cells) in rows.iter().enumerate() {
            for (col, text) in cells.iter().enumerate() {
                let text = wide(text);
                let item = LVITEMW { mask: LVIF_TEXT, iItem: i as i32, iSubItem: col as i32, pszText: PWSTR(text.as_ptr() as *mut _), ..Default::default() };
                let msg = if col == 0 && have != rows.len() { LVM_INSERTITEMW } else { LVM_SETITEMW };
                SendMessageW(list, msg, None, Some(LPARAM(&item as *const _ as isize)));
            }
        }
    }
}

fn combo(h: HWND, items: &[&str], selected: usize) {
    unsafe {
        for it in items {
            let t = wide(it);
            SendMessageW(h, CB_ADDSTRING, None, Some(LPARAM(t.as_ptr() as isize)));
        }
        SendMessageW(h, CB_SETCURSEL, Some(WPARAM(selected)), None);
    }
}

fn checked(h: HWND) -> bool {
    unsafe { SendMessageW(h, BM_GETCHECK, None, None).0 == 1 }
}

fn set_checked(h: HWND, on: bool) {
    unsafe {
        SendMessageW(h, BM_SETCHECK, Some(WPARAM(on as usize)), None);
    }
}

fn selection(h: HWND) -> usize {
    unsafe { SendMessageW(h, CB_GETCURSEL, None, None).0.max(0) as usize }
}

/// Opens (or brings forward) a download's window, on tab 0 Info, 1 Connections, 2 Mirrors or 3 Options.
pub fn open(ctx: &Ctx, id: u64, tab: Option<usize>) {
    if let Some(h) = OPEN.with(|o| o.borrow().get(&id).copied())
        && unsafe { IsWindow(Some(h)).as_bool() }
    {
        if let Some(tab) = tab {
            show_tab(h, tab);
        }
        unsafe {
            let _ = SetForegroundWindow(h);
        }
        return;
    }
    let Some(d) = ctx.manager.detail(id) else { return };
    let combo_style = (CBS_DROPDOWNLIST | WS_VSCROLL.0 as i32) as u32;
    let mut specs: Vec<Spec> = vec![
        (D_BAR, w!("STATIC"), "", 0xD /* SS_OWNERDRAW */, 12, 12, 656, 18),
        (D_PERCENT, w!("STATIC"), "", 0, 12, 40, 60, 22),
        (D_NUMBERS, w!("STATIC"), "", 0, 76, 42, 400, 22),
        (D_TOGGLE, w!("BUTTON"), "Pause", BUTTON, 490, 36, 85, 30),
        (D_CLOSE, w!("BUTTON"), "Close", BUTTON, 583, 36, 85, 30),
        // Just the strip of tabs: the pages sit below it, so nothing overlaps (WM_PRINT pictures and the screen agree).
        (D_TABS, WC_TABCONTROLW, "", 0, 12, 76, 656, 28),
        (D_CONNS, WC_LISTVIEWW, "", LVS_REPORT | LVS_NOSORTHEADER, 22, 110, 636, 290),
        (D_PLUS, w!("BUTTON"), "+", BUTTON, 22, 408, 36, 28),
        (D_MINUS, w!("BUTTON"), "−", BUTTON, 62, 408, 36, 28),
        (D_SOURCES, WC_LISTVIEWW, "", LVS_REPORT | LVS_NOSORTHEADER | LVS_SINGLESEL | LVS_SHOWSELALWAYS, 22, 110, 636, 290),
        (D_ADD_MIRROR, w!("BUTTON"), "Add Mirror…", BUTTON, 22, 408, 110, 28),
        (D_REMOVE, w!("BUTTON"), "Remove", BUTTON, 140, 408, 90, 28),
        (D_CONN_LABEL, w!("STATIC"), "Connections:", LABEL, 22, 124, 110, 20),
        (D_CONN_COUNT, w!("EDIT"), "", EDIT | ES_NUMBER as u32, 140, 120, 60, 24),
        (D_LIMIT_ON, w!("BUTTON"), "Limit to", BS_AUTOCHECKBOX as u32, 22, 160, 110, 24),
        (D_LIMIT, w!("EDIT"), "500", EDIT | ES_NUMBER as u32, 140, 160, 90, 24),
        (D_UNIT, w!("COMBOBOX"), "", combo_style, 238, 160, 80, 120),
        (D_ALGO_LABEL, w!("STATIC"), "Checksum:", LABEL, 22, 202, 110, 20),
        (D_ALGO, w!("COMBOBOX"), "", combo_style, 140, 198, 100, 160),
        (D_VALUE, w!("EDIT"), "", EDIT, 248, 198, 410, 24),
        (D_NOTE, w!("STATIC"), "", 0, 140, 230, 518, 20),
    ];
    let info_labels: Vec<String> = INFO_KEYS.iter().map(|k| k.to_string()).collect();
    for (i, k) in info_labels.iter().enumerate() {
        specs.push((D_INFO + i as u16, w!("STATIC"), k, LABEL, 22, 116 + 30 * i as i32, 110, 22));
        specs.push((D_INFO + 10 + i as u16, w!("EDIT"), "", (ES_READONLY | ES_AUTOHSCROLL) as u32, 140, 113 + 30 * i as i32, 510, 24));
    }
    let hwnd = create(ctx, Kind::Download { id, detail: None }, &d.row.name, (680, 458), &specs);
    with_win(hwnd, |w| unsafe {
        for (i, name) in ["Info", "Connections", "Mirrors", "Options"].iter().enumerate() {
            let t = wide(name);
            let item = TCITEMW { mask: TCIF_TEXT, pszText: PWSTR(t.as_ptr() as *mut _), ..Default::default() };
            SendMessageW(w.controls[&D_TABS], TCM_INSERTITEMW, Some(WPARAM(i)), Some(LPARAM(&item as *const _ as isize)));
        }
        columns(w.controls[&D_CONNS], &[("Source", 150), ("Range", 170), ("Downloaded", 90), ("Speed", 90), ("State", 120)], false);
        columns(w.controls[&D_SOURCES], &[("Source", 300), ("State", 110), ("Active", 60), ("Speed", 80), ("Served", 80)], true);
        combo(w.controls[&D_UNIT], &["KB/s", "MB/s"], 0);
        let algo = d.checksum.as_ref().map(|c| 1 + hd_core::checksum::Algorithm::ALL.iter().position(|a| *a == c.algorithm).unwrap_or(0)).unwrap_or(0);
        combo(w.controls[&D_ALGO], &ALGORITHMS, algo);
        if let Some(c) = &d.checksum {
            set_text(w.controls[&D_VALUE], &c.value);
        }
        SendMessageW(w.controls[&D_VALUE], EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(w!("Paste the expected value").0 as isize)));
        if let Some(l) = d.speed_limit {
            let mb = l >= 1_000_000 && l % 100_000 == 0;
            set_checked(w.controls[&D_LIMIT_ON], true);
            SendMessageW(w.controls[&D_UNIT], CB_SETCURSEL, Some(WPARAM(mb as usize)), None);
            set_text(w.controls[&D_LIMIT], &(l / if mb { 1_000_000 } else { 1_000 }).to_string());
        }
        set_text(w.controls[&D_CONN_COUNT], &d.row.allowed_connections.to_string());
    });
    explain_ids(hwnd, &[
        (D_TOGGLE, "", "Pause or resume this download (Ctrl+. / Ctrl+R)"),
        (D_CLOSE, "", "Close this window; the download carries on (Esc)"),
        (D_TABS, "Pages", "Info, Connections, Mirrors and Options"),
        (D_CONNS, "Connections", "Each connection: its server, the part it fetches, how much it got, its speed and state"),
        (D_PLUS, "Add a connection", "Add a connection now"),
        (D_MINUS, "Remove a connection", "Remove a connection now"),
        (D_SOURCES, "Sources", "The link and its mirrors: untick one to stop using it"),
        (D_ADD_MIRROR, "", "Add other links to the same file (Ctrl+Alt+M)"),
        (D_REMOVE, "", "Remove the selected mirror (the original link stays)"),
        (D_CONN_COUNT, "", "Connections for this download (1–32), applied when you leave the field"),
        (D_LIMIT_ON, "", "Limit this download's speed"),
        (D_LIMIT, "Speed limit", "The most this download may fetch each second"),
        (D_UNIT, "Speed limit unit", "Kilobytes or megabytes a second"),
        (D_ALGO, "", "The kind of checksum: MD5, SHA-1, SHA-256 or SHA-512"),
        (D_VALUE, "Checksum value", "The value the site publishes for this file; it's checked once every byte is in"),
    ]);
    show_tab(hwnd, tab.unwrap_or(0));
    refresh(hwnd);
    unsafe {
        SetTimer(Some(hwnd), TIMER_REFRESH, 500, None);
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
    }
    OPEN.with(|o| o.borrow_mut().insert(id, hwnd));
}

fn show_tab(hwnd: HWND, tab: usize) {
    with_win(hwnd, |w| unsafe {
        SendMessageW(w.controls[&D_TABS], TCM_SETCURSEL, Some(WPARAM(tab)), None);
        for (i, page) in PAGES.iter().enumerate() {
            for id in page.iter() {
                if let Some(c) = w.controls.get(id) {
                    let _ = ShowWindow(*c, if i == tab { SW_SHOW } else { SW_HIDE });
                }
            }
        }
    });
}

fn refresh(hwnd: HWND) {
    let Some((manager, id)) = with_win(hwnd, |w| match w.kind {
        Kind::Download { id, .. } => Some((w.ctx.manager.clone(), id)),
        _ => None,
    })
    .flatten() else { return };
    let Some(d) = manager.detail(id) else {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        return;
    };
    with_win(hwnd, |w| unsafe {
        let r = &d.row;
        set_text(hwnd, &r.name);
        let finished = r.status == Status::Finished;
        set_text(w.controls[&D_PERCENT], &match r.fraction {
            Some(f) => format!("{}%", (f * 100.0).floor() as u32),
            None if finished => "100%".into(),
            None => String::new(),
        });
        let left = format::time_left_text(r.time_left);
        let numbers: Vec<String> = [format::size_text(r.downloaded, r.size, finished), format::speed_text(r.speed),
                                    if left.is_empty() { String::new() } else { format!("{left} left") }, r.status_text.clone()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();
        set_text(w.controls[&D_NUMBERS], &numbers.join("  ·  "));
        set_text(w.controls[&D_TOGGLE], if r.status.is_running() { "Pause" } else { "Resume" });
        let _ = EnableWindow(w.controls[&D_TOGGLE], r.status.is_running() || matches!(r.status, Status::Waiting | Status::Paused | Status::Failed(_)));
        let values = [
            r.url.clone(),
            r.folder.display().to_string(),
            r.size.map(format::bytes).unwrap_or_else(|| "Unknown".into()),
            if r.resumable { "Yes".into() } else { "No: pausing starts it over".into() },
            crate::app::date_text(Some(r.added)),
            r.finished.map(|f| crate::app::date_text(Some(f))).unwrap_or_else(|| "—".into()),
            d.verified.clone().unwrap_or_else(|| match &d.checksum {
                Some(c) => format!("{}, checked when the download finishes", c.algorithm.name()),
                None => "—".into(),
            }),
        ];
        for (i, v) in values.iter().enumerate() {
            let c = w.controls[&(D_INFO + 10 + i as u16)];
            if text_of(c) != *v {
                set_text(c, v);
            }
        }
        let _ = EnableWindow(w.controls[&D_PLUS], !finished && r.allowed_connections < 32);
        let _ = EnableWindow(w.controls[&D_MINUS], !finished && r.allowed_connections > 1);
        let count = w.controls[&D_CONN_COUNT];
        if windows::Win32::UI::Input::KeyboardAndMouse::GetFocus() != count && text_of(count) != r.allowed_connections.to_string() {
            set_text(count, &r.allowed_connections.to_string());
        }
        let host = |u: &str| u.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or(u).to_string();
        let conns: Vec<Vec<String>> = d
            .connections
            .iter()
            .map(|c| {
                let range = if c.end == 0 { String::new() } else { format!("{}–{}", format::bytes(c.start), format::bytes(c.end)) };
                vec![host(&c.source), range, if c.bytes > 0 { format::bytes(c.bytes) } else { String::new() }, format::speed_text(c.speed), c.state.clone()]
            })
            .collect();
        fill(w.controls[&D_CONNS], &conns);
        let sources: Vec<Vec<String>> = d
            .sources
            .iter()
            .map(|s| {
                let original = if s.url == r.url { " (original)" } else { "" };
                vec![format!("{}{original}", s.url), s.status.text(), s.active.to_string(), format::speed_text(s.speed),
                     if s.bytes > 0 { format::bytes(s.bytes) } else { String::new() }]
            })
            .collect();
        let list = w.controls[&D_SOURCES];
        fill(list, &sources);
        for (i, s) in d.sources.iter().enumerate() {
            let state = LVITEMW {
                stateMask: LVIS_STATEIMAGEMASK,
                state: LIST_VIEW_ITEM_STATE_FLAGS(((if s.enabled { 2 } else { 1 }) << 12) as u32),
                ..Default::default()
            };
            SendMessageW(list, LVM_SETITEMSTATE, Some(WPARAM(i)), Some(LPARAM(&state as *const _ as isize)));
        }
        let _ = InvalidateRect(Some(w.controls[&D_BAR]), None, false);
        if let Kind::Download { detail, .. } = &mut w.kind {
            *detail = Some(d);
        }
    });
}

/// The header's bar: each segment's downloaded part filled (green while a connection works on it), the gaps still to
/// fetch left grey.
fn draw_bar(item: &DRAWITEMSTRUCT, d: &Detail) {
    unsafe {
        let r = item.rcItem;
        let brush = |rgb: (u8, u8, u8)| CreateSolidBrush(COLORREF(rgb.0 as u32 | (rgb.1 as u32) << 8 | (rgb.2 as u32) << 16));
        let (grey, blue, green) = (brush((220, 220, 220)), brush((61, 133, 227)), brush((77, 179, 102)));
        FillRect(item.hDC, &r, grey);
        let w = (r.right - r.left) as f64;
        let size = d.row.size.unwrap_or(0);
        if d.row.status == Status::Finished {
            FillRect(item.hDC, &r, blue);
        } else if size > 0 {
            for s in &d.segments {
                let x0 = r.left + (s.start as f64 / size as f64 * w) as i32;
                let x1 = r.left + (s.pos as f64 / size as f64 * w) as i32;
                if x1 > x0 {
                    let part = RECT { left: x0, top: r.top, right: x1, bottom: r.bottom };
                    FillRect(item.hDC, &part, if s.owner.is_some() { green } else { blue });
                }
            }
        }
        for b in [grey, blue, green] {
            let _ = DeleteObject(b.into());
        }
    }
}

/// Settings' Sign-ins line, when Settings is open.
pub fn show_sign_ins_summary(text: &str) {
    if let Some(h) = SETTINGS.with(|s| *s.borrow())
        && unsafe { IsWindow(Some(h)).as_bool() }
    {
        with_win(h, |w| set_text(w.controls[&S_SIGN_INS], text));
    }
}

/// Add Mirror… (§7.1 M2, M3): one link per line.
pub fn ask_mirrors(ctx: &Ctx, id: u64) {
    let Some(d) = ctx.manager.detail(id) else { return };
    let multi = (ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32 | WS_VSCROLL.0;
    let note = format!("Other links to “{}”, one per line. Each is checked before it's used.", d.row.name);
    let hwnd = create(ctx, Kind::Mirror { id }, "Add Mirror", (480, 220), &[
        (0, w!("STATIC"), &note, 0, 12, 12, 456, 36),
        (M_TEXT, w!("EDIT"), "", multi, 12, 52, 456, 110),
        (M_CANCEL, w!("BUTTON"), "Cancel", BUTTON, 290, 176, 85, 30),
        (M_ADD, w!("BUTTON"), "Add", DEFAULT, 383, 176, 85, 30),
    ]);
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        if let Some(t) = with_win(hwnd, |w| w.controls[&M_TEXT]) {
            let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(t));
        }
    }
}

const DONE: [(&str, WhenDone); 4] =
    [("Do nothing", WhenDone::Nothing), ("Quit", WhenDone::Quit), ("Sleep", WhenDone::Sleep), ("Shut down", WhenDone::ShutDown)];
const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// Turns the scheduler's time boxes and days on or off with their checkboxes.
fn enable_scheduler(w: &Win) {
    let (start, stop) = (checked(w.controls[&S_START_ON]), checked(w.controls[&S_STOP_ON]));
    unsafe {
        let _ = EnableWindow(w.controls[&S_START], start);
        let _ = EnableWindow(w.controls[&S_STOP], stop);
        for d in 0..7 {
            let _ = EnableWindow(w.controls[&(S_DAY + d)], start || stop);
        }
    }
}

const EXISTS: [(&str, WhenFileExists); 4] = [
    ("Ask", WhenFileExists::Ask),
    ("Keep both", WhenFileExists::KeepBoth),
    ("Skip the new link", WhenFileExists::Skip),
    ("Replace the existing file", WhenFileExists::Replace),
];

/// Settings (§9, Ctrl+,).
pub fn settings(ctx: &Ctx) {
    if let Some(h) = SETTINGS.with(|s| *s.borrow())
        && unsafe { IsWindow(Some(h)).as_bool() }
    {
        unsafe {
            let _ = SetForegroundWindow(h);
        }
        return;
    }
    let s = ctx.manager.settings();
    let combo_style = (CBS_DROPDOWNLIST | WS_VSCROLL.0 as i32) as u32;
    let folder = crate::save_folder(&ctx.manager).display().to_string();
    let (limits_text, limits_detail) = hd_core::hosts::describe(&ctx.manager.host_limits());
    let sign_ins = crate::signins::summary(ctx);
    let start_text = schedule::time_text(s.schedule.start.unwrap_or(60));
    let stop_text = schedule::time_text(s.schedule.stop.unwrap_or(7 * 60));
    let hwnd = create(ctx, Kind::Settings, "Settings", (560, 734), &[
        (0, w!("STATIC"), "Save downloads to:", LABEL, 12, 18, 170, 20),
        (S_FOLDER, w!("STATIC"), &folder, 0x4000 /* SS_PATHELLIPSIS */, 190, 18, 350, 20),
        (S_CHOOSE, w!("BUTTON"), "Choose…", BUTTON, 190, 44, 100, 28),
        (S_RESET, w!("BUTTON"), "Use Downloads", BUTTON, 298, 44, 130, 28),
        (0, w!("STATIC"), "Downloads at once:", LABEL, 12, 92, 170, 20),
        (S_AT_ONCE, w!("EDIT"), &s.downloads_at_once.to_string(), EDIT | ES_NUMBER as u32, 190, 88, 60, 24),
        (0, w!("STATIC"), "Speed limit (all):", LABEL, 12, 126, 170, 20),
        (S_LIMIT_ON, w!("BUTTON"), "Limit to", BS_AUTOCHECKBOX as u32, 190, 124, 90, 24),
        (S_LIMIT, w!("EDIT"), "500", EDIT | ES_NUMBER as u32, 284, 122, 90, 24),
        (S_UNIT, w!("COMBOBOX"), "", combo_style, 382, 122, 80, 120),
        (0, w!("STATIC"), "Connections per download:", LABEL, 12, 166, 170, 20),
        (S_CONNECTIONS, w!("EDIT"), &s.connections.to_string(), EDIT | ES_NUMBER as u32, 190, 162, 60, 24),
        (0, w!("STATIC"), "Connections per server:", LABEL, 12, 200, 170, 20),
        (S_PER_SERVER, w!("EDIT"), &s.connections_per_server.to_string(), EDIT | ES_NUMBER as u32, 190, 196, 60, 24),
        (0, w!("STATIC"), "Retries per source:", LABEL, 12, 234, 170, 20),
        (S_RETRIES, w!("EDIT"), &s.retries.to_string(), EDIT | ES_NUMBER as u32, 190, 230, 60, 24),
        (0, w!("STATIC"), "When a file exists:", LABEL, 12, 270, 170, 20),
        (S_EXISTS, w!("COMBOBOX"), "", combo_style, 190, 266, 230, 160),
        (0, w!("STATIC"), "Scheduler:", LABEL, 12, 306, 170, 20),
        (S_START_ON, w!("BUTTON"), "Start the queue at", BS_AUTOCHECKBOX as u32, 190, 302, 146, 24),
        (S_START, w!("EDIT"), &start_text, EDIT, 338, 300, 64, 24),
        (S_STOP_ON, w!("BUTTON"), "Stop the queue at", BS_AUTOCHECKBOX as u32, 190, 332, 146, 24),
        (S_STOP, w!("EDIT"), &stop_text, EDIT, 338, 330, 64, 24),
        (0, w!("STATIC"), "On:", LABEL, 12, 366, 170, 20),
        (S_DAY, w!("BUTTON"), DAYS[0], BS_AUTOCHECKBOX as u32, 190, 362, 50, 24),
        (S_DAY + 1, w!("BUTTON"), DAYS[1], BS_AUTOCHECKBOX as u32, 240, 362, 50, 24),
        (S_DAY + 2, w!("BUTTON"), DAYS[2], BS_AUTOCHECKBOX as u32, 290, 362, 50, 24),
        (S_DAY + 3, w!("BUTTON"), DAYS[3], BS_AUTOCHECKBOX as u32, 340, 362, 50, 24),
        (S_DAY + 4, w!("BUTTON"), DAYS[4], BS_AUTOCHECKBOX as u32, 390, 362, 50, 24),
        (S_DAY + 5, w!("BUTTON"), DAYS[5], BS_AUTOCHECKBOX as u32, 440, 362, 50, 24),
        (S_DAY + 6, w!("BUTTON"), DAYS[6], BS_AUTOCHECKBOX as u32, 490, 362, 50, 24),
        (0, w!("STATIC"), "When the queue is done:", LABEL, 12, 400, 170, 20),
        (S_WHEN_DONE, w!("COMBOBOX"), "", combo_style, 190, 396, 160, 160),
        (S_CHECKSUM_FILES, w!("BUTTON"), "Look for checksum files next to the link", BS_AUTOCHECKBOX as u32, 190, 438, 340, 24),
        (S_SERVER_DATE, w!("BUTTON"), "Keep the server's date for finished files", BS_AUTOCHECKBOX as u32, 190, 466, 340, 24),
        (S_NOTIFY, w!("BUTTON"), "Show a notification when a download finishes", BS_AUTOCHECKBOX as u32, 190, 494, 340, 24),
        (0, w!("STATIC"), "Server limits:", LABEL, 12, 530, 170, 20),
        (S_LIMITS, w!("STATIC"), &limits_text, 0x0100 /* SS_NOTIFY, for its hover text */, 190, 530, 250, 20),
        (S_FORGET, w!("BUTTON"), "Forget", BUTTON, 445, 524, 95, 28),
        (0, w!("STATIC"), "Sign-ins:", LABEL, 12, 564, 170, 20),
        (S_SIGN_INS, w!("STATIC"), &sign_ins, 0, 190, 564, 250, 20),
        (S_MANAGE, w!("BUTTON"), "Manage…", BUTTON, 445, 558, 95, 28),
        (0, w!("STATIC"), "Browser extension:", LABEL, 12, 598, 170, 20),
        (S_EXTENSION, w!("BUTTON"), "Show Extension Folder", BUTTON, 190, 592, 190, 28),
        (0, w!("STATIC"), "Chrome, Edge, Brave and Vivaldi: Extensions ▸ Developer mode ▸ Load Unpacked, and choose the folder. Firefox: about:debugging ▸ This Firefox ▸ Load Temporary Add-on, and choose manifest.json in it.", 0, 190, 626, 350, 52),
        (S_CLOSE, w!("BUTTON"), "Close", DEFAULT, 455, 690, 85, 30),
    ]);
    explain_ids(hwnd, &[
        (S_CHOOSE, "", "Choose the folder new downloads are saved to"),
        (S_RESET, "", "Save new downloads to your Downloads folder"),
        (S_AT_ONCE, "", "Downloads the queue runs at the same time (1–10)"),
        (S_LIMIT_ON, "", "Limit the speed of all downloads together"),
        (S_LIMIT, "Speed limit", "The most all downloads together may fetch each second"),
        (S_UNIT, "Speed limit unit", "Kilobytes or megabytes a second"),
        (S_CONNECTIONS, "", "The most connections a new download uses (1–32); New Download can choose another number"),
        (S_PER_SERVER, "", "Connections a download opens to any one server (1–32), from its next start"),
        (S_RETRIES, "", "Times a link or mirror is tried again before it counts as failed (0–20)"),
        (S_EXISTS, "", "What happens when a different file already has a new download's name"),
        (S_CHECKSUM_FILES, "", "With no checksum set, check the file with the site's file.sha256 (or .sha512, .sha1, .md5)"),
        (S_NOTIFY, "", "Show a notification each time a download finishes"),
        (S_SERVER_DATE, "", "A finished file gets the date the server gives it (Last-Modified) instead of the time it finished downloading"),
        (S_START_ON, "", "Start the queue at this time (hours:minutes) on the days chosen below"),
        (S_START, "Time to start the queue", "Hours and minutes, such as 01:30"),
        (S_STOP_ON, "", "Stop the queue at this time; running downloads wait in the queue for the next start. A time before the start time is the next morning's"),
        (S_STOP, "Time to stop the queue", "Hours and minutes, such as 07:00"),
        (S_WHEN_DONE, "", "Once the running queue has nothing left to download: quit the app, or put the computer to sleep or shut it down, after a minute's countdown you can cancel"),
        (S_DAY, "", "Start the queue on Mondays"),
        (S_DAY + 1, "", "Start the queue on Tuesdays"),
        (S_DAY + 2, "", "Start the queue on Wednesdays"),
        (S_DAY + 3, "", "Start the queue on Thursdays"),
        (S_DAY + 4, "", "Start the queue on Fridays"),
        (S_DAY + 5, "", "Start the queue on Saturdays"),
        (S_DAY + 6, "", "Start the queue on Sundays"),
        (S_EXTENSION, "", "Open the folder with the extension that hands Chrome's, Edge's, Brave's and Firefox's downloads to the app"),
        (S_LIMITS, "Server limits", limits_detail.as_str()),
        (S_FORGET, "", "Forget every server's remembered limit; downloads learn them again"),
        (S_MANAGE, "Manage sign-ins", "See, add and change the user names, passwords, cookies and referers saved for sites, and the Administrator Password that encrypts them"),
        (S_CLOSE, "", "Close Settings; changes are already saved"),
    ]);
    with_win(hwnd, |w| {
        combo(w.controls[&S_UNIT], &["KB/s", "MB/s"], 0);
        combo(w.controls[&S_EXISTS], &EXISTS.map(|e| e.0), EXISTS.iter().position(|e| e.1 == s.when_file_exists).unwrap_or(0));
        if let Some(l) = s.speed_limit {
            let mb = l >= 1_000_000 && l % 100_000 == 0;
            set_checked(w.controls[&S_LIMIT_ON], true);
            unsafe {
                SendMessageW(w.controls[&S_UNIT], CB_SETCURSEL, Some(WPARAM(mb as usize)), None);
            }
            set_text(w.controls[&S_LIMIT], &(l / if mb { 1_000_000 } else { 1_000 }).to_string());
        }
        set_checked(w.controls[&S_NOTIFY], s.notify_when_finished);
        set_checked(w.controls[&S_CHECKSUM_FILES], s.look_for_checksum_files);
        set_checked(w.controls[&S_SERVER_DATE], s.keep_server_date);
        combo(w.controls[&S_WHEN_DONE], &DONE.map(|d| d.0), DONE.iter().position(|d| d.1 == s.schedule.when_done).unwrap_or(0));
        set_checked(w.controls[&S_START_ON], s.schedule.start.is_some());
        set_checked(w.controls[&S_STOP_ON], s.schedule.stop.is_some());
        for (d, on) in s.schedule.days.iter().enumerate() {
            set_checked(w.controls[&(S_DAY + d as u16)], *on);
        }
        enable_scheduler(w);
        unsafe {
            let _ = EnableWindow(w.controls[&S_FORGET], !w.ctx.manager.host_limits().is_empty());
        }
    });
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
    }
    SETTINGS.with(|st| *st.borrow_mut() = Some(hwnd));
}

/// Reads the Settings window into the saved settings (numbers out of range are brought into it by the core).
fn save_settings(hwnd: HWND) {
    with_win(hwnd, |w| {
        let number = |id: u16, fallback: u32| text_of(w.controls[&id]).trim().parse::<u32>().unwrap_or(fallback);
        let current = w.ctx.manager.settings();
        let limit = checked(w.controls[&S_LIMIT_ON]).then(|| {
            number(S_LIMIT, 500) as u64 * if selection(w.controls[&S_UNIT]) == 1 { 1_000_000 } else { 1_000 }
        });
        let s = Settings {
            downloads_at_once: number(S_AT_ONCE, current.downloads_at_once),
            speed_limit: limit,
            connections: number(S_CONNECTIONS, current.connections),
            connections_per_server: number(S_PER_SERVER, current.connections_per_server),
            retries: number(S_RETRIES, current.retries),
            when_file_exists: EXISTS[selection(w.controls[&S_EXISTS]).min(3)].1,
            notify_when_finished: checked(w.controls[&S_NOTIFY]),
            look_for_checksum_files: checked(w.controls[&S_CHECKSUM_FILES]),
            keep_server_date: checked(w.controls[&S_SERVER_DATE]),
            schedule: {
                // A time that doesn't read as hours:minutes keeps the one saved before, and the box shows it again.
                let time = |on: u16, edit: u16, before: Option<u32>, fallback: u32| {
                    let typed = schedule::parse_time(&text_of(w.controls[&edit]));
                    let value = typed.or(before).unwrap_or(fallback);
                    set_text(w.controls[&edit], &schedule::time_text(value));
                    checked(w.controls[&on]).then_some(value)
                };
                let mut q = current.schedule.clone();
                q.start = time(S_START_ON, S_START, current.schedule.start, 60);
                q.stop = time(S_STOP_ON, S_STOP, current.schedule.stop, 7 * 60);
                q.days = std::array::from_fn(|d| checked(w.controls[&(S_DAY + d as u16)]));
                q.when_done = DONE[selection(w.controls[&S_WHEN_DONE]).min(3)].1;
                q
            },
            ..current
        };
        enable_scheduler(w);
        w.ctx.manager.set_settings(s);
    });
}

/// §6: the checksum to verify; a pasted value picks its algorithm by length.
fn apply_checksum(hwnd: HWND, id: u64, guess: bool) {
    with_win(hwnd, |w| {
        let text = text_of(w.controls[&D_VALUE]).trim().to_string();
        if guess && let Some(a) = hd_core::checksum::Algorithm::of_value(&text) {
            let i = 1 + hd_core::checksum::Algorithm::ALL.iter().position(|x| *x == a).unwrap_or(0);
            unsafe {
                SendMessageW(w.controls[&D_ALGO], CB_SETCURSEL, Some(WPARAM(i)), None);
            }
        }
        let chosen = selection(w.controls[&D_ALGO]).checked_sub(1).map(|i| hd_core::checksum::Algorithm::ALL[i]);
        let note = match chosen.filter(|_| !text.is_empty()) {
            None => {
                w.ctx.manager.set_checksum(id, None);
                String::new()
            }
            Some(a) => match hd_core::checksum::Checksum::new(a, &text) {
                Ok(c) => {
                    w.ctx.manager.set_checksum(id, Some(c));
                    "Checked once every byte is in, before the file gets its name".into()
                }
                Err(e) => e,
            },
        };
        set_text(w.controls[&D_NOTE], &note);
    });
}

fn apply_limit(hwnd: HWND, id: u64) {
    with_win(hwnd, |w| {
        let n = text_of(w.controls[&D_LIMIT]).trim().parse::<u64>().unwrap_or(500);
        let limit = checked(w.controls[&D_LIMIT_ON]).then(|| n * if selection(w.controls[&D_UNIT]) == 1 { 1_000_000 } else { 1_000 });
        w.ctx.manager.set_speed_limit(id, limit);
    });
}

extern "system" fn proc_(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TIMER if wparam.0 == TIMER_REFRESH => {
                refresh(hwnd);
                LRESULT(0)
            }
            WM_DRAWITEM => {
                let item = &*(lparam.0 as *const DRAWITEMSTRUCT);
                if item.CtlID as u16 == D_BAR {
                    let d = with_win(hwnd, |w| match &w.kind {
                        Kind::Download { detail, .. } => detail.clone(),
                        _ => None,
                    })
                    .flatten();
                    if let Some(d) = d {
                        draw_bar(item, &d);
                    }
                }
                LRESULT(1)
            }
            WM_NOTIFY => {
                let header = &*(lparam.0 as *const NMHDR);
                if header.idFrom as u16 == D_TABS && header.code == TCN_SELCHANGE {
                    let tab = with_win(hwnd, |w| SendMessageW(w.controls[&D_TABS], TCM_GETCURSEL, None, None).0 as usize).unwrap_or(0);
                    show_tab(hwnd, tab);
                } else if header.idFrom as u16 == D_SOURCES && header.code == LVN_ITEMCHANGED {
                    // §7.3: a source's checkbox turns it on or off.
                    let change = &*(lparam.0 as *const NMLISTVIEW);
                    if (change.uNewState ^ change.uOldState) & LVIS_STATEIMAGEMASK.0 != 0 && change.uOldState & LVIS_STATEIMAGEMASK.0 != 0 {
                        let on = (change.uNewState & LVIS_STATEIMAGEMASK.0) >> 12 == 2;
                        with_win(hwnd, |w| {
                            if let Kind::Download { id, detail: Some(d), .. } = &w.kind
                                && let Some(s) = d.sources.get(change.iItem as usize)
                                && s.enabled != on
                            {
                                w.ctx.manager.set_source_enabled(*id, &s.url, on);
                            }
                        });
                    }
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                let id = (wparam.0 & 0xffff) as u16;
                let code = ((wparam.0 >> 16) & 0xffff) as u32;
                let kind = with_win(hwnd, |w| match w.kind {
                    Kind::Download { id, .. } => (0, id),
                    Kind::Mirror { id } => (1, id),
                    Kind::Settings => (2, 0),
                });
                match kind {
                    Some((0, did)) => match id {
                        D_CLOSE => drop(DestroyWindow(hwnd)),
                        D_TOGGLE => drop(with_win(hwnd, |w| {
                            let running = matches!(&w.kind, Kind::Download { detail: Some(d), .. } if d.row.status.is_running());
                            if running { w.ctx.manager.pause(&[did]) } else { w.ctx.manager.resume(&[did]) }
                        })),
                        D_PLUS | D_MINUS => drop(with_win(hwnd, |w| {
                            if let Kind::Download { detail: Some(d), .. } = &w.kind {
                                let n = d.row.allowed_connections;
                                w.ctx.manager.set_connections(did, if id == D_PLUS { (n + 1).min(32) } else { n.saturating_sub(1).max(1) });
                            }
                        })),
                        D_CONN_COUNT if code == EN_KILLFOCUS => drop(with_win(hwnd, |w| {
                            if let Ok(n) = text_of(w.controls[&D_CONN_COUNT]).trim().parse::<u32>() {
                                w.ctx.manager.set_connections(did, n.clamp(1, 32));
                            }
                        })),
                        D_LIMIT_ON => apply_limit(hwnd, did),
                        D_LIMIT if code == EN_CHANGE => apply_limit(hwnd, did),
                        D_UNIT if code == CBN_SELCHANGE => apply_limit(hwnd, did),
                        D_VALUE if code == EN_CHANGE => apply_checksum(hwnd, did, true),
                        D_ALGO if code == CBN_SELCHANGE => apply_checksum(hwnd, did, false),
                        D_ADD_MIRROR => {
                            if let Some(ctx) = with_win(hwnd, |w| w.ctx.clone()) {
                                ask_mirrors(&ctx, did);
                            }
                        }
                        D_REMOVE => drop(with_win(hwnd, |w| {
                            let row = SendMessageW(w.controls[&D_SOURCES], LVM_GETNEXTITEM, Some(WPARAM(usize::MAX)), Some(LPARAM(LVNI_SELECTED as isize))).0;
                            if let Kind::Download { detail: Some(d), .. } = &w.kind
                                && row >= 0
                                && let Some(s) = d.sources.get(row as usize)
                                && s.url != d.row.url
                            {
                                w.ctx.manager.remove_source(did, &s.url);
                            }
                        })),
                        _ => {}
                    },
                    Some((1, mid)) => match id {
                        M_CANCEL => drop(DestroyWindow(hwnd)),
                        M_ADD => {
                            let Some((ctx, text)) = with_win(hwnd, |w| (w.ctx.clone(), text_of(w.controls[&M_TEXT]))) else { return LRESULT(0) };
                            let links = hd_core::links::find(&text);
                            if links.is_empty() {
                                return LRESULT(0);
                            }
                            let name = ctx.manager.detail(mid).map(|d| d.row.name).unwrap_or_default();
                            let added = ctx.manager.add_mirrors(mid, &links);
                            let _ = DestroyWindow(hwnd);
                            (ctx.done)(Some(if added == 0 {
                                format!("“{name}” already uses {}", if links.len() == 1 { "that link" } else { "those links" })
                            } else {
                                format!("Added {added} mirror{} to “{name}”", if added == 1 { "" } else { "s" })
                            }));
                        }
                        _ => {}
                    },
                    Some((2, _)) => match id {
                        S_CLOSE => drop(DestroyWindow(hwnd)),
                        S_CHOOSE => {
                            if let Some(dir) = choose_folder(hwnd, "Save downloads to") {
                                with_win(hwnd, |w| {
                                    w.ctx.manager.set_settings(Settings { save_to: Some(dir.display().to_string()), ..w.ctx.manager.settings() });
                                    set_text(w.controls[&S_FOLDER], &dir.display().to_string());
                                });
                            }
                        }
                        S_RESET => drop(with_win(hwnd, |w| {
                            w.ctx.manager.set_settings(Settings { save_to: None, ..w.ctx.manager.settings() });
                            set_text(w.controls[&S_FOLDER], &crate::downloads_folder().display().to_string());
                        })),
                        S_AT_ONCE | S_LIMIT | S_CONNECTIONS | S_PER_SERVER | S_RETRIES | S_START | S_STOP if code == EN_KILLFOCUS => save_settings(hwnd),
                        S_LIMIT_ON | S_NOTIFY | S_CHECKSUM_FILES | S_SERVER_DATE | S_START_ON | S_STOP_ON => save_settings(hwnd),
                        id if (S_DAY..S_DAY + 7).contains(&id) => save_settings(hwnd),
                        S_EXTENSION => match hd_core::browser::extension_folder(&crate::data_folder()) {
                            Ok(folder) => crate::app::open_path(&folder, false),
                            Err(e) => eprintln!("The extension can't be written: {e}"),
                        },
                        S_MANAGE => {
                            if let Some(ctx) = with_win(hwnd, |w| w.ctx.clone()) {
                                crate::signins::open(&ctx);
                            }
                        }
                        S_FORGET => drop(with_win(hwnd, |w| {
                            w.ctx.manager.forget_host_limits();
                            let (summary, detail) = hd_core::hosts::describe(&[]);
                            set_text(w.controls[&S_LIMITS], &summary);
                            crate::a11y::tip(w.controls[&S_LIMITS], &detail);
                            let _ = EnableWindow(w.controls[&S_FORGET], false);
                        })),
                        S_UNIT | S_EXISTS | S_WHEN_DONE if code == CBN_SELCHANGE => save_settings(hwnd),
                        _ => {}
                    },
                    _ => {}
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                // Settings typed but not yet left are saved too.
                if with_win(hwnd, |w| matches!(w.kind, Kind::Settings)).unwrap_or(false) {
                    save_settings(hwnd);
                }
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                if let Some(Kind::Download { id, .. }) = WINDOWS.with(|w| w.borrow_mut().remove(&(hwnd.0 as isize))).map(|w| w.kind) {
                    OPEN.with(|o| o.borrow_mut().remove(&id));
                }
                SETTINGS.with(|s| {
                    if *s.borrow() == Some(hwnd) {
                        *s.borrow_mut() = None;
                    }
                });
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

