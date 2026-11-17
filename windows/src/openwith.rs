//! Open With ▸ (§4.1): the apps Windows recommends for a file's type, the default one marked, as the Mac's list; and
//! Choose Another App…, Windows' own "Open with" window.

use std::path::Path;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::IDataObject;
use windows::Win32::UI::Shell::*;
use windows::core::{HSTRING, PCWSTR};

pub struct App {
    pub label: String,
    handler: IAssocHandler,
}

/// The recommended apps for this file's type, the default first.
pub fn handlers(path: &Path) -> Vec<App> {
    let Some(ext) = path.extension().map(|e| format!(".{}", e.to_string_lossy())) else { return Vec::new() };
    let ext = HSTRING::from(ext.as_str());
    let mut out = Vec::new();
    unsafe {
        let default = {
            let mut buffer = [0u16; 260];
            let mut n = buffer.len() as u32;
            AssocQueryStringW(ASSOCF_NONE, ASSOCSTR_FRIENDLYAPPNAME, &ext, None, Some(windows::core::PWSTR(buffer.as_mut_ptr())), &mut n)
                .is_ok()
                .then(|| String::from_utf16_lossy(&buffer[..(n as usize).saturating_sub(1)]))
        };
        let Ok(list) = SHAssocEnumHandlers(&ext, ASSOC_FILTER_RECOMMENDED) else { return out };
        loop {
            let mut item: [Option<IAssocHandler>; 1] = [None];
            let mut fetched = 0u32;
            if list.Next(&mut item, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            let Some(handler) = item[0].take() else { break };
            let Ok(name) = handler.GetUIName() else { continue };
            let text = name.to_string().unwrap_or_default();
            windows::Win32::System::Com::CoTaskMemFree(Some(name.0 as *const _));
            let name = text;
            let is_default = default.as_deref() == Some(name.as_str());
            let label = if is_default { format!("{name} (default)") } else { name };
            if is_default { out.insert(0, App { label, handler }) } else { out.push(App { label, handler }) }
            if out.len() >= 30 {
                break;
            }
        }
    }
    out
}

/// Opens the file with one of the apps `handlers` listed.
pub fn open(app: &App, path: &Path) -> Result<(), String> {
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None).map_err(|e| e.message())?;
        let data: IDataObject = item.BindToHandler(None, &BHID_DataObject).map_err(|e| e.message())?;
        app.handler.Invoke(&data).map_err(|e| e.message())
    }
}

/// Windows' own "Open with" window for the file.
pub fn choose(owner: HWND, path: &Path) {
    let file = HSTRING::from(path.as_os_str());
    let info = OPENASINFO { pcszFile: PCWSTR(file.as_ptr()), pcszClass: PCWSTR::null(), oaifInFlags: OAIF_ALLOW_REGISTRATION | OAIF_EXEC };
    unsafe {
        let _ = SHOpenWithDialog(Some(owner), &info);
    }
}
