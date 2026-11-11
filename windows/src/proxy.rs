//! §9.2: the system's automatic proxy setup on Windows (an automatic setup script, PAC, or "Automatically detect
//! settings"), asked of WinHTTP for each address, which reads the script and runs it as Windows does. Without either,
//! nothing is set here and the HTTP client follows the fixed proxy settings itself.

use std::sync::{Arc, Mutex};

use windows::Win32::Foundation::{GlobalFree, HGLOBAL};
use windows::Win32::Networking::WinHttp::*;
use windows::core::{PCWSTR, PWSTR, w};

/// A PWSTR WinHTTP allocated: its text, and the memory given back.
fn take(p: PWSTR) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let text = unsafe { p.to_string() }.ok();
    unsafe {
        let _ = GlobalFree(Some(HGLOBAL(p.0 as *mut _)));
    }
    text.filter(|t| !t.trim().is_empty())
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A proxy list as Windows writes it ("host:port", "host:port;other:port", "http=host:port;https=host:port") as the
/// proxy for this address's scheme, as a URL.
pub fn pick(list: &str, scheme: &str) -> Option<String> {
    let entries: Vec<&str> = list.split([';', ' ']).map(str::trim).filter(|e| !e.is_empty()).collect();
    let chosen = entries
        .iter()
        .find_map(|e| e.strip_prefix(&format!("{scheme}=")))
        .or_else(|| entries.iter().find(|e| !e.contains('=')).copied())
        .or_else(|| entries.iter().find_map(|e| e.strip_prefix("http=")))?;
    if let Some(socks) = entries.iter().find_map(|e| e.strip_prefix("socks=")).filter(|_| chosen.is_empty()) {
        return Some(format!("socks5://{socks}"));
    }
    Some(if chosen.contains("://") { chosen.to_string() } else { format!("http://{chosen}") })
}

/// Whether an address is left out of the fixed proxy ("<local>", "*.example.com", "host").
fn bypassed(host: &str, bypass: &str) -> bool {
    bypass.split([';', ' ']).map(str::trim).filter(|b| !b.is_empty()).any(|b| {
        let b = b.to_ascii_lowercase();
        if b == "<local>" {
            return !host.contains('.');
        }
        match b.strip_prefix('*') {
            Some(suffix) => host.ends_with(suffix),
            None => host == b,
        }
    })
}

struct Session(*mut core::ffi::c_void);
unsafe impl Send for Session {}

/// Sets the resolver when the system (or a test copy's `--pac URL`) uses automatic setup. Call before the Manager opens.
pub fn install(test_pac: Option<String>) {
    let mut ie = WINHTTP_CURRENT_USER_IE_PROXY_CONFIG::default();
    let read = unsafe { WinHttpGetIEProxyConfigForCurrentUser(&mut ie) }.is_ok();
    let (auto_detect, script, fixed, bypass) = if read {
        (ie.fAutoDetect.as_bool(), take(ie.lpszAutoConfigUrl), take(ie.lpszProxy), take(ie.lpszProxyBypass))
    } else {
        (false, None, None, None)
    };
    let (auto_detect, script) = match test_pac {
        Some(url) => (false, Some(url)),
        None => (auto_detect, script),
    };
    if !auto_detect && script.is_none() {
        return;
    }
    let session = unsafe { WinHttpOpen(w!("HerbsDownloadManager"), WINHTTP_ACCESS_TYPE_NO_PROXY, PCWSTR::null(), PCWSTR::null(), 0) };
    if session.is_null() {
        return;
    }
    let session = Arc::new(Mutex::new(Session(session)));
    hd_core::http::set_proxy_resolver(Arc::new(move |url| {
        let address = wide(url.as_str());
        let script_w = script.as_deref().map(wide);
        let mut options = WINHTTP_AUTOPROXY_OPTIONS {
            dwFlags: if script_w.is_some() { WINHTTP_AUTOPROXY_CONFIG_URL } else { WINHTTP_AUTOPROXY_AUTO_DETECT },
            dwAutoDetectFlags: if script_w.is_some() { 0 } else { WINHTTP_AUTO_DETECT_TYPE_DHCP | WINHTTP_AUTO_DETECT_TYPE_DNS_A },
            lpszAutoConfigUrl: script_w.as_ref().map_or(PCWSTR::null(), |s| PCWSTR(s.as_ptr())),
            fAutoLogonIfChallenged: true.into(),
            ..Default::default()
        };
        let mut info = WINHTTP_PROXY_INFO::default();
        let found = {
            let s = session.lock().unwrap();
            unsafe { WinHttpGetProxyForUrl(s.0, PCWSTR(address.as_ptr()), &mut options, &mut info) }.is_ok()
        };
        if found {
            let list = take(info.lpszProxy);
            let _ = take(info.lpszProxyBypass);
            // The script said DIRECT, or named proxies.
            return if info.dwAccessType == WINHTTP_ACCESS_TYPE_NAMED_PROXY { list.and_then(|l| pick(&l, url.scheme())) } else { None };
        }
        // As Windows does when the script can't be read or run: the fixed proxy, if there is one.
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        fixed.as_deref().filter(|_| !bypass.as_deref().is_some_and(|b| bypassed(&host, b))).and_then(|f| pick(f, url.scheme()))
    }));
}
