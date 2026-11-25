//! YouTube links pasted into the app (SPECIFICATION.md §15): the video's page is opened out of sight in a WebView2 of
//! its own (as `played`: its own user-data folder, removed afterwards, muted, in a window nobody sees so its player
//! really plays), with the browser extension's `youtube.js` and the core's bridge in it. `describe` waits for the
//! bridge's line about the video, for Add Links; `feed_wanted` opens a page for each fed download that has just
//! started and hands it the page's pieces. Each page runs on a thread of its own with its own message loop, so a
//! piece (a few MB of base64) is decoded and stored without holding up the window.

use crate::played::{controller, hidden_window, remove_folder, take};
use hd_core::media::{feed, youtube};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, Interface, PWSTR, w};

/// What a page said when its time ran out before its video started.
const NOT_STARTED: &str = "YouTube's page didn't start the video: try again, or download it from the browser.";

/// The fed downloads whose page is open, so each gets one.
static FEEDING: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// The bridge's line about the video `url` names, or `None` when its page never streamed it within
/// `youtube::WAIT_SECS`. Blocks: run it on a thread of its own.
pub fn describe(url: &str, data: &Path) -> Option<String> {
    let id = youtube::video_id(url)?;
    let line = Rc::new(RefCell::new(None));
    let l = line.clone();
    open(&youtube::watch_url(&id), data, move |_, json, text| {
        if json.get("hdm").and_then(|h| h.as_str()) == Some("video") {
            *l.borrow_mut() = Some(text.to_string());
            return Next::Close;
        }
        Next::Wait
    });
    line.take()
}

/// Opens a page for each fed download of the app's own that has just started (called with each refresh).
pub fn feed_wanted(data: &Path) {
    for want in feed::wanted() {
        {
            let mut feeding = FEEDING.lock().unwrap();
            if !feeding.get_or_insert_with(HashSet::new).insert(want.feed.clone()) {
                continue;
            }
        }
        let data = data.to_path_buf();
        std::thread::spawn(move || {
            run_feed(&want, &data);
            if let Some(feeding) = FEEDING.lock().unwrap().as_mut() {
                feeding.remove(&want.feed);
            }
        });
    }
}

/// Feeds one download from its page until the page is done, refused or out of time.
fn run_feed(want: &feed::Wanted, data: &Path) {
    let started = Cell::new(false);
    let ended = Rc::new(Cell::new(false));
    let (e, w) = (ended.clone(), want.clone());
    let timed_out = !open(&want.page, data, move |view, json, text| {
        let want = &w;
        let script = |s: String| unsafe {
            let _ = view.ExecuteScript(&HSTRING::from(s), &ExecuteScriptCompletedHandler::create(Box::new(|_, _| Ok(()))));
        };
        match json.get("hdm").and_then(|h| h.as_str()) {
            Some("video") if !started.get() => {
                started.set(true);
                if let Some(size) = youtube::size(text, want.sound_only, want.lines) {
                    feed::expect(&want.feed, size);
                }
                script(format!("__hdmYouTube.download({}, {})", want.sound_only, want.lines));
                Next::Forever
            }
            Some("piece") => {
                let (Some(ack), Some(data)) = (json.get("ack"), json.get("data").and_then(|d| d.as_str())) else { return Next::Forever };
                let track = json.get("track").and_then(|t| t.as_u64()).unwrap_or(0) as usize;
                let init = json.get("init").and_then(|i| i.as_bool()).unwrap_or(false);
                let ok = feed::put_base64(&want.feed, track, init, data).is_ok();
                script(format!("__hdmYouTube.ack({ack}, {ok})"));
                // Paused, cancelled or gone: the page stops at the ack and the download needs nothing more.
                if ok { Next::Forever } else { e.set(true); Next::Close }
            }
            Some("done") => {
                let error = json.get("error").and_then(|e| e.as_str()).filter(|e| !e.is_empty());
                let stopped = json.get("stopped").and_then(|s| s.as_bool()).unwrap_or(false);
                if let Some(error) = error {
                    let _ = feed::end(&want.feed, Some(error.to_string()));
                } else if !stopped {
                    let _ = feed::end(&want.feed, None);
                }
                e.set(true);
                Next::Close
            }
            _ if started.get() => Next::Forever,
            _ => Next::Wait,
        }
    });
    if timed_out && !ended.get() {
        let _ = feed::end(&want.feed, Some(NOT_STARTED.into()));
    }
}

/// What a page does after one of the bridge's lines.
enum Next {
    /// Keeps waiting, within its time.
    Wait,
    /// Keeps going without a time limit: its video is downloading.
    Forever,
    Close,
}

/// Opens `page` out of sight with the YouTube scripts in it and hands each of the bridge's lines to `told`, until it
/// says to close or the time runs out. Returns whether it was closed by `told` (false: out of time, or it couldn't
/// open).
fn open(page: &str, data: &Path, told: impl FnMut(&ICoreWebView2, &serde_json::Value, &str) -> Next + 'static) -> bool {
    static RUNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let run = RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let folder: PathBuf = data.join(format!("youtube-{}-{run}", std::process::id()));
    let mut closed = false;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if let Ok(parent) = hidden_window() {
            closed = watch(parent, page, &folder, told).unwrap_or(false);
            let _ = DestroyWindow(parent);
        }
        CoUninitialize();
    }
    remove_folder(&folder);
    closed
}

unsafe fn watch(
    parent: windows::Win32::Foundation::HWND,
    page: &str,
    folder: &Path,
    mut told: impl FnMut(&ICoreWebView2, &serde_json::Value, &str) -> Next + 'static,
) -> windows::core::Result<bool> {
    let controller = unsafe { controller(parent, folder)? };
    let view = unsafe { controller.CoreWebView2()? };
    // Until the time is up (None: no limit while downloading), or closed (`done`).
    let end = Rc::new(Cell::new(Some(Instant::now() + Duration::from_secs(youtube::WAIT_SECS))));
    let done = Rc::new(Cell::new(false));
    unsafe {
        if let Ok(v8) = view.cast::<ICoreWebView2_8>() {
            let _ = v8.SetIsMuted(true);
        }
        if let Ok(settings) = view.Settings() {
            let _ = settings.SetAreDefaultScriptDialogsEnabled(false);
        }
        // Both run in the page's own world, in its main frame only (the bridge checks that itself).
        for script in [youtube::SCRIPT, youtube::BRIDGE] {
            let script = format!("if (window.top === window) {{\n{script}\n}}");
            view.AddScriptToExecuteOnDocumentCreated(&HSTRING::from(script), None)?;
        }
        let mut token = 0i64;
        // The page can't open windows or download anything.
        view.add_NewWindowRequested(
            &NewWindowRequestedEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    args.SetHandled(true)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
        if let Ok(v4) = view.cast::<ICoreWebView2_4>() {
            let _ = v4.add_DownloadStarting(
                &DownloadStartingEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        args.SetCancel(true)?;
                    }
                    Ok(())
                })),
                &mut token,
            );
        }
        let (e, d, v) = (end.clone(), done.clone(), view.clone());
        view.add_WebMessageReceived(
            &WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                if d.get() {
                    return Ok(());
                }
                let mut text = PWSTR::null();
                if args.TryGetWebMessageAsString(&mut text).is_ok() {
                    let text = take(text);
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                        match told(&v, &json, &text) {
                            Next::Wait => {}
                            Next::Forever => e.set(None),
                            Next::Close => d.set(true),
                        }
                    }
                }
                Ok(())
            })),
            &mut token,
        )?;
        // YouTube's cookie question (asked in the EU) pauses the video until it's answered: it's answered no, as a
        // person who declines its optional cookies would (its "SOCS" cookie; the view keeps nothing afterwards).
        if let Ok(cookies) = view.cast::<ICoreWebView2_2>().and_then(|v| v.CookieManager())
            && let Ok(cookie) = cookies.CreateCookie(w!("SOCS"), w!("CAI"), w!(".youtube.com"), w!("/"))
        {
            let _ = cookie.SetIsSecure(true);
            let _ = cookies.AddOrUpdateCookie(&cookie);
        }
        view.Navigate(&HSTRING::from(page))?;

        let mut msg = MSG::default();
        while !done.get() && end.get().is_none_or(|t| Instant::now() < t) {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // An ack sent just before closing still reaches the page's script before it goes.
        let flush = Instant::now() + Duration::from_millis(200);
        while Instant::now() < flush {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = view.Stop();
        let _ = controller.Close();
    }
    Ok(done.get())
}
