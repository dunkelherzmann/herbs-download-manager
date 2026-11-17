//! The main window (§2–§4), drawn as the Mac's (chrome.rs): one title row with the title, the toolbar, Search and the
//! Main Menu (☰) at its right end and the window controls after them, the sidebar filters with their file-type groups
//! (a tree), the download list (a report ListView with the Mac's ten columns), the find bar and the
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

use crate::chrome::{self, Symbol};
use crate::theme;

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
const C_HELP: u16 = 136;
const C_ABOUT: u16 = 137;
const C_OPEN: u16 = 138;
const C_SHOW_IN_FOLDER: u16 = 139;
const C_RESTART: u16 = 140;
const C_ADD_CONNECTION: u16 = 141;
const C_REMOVE_CONNECTION: u16 = 142;
const C_FIND_DONE: u16 = 143;
const C_RENAME: u16 = 144;
const C_MOVE: u16 = 145;
const C_NOTE: u16 = 146;
const C_EXPORT: u16 = 147;
const C_UNDO: u16 = 150;
const C_REDO: u16 = 151;
const C_CUT: u16 = 152;
const C_COPY: u16 = 153;
const C_PASTE: u16 = 154;
const C_CLOSE_WINDOW: u16 = 155;
const C_HIDE: u16 = 156;
const C_BRING_ALL: u16 = 157;
const C_SHOW: u16 = 158;
const C_OPEN_WITH_OTHER: u16 = 159;
/// Open With ▸ the n-th app (`OPEN_WITH`).
const C_OPEN_WITH: u16 = 180;
/// Posted by a second launch to the running copy: show the window (one copy per data folder, as on the Mac).
const WM_SHOW_AGAIN: u32 = WM_APP + 30;
/// Files handed to a second copy, passed on to this one (WM_COPYDATA), then imported from its own message.
const COPY_FILES: usize = 0x4844_4d46;
const WM_IMPORT_FILES: u32 = WM_APP + 31;
// The private messages: WM_PROBED (add.rs) +1, WM_SCANNED (add.rs) +2, WM_FEATURE (features.rs) +20, WM_TRAY +21, WM_QUEUE_DONE (queuedone.rs) +22, WM_DROPPED (dragdrop.rs) +23, WM_TOAST (toast.rs) +24.
/// The notification area icon's messages (a click on the "Download finished" balloon).
const WM_TRAY: u32 = WM_APP + 21;
const NIN_BALLOONUSERCLICK: u32 = 0x405;

thread_local! {
    /// The download the last "Download finished" balloon was about.
    static LAST_FINISHED: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
    static PENDING_FILES: RefCell<Vec<std::path::PathBuf>> = const { RefCell::new(Vec::new()) };
}
const C_COLUMN: u16 = 160; // + column index
const ID_SIDEBAR: u16 = 200;
const ID_LIST: u16 = 201;
const ID_SEARCH: u16 = 202;
const TIMER_REFRESH: usize = 1;
const TIMER_CAPTURE: usize = 2;
const TIMER_A11Y: usize = 3;

thread_local! {
    static A11Y_REPORT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Commands that arrive with their own windows later in `windows\pending.md`; their items show, disabled.
const LATER: [u16; 0] = [];

/// L1–L11: title, width; Date Finished and Location start hidden, as on the Mac. Note (L11) is the last column here
/// but shows second, after Name (`COLUMN_ORDER`).
const COLUMNS: [(&str, i32, bool); 11] = [
    ("Name", 280, true),
    ("Status", 150, true),
    ("Size", 140, true),
    ("Speed", 90, true),
    ("Time Left", 90, true),
    ("Connections", 90, true),
    ("Mirrors", 70, true),
    ("Date Added", 140, true),
    ("Date Finished", 140, false),
    ("Location", 200, false),
    ("Note", 180, true),
];
/// Where each column shows, left to right.
const COLUMN_ORDER: [i32; 11] = [0, 10, 1, 2, 3, 4, 5, 6, 7, 8, 9];

/// T1–T11 (§3), left to right as on the Mac, ending with Search and the Main Menu: command, symbol, name, hover text.
const TOOLS: [(u16, Symbol, &str, &str); 11] = [
    (C_NEW, Symbol::NewDownload, "New Download", "New Download: add a link to download (Ctrl+N)"),
    (C_RESUME, Symbol::Resume, "Resume", "Resume: continue the selected downloads, or with none selected start the waiting ones (Ctrl+R)"),
    (C_PAUSE, Symbol::Pause, "Pause", "Pause: stop the selected downloads, keeping what's downloaded (Ctrl+.)"),
    (C_START_QUEUE, Symbol::StartQueue, "Start Queue", "Start Queue: download waiting items one after another (Ctrl+Shift+R)"),
    (C_STOP_QUEUE, Symbol::StopQueue, "Stop Queue", "Stop Queue: stop starting new items from the queue"),
    (C_STOP_ALL, Symbol::StopAll, "Stop All", "Stop All: stop the queue and pause every running download (Ctrl+Alt+.)"),
    (C_DELETE, Symbol::Delete, "Delete", "Delete: remove the selected downloads from the list (Delete)"),
    (C_SETTINGS, Symbol::Settings, "Settings", "Settings (Ctrl+,)"),
    (C_KEEP_ON_TOP, Symbol::KeepOnTop(false), "Keep on Top", "Keep on Top: keep Herb's DM above every other app's windows (Ctrl+Alt+T)"),
    (C_FIND, Symbol::Search, "Search", "Search: show only downloads whose name, link or note contains some text (Ctrl+F)"),
    (C_MENU, Symbol::Menu, "Main Menu", "Main Menu: every command, from File, Edit, Tasks, View, Window and Help to Settings and Quit"),
];

struct State {
    manager: Manager,
    window: HWND,
    menu_button: HWND,
    tools: Vec<(u16, HWND)>,
    sidebar: HWND,
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
    columns: [bool; 11],
    widths: [i32; 11],
    find_open: bool,
    /// View ▸ Show Sidebar (IsWindowVisible can't tell before the window first shows).
    sidebar_shown: bool,
    /// The sidebar's width (190–360 at 96 DPI), dragged at its edge as on the Mac.
    sidebar_width: i32,
    keep_on_top: bool,
    capture: Option<std::path::PathBuf>,
    notice: Option<(String, std::time::Instant)>,
    font: HFONT,
    tooltips: HWND,
    /// Whether the window is the active one (its title and window controls dim when it isn't, as on the Mac).
    active: bool,
    /// Downloads finished at the last refresh, so a newly finished one can be announced (S9).
    finished_ids: Option<std::collections::HashSet<u64>>,
    tray_icon: bool,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// The main window's state. Win32 controls notify synchronously (setting a row's state while the list is rebuilt
/// sends LVN_ITEMCHANGED at once), so a re-entrant call finds the state busy and is skipped instead of panicking:
/// the code changing it refreshes when it's done.
fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.try_borrow_mut().ok()?.as_mut().map(f))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Keep on Top is remembered in the data folder (a test copy with %HD_DATA_DIR% keeps its own).
fn keep_on_top_file() -> std::path::PathBuf {
    crate::data_folder().join("keep-on-top")
}

/// "Today at 10:48", "Yesterday at 09:02", or an older date in the system's short date format with its time, as
/// the Mac writes dates in its own format.
pub(crate) fn date_text(seconds: Option<u64>) -> String {
    use windows::Win32::Globalization::{DATE_SHORTDATE, GetDateFormatEx, GetTimeFormatEx, TIME_NOSECONDS};
    use windows::Win32::Foundation::SYSTEMTIME;
    let Some(s) = seconds else { return String::new() };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let offset = local_offset();
    let local = |t: u64| (t as i64 + offset).max(0) as u64;
    let (t, today) = (local(s), local(now) / 86400);
    let day = t / 86400;
    let (y, mo, d) = civil(day as i64);
    let time = SYSTEMTIME {
        wYear: y as u16, wMonth: mo as u16, wDay: d as u16,
        wHour: ((t % 86400) / 3600) as u16, wMinute: ((t % 3600) / 60) as u16, ..Default::default()
    };
    let mut buffer = [0u16; 64];
    let clock = unsafe {
        let n = GetTimeFormatEx(PCWSTR::null(), TIME_NOSECONDS, Some(&time), PCWSTR::null(), Some(&mut buffer));
        String::from_utf16_lossy(&buffer[..(n.max(1) - 1) as usize])
    };
    if day == today {
        format!("Today at {clock}")
    } else if day + 1 == today {
        format!("Yesterday at {clock}")
    } else {
        let date = unsafe {
            let n = GetDateFormatEx(PCWSTR::null(), DATE_SHORTDATE, Some(&time), PCWSTR::null(), Some(&mut buffer), PCWSTR::null());
            String::from_utf16_lossy(&buffer[..(n.max(1) - 1) as usize])
        };
        format!("{date} at {clock}")
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
        5 => r.connections_text.clone(),
        6 => r.mirrors_text.clone(),
        7 => date_text(Some(r.added)),
        8 => date_text(r.finished),
        9 => r.folder.display().to_string(),
        10 => r.note.clone(),
        _ => String::new(),
    }
}

fn compare(column: usize, a: &Row, b: &Row) -> std::cmp::Ordering {
    match column {
        0 => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        1 => a.status_text.cmp(&b.status_text),
        2 => a.size.cmp(&b.size).then(a.downloaded.cmp(&b.downloaded)),
        3 => a.speed.cmp(&b.speed),
        // An unknown time left counts as the longest.
        4 => a.time_left.unwrap_or(u64::MAX).cmp(&b.time_left.unwrap_or(u64::MAX)),
        5 => a.connections_sort.cmp(&b.connections_sort),
        6 => a.mirrors_sort.cmp(&b.mirrors_sort),
        8 => a.finished.cmp(&b.finished),
        9 => a.folder.cmp(&b.folder),
        10 => a.note.to_lowercase().cmp(&b.note.to_lowercase()),
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
        // §9.2: the system's automatic proxy setup (PAC, auto-detect), set before the Manager makes its client. A test
        // copy can name its own script (`--pac URL`), so the computer's real settings never change for a test.
        let args: Vec<String> = std::env::args().collect();
        let test_pac = std::env::var_os("HD_DATA_DIR")
            .and(args.iter().position(|a| a == "--pac").and_then(|i| args.get(i + 1)).cloned());
        crate::proxy::install(test_pac);
        // One copy per data folder, as on the Mac: a second launch (the Start menu, the desktop shortcut) brings the running
        // copy's window back and ends.
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            crate::data_folder().to_string_lossy().to_lowercase().hash(&mut h);
            format!("{:016x}", h.finish())
        };
        let mutex_name = wide(&format!("Local\\HerbsDownloadManager-{key}"));
        let _mutex = windows::Win32::System::Threading::CreateMutexW(None, false, PCWSTR(mutex_name.as_ptr()));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let class = w!("HerbsDownloadManager");
            let mut after: Option<HWND> = None;
            while let Ok(found) = FindWindowExW(None, after, class, PCWSTR::null()) {
                if found.is_invalid() {
                    break;
                }
                let prop = GetPropW(found, w!("hdm-key"));
                if prop.0 as usize as u64 == u64::from_str_radix(&key[..12], 16).unwrap_or(0) {
                    // Files handed to this copy go to the running one's Import Links.
                    let files = file_arguments();
                    if !files.is_empty() {
                        let text: Vec<u16> = files.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n").encode_utf16().collect();
                        let data = COPYDATASTRUCT { dwData: COPY_FILES, cbData: (text.len() * 2) as u32, lpData: text.as_ptr() as *mut _ };
                        let _ = AllowSetForegroundWindow(ASFW_ANY);
                        SendMessageW(found, WM_COPYDATA, None, Some(LPARAM(&data as *const _ as isize)));
                    }
                    let _ = PostMessageW(Some(found), WM_SHOW_AGAIN, WPARAM(0), LPARAM(0));
                    let _ = AllowSetForegroundWindow(ASFW_ANY);
                    break;
                }
                after = Some(found);
            }
            return;
        }
        let manager = match Manager::open(&crate::data_folder()) {
            // A test copy never fills the Recycle Bin.
            Ok(m) if std::env::var_os("HD_DATA_DIR").is_some() => {
                m.use_trash_folder(&crate::data_folder().join("Trash"));
                m
            }
            Ok(m) => m,
            Err(e) => {
                let text = wide(&format!("Herb's Download Manager can't open its download list:\n{e}"));
                MessageBoxW(None, PCWSTR(text.as_ptr()), w!("Herb's Download Manager"), MB_ICONERROR);
                return;
            }
        };
        // hdm, the command line and MCP server, reaches the app through it (§12); the app works without it.
        let _ = hd_core::control::serve(&manager, crate::downloads_folder());
        // The browser extension reaches the app through hdm.exe beside it (§13): tell the browsers where it is. A test
        // copy leaves them pointing at the installed app.
        if std::env::var_os("HD_DATA_DIR").is_none()
            && let Some(hdm) = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.join("hdm.exe")))
        {
            std::thread::spawn(move || {
                let _ = hd_core::browser::register(&hdm, &crate::data_folder());
            });
        }
        let args: Vec<String> = std::env::args().collect();
        // Test hooks, as on the Mac: `--add URL` (repeatable) adds and starts a download, `--queue URL` adds it to the
        // queue.
        for (i, a) in args.iter().enumerate() {
            if (a == "--add" || a == "--queue")
                && let Some(url) = args.get(i + 1)
            {
                let s = manager.settings();
                manager.add(hd_core::manager::NewDownload {
                    url: url.clone(),
                    folder: crate::save_folder(&manager),
                    connections: s.connections,
                    connections_per_server: s.connections_per_server,
                    retries: s.retries,
                    start: a == "--add",
                    ..Default::default()
                });
            }
        }
        let capture = args.iter().position(|a| a == "--capture").and_then(|i| args.get(i + 1)).map(Into::into);

        theme::init();
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("HerbsDownloadManager"),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            // The app's icon (app.rc), for the window, the taskbar and Alt+Tab.
            hIcon: LoadIconW(Some(instance.into()), PCWSTR(1 as *const u16)).unwrap_or_default(),
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
            // 1100 × 640 as on the Mac, in the screen's scale (the window is per-monitor DPI aware).
            1100 * GetDpiForSystem().max(96) as i32 / 96,
            640 * GetDpiForSystem().max(96) as i32 / 96,
            None,
            None,
            Some(instance.into()),
            None,
        ) else {
            return;
        };
        // A second launch finds this window by its data folder's key.
        let _ = SetPropW(hwnd, w!("hdm-key"), Some(HANDLE(u64::from_str_radix(&key[..12], 16).unwrap_or(0) as usize as *mut _)));
        // The title row is the window's own (chrome.rs): Windows recalculates the frame without its title bar.
        let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        let child = |class: PCWSTR, text: &str, style: u32, ex: WINDOW_EX_STYLE, id: u16| {
            let text = wide(text);
            CreateWindowExW(ex, class, PCWSTR(text.as_ptr()), WS_CHILD | WS_VISIBLE | WINDOW_STYLE(style), 0, 0, 10, 10,
                            Some(hwnd), Some(HMENU(id as usize as *mut _)), Some(instance.into()), None)
                .unwrap_or_default()
        };

        // Fonts as on the Mac (theme.rs): the system font at 13 points, 11 for the footer.
        let fonts = theme::fonts(GetDpiForWindow(hwnd));
        let text_font = fonts.text;
        let set_font = |h: HWND, f: HFONT| {
            SendMessageW(h, WM_SETFONT, Some(WPARAM(f.0 as usize)), Some(LPARAM(1)));
        };
        let tooltips = CreateWindowExW(WS_EX_TOPMOST, TOOLTIPS_CLASSW, PCWSTR::null(), WINDOW_STYLE(TTS_ALWAYSTIP),
                                       CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, Some(hwnd), None,
                                       Some(instance.into()), None)
            .unwrap_or_default();
        theme::control(tooltips);
        // The toolbar's buttons, drawn as the Mac's symbols (chrome.rs); their text is their name, for screen readers.
        let mut tools = Vec::new();
        for (id, _, name, help) in TOOLS {
            let b = child(w!("BUTTON"), name, BS_OWNERDRAW as u32, Default::default(), id);
            crate::a11y::explain(b, name, help);
            chrome::track_tool(b);
            tools.push((id, b));
        }
        let menu_button = tools.iter().find(|(id, _)| *id == C_MENU).map(|t| t.1).unwrap_or_default();

        // Sidebar (§2.1): each filter with its count, its file-type groups under it.
        // Its lines are drawn in chrome.rs; the tree keeps the selection, the keys and opening a filter's types.
        let sidebar = child(WC_TREEVIEWW, "", (TVS_HASBUTTONS | TVS_SHOWSELALWAYS | TVS_FULLROWSELECT | TVS_LINESATROOT | TVS_NOHSCROLL) as u32,
                            Default::default(), ID_SIDEBAR);
        set_font(sidebar, text_font);
        SendMessageW(sidebar, TVM_SETITEMHEIGHT, Some(WPARAM(chrome::px(hwnd, chrome::SIDEBAR_ROW) as usize)), None);
        SendMessageW(sidebar, TVM_SETINDENT, Some(WPARAM(chrome::px(hwnd, 16) as usize)), None);
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

        // No sunken 3D edge: the window draws one-pixel lines between its parts (bands()).
        let list = child(WC_LISTVIEWW, "", LVS_REPORT | LVS_SHOWSELALWAYS | LVS_SHAREIMAGELISTS, Default::default(), ID_LIST);
        set_font(list, text_font);
        SendMessageW(list, LVM_SETEXTENDEDLISTVIEWSTYLE, Some(WPARAM(0)),
                     Some(LPARAM((LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER | LVS_EX_HEADERDRAGDROP) as isize)));
        // Rows are drawn in chrome.rs, 30 pixels high, with the system's small file icons.
        let mut file_info = SHFILEINFOW::default();
        let images = SHGetFileInfoW(w!(".txt"), FILE_ATTRIBUTE_NORMAL, Some(&mut file_info), std::mem::size_of::<SHFILEINFOW>() as u32,
                                    SHGFI_USEFILEATTRIBUTES | SHGFI_SYSICONINDEX | SHGFI_SMALLICON);
        chrome::IMAGES.set(images as isize);
        chrome::CELL_TEXT.set(Some(cell_text));
        chrome::ICON_OF.set(Some(icon_index));
        chrome::row_height(list);
        chrome::subclass_list(list);
        chrome::grip_edge(list, false);
        chrome::grip_edge(sidebar, true);
        let header = HWND(SendMessageW(list, LVM_GETHEADER, None, None).0 as *mut _);
        set_font(header, fonts.small);
        let mut widths = [0; 11];
        let mut shown_columns = [true; 11];
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
        SendMessageW(list, LVM_SETCOLUMNORDERARRAY, Some(WPARAM(COLUMN_ORDER.len())), Some(LPARAM(COLUMN_ORDER.as_ptr() as isize)));

        // The find bar (§2), at the top right of the list, as in Herb's Terminal.
        let find_edit = child(w!("EDIT"), "", ES_AUTOHSCROLL as u32 | WS_BORDER.0, Default::default(), ID_SEARCH);
        set_font(find_edit, text_font);
        SendMessageW(find_edit, EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(w!("Search downloads").0 as isize)));
        let find_done = child(w!("BUTTON"), "Done", BS_PUSHBUTTON as u32, Default::default(), C_FIND_DONE);
        set_font(find_done, text_font);
        crate::a11y::tip(find_done, "Done: hide the search and show every download (Esc)");
        crate::a11y::explain(find_edit, "Search downloads", "Show only downloads whose name, link or note contains this text");
        crate::a11y::explain(sidebar, "Filters", "Show downloads by status; open a filter for its file types");
        crate::a11y::explain(list, "Downloads", "Your downloads: double-click to open, right-click for more, click a column title to sort");
        let _ = ShowWindow(find_edit, SW_HIDE);
        let _ = ShowWindow(find_done, SW_HIDE);

        let footer = child(w!("STATIC"), "", 0x200, Default::default(), 0);
        set_font(footer, fonts.small);

        let keep_on_top = keep_on_top_file().exists();
        STATE.with(|s| {
            *s.borrow_mut() = Some(State {
                manager,
                window: hwnd,
                menu_button,
                tools,
                sidebar,
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
                sidebar_width: 190,
                keep_on_top: false,
                capture: capture.clone(),
                notice: None,
                font: text_font,
                tooltips,
                active: true,
                finished_ids: None,
                tray_icon: false,
            })
        });
        set_keep_on_top(keep_on_top);
        load_layout(hwnd);
        theme::apply(hwnd);
        // Links and files dropped on the window, or on a download in it (§5.4, §7.1 M4).
        crate::dragdrop::register(hwnd, true, "Drop links here");
        // S16: told of every clipboard change (used only while Watch the clipboard is on).
        let _ = windows::Win32::System::DataExchange::AddClipboardFormatListener(hwnd);
        layout(hwnd);
        refresh();
        SetTimer(Some(hwnd), TIMER_REFRESH, 500, None);
        if capture.is_some() {
            SetTimer(Some(hwnd), TIMER_CAPTURE, 2000, None);
        }
        // Opened by the browser extension's right-click menu (§13): minimized, out of the way.
        let background = std::env::args().any(|a| a == "--background");
        let _ = ShowWindow(hwnd, if background {
            SW_SHOWMINNOACTIVE
        } else if LAYOUT_MAXIMIZED.get() {
            SW_SHOWMAXIMIZED
        } else {
            SW_SHOWNORMAL
        });

        // Test hook: `--show new-download|add-links|batch [LINK…]` opens that window over the main one.
        if let Some(i) = args.iter().position(|a| a == "--show")
            && let Some(ctx) = add_ctx()
        {
            let links: Vec<String> = args[i + 2..].iter().take_while(|a| !a.starts_with("--")).cloned().collect();
            match args.get(i + 1).map(String::as_str) {
                Some("new-download") => crate::add::new_download(&ctx, links.into_iter().next()),
                Some("add-links") => crate::add::add_links(&ctx, links),
                Some("batch") => crate::add::batch(&ctx),
                Some("settings") => crate::detail::settings(&ctx),
                Some("download-window") => {
                    if let Some(r) = ctx.manager.rows().first() {
                        let tab = links.first().and_then(|t| ["info", "connections", "mirrors", "options"].iter().position(|n| n == t));
                        crate::detail::open(&ctx, r.id, tab);
                    }
                }
                _ => {}
            }
        }
        // Test hook: --a11y-report FILE opens every window and lists controls without hover text or a name a screen
        // reader can say (a11y.rs).
        if let Some(i) = args.iter().position(|a| a == "--a11y-report")
            && let Some(path) = args.get(i + 1).cloned()
            && let Some(ctx) = add_ctx()
        {
            open_find();
            crate::add::new_download(&ctx, None);
            crate::add::add_links(&ctx, vec!["http://127.0.0.1:9/a.bin".into(), "http://127.0.0.1:9/b.bin".into()]);
            crate::add::batch(&ctx);
            crate::detail::settings(&ctx);
            crate::signins::open(&ctx);
            if let Some(r) = ctx.manager.rows().first() {
                crate::detail::open(&ctx, r.id, None);
            }
            A11Y_REPORT.with(|p| *p.borrow_mut() = Some(path));
            SetTimer(Some(hwnd), TIMER_A11Y, 1500, None);
        }
        // Opened with files ("Open with", a drop on the .exe's icon): Import Links with them.
        let files = file_arguments();
        if !files.is_empty() {
            import_paths(hwnd, &files);
        }
        if let Some(manager) = with(|s| s.manager.clone()) {
            crate::features::run_if_asked(hwnd, &manager);
        }
        let accelerators = accelerator_table();
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            // Tab, Enter and Esc in the add windows (they are dialogs in all but name).
            let root = GetAncestor(msg.hwnd, GA_ROOT);
            if root != hwnd && !root.is_invalid() && IsDialogMessageW(root, &msg).as_bool() {
                continue;
            }
            let in_find = with(|s| GetFocus() == s.find_edit).unwrap_or(false);
            if in_find && msg.message == WM_KEYDOWN && msg.wParam.0 == VK_ESCAPE.0 as usize {
                close_find();
                continue;
            }
            // While typing in the find bar, or any text field (a name being edited in the list), Delete, Ctrl+A,
            // Ctrl+C and Ctrl+V are the field's own.
            let typing = in_find || {
                let mut class = [0u16; 16];
                let n = GetClassNameW(GetFocus(), &mut class);
                String::from_utf16_lossy(&class[..n.max(0) as usize]).eq_ignore_ascii_case("Edit")
            };
            if !typing && TranslateAcceleratorW(hwnd, accelerators, &msg) != 0 {
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
    let keys: [(ACCEL_VIRT_FLAGS, u16, u16); 28] = [
        (ctrl, b'O' as u16, C_DOWNLOAD_WINDOW),
        (ctrl | FALT, b'M' as u16, C_ADD_MIRROR),
        (ctrl, b'N' as u16, C_NEW),
        (ctrl | FSHIFT, b'O' as u16, C_IMPORT),
        (ctrl | FSHIFT, b'E' as u16, C_EXPORT),
        (ctrl | FSHIFT, b'V' as u16, C_CLIPBOARD),
        // Edit ▸ Paste: the clipboard's links (§5.2); a text field keeps its own keys, as accelerators aren't used while
        // typing there.
        (ctrl, b'V' as u16, C_PASTE),
        (ctrl, b'Z' as u16, C_UNDO),
        (ctrl | FSHIFT, b'Z' as u16, C_REDO),
        (ctrl, b'X' as u16, C_CUT),
        (ctrl | FALT, b'S' as u16, C_SIDEBAR),
        (ctrl, b'H' as u16, C_HIDE),
        (ctrl | FSHIFT, b'N' as u16, C_BATCH),
        (ctrl, b'Q' as u16, C_QUIT),
        (ctrl, b'W' as u16, C_CLOSE_WINDOW),
        (ctrl, b'R' as u16, C_RESUME),
        (ctrl, VK_OEM_PERIOD.0, C_PAUSE),
        (ctrl | FSHIFT, b'R' as u16, C_START_QUEUE),
        (ctrl | FALT, VK_OEM_PERIOD.0, C_STOP_ALL),
        (FVIRTKEY, VK_DELETE.0, C_DELETE),
        (ctrl, VK_OEM_COMMA.0, C_SETTINGS),
        (ctrl | FALT, b'T' as u16, C_KEEP_ON_TOP),
        (ctrl, b'F' as u16, C_FIND),
        (ctrl, b'A' as u16, C_SELECT_ALL),
        (ctrl, b'C' as u16, C_COPY),
        (FVIRTKEY, VK_F2.0, C_RENAME),
        (FVIRTKEY, VK_F1.0, C_HELP),
        (ctrl, b'I' as u16, C_PROPERTIES),
    ];
    let table: Vec<ACCEL> = keys.iter().map(|&(f, key, cmd)| ACCEL { fVirt: f, key, cmd }).collect();
    unsafe { CreateAcceleratorTableW(&table).unwrap_or_default() }
}

/// Places the controls: the title row (the toolbar at its right, before the window controls), the sidebar and list
/// below it, the footer at the bottom, the find bar over the list's top right (scaled for the window's DPI).
fn layout(hwnd: HWND) {
    unsafe {
        let scale = GetDpiForWindow(hwnd).max(96) as i32;
        let px = |v: i32| v * scale / 96;
        let mut rect = RECT::default();
        let _ = GetClientRect(hwnd, &mut rect);
        let (w, h) = (rect.right, rect.bottom);
        with(|s| {
            let (bar, foot) = (px(chrome::HEADER), px(chrome::FOOTER));
            let side = if s.sidebar_shown { px(s.sidebar_width) } else { 0 };
            // The toolbar from the window controls leftwards, the Main Menu last, as at the Mac's right edge.
            let tool = px(chrome::TOOL);
            let mut x = w - 3 * px(chrome::CAPTION_BUTTON) - px(10);
            for (_, control) in s.tools.iter().rev() {
                x -= px(chrome::TOOL_STEP);
                let _ = MoveWindow(*control, x + (px(chrome::TOOL_STEP) - tool) / 2, (bar - tool) / 2, tool, tool, true);
            }
            let _ = MoveWindow(s.sidebar, 0, bar, side, h - bar - foot - 1, true);
            let x = if side > 0 { side + 1 } else { 0 };
            let _ = MoveWindow(s.list, x, bar, w - x, h - bar - foot - 1, true);
            fit_columns(s.list, &px);
            let _ = MoveWindow(s.footer, px(12), h - foot, w - px(24), foot, true);
            let _ = MoveWindow(s.find_edit, w - px(330), bar + px(34), px(250), px(26), true);
            let _ = MoveWindow(s.find_done, w - px(74), bar + px(34), px(60), px(26), true);
        });
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

/// The list's columns, wider together than the list, shrink to fit it, each down to its minimum, so every shown column
/// shows whole: Date Added and Date Finished never below their text's width (bugs.md: Date Added was cut off).
fn fit_columns(list: HWND, px: &dyn Fn(i32) -> i32) {
    unsafe {
        let mut rect = RECT::default();
        let _ = GetClientRect(list, &mut rect);
        let room = rect.right - px(4);
        if room < px(200) {
            return;
        }
        let minimum = |i: usize| match COLUMNS[i].0 {
            "Date Added" | "Date Finished" => px(130),
            "Name" => px(120),
            "Location" => px(100),
            "Status" => px(80),
            "Size" => px(70),
            _ => px(46),
        };
        let widths: Vec<i32> = (0..COLUMNS.len()).map(|i| SendMessageW(list, LVM_GETCOLUMNWIDTH, Some(WPARAM(i)), None).0 as i32).collect();
        let shown: Vec<usize> = (0..COLUMNS.len()).filter(|&i| widths[i] > 0).collect();
        let total: i32 = shown.iter().map(|&i| widths[i]).sum();
        let spare: i32 = shown.iter().map(|&i| (widths[i] - minimum(i)).max(0)).sum();
        if total <= room || spare <= 0 {
            return;
        }
        // Each column gives up its share of the excess, in proportion to how far it is above its minimum.
        let excess = (total - room).min(spare);
        for &i in &shown {
            let above = (widths[i] - minimum(i)).max(0);
            let cut = (excess as i64 * above as i64 / spare as i64) as i32;
            let width = (widths[i] - cut).max(minimum(i));
            SendMessageW(list, LVM_SETCOLUMNWIDTH, Some(WPARAM(i)), Some(LPARAM(width as isize)));
        }
    }
}

/// The title row, the footer band and the lines between the window's parts (chrome.rs).
fn bands(hwnd: HWND, dc: HDC) {
    let (side, active) = with(|s| (if s.sidebar_shown { chrome::px(hwnd, s.sidebar_width) } else { 0 }, s.active)).unwrap_or((0, true));
    chrome::paint_bands(hwnd, dc, side, active);
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
        // The sidebar's counts, drawn at the right of each line (chrome.rs).
        let mut counts = vec![0usize; 40];
        for (fi, f) in Filter::ALL.iter().enumerate() {
            for r in rows.iter().filter(|r| f.matches(&r.status)) {
                counts[fi * 8] += 1;
                if let Some(ki) = FileKind::ALL.iter().position(|k| *k == r.kind) {
                    counts[fi * 8 + ki + 1] += 1;
                }
            }
        }
        if chrome::COUNTS.with(|c| c.replace(counts.clone())) != counts {
            let _ = InvalidateRect(Some(s.sidebar), None, false);
        }

        let mut shown: Vec<Row> = rows
            .iter()
            .filter(|r| s.filter.matches(&r.status) && s.kind.is_none_or(|k| r.kind == k))
            .filter(|r| r.matches(&s.search))
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
        chrome::ROWS.with(|r| *r.borrow_mut() = shown.clone());
        chrome::EMPTY_TEXT.with(|t| *t.borrow_mut() = if s.rows.is_empty() { "No downloads yet" } else { "No downloads match" });
        // A bar without a size moves while its download runs.
        for (i, r) in shown.iter().enumerate() {
            if r.fraction.is_none() && r.status.is_running() {
                SendMessageW(s.list, LVM_REDRAWITEMS, Some(WPARAM(i)), Some(LPARAM(i as isize)));
            }
        }
        if shown.is_empty() {
            let _ = InvalidateRect(Some(s.list), None, false);
        }
        // S9: a notification for each download that finished since the last refresh.
        let finished: std::collections::HashSet<u64> = s.rows.iter().filter(|r| r.status == Status::Finished).map(|r| r.id).collect();
        if let Some(before) = &s.finished_ids
            && s.manager.settings().notify_when_finished
        {
            for r in s.rows.iter().filter(|r| finished.contains(&r.id) && !before.contains(&r.id)) {
                // A toast with Open and Show in Folder, as on the Mac; the balloon where Windows shows no toast.
                if !crate::toast::finished(s.window, r.id, &r.name) {
                    notify(s.window, &mut s.tray_icon, &r.name);
                }
                LAST_FINISHED.set(Some(r.id));
            }
        }
        s.finished_ids = Some(finished);

        let active: Vec<&Row> = s.rows.iter().filter(|r| r.status.is_running()).collect();
        keep_awake(!active.is_empty());
        let mut parts = vec![format!("{} selected", selection.len()), format!("{} active", active.len())];
        let speed: u64 = active.iter().map(|r| r.speed).sum();
        if speed > 0 {
            parts.push(format::speed_text(speed));
        }
        parts.extend(s.manager.queue_text());
        if s.notice.as_ref().is_some_and(|(_, at)| at.elapsed().as_secs() >= 6) {
            s.notice = None;
        }
        if let Some((text, _)) = &s.notice {
            parts.push(text.clone());
        }
        let text = wide(&parts.join("  ·  "));
        let _ = SetWindowTextW(s.footer, PCWSTR(text.as_ptr()));
        // The scheduler (§11.1): the countdown runs from its own message, never inside this refresh.
        if let Some(action) = s.manager.take_queue_done() {
            let _ = PostMessageW(Some(s.window), crate::queuedone::WM_QUEUE_DONE, WPARAM(crate::queuedone::code(action)), LPARAM(0));
        }
        for (id, control) in &s.tools {
            let _ = EnableWindow(*control, enabled(*id, &selection, &s.rows));
        }
    });
}

/// Whether a command applies now (§3: buttons that don't apply are disabled), with the Mac's rules.
fn enabled(id: u16, selected: &[Row], rows: &[Row]) -> bool {
    let queue_running = with(|s| s.manager.queue_running()).unwrap_or(false);
    let on_disk = |r: &Row| r.path.as_ref().is_some_and(|p| p.exists());
    let one_on_disk = selected.len() == 1 && on_disk(&selected[0]);
    let unfinished_one = selected.len() == 1 && selected[0].status != Status::Finished;
    match id {
        _ if LATER.contains(&id) => false,
        // With nothing selected, Resume starts what is waiting in the queue (oldest first).
        C_RESUME if selected.is_empty() => !queue_running && rows.iter().any(|r| r.status == Status::Waiting),
        C_RESUME => selected.iter().any(|r| matches!(r.status, Status::Waiting | Status::Paused | Status::Failed(_))),
        C_PAUSE => selected.iter().any(|r| r.status.is_running()),
        C_RESTART => !selected.is_empty(),
        C_STOP_ALL => queue_running || rows.iter().any(|r| r.status.is_running()),
        C_START_QUEUE => !queue_running,
        C_STOP_QUEUE => queue_running,
        C_DELETE | C_COPY_LINK => !selected.is_empty(),
        C_OPEN | C_SHOW_IN_FOLDER | C_RENAME | C_MOVE | C_OPEN_WITH_OTHER => one_on_disk,
        _ if id >= C_OPEN_WITH && id < C_OPEN_WITH + 40 => one_on_disk,
        C_NOTE | C_ADD_MIRROR => selected.len() == 1,
        C_ADD_CONNECTION => unfinished_one && selected[0].allowed_connections < 32,
        C_REMOVE_CONNECTION => unfinished_one && selected[0].allowed_connections > 1,
        C_DELETE_FINISHED => rows.iter().any(|r| r.status == Status::Finished),
        C_DELETE_UNFINISHED => rows.iter().any(|r| r.status != Status::Finished),
        C_DELETE_MISSING => rows.iter().any(|r| r.status == Status::Finished && !on_disk(r)),
        C_DELETE_ALL | C_EXPORT => !rows.is_empty(),
        C_PROPERTIES | C_DOWNLOAD_WINDOW => !selected.is_empty(),
        // Edit ▸ Copy copies the selected downloads' links when no text field has the cursor.
        C_COPY | C_CUT | C_UNDO | C_REDO if typing_field().is_none() => id == C_COPY && !selected.is_empty(),
        _ => true,
    }
}

/// The text field with the keyboard (the find bar, a list name being edited), if one has it.
fn typing_field() -> Option<HWND> {
    unsafe {
        let focus = GetFocus();
        let mut class = [0u16; 16];
        let n = GetClassNameW(focus, &mut class);
        String::from_utf16_lossy(&class[..n.max(0) as usize]).eq_ignore_ascii_case("Edit").then_some(focus)
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
                // 1 is a line that only says something ("No Apps").
                if id == 1 || !enabled(id, selected, rows) {
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
    let sidebar = if with(|s| s.sidebar_shown).unwrap_or(true) { "Hide Sidebar\tCtrl+Alt+S" } else { "Show Sidebar\tCtrl+Alt+S" };
    let mut items: Vec<(&str, u16)> = vec![
        ("File", 0), ("New Download…\tCtrl+N", C_NEW), ("Import Links…\tCtrl+Shift+O", C_IMPORT),
        ("Add from Clipboard\tCtrl+Shift+V", C_CLIPBOARD), ("Batch Download…\tCtrl+Shift+N", C_BATCH),
        ("Export Links…\tCtrl+Shift+E", C_EXPORT), ("-", 0),
        ("Close Window\tCtrl+W", C_CLOSE_WINDOW), (")", 0),
        ("Edit", 0), ("Undo\tCtrl+Z", C_UNDO), ("Redo\tCtrl+Shift+Z", C_REDO), ("-", 0), ("Cut\tCtrl+X", C_CUT),
        ("Copy\tCtrl+C", C_COPY), ("Paste\tCtrl+V", C_PASTE), ("Select All\tCtrl+A", C_SELECT_ALL), ("-", 0), ("Find\tCtrl+F", C_FIND), (")", 0),
        ("Tasks", 0), ("Resume\tCtrl+R", C_RESUME), ("Pause\tCtrl+.", C_PAUSE), ("-", 0),
        ("Start Queue\tCtrl+Shift+R", C_START_QUEUE), ("Stop Queue", C_STOP_QUEUE), ("-", 0),
        ("Stop All\tCtrl+Alt+.", C_STOP_ALL), ("-", 0), ("Add Mirror…\tCtrl+Alt+M", C_ADD_MIRROR), ("Properties\tCtrl+I", C_PROPERTIES), ("-", 0),
        ("Rename…\tF2", C_RENAME), ("Move To…", C_MOVE), ("Note…", C_NOTE), ("-", 0),
        ("Delete", 0), ("Selected\tDelete", C_DELETE), ("All Finished", C_DELETE_FINISHED), ("All Unfinished", C_DELETE_UNFINISHED),
        ("All Missing Files", C_DELETE_MISSING), ("Entire List", C_DELETE_ALL), (")", 0), (")", 0),
        ("View", 0), (sidebar, C_SIDEBAR), ("Keep on Top\tCtrl+Alt+T", C_KEEP_ON_TOP), ("Columns", 0),
    ];
    for (i, (name, _, _)) in COLUMNS.iter().enumerate().skip(1) {
        items.push((name, C_COLUMN + i as u16));
    }
    items.extend_from_slice(&[
        (")", 0), ("Show Download Window\tCtrl+O", C_DOWNLOAD_WINDOW), (")", 0),
        ("Window", 0), ("Minimize", C_MINIMIZE), ("Zoom", C_ZOOM), ("-", 0), ("Bring All to Front", C_BRING_ALL), (")", 0),
        ("Help", 0), ("Herb's Download Manager Help\tF1", C_HELP), (")", 0),
        ("-", 0), ("Settings…\tCtrl+,", C_SETTINGS), ("About Herb's Download Manager", C_ABOUT), ("-", 0),
        ("Hide Herb's DM\tCtrl+H", C_HIDE), ("Quit Herb's DM\tCtrl+Q", C_QUIT),
    ]);
    unsafe {
        let menu = popup(&items, &sel, &rows, &checks);
        let mut rect = RECT::default();
        let _ = GetWindowRect(button, &mut rect);
        // Under the button, its right edge at the button's, as it sits at the toolbar's right end.
        let _ = TrackPopupMenu(menu, TPM_RIGHTALIGN | TPM_TOPALIGN, rect.right, rect.bottom, None, hwnd, None);
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
    // Connections ▸ for one download: how many are in use, then each live connection, as on the Mac.
    let live: Vec<String> = match sel.as_slice() {
        [r] => std::iter::once(format!("{} of {} connections in use", r.active_connections, r.allowed_connections)).chain(r.live.iter().cloned()).collect(),
        _ => Vec::new(),
    };
    let items: Vec<(&str, u16)> = if on_header {
        COLUMNS.iter().enumerate().skip(1).map(|(i, (name, _, _))| (*name, C_COLUMN + i as u16)).collect()
    } else {
        if sel.is_empty() {
            return;
        }
        // As on the Mac: Rename and Move To only for one finished download, Note for one, Connections while unfinished.
        let one = sel.len() == 1;
        let finished_one = one && sel[0].status == Status::Finished;
        let unfinished = sel.iter().all(|r| r.status != Status::Finished);
        let mut v: Vec<(&str, u16)> = vec![("Open", C_OPEN), ("Open With", 0)];
        let apps = OPEN_WITH.with(|o| {
            *o.borrow_mut() = sel.first().and_then(|r| r.path.clone()).filter(|_| one).map(|p| crate::openwith::handlers(&p)).unwrap_or_default();
            o.borrow().iter().map(|a| a.label.clone()).collect::<Vec<_>>()
        });
        let labels: &'static [String] = Box::leak(apps.into_boxed_slice());
        for (i, label) in labels.iter().enumerate() {
            v.push((label.as_str(), C_OPEN_WITH + i as u16));
        }
        if labels.is_empty() {
            v.push((if sel.first().is_some_and(|r| r.path.as_ref().is_some_and(|p| p.exists())) { "No Apps" } else { "No File Yet" }, 1));
        }
        v.extend_from_slice(&[("-", 0), ("Choose Another App…", C_OPEN_WITH_OTHER), (")", 0), ("Show in Folder", C_SHOW_IN_FOLDER)]);
        if finished_one {
            v.extend_from_slice(&[("Rename…", C_RENAME), ("Move To…", C_MOVE)]);
        }
        if one {
            v.push(("Note…", C_NOTE));
        }
        v.extend_from_slice(&[("-", 0), ("Resume", C_RESUME), ("Pause", C_PAUSE), ("Restart…", C_RESTART), ("-", 0), ("Add Mirror…", C_ADD_MIRROR)]);
        if unfinished {
            v.extend_from_slice(&[("Connections", 0), ("Add Connection", C_ADD_CONNECTION), ("Remove Connection", C_REMOVE_CONNECTION)]);
            if !live.is_empty() {
                v.push(("-", 0));
                // 1: lines that only say something, greyed.
                v.extend(live.iter().map(|l| (l.as_str(), 1)));
            }
            v.push((")", 0));
        }
        v.extend_from_slice(&[("-", 0), ("Copy Link", C_COPY_LINK), ("Properties", C_PROPERTIES), ("-", 0), ("Delete…", C_DELETE)]);
        v
    };
    unsafe {
        let menu = popup(&items, &sel, &rows, &checks);
        let _ = TrackPopupMenu(menu, TPM_LEFTALIGN | TPM_TOPALIGN, x, y, None, hwnd, None);
        let _ = DestroyMenu(menu);
    }
}

/// "Download finished" as a notification (the notification area's balloon, which Windows 11 shows as a toast).
fn notify(hwnd: HWND, added: &mut bool, name: &str) {
    unsafe {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            // A click on the balloon shows the file in its folder (WM_TRAY).
            uFlags: NIF_ICON | NIF_TIP | NIF_INFO | NIF_MESSAGE,
            uCallbackMessage: WM_TRAY,
            hIcon: app_icon(),
            dwInfoFlags: NIIF_INFO,
            ..Default::default()
        };
        let put = |dst: &mut [u16], text: &str| {
            let t: Vec<u16> = text.encode_utf16().take(dst.len() - 1).collect();
            dst[..t.len()].copy_from_slice(&t);
        };
        put(&mut data.szTip, "Herb's Download Manager");
        put(&mut data.szInfoTitle, "Download finished");
        put(&mut data.szInfo, name);
        let _ = Shell_NotifyIconW(if *added { NIM_MODIFY } else { NIM_ADD }, &data);
        *added = true;
    }
}

fn set_keep_on_top(on: bool) {
    with(|s| unsafe {
        s.keep_on_top = on;
        let after = if on { HWND_TOPMOST } else { HWND_NOTOPMOST };
        let _ = SetWindowPos(s.window, Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        if let Some((_, b)) = s.tools.iter().find(|(id, _)| *id == C_KEEP_ON_TOP) {
            // The pin, filled and tinted while on (chrome.rs); its hover text says which.
            let _ = InvalidateRect(Some(*b), None, false);
            crate::a11y::tip(*b, if on {
                "Keep on Top is on: Herb's DM stays above other apps' windows. Click to turn it off (Ctrl+Alt+T)"
            } else {
                "Keep on Top: keep Herb's DM above every other app's windows (Ctrl+Alt+T)"
            });
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

pub(crate) fn open_path(path: &std::path::Path, select: bool) {
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

pub(crate) fn copy_text(hwnd: HWND, text: &str) {
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
    let heading = if one { format!("Delete “{}”?", rows[0].name) } else { format!("Delete {} downloads?", rows.len()) };
    // As on the Mac: Remove from List is the default; with files on disk, the three answers stack.
    let alert = if files_gone {
        crate::alert::Alert::new(heading, "Their files are no longer on disk; only the list entries are removed.", &["Cancel", "Remove from List"], 1, Some(0))
    } else {
        crate::alert::Alert::new(
            heading,
            format!("Remove {} from the list only, or also move {} to the Recycle Bin? Files in the Recycle Bin can be restored until it's emptied.",
                    if one { "it" } else { "them" }, if one { "its file" } else { "their files" }),
            &["Remove from List", "Cancel", "Move to Recycle Bin Too"], 0, Some(1))
    };
    let answer = crate::alert::run(hwnd, alert).0;
    let pressed = match (files_gone, answer) {
        (true, Some(1)) | (false, Some(0)) => 1001,
        (false, Some(2)) => 1002,
        _ => 0,
    };
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

/// The main window's tooltip control (for the accessibility audit).
pub(crate) fn main_tooltips() -> Option<HWND> {
    with(|s| s.tooltips)
}

/// The add windows' context, for the feature tests.
pub(crate) fn feature_ctx() -> Option<crate::add::Ctx> {
    add_ctx()
}

/// Selects these downloads in the list (the feature tests' way to point a command at them).
pub(crate) fn select_ids(ids: &[u64]) {
    refresh();
    with(|s| unsafe {
        let none = LVITEMW { stateMask: LVIS_SELECTED, state: LIST_VIEW_ITEM_STATE_FLAGS(0), ..Default::default() };
        SendMessageW(s.list, LVM_SETITEMSTATE, Some(WPARAM(usize::MAX)), Some(LPARAM(&none as *const _ as isize)));
        for (i, id) in s.shown.iter().enumerate() {
            if ids.contains(id) {
                let on = LVITEMW { stateMask: LVIS_SELECTED, state: LVIS_SELECTED, ..Default::default() };
                SendMessageW(s.list, LVM_SETITEMSTATE, Some(WPARAM(i)), Some(LPARAM(&on as *const _ as isize)));
            }
        }
    });
}

/// A toolbar or menu command by name, as its button runs it.
pub(crate) fn run_command(hwnd: HWND, name: &str) -> Result<(), String> {
    let id = match name {
        "resume" => C_RESUME,
        "pause" => C_PAUSE,
        "restart" => C_RESTART,
        "start-queue" => C_START_QUEUE,
        "stop-queue" => C_STOP_QUEUE,
        "stop-all" => C_STOP_ALL,
        other => return Err(format!("no command {other}")),
    };
    command(hwnd, id);
    Ok(())
}

/// What the add windows need from the main window.
fn add_ctx() -> Option<crate::add::Ctx> {
    with(|s| crate::add::Ctx { manager: s.manager.clone(), owner: s.window, font: s.font, done: added })
}

/// Links were added: the footer's note, and the list at once.
fn added(note: Option<String>) {
    if let Some(note) = note {
        with(|s| s.notice = Some((note, std::time::Instant::now())));
    }
    refresh();
}

thread_local! {
    /// The apps the last Open With ▸ listed.
    static OPEN_WITH: RefCell<Vec<crate::openwith::App>> = const { RefCell::new(Vec::new()) };
}

/// The list row under a screen point, when it is an unfinished download (links dropped there become its mirrors,
/// §7.1 M4).
pub(crate) fn mirror_row_at(x: i32, y: i32) -> Option<usize> {
    let list = with(|s| s.list)?;
    unsafe {
        let mut hit = LVHITTESTINFO { pt: POINT { x, y }, ..Default::default() };
        let _ = ScreenToClient(list, &mut hit.pt);
        let i = SendMessageW(list, LVM_HITTEST, None, Some(LPARAM(&mut hit as *mut _ as isize))).0;
        let row = chrome::ROWS.with(|r| r.borrow().get(usize::try_from(i).ok()?).cloned())?;
        (row.status != Status::Finished).then_some(i as usize)
    }
}

/// Frames the row links are dragged over (or none).
pub(crate) fn set_drop_row(row: Option<usize>) {
    if chrome::DROP_ROW.get() != row {
        chrome::DROP_ROW.set(row);
        if let Some(list) = with(|s| s.list) {
            unsafe {
                let _ = InvalidateRect(Some(list), None, false);
            }
        }
    }
}

/// Import Links (§5.7) from these files: Add Links with every link in them, or an alert saying they have none. Also
/// for files handed to the .exe ("Open with", a drop on its icon).
fn import_paths(hwnd: HWND, paths: &[std::path::PathBuf]) {
    let Some(ctx) = add_ctx() else { return };
    match crate::add::links_in_files(paths).filter(|l| !l.is_empty()) {
        Some(links) => crate::add::add_links(&ctx, links),
        None => {
            let heading = match paths {
                [one] => format!("“{}” has no links.", one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
                _ => "These files have no links.".to_string(),
            };
            crate::alert::tell(hwnd, &heading, "Import Links reads text files (with or without an extension), Markdown, rich text \
                                                and HTML, and finds the web addresses (http, https or ftp) in them.");
        }
    }
}

/// Files handed to the .exe ("Open with", a drop on its icon, §5.7): every argument, when none is one of the app's
/// own flags.
fn file_arguments() -> Vec<std::path::PathBuf> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a.starts_with("--")) {
        return Vec::new();
    }
    args.into_iter().map(std::path::PathBuf::from).filter(|p| p.is_file()).collect()
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
            // Edit ▸ Undo, Redo, Cut, Copy, Paste and Select All act on the text field with the cursor; without one,
            // Copy copies the selected links, Paste adds the clipboard's links and Select All selects every download.
            C_UNDO | C_REDO | C_CUT | C_COPY | C_PASTE | C_SELECT_ALL if typing_field().is_some() => {
                let field = typing_field().unwrap_or_default();
                match id {
                    C_UNDO | C_REDO => drop(SendMessageW(field, EM_UNDO, None, None)),
                    C_CUT => drop(SendMessageW(field, WM_CUT, None, None)),
                    C_COPY => drop(SendMessageW(field, WM_COPY, None, None)),
                    C_PASTE => drop(SendMessageW(field, WM_PASTE, None, None)),
                    _ => drop(SendMessageW(field, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)))),
                }
            }
            C_COPY => command(hwnd, C_COPY_LINK),
            C_PASTE => command(hwnd, C_CLIPBOARD),
            // Close Window and the window's close box: the window goes, the app keeps downloading, reached again from
            // its icon in the notification area or the Start menu (the Mac keeps running in the Dock).
            C_CLOSE_WINDOW | C_HIDE => hide_window(hwnd),
            C_SHOW => show_window(hwnd),
            C_BRING_ALL => {
                show_window(hwnd);
                unsafe extern "system" fn raise(h: HWND, _: LPARAM) -> windows::core::BOOL {
                    if unsafe { IsWindowVisible(h) }.as_bool() {
                        let _ = unsafe { SetWindowPos(h, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
                    }
                    windows::core::BOOL(1)
                }
                let _ = EnumThreadWindows(windows::Win32::System::Threading::GetCurrentThreadId(), Some(raise), LPARAM(0));
            }
            C_OPEN_WITH_OTHER => {
                if let Some(p) = sel.first().and_then(|r| r.path.clone()) {
                    crate::openwith::choose(hwnd, &p);
                }
            }
            _ if (C_OPEN_WITH..C_OPEN_WITH + 40).contains(&id) => {
                if let Some(p) = sel.first().and_then(|r| r.path.clone()) {
                    let result = OPEN_WITH.with(|o| o.borrow().get((id - C_OPEN_WITH) as usize).map(|app| crate::openwith::open(app, &p)));
                    if let Some(Err(e)) = result {
                        crate::alert::tell(hwnd, "The file couldn't be opened.", &e);
                    }
                }
            }
            C_MENU => {
                if let Some(button) = with(|s| s.menu_button) {
                    show_main_menu(hwnd, button);
                }
            }
            C_NEW => {
                if let Some(ctx) = add_ctx() {
                    let link = hd_core::links::find(&crate::add::clipboard_text(hwnd)).into_iter().next();
                    crate::add::new_download(&ctx, link);
                }
            }
            C_CLIPBOARD => {
                let links = hd_core::links::find(&crate::add::clipboard_text(hwnd));
                if links.is_empty() {
                    crate::alert::tell(hwnd, "The clipboard has no links.", "Copy one or more web addresses (http, https or ftp), then try again.");
                } else if let Some(ctx) = add_ctx() {
                    crate::add::add_links(&ctx, links);
                }
            }
            C_BATCH => {
                if let Some(ctx) = add_ctx() {
                    crate::add::batch(&ctx);
                }
            }
            C_IMPORT => {
                if let Some(paths) = crate::add::import_files(hwnd) {
                    import_paths(hwnd, &paths);
                }
            }
            // Export Links (§5.8): the selected downloads, or all with none selected, as a tree of names and links.
            C_EXPORT => {
                let today = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) + local_offset()).div_euclid(86_400);
                let (y, m, d) = civil(today);
                if let Some(path) = crate::add::export_file(hwnd, &format!("Herb's DM links {y}-{m:02}-{d:02}.txt")) {
                    let ids: Vec<u64> = sel.iter().map(|r| r.id).collect();
                    let text = with(|s| s.manager.links_tree(&ids)).unwrap_or_default();
                    match std::fs::write(&path, text) {
                        Ok(()) => added(Some(format!("Links saved to {}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()))),
                        Err(e) => crate::alert::tell(hwnd, "The links couldn't be saved.", &e.to_string()),
                    }
                }
            }
            C_ADD_MIRROR => {
                if let (Some(ctx), [r]) = (add_ctx(), sel.as_slice()) {
                    crate::detail::ask_mirrors(&ctx, r.id);
                }
            }
            C_PROPERTIES | C_DOWNLOAD_WINDOW => {
                if let Some(ctx) = add_ctx() {
                    for r in &sel {
                        crate::detail::open(&ctx, r.id, (id == C_PROPERTIES).then_some(0));
                    }
                }
            }
            C_SETTINGS => {
                if let Some(ctx) = add_ctx() {
                    crate::detail::settings(&ctx);
                }
            }
            C_RESUME if ids.is_empty() => drop(with(|s| s.manager.start_queue())),
            C_RESUME => drop(with(|s| s.manager.resume(&ids))),
            C_PAUSE => {
                let running: Vec<Row> = sel.iter().filter(|r| r.status.is_running()).cloned().collect();
                if confirm_pause(hwnd, &running) {
                    let ids: Vec<u64> = running.iter().map(|r| r.id).collect();
                    with(|s| s.manager.pause(&ids));
                }
            }
            C_RESTART => {
                // Restart discards what was downloaded after asking, Cancel the default (§10).
                let heading = if sel.len() == 1 { format!("Restart “{}”?", sel[0].name) } else { format!("Restart {} downloads?", sel.len()) };
                let alert = crate::alert::Alert::new(heading, "What was downloaded is discarded and the download starts again from the beginning.",
                                                     &["Restart", "Cancel"], 1, Some(1));
                if crate::alert::run(hwnd, alert).0 == Some(0) {
                    with(|s| s.manager.restart(&ids));
                }
            }
            C_START_QUEUE => drop(with(|s| s.manager.start_queue())),
            C_STOP_QUEUE => drop(with(|s| s.manager.stop_queue())),
            C_STOP_ALL => {
                if confirm_pause(hwnd, &rows.iter().filter(|r| r.status.is_running()).cloned().collect::<Vec<_>>()) {
                    with(|s| s.manager.pause_all());
                }
            }
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
            // Ctrl+F opens the find bar (or puts the cursor back in it); the toolbar's Search opens or closes it.
            C_FIND if GetFocus() != with(|s| s.find_edit).unwrap_or_default()
                && with(|s| s.find_open).unwrap_or(false)
                && GetKeyState(VK_CONTROL.0 as i32) >= 0 => close_find(),
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
            C_RENAME => {
                if let Some(r) = sel.first().filter(|r| r.status == Status::Finished)
                    && let Some(ctx) = add_ctx()
                    && let Some((v, _)) = crate::signins::form_selecting(&ctx, hwnd, "Rename", &format!("Rename “{}”. The file keeps its folder and stays in the list.", r.name),
                                                               &[crate::signins::Field { label: "New name", value: &r.name, secret: false, help: "The file's new name" }],
                                                               None, "Rename", true)
                    && v[0] != r.name
                    && let Err(e) = ctx.manager.rename_file(r.id, &v[0])
                {
                    message(hwnd, &e);
                }
                refresh();
            }
            C_NOTE => {
                if let Some(r) = sel.first()
                    && let Some(ctx) = add_ctx()
                    && let Some((v, _)) = crate::signins::form(&ctx, hwnd, "Note",
                                                               &format!("Note on “{}”. Shown in the Note column and found by the search; leave it blank to remove it.", r.name),
                                                               &[crate::signins::Field { label: "Note", value: &r.note, secret: false, help: "Your note on this download" }],
                                                               None, "Save")
                {
                    ctx.manager.set_note(r.id, &v[0]);
                }
                refresh();
            }
            C_MOVE => {
                if let Some(r) = sel.first().filter(|r| r.status == Status::Finished)
                    && let Some(folder) = crate::add::choose_folder(hwnd, &format!("Move “{}” To", r.name))
                    && let Some(manager) = with(|s| s.manager.clone())
                    && let Err(e) = manager.move_file(r.id, &folder.join(&r.name))
                {
                    message(hwnd, &e);
                }
                refresh();
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
            // Help (F1): the guide, HELP.md, on the project's page.
            C_HELP => {
                let url = wide(&format!("{GITHUB}/blob/main/HELP.md"));
                ShellExecuteW(None, w!("open"), PCWSTR(url.as_ptr()), None, None, SW_SHOWNORMAL);
            }
            C_ABOUT => about(hwnd),
            C_QUIT => quit(hwnd),
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
        if let Some(r) = chrome::caption_mouse(hwnd, msg, wparam) {
            return r;
        }
        if let Some(r) = theme::ctl_color(msg, wparam, lparam).filter(|_| msg != WM_CTLCOLORSTATIC) {
            return r;
        }
        match msg {
            WM_NCCALCSIZE if wparam.0 == 1 => chrome::calc_size(hwnd, lparam),
            WM_NCHITTEST => chrome::hit_test(hwnd, wparam, lparam),
            WM_ACTIVATE => {
                with(|s| s.active = (wparam.0 & 0xffff) != 0);
                let header = RECT { left: 0, top: 0, right: 10000, bottom: chrome::px(hwnd, chrome::HEADER) };
                let _ = InvalidateRect(Some(hwnd), Some(&header), false);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_GETMINMAXINFO => {
                let info = &mut *(lparam.0 as *mut MINMAXINFO);
                info.ptMinTrackSize = POINT { x: chrome::px(hwnd, 760), y: chrome::px(hwnd, 420) };
                LRESULT(0)
            }
            WM_DPICHANGED => {
                let r = &*(lparam.0 as *const RECT);
                let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                LRESULT(0)
            }
            // Windows' app mode changed: the whole app follows (theme.rs).
            WM_SETTINGCHANGE => {
                let changed = !HWND(lparam.0 as *mut _).is_invalid()
                    && PCWSTR(lparam.0 as *const u16).to_string().is_ok_and(|t| t == "ImmersiveColorSet");
                if changed && theme::reload() {
                    let _ = InvalidateRect(Some(hwnd), None, true);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_DRAWITEM => {
                let item = &*(lparam.0 as *const DRAWITEMSTRUCT);
                let id = item.CtlID as u16;
                if let Some((_, symbol, _, _)) = TOOLS.iter().find(|t| t.0 == id) {
                    let symbol = match symbol {
                        Symbol::KeepOnTop(_) => Symbol::KeepOnTop(with(|s| s.keep_on_top).unwrap_or(false)),
                        other => *other,
                    };
                    chrome::draw_tool(item, symbol);
                }
                LRESULT(1)
            }
            // The sidebar's edge can be dragged, 190 to 360 pixels wide, as on the Mac.
            WM_SETCURSOR if (lparam.0 & 0xffff) as u32 == HTCLIENT && on_sidebar_edge(hwnd) => {
                let _ = SetCursor(LoadCursorW(None, IDC_SIZEWE).ok());
                LRESULT(1)
            }
            WM_LBUTTONDOWN if on_sidebar_edge(hwnd) => {
                SIZING.set(true);
                SetCapture(hwnd);
                LRESULT(0)
            }
            WM_MOUSEMOVE if SIZING.get() => {
                let x = (lparam.0 & 0xffff) as i16 as i32;
                let width = (x * 96 / GetDpiForWindow(hwnd).max(96) as i32).clamp(190, 360);
                with(|s| s.sidebar_width = width);
                layout(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONUP if SIZING.get() => {
                SIZING.set(false);
                let _ = ReleaseCapture();
                LRESULT(0)
            }
            WM_SIZE => {
                layout(hwnd);
                LRESULT(0)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                bands(hwnd, dc);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_PRINTCLIENT => {
                bands(hwnd, HDC(wparam.0 as *mut _));
                LRESULT(0)
            }
            // The footer: small grey text on its band, as on the Mac.
            WM_CTLCOLORSTATIC => {
                let dc = HDC(wparam.0 as *mut _);
                let p = theme::palette();
                SetBkMode(dc, TRANSPARENT);
                SetTextColor(dc, p.secondary);
                LRESULT(theme::brush(p.chrome).0 as isize)
            }
            WM_TIMER if wparam.0 == TIMER_REFRESH => {
                refresh();
                // A download that met a server asking for a login gets a prompt for one.
                if let Some(ctx) = add_ctx() {
                    crate::signins::ask_if_needed(&ctx);
                }
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_A11Y => {
                let _ = KillTimer(Some(hwnd), TIMER_A11Y);
                if let Some(path) = A11Y_REPORT.with(|p| p.borrow_mut().take()) {
                    let _ = std::fs::write(path, crate::a11y::report().join("\n") + "\n");
                }
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_CAPTURE => {
                let _ = KillTimer(Some(hwnd), TIMER_CAPTURE);
                // A sign-in question, an add window the test opened, else the main window.
                let target = [w!("HerbsDMSignInForm"), w!("HerbsDMSignIns"), w!("HerbsDMDetail"), w!("HerbsDMForm")]
                    .iter()
                    .find_map(|c| FindWindowW(*c, PCWSTR::null()).ok().filter(|h| !h.is_invalid()))
                    .unwrap_or(hwnd);
                if let Some(path) = with(|s| s.capture.clone()).flatten()
                    && let Err(e) = capture(target, &path)
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
                    (ID_SIDEBAR, NM_CUSTOMDRAW) => return chrome::sidebar_draw(lparam),
                    (ID_LIST, NM_CUSTOMDRAW) => return chrome::list_draw(lparam, COLUMNS.len()),
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
                        with(|s| {
                            s.sort = if s.sort.0 == column { (column, !s.sort.1) } else { (column, column != 7) };
                            chrome::SORT.set(s.sort);
                            let header = HWND(SendMessageW(s.list, LVM_GETHEADER, None, None).0 as *mut _);
                            let _ = InvalidateRect(Some(header), None, false);
                        });
                        refresh();
                    }
                    (ID_LIST, NM_DBLCLK) => {
                        // §2.2: a finished download opens its file; an unfinished one its Download window.
                        if let Some(r) = selected().into_iter().next() {
                            match (&r.status, &r.path) {
                                (Status::Finished, Some(p)) if p.exists() => open_path(p, false),
                                _ => {
                                    if let Some(ctx) = add_ctx() {
                                        crate::detail::open(&ctx, r.id, None);
                                    }
                                }
                            }
                        }
                    }
                    (ID_LIST, LVN_ITEMCHANGED) => refresh(),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_TRAY => {
                match lparam.0 as u32 & 0xffff {
                    // A click on a "Download finished" notification shows the file in its folder.
                    NIN_BALLOONUSERCLICK => {
                        if let Some(id) = LAST_FINISHED.get()
                            && let Some(r) = with(|s| s.manager.rows().into_iter().find(|r| r.id == id)).flatten()
                            && let Some(p) = r.path.filter(|p| p.exists())
                        {
                            open_path(&p, true);
                        }
                    }
                    WM_LBUTTONUP => show_window(hwnd),
                    WM_RBUTTONUP => {
                        let mut pt = POINT::default();
                        let _ = GetCursorPos(&mut pt);
                        let menu = CreatePopupMenu().unwrap_or_default();
                        let _ = AppendMenuW(menu, MF_STRING, C_SHOW as usize, w!("Show Herb's Download Manager"));
                        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
                        let _ = AppendMenuW(menu, MF_STRING, C_QUIT as usize, w!("Quit Herb's DM"));
                        let _ = SetForegroundWindow(hwnd);
                        let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, None, hwnd, None);
                        let _ = DestroyMenu(menu);
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            crate::toast::WM_TOAST => {
                // The toast's buttons: Open, or Show in Folder (also a click on the toast). The file may have been
                // renamed or moved since: it is looked up now.
                let id = wparam.0 as u64;
                if let Some(r) = with(|s| s.manager.rows().into_iter().find(|r| r.id == id)).flatten()
                    && let Some(p) = r.path.filter(|p| p.exists())
                {
                    open_path(&p, lparam.0 != crate::toast::OPEN);
                }
                LRESULT(0)
            }
            crate::queuedone::WM_QUEUE_DONE => {
                crate::queuedone::countdown(hwnd, wparam.0);
                LRESULT(0)
            }
            crate::features::WM_FEATURE => {
                crate::features::on_main(hwnd, &mut *(lparam.0 as *mut crate::features::Request));
                LRESULT(0)
            }
            crate::dragdrop::WM_DROPPED => {
                let dropped = Box::from_raw(lparam.0 as *mut crate::dragdrop::Dropped);
                let links = dropped.links;
                // On an unfinished download in the list: its mirrors (§7.1 M4); anywhere else, finished downloads
                // included: Add Links.
                match mirror_row_at(dropped.x, dropped.y).and_then(|i| with(|s| s.shown.get(i).copied()).flatten()) {
                    Some(id) => {
                        let note = with(|s| {
                            let n = s.manager.add_mirrors(id, &links);
                            let name = s.manager.rows().into_iter().find(|r| r.id == id).map(|r| r.name).unwrap_or_default();
                            match n {
                                0 => format!("“{name}” already has those links"),
                                1 => format!("Added as a mirror of “{name}”"),
                                n => format!("Added {n} mirrors of “{name}”"),
                            }
                        });
                        added(note);
                    }
                    None => {
                        if let Some(ctx) = add_ctx() {
                            crate::add::add_links(&ctx, links);
                        }
                    }
                }
                LRESULT(0)
            }
            WM_CLIPBOARDUPDATE => {
                // S16, Watch the clipboard: links copied in other apps open Add Links; copies made in this app (Copy
                // Link) are left alone.
                let foreground = GetForegroundWindow();
                let mut pid = 0u32;
                GetWindowThreadProcessId(foreground, Some(&mut pid));
                let ours = pid == windows::Win32::System::Threading::GetCurrentProcessId();
                if !ours && with(|s| s.manager.settings().watch_clipboard).unwrap_or(false) {
                    let links = hd_core::links::find(&crate::add::clipboard_text(hwnd));
                    if !links.is_empty()
                        && let Some(ctx) = add_ctx()
                    {
                        crate::add::add_links(&ctx, links);
                    }
                }
                LRESULT(0)
            }
            WM_DROPFILES => {
                // Links in text files dropped on the window open Add Links (§5.4).
                let paths = crate::add::dropped_files(HDROP(wparam.0 as *mut _));
                if let Some(ctx) = add_ctx() {
                    match crate::add::links_in_files(&paths) {
                        Some(links) => crate::add::add_links(&ctx, links),
                        None => added(Some("No links found in what was dropped".into())),
                    }
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
            // The close box closes the window; the app keeps running, as the Mac's does (Quit, Ctrl+Q, quits).
            WM_CLOSE => {
                hide_window(hwnd);
                LRESULT(0)
            }
            WM_COPYDATA => {
                let data = &*(lparam.0 as *const COPYDATASTRUCT);
                if data.dwData == COPY_FILES && !data.lpData.is_null() {
                    let text = String::from_utf16_lossy(std::slice::from_raw_parts(data.lpData as *const u16, data.cbData as usize / 2));
                    let paths: Vec<std::path::PathBuf> = text.lines().map(std::path::PathBuf::from).collect();
                    // After this message returns, so the copy that sent it can end.
                    PENDING_FILES.with(|p| p.borrow_mut().extend(paths));
                    let _ = PostMessageW(Some(hwnd), WM_IMPORT_FILES, WPARAM(0), LPARAM(0));
                }
                LRESULT(1)
            }
            WM_IMPORT_FILES => {
                show_window(hwnd);
                let paths: Vec<std::path::PathBuf> = PENDING_FILES.with(|p| p.borrow_mut().drain(..).collect());
                if !paths.is_empty() {
                    import_paths(hwnd, &paths);
                }
                LRESULT(0)
            }
            WM_SHOW_AGAIN => {
                show_window(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                save_layout(hwnd);
                if with(|s| s.tray_icon).unwrap_or(false) {
                    let data = NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 1, ..Default::default() };
                    let _ = Shell_NotifyIconW(NIM_DELETE, &data);
                }
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

fn layout_file() -> std::path::PathBuf {
    crate::data_folder().join("window-layout.txt")
}

/// The window's place and size, the sidebar's width and whether it shows, the columns' widths, order and which show,
/// and the sort, as the Mac keeps them (a test copy keeps its own, in its data folder).
fn save_layout(hwnd: HWND) {
    let Some(text) = with(|s| unsafe {
        let mut place = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
        let _ = GetWindowPlacement(hwnd, &mut place);
        let r = place.rcNormalPosition;
        let widths: Vec<String> = (0..COLUMNS.len())
            .map(|i| if s.columns[i] { SendMessageW(s.list, LVM_GETCOLUMNWIDTH, Some(WPARAM(i)), None).0 as i32 } else { s.widths[i] })
            .map(|w| (w * 96 / GetDpiForWindow(hwnd).max(96) as i32).to_string())
            .collect();
        let mut order = [0i32; 11];
        SendMessageW(s.list, LVM_GETCOLUMNORDERARRAY, Some(WPARAM(order.len())), Some(LPARAM(order.as_mut_ptr() as isize)));
        let join = |v: &[String]| v.join(",");
        format!(
            "frame={},{},{},{}\nmaximized={}\nsidebar={},{}\nwidths={}\norder={}\nshown={}\nsort={},{}\n",
            r.left, r.top, r.right, r.bottom, (place.showCmd == SW_SHOWMAXIMIZED.0 as u32) as u8,
            s.sidebar_shown as u8, s.sidebar_width, join(&widths),
            join(&order.iter().map(|o| o.to_string()).collect::<Vec<_>>()),
            join(&s.columns.iter().map(|c| (*c as u8).to_string()).collect::<Vec<_>>()),
            s.sort.0, s.sort.1 as u8,
        )
    }) else { return };
    let _ = std::fs::write(layout_file(), text);
}

fn load_layout(hwnd: HWND) {
    let Ok(text) = std::fs::read_to_string(layout_file()) else { return };
    let get = |key: &str| -> Vec<i32> {
        text.lines().find_map(|l| l.strip_prefix(&format!("{key}="))).map(|v| v.split(',').filter_map(|n| n.trim().parse().ok()).collect()).unwrap_or_default()
    };
    let scale = unsafe { GetDpiForWindow(hwnd).max(96) as i32 };
    unsafe {
        if let [l, t, r, b] = get("frame")[..] {
            // Only where it is still on a screen.
            let rect = RECT { left: l, top: t, right: r, bottom: b };
            if r - l > 200 && b - t > 150 && !windows::Win32::Graphics::Gdi::MonitorFromRect(&rect, MONITOR_DEFAULTTONULL).is_invalid() {
                let mut place = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
                let _ = GetWindowPlacement(hwnd, &mut place);
                place.rcNormalPosition = rect;
                place.showCmd = SW_HIDE.0 as u32;
                let _ = SetWindowPlacement(hwnd, &place);
            }
        }
        if get("maximized").first() == Some(&1) {
            LAYOUT_MAXIMIZED.set(true);
        }
    }
    with(|s| unsafe {
        if let [shown, width] = get("sidebar")[..] {
            s.sidebar_shown = shown == 1;
            s.sidebar_width = width.clamp(190, 360);
            let _ = ShowWindow(s.sidebar, if s.sidebar_shown { SW_SHOW } else { SW_HIDE });
        }
        let widths = get("widths");
        let shown = get("shown");
        if widths.len() == COLUMNS.len() && shown.len() == COLUMNS.len() {
            for i in 0..COLUMNS.len() {
                s.widths[i] = widths[i].max(30);
                s.columns[i] = i == 0 || shown[i] == 1;
                let w = if s.columns[i] { s.widths[i] * scale / 96 } else { 0 };
                SendMessageW(s.list, LVM_SETCOLUMNWIDTH, Some(WPARAM(i)), Some(LPARAM(w as isize)));
            }
        }
        let order = get("order");
        if order.len() == COLUMNS.len() {
            SendMessageW(s.list, LVM_SETCOLUMNORDERARRAY, Some(WPARAM(order.len())), Some(LPARAM(order.as_ptr() as isize)));
        }
        if let [column, ascending] = get("sort")[..]
            && (0..COLUMNS.len() as i32).contains(&column)
        {
            s.sort = (column as usize, ascending == 1);
            chrome::SORT.set(s.sort);
        }
    });
}

thread_local! {
    static LAYOUT_MAXIMIZED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The window goes to the notification area: the app keeps running and downloading.
fn hide_window(hwnd: HWND) {
    unsafe {
        tray_icon(hwnd, true);
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

fn show_window(hwnd: HWND) {
    tray_icon(hwnd, false);
    chrome::reset_captions();
    unsafe {
        if IsIconic(hwnd).as_bool() || !IsWindowVisible(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = ShowWindow(hwnd, SW_SHOW);
        }
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Quit (Ctrl+Q): while downloads run it asks first, Cancel the default (§10); they start again at the next launch.
fn quit(hwnd: HWND) {
    let running = with(|s| s.manager.any_running()).unwrap_or(false);
    if running && crate::signins::ASKS.get() {
        show_window(hwnd);
        let alert = crate::alert::Alert::new("Downloads are running. Pause them and quit?",
                                             "They continue the next time you open Herb's Download Manager.",
                                             &["Pause and Quit", "Cancel"], 1, Some(1));
        if crate::alert::run(hwnd, alert).0 != Some(0) {
            return;
        }
    }
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

/// The app's icon in the notification area: there while the window is closed, and for "Download finished".
fn tray_icon(hwnd: HWND, on: bool) {
    let added = with(|s| s.tray_icon).unwrap_or(false);
    if added == on {
        return;
    }
    unsafe {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_ICON | NIF_TIP | NIF_MESSAGE,
            uCallbackMessage: WM_TRAY,
            hIcon: app_icon(),
            ..Default::default()
        };
        let tip: Vec<u16> = "Herb's Download Manager".encode_utf16().collect();
        data.szTip[..tip.len()].copy_from_slice(&tip);
        let _ = Shell_NotifyIconW(if on { NIM_ADD } else { NIM_DELETE }, &data);
    }
    with(|s| s.tray_icon = on);
}

/// The app's small icon (app.rc).
fn app_icon() -> HICON {
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let size = GetSystemMetrics(SM_CXSMICON);
        LoadImageW(Some(instance.into()), PCWSTR(1 as *const u16), IMAGE_ICON, size, size, LR_DEFAULTCOLOR)
            .map(|h| HICON(h.0))
            .unwrap_or_default()
    }
}

thread_local! {
    /// The sidebar's edge is being dragged.
    static SIZING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether the pointer is on the sidebar's right edge (a few pixels either side of its line).
fn on_sidebar_edge(hwnd: HWND) -> bool {
    unsafe {
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = ScreenToClient(hwnd, &mut pt);
        let Some((shown, width)) = with(|s| (s.sidebar_shown, s.sidebar_width)) else { return false };
        let edge = chrome::px(hwnd, width);
        shown && pt.y > chrome::px(hwnd, chrome::HEADER) && (pt.x - edge).abs() <= chrome::px(hwnd, 3)
    }
}

/// A short message over the window.
fn message(hwnd: HWND, text: &str) {
    crate::alert::tell(hwnd, text, "");
}

/// Pausing downloads no source can resume asks first, Cancel the default (§10). True to go ahead.
fn confirm_pause(hwnd: HWND, rows: &[Row]) -> bool {
    let lost: Vec<&Row> = rows.iter().filter(|r| !r.resumable).collect();
    if lost.is_empty() {
        return true;
    }
    let heading = if lost.len() == 1 && rows.len() == 1 {
        "This download can't be resumed and will start over. Pause anyway?".to_string()
    } else if lost.len() == 1 {
        format!("“{}” can't be resumed and will start over. Pause anyway?", lost[0].name)
    } else {
        format!("{} of these downloads can't be resumed and will start over. Pause anyway?", lost.len())
    };
    crate::alert::run(hwnd, crate::alert::Alert::new(heading, "", &["Pause", "Cancel"], 1, Some(1))).0 == Some(0)
}

/// About: the name, version, what it is, the author and a link to the project, which opens in the browser.
fn about(hwnd: HWND) {
    let mut alert = crate::alert::Alert::new(
        format!("Herb's Download Manager {}", hd_core::version()),
        "A download manager\nMany connections · Mirrors · Streams\n\nBy dunkelherzmann",
        &["OK"],
        0,
        Some(0),
    );
    alert.link = Some(GITHUB.to_string());
    crate::alert::run(hwnd, alert);
}

thread_local! {
    static AWAKE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The PC doesn't go to sleep while a download runs (SPECIFICATION.md §10): the window's thread asks Windows to keep
/// the system on while any download is active, and lets go when none is. The display still turns off as set.
fn keep_awake(downloading: bool) {
    use windows::Win32::System::Power::{ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState};
    if AWAKE.get() != downloading {
        unsafe {
            SetThreadExecutionState(if downloading { ES_CONTINUOUS | ES_SYSTEM_REQUIRED } else { ES_CONTINUOUS });
        }
        AWAKE.set(downloading);
    }
}

