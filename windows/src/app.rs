//! The main window (§2–§4): the Main Menu (☰) in the top-left corner, the toolbar, the sidebar filters with their
//! file-type groups (a tree), the download list (a report ListView with the Mac's ten columns), the find bar and the
//! footer. Every command has the Mac's shortcut with Ctrl. The list refreshes twice a second, updating its cells in
//! place so the selection never jumps. `--capture FILE` saves a BMP of the window (WM_PRINT, which works over SSH
//! where PrintWindow returns black) and quits.

use std::cell::RefCell;

use hd_core::format::{self, FileKind, Filter};
use hd_core::manager::{Manager, Row, Status};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, PWSTR, w};

const GITHUB: &str = "https://github.com/dunkelherzmann/herbs-download-manager";

// Command ids: toolbar buttons and menu items share them.
const C_MENU: u16 = 100;
const C_NEW: u16 = 101;
const C_RESUME: u16 = 102;
const C_PAUSE: u16 = 103;
const C_START_QUEUE: u16 = 104;
const C_STOP_QUEUE: u16 = 105;
const C_STOP_ALL: u16 = 106;
const C_DELETE: u16 = 107;
const C_SETTINGS: u16 = 108;
const C_KEEP_ON_TOP: u16 = 109;
const C_FIND: u16 = 110;
const C_IMPORT: u16 = 120;
const C_CLIPBOARD: u16 = 121;
const C_BATCH: u16 = 122;
const C_QUIT: u16 = 123;
const C_COPY_LINK: u16 = 124;
const C_SELECT_ALL: u16 = 125;
const C_ADD_MIRROR: u16 = 126;
const C_PROPERTIES: u16 = 127;
const C_DELETE_FINISHED: u16 = 128;
const C_DELETE_UNFINISHED: u16 = 129;
const C_DELETE_MISSING: u16 = 130;
const C_DELETE_ALL: u16 = 131;
const C_SIDEBAR: u16 = 132;
const C_DOWNLOAD_WINDOW: u16 = 133;
const C_MINIMIZE: u16 = 134;
const C_ZOOM: u16 = 135;
const C_GITHUB: u16 = 136;
const C_ABOUT: u16 = 137;
const C_OPEN: u16 = 138;
const C_SHOW_IN_FOLDER: u16 = 139;
const C_RESTART: u16 = 140;
const C_ADD_CONNECTION: u16 = 141;
const C_REMOVE_CONNECTION: u16 = 142;
const C_FIND_DONE: u16 = 143;
const C_COLUMN: u16 = 160; // + column index
const ID_SIDEBAR: u16 = 200;
const ID_LIST: u16 = 201;
const ID_SEARCH: u16 = 202;
const TIMER_REFRESH: usize = 1;
const TIMER_CAPTURE: usize = 2;

/// Commands that arrive with their own windows later in `windows\pending.md`; their items show, disabled.
const LATER: [u16; 8] = [C_NEW, C_IMPORT, C_CLIPBOARD, C_BATCH, C_ADD_MIRROR, C_PROPERTIES, C_SETTINGS, C_DOWNLOAD_WINDOW];

/// L1–L10: title, width; Date Finished and Location start hidden, as on the Mac.
const COLUMNS: [(&str, i32, bool); 10] = [
    ("Name", 230, true),
    ("Status", 130, true),
    ("Size", 140, true),
    ("Speed", 90, true),
    ("Time Left", 80, true),
    ("Connections", 90, true),
    ("Mirrors", 70, true),
    ("Date Added", 140, true),
    ("Date Finished", 140, false),
    ("Location", 220, false),
];

/// T1–T11 (§3): command, Segoe Fluent Icons glyph, name, hover text. 0 is a separator.
const TOOLS: [(u16, char, &str, &str); 14] = [
    (C_NEW, '\u{E710}', "New Download", "New Download: add a link to download (Ctrl+N)"),
    (0, ' ', "", ""),
    (C_RESUME, '\u{E768}', "Resume", "Resume: continue the selected downloads (Ctrl+R)"),
    (C_PAUSE, '\u{E769}', "Pause", "Pause: stop the selected downloads, keeping what's downloaded (Ctrl+.)"),
    (0, ' ', "", ""),
    (C_START_QUEUE, '\u{E893}', "Start Queue", "Start Queue: download waiting items one after another (Ctrl+Shift+R)"),
    (C_STOP_QUEUE, '\u{E71A}', "Stop Queue", "Stop Queue: stop starting new items from the queue"),
    (0, ' ', "", ""),
    (C_STOP_ALL, '\u{E978}', "Stop All", "Stop All: stop the queue and pause every running download (Ctrl+Alt+.)"),
    (0, ' ', "", ""),
    (C_DELETE, '\u{E74D}', "Delete", "Delete: remove the selected downloads from the list (Delete)"),
    (0, ' ', "", ""),
    (C_SETTINGS, '\u{E713}', "Settings", "Settings (Ctrl+,)"),
    (C_KEEP_ON_TOP, '\u{E718}', "Keep on Top", "Keep on Top: keep Herb's DM above every other app's windows (Ctrl+Alt+T)"),
];

struct State {
    manager: Manager,
    window: HWND,
    menu_button: HWND,
    title: HWND,
    tools: Vec<(u16, HWND)>,
    search_button: HWND,
    sidebar: HWND,
    sidebar_items: Vec<(Filter, Option<FileKind>, isize)>,
    list: HWND,
    find_edit: HWND,
    find_done: HWND,
    footer: HWND,
    filter: Filter,
    kind: Option<FileKind>,
    search: String,
    /// The ids shown, in list order; the cells are rewritten in place while it doesn't change.
    shown: Vec<u64>,
    rows: Vec<Row>,
    sort: (usize, bool),
    columns: [bool; 10],
    widths: [i32; 10],
    find_open: bool,
    /// View ▸ Show Sidebar (IsWindowVisible can't tell before the window first shows).
    sidebar_shown: bool,
    keep_on_top: bool,
    capture: Option<std::path::PathBuf>,
    notice: Option<(String, std::time::Instant)>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.borrow_mut().as_mut().map(f))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Keep on Top is remembered in the data folder (a test copy with %HD_DATA_DIR% keeps its own).
fn keep_on_top_file() -> std::path::PathBuf {
    crate::data_folder().join("keep-on-top")
}

/// "Today at 10:48", "Yesterday at 09:02", or "2026-11-03 17:20".
fn date_text(seconds: Option<u64>) -> String {
    let Some(s) = seconds else { return String::new() };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let local = |t: u64| -> (u64, u32, u32) {
        // Local time through the C runtime's offset: days since 1970, hour, minute.
        let offset = local_offset();
        let t = (t as i64 + offset).max(0) as u64;
        (t / 86400, ((t % 86400) / 3600) as u32, ((t % 3600) / 60) as u32)
    };
    let (day, h, m) = local(s);
    let (today, _, _) = local(now);
    if day == today {
        format!("Today at {h:02}:{m:02}")
    } else if day + 1 == today {
        format!("Yesterday at {h:02}:{m:02}")
    } else {
        let (y, mo, d) = civil(day as i64);
        format!("{y}-{mo:02}-{d:02} {h:02}:{m:02}")
    }
}

/// Seconds east of UTC, from the time zone Windows reports.
fn local_offset() -> i64 {
    use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    let mut tz = TIME_ZONE_INFORMATION::default();
    let kind = unsafe { GetTimeZoneInformation(&mut tz) };
    let bias = tz.Bias + if kind == 2 { tz.DaylightBias } else { 0 };
    -(bias as i64) * 60
}

/// Days since 1970 to year, month, day (Howard Hinnant's algorithm).
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

fn cell_text(column: usize, r: &Row) -> String {
    let finished = r.status == Status::Finished;
    match column {
        0 => r.name.clone(),
        1 => r.status_text.clone(),
        2 => format::size_text(r.downloaded, r.size, finished),
        3 => format::speed_text(r.speed),
        4 => format::time_left_text(r.time_left),
        5 if !finished => format!("{} / {}", r.active_connections, r.allowed_connections),
        6 => format!("{} / {}", r.sources_in_use, r.sources_known),
        7 => date_text(Some(r.added)),
        8 => date_text(r.finished),
        9 => r.folder.display().to_string(),
        _ => String::new(),
    }
}

fn compare(column: usize, a: &Row, b: &Row) -> std::cmp::Ordering {
    match column {
        0 => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        1 => a.status_text.cmp(&b.status_text),
        2 => a.size.cmp(&b.size).then(a.downloaded.cmp(&b.downloaded)),
        3 => a.speed.cmp(&b.speed),
        4 => a.time_left.cmp(&b.time_left),
        5 => a.active_connections.cmp(&b.active_connections),
        6 => a.sources_known.cmp(&b.sources_known),
        8 => a.finished.cmp(&b.finished),
        9 => a.folder.cmp(&b.folder),
        _ => a.added.cmp(&b.added).then(a.id.cmp(&b.id)),
    }
}

/// The system image list's index of a file type's small icon (by its name's extension; nothing is read from disk).
fn icon_index(name: &str) -> i32 {
    let text = wide(name);
    let mut info = SHFILEINFOW::default();
    unsafe {
        SHGetFileInfoW(PCWSTR(text.as_ptr()), FILE_ATTRIBUTE_NORMAL, Some(&mut info), std::mem::size_of::<SHFILEINFOW>() as u32,
                       SHGFI_USEFILEATTRIBUTES | SHGFI_SYSICONINDEX | SHGFI_SMALLICON);
    }
    info.iIcon
}

pub fn run() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES | ICC_TREEVIEW_CLASSES | ICC_STANDARD_CLASSES | ICC_BAR_CLASSES,
        });
        let manager = match Manager::open(&crate::data_folder()) {
            Ok(m) => m,
            Err(e) => {
                let text = wide(&format!("Herb's Download Manager can't open its download list:\n{e}"));
                MessageBoxW(None, PCWSTR(text.as_ptr()), w!("Herb's Download Manager"), MB_ICONERROR);
                return;
            }
        };
        let args: Vec<String> = std::env::args().collect();
        // Test hook: `--add URL` (repeatable) adds and starts a download, as on the Mac.
        for (i, a) in args.iter().enumerate() {
            if a == "--add"
                && let Some(url) = args.get(i + 1)
            {
                let s = manager.settings();
                manager.add(hd_core::manager::NewDownload {
                    url: url.clone(),
                    folder: crate::downloads_folder(),
                    connections: s.connections,
                    connections_per_server: s.connections_per_server,
                    retries: s.retries,
                    start: true,
                    ..Default::default()
                });
            }
        }
        let capture = args.iter().position(|a| a == "--capture").and_then(|i| args.get(i + 1)).map(Into::into);

        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("HerbsDownloadManager"),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: GetSysColorBrush(COLOR_WINDOW),
            ..Default::default()
        };
        RegisterClassW(&class);
        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("HerbsDownloadManager"),
            w!("Herb's Download Manager"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1100,
            640,
            None,
            None,
            Some(instance.into()),
            None,
        ) else {
            return;
        };
        let child = |class: PCWSTR, text: &str, style: u32, ex: WINDOW_EX_STYLE, id: u16| {
            let text = wide(text);
            CreateWindowExW(ex, class, PCWSTR(text.as_ptr()), WS_CHILD | WS_VISIBLE | WINDOW_STYLE(style), 0, 0, 10, 10,
                            Some(hwnd), Some(HMENU(id as usize as *mut _)), Some(instance.into()), None)
                .unwrap_or_default()
        };

        // Fonts: the system's message font for text, Segoe Fluent Icons for the toolbar's glyphs.
        let mut metrics = NONCLIENTMETRICSW { cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
        let _ = SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, metrics.cbSize, Some(&mut metrics as *mut _ as *mut _),
                                      SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
        let text_font = CreateFontIndirectW(&metrics.lfMessageFont);
        let mut glyph = metrics.lfMessageFont;
        glyph.lfHeight = -16;
        let face = wide("Segoe Fluent Icons");
        glyph.lfFaceName = [0; 32];
        glyph.lfFaceName[..face.len().min(32)].copy_from_slice(&face[..face.len().min(32)]);
        let glyph_font = CreateFontIndirectW(&glyph);
        let set_font = |h: HWND, f: HFONT| {
            SendMessageW(h, WM_SETFONT, Some(WPARAM(f.0 as usize)), Some(LPARAM(1)));
        };

        let menu_button = child(w!("BUTTON"), "\u{E700}", BS_PUSHBUTTON as u32 | BS_FLAT as u32, Default::default(), C_MENU);
        set_font(menu_button, glyph_font);
        let title = child(w!("STATIC"), "Herb's Download Manager", 0x200 /* SS_CENTERIMAGE */, Default::default(), 0);
        set_font(title, text_font);
        let tooltips = CreateWindowExW(WS_EX_TOPMOST, TOOLTIPS_CLASSW, PCWSTR::null(), WINDOW_STYLE(TTS_ALWAYSTIP),
                                       CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, Some(hwnd), None,
                                       Some(instance.into()), None)
            .unwrap_or_default();
        let add_tip = |control: HWND, text: &str| {
            let text = wide(text);
            let info = TTTOOLINFOW {
                cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
                uFlags: TTF_IDISHWND | TTF_SUBCLASS,
                hwnd,
                uId: control.0 as usize,
                lpszText: PWSTR(text.as_ptr() as *mut _),
                ..Default::default()
            };
            SendMessageW(tooltips, TTM_ADDTOOLW, None, Some(LPARAM(&info as *const _ as isize)));
        };
        add_tip(menu_button, "Main Menu: every command, from File, Edit, Tasks, View, Window and Help to Settings and Quit");
        let mut tools = Vec::new();
        for (id, glyph, name, help) in TOOLS {
            if id == 0 {
                tools.push((0, child(w!("STATIC"), "", 0x10 /* SS_ETCHEDVERT */, Default::default(), 0)));
                continue;
            }
            let b = child(w!("BUTTON"), &glyph.to_string(), BS_PUSHBUTTON as u32 | BS_FLAT as u32, Default::default(), id);
            set_font(b, glyph_font);
            add_tip(b, help);
            let label = wide(name);
            let _ = SetPropW(b, w!("name"), Some(HANDLE(label.as_ptr() as *mut _)));
            std::mem::forget(label);
            tools.push((id, b));
        }
        let search_button = child(w!("BUTTON"), "\u{E721}", BS_PUSHBUTTON as u32 | BS_FLAT as u32, Default::default(), C_FIND);
        set_font(search_button, glyph_font);
        add_tip(search_button, "Search: show only downloads whose name or link contains some text (Ctrl+F)");

        // Sidebar (§2.1): each filter with its count, its file-type groups under it.
        let sidebar = child(WC_TREEVIEWW, "", (TVS_HASBUTTONS | TVS_SHOWSELALWAYS | TVS_FULLROWSELECT | TVS_LINESATROOT) as u32,
                            Default::default(), ID_SIDEBAR);
        set_font(sidebar, text_font);
        let mut sidebar_items = Vec::new();
        for (fi, f) in Filter::ALL.iter().enumerate() {
            let parent = insert_tree(sidebar, HTREEITEM::default(), f.title(), (fi * 8) as isize);
            sidebar_items.push((*f, None, parent.0));
            for (ki, k) in FileKind::ALL.iter().enumerate() {
                let item = insert_tree(sidebar, parent, k.title(), (fi * 8 + ki + 1) as isize);
                sidebar_items.push((*f, Some(*k), item.0));
            }
        }
        SendMessageW(sidebar, TVM_SELECTITEM, Some(WPARAM(TVGN_CARET as usize)), Some(LPARAM(sidebar_items[0].2)));

        let list = child(WC_LISTVIEWW, "", LVS_REPORT | LVS_SHOWSELALWAYS | LVS_SHAREIMAGELISTS, WS_EX_CLIENTEDGE, ID_LIST);
        set_font(list, text_font);
        SendMessageW(list, LVM_SETEXTENDEDLISTVIEWSTYLE, Some(WPARAM(0)),
                     Some(LPARAM((LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER | LVS_EX_HEADERDRAGDROP | LVS_EX_LABELTIP) as isize)));
        let mut file_info = SHFILEINFOW::default();
        let images = SHGetFileInfoW(w!(".txt"), FILE_ATTRIBUTE_NORMAL, Some(&mut file_info), std::mem::size_of::<SHFILEINFOW>() as u32,
                                    SHGFI_USEFILEATTRIBUTES | SHGFI_SYSICONINDEX | SHGFI_SMALLICON);
        SendMessageW(list, LVM_SETIMAGELIST, Some(WPARAM(LVSIL_SMALL as usize)), Some(LPARAM(images as isize)));
        let mut widths = [0; 10];
        let mut shown_columns = [true; 10];
        for (i, (name, width, shown)) in COLUMNS.iter().enumerate() {
            let text = wide(name);
            let column = LVCOLUMNW {
                mask: LVCF_TEXT | LVCF_WIDTH,
                cx: if *shown { *width } else { 0 },
                pszText: PWSTR(text.as_ptr() as *mut _),
                ..Default::default()
            };
            SendMessageW(list, LVM_INSERTCOLUMNW, Some(WPARAM(i)), Some(LPARAM(&column as *const _ as isize)));
            widths[i] = *width;
            shown_columns[i] = *shown;
        }

        // The find bar (§2), at the top right of the list, as in Herb's Terminal.
        let find_edit = child(w!("EDIT"), "", ES_AUTOHSCROLL as u32 | WS_BORDER.0, Default::default(), ID_SEARCH);
        set_font(find_edit, text_font);
        SendMessageW(find_edit, EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(w!("Search downloads").0 as isize)));
        let find_done = child(w!("BUTTON"), "Done", BS_PUSHBUTTON as u32, Default::default(), C_FIND_DONE);
        set_font(find_done, text_font);
        add_tip(find_done, "Done: hide the search and show every download (Esc)");
        let _ = ShowWindow(find_edit, SW_HIDE);
        let _ = ShowWindow(find_done, SW_HIDE);

        let footer = child(w!("STATIC"), "", 0x200, Default::default(), 0);
        set_font(footer, text_font);

        let keep_on_top = keep_on_top_file().exists();
        STATE.with(|s| {
            *s.borrow_mut() = Some(State {
                manager,
                window: hwnd,
                menu_button,
                title,
                tools,
                search_button,
                sidebar,
                sidebar_items,
                list,
                find_edit,
                find_done,
                footer,
                filter: Filter::All,
                kind: None,
                search: String::new(),
                shown: Vec::new(),
                rows: Vec::new(),
                sort: (7, false),
                columns: shown_columns,
                widths,
                find_open: false,
                sidebar_shown: true,
                keep_on_top: false,
                capture: capture.clone(),
                notice: None,
            })
        });
        set_keep_on_top(keep_on_top);
        layout(hwnd);
        refresh();
        SetTimer(Some(hwnd), TIMER_REFRESH, 500, None);
        if capture.is_some() {
            SetTimer(Some(hwnd), TIMER_CAPTURE, 2000, None);
        }
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        let accelerators = accelerator_table();
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let in_find = with(|s| GetFocus() == s.find_edit).unwrap_or(false);
            if in_find && msg.message == WM_KEYDOWN && msg.wParam.0 == VK_ESCAPE.0 as usize {
                close_find();
                continue;
            }
            // While typing in the find bar, Delete, Ctrl+A and Ctrl+C are the field's own.
            if !in_find && TranslateAcceleratorW(hwnd, accelerators, &msg) != 0 {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe fn insert_tree(tree: HWND, parent: HTREEITEM, text: &str, data: isize) -> HTREEITEM {
    let text = wide(text);
    let insert = TVINSERTSTRUCTW {
        hParent: parent,
        hInsertAfter: TVI_LAST,
        Anonymous: TVINSERTSTRUCTW_0 {
            item: TVITEMW {
                mask: TVIF_TEXT | TVIF_PARAM,
                pszText: PWSTR(text.as_ptr() as *mut _),
                lParam: LPARAM(data),
                ..Default::default()
            },
        },
    };
    HTREEITEM(unsafe { SendMessageW(tree, TVM_INSERTITEMW, None, Some(LPARAM(&insert as *const _ as isize))) }.0)
}

/// The Mac's shortcuts with Ctrl (§3, §4).
unsafe fn accelerator_table() -> HACCEL {
    let ctrl = FCONTROL | FVIRTKEY;
    let keys: [(ACCEL_VIRT_FLAGS, u16, u16); 17] = [
        (ctrl, b'N' as u16, C_NEW),
        (ctrl | FSHIFT, b'O' as u16, C_IMPORT),
        (ctrl | FSHIFT, b'V' as u16, C_CLIPBOARD),
        (ctrl | FSHIFT, b'N' as u16, C_BATCH),
        (ctrl, b'Q' as u16, C_QUIT),
        (ctrl, b'W' as u16, C_QUIT),
        (ctrl, b'R' as u16, C_RESUME),
        (ctrl, VK_OEM_PERIOD.0, C_PAUSE),
        (ctrl | FSHIFT, b'R' as u16, C_START_QUEUE),
        (ctrl | FALT, VK_OEM_PERIOD.0, C_STOP_ALL),
        (FVIRTKEY, VK_DELETE.0, C_DELETE),
        (ctrl, VK_OEM_COMMA.0, C_SETTINGS),
        (ctrl | FALT, b'T' as u16, C_KEEP_ON_TOP),
        (ctrl, b'F' as u16, C_FIND),
        (ctrl, b'A' as u16, C_SELECT_ALL),
        (ctrl, b'C' as u16, C_COPY_LINK),
        (ctrl, b'I' as u16, C_PROPERTIES),
    ];
    let table: Vec<ACCEL> = keys.iter().map(|&(f, key, cmd)| ACCEL { fVirt: f, key, cmd }).collect();
    unsafe { CreateAcceleratorTableW(&table).unwrap_or_default() }
}

/// Places the controls: a 44-pixel bar (☰, the title, the toolbar, Search), the sidebar and list below it, the
/// footer at the bottom, the find bar over the list's top right (scaled for the window's DPI).
fn layout(hwnd: HWND) {
    unsafe {
        let scale = GetDpiForWindow(hwnd).max(96) as i32;
        let px = |v: i32| v * scale / 96;
        let mut rect = RECT::default();
        let _ = GetClientRect(hwnd, &mut rect);
        let (w, h) = (rect.right, rect.bottom);
        with(|s| {
            let (bar, foot) = (px(44), px(26));
            let side = if s.sidebar_shown { px(190) } else { 0 };
            let _ = MoveWindow(s.menu_button, px(6), px(6), px(36), px(32), true);
            // The toolbar from the right edge leftwards, Search last.
            let mut x = w - px(6) - px(36);
            let _ = MoveWindow(s.search_button, x, px(6), px(36), px(32), true);
            for (id, control) in s.tools.iter().rev() {
                if *id == 0 {
                    x -= px(9);
                    let _ = MoveWindow(*control, x + px(4), px(12), px(2), px(20), true);
                } else {
                    x -= px(38);
                    let _ = MoveWindow(*control, x, px(6), px(36), px(32), true);
                }
            }
            let _ = MoveWindow(s.title, px(50), 0, (x - px(60)).max(0), bar, true);
            let _ = MoveWindow(s.sidebar, 0, bar, side, h - bar - foot, true);
            let _ = MoveWindow(s.list, side, bar, w - side, h - bar - foot, true);
            let _ = MoveWindow(s.footer, px(12), h - foot, w - px(24), foot, true);
            let _ = MoveWindow(s.find_edit, w - px(330), bar + px(30), px(250), px(26), true);
            let _ = MoveWindow(s.find_done, w - px(74), bar + px(30), px(60), px(26), true);
        });
    }
}

/// The selected downloads, in list order.
fn selected() -> Vec<Row> {
    with(|s| unsafe {
        let mut out = Vec::new();
        let mut i = -1isize;
        loop {
            i = SendMessageW(s.list, LVM_GETNEXTITEM, Some(WPARAM(i as usize)), Some(LPARAM(LVNI_SELECTED as isize))).0;
            if i < 0 {
                break;
            }
            if let Some(r) = s.shown.get(i as usize).and_then(|id| s.rows.iter().find(|r| r.id == *id)) {
                out.push(r.clone());
            }
        }
        out
    })
    .unwrap_or_default()
}

/// Shows the list's rows, the sidebar's counts, the footer and which commands apply.
fn refresh() {
    let selection = selected();
    with(|s| unsafe {
        let rows = s.manager.rows();
        for &(f, kind, item) in &s.sidebar_items {
            let n = rows.iter().filter(|r| f.matches(&r.status) && kind.is_none_or(|k| r.kind == k)).count();
            let title = kind.map(|k| k.title()).unwrap_or(f.title());
            let text = wide(&format!("{title}   {n}"));
            let tv = TVITEMW { mask: TVIF_TEXT, hItem: HTREEITEM(item), pszText: PWSTR(text.as_ptr() as *mut _), ..Default::default() };
            SendMessageW(s.sidebar, TVM_SETITEMW, None, Some(LPARAM(&tv as *const _ as isize)));
        }

        let needle = s.search.to_lowercase();
        let mut shown: Vec<Row> = rows
            .iter()
            .filter(|r| s.filter.matches(&r.status) && s.kind.is_none_or(|k| r.kind == k))
            .filter(|r| needle.is_empty() || r.name.to_lowercase().contains(&needle) || r.url.to_lowercase().contains(&needle))
            .cloned()
            .collect();
        let (column, ascending) = s.sort;
        shown.sort_by(|a, b| if ascending { compare(column, a, b) } else { compare(column, b, a) });
        let ids: Vec<u64> = shown.iter().map(|r| r.id).collect();
        let set_cell = |row: usize, col: usize, text: &str, insert: bool, r: &Row| {
            let text = wide(text);
            let item = LVITEMW {
                mask: if col == 0 { LVIF_TEXT | LVIF_IMAGE } else { LVIF_TEXT },
                iItem: row as i32,
                iSubItem: col as i32,
                pszText: PWSTR(text.as_ptr() as *mut _),
                iImage: if col == 0 { icon_index(&r.name) } else { 0 },
                ..Default::default()
            };
            let message = if insert { LVM_INSERTITEMW } else { LVM_SETITEMW };
            SendMessageW(s.list, message, None, Some(LPARAM(&item as *const _ as isize)));
        };
        if ids != s.shown {
            // The downloads changed: rebuild, keeping the selection by id.
            let keep: Vec<u64> = selection.iter().map(|r| r.id).collect();
            SendMessageW(s.list, WM_SETREDRAW, Some(WPARAM(0)), None);
            SendMessageW(s.list, LVM_DELETEALLITEMS, None, None);
            for (i, r) in shown.iter().enumerate() {
                for col in 0..COLUMNS.len() {
                    set_cell(i, col, &cell_text(col, r), col == 0, r);
                }
                if keep.contains(&r.id) {
                    let state = LVITEMW { stateMask: LVIS_SELECTED, state: LVIS_SELECTED, ..Default::default() };
                    SendMessageW(s.list, LVM_SETITEMSTATE, Some(WPARAM(i)), Some(LPARAM(&state as *const _ as isize)));
                }
            }
            SendMessageW(s.list, WM_SETREDRAW, Some(WPARAM(1)), None);
            s.shown = ids;
        } else {
            for (i, r) in shown.iter().enumerate() {
                let old = s.rows.iter().find(|o| o.id == r.id);
                for col in 0..COLUMNS.len() {
                    let text = cell_text(col, r);
                    if old.is_none_or(|o| cell_text(col, o) != text) {
                        set_cell(i, col, &text, false, r);
                    }
                }
            }
        }
        s.rows = rows;

        let active: Vec<&Row> = s.rows.iter().filter(|r| r.status.is_running()).collect();
        let mut parts = vec![format!("{} selected", selection.len()), format!("{} active", active.len())];
        let speed: u64 = active.iter().map(|r| r.speed).sum();
        if speed > 0 {
            parts.push(format::speed_text(speed));
        }
        let waiting = s.rows.iter().filter(|r| r.status == Status::Waiting).count();
        if s.manager.queue_running() {
            parts.push(if waiting == 0 { "Queue running".into() } else { format!("Queue running, {waiting} waiting") });
        } else if waiting > 0 {
            parts.push(format!("Queue stopped, {waiting} waiting"));
        }
        if s.notice.as_ref().is_some_and(|(_, at)| at.elapsed().as_secs() >= 6) {
            s.notice = None;
        }
        if let Some((text, _)) = &s.notice {
            parts.push(text.clone());
        }
        let text = wide(&parts.join("  ·  "));
        let _ = SetWindowTextW(s.footer, PCWSTR(text.as_ptr()));
        for (id, control) in &s.tools {
            if *id != 0 {
                let _ = EnableWindow(*control, enabled(*id, &selection, &s.rows));
            }
        }
    });
}

/// Whether a command applies now (§3: buttons that don't apply are disabled).
fn enabled(id: u16, selected: &[Row], rows: &[Row]) -> bool {
    let unfinished = selected.iter().any(|r| r.status != Status::Finished);
    match id {
        _ if LATER.contains(&id) => false,
        C_RESUME => selected.iter().any(|r| matches!(r.status, Status::Waiting | Status::Paused | Status::Failed(_))),
        C_PAUSE => selected.iter().any(|r| r.status.is_running()),
        C_RESTART => unfinished,
        C_STOP_ALL => rows.iter().any(|r| r.status.is_running()),
        C_DELETE | C_COPY_LINK => !selected.is_empty(),
        C_OPEN => selected.len() == 1 && selected[0].status == Status::Finished,
        C_SHOW_IN_FOLDER => selected.len() == 1,
        C_ADD_CONNECTION => selected.len() == 1 && unfinished && selected[0].allowed_connections < 32,
        C_REMOVE_CONNECTION => selected.len() == 1 && unfinished && selected[0].allowed_connections > 1,
        C_DELETE_FINISHED => rows.iter().any(|r| r.status == Status::Finished),
        C_DELETE_UNFINISHED => rows.iter().any(|r| r.status != Status::Finished),
        C_DELETE_ALL => !rows.is_empty(),
        _ => true,
    }
}

/// Builds a popup menu from (label, command) pairs; "-" is a separator and a command of 0 starts a submenu that the
/// following items up to ")" fill.
unsafe fn popup(items: &[(&str, u16)], selected: &[Row], rows: &[Row], checks: &[(u16, bool)]) -> HMENU {
    unsafe {
        let mut stack = vec![CreatePopupMenu().unwrap_or_default()];
        let mut titles: Vec<Vec<u16>> = Vec::new();
        for &(label, id) in items {
            let menu = *stack.last().unwrap();
            if label == "-" {
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            } else if label == ")" {
                let sub = stack.pop().unwrap();
                let parent = *stack.last().unwrap();
                let title = titles.pop().unwrap();
                let _ = AppendMenuW(parent, MF_POPUP | MF_STRING, sub.0 as usize, PCWSTR(title.as_ptr()));
            } else if id == 0 {
                titles.push(wide(label));
                stack.push(CreatePopupMenu().unwrap_or_default());
            } else {
                let text = wide(label);
                let mut flags = MF_STRING;
                if !enabled(id, selected, rows) {
                    flags |= MF_GRAYED;
                }
                if checks.iter().any(|&(c, on)| c == id && on) {
                    flags |= MF_CHECKED;
                }
                let _ = AppendMenuW(menu, flags, id as usize, PCWSTR(text.as_ptr()));
            }
        }
        stack[0]
    }
}

/// The Main Menu (§4): File, Edit, Tasks, View, Window and Help, then Settings, About and Quit.
unsafe fn show_main_menu(hwnd: HWND, button: HWND) {
    let (sel, rows, checks) = (selected(), with(|s| s.rows.clone()).unwrap_or_default(), checks());
    let mut items: Vec<(&str, u16)> = vec![
        ("File", 0), ("New Download…\tCtrl+N", C_NEW), ("Import Links…\tCtrl+Shift+O", C_IMPORT),
        ("Add from Clipboard\tCtrl+Shift+V", C_CLIPBOARD), ("Batch Download…\tCtrl+Shift+N", C_BATCH), ("-", 0),
        ("Close Window\tCtrl+W", C_QUIT), ("Exit", C_QUIT), (")", 0),
        ("Edit", 0), ("Copy Link\tCtrl+C", C_COPY_LINK), ("Select All\tCtrl+A", C_SELECT_ALL), ("-", 0), ("Find\tCtrl+F", C_FIND), (")", 0),
        ("Tasks", 0), ("Resume\tCtrl+R", C_RESUME), ("Pause\tCtrl+.", C_PAUSE), ("-", 0),
        ("Start Queue\tCtrl+Shift+R", C_START_QUEUE), ("Stop Queue", C_STOP_QUEUE), ("-", 0),
        ("Stop All\tCtrl+Alt+.", C_STOP_ALL), ("-", 0), ("Add Mirror…\tCtrl+Alt+M", C_ADD_MIRROR), ("Properties\tCtrl+I", C_PROPERTIES), ("-", 0),
        ("Delete", 0), ("Selected\tDelete", C_DELETE), ("All Finished", C_DELETE_FINISHED), ("All Unfinished", C_DELETE_UNFINISHED),
        ("All Missing Files", C_DELETE_MISSING), ("Entire List", C_DELETE_ALL), (")", 0), (")", 0),
        ("View", 0), ("Show Sidebar", C_SIDEBAR), ("Keep on Top\tCtrl+Alt+T", C_KEEP_ON_TOP), ("Columns", 0),
    ];
    for (i, (name, _, _)) in COLUMNS.iter().enumerate().skip(1) {
        items.push((name, C_COLUMN + i as u16));
    }
    items.extend_from_slice(&[
        (")", 0), ("Show Download Window\tCtrl+O", C_DOWNLOAD_WINDOW), (")", 0),
        ("Window", 0), ("Minimize", C_MINIMIZE), ("Zoom", C_ZOOM), (")", 0),
        ("Help", 0), ("Herb's Download Manager on GitHub", C_GITHUB), (")", 0),
        ("-", 0), ("Settings…\tCtrl+,", C_SETTINGS), ("About Herb's Download Manager", C_ABOUT), ("-", 0), ("Quit Herb's DM\tCtrl+Q", C_QUIT),
    ]);
    unsafe {
        let menu = popup(&items, &sel, &rows, &checks);
        let mut rect = RECT::default();
        let _ = GetWindowRect(button, &mut rect);
        let _ = TrackPopupMenu(menu, TPM_LEFTALIGN | TPM_TOPALIGN, rect.left, rect.bottom, None, hwnd, None);
        let _ = DestroyMenu(menu);
    }
}

fn checks() -> Vec<(u16, bool)> {
    with(|s| {
        let mut c = vec![(C_KEEP_ON_TOP, s.keep_on_top), (C_SIDEBAR, s.sidebar_shown)];
        c.extend((1..COLUMNS.len()).map(|i| (C_COLUMN + i as u16, s.columns[i])));
        c
    })
    .unwrap_or_default()
}

/// §4.1: the menu for right-clicked downloads; right-clicking the column titles shows which columns to show.
unsafe fn show_context_menu(hwnd: HWND, x: i32, y: i32, on_header: bool) {
    let (sel, rows, checks) = (selected(), with(|s| s.rows.clone()).unwrap_or_default(), checks());
    let items: Vec<(&str, u16)> = if on_header {
        COLUMNS.iter().enumerate().skip(1).map(|(i, (name, _, _))| (*name, C_COLUMN + i as u16)).collect()
    } else {
        vec![
            ("Open", C_OPEN), ("Show in Folder", C_SHOW_IN_FOLDER), ("-", 0), ("Resume", C_RESUME), ("Pause", C_PAUSE),
            ("Restart", C_RESTART), ("-", 0), ("Add Mirror…", C_ADD_MIRROR), ("Connections", 0),
            ("Add Connection", C_ADD_CONNECTION), ("Remove Connection", C_REMOVE_CONNECTION), (")", 0), ("-", 0),
            ("Copy Link", C_COPY_LINK), ("Properties", C_PROPERTIES), ("-", 0), ("Delete", C_DELETE),
        ]
    };
    unsafe {
        let menu = popup(&items, &sel, &rows, &checks);
        let _ = TrackPopupMenu(menu, TPM_LEFTALIGN | TPM_TOPALIGN, x, y, None, hwnd, None);
        let _ = DestroyMenu(menu);
    }
}

fn set_keep_on_top(on: bool) {
    with(|s| unsafe {
        s.keep_on_top = on;
        let after = if on { HWND_TOPMOST } else { HWND_NOTOPMOST };
        let _ = SetWindowPos(s.window, Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        if let Some((_, b)) = s.tools.iter().find(|(id, _)| *id == C_KEEP_ON_TOP) {
            // Pin, or Pinned while on.
            let _ = SetWindowTextW(*b, if on { w!("\u{E841}") } else { w!("\u{E718}") });
        }
    });
    let file = keep_on_top_file();
    if on {
        let _ = std::fs::write(file, b"");
    } else {
        let _ = std::fs::remove_file(file);
    }
}

fn open_find() {
    with(|s| unsafe {
        s.find_open = true;
        let _ = ShowWindow(s.find_edit, SW_SHOW);
        let _ = ShowWindow(s.find_done, SW_SHOW);
        let _ = SetWindowPos(s.find_edit, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        let _ = SetWindowPos(s.find_done, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        let _ = SetFocus(Some(s.find_edit));
    });
}

fn close_find() {
    with(|s| unsafe {
        s.find_open = false;
        s.search.clear();
        let _ = SetWindowTextW(s.find_edit, w!(""));
        let _ = ShowWindow(s.find_edit, SW_HIDE);
        let _ = ShowWindow(s.find_done, SW_HIDE);
        let _ = SetFocus(Some(s.list));
    });
    refresh();
}

fn set_column(i: usize, on: bool) {
    with(|s| unsafe {
        if !on {
            s.widths[i] = SendMessageW(s.list, LVM_GETCOLUMNWIDTH, Some(WPARAM(i)), None).0.max(40) as i32;
        }
        s.columns[i] = on;
        SendMessageW(s.list, LVM_SETCOLUMNWIDTH, Some(WPARAM(i)), Some(LPARAM(if on { s.widths[i] as isize } else { 0 })));
    });
}

fn open_path(path: &std::path::Path, select: bool) {
    unsafe {
        if select {
            let args = wide(&format!("/select,\"{}\"", path.display()));
            ShellExecuteW(None, w!("open"), w!("explorer.exe"), PCWSTR(args.as_ptr()), None, SW_SHOWNORMAL);
        } else {
            let target = wide(&path.display().to_string());
            ShellExecuteW(None, w!("open"), PCWSTR(target.as_ptr()), None, None, SW_SHOWNORMAL);
        }
    }
}

fn copy_text(hwnd: HWND, text: &str) {
    unsafe {
        let data = wide(text);
        if OpenClipboard(Some(hwnd)).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        if let Ok(memory) = GlobalAlloc(GMEM_MOVEABLE, data.len() * 2) {
            let target = GlobalLock(memory) as *mut u16;
            if !target.is_null() {
                std::ptr::copy_nonoverlapping(data.as_ptr(), target, data.len());
                let _ = GlobalUnlock(memory);
                let _ = SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(memory.0)));
            }
        }
        let _ = CloseClipboard();
    }
}

/// Delete asks whether to remove only the list entries (the default) or to move the files to the Recycle Bin as
/// well (§10); nothing is ever erased.
fn confirm_delete(hwnd: HWND, rows: Vec<Row>, files_gone: bool) {
    if rows.is_empty() {
        return;
    }
    let one = rows.len() == 1;
    let heading = wide(&if one { format!("Delete “{}”?", rows[0].name) } else { format!("Delete {} downloads?", rows.len()) });
    let body = wide(&if files_gone {
        "Their files are no longer on disk; only the list entries are removed.".to_string()
    } else {
        format!("Remove {} from the list only, or also move {} to the Recycle Bin? Files in the Recycle Bin can be restored until it's emptied.",
                if one { "it" } else { "them" }, if one { "its file" } else { "their files" })
    });
    let (list_label, trash_label) = (wide("Remove from List"), wide("Move to Recycle Bin Too"));
    let mut buttons = vec![TASKDIALOG_BUTTON { nButtonID: 1001, pszButtonText: PCWSTR(list_label.as_ptr()) }];
    if !files_gone {
        buttons.push(TASKDIALOG_BUTTON { nButtonID: 1002, pszButtonText: PCWSTR(trash_label.as_ptr()) });
    }
    let config = TASKDIALOGCONFIG {
        cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: hwnd,
        dwCommonButtons: TDCBF_CANCEL_BUTTON,
        pszWindowTitle: w!("Herb's Download Manager"),
        pszMainInstruction: PCWSTR(heading.as_ptr()),
        pszContent: PCWSTR(body.as_ptr()),
        cButtons: buttons.len() as u32,
        pButtons: buttons.as_ptr(),
        nDefaultButton: 1001,
        ..Default::default()
    };
    let mut pressed = 0i32;
    unsafe {
        let _ = TaskDialogIndirect(&config, Some(&mut pressed), None, None);
    }
    let ids: Vec<u64> = rows.iter().map(|r| r.id).collect();
    let kept = with(|s| match pressed {
        1001 => s.manager.remove(&ids, false),
        1002 => s.manager.remove(&ids, true),
        _ => Vec::new(),
    })
    .unwrap_or_default();
    if let Some(first) = kept.first() {
        let name = first.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        with(|s| s.notice = Some((format!("“{name}” couldn't go to the Recycle Bin, so it was left where it is"), std::time::Instant::now())));
    }
    refresh();
}

fn command(hwnd: HWND, id: u16) {
    let sel = selected();
    let rows = with(|s| s.rows.clone()).unwrap_or_default();
    if !enabled(id, &sel, &rows) {
        return;
    }
    let ids: Vec<u64> = sel.iter().map(|r| r.id).collect();
    unsafe {
        match id {
            C_MENU => {
                if let Some(button) = with(|s| s.menu_button) {
                    show_main_menu(hwnd, button);
                }
            }
            C_RESUME => drop(with(|s| s.manager.resume(&ids))),
            C_PAUSE => drop(with(|s| s.manager.pause(&ids))),
            C_RESTART => drop(with(|s| s.manager.restart(&ids))),
            C_START_QUEUE => drop(with(|s| s.manager.start_queue())),
            C_STOP_QUEUE => drop(with(|s| s.manager.stop_queue())),
            C_STOP_ALL => drop(with(|s| s.manager.pause_all())),
            C_ADD_CONNECTION => drop(with(|s| s.manager.set_connections(sel[0].id, (sel[0].allowed_connections + 1).min(32)))),
            C_REMOVE_CONNECTION => drop(with(|s| s.manager.set_connections(sel[0].id, sel[0].allowed_connections.saturating_sub(1).max(1)))),
            C_DELETE => confirm_delete(hwnd, sel, false),
            C_DELETE_FINISHED => confirm_delete(hwnd, rows.into_iter().filter(|r| r.status == Status::Finished).collect(), false),
            C_DELETE_UNFINISHED => confirm_delete(hwnd, rows.into_iter().filter(|r| r.status != Status::Finished).collect(), false),
            C_DELETE_MISSING => confirm_delete(
                hwnd,
                rows.into_iter().filter(|r| r.status == Status::Finished && !r.path.as_ref().is_some_and(|p| p.exists())).collect(),
                true,
            ),
            C_DELETE_ALL => confirm_delete(hwnd, rows, false),
            C_KEEP_ON_TOP => {
                let on = with(|s| s.keep_on_top).unwrap_or(false);
                set_keep_on_top(!on);
            }
            C_FIND => open_find(),
            C_FIND_DONE => close_find(),
            C_SELECT_ALL => drop(with(|s| {
                let state = LVITEMW { stateMask: LVIS_SELECTED, state: LVIS_SELECTED, ..Default::default() };
                SendMessageW(s.list, LVM_SETITEMSTATE, Some(WPARAM(usize::MAX)), Some(LPARAM(&state as *const _ as isize)));
            })),
            C_COPY_LINK => copy_text(hwnd, &sel.iter().map(|r| r.url.clone()).collect::<Vec<_>>().join("\r\n")),
            C_OPEN => {
                if let Some(p) = sel.first().and_then(|r| r.path.clone()) {
                    open_path(&p, false);
                }
            }
            C_SHOW_IN_FOLDER => {
                if let Some(r) = sel.first() {
                    match &r.path {
                        Some(p) if p.exists() => open_path(p, true),
                        _ => open_path(&r.folder, false),
                    }
                }
            }
            C_SIDEBAR => {
                if let Some((side, on)) = with(|s| {
                    s.sidebar_shown = !s.sidebar_shown;
                    (s.sidebar, s.sidebar_shown)
                }) {
                    let _ = ShowWindow(side, if on { SW_SHOW } else { SW_HIDE });
                    layout(hwnd);
                }
            }
            C_MINIMIZE => drop(ShowWindow(hwnd, SW_MINIMIZE)),
            C_ZOOM => drop(ShowWindow(hwnd, if IsZoomed(hwnd).as_bool() { SW_RESTORE } else { SW_MAXIMIZE })),
            C_GITHUB => {
                let url = wide(GITHUB);
                ShellExecuteW(None, w!("open"), PCWSTR(url.as_ptr()), None, None, SW_SHOWNORMAL);
            }
            C_ABOUT => {
                let text = wide(&format!("Herb's Download Manager {}\n\nby dunkelherzmann\n{GITHUB}", hd_core::version()));
                MessageBoxW(Some(hwnd), PCWSTR(text.as_ptr()), w!("About Herb's Download Manager"), MB_OK | MB_ICONINFORMATION);
            }
            C_QUIT => drop(DestroyWindow(hwnd)),
            _ if (C_COLUMN + 1..C_COLUMN + COLUMNS.len() as u16).contains(&id) => {
                let i = (id - C_COLUMN) as usize;
                let on = with(|s| s.columns[i]).unwrap_or(true);
                set_column(i, !on);
            }
            _ => {}
        }
    }
    refresh();
}

/// Saves a 24-bit BMP of the window as WM_PRINT draws it (frame and every control).
unsafe fn capture(hwnd: HWND, path: &std::path::Path) -> std::io::Result<()> {
    unsafe {
        let mut rect = RECT::default();
        let _ = GetWindowRect(hwnd, &mut rect);
        let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
        let screen = GetDC(Some(hwnd));
        let dc = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(dc, bitmap.into());
        SendMessageW(hwnd, WM_PRINT, Some(WPARAM(dc.0 as usize)),
                     Some(LPARAM((PRF_CLIENT | PRF_CHILDREN | PRF_NONCLIENT | PRF_ERASEBKGND) as isize)));
        let row = ((w * 3 + 3) & !3) as usize;
        let mut pixels = vec![0u8; row * h as usize];
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: h,
                biPlanes: 1,
                biBitCount: 24,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        GetDIBits(dc, bitmap, 0, h as u32, Some(pixels.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
        SelectObject(dc, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        ReleaseDC(Some(hwnd), screen);
        let mut file = Vec::with_capacity(54 + pixels.len());
        file.extend_from_slice(b"BM");
        file.extend_from_slice(&((54 + pixels.len()) as u32).to_le_bytes());
        file.extend_from_slice(&0u32.to_le_bytes());
        file.extend_from_slice(&54u32.to_le_bytes());
        let header = std::slice::from_raw_parts(&info.bmiHeader as *const _ as *const u8, 40);
        file.extend_from_slice(header);
        file.extend_from_slice(&pixels);
        std::fs::write(path, file)
    }
}

extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_SIZE => {
                layout(hwnd);
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_REFRESH => {
                refresh();
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_CAPTURE => {
                let _ = KillTimer(Some(hwnd), TIMER_CAPTURE);
                if let Some(path) = with(|s| s.capture.clone()).flatten()
                    && let Err(e) = capture(hwnd, &path)
                {
                    eprintln!("capture failed: {e}");
                }
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_COMMAND => {
                let id = (wparam.0 & 0xffff) as u16;
                let code = ((wparam.0 >> 16) & 0xffff) as u32;
                if id == ID_SEARCH {
                    if code == EN_CHANGE {
                        let text = with(|s| {
                            let mut buffer = [0u16; 512];
                            let n = GetWindowTextW(s.find_edit, &mut buffer);
                            String::from_utf16_lossy(&buffer[..n as usize])
                        });
                        if let Some(text) = text {
                            with(|s| s.search = text);
                            refresh();
                        }
                    }
                } else {
                    command(hwnd, id);
                }
                LRESULT(0)
            }
            WM_NOTIFY => {
                let header = &*(lparam.0 as *const NMHDR);
                match (header.idFrom as u16, header.code) {
                    (ID_SIDEBAR, TVN_SELCHANGEDW) => {
                        let change = &*(lparam.0 as *const NMTREEVIEWW);
                        let n = change.itemNew.lParam.0 as usize;
                        with(|s| {
                            s.filter = Filter::ALL[(n / 8).min(4)];
                            s.kind = if n % 8 == 0 { None } else { Some(FileKind::ALL[n % 8 - 1]) };
                        });
                        refresh();
                    }
                    (ID_LIST, LVN_COLUMNCLICK) => {
                        let click = &*(lparam.0 as *const NMLISTVIEW);
                        let column = click.iSubItem as usize;
                        with(|s| s.sort = if s.sort.0 == column { (column, !s.sort.1) } else { (column, column != 7) });
                        refresh();
                    }
                    (ID_LIST, NM_DBLCLK) => {
                        // §2.2: a finished download opens its file; an unfinished one its Download window (later).
                        if let Some(r) = selected().into_iter().next()
                            && r.status == Status::Finished
                            && let Some(p) = r.path
                        {
                            open_path(&p, false);
                        }
                    }
                    (ID_LIST, LVN_ITEMCHANGED) => refresh(),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_CONTEXTMENU => {
                let (x, y) = ((lparam.0 & 0xffff) as i16 as i32, ((lparam.0 >> 16) & 0xffff) as i16 as i32);
                if let Some(list) = with(|s| s.list)
                    && HWND(wparam.0 as *mut _) == list
                {
                    let header = HWND(SendMessageW(list, LVM_GETHEADER, None, None).0 as *mut _);
                    let mut rect = RECT::default();
                    let _ = GetWindowRect(header, &mut rect);
                    let on_header = y >= rect.top && y < rect.bottom;
                    show_context_menu(hwnd, x, y, on_header);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                if let Some(state) = STATE.with(|s| s.borrow_mut().take()) {
                    state.manager.quit(std::time::Duration::from_secs(5));
                }
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
