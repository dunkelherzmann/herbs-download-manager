//! The main window (§2): the Main Menu (☰) in the top-left corner and the title, the sidebar filters with their
//! counts, the download list (a report-style ListView) and the footer. The list refreshes twice a second, as on the
//! Mac. `--capture FILE` saves a BMP of the window (drawn with WM_PRINT, which works over SSH where PrintWindow
//! returns black) and quits.

use std::cell::RefCell;

use hd_core::format::{self, Filter};
use hd_core::manager::{Manager, Status};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

const ID_MENU: usize = 100;
const ID_SIDEBAR: usize = 101;
const ID_QUIT: usize = 200;
const ID_SETTINGS: usize = 201;
const ID_ABOUT: usize = 202;
const TIMER_REFRESH: usize = 1;
const TIMER_CAPTURE: usize = 2;

struct State {
    manager: Manager,
    menu_button: HWND,
    title: HWND,
    sidebar: HWND,
    list: HWND,
    footer: HWND,
    filter: Filter,
    capture: Option<std::path::PathBuf>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn run() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = InitCommonControlsEx(&INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_LISTVIEW_CLASSES | ICC_STANDARD_CLASSES,
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
        let child = |class: PCWSTR, text: PCWSTR, style: WINDOW_STYLE, ex: WINDOW_EX_STYLE, id: usize| {
            CreateWindowExW(ex, class, text, WS_CHILD | WS_VISIBLE | style, 0, 0, 10, 10, Some(hwnd),
                            Some(HMENU(id as *mut _)), Some(instance.into()), None)
                .unwrap_or_default()
        };
        let menu_button = child(w!("BUTTON"), w!("☰"), WINDOW_STYLE(BS_PUSHBUTTON as u32), Default::default(), ID_MENU);
        let title = child(w!("STATIC"), w!("Herb's Download Manager"), WINDOW_STYLE(0x200 /* SS_CENTERIMAGE */), Default::default(), 0);
        let sidebar = child(w!("LISTBOX"), w!(""), WINDOW_STYLE((LBS_NOTIFY | LBS_NOINTEGRALHEIGHT | LBS_USETABSTOPS) as u32), Default::default(), ID_SIDEBAR);
        let list = child(WC_LISTVIEWW, w!(""), WINDOW_STYLE(LVS_REPORT | LVS_SHOWSELALWAYS), WS_EX_CLIENTEDGE, 0);
        let footer = child(w!("STATIC"), w!(""), WINDOW_STYLE(0x200 /* SS_CENTERIMAGE */), Default::default(), 0);

        // The system's message font (Segoe UI) on every control.
        let mut metrics = NONCLIENTMETRICSW { cbSize: std::mem::size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
        let _ = SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, metrics.cbSize, Some(&mut metrics as *mut _ as *mut _),
                                      SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
        let font = CreateFontIndirectW(&metrics.lfMessageFont);
        for h in [menu_button, title, sidebar, list, footer] {
            SendMessageW(h, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
        }
        SendMessageW(list, LVM_SETEXTENDEDLISTVIEWSTYLE, Some(WPARAM(0)),
                     Some(LPARAM((LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize)));
        for (i, (name, width)) in [("Name", 320), ("Status", 140), ("Size", 160), ("Speed", 100), ("Mirrors", 80)].iter().enumerate() {
            let text = wide(name);
            let column = LVCOLUMNW {
                mask: LVCF_TEXT | LVCF_WIDTH,
                cx: *width,
                pszText: windows::core::PWSTR(text.as_ptr() as *mut _),
                ..Default::default()
            };
            SendMessageW(list, LVM_INSERTCOLUMNW, Some(WPARAM(i)), Some(LPARAM(&column as *const _ as isize)));
        }
        for f in Filter::ALL {
            let text = wide(f.title());
            SendMessageW(sidebar, LB_ADDSTRING, Some(WPARAM(0)), Some(LPARAM(text.as_ptr() as isize)));
        }
        // The counts line up at the sidebar's right edge (one tab stop, in dialog units).
        let stop: i32 = 100;
        SendMessageW(sidebar, LB_SETTABSTOPS, Some(WPARAM(1)), Some(LPARAM(&stop as *const i32 as isize)));
        SendMessageW(sidebar, LB_SETCURSEL, Some(WPARAM(0)), Some(LPARAM(0)));
        let _ = SetWindowTextW(menu_button, w!("☰"));

        STATE.with(|s| {
            *s.borrow_mut() =
                Some(State { manager, menu_button, title, sidebar, list, footer, filter: Filter::All, capture: capture.clone() })
        });
        layout(hwnd);
        refresh();
        SetTimer(Some(hwnd), TIMER_REFRESH, 500, None);
        if capture.is_some() {
            SetTimer(Some(hwnd), TIMER_CAPTURE, 1500, None);
        }
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Places the controls: a 40-pixel bar with ☰ and the title, the sidebar and list below it, the footer at the bottom
/// (scaled for the window's DPI).
fn layout(hwnd: HWND) {
    unsafe {
        let scale = GetDpiForWindow(hwnd).max(96) as i32;
        let px = |v: i32| v * scale / 96;
        let mut rect = RECT::default();
        let _ = GetClientRect(hwnd, &mut rect);
        let (w, h) = (rect.right, rect.bottom);
        STATE.with(|s| {
            if let Some(s) = s.borrow().as_ref() {
                let (bar, foot, side) = (px(40), px(24), px(190));
                let _ = MoveWindow(s.menu_button, px(6), px(6), px(32), px(28), true);
                let _ = MoveWindow(s.title, px(44), 0, w - px(88), bar, true);
                let _ = MoveWindow(s.sidebar, 0, bar, side, h - bar - foot, true);
                let _ = MoveWindow(s.list, side, bar, w - side, h - bar - foot, true);
                let _ = MoveWindow(s.footer, px(12), h - foot, w - px(24), foot, true);
            }
        });
    }
}

/// Shows the list's rows, the sidebar's counts and the footer.
fn refresh() {
    STATE.with(|s| {
        let s = s.borrow();
        let Some(s) = s.as_ref() else { return };
        let rows = s.manager.rows();
        unsafe {
            // The sidebar's counts, keeping the selection.
            let selected = SendMessageW(s.sidebar, LB_GETCURSEL, None, None).0;
            for (i, f) in Filter::ALL.iter().enumerate() {
                let n = rows.iter().filter(|r| f.matches(&r.status)).count();
                let text = wide(&format!("{}\t{n}", f.title()));
                SendMessageW(s.sidebar, LB_DELETESTRING, Some(WPARAM(i)), None);
                SendMessageW(s.sidebar, LB_INSERTSTRING, Some(WPARAM(i)), Some(LPARAM(text.as_ptr() as isize)));
            }
            SendMessageW(s.sidebar, LB_SETCURSEL, Some(WPARAM(selected.max(0) as usize)), None);

            SendMessageW(s.list, WM_SETREDRAW, Some(WPARAM(0)), None);
            SendMessageW(s.list, LVM_DELETEALLITEMS, None, None);
            for (i, r) in rows.iter().filter(|r| s.filter.matches(&r.status)).enumerate() {
                let cells = [
                    r.name.clone(),
                    r.status_text.clone(),
                    format::size_text(r.downloaded, r.size, r.status == Status::Finished),
                    format::speed_text(r.speed),
                    format!("{} / {}", r.sources_in_use, r.sources_known),
                ];
                for (col, text) in cells.iter().enumerate() {
                    let text = wide(text);
                    let item = LVITEMW {
                        mask: LVIF_TEXT,
                        iItem: i as i32,
                        iSubItem: col as i32,
                        pszText: windows::core::PWSTR(text.as_ptr() as *mut _),
                        ..Default::default()
                    };
                    let message = if col == 0 { LVM_INSERTITEMW } else { LVM_SETITEMW };
                    SendMessageW(s.list, message, None, Some(LPARAM(&item as *const _ as isize)));
                }
            }
            SendMessageW(s.list, WM_SETREDRAW, Some(WPARAM(1)), None);

            let active = rows.iter().filter(|r| r.status.is_running()).count();
            let selected_rows = SendMessageW(s.list, LVM_GETSELECTEDCOUNT, None, None).0;
            let mut parts = vec![format!("{selected_rows} selected"), format!("{active} active")];
            let speed: u64 = rows.iter().map(|r| r.speed).sum();
            if speed > 0 {
                parts.push(format::speed_text(speed));
            }
            parts.push(if s.manager.queue_running() { "Queue running".into() } else { "Queue stopped".into() });
            let text = wide(&parts.join("  ·  "));
            let _ = SetWindowTextW(s.footer, PCWSTR(text.as_ptr()));
        }
    });
}

/// The Main Menu (§4): one submenu per menu, then Settings, About and Quit, as on the Mac.
unsafe fn show_main_menu(hwnd: HWND, button: HWND) {
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        for title in ["File", "Edit", "Tasks", "View", "Window", "Help"] {
            if let Ok(sub) = CreatePopupMenu() {
                let text = wide(title);
                let _ = AppendMenuW(menu, MF_POPUP | MF_STRING, sub.0 as usize, PCWSTR(text.as_ptr()));
            }
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, ID_SETTINGS, w!("Settings…\tCtrl+,"));
        let _ = AppendMenuW(menu, MF_STRING, ID_ABOUT, w!("About Herb's Download Manager"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, w!("Quit Herb's DM\tCtrl+Q"));
        let mut rect = RECT::default();
        let _ = GetWindowRect(button, &mut rect);
        let _ = TrackPopupMenu(menu, TPM_LEFTALIGN | TPM_TOPALIGN, rect.left, rect.bottom, None, hwnd, None);
        let _ = DestroyMenu(menu);
    }
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
                let path = STATE.with(|s| s.borrow().as_ref().and_then(|s| s.capture.clone()));
                if let Some(path) = path
                    && let Err(e) = capture(hwnd, &path)
                {
                    eprintln!("capture failed: {e}");
                }
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_COMMAND => {
                let id = wparam.0 & 0xffff;
                let code = (wparam.0 >> 16) & 0xffff;
                match id {
                    ID_MENU => {
                        let button = STATE.with(|s| s.borrow().as_ref().map(|s| s.menu_button));
                        if let Some(button) = button {
                            show_main_menu(hwnd, button);
                        }
                    }
                    ID_SIDEBAR if code == LBN_SELCHANGE as usize => {
                        STATE.with(|s| {
                            if let Some(s) = s.borrow_mut().as_mut() {
                                let i = SendMessageW(s.sidebar, LB_GETCURSEL, None, None).0;
                                s.filter = Filter::ALL[i.clamp(0, 4) as usize];
                            }
                        });
                        refresh();
                    }
                    ID_QUIT => {
                        let _ = DestroyWindow(hwnd);
                    }
                    _ => {}
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
