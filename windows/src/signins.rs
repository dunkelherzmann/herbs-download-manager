//! Links that need a login (enhancements.md), as on the Mac: asking for a site's user name and password when a
//! download meets a server that wants one, the Administrator Password that encrypts saved sign-ins (as in Herb's
//! Terminal), and the Sign-ins window (Settings ▸ Sign-ins ▸ Manage…).
//!
//! The questions are synchronous: a small form with its own message loop, or a Task Dialog, so each flow reads top
//! to bottom like the Mac's.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;

use hd_core::vault::{Listed, SignIn, State};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

use crate::add::{BUTTON, Ctx, DEFAULT, EDIT, LABEL, set_text, text_of, wide};

thread_local! {
    /// Off while the feature tests run: the scenario gives sign-ins itself.
    pub static ASKS: Cell<bool> = const { Cell::new(true) };
    static ASKING: Cell<bool> = const { Cell::new(false) };
    /// Downloads already asked about, so a cancelled prompt isn't shown again until the download runs again.
    static ASKED: RefCell<HashSet<u64>> = RefCell::new(HashSet::new());
    static FORM_DONE: Cell<Option<bool>> = const { Cell::new(None) };
    static WINDOW: RefCell<Option<(HWND, Ctx)>> = const { RefCell::new(None) };
}

// --- A small modal form ----------------------------------------------------------------------------------------------

const F_OK: u16 = 1; // IDOK: Enter
const F_CANCEL: u16 = 2; // IDCANCEL: Esc
const F_FIELD: u16 = 100;
const F_CHECK: u16 = 200;
const ES_PASSWORD: u32 = 0x20;

pub struct Field<'a> {
    pub label: &'a str,
    pub value: &'a str,
    pub secret: bool,
    pub help: &'a str,
}

extern "system" fn form_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_COMMAND => {
                match (wparam.0 & 0xffff) as u16 {
                    F_OK => FORM_DONE.set(Some(true)),
                    F_CANCEL => FORM_DONE.set(Some(false)),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                FORM_DONE.set(Some(false));
                LRESULT(0)
            }
            WM_CTLCOLORSTATIC => {
                SetBkMode(HDC(wparam.0 as *mut _), TRANSPARENT);
                LRESULT(GetSysColorBrush(COLOR_BTNFACE).0 as isize)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Shows a form over `owner` and waits: the fields' texts and the checkbox, or `None` when cancelled.
pub fn form(ctx: &Ctx, owner: HWND, title: &str, note: &str, fields: &[Field], check: Option<(&str, bool, &str)>, ok: &str)
    -> Option<(Vec<String>, bool)> {
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class = WNDCLASSW {
            lpfnWndProc: Some(form_proc),
            hInstance: instance.into(),
            lpszClassName: w!("HerbsDMSignInForm"),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            ..Default::default()
        };
        RegisterClassW(&class);
        let scale = GetDpiForWindow(ctx.owner).max(96) as i32;
        let px = |v: i32| v * scale / 96;
        let width = 460;
        // About 62 characters a line at this width.
        let note_lines = note.split('\n').map(|l| (l.chars().count() as i32 / 62) + 1).sum::<i32>().max(1);
        let note_h = note_lines * 17 + 4;
        let mut y = 14 + note_h + 8;
        let fields_top = y;
        y += fields.len() as i32 * 32;
        let check_y = y;
        if check.is_some() {
            y += 30;
        }
        let height = y + 50;
        let mut rect = RECT { left: 0, top: 0, right: px(width), bottom: px(height) };
        let _ = AdjustWindowRectEx(&mut rect, WS_CAPTION | WS_SYSMENU, false, WS_EX_DLGMODALFRAME);
        let mut owner_rect = RECT::default();
        let _ = GetWindowRect(owner, &mut owner_rect);
        let (w_, h_) = (rect.right - rect.left, rect.bottom - rect.top);
        let x = owner_rect.left + ((owner_rect.right - owner_rect.left) - w_) / 2;
        let top = owner_rect.top + ((owner_rect.bottom - owner_rect.top) - h_) / 3;
        let t = wide(title);
        let hwnd = CreateWindowExW(WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT, w!("HerbsDMSignInForm"), PCWSTR(t.as_ptr()),
                                   WS_CAPTION | WS_SYSMENU, x, top.max(0), w_, h_, Some(owner), None, Some(instance.into()), None)
            .unwrap_or_default();
        let make = |id: u16, class: PCWSTR, text: &str, style: u32, x: i32, y: i32, w: i32, h: i32| {
            let text = wide(text);
            let ex = if class == w!("EDIT") { WS_EX_CLIENTEDGE } else { WINDOW_EX_STYLE::default() };
            let tab = if class == w!("STATIC") { WINDOW_STYLE(0) } else { WS_TABSTOP };
            let c = CreateWindowExW(ex, class, PCWSTR(text.as_ptr()), WS_CHILD | WS_VISIBLE | tab | WINDOW_STYLE(style),
                                    px(x), px(y), px(w), px(h), Some(hwnd), Some(HMENU(id as usize as *mut _)), Some(instance.into()), None)
                .unwrap_or_default();
            SendMessageW(c, WM_SETFONT, Some(WPARAM(ctx.font.0 as usize)), Some(LPARAM(1)));
            c
        };
        make(0, w!("STATIC"), note, 0, 14, 14, width - 28, note_h);
        let mut edits = Vec::new();
        for (i, f) in fields.iter().enumerate() {
            let fy = fields_top + i as i32 * 32;
            make(0, w!("STATIC"), &format!("{}:", f.label), LABEL, 14, fy + 3, 150, 20);
            let e = make(F_FIELD + i as u16, w!("EDIT"), f.value, EDIT | if f.secret { ES_PASSWORD } else { 0 }, 170, fy, width - 184, 24);
            crate::a11y::explain(e, f.label, f.help);
            edits.push(e);
        }
        let check_box = check.map(|(label, on, help)| {
            let c = make(F_CHECK, w!("BUTTON"), label, BS_AUTOCHECKBOX as u32, 170, check_y, width - 184, 24);
            SendMessageW(c, BM_SETCHECK, Some(WPARAM(on as usize)), None);
            crate::a11y::tip(c, help);
            c
        });
        let cancel = make(F_CANCEL, w!("BUTTON"), "Cancel", BUTTON, width - 214, height - 42, 96, 30);
        let okb = make(F_OK, w!("BUTTON"), ok, DEFAULT, width - 110, height - 42, 96, 30);
        crate::a11y::tip(cancel, "Close without saving");
        crate::a11y::tip(okb, ok);

        FORM_DONE.set(None);
        let _ = EnableWindow(owner, false);
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        let _ = SetFocus(edits.iter().find(|&&e| text_of(e).is_empty()).or(edits.first()).copied());
        let mut msg = MSG::default();
        while FORM_DONE.get().is_none() {
            if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
                PostQuitMessage(msg.wParam.0 as i32);
                FORM_DONE.set(Some(false));
                break;
            }
            if !IsDialogMessageW(hwnd, &msg).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let values: Vec<String> = edits.iter().map(|&e| text_of(e)).collect();
        let checked = check_box.is_some_and(|c| SendMessageW(c, BM_GETCHECK, None, None).0 == 1);
        let _ = EnableWindow(owner, true);
        let _ = DestroyWindow(hwnd);
        let _ = SetForegroundWindow(owner);
        FORM_DONE.take().filter(|&ok| ok).map(|_| (values, checked))
    }
}

/// A Task Dialog with these buttons (the first is the default); returns the pressed button's id (2 when cancelled).
fn choose(owner: HWND, heading: &str, body: &str, buttons: &[(i32, &str)], default: i32) -> i32 {
    let heading = wide(heading);
    let body = wide(body);
    let labels: Vec<Vec<u16>> = buttons.iter().map(|b| wide(b.1)).collect();
    let list: Vec<TASKDIALOG_BUTTON> =
        buttons.iter().zip(&labels).map(|(b, l)| TASKDIALOG_BUTTON { nButtonID: b.0, pszButtonText: PCWSTR(l.as_ptr()) }).collect();
    let config = TASKDIALOGCONFIG {
        cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: owner,
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION,
        pszWindowTitle: w!("Herb's Download Manager"),
        pszMainInstruction: PCWSTR(heading.as_ptr()),
        pszContent: PCWSTR(body.as_ptr()),
        cButtons: list.len() as u32,
        pButtons: list.as_ptr(),
        nDefaultButton: default,
        ..Default::default()
    };
    let mut pressed = 2i32;
    unsafe {
        let _ = TaskDialogIndirect(&config, Some(&mut pressed), None, None);
    }
    pressed
}

fn tell(owner: HWND, text: &str) {
    choose(owner, text, "", &[(1, "OK")], 1);
}

// --- The flows --------------------------------------------------------------------------------------------------------

/// After each refresh of the main window: a download that failed on a server asking for a user name and password
/// gets a prompt.
pub fn ask_if_needed(ctx: &Ctx) {
    let rows = ctx.manager.rows();
    let waiting: HashSet<u64> = rows.iter().filter(|r| r.needs_login.is_some()).map(|r| r.id).collect();
    ASKED.with(|a| a.borrow_mut().retain(|id| waiting.contains(id)));
    if !ASKS.get() || ASKING.get() {
        return;
    }
    let Some(row) = rows.iter().find(|r| r.needs_login.is_some() && !ASKED.with(|a| a.borrow().contains(&r.id))) else { return };
    ASKED.with(|a| a.borrow_mut().insert(row.id));
    ASKING.set(true);
    ask_for_login(ctx, ctx.owner, row.needs_login.as_deref().unwrap_or(""));
    ASKING.set(false);
}

fn resume(ctx: &Ctx, site: &str) {
    ctx.manager.resume(&ctx.manager.waiting_for_login(site));
    (ctx.done)(None);
}

/// With a saved sign-in that's locked, asks for the Administrator Password; otherwise for the site's user name and
/// password. Then starts the waiting downloads again.
pub fn ask_for_login(ctx: &Ctx, owner: HWND, host: &str) {
    let vault = ctx.manager.vault();
    let site = hd_core::vault::site_of(host);
    let saved = vault.list().into_iter().find(|s| hd_core::vault::covers(&s.site, host));
    if let Some(s) = &saved
        && s.password.is_none()
    {
        let detail = format!("A download from {host} needs your saved sign-in. Enter the Administrator Password to use it.");
        if unlock(ctx, owner, Some(&detail)) {
            resume(ctx, &s.site);
        }
        return;
    }
    let note = match &saved {
        None => format!("{host} asks for a user name and password. Sign in to download the file."),
        Some(s) => format!("{host} asks for a user name and password. The saved sign-in for {} wasn't accepted: sign in again.", s.site),
    };
    let user_value = saved.as_ref().map(|s| s.user.clone()).unwrap_or_default();
    let remember = format!("Remember for {site}");
    let help = format!("Save this sign-in in the app (encrypted with the Administrator Password) for later downloads from {site}");
    let Some((values, keep)) = form(ctx, owner, "Sign In", &note, &[
        Field { label: "User name", value: &user_value, secret: false, help: "The user name for this site" },
        Field { label: "Password", value: "", secret: true, help: "The password for this site" },
    ], Some((&remember, true, &help)), "Sign In") else { return };
    let sign = SignIn {
        site: saved.as_ref().map(|s| s.site.clone()).unwrap_or(site),
        user: values[0].clone(),
        password: values[1].clone(),
        cookies: saved.as_ref().and_then(|s| s.cookies.clone()).unwrap_or_default(),
        referer: saved.as_ref().map(|s| s.referer.clone()).unwrap_or_default(),
    };
    if keep && ready_to_save(ctx, owner) {
        if let Err(e) = vault.save_sign_in(sign.clone()) {
            tell(owner, &e);
            vault.remember_for_session(sign.clone());
        }
    } else {
        vault.remember_for_session(sign.clone());
    }
    resume(ctx, &sign.site);
}

/// Before saving: a protection is chosen, and encrypted sign-ins are unlocked.
pub fn ready_to_save(ctx: &Ctx, owner: HWND) -> bool {
    match ctx.manager.vault().state() {
        State::NotChosen => explain_setup(ctx, owner),
        State::Locked => unlock(ctx, owner, None),
        State::Plain | State::Unlocked => true,
    }
}

/// The first-use explanation; true once a protection has been chosen.
pub fn explain_setup(ctx: &Ctx, owner: HWND) -> bool {
    let pressed = choose(
        owner,
        "Create an Administrator Password",
        "Herb's Download Manager keeps saved sign-ins inside the app. To encrypt them, create an Administrator Password; \
         you'll enter it once each time you open the app, when a download first needs a saved sign-in.\n\n\
         Without one, saved sign-ins (passwords and cookies included) are stored as plain text, which is a security \
         risk: anyone or anything that can read your files could read them.",
        &[(101, "Create Administrator Password…"), (102, "Store as Plain Text"), (2, "Not Now")],
        101,
    );
    match pressed {
        101 => create_or_modify(ctx, owner),
        102 => {
            // The risky choice is never one click away.
            let sure = choose(
                owner,
                "Store saved sign-ins as plain text?",
                "Passwords and cookies you save will be readable by anyone or anything that can read your files. You can \
                 create an Administrator Password later in Settings ▸ Sign-ins.",
                &[(2, "Cancel"), (102, "Store as Plain Text")],
                2,
            );
            sure == 102 && ctx.manager.vault().use_plain_text().is_ok()
        }
        _ => false,
    }
}

/// Creates the Administrator Password, or changes it after checking the current one (blank removes it).
pub fn create_or_modify(ctx: &Ctx, owner: HWND) -> bool {
    let vault = ctx.manager.vault();
    let modifying = matches!(vault.state(), State::Locked | State::Unlocked);
    let mut message: Option<String> = None;
    loop {
        let info = if modifying {
            "Saved sign-ins are encrypted again with the new password. Leave both new password fields blank to remove the \
             Administrator Password: saved sign-ins are then stored as plain text, unprotected."
        } else {
            "Saved sign-ins will be encrypted with this password. It can't be recovered if you forget it; you would have \
             to delete your sign-ins and enter them again."
        };
        let note = match &message {
            Some(m) => format!("{m}\n{info}"),
            None => info.to_string(),
        };
        let mut fields = Vec::new();
        if modifying {
            fields.push(Field { label: "Current password", value: "", secret: true, help: "The Administrator Password you use now" });
        }
        fields.push(Field {
            label: if modifying { "New password" } else { "New password (8 or more)" },
            value: "",
            secret: true,
            help: "The new Administrator Password, at least 8 characters",
        });
        fields.push(Field { label: "Confirm new password", value: "", secret: true, help: "The new Administrator Password again" });
        let title = if modifying { "Change Administrator Password" } else { "Create Administrator Password" };
        let Some((v, _)) = form(ctx, owner, title, &note, &fields, None, if modifying { "Change Password" } else { "Create Password" }) else {
            return false;
        };
        let (old, new, again) = if modifying { (v[0].clone(), v[1].clone(), v[2].clone()) } else { (String::new(), v[0].clone(), v[1].clone()) };
        if new != again {
            message = Some("The new passwords don't match.".into());
            continue;
        }
        if modifying {
            if !vault.unlock(&old) {
                message = Some("The current password is wrong.".into());
                continue;
            }
            if new.is_empty()
                && choose(owner, "Remove the Administrator Password?",
                          "Saved sign-ins, passwords and cookies included, will be stored as plain text: anyone or anything that \
                           can read your files could read them.",
                          &[(2, "Cancel"), (103, "Remove Password")], 2) != 103
            {
                continue;
            }
        }
        match if modifying { vault.change_password(&new) } else { vault.create_password(&new) } {
            Ok(()) => return true,
            Err(e) => message = Some(e),
        }
    }
}

/// Asks for the Administrator Password to unlock saved sign-ins for this session.
pub fn unlock(ctx: &Ctx, owner: HWND, detail: Option<&str>) -> bool {
    let vault = ctx.manager.vault();
    let mut note = detail.unwrap_or("Enter the Administrator Password to use your saved sign-ins.").to_string();
    while vault.state() == State::Locked {
        let Some((v, _)) = form(ctx, owner, "Unlock Saved Sign-ins", &note, &[Field {
            label: "Administrator Password",
            value: "",
            secret: true,
            help: "The Administrator Password that encrypts saved sign-ins",
        }], None, "Unlock") else {
            return false;
        };
        if !vault.unlock(&v[0]) {
            note = "That isn't the Administrator Password.".into();
        }
    }
    true
}

// --- The Sign-ins window -----------------------------------------------------------------------------------------------

const W_LIST: u16 = 10;
const W_ADD: u16 = 11;
const W_EDIT: u16 = 12;
const W_DELETE: u16 = 13;
const W_LOCK: u16 = 14;
const W_ADMIN: u16 = 15;
const W_STATUS: u16 = 16;
const W_CLOSE: u16 = 2; // IDCANCEL: Esc closes

fn control(hwnd: HWND, id: u16) -> HWND {
    unsafe { GetDlgItem(Some(hwnd), id as i32).unwrap_or_default() }
}

/// Settings' "3 saved, encrypted" line.
pub fn summary(ctx: &Ctx) -> String {
    let vault = ctx.manager.vault();
    let count = vault.list().len();
    let saved = if count == 0 { "None saved".to_string() } else { format!("{count} saved") };
    let how = match vault.state() {
        State::NotChosen => None,
        State::Plain => (count > 0).then_some("as plain text"),
        State::Locked => Some("encrypted, locked"),
        State::Unlocked => Some("encrypted"),
    };
    match how {
        Some(h) => format!("{saved}, {h}"),
        None => saved,
    }
}

/// Opens the Sign-ins window (or brings it forward).
pub fn open(ctx: &Ctx) {
    if let Some(h) = WINDOW.with(|w| w.borrow().as_ref().map(|(h, _)| *h))
        && unsafe { IsWindow(Some(h)).as_bool() }
    {
        unsafe {
            let _ = SetForegroundWindow(h);
        }
        return;
    }
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("HerbsDMSignIns"),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            ..Default::default()
        };
        RegisterClassW(&class);
        let scale = GetDpiForWindow(ctx.owner).max(96) as i32;
        let px = |v: i32| v * scale / 96;
        let mut rect = RECT { left: 0, top: 0, right: px(660), bottom: px(380) };
        let _ = AdjustWindowRectEx(&mut rect, WS_CAPTION | WS_SYSMENU, false, WINDOW_EX_STYLE::default());
        let hwnd = CreateWindowExW(WS_EX_CONTROLPARENT, w!("HerbsDMSignIns"), w!("Sign-ins"), WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
                                   CW_USEDEFAULT, CW_USEDEFAULT, rect.right - rect.left, rect.bottom - rect.top, Some(ctx.owner), None,
                                   Some(instance.into()), None)
            .unwrap_or_default();
        let specs: [(u16, PCWSTR, &str, u32, i32, i32, i32, i32); 8] = [
            (W_ADD, w!("BUTTON"), "Add…", BUTTON, 14, 12, 80, 28),
            (W_EDIT, w!("BUTTON"), "Edit…", BUTTON, 100, 12, 80, 28),
            (W_DELETE, w!("BUTTON"), "Delete", BUTTON, 186, 12, 80, 28),
            (W_LOCK, w!("BUTTON"), "Unlock…", BUTTON, 300, 12, 100, 28),
            (W_ADMIN, w!("BUTTON"), "Create Administrator Password…", BUTTON, 406, 12, 240, 28),
            (W_LIST, WC_LISTVIEWW, "", (LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS) as u32, 14, 50, 632, 230),
            (W_STATUS, w!("STATIC"), "", 0, 14, 288, 632, 40),
            (W_CLOSE, w!("BUTTON"), "Close", DEFAULT, 561, 338, 85, 30),
        ];
        for (id, class, text, style, x, y, w_, h) in specs {
            let text = wide(text);
            let ex = if class == WC_LISTVIEWW { WS_EX_CLIENTEDGE } else { WINDOW_EX_STYLE::default() };
            let tab = if id == W_STATUS { WINDOW_STYLE(0) } else { WS_TABSTOP };
            let c = CreateWindowExW(ex, class, PCWSTR(text.as_ptr()), WS_CHILD | WS_VISIBLE | tab | WINDOW_STYLE(style),
                                    px(x), px(y), px(w_), px(h), Some(hwnd), Some(HMENU(id as usize as *mut _)), Some(instance.into()), None)
                .unwrap_or_default();
            SendMessageW(c, WM_SETFONT, Some(WPARAM(ctx.font.0 as usize)), Some(LPARAM(1)));
        }
        crate::detail::columns(control(hwnd, W_LIST), &[("Site", px(170)), ("User name", px(140)), ("Password", px(90)),
                                                         ("Cookies and referer", px(130)), ("Stored", px(90))], false);
        for (id, name, help) in [
            (W_ADD, "", "Add a site's user name, password, cookies or referer"),
            (W_EDIT, "", "Change the selected sign-in"),
            (W_DELETE, "", "Delete the selected sign-in from the app"),
            (W_LOCK, "", "Unlock saved sign-ins with the Administrator Password, or lock them again"),
            (W_ADMIN, "", "Create, change or remove the Administrator Password that encrypts saved sign-ins"),
            (W_LIST, "Saved sign-ins", "Every site's saved sign-in; double-click one to edit it"),
            (W_CLOSE, "", "Close Sign-ins; changes are already saved"),
        ] {
            let c = control(hwnd, id);
            if name.is_empty() { crate::a11y::tip(c, help) } else { crate::a11y::explain(c, name, help) }
        }
        WINDOW.with(|w| *w.borrow_mut() = Some((hwnd, ctx.clone())));
        reload(hwnd);
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
    }
}

fn ctx_of() -> Option<Ctx> {
    WINDOW.with(|w| w.borrow().as_ref().map(|(_, c)| c.clone()))
}

fn rows_of(ctx: &Ctx) -> Vec<Listed> {
    ctx.manager.vault().list()
}

fn selected(hwnd: HWND) -> Option<usize> {
    let i = unsafe { SendMessageW(control(hwnd, W_LIST), LVM_GETNEXTITEM, Some(WPARAM(usize::MAX)), Some(LPARAM(LVNI_SELECTED as isize))).0 };
    (i >= 0).then_some(i as usize)
}

fn reload(hwnd: HWND) {
    let Some(ctx) = ctx_of() else { return };
    let rows = rows_of(&ctx);
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            let extras: Vec<&str> = [
                r.cookies.as_deref().is_some_and(|c| !c.is_empty()).then_some("Cookies"),
                (!r.referer.is_empty()).then_some("Referer"),
            ]
            .into_iter()
            .flatten()
            .collect();
            vec![
                r.site.clone(),
                r.user.clone(),
                match &r.password {
                    None => "Locked".into(),
                    Some(p) if p.is_empty() => String::new(),
                    Some(_) => "••••••••".into(),
                },
                extras.join(", "),
                if r.encrypted { "Encrypted".into() } else { "Plain text".into() },
            ]
        })
        .collect();
    let list = control(hwnd, W_LIST);
    unsafe {
        SendMessageW(list, LVM_DELETEALLITEMS, None, None);
    }
    crate::detail::fill(list, &cells);
    let state = ctx.manager.vault().state();
    set_text(control(hwnd, W_LOCK), if state == State::Unlocked { "Lock" } else { "Unlock…" });
    set_text(control(hwnd, W_ADMIN), if matches!(state, State::Locked | State::Unlocked) {
        "Change Administrator Password…"
    } else {
        "Create Administrator Password…"
    });
    set_text(control(hwnd, W_STATUS), match state {
        State::NotChosen => "Sign-ins are asked for when a download needs one. Create an Administrator Password to encrypt the ones you save.",
        State::Plain => "Saved sign-ins are stored as plain text. Create an Administrator Password to encrypt them.",
        State::Locked => "Saved sign-ins are encrypted and locked. Unlock them to see or change passwords.",
        State::Unlocked => "Saved sign-ins are encrypted with the Administrator Password and unlocked until the app quits.",
    });
    unsafe {
        let _ = EnableWindow(control(hwnd, W_LOCK), matches!(state, State::Locked | State::Unlocked));
    }
    enable(hwnd);
    crate::detail::show_sign_ins_summary(&summary(&ctx));
}

fn enable(hwnd: HWND) {
    let Some(ctx) = ctx_of() else { return };
    let rows = rows_of(&ctx);
    let one = selected(hwnd).and_then(|i| rows.get(i));
    unsafe {
        let _ = EnableWindow(control(hwnd, W_EDIT), one.is_some());
        let _ = EnableWindow(control(hwnd, W_DELETE), one.is_some());
    }
}

/// Add or Edit: site, user name, password, cookies and referer.
fn edit_form(hwnd: HWND, ctx: &Ctx, existing: Option<Listed>) {
    if !ready_to_save(ctx, hwnd) {
        return reload(hwnd);
    }
    // Reread: unlocking fills in the secrets.
    let existing = existing.and_then(|e| rows_of(ctx).into_iter().find(|r| r.site == e.site));
    let mut note = "Downloads from the site and its subdomains send these. Leave out what the site doesn't need.".to_string();
    let e = existing.clone();
    let mut values: Vec<String> = vec![
        e.as_ref().map(|x| x.site.clone()).unwrap_or_default(),
        e.as_ref().map(|x| x.user.clone()).unwrap_or_default(),
        e.as_ref().and_then(|x| x.password.clone()).unwrap_or_default(),
        e.as_ref().and_then(|x| x.cookies.clone()).unwrap_or_default(),
        e.as_ref().map(|x| x.referer.clone()).unwrap_or_default(),
    ];
    let title = match &existing {
        None => "Add a Sign-in".to_string(),
        Some(x) => format!("Edit the Sign-in for {}", x.site),
    };
    loop {
        let Some((v, _)) = form(ctx, hwnd, &title, &note, &[
            Field { label: "Site", value: &values[0], secret: false, help: "The site these apply to, such as example.com; its subdomains too" },
            Field { label: "User name", value: &values[1], secret: false, help: "The user name for this site" },
            Field { label: "Password", value: &values[2], secret: true, help: "The password for this site" },
            Field { label: "Cookies", value: &values[3], secret: false,
                    help: "The Cookie header downloads from this site send, as name=value pairs separated by semicolons" },
            Field { label: "Referer", value: &values[4], secret: false, help: "For sites that only allow downloads coming from one of their pages" },
        ], None, "Save") else {
            return;
        };
        values = v;
        let vault = ctx.manager.vault();
        let new_site = hd_core::vault::site_of(&values[0]);
        if let Some(old) = &existing
            && old.site != new_site
        {
            let _ = vault.delete(&old.site);
        }
        let sign = SignIn {
            site: values[0].clone(),
            user: values[1].clone(),
            password: values[2].clone(),
            cookies: values[3].clone(),
            referer: values[4].clone(),
        };
        match vault.save_sign_in(sign) {
            Ok(()) => {
                resume(ctx, &new_site);
                return reload(hwnd);
            }
            Err(m) => note = m,
        }
    }
}

extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_CTLCOLORSTATIC => {
                SetBkMode(HDC(wparam.0 as *mut _), TRANSPARENT);
                LRESULT(GetSysColorBrush(COLOR_BTNFACE).0 as isize)
            }
            WM_NOTIFY => {
                let header = &*(lparam.0 as *const NMHDR);
                if header.idFrom as u16 == W_LIST {
                    match header.code {
                        LVN_ITEMCHANGED => enable(hwnd),
                        NM_DBLCLK => {
                            let _ = PostMessageW(Some(hwnd), WM_COMMAND, WPARAM(W_EDIT as usize), LPARAM(0));
                        }
                        _ => {}
                    }
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                let Some(ctx) = ctx_of() else { return LRESULT(0) };
                let rows = rows_of(&ctx);
                let picked = selected(hwnd).and_then(|i| rows.get(i).cloned());
                match (wparam.0 & 0xffff) as u16 {
                    W_CLOSE => drop(DestroyWindow(hwnd)),
                    W_ADD => edit_form(hwnd, &ctx, None),
                    W_EDIT => {
                        if let Some(r) = picked {
                            edit_form(hwnd, &ctx, Some(r));
                        }
                    }
                    W_DELETE => {
                        if let Some(r) = picked
                            && choose(hwnd, &format!("Delete the sign-in for {}?", r.site),
                                      "Its user name, password, cookies and referer are removed from the app. This can't be undone.",
                                      &[(2, "Cancel"), (104, "Delete")], 2) == 104
                        {
                            let _ = ctx.manager.vault().delete(&r.site);
                            reload(hwnd);
                        }
                    }
                    W_LOCK => {
                        if ctx.manager.vault().state() == State::Unlocked {
                            ctx.manager.vault().lock();
                        } else if unlock(&ctx, hwnd, None) {
                            resume(&ctx, "");
                        }
                        reload(hwnd);
                    }
                    W_ADMIN => {
                        if ctx.manager.vault().state() == State::NotChosen {
                            explain_setup(&ctx, hwnd);
                        } else {
                            create_or_modify(&ctx, hwnd);
                        }
                        reload(hwnd);
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                WINDOW.with(|w| *w.borrow_mut() = None);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
