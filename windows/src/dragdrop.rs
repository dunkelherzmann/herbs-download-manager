//! Links and files dragged onto a window (SPECIFICATION.md §2, §5.3, §5.4, §7.1 M4): a link from a browser, a web
//! page's text or HTML, or files holding links. While links are dragged over it, the window dims and says "Drop links
//! here" (Add Links: "Drop links to add them"), and the copy cursor shows; with no links in what is dragged, neither
//! does. Over an unfinished download in the main list, the row is framed instead, and the links become its mirrors.
//! An OLE drop target, since WM_DROPFILES only ever brings files.

use std::cell::{Cell, RefCell};

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, POINT, POINTL, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, IDataObject, TYMED_HGLOBAL};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{
    CF_HDROP, CF_UNICODETEXT, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl, OleInitialize, RegisterDragDrop,
    ReleaseStgMedium, RevokeDragDrop,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, Ref, Result, implement, w};

/// Posted to the window with a `Dropped`.
pub const WM_DROPPED: u32 = WM_APP + 23;

/// What was dropped: the links in it, and where (screen pixels).
pub struct Dropped {
    pub links: Vec<String>,
    pub x: i32,
    pub y: i32,
}

#[implement(IDropTarget)]
struct Target {
    window: isize,
    /// The main window: an unfinished download under the pointer takes the links as mirrors.
    rows: bool,
    message: &'static str,
    links: RefCell<Vec<String>>,
    overlay: Cell<isize>,
}

impl Target {
    fn hwnd(&self) -> HWND {
        HWND(self.window as *mut _)
    }

    /// Frames the row under the pointer when it can take mirrors; dims the window with the message otherwise.
    fn hover(&self, pt: &POINTL) {
        let row = if self.rows { crate::app::mirror_row_at(pt.x, pt.y) } else { None };
        crate::app::set_drop_row(row);
        if row.is_some() {
            self.hide();
        } else if self.overlay.get() == 0 {
            self.overlay.set(show_overlay(self.hwnd(), self.message).0 as isize);
        }
    }

    fn hide(&self) {
        let overlay = self.overlay.replace(0);
        if overlay != 0 {
            unsafe {
                let _ = DestroyWindow(HWND(overlay as *mut _));
            }
        }
    }

    fn effect(&self) -> DROPEFFECT {
        if self.links.borrow().is_empty() { DROPEFFECT_NONE } else { DROPEFFECT_COPY }
    }
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, data: Ref<'_, IDataObject>, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        *self.links.borrow_mut() = data.ok().map(links_of).unwrap_or_default();
        if !self.links.borrow().is_empty() {
            self.hover(pt);
        }
        unsafe { *effect = self.effect() };
        Ok(())
    }

    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        if !self.links.borrow().is_empty() {
            self.hover(pt);
        }
        unsafe { *effect = self.effect() };
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        self.hide();
        crate::app::set_drop_row(None);
        self.links.borrow_mut().clear();
        Ok(())
    }

    fn Drop(&self, _: Ref<'_, IDataObject>, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        self.hide();
        crate::app::set_drop_row(None);
        unsafe { *effect = self.effect() };
        let links = std::mem::take(&mut *self.links.borrow_mut());
        if !links.is_empty() {
            let dropped = Box::new(Dropped { links, x: pt.x, y: pt.y });
            unsafe {
                let _ = PostMessageW(Some(self.hwnd()), WM_DROPPED, WPARAM(0), LPARAM(Box::into_raw(dropped) as isize));
            }
        }
        Ok(())
    }
}

/// The links in what is dragged: files' links, a link, the text, and a web page's HTML (its links' addresses).
fn links_of(data: &IDataObject) -> Vec<String> {
    let format = |cf: u16| FORMATETC { cfFormat: cf, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
    let mut text = String::new();
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
            return crate::add::links_in_files(&paths).unwrap_or_default();
        }
        let url = RegisterClipboardFormatW(w!("UniformResourceLocatorW")) as u16;
        for cf in [url, CF_UNICODETEXT.0] {
            if let Ok(mut medium) = data.GetData(&format(cf)) {
                text.push('\n');
                text.push_str(&wide_text(medium.u.hGlobal));
                ReleaseStgMedium(&mut medium);
            }
        }
        let html = RegisterClipboardFormatW(w!("HTML Format")) as u16;
        if let Ok(mut medium) = data.GetData(&format(html)) {
            text.push('\n');
            text.push_str(&html_fragment(&bytes(medium.u.hGlobal)));
            ReleaseStgMedium(&mut medium);
        }
    }
    hd_core::links::find(&text)
}

/// A global memory block's UTF-16 text, up to its first NUL.
unsafe fn wide_text(memory: windows::Win32::Foundation::HGLOBAL) -> String {
    unsafe {
        let p = GlobalLock(memory) as *const u16;
        if p.is_null() {
            return String::new();
        }
        let max = GlobalSize(memory) / 2;
        let n = (0..max).find(|&i| *p.add(i) == 0).unwrap_or(max);
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
        let _ = GlobalUnlock(memory);
        text
    }
}

/// A global memory block's bytes, up to its first NUL.
pub(crate) unsafe fn bytes(memory: windows::Win32::Foundation::HGLOBAL) -> Vec<u8> {
    unsafe {
        let p = GlobalLock(memory) as *const u8;
        if p.is_null() {
            return Vec::new();
        }
        let max = GlobalSize(memory);
        let n = (0..max).find(|&i| *p.add(i) == 0).unwrap_or(max);
        let out = std::slice::from_raw_parts(p, n).to_vec();
        let _ = GlobalUnlock(memory);
        out
    }
}

/// The HTML in Windows' "HTML Format" (UTF-8 after a header of offsets): the copied fragment, so the header's
/// SourceURL (the page's own address) isn't taken for a link.
pub(crate) fn html_fragment(data: &[u8]) -> String {
    let text = String::from_utf8_lossy(data);
    let offset = |key: &str| {
        text.lines().find_map(|l| l.strip_prefix(key)).and_then(|v| v.trim().parse::<usize>().ok()).filter(|&v| v <= data.len())
    };
    let (start, end) = match (offset("StartFragment:"), offset("EndFragment:")) {
        (Some(s), Some(e)) if s <= e => (s, e),
        _ => match (offset("StartHTML:"), offset("EndHTML:")) {
            (Some(s), Some(e)) if s <= e => (s, e),
            _ => return String::new(),
        },
    };
    String::from_utf8_lossy(&data[start..end]).into_owned()
}

/// The dimmed layer over a window while links are dragged over it, with its message in the middle (the Mac's: black
/// at 45 %, white semibold text). A layered window the pointer and the drop pass through.
fn show_overlay(owner: HWND, message: &str) -> HWND {
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class = WNDCLASSW {
            lpfnWndProc: Some(DefWindowProcW),
            hInstance: instance.into(),
            lpszClassName: w!("HerbsDMDropOverlay"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let mut client = RECT::default();
        let _ = GetClientRect(owner, &mut client);
        let mut origin = POINT::default();
        let _ = ClientToScreen(owner, &mut origin);
        let (width, height) = (client.right.max(1), client.bottom.max(1));
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            w!("HerbsDMDropOverlay"),
            PCWSTR::null(),
            WS_POPUP,
            origin.x,
            origin.y,
            width,
            height,
            Some(owner),
            None,
            Some(instance.into()),
            None,
        ) else {
            return HWND::default();
        };
        // White text drawn on black in a 32-bit picture, then its brightness made the coverage: black at 45 % where
        // there is no text, white where there is.
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(dc);
            ReleaseDC(None, screen);
            return hwnd;
        };
        let old = SelectObject(dc, bitmap.into());
        let dpi = GetDpiForWindow(owner).max(96) as i32;
        let mut lf = LOGFONTW { lfHeight: -(24 * dpi / 96), lfWeight: 600, lfQuality: ANTIALIASED_QUALITY, ..Default::default() };
        let face: Vec<u16> = "Segoe UI".encode_utf16().collect();
        lf.lfFaceName[..face.len()].copy_from_slice(&face);
        let font = CreateFontIndirectW(&lf);
        let old_font = SelectObject(dc, font.into());
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0xFFFFFF));
        let mut text: Vec<u16> = message.encode_utf16().collect();
        let mut rect = RECT { left: 0, top: 0, right: width, bottom: height };
        DrawTextW(dc, &mut text, &mut rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        SelectObject(dc, old_font);
        let _ = DeleteObject(font.into());
        let _ = GdiFlush();
        let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (width * height) as usize);
        for p in pixels.iter_mut() {
            let v = ((*p >> 16) & 0xff).max((*p >> 8) & 0xff).max(*p & 0xff);
            let alpha = 115 + (140 * v) / 255;
            *p = (alpha << 24) | (v << 16) | (v << 8) | v;
        }
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8, ..Default::default() };
        let size = SIZE { cx: width, cy: height };
        let source = POINT::default();
        let _ = UpdateLayeredWindow(hwnd, Some(screen), Some(&origin), Some(&size), Some(dc), Some(&source), COLORREF(0), Some(&blend), ULW_ALPHA);
        SelectObject(dc, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        let _ = SetWindowPos(hwnd, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
        hwnd
    }
}

/// Makes a window a drop target (OLE, on its UI thread). `rows`: the main window, whose unfinished downloads take
/// links dropped on them as mirrors.
pub fn register(window: HWND, rows: bool, message: &'static str) {
    unsafe {
        let _ = OleInitialize(None);
        let target: IDropTarget =
            Target { window: window.0 as isize, rows, message, links: RefCell::new(Vec::new()), overlay: Cell::new(0) }.into();
        let _ = RegisterDragDrop(window, &target);
    }
}

/// Before the window goes.
pub fn revoke(window: HWND) {
    unsafe {
        let _ = RevokeDragDrop(window);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn html_fragment_skips_the_header() {
        let body = "<a href=\"https://example.com/a.zip\">a</a>";
        let header_len = 120;
        let header = format!(
            "Version:0.9\r\nStartHTML:{:08}\r\nEndHTML:{:08}\r\nStartFragment:{:08}\r\nEndFragment:{:08}\r\nSourceURL:https://p/x\r\n",
            header_len, header_len + body.len(), header_len, header_len + body.len()
        );
        assert_eq!(header.len(), header_len);
        assert_eq!(super::html_fragment(format!("{header}{body}").as_bytes()), body);
    }
}
