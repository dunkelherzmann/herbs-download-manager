//! Links and files dragged onto the main window (SPECIFICATION.md §5.4, §7.1 M4): a link from a browser (text) or
//! files holding links. Dropped on a download in the list, they become its mirrors; anywhere else, they open Add
//! Links. An OLE drop target, since WM_DROPFILES only ever brings files.

use windows::Win32::Foundation::{HWND, LPARAM, POINTL, WPARAM};
use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, IDataObject, TYMED_HGLOBAL};
use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{
    CF_HDROP, CF_UNICODETEXT, DROPEFFECT, DROPEFFECT_COPY, IDropTarget, IDropTarget_Impl, OleInitialize, RegisterDragDrop, ReleaseStgMedium,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};
use windows::core::{Ref, Result, implement};

/// Posted to the main window with a `Dropped`.
pub const WM_DROPPED: u32 = WM_APP + 23;

/// What was dropped: its text (links found in files are listed), and where (screen pixels).
pub struct Dropped {
    pub text: String,
    pub x: i32,
    pub y: i32,
}

#[implement(IDropTarget)]
struct Target {
    window: isize,
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, _: Ref<'_, IDataObject>, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe { *effect = DROPEFFECT_COPY };
        Ok(())
    }

    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe { *effect = DROPEFFECT_COPY };
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        Ok(())
    }

    fn Drop(&self, data: Ref<'_, IDataObject>, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let text = data.ok().map(text_of).unwrap_or_default();
        unsafe { *effect = DROPEFFECT_COPY };
        let dropped = Box::new(Dropped { text, x: pt.x, y: pt.y });
        unsafe {
            let _ = PostMessageW(Some(HWND(self.window as *mut _)), WM_DROPPED, WPARAM(0), LPARAM(Box::into_raw(dropped) as isize));
        }
        Ok(())
    }
}

/// The dropped data's text: files' links, or the text itself (a browser's link).
fn text_of(data: &IDataObject) -> String {
    let format = |cf: u16| FORMATETC { cfFormat: cf, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
    unsafe {
        if let Ok(mut medium) = data.GetData(&format(CF_HDROP.0)) {
            let drop = HDROP(medium.u.hGlobal.0);
            let n = DragQueryFileW(drop, u32::MAX, None);
            let paths: Vec<std::path::PathBuf> = (0..n)
                .map(|i| {
                    let mut buffer = [0u16; 1024];
                    let len = DragQueryFileW(drop, i, Some(&mut buffer));
                    std::path::PathBuf::from(String::from_utf16_lossy(&buffer[..len as usize]))
                })
                .collect();
            ReleaseStgMedium(&mut medium);
            return crate::add::links_in_files(&paths).unwrap_or_default().join("\n");
        }
        if let Ok(mut medium) = data.GetData(&format(CF_UNICODETEXT.0)) {
            let p = GlobalLock(medium.u.hGlobal) as *const u16;
            let mut text = String::new();
            if !p.is_null() {
                let mut n = 0;
                while *p.add(n) != 0 {
                    n += 1;
                }
                text = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
                let _ = GlobalUnlock(medium.u.hGlobal);
            }
            ReleaseStgMedium(&mut medium);
            return text;
        }
    }
    String::new()
}

/// Makes the main window a drop target (OLE, on its UI thread).
pub fn register(window: HWND) {
    unsafe {
        let _ = OleInitialize(None);
        let target: IDropTarget = Target { window: window.0 as isize }.into();
        let _ = RegisterDragDrop(window, &target);
    }
}
