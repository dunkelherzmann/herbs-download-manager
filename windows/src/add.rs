//! Adding downloads (§5) on Windows: New Download (Ctrl+N), Add Links (several links, from the clipboard, a drop,
//! Import or Batch), Batch Download (Ctrl+Shift+N) and Import Links (Ctrl+Shift+O). Every add goes through the core's
//! `AddSession`; a name in use (§5.6) is asked with a task dialog whose checkbox is "Do the same for the rest".
//! Servers are asked about links on background threads, which post their answers back to the window.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use hd_core::addflow::{AddSession, Choice, PendingLink, Step};
use hd_core::format;
use hd_core::links::{LinkColumn, LinkRow};
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
use windows::core::{PCWSTR, PWSTR, w};

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
const ID_SAVE_LABEL: u16 = 28;
const ID_BATCH_HELP: u16 = 26;
/// Save to, as the Mac's folder pop-up.
const FOLDER_POPUP: u32 = (CBS_DROPDOWNLIST | WS_VSCROLL.0 as i32) as u32;
const ID_BATCH_PREVIEW: u16 = 27;
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
                let mut alert = crate::alert::Alert::new(q.title(), q.body(),
                                                         &["Keep Both", "Skip", "Replace"], 0, Some(1));
                if q.more {
                    alert.checkbox = Some("Do the same for the rest".into());
                }
                let (answer, for_rest) = crate::alert::run(owner, alert);
                let pressed = answer.map(|i| i as i32 + 1).unwrap_or(2);
                let choice = match pressed {
                    1 => Choice::KeepBoth,
                    3 => Choice::Replace,
                    _ => Choice::Skip,
                };
                session.answer(manager, choice, for_rest);
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
    /// Not an answer yet: the page's text had nothing, and its player is being looked at.
    looking: bool,
}

fn scan_in_background(window: HWND, manager: Manager, url: String) {
    let handle = window.0 as isize;
    std::thread::spawn(move || {
        let post = |answer: Scanned| unsafe {
            let _ = PostMessageW(Some(HWND(handle as *mut _)), WM_SCANNED, WPARAM(0), LPARAM(Box::into_raw(Box::new(answer)) as isize));
        };
        let mut result = manager.scan_page(&url);
        if result.as_ref().is_ok_and(|scan| scan.found.is_empty()) {
            // Nothing in its text: its player may build the address only while it runs.
            post(Scanned { url: url.clone(), result: Err(String::new()), looking: true });
            let seen = crate::played::run(&url, &crate::data_folder());
            if !seen.is_empty() {
                let seen: Vec<_> = seen.iter().filter_map(|j| hd_core::media::played::parse(j)).collect();
                let given = hd_core::media::scan::Given { played: hd_core::media::played::picked(&seen), ..Default::default() };
                result = manager.scan_page_given(&url, given);
            }
        }
        post(Scanned { url, result, looking: false });
    });
}

/// Chooses a folder (the shell's folder browser), opened at `start` as the Mac's panel opens at the chosen folder,
/// its message `title` and its OK button `ok`, as the Mac's panel's message and prompt.
pub(crate) fn choose_folder(owner: HWND, title: &str, start: &std::path::Path, ok: &str) -> Option<PathBuf> {
    /// Once the browser is up (BFFM_INITIALIZED): selects the start folder (BFFM_SETSELECTIONW) and names the OK
    /// button (BFFM_SETOKTEXT).
    unsafe extern "system" fn opened(hwnd: HWND, message: u32, _: LPARAM, data: LPARAM) -> i32 {
        if message == 1 && data.0 != 0 {
            let (start, ok) = unsafe { &*(data.0 as *const (Vec<u16>, Vec<u16>)) };
            unsafe {
                SendMessageW(hwnd, WM_USER + 103, Some(WPARAM(1)), Some(LPARAM(start.as_ptr() as isize)));
                SendMessageW(hwnd, WM_USER + 105, Some(WPARAM(0)), Some(LPARAM(ok.as_ptr() as isize)));
            }
        }
        0
    }
    unsafe {
        let title = wide(title);
        let data = (wide(&start.display().to_string()), wide(ok));
        let info = BROWSEINFOW {
            hwndOwner: owner,
            lpszTitle: PCWSTR(title.as_ptr()),
            ulFlags: BIF_RETURNONLYFSDIRS | 0x50 /* BIF_USENEWUI */,
            lpfn: Some(opened),
            lParam: LPARAM(&data as *const _ as isize),
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

/// The clipboard's text, if any, and a copied web page's HTML.
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
        // A copied web page also brings its HTML: its links' addresses count too, as on the Mac.
        let html = RegisterClipboardFormatW(w!("HTML Format"));
        if let Ok(data) = GetClipboardData(html) {
            out.push('\n');
            out.push_str(&crate::dragdrop::html_fragment(&crate::dragdrop::bytes(HGLOBAL(data.0))));
        }
        let _ = CloseClipboard();
        out
    }
}

/// Export Links (§5.8): where to save the links, from the Save dialog, starting at `name`, titled with what goes in.
pub fn export_file(owner: HWND, name: &str, what: &str) -> Option<PathBuf> {
    unsafe {
        let mut buffer = vec![0u16; 32 * 1024];
        for (i, c) in name.encode_utf16().enumerate() {
            buffer[i] = c;
        }
        let filter = wide("Text files (*.txt)\0*.txt\0All files\0*.*\0");
        let what = wide(what);
        let mut ofn = OPENFILENAMEW {
            lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: owner,
            lpstrFilter: PCWSTR(filter.as_ptr()),
            lpstrFile: PWSTR(buffer.as_mut_ptr()),
            nMaxFile: buffer.len() as u32,
            lpstrTitle: PCWSTR(what.as_ptr()),
            lpstrDefExt: w!("txt"),
            Flags: OFN_EXPLORER | OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST,
            ..Default::default()
        };
        if !GetSaveFileNameW(&mut ofn).as_bool() {
            return None;
        }
        let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        Some(PathBuf::from(String::from_utf16_lossy(&buffer[..end])))
    }
}

/// Import Links (§5.7): the links in one or more text files chosen with the Open dialog.
pub fn import_files(owner: HWND) -> Option<Vec<PathBuf>> {
    unsafe {
        let mut buffer = vec![0u16; 32 * 1024];
        // Any file, as on the Mac: the text kinds are there to narrow the list.
        let filter = wide("All files\0*.*\0Text files (*.txt;*.md;*.csv;*.html;*.htm;*.xml;*.json;*.rtf)\0*.txt;*.md;*.csv;*.html;*.htm;*.xml;*.json;*.rtf\0");
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
    /// New Download's name was typed by the user: a new link's name from the server doesn't replace it.
    name_edited: bool,
    shown: Vec<usize>,
    /// Add Links' sort: the column clicked last (0 Name, 1 Size, 2 Folder, 3 Link) and whether A to Z (smallest first).
    sort: Option<(usize, bool)>,
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
        // Add Links resizes, as on the Mac; the other forms keep their size.
        let style = if kind == FormKind::AddLinks { WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MAXIMIZEBOX } else { WS_CAPTION | WS_SYSMENU };
        let mut rect = RECT { left: 0, top: 0, right: px(size.0), bottom: px(size.1) };
        let _ = AdjustWindowRectEx(&mut rect, style, false, WINDOW_EX_STYLE::default());
        let hwnd = CreateWindowExW(
            // Its own taskbar button, as it isn't owned by the main window.
            WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT | WS_EX_APPWINDOW,
            w!("HerbsDMForm"),
            PCWSTR(t.as_ptr()),
            style | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            rect.right - rect.left,
            rect.bottom - rect.top,
            // Not owned by the main window, as the Mac's are independent: they stay when it hides or minimizes.
            None,
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
        // Created visible, so its controls take the light or dark look here, now that they exist.
        crate::theme::apply(hwnd);
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
                name_edited: false,
                shown: Vec::new(),
                sort: None,
                batch: Vec::new(),
            })
        });
        if let Some(popup) = FORMS.with(|forms| forms.borrow().get(&(hwnd.0 as isize)).and_then(|f| f.controls.get(&22).copied())) {
            fill_folder_popup(popup, &crate::save_folder(&ctx.manager));
        }
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
        (22, w!("COMBOBOX"), "", FOLDER_POPUP, 110, 164, 330, 300),
        (ID_CHOOSE, w!("BUTTON"), "Choose…", BUTTON, 450, 162, 90, 28),
        (0, w!("STATIC"), "Connections:", LABEL, 12, 206, 90, 20),
        (23, w!("COMBOBOX"), "", (CBS_DROPDOWNLIST | WS_VSCROLL.0 as i32) as u32, 110, 202, 70, 400),
        (0, w!("STATIC"), "Size:", LABEL, 12, 240, 90, 20),
        (24, w!("STATIC"), "", 0, 110, 240, 430, 20),
        (0, w!("STATIC"), "Stream:", LABEL, 12, 274, 90, 20),
        (ID_SOUND, w!("BUTTON"), "Only the sound", BS_AUTOCHECKBOX as u32, 110, 272, 300, 24),
        (ID_CANCEL, w!("BUTTON"), "Cancel", BUTTON, 270, 324, 85, 30),
        (ID_ADD, w!("BUTTON"), "Add", BUTTON, 362, 324, 85, 30),
        (ID_DOWNLOAD, w!("BUTTON"), "Download", DEFAULT, 455, 324, 85, 30),
    ]);
    explain_ids(hwnd, &[
        (ID_LINK, "", "Link: the address of the file to download"),
        (20, "", "Mirrors: other servers with the same file, one address per line (optional)"),
        (21, "", "Name: the file name to save as"),
        (ID_CHOOSE, "", "Choose the folder to save to"),
        (23, "", "Connections: how many parts of the file download at once (1–32)"),
        (ID_SOUND, "", "For a streamed video: save just its sound (.m4a, or .mp3), as the site sends it"),
        (ID_CANCEL, "", "Close without adding anything (Esc)"),
        (ID_ADD, "", "Add: put it in the queue without starting it"),
        (ID_DOWNLOAD, "", "Download: start it now (Enter)"),
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
        // As on the Mac: hints in the empty fields, and Connections as a list of 1–32.
        unsafe {
            SendMessageW(f.controls[&ID_LINK], EM_SETCUEBANNER, Some(WPARAM(0)), Some(LPARAM(w!("https://example.com/file.zip").0 as isize)));
            SendMessageW(f.controls[&21], EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(w!("From the server").0 as isize)));
            let connections = f.ctx.manager.settings().connections.clamp(1, 32);
            for n in 1..=32 {
                let t = wide(&n.to_string());
                SendMessageW(f.controls[&23], CB_ADDSTRING, None, Some(LPARAM(t.as_ptr() as isize)));
            }
            SendMessageW(f.controls[&23], CB_SETCURSEL, Some(WPARAM(connections as usize - 1)), None);
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
                (ID_LIST, WC_LISTVIEWW, "", LVS_REPORT | LVS_SHOWSELALWAYS, 12, 46, 736, 280),
                (ID_MIRRORS, w!("BUTTON"), "Treat as mirrors of one file", BS_AUTOCHECKBOX as u32, 12, 334, 300, 24),
                (ID_SOUND, w!("BUTTON"), "Only the sound of streamed videos", BS_AUTOCHECKBOX as u32, 320, 334, 320, 24),
                (ID_SAVE_LABEL, w!("STATIC"), "Save to:", 0, 12, 370, 60, 20),
                (22, w!("COMBOBOX"), "", FOLDER_POPUP, 76, 366, 560, 300),
                (ID_CHOOSE, w!("BUTTON"), "Choose…", BUTTON, 648, 364, 100, 28),
                (25, w!("STATIC"), "", 0x0100 /* SS_NOTIFY, for its hover text */, 12, 418, 380, 20),
                (ID_CANCEL, w!("BUTTON"), "Cancel", BUTTON, 470, 412, 85, 30),
                (ID_ADD, w!("BUTTON"), "Add", BUTTON, 563, 412, 85, 30),
                (ID_DOWNLOAD, w!("BUTTON"), "Download", DEFAULT, 656, 412, 92, 30),
            ]);
            with_form(h, |f| unsafe {
                let list = f.controls[&ID_LIST];
                SendMessageW(list, LVM_SETEXTENDEDLISTVIEWSTYLE, Some(WPARAM(0)),
                             Some(LPARAM((LVS_EX_CHECKBOXES | LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize)));
                // The Mac's columns: Name, Size, Folder (where it's saved) and Link.
                for (i, (name, width)) in [("Name", 260), ("Size", 110), ("Folder", 140), ("Link", 260)].iter().enumerate() {
                    let text = wide(name);
                    let column = LVCOLUMNW { mask: LVCF_TEXT | LVCF_WIDTH, cx: *width, pszText: PWSTR(text.as_ptr() as *mut _), ..Default::default() };
                    SendMessageW(list, LVM_INSERTCOLUMNW, Some(WPARAM(i)), Some(LPARAM(&column as *const _ as isize)));
                }
                SendMessageW(f.controls[&ID_FILTER], EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(w!("Filter").0 as isize)));
                // Each cell shows its text as its hover text, as on the Mac.
                crate::a11y::cell_tips(list);
            });
            // Links dropped on the window while it's open join the list; while dragging, it dims and says so.
            crate::dragdrop::register(h, false, "Drop links to add them");
            explain_ids(h, &[
                (ID_FILTER, "Filter", "Filter: show links whose name or address contains the text"),
                (ID_CHECK_ALL, "", "Tick every link shown"),
                (ID_UNCHECK_ALL, "", "Untick every link shown"),
                (ID_LIST, "Links", "The links found: untick the ones not to download"),
                (ID_MIRRORS, "", "Combine the checked links into one download that uses each of them as a source"),
                (ID_SOUND, "", "Save just the sound of each checked stream (.m4a, or .mp3), as the site sends it"),
                (ID_CHOOSE, "", "Choose the folder the files are saved to"),
                (ID_CANCEL, "", "Close without adding anything (Esc)"),
                (ID_ADD, "", "Add: put them in the queue without starting them"),
                (ID_DOWNLOAD, "", "Download: start them now (Enter)"),
                (25, "", "Links to the same file (same name and size) are added as one download and its mirrors"),
            ]);
            OPEN_LINKS.with(|o| *o.borrow_mut() = Some(h));
            // Test hook: `--sort-links name|size|link [desc]` sorts as clicks on that header do.
            let args: Vec<String> = std::env::args().collect();
            if let Some(i) = args.iter().position(|a| a == "--sort-links") {
                let column = ["name", "size", "folder", "link"].iter().position(|c| args.get(i + 1).is_some_and(|a| a == c)).unwrap_or(0);
                with_form(h, |f| f.sort = Some((column, args.get(i + 2).is_none_or(|a| a != "desc"))));
            }
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
            f.items.push((url.clone(), link_name(url), None, "Checking…".into(), true));
            if hd_core::media::download::is_stream_link(url) {
                f.streams.insert(url.clone());
            }
        }
        fresh
    })
    .unwrap_or_default();
    // Every link already listed: a beep, as on the Mac (MessageBeep is in user32, outside the enabled features).
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBeep(kind: u32) -> i32;
    }
    if fresh.is_empty() {
        unsafe {
            MessageBeep(0);
        }
    }
    show_items(hwnd);
    probe_in_background(hwnd, ctx.manager.clone(), fresh);
    unsafe {
        let _ = SetForegroundWindow(hwnd);
    }
}

/// A link's name before the server answers, as the Mac's: the last part of its path, percent-decoded, without the
/// query; the whole link when its path has none.
fn link_name(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let path = rest.split(['?', '#']).next().unwrap_or("");
    let path = path.find('/').map_or("", |i| &path[i..]);
    let last = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    if last.is_empty() {
        return url.to_string();
    }
    let bytes = last.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(h), Some(l)) = (bytes.get(i + 1).and_then(|&b| hex(b)), bytes.get(i + 2).and_then(|&b| hex(b)))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| url.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_link_s_name_is_its_last_path_part_decoded_without_the_query() {
        assert_eq!(super::link_name("https://example.com/files/My%20Movie.mkv?token=abc#t=1"), "My Movie.mkv");
        assert_eq!(super::link_name("https://example.com/files/dir/"), "dir");
        assert_eq!(super::link_name("https://example.com/"), "https://example.com/");
        assert_eq!(super::link_name("https://example.com?x=1"), "https://example.com?x=1");
        assert_eq!(super::link_name("https://example.com/caf%C3%A9.zip"), "café.zip");
    }
}

/// Rebuilds Add Links' list from its items, filtered, and updates the title, mirrors option and summary.
fn show_items(hwnd: HWND) {
    with_form(hwnd, |f| unsafe {
        let list = f.controls[&ID_LIST];
        // In the order of the header clicked last (the core's `links::sort_order`, sizes by bytes), which is also the
        // order they are added in.
        if let Some((column, ascending)) = f.sort {
            let column = [LinkColumn::Name, LinkColumn::Size, LinkColumn::Folder, LinkColumn::Link][column];
            let folder = f.folder.display().to_string();
            let rows: Vec<LinkRow> = f.items.iter().map(|i| LinkRow { name: &i.1, size: i.2, folder: &folder, url: &i.0 }).collect();
            let order = hd_core::links::sort_order(&rows, column, ascending);
            let mut slots: Vec<Option<_>> = f.items.drain(..).map(Some).collect();
            f.items = order.into_iter().filter_map(|i| slots[i].take()).collect();
        }
        let needle = text_of(f.controls[&ID_FILTER]).to_lowercase();
        f.shown = (0..f.items.len())
            .filter(|&i| needle.is_empty() || f.items[i].0.to_lowercase().contains(&needle) || f.items[i].1.to_lowercase().contains(&needle))
            .collect();
        // The header's sort arrow.
        let header = HWND(SendMessageW(list, LVM_GETHEADER, None, None).0 as *mut _);
        for c in 0..4 {
            let mut item = HDITEMW { mask: HDI_FORMAT, ..Default::default() };
            SendMessageW(header, HDM_GETITEMW, Some(WPARAM(c)), Some(LPARAM(&mut item as *mut _ as isize)));
            item.fmt.0 &= !(HDF_SORTUP.0 | HDF_SORTDOWN.0);
            if let Some((column, ascending)) = f.sort
                && column == c
            {
                item.fmt.0 |= if ascending { HDF_SORTUP.0 } else { HDF_SORTDOWN.0 };
            }
            SendMessageW(header, HDM_SETITEMW, Some(WPARAM(c)), Some(LPARAM(&item as *const _ as isize)));
        }
        SendMessageW(list, WM_SETREDRAW, Some(WPARAM(0)), None);
        SendMessageW(list, LVM_DELETEALLITEMS, None, None);
        for (row, &i) in f.shown.iter().enumerate() {
            let (url, name, _, size, checked) = &f.items[i];
            let folder = f.folder.display().to_string();
            for (col, text) in [name, size, &folder, url].iter().enumerate() {
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

/// Batch Download (§5.5), as the Mac's: the link, the help on ranges, and a preview of the count with the first and
/// last links (a mistake in red).
pub fn batch(ctx: &Ctx) {
    let hwnd = open_form(ctx, FormKind::Batch, "Batch Download", (600, 212), &[
        (ID_PATTERN, w!("EDIT"), "", EDIT, 20, 20, 560, 26),
        (ID_BATCH_HELP, w!("STATIC"), BATCH_HELP, 0, 20, 56, 560, 32),
        (ID_BATCH_PREVIEW, w!("STATIC"), "", 0x80 /* SS_NOPREFIX */, 20, 98, 560, 50),
        (ID_CANCEL, w!("BUTTON"), "Cancel", BUTTON, 402, 162, 85, 30),
        (ID_NEXT, w!("BUTTON"), "Continue…", DEFAULT, 495, 162, 85, 30),
    ]);
    explain_ids(hwnd, &[
        (ID_PATTERN, "Batch link", "Batch link: a link with a range placeholder"),
        (ID_CANCEL, "", "Close without adding anything (Esc)"),
        (ID_NEXT, "", "Show the links in the Add Links window (Enter)"),
    ]);
    with_form(hwnd, |f| unsafe {
        SendMessageW(f.controls[&ID_PATTERN], EM_SETCUEBANNER, Some(WPARAM(1)),
                     Some(LPARAM(w!("https://example.com/img^(###1-100).jpg").0 as isize)));
        let dpi = GetDpiForWindow(hwnd);
        SendMessageW(f.controls[&ID_BATCH_HELP], WM_SETFONT, Some(WPARAM(crate::theme::fonts(dpi).small.0 as usize)), Some(LPARAM(1)));
        crate::theme::set_tone(f.controls[&ID_BATCH_HELP], crate::theme::Tone::Secondary);
        SendMessageW(f.controls[&ID_BATCH_PREVIEW], WM_SETFONT, Some(WPARAM(crate::theme::mono(dpi).0 as usize)), Some(LPARAM(1)));
        crate::theme::set_tone(f.controls[&ID_BATCH_PREVIEW], crate::theme::Tone::Secondary);
        let _ = EnableWindow(f.controls[&ID_NEXT], false);
        let _ = SetFocus(Some(f.controls[&ID_PATTERN]));
    });
}

/// The Batch window's help on ranges (the Mac's words).
const BATCH_HELP: &str = "Put a range where the number or letters change: ^(###1-100) pads to 3 digits, ^(0-ffH) counts in \
                          hexadecimal, ^(a-z) or ^(AA-BZ) in letters. Separate several ranges with commas.";

/// Batch's preview for a pattern: the count with the first and last links, or what is wrong (true), or nothing.
fn batch_preview(pattern: &str) -> (String, bool, Vec<String>) {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return (String::new(), false, Vec::new());
    }
    match hd_core::links::expand_batch(pattern) {
        Ok(l) if l.len() == 1 => (format!("1 link: {}", l[0]), false, l),
        Ok(l) if !l.is_empty() => (format!("{} links\r\nFirst: {}\r\nLast:  {}", l.len(), l[0], l[l.len() - 1]), false, l),
        Ok(_) => (String::new(), false, Vec::new()),
        Err(e) => (e, true, Vec::new()),
    }
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
        if let Some(r) = crate::theme::dialog(hwnd, msg, wparam, lparam) {
            return r;
        }
        match msg {
            WM_COMMAND => {
                let id = (wparam.0 & 0xffff) as u16;
                let code = ((wparam.0 >> 16) & 0xffff) as u32;
                match (id, code) {
                    (ID_CANCEL, _) => drop(DestroyWindow(hwnd)),
                    (ID_ADD, _) => submit(hwnd, false),
                    (ID_DOWNLOAD, _) => submit(hwnd, true),
                    (ID_CHOOSE, _) => {
                        let start = with_form(hwnd, |f| f.folder.clone()).unwrap_or_default();
                        if let Some(dir) = choose_folder(hwnd, "Save to", &start, "Choose") {
                            let links = with_form(hwnd, |f| {
                                fill_folder_popup(f.controls[&22], &dir);
                                f.folder = dir;
                                f.kind == FormKind::AddLinks
                            });
                            // The Folder column shows it.
                            if links == Some(true) {
                                show_items(hwnd);
                            }
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
                    (21, EN_CHANGE) => drop(with_form(hwnd, |f| f.name_edited = !text_of(f.controls[&21]).is_empty())),
                    (ID_FILTER, EN_CHANGE) => show_items(hwnd),
                    // Save to's pop-up: the folder, or one of the folders above it.
                    (22, CBN_SELCHANGE) => {
                        let links = with_form(hwnd, |f| {
                            let popup = f.controls[&22];
                            let at = SendMessageW(popup, CB_GETCURSEL, None, None).0;
                            if let Some(dir) = f.folder.ancestors().nth(at.max(0) as usize).map(PathBuf::from) {
                                fill_folder_popup(popup, &dir);
                                f.folder = dir;
                            }
                            f.kind == FormKind::AddLinks
                        });
                        if links == Some(true) {
                            show_items(hwnd);
                        }
                    }
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
                        with_form(hwnd, |f| {
                            let (text, mistake, links) = batch_preview(&text_of(f.controls[&ID_PATTERN]));
                            let preview = f.controls[&ID_BATCH_PREVIEW];
                            set_text(preview, &text);
                            crate::theme::set_tone(preview, if mistake { crate::theme::Tone::Red } else { crate::theme::Tone::Secondary });
                            let _ = EnableWindow(f.controls[&ID_NEXT], !links.is_empty());
                            f.batch = links;
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
                    if answer.looking {
                        f.items[at].3 = "Looking at the page's player…".into();
                        return;
                    }
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
                                // Its notification finds the form borrowed, so the name still counts as not typed.
                                if !f.name_edited {
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
                                // Offline, or behind a sign-in: said so, and left out unless ticked again.
                                Err(e) => {
                                    item.3 = format::offline_text(e);
                                    item.4 = false;
                                }
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
                if header.idFrom as u16 == ID_LIST && header.code == LVN_COLUMNCLICK {
                    // A header click sorts by its column, again for the reverse order.
                    let column = (*(lparam.0 as *const NMLISTVIEW)).iSubItem as usize;
                    with_form(hwnd, |f| f.sort = Some((column, !matches!(f.sort, Some((c, true)) if c == column))));
                    show_items(hwnd);
                }
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
            crate::dragdrop::WM_DROPPED => {
                // Links dropped on Add Links join the list.
                let dropped = Box::from_raw(lparam.0 as *mut crate::dragdrop::Dropped);
                if let Some(ctx) = with_form(hwnd, |f| f.ctx.clone()) {
                    add_links(&ctx, dropped.links);
                }
                LRESULT(0)
            }
            WM_SIZE => {
                layout_links(hwnd);
                LRESULT(0)
            }
            WM_GETMINMAXINFO => {
                // Add Links: no smaller than the Mac's 560 × 320.
                if (GetWindowLongW(hwnd, GWL_STYLE) as u32 & WS_THICKFRAME.0) == 0 {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                }
                let info = &mut *(lparam.0 as *mut MINMAXINFO);
                let scale = GetDpiForWindow(hwnd).max(96) as i32;
                let mut rect = RECT { left: 0, top: 0, right: 560 * scale / 96, bottom: 320 * scale / 96 };
                let _ = AdjustWindowRectEx(&mut rect, WS_CAPTION | WS_SYSMENU | WS_THICKFRAME, false, WINDOW_EX_STYLE::default());
                info.ptMinTrackSize = POINT { x: rect.right - rect.left, y: rect.bottom - rect.top };
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                crate::dragdrop::revoke(hwnd);
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

/// Save to's pop-up, as the Mac's: the folder, then each folder above it, by name.
fn fill_folder_popup(popup: HWND, folder: &std::path::Path) {
    unsafe {
        SendMessageW(popup, CB_RESETCONTENT, None, None);
        for dir in folder.ancestors() {
            let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dir.display().to_string());
            let text = wide(&name);
            SendMessageW(popup, CB_ADDSTRING, None, Some(LPARAM(text.as_ptr() as isize)));
        }
        SendMessageW(popup, CB_SETCURSEL, Some(WPARAM(0)), None);
        crate::a11y::explain(popup, "Save to", &format!("{}: the folder the files are saved to; pick a folder above it here, or Choose… another", folder.display()));
    }
}

/// Add Links' controls placed for its size (it resizes, as on the Mac): the list takes the room, the rows under it
/// keep to the bottom and the buttons to the right. In 96-dpi pixels, as made in `add_links`.
fn layout_links(hwnd: HWND) {
    let Some(controls) = with_form(hwnd, |f| (f.kind == FormKind::AddLinks).then(|| f.controls.clone())).flatten() else { return };
    unsafe {
        let scale = GetDpiForWindow(hwnd).max(96) as i32;
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let (w, h) = (client.right * 96 / scale, client.bottom * 96 / scale);
        let px = |v: i32| v * scale / 96;
        let place = |id: u16, x: i32, y: i32, cw: i32, ch: i32| {
            if let Some(&c) = controls.get(&id) {
                let _ = MoveWindow(c, px(x), px(y), px(cw), px(ch), true);
            }
        };
        place(ID_FILTER, 12, 12, w - 252, 26);
        place(ID_CHECK_ALL, w - 220, 11, 100, 28);
        place(ID_UNCHECK_ALL, w - 112, 11, 100, 28);
        place(ID_LIST, 12, 46, w - 24, h - 180);
        place(ID_MIRRORS, 12, h - 126, 300, 24);
        place(ID_SOUND, 320, h - 126, 320, 24);
        place(ID_SAVE_LABEL, 12, h - 90, 60, 20);
        place(22, 76, h - 94, w - 200, 300);
        place(ID_CHOOSE, w - 112, h - 96, 100, 28);
        place(25, 12, h - 42, w - 380, 20);
        place(ID_CANCEL, w - 290, h - 48, 85, 30);
        place(ID_ADD, w - 197, h - 48, 85, 30);
        place(ID_DOWNLOAD, w - 104, h - 48, 92, 30);
        let _ = RedrawWindow(Some(hwnd), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN);
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
