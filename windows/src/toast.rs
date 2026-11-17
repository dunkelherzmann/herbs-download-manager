//! "Download finished" as a Windows toast with **Open** and **Show in Folder** buttons, as the Mac's notification
//! has (SPECIFICATION.md §9 S9); clicking the toast itself shows the file in its folder. The app isn't packaged, so it
//! names itself to Windows under its own AppUserModelID (the one the installer's Start menu shortcut carries): a key
//! under HKCU\Software\Classes\AppUserModelId with its name and icon. The toast's clicks come back on a Windows
//! thread and are posted to the main window as `WM_TOAST`.

use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{ToastActivatedEventArgs, ToastNotification, ToastNotificationManager};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegSetValueExW};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};
use windows_core::{HSTRING, IInspectable, Interface, PCWSTR};

/// Posted to the main window: wparam is the download's id, lparam 1 to open the file, 2 to show it in its folder.
pub const WM_TOAST: u32 = WM_APP + 24;
pub const OPEN: isize = 1;
pub const SHOW: isize = 2;

const APP_ID: &str = "dunkelherzmann.HerbsDownloadManager";

thread_local! {
    /// The toasts shown, kept so their click handlers stay alive (the last few).
    static SHOWN: std::cell::RefCell<Vec<ToastNotification>> = const { std::cell::RefCell::new(Vec::new()) };
    static REGISTERED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The app's name and icon for Windows to show on its toasts (written once a run; the icon goes in the data folder).
fn register() {
    if REGISTERED.get() {
        return;
    }
    REGISTERED.set(true);
    let icon = crate::data_folder().join("notification-icon.png");
    let _ = std::fs::create_dir_all(crate::data_folder());
    if std::fs::read(&icon).ok().as_deref() != Some(include_bytes!("../icons/app-128.png")) {
        let _ = std::fs::write(&icon, include_bytes!("../icons/app-128.png"));
    }
    unsafe {
        let path = wide(&format!("Software\\Classes\\AppUserModelId\\{APP_ID}"));
        let mut key = HKEY::default();
        if RegCreateKeyExW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), None, None, REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None).is_err() {
            return;
        }
        for (name, value) in [("DisplayName", "Herb's Download Manager".to_string()), ("IconUri", icon.display().to_string())] {
            let name = wide(name);
            let value = wide(&value);
            let bytes = std::slice::from_raw_parts(value.as_ptr() as *const u8, value.len() * 2);
            let _ = RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes));
        }
        let _ = RegCloseKey(key);
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// Shows "Download finished" for a download; false when Windows wouldn't show a toast (the caller falls back to the
/// notification area's balloon).
pub fn finished(window: HWND, id: u64, name: &str) -> bool {
    register();
    let xml = format!(
        "<toast launch=\"show:{id}\"><visual><binding template=\"ToastGeneric\"><text>Download finished</text><text>{}</text>\
         </binding></visual><actions><action content=\"Open\" arguments=\"open:{id}\"/>\
         <action content=\"Show in Folder\" arguments=\"show:{id}\"/></actions></toast>",
        escape(name)
    );
    let show = || -> windows_core::Result<()> {
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(xml.as_str()))?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;
        let target = window.0 as isize;
        toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(move |_, args| {
            let arguments = args.ok()?.cast::<ToastActivatedEventArgs>()?.Arguments()?.to_string();
            if let Some((action, id)) = arguments.split_once(':')
                && let Ok(id) = id.parse::<usize>()
            {
                let what = if action == "open" { OPEN } else { SHOW };
                unsafe {
                    let _ = PostMessageW(Some(HWND(target as *mut _)), WM_TOAST, WPARAM(id), LPARAM(what));
                }
            }
            Ok(())
        }))?;
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))?.Show(&toast)?;
        SHOWN.with(|s| {
            let mut s = s.borrow_mut();
            s.push(toast);
            let extra = s.len().saturating_sub(20);
            s.drain(..extra);
        });
        Ok(())
    };
    show().is_ok()
}
