//! Adding downloads (§5) on Windows: New Download (Ctrl+N), Add Links (several links, from the clipboard, a drop,
//! Import or Batch), Batch Download (Ctrl+Shift+N) and Import Links (Ctrl+Shift+O). Every add goes through the core's
//! `AddSession`; a name in use (§5.6) is asked with a task dialog whose checkbox is "Do the same for the rest".
//! Servers are asked about links on background threads, which post their answers back to the window.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use hd_core::addflow::{AddSession, Choice, PendingLink, Step};
use hd_core::format;
use hd_core::manager::Manager;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::UI::Controls::Dialogs::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, PCWSTR, PWSTR, w};

const WM_PROBED: u32 = WM_APP + 1;
/// A page scan's answer (§15), posted back to Add Links (WM_APP + 2: see app.rs's list of private messages).
const WM_SCANNED: u32 = WM_APP + 2;
const TIMER_PROBE: usize = 10;
const TIMER_EARLY: usize = 11;

// Enter sends IDOK (1) and Esc IDCANCEL (2) through IsDialogMessage: Download (or Next) and Cancel.
const ID_DOWNLOAD: u16 = 1;
const ID_CANCEL: u16 = 2;
const ID_ADD: u16 = 3;
const ID_CHOOSE: u16 = 4;
const ID_LINK: u16 = 5;
const ID_FILTER: u16 = 6;
const ID_CHECK_ALL: u16 = 7;
const ID_UNCHECK_ALL: u16 = 8;
const ID_MIRRORS: u16 = 9;
const ID_LIST: u16 = 10;
const ID_PATTERN: u16 = 11;
/// "Only the sound" (New Download) and "Only the sound of streamed videos" (Add Links).
const ID_SOUND: u16 = 12;
const ID_NEXT: u16 = ID_DOWNLOAD;

pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub(crate) fn text_of(h: HWND) -> String {
    unsafe {
        let n = GetWindowTextLengthW(h).max(0) as usize;
        let mut buffer = vec![0u16; n + 1];
        let got = GetWindowTextW(h, &mut buffer);
        String::from_utf16_lossy(&buffer[..got as usize])
    }
}

pub(crate) fn set_text(h: HWND, text: &str) {
    let t = wide(text);
    unsafe {
        let _ = SetWindowTextW(h, PCWSTR(t.as_ptr()));
    }
}

/// Adds the links, asking about names in use, and returns the footer's summary.
pub fn add_all(owner: HWND, manager: &Manager, links: Vec<PendingLink>, folder: PathBuf, connections: u32, start: bool) -> Option<String> {
    let mut session = AddSession::new(manager, links, &folder, connections, start);
    loop {
        match session.step(manager) {
            Step::Done(summary) => return summary.text(),
            Step::Ask(q) => {
                let (title, body) = (wide(&q.title()), wide(&q.body()));
                let labels = [wide("Keep Both"), wide("Skip"), wide("Replace (the old file goes to the Recycle Bin)")];
                let buttons = [
                    TASKDIALOG_BUTTON { nButtonID: 1, pszButtonText: PCWSTR(labels[0].as_ptr()) },
                    TASKDIALOG_BUTTON { nButtonID: 2, pszButtonText: PCWSTR(labels[1].as_ptr()) },
                    TASKDIALOG_BUTTON { nButtonID: 3, pszButtonText: PCWSTR(labels[2].as_ptr()) },
                ];
                let rest = wide("Do the same for the rest");
                let config = TASKDIALOGCONFIG {
                    cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
                    hwndParent: owner,
                    pszWindowTitle: w!("Herb's Download Manager"),
                    pszMainInstruction: PCWSTR(title.as_ptr()),
                    pszContent: PCWSTR(body.as_ptr()),
                    cButtons: 3,
                    pButtons: buttons.as_ptr(),
                    nDefaultButton: 1,
                    pszVerificationText: if q.more { PCWSTR(rest.as_ptr()) } else { PCWSTR::null() },
                    ..Default::default()
                };
                let (mut pressed, mut for_rest) = (0i32, BOOL(0));
                unsafe {
                    let _ = TaskDialogIndirect(&config, Some(&mut pressed), None, Some(&mut for_rest));
                }
                let choice = match pressed {
                    1 => Choice::KeepBoth,
                    3 => Choice::Replace,
                    _ => Choice::Skip,
                };
                session.answer(manager, choice, for_rest.as_bool());
            }
        }
    }
}

/// A probe's answer, posted back to the window that asked.
struct Probed {
    url: String,
    result: Result<hd_core::manager::Probe, String>,
}

fn probe_in_background(window: HWND, manager: Manager, urls: Vec<String>) {
    let handle = window.0 as isize;
    let queue = std::sync::Arc::new(std::sync::Mutex::new(urls));
    for _ in 0..4 {
        let (queue, manager) = (queue.clone(), manager.clone());
        std::thread::spawn(move || {
            loop {
                let Some(url) = queue.lock().unwrap().pop() else { break };
                let result = manager.probe(&url);
                let answer = Box::new(Probed { url, result });
                unsafe {
                    if PostMessageW(Some(HWND(handle as *mut _)), WM_PROBED, WPARAM(0), LPARAM(Box::into_raw(answer) as isize)).is_err() {
                        break;
                    }
                }
            }
        });
    }
}

/// A page scan's answer.
struct Scanned {
    url: String,
    result: Result<hd_core::media::scan::Scan, String>,
}

fn scan_in_background(window: HWND, manager: Manager, url: String) {
    let handle = window.0 as isize;
    std::thread::spawn(move || {
        let result = manager.scan_page(&url);
        let answer = Box::new(Scanned { url, result });
        unsafe {
            let _ = PostMessageW(Some(HWND(handle as *mut _)), WM_SCANNED, WPARAM(0), LPARAM(Box::into_raw(answer) as isize));
        }
    });
}

/// Chooses a folder (the shell's folder browser).
pub(crate) fn choose_folder(owner: HWND, title: &str) -> Option<PathBuf> {
    unsafe {
        let title = wide(title);
        let info = BROWSEINFOW {
            hwndOwner: owner,
            lpszTitle: PCWSTR(title.as_ptr()),
            ulFlags: BIF_RETURNONLYFSDIRS | 0x50 /* BIF_USENEWUI */,
            ..Default::default()
        };
        let list = SHBrowseForFolderW(&info);
        if list.is_null() {
            return None;
        }
        let mut path = [0u16; 260];
        let ok = SHGetPathFromIDListW(list, &mut path).as_bool();
        windows::Win32::System::Com::CoTaskMemFree(Some(list as *const _));
        ok.then(|| PathBuf::from(String::from_utf16_lossy(&path[..path.iter().position(|&c| c == 0).unwrap_or(0)])))
    }
}

/// The clipboard's text, if any.
pub fn clipboard_text(owner: HWND) -> String {
    unsafe {
        if OpenClipboard(Some(owner)).is_err() {
            return String::new();
        }
        let mut out = String::new();
        if let Ok(data) = GetClipboardData(CF_UNICODETEXT.0 as u32) {
            let memory = HGLOBAL(data.0);
            let p = GlobalLock(memory) as *const u16;
            if !p.is_null() {
                let mut n = 0;
                while *p.add(n) != 0 {
                    n += 1;
                }
                out = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
                let _ = GlobalUnlock(memory);
            }
        }
        let _ = CloseClipboard();
        out
    }
}

/// Import Links (§5.7): the links in one or more text files chosen with the Open dialog.
pub fn import_files(owner: HWND) -> Option<Vec<PathBuf>> {
    unsafe {
        let mut buffer = vec![0u16; 32 * 1024];
        let filter = wide("Text files (*.txt;*.md;*.csv;*.html;*.htm;*.xml;*.json;*.rtf)\0*.txt;*.md;*.csv;*.html;*.htm;*.xml;*.json;*.rtf\0All files\0*.*\0");
        let mut ofn = OPENFILENAMEW {
            lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: owner,
            lpstrFilter: PCWSTR(filter.as_ptr()),
            lpstrFile: PWSTR(buffer.as_mut_ptr()),
            nMaxFile: buffer.len() as u32,
            lpstrTitle: w!("Import Links"),
            Flags: OFN_ALLOWMULTISELECT | OFN_EXPLORER | OFN_FILEMUSTEXIST,
            ..Default::default()
        };
        if !GetOpenFileNameW(&mut ofn).as_bool() {
            return None;
        }
        // One path, or a folder followed by file names, separated by NULs and ended by two.
        let parts: Vec<String> = buffer.split(|&c| c == 0).take_while(|p| !p.is_empty()).map(String::from_utf16_lossy).collect();
        Some(match parts.as_slice() {
            [one] => vec![PathBuf::from(one)],
            [dir, names @ ..] => names.iter().map(|n| PathBuf::from(dir).join(n)).collect(),
            [] => Vec::new(),
        })
    }
}

/// The links in files (dropped or imported); `None` when none of them is text.
pub fn links_in_files(paths: &[PathBuf]) -> Option<Vec<String>> {
    let texts: Vec<String> = paths.iter().filter_map(|p| hd_core::links::text_of_file(p)).collect();
    (!texts.is_empty()).then(|| hd_core::links::find(&texts.join("\n")))
}

/// Names (where a control's text isn't one) and hover texts for a form's controls.
fn explain_ids(hwnd: HWND, items: &[(u16, &str, &str)]) {
    with_form(hwnd, |f| {
        for &(id, name, help) in items {
            if let Some(&c) = f.controls.get(&id) {
                if name.is_empty() { crate::a11y::tip(c, help) } else { crate::a11y::explain(c, name, help) }
            }
        }
    });
}

// --- Dialog windows ---------------------------------------------------------------------------------------------

/// What every dialog needs from the main window.
#[derive(Clone)]
pub struct Ctx {
    pub manager: Manager,
    pub owner: HWND,
    pub font: HFONT,
    /// Called with the footer's note (and to refresh the list) once links were added.
    pub done: fn(Option<String>),
}

struct Form {
    ctx: Ctx,
    kind: FormKind,
    controls: HashMap<u16, HWND>,
    folder: PathBuf,
    probed_size: Option<u64>,
    /// New Download's early download (enhancements.md), once the link proved resumable: Download or Add adopts it,
    /// Cancel discards it. With the size line it adds to.
    early: Option<u64>,
    size_text: String,
    /// Add Links' rows: url, name, size, size text, checked.
    items: Vec<(String, String, Option<u64>, String, bool)>,
    /// Rows that are streams a page scan found: added as streams (§15).
    streams: std::collections::HashSet<String>,
    /// New Download's link is a web page: Download and Add open Add Links with its files instead.
    page: bool,
    shown: Vec<usize>,
    batch: Vec<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum FormKind {
    NewDownload,
    AddLinks,
    Batch,
}

thread_local! {
    static FORMS: RefCell<HashMap<isize, Form>> = RefCell::new(HashMap::new());
    static OPEN_LINKS: RefCell<Option<HWND>> = const { RefCell::new(None) };
}

/// The form's state. A control that notifies synchronously while the form is being changed (a list checkbox set
/// from code) finds it busy and is skipped: the code changing it already knows the new state.
fn with_form<R>(hwnd: HWND, f: impl FnOnce(&mut Form) -> R) -> Option<R> {
    FORMS.with(|forms| forms.try_borrow_mut().ok()?.get_mut(&(hwnd.0 as isize)).map(f))
}

fn register(class: PCWSTR) {
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(form_proc),
            hInstance: instance.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            ..Default::default()
        };
        RegisterClassW(&wc);
    }
}

/// Creates a dialog window and its controls; `controls` is (id, class, text, style, x, y, w, h) in 96-dpi pixels.
#[allow(clippy::type_complexity)]
fn open_form(ctx: &Ctx, kind: FormKind, title: &str, size: (i32, i32), controls: &[(u16, PCWSTR, &str, u32, i32, i32, i32, i32)]) -> HWND {
    register(w!("HerbsDMForm"));
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let scale = GetDpiForWindow(ctx.owner).max(96) as i32;
        let px = |v: i32| v * scale / 96;
        let t = wide(title);
        let mut rect = RECT { left: 0, top: 0, right: px(size.0), bottom: px(size.1) };
        let _ = AdjustWindowRectEx(&mut rect, WS_CAPTION | WS_SYSMENU, false, WINDOW_EX_STYLE::default());
        let hwnd = CreateWindowExW(
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
            w!("HerbsDMForm"),
            PCWSTR(t.as_ptr()),
            WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            rect.right - rect.left,
            rect.bottom - rect.top,
            Some(ctx.owner),
            None,
            Some(instance.into()),
            None,
        )
        .unwrap_or_default();
        let mut map = HashMap::new();
        for &(id, class, text, style, x, y, w, h) in controls {
            let text = wide(text);
            let ex = if class == WC_LISTVIEWW || (class == w!("EDIT")) { WS_EX_CLIENTEDGE } else { WINDOW_EX_STYLE::default() };
            let c = CreateWindowExW(ex, class, PCWSTR(text.as_ptr()), WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(style),
                                    px(x), px(y), px(w), px(h), Some(hwnd), Some(HMENU(id as usize as *mut _)), Some(instance.into()), None)
                .unwrap_or_default();
            SendMessageW(c, WM_SETFONT, Some(WPARAM(ctx.font.0 as usize)), Some(LPARAM(1)));
            if id != 0 {
                map.insert(id, c);
            }
        }
        FORMS.with(|forms| {
            forms.borrow_mut().insert(hwnd.0 as isize, Form {
                ctx: ctx.clone(),
                kind,
                controls: map,
                folder: crate::save_folder(&ctx.manager),
                probed_size: None,
                early: None,
                size_text: String::new(),
                items: Vec::new(),
                streams: Default::default(),
                page: false,
                shown: Vec::new(),
                batch: Vec::new(),
            })
        });
        hwnd
    }
}

pub(crate) const LABEL: u32 = 0x2; // SS_RIGHT
pub(crate) const EDIT: u32 = ES_AUTOHSCROLL as u32;
pub(crate) const BUTTON: u32 = BS_PUSHBUTTON as u32;
pub(crate) const DEFAULT: u32 = BS_DEFPUSHBUTTON as u32;

/// New Download (§5.1).
pub fn new_download(ctx: &Ctx, link: Option<String>) {
    let multi = (ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32 | WS_VSCROLL.0;
    let hwnd = open_form(ctx, FormKind::NewDownload, "New Download", (560, 374), &[
        (0, w!("STATIC"), "Link:", LABEL, 12, 18, 90, 20),
        (ID_LINK, w!("EDIT"), "", EDIT, 110, 14, 430, 24),
        (0, w!("STATIC"), "Mirrors:", LABEL, 12, 50, 90, 20),
        (20, w!("EDIT"), "", multi, 110, 48, 430, 70),
        (0, w!("STATIC"), "Name:", LABEL, 12, 132, 90, 20),
        (21, w!("EDIT"), "", EDIT, 110, 128, 430, 24),
        (0, w!("STATIC"), "Save to:", LABEL, 12, 168, 90, 20),
        (22, w!("STATIC"), &crate::save_folder(&ctx.manager).display().to_string(), 0x4000 /* SS_PATHELLIPSIS */, 110, 168, 330, 20),
        (ID_CHOOSE, w!("BUTTON"), "Choose…", BUTTON, 450, 162, 90, 28),
        (0, w!("STATIC"), "Connections:", LABEL, 12, 206, 90, 20),
        (23, w!("EDIT"), &ctx.manager.settings().connections.to_string(), EDIT | ES_NUMBER as u32, 110, 202, 60, 24),
        (0, w!("STATIC"), "Size:", LABEL, 12, 240, 90, 20),
        (24, w!("STATIC"), "", 0, 110, 240, 430, 20),
        (0, w!("STATIC"), "Stream:", LABEL, 12, 274, 90, 20),
        (ID_SOUND, w!("BUTTON"), "Only the sound", BS_AUTOCHECKBOX as u32, 110, 272, 300, 24),
        (ID_CANCEL, w!("BUTTON"), "Cancel", BUTTON, 270, 324, 85, 30),
        (ID_ADD, w!("BUTTON"), "Add", BUTTON, 362, 324, 85, 30),
        (ID_DOWNLOAD, w!("BUTTON"), "Download", DEFAULT, 455, 324, 85, 30),
    ]);
    explain_ids(hwnd, &[
        (ID_LINK, "", "The address of the file to download"),
        (20, "", "Other servers with the same file, one link per line (optional)"),
        (21, "", "The file's name, from the server; change it to save under another name"),
        (ID_CHOOSE, "", "Choose the folder the file is saved to"),
        (23, "", "Connections for this download (1–32)"),
        (ID_SOUND, "", "For a streamed video: save just its sound (.m4a, or .mp3), as the site sends it"),
        (ID_CANCEL, "", "Close without adding anything (Esc)"),
        (ID_ADD, "", "Add: put it in the queue; it starts when the queue runs"),
        (ID_DOWNLOAD, "", "Download: start now (Enter)"),
    ]);
    with_form(hwnd, |f| {
        // Set here as well as on EN_CHANGE: setting the link below notifies while this form is borrowed, so the
        // handler can't see it.
        let stream = link.as_deref().is_some_and(|l| hd_core::media::download::is_stream_link(l.trim()));
        unsafe {
            let _ = EnableWindow(f.controls[&ID_SOUND], stream);
        }
        if let Some(l) = link {
            set_text(f.controls[&ID_LINK], &l);
        }
        unsafe {
            let _ = SetFocus(Some(f.controls[&ID_LINK]));
        }
    });
}

/// Opens Add Links with the links, or adds them to the one already open (§5.3).
pub fn add_links(ctx: &Ctx, links: Vec<String>) {
    if links.is_empty() {
        (ctx.done)(Some("No links found".into()));
        return;
    }
    let open = OPEN_LINKS.with(|o| *o.borrow());
    let hwnd = match open {
        Some(h) if unsafe { IsWindow(Some(h)).as_bool() } => h,
        _ => {
            let h = open_form(ctx, FormKind::AddLinks, "Add Links", (760, 460), &[
                (ID_FILTER, w!("EDIT"), "", EDIT, 12, 12, 520, 26),
                (ID_CHECK_ALL, w!("BUTTON"), "Check All", BUTTON, 540, 11, 100, 28),
                (ID_UNCHECK_ALL, w!("BUTTON"), "Uncheck All", BUTTON, 648, 11, 100, 28),
                (ID_LIST, WC_LISTVIEWW, "", LVS_REPORT | LVS_SHOWSELALWAYS | LVS_NOSORTHEADER, 12, 46, 736, 280),
                (ID_MIRRORS, w!("BUTTON"), "Treat as mirrors of one file", BS_AUTOCHECKBOX as u32, 12, 334, 300, 24),
                (ID_SOUND, w!("BUTTON"), "Only the sound of streamed videos", BS_AUTOCHECKBOX as u32, 320, 334, 320, 24),
                (0, w!("STATIC"), "Save to:", 0, 12, 370, 60, 20),
                (22, w!("STATIC"), &crate::save_folder(&ctx.manager).display().to_string(), 0x4000, 76, 370, 560, 20),
                (ID_CHOOSE, w!("BUTTON"), "Choose…", BUTTON, 648, 364, 100, 28),
                (25, w!("STATIC"), "", 0, 12, 418, 380, 20),
                (ID_CANCEL, w!("BUTTON"), "Cancel", BUTTON, 470, 412, 85, 30),
                (ID_ADD, w!("BUTTON"), "Add", BUTTON, 563, 412, 85, 30),
                (ID_DOWNLOAD, w!("BUTTON"), "Download", DEFAULT, 656, 412, 92, 30),
            ]);
            with_form(h, |f| unsafe {
                let list = f.controls[&ID_LIST];
                SendMessageW(list, LVM_SETEXTENDEDLISTVIEWSTYLE, Some(WPARAM(0)),
                             Some(LPARAM((LVS_EX_CHECKBOXES | LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize)));
                for (i, (name, width)) in [("Name", 300), ("Size", 110), ("Link", 300)].iter().enumerate() {
                    let text = wide(name);
                    let column = LVCOLUMNW { mask: LVCF_TEXT | LVCF_WIDTH, cx: *width, pszText: PWSTR(text.as_ptr() as *mut _), ..Default::default() };
                    SendMessageW(list, LVM_INSERTCOLUMNW, Some(WPARAM(i)), Some(LPARAM(&column as *const _ as isize)));
                }
                SendMessageW(f.controls[&ID_FILTER], EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(w!("Filter").0 as isize)));
                DragAcceptFiles(h, true);
            });
            explain_ids(h, &[
                (ID_FILTER, "Filter", "Show only links whose name or address contains this text"),
                (ID_CHECK_ALL, "", "Tick every link shown"),
                (ID_UNCHECK_ALL, "", "Untick every link shown"),
                (ID_LIST, "Links", "The links found: untick the ones not to download"),
                (ID_MIRRORS, "", "Add the checked links as one download, each a source of the same file"),
                (ID_SOUND, "", "Save just the sound of each checked stream (.m4a, or .mp3), as the site sends it"),
                (ID_CHOOSE, "", "Choose the folder the files are saved to"),
                (ID_CANCEL, "", "Close without adding anything (Esc)"),
                (ID_ADD, "", "Add: put them in the queue; they start when the queue runs"),
                (ID_DOWNLOAD, "", "Download: start now (Enter)"),
            ]);
            OPEN_LINKS.with(|o| *o.borrow_mut() = Some(h));
            h
        }
    };
    let fresh: Vec<String> = with_form(hwnd, |f| {
        let mut fresh = Vec::new();
        for url in links {
            if !f.items.iter().any(|i| i.0 == url) && !fresh.contains(&url) {
                fresh.push(url);
            }
        }
        for url in &fresh {
            let name = url.trim_end_matches('/').rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(url).to_string();
            f.items.push((url.clone(), name, None, "Checking…".into(), true));
            if hd_core::media::download::is_stream_link(url) {
                f.streams.insert(url.clone());
            }
        }
        fresh
    })
    .unwrap_or_default();
    show_items(hwnd);
    probe_in_background(hwnd, ctx.manager.clone(), fresh);
    unsafe {
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Rebuilds Add Links' list from its items, filtered, and updates the title, mirrors option and summary.
fn show_items(hwnd: HWND) {
    with_form(hwnd, |f| unsafe {
        let list = f.controls[&ID_LIST];
        let needle = text_of(f.controls[&ID_FILTER]).to_lowercase();
        f.shown = (0..f.items.len())
            .filter(|&i| needle.is_empty() || f.items[i].0.to_lowercase().contains(&needle) || f.items[i].1.to_lowercase().contains(&needle))
            .collect();
        SendMessageW(list, WM_SETREDRAW, Some(WPARAM(0)), None);
        SendMessageW(list, LVM_DELETEALLITEMS, None, None);
        for (row, &i) in f.shown.iter().enumerate() {
            let (url, name, _, size, checked) = &f.items[i];
            for (col, text) in [name, size, url].iter().enumerate() {
                let text = wide(text);
                let item = LVITEMW { mask: LVIF_TEXT, iItem: row as i32, iSubItem: col as i32, pszText: PWSTR(text.as_ptr() as *mut _), ..Default::default() };
                SendMessageW(list, if col == 0 { LVM_INSERTITEMW } else { LVM_SETITEMW }, None, Some(LPARAM(&item as *const _ as isize)));
            }
            // The checkbox is state image 2 (checked) or 1.
            let state = LVITEMW { stateMask: LVIS_STATEIMAGEMASK, state: LIST_VIEW_ITEM_STATE_FLAGS(((if *checked { 2 } else { 1 }) << 12) as u32), ..Default::default() };
            SendMessageW(list, LVM_SETITEMSTATE, Some(WPARAM(row)), Some(LPARAM(&state as *const _ as isize)));
        }
        SendMessageW(list, WM_SETREDRAW, Some(WPARAM(1)), None);
    });
    update_summary(hwnd);
}

fn update_summary(hwnd: HWND) {
    with_form(hwnd, |f| unsafe {
        let n_all = f.items.len();
        let title = if n_all == 1 { "Add 1 Link".to_string() } else { format!("Add {n_all} Links") };
        set_text(hwnd, &title);
        let checked: Vec<&(String, String, Option<u64>, String, bool)> = f.items.iter().filter(|i| i.4).collect();
        let n = checked.len();
        let names: std::collections::HashSet<&String> = checked.iter().map(|i| &i.1).collect();
        let sizes: std::collections::HashSet<u64> = checked.iter().filter_map(|i| i.2).collect();
        let can = n > 1 && (names.len() == 1 || (sizes.len() == 1 && checked.iter().all(|i| i.2.is_some())));
        let mirrors = f.controls[&ID_MIRRORS];
        let _ = EnableWindow(mirrors, can);
        if !can {
            SendMessageW(mirrors, BM_SETCHECK, Some(WPARAM(0)), None);
        }
        let on = SendMessageW(mirrors, BM_GETCHECK, None, None).0 == 1;
        // Only for streams; a whole video file isn't split.
        let streams = checked.iter().any(|i| f.streams.contains(&i.0));
        let sound = f.controls[&ID_SOUND];
        let _ = EnableWindow(sound, streams);
        if !streams {
            SendMessageW(sound, BM_SETCHECK, Some(WPARAM(0)), None);
        }
        let files: std::collections::HashSet<String> = checked.iter().map(|i| format!("{}\u{0}{:?}", i.1, i.2)).collect();
        let text = if on {
            format!("1 download from {n} sources")
        } else if n == 1 {
            "1 link checked".into()
        } else if files.len() < n {
            format!("{n} links checked: {} download{} with mirrors", files.len(), if files.len() == 1 { "" } else { "s" })
        } else {
            format!("{n} links checked")
        };
        set_text(f.controls[&25], &text);
    });
}

/// Batch Download (§5.5).
pub fn batch(ctx: &Ctx) {
    let hwnd = open_form(ctx, FormKind::Batch, "Batch Download", (560, 160), &[
        (ID_PATTERN, w!("EDIT"), "", EDIT, 12, 14, 536, 26),
        (26, w!("STATIC"), "^(###1-100) counts with 3 digits; H after a range for hexadecimal; a-z for letters.", 0, 12, 50, 536, 40),
        (ID_CANCEL, w!("BUTTON"), "Cancel", BUTTON, 370, 112, 85, 30),
        (ID_NEXT, w!("BUTTON"), "Next", DEFAULT, 463, 112, 85, 30),
    ]);
    explain_ids(hwnd, &[
        (ID_PATTERN, "Link pattern", "^(###1-100) counts with 3 digits; H after a range for hexadecimal; a-z for letters"),
        (ID_CANCEL, "", "Close without adding anything (Esc)"),
        (ID_NEXT, "", "Show these links in Add Links (Enter)"),
    ]);
    with_form(hwnd, |f| unsafe {
        SendMessageW(f.controls[&ID_PATTERN], EM_SETCUEBANNER, Some(WPARAM(1)),
                     Some(LPARAM(w!("https://example.com/img^(###1-100).jpg").0 as isize)));
        let _ = EnableWindow(f.controls[&ID_NEXT], false);
        let _ = SetFocus(Some(f.controls[&ID_PATTERN]));
    });
}

fn submit(hwnd: HWND, start: bool) {
    let Some((ctx, kind)) = with_form(hwnd, |f| (f.ctx.clone(), f.kind)) else { return };
    // New Download with a web page: its files, listed to tick, in Add Links (§15).
    if let Some(url) = with_form(hwnd, |f| (f.kind == FormKind::NewDownload && f.page).then(|| text_of(f.controls[&ID_LINK]).trim().to_string())).flatten() {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        add_links(&ctx, vec![url]);
        return;
    }
    let request = with_form(hwnd, |f| -> Option<(Vec<PendingLink>, PathBuf, u32)> {
        match f.kind {
            FormKind::NewDownload => {
                let url = text_of(f.controls[&ID_LINK]).trim().to_string();
                if hd_core::links::find(&url).is_empty() {
                    set_text(f.controls[&24], "Enter a web link (http, https or ftp)");
                    return None;
                }
                let name = text_of(f.controls[&21]).trim().to_string();
                let connections = text_of(f.controls[&23]).trim().parse::<u32>().unwrap_or(32).clamp(1, 32);
                let sound_only = hd_core::media::download::is_stream_link(&url) && checked(f.controls[&ID_SOUND]);
                Some((
                    vec![PendingLink {
                        url,
                        mirrors: hd_core::links::find(&text_of(f.controls[&20])),
                        name: Some(name).filter(|n| !n.is_empty()),
                        size: f.probed_size,
                        // The download already under way is adopted (moved to the chosen folder and name).
                        early: if sound_only { None } else { f.early.take() },
                        stream: sound_only,
                        choice: sound_only.then(|| hd_core::addflow::sound_only_choice(&f.ctx.manager)),
                        dress: None,
                    }],
                    f.folder.clone(),
                    connections,
                ))
            }
            FormKind::AddLinks => {
                let mut links: Vec<PendingLink> = f
                    .items
                    .iter()
                    .filter(|i| i.4)
                    .map(|i| {
                        let stream = f.streams.contains(&i.0);
                        let sound_only = stream && checked(f.controls[&ID_SOUND]);
                        PendingLink {
                            url: i.0.clone(),
                            name: Some(i.1.clone()),
                            size: i.2,
                            stream,
                            choice: sound_only.then(|| hd_core::addflow::sound_only_choice(&f.ctx.manager)),
                            ..Default::default()
                        }
                    })
                    .collect();
                if links.is_empty() {
                    return None;
                }
                let as_mirrors = unsafe { SendMessageW(f.controls[&ID_MIRRORS], BM_GETCHECK, None, None).0 == 1 };
                if as_mirrors {
                    let first = links.remove(0);
                    let size = first.size.or(links.iter().find_map(|l| l.size));
                    links = vec![PendingLink { mirrors: links.iter().map(|l| l.url.clone()).collect(), size, ..first }];
                }
                Some((links, f.folder.clone(), f.ctx.manager.settings().connections))
            }
            FormKind::Batch => None,
        }
    })
    .flatten();
    if kind == FormKind::Batch {
        let links = with_form(hwnd, |f| f.batch.clone()).unwrap_or_default();
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        add_links(&ctx, links);
        return;
    }
    let Some((links, folder, connections)) = request else { return };
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
    let note = add_all(ctx.owner, &ctx.manager, links, folder, connections, start);
    (ctx.done)(note);
}

extern "system" fn form_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_COMMAND => {
                let id = (wparam.0 & 0xffff) as u16;
                let code = ((wparam.0 >> 16) & 0xffff) as u32;
                match (id, code) {
                    (ID_CANCEL, _) => drop(DestroyWindow(hwnd)),
                    (ID_ADD, _) => submit(hwnd, false),
                    (ID_DOWNLOAD, _) => submit(hwnd, true),
                    (ID_CHOOSE, _) => {
                        if let Some(dir) = choose_folder(hwnd, "Save to") {
                            with_form(hwnd, |f| {
                                set_text(f.controls[&22], &dir.display().to_string());
                                f.folder = dir;
                            });
                        }
                    }
                    (ID_LINK, EN_CHANGE) => {
                        // Sound only is for streams; a whole video file isn't split.
                        with_form(hwnd, |f| {
                            let stream = hd_core::media::download::is_stream_link(text_of(f.controls[&ID_LINK]).trim());
                            let _ = EnableWindow(f.controls[&ID_SOUND], stream);
                            if !stream {
                                SendMessageW(f.controls[&ID_SOUND], BM_SETCHECK, Some(WPARAM(0)), None);
                            }
                        });
                        // A6: ask the server once the link stops changing.
                        drop_early(hwnd);
                        SetTimer(Some(hwnd), TIMER_PROBE, 500, None);
                    }
                    (ID_FILTER, EN_CHANGE) => show_items(hwnd),
                    (ID_MIRRORS, _) => update_summary(hwnd),
                    (ID_SOUND, _) => {}
                    (ID_CHECK_ALL | ID_UNCHECK_ALL, _) => {
                        with_form(hwnd, |f| {
                            let shown = f.shown.clone();
                            for i in shown {
                                f.items[i].4 = id == ID_CHECK_ALL;
                            }
                        });
                        show_items(hwnd);
                    }
                    (ID_PATTERN, EN_CHANGE) => {
                        with_form(hwnd, |f| match hd_core::links::expand_batch(text_of(f.controls[&ID_PATTERN]).trim()) {
                            Ok(l) if !l.is_empty() => {
                                set_text(f.controls[&26], &format!("{} links: {} … {}", l.len(), l[0], l[l.len() - 1]));
                                let _ = EnableWindow(f.controls[&ID_NEXT], true);
                                f.batch = l;
                            }
                            Ok(_) => {}
                            Err(e) => {
                                set_text(f.controls[&26], &e);
                                let _ = EnableWindow(f.controls[&ID_NEXT], false);
                            }
                        });
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_EARLY => {
                with_form(hwnd, |f| {
                    if let Some((got, total)) = f.early.and_then(|t| f.ctx.manager.early_progress(t))
                        && got > 0
                    {
                        let whole = total.is_some_and(|t| got >= t);
                        set_text(f.controls[&24], &if whole {
                            format!("{} — already downloaded", f.size_text)
                        } else {
                            format!("{} — {} downloaded already", f.size_text, format::bytes(got))
                        });
                    }
                });
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_PROBE => {
                let _ = KillTimer(Some(hwnd), TIMER_PROBE);
                if let Some((url, manager, size_label)) = with_form(hwnd, |f| (text_of(f.controls[&ID_LINK]).trim().to_string(), f.ctx.manager.clone(), f.controls[&24])) {
                    if hd_core::links::find(&url).is_empty() {
                        set_text(size_label, "");
                    } else {
                        set_text(size_label, "Checking…");
                        probe_in_background(hwnd, manager, vec![url]);
                    }
                }
                LRESULT(0)
            }
            WM_SCANNED => {
                let answer = Box::from_raw(lparam.0 as *mut Scanned);
                with_form(hwnd, |f| {
                    let Some(at) = f.items.iter().position(|i| i.0 == answer.url) else { return };
                    match &answer.result {
                        Ok(scan) if !scan.found.is_empty() => {
                            let known: Vec<String> = f.items.iter().map(|i| i.0.clone()).collect();
                            let rows: Vec<(String, String, Option<u64>, String, bool)> = scan
                                .found
                                .iter()
                                .filter(|x| !known.contains(&x.url))
                                .map(|x| {
                                    let text = match x.size {
                                        Some(n) => format!("{}, {}", format::bytes(n), x.kind.word()),
                                        None => x.kind.word().to_string(),
                                    };
                                    (x.url.clone(), x.name.clone(), x.size, text, true)
                                })
                                .collect();
                            for x in scan.found.iter().filter(|x| x.kind == hd_core::media::scan::Kind::Stream) {
                                f.streams.insert(x.url.clone());
                            }
                            f.items.splice(at..=at, rows);
                        }
                        Ok(_) => {
                            f.items[at].3 = "Web page: no files found".into();
                            f.items[at].4 = false;
                        }
                        Err(e) => {
                            f.items[at].3 = format!("Web page: {e}");
                            f.items[at].4 = false;
                        }
                    }
                });
                show_items(hwnd);
                LRESULT(0)
            }
            WM_PROBED => {
                let answer = Box::from_raw(lparam.0 as *mut Probed);
                with_form(hwnd, |f| match f.kind {
                    FormKind::NewDownload => {
                        if text_of(f.controls[&ID_LINK]).trim() != answer.url {
                            return;
                        }
                        f.page = false;
                        match &answer.result {
                            Ok(p) if p.page => {
                                f.page = true;
                                f.size_text = "A web page: Download lists the files and media on it".into();
                                set_text(f.controls[&24], &f.size_text);
                            }
                            Ok(p) => {
                                if text_of(f.controls[&21]).is_empty() {
                                    set_text(f.controls[&21], &p.name);
                                }
                                f.probed_size = p.size;
                                let mut text = match (p.size, p.resumable) {
                                    (Some(s), true) => format!("{}, resumable", format::bytes(s)),
                                    (Some(s), false) => format!("{}, can't resume", format::bytes(s)),
                                    (None, _) => "size unknown, can't resume".into(),
                                };
                                // Said now rather than when the download fails (free space, FAT32's 4 GB limit).
                                if let Some(problem) = p.size.and_then(|s| hd_core::disk::space_problem(&f.folder, s)) {
                                    text = format!("{text} — {problem}");
                                } else if p.resumable && f.early.is_none() {
                                    // Fetching starts now, so no time is lost while the folder and name are chosen.
                                    f.early = Some(f.ctx.manager.start_early(&answer.url, &[], &f.folder, Some(&p.name)));
                                    SetTimer(Some(hwnd), TIMER_EARLY, 500, None);
                                }
                                f.size_text = text.clone();
                                set_text(f.controls[&24], &text);
                            }
                            Err(e) => set_text(f.controls[&24], e),
                        }
                    }
                    _ => {
                        // A web page: what's on it takes its place (§15).
                        if answer.result.as_ref().is_ok_and(|p| p.page) {
                            for item in f.items.iter_mut().filter(|i| i.0 == answer.url) {
                                item.3 = "Scanning page…".into();
                            }
                            scan_in_background(hwnd, f.ctx.manager.clone(), answer.url.clone());
                            return;
                        }
                        for item in f.items.iter_mut().filter(|i| i.0 == answer.url) {
                            match &answer.result {
                                Ok(p) => {
                                    item.1 = p.name.clone();
                                    item.2 = p.size;
                                    item.3 = p.size.map(format::bytes).unwrap_or_else(|| "Unknown".into());
                                }
                                Err(e) => item.3 = e.clone(),
                            }
                        }
                    }
                });
                if with_form(hwnd, |f| f.kind == FormKind::AddLinks).unwrap_or(false) {
                    show_items(hwnd);
                }
                LRESULT(0)
            }
            WM_NOTIFY => {
                let header = &*(lparam.0 as *const NMHDR);
                if header.idFrom as u16 == ID_LIST && header.code == LVN_ITEMCHANGED {
                    let change = &*(lparam.0 as *const NMLISTVIEW);
                    if change.uChanged.0 & LVIF_STATE.0 != 0 && (change.uNewState ^ change.uOldState) & LVIS_STATEIMAGEMASK.0 != 0 {
                        let checked = (change.uNewState & LVIS_STATEIMAGEMASK.0) >> 12 == 2;
                        with_form(hwnd, |f| {
                            if let Some(&i) = f.shown.get(change.iItem as usize) {
                                f.items[i].4 = checked;
                            }
                        });
                        update_summary(hwnd);
                    }
                }
                LRESULT(0)
            }
            WM_DROPFILES => {
                // Links in files dropped on Add Links join the list.
                let paths = dropped_files(HDROP(wparam.0 as *mut _));
                if let (Some(links), Some(ctx)) = (links_in_files(&paths), with_form(hwnd, |f| f.ctx.clone())) {
                    add_links(&ctx, links);
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                // Closed without adding: what was fetched is discarded.
                drop_early(hwnd);
                FORMS.with(|forms| forms.borrow_mut().remove(&(hwnd.0 as isize)));
                OPEN_LINKS.with(|o| {
                    if *o.borrow() == Some(hwnd) {
                        *o.borrow_mut() = None;
                    }
                });
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// The paths of files dropped on a window (WM_DROPFILES).
pub fn dropped_files(drop: HDROP) -> Vec<PathBuf> {
    unsafe {
        let n = DragQueryFileW(drop, u32::MAX, None);
        let paths = (0..n)
            .map(|i| {
                let mut buffer = [0u16; 1024];
                let len = DragQueryFileW(drop, i, Some(&mut buffer));
                PathBuf::from(String::from_utf16_lossy(&buffer[..len as usize]))
            })
            .collect();
        DragFinish(drop);
        paths
    }
}

/// Discards New Download's early download, off the UI thread (stopping waits for it to save).
fn drop_early(hwnd: HWND) {
    unsafe {
        let _ = KillTimer(Some(hwnd), TIMER_EARLY);
    }
    if let Some((manager, token)) = with_form(hwnd, |f| f.early.take().map(|t| (f.ctx.manager.clone(), t))).flatten() {
        std::thread::spawn(move || manager.cancel_early(token));
    }
}

/// Whether a checkbox is ticked.
fn checked(button: HWND) -> bool {
    unsafe { SendMessageW(button, BM_GETCHECK, None, None).0 == 1 }
}
