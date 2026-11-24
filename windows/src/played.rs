//! A page whose player builds its video's address only while it runs (SPECIFICATION.md §15): opened out of sight,
//! muted, in a WebView2 of its own (a user-data folder made for the run and removed after it), for a few seconds,
//! noting what its frames fetch or play. What it saw goes back to the core's page scan.
//!
//! The catch script runs in every frame and starts the players; its lines come from the page and from each frame
//! (`FrameCreated`). The network is watched too, with the frame that asked as the Referer, so a frame whose
//! messages don't arrive is still seen.

use hd_core::media::played;
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use windows::Win32::Foundation::{E_POINTER, HWND, RECT};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, Interface, PCWSTR, PWSTR, w};

/// What the page at `url` fetched or played, one line of the catch script each, after `played::WAIT_SECS` (or
/// `played::AFTER_PLAYLIST_MS` after the first playlist). Blocks: run it on a thread of its own.
pub fn run(url: &str, data: &Path) -> Vec<String> {
    let folder = data.join(format!("player-{}", std::process::id()));
    let seen = Rc::new(RefCell::new(Vec::<String>::new()));
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        // A window that is never shown, for the view to belong to.
        let parent = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!(""),
            WS_POPUP,
            0,
            0,
            960,
            540,
            None,
            None,
            None,
            None,
        );
        if let Ok(parent) = parent {
            let _ = watch(parent, url, &folder, &seen);
            let _ = DestroyWindow(parent);
        }
        CoUninitialize();
    }
    // The browser's own processes let go of the folder a moment after the view closes.
    for _ in 0..30 {
        if std::fs::remove_dir_all(&folder).is_ok() || !folder.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    seen.take()
}

unsafe fn watch(parent: HWND, url: &str, folder: &Path, seen: &Rc<RefCell<Vec<String>>>) -> windows::core::Result<()> {
    let options = CoreWebView2EnvironmentOptions::default();
    unsafe {
        options.set_additional_browser_arguments(
            "--mute-audio --autoplay-policy=no-user-gesture-required --disable-background-timer-throttling \
             --disable-renderer-backgrounding --disable-backgrounding-occluded-windows"
                .into(),
        );
    }
    let options: ICoreWebView2EnvironmentOptions = options.into();
    let dir = HSTRING::from(folder.as_os_str());
    let (tx, rx) = std::sync::mpsc::channel();
    CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| unsafe {
            CreateCoreWebView2EnvironmentWithOptions(PCWSTR::null(), &dir, &options, &handler).map_err(Error::WindowsError)
        }),
        Box::new(move |code, environment| {
            code?;
            let _ = tx.send(environment.ok_or_else(|| windows::core::Error::from(E_POINTER)));
            Ok(())
        }),
    )
    .map_err(to_win)?;
    let environment = rx.recv().map_err(|_| windows::core::Error::from(E_POINTER))??;

    let (tx, rx) = std::sync::mpsc::channel();
    CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| unsafe { environment.CreateCoreWebView2Controller(parent, &handler).map_err(Error::WindowsError) }),
        Box::new(move |code, controller| {
            code?;
            let _ = tx.send(controller.ok_or_else(|| windows::core::Error::from(E_POINTER)));
            Ok(())
        }),
    )
    .map_err(to_win)?;
    let controller = rx.recv().map_err(|_| windows::core::Error::from(E_POINTER))??;

    let end = Rc::new(Cell::new(Instant::now() + Duration::from_secs(played::WAIT_SECS)));
    let note = {
        let (seen, end) = (seen.clone(), end.clone());
        Rc::new(move |line: String| {
            let mut seen = seen.borrow_mut();
            if seen.contains(&line) {
                return;
            }
            let first = !seen.iter().any(|s| is_playlist(s));
            let list = is_playlist(&line);
            seen.push(line);
            // Once a playlist turned up, its other qualities and sound follow within moments.
            if first && list {
                end.set(end.get().min(Instant::now() + Duration::from_millis(played::AFTER_PLAYLIST_MS)));
            }
        })
    };

    unsafe {
        let _ = controller.SetBounds(RECT { left: 0, top: 0, right: 960, bottom: 540 });
        let _ = controller.SetIsVisible(true);
        let view = controller.CoreWebView2()?;
        if let Ok(v8) = view.cast::<ICoreWebView2_8>() {
            let _ = v8.SetIsMuted(true);
        }
        if let Ok(settings) = view.Settings() {
            let _ = settings.SetAreDefaultScriptDialogsEnabled(false);
        }
        view.AddScriptToExecuteOnDocumentCreated(&HSTRING::from(played::CATCH_SCRIPT), None)?;
        let mut token = 0i64;
        // The page can't open windows or download anything.
        view.add_NewWindowRequested(
            &NewWindowRequestedEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    unsafe { args.SetHandled(true)? };
                }
                Ok(())
            })),
            &mut token,
        )?;
        if let Ok(v4) = view.cast::<ICoreWebView2_4>() {
            let _ = v4.add_DownloadStarting(
                &DownloadStartingEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        unsafe { args.SetCancel(true)? };
                    }
                    Ok(())
                })),
                &mut token,
            );
            // Each frame's lines.
            let n = note.clone();
            let _ = v4.add_FrameCreated(
                &FrameCreatedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    let frame: ICoreWebView2Frame2 = unsafe { args.Frame()? }.cast()?;
                    let n = n.clone();
                    let mut token = 0i64;
                    unsafe {
                        frame.add_WebMessageReceived(
                            &FrameWebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                                if let Some(args) = args {
                                    let mut text = PWSTR::null();
                                    if unsafe { args.TryGetWebMessageAsString(&mut text) }.is_ok() {
                                        n(take(text));
                                    }
                                }
                                Ok(())
                            })),
                            &mut token,
                        )?;
                    }
                    Ok(())
                })),
                &mut token,
            );
        }
        // The page's own lines.
        let n = note.clone();
        view.add_WebMessageReceived(
            &WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    let mut text = PWSTR::null();
                    if unsafe { args.TryGetWebMessageAsString(&mut text) }.is_ok() {
                        n(take(text));
                    }
                }
                Ok(())
            })),
            &mut token,
        )?;
        // What the network gave the page and its frames, in the script's words.
        let n = note.clone();
        view.cast::<ICoreWebView2_2>()?.add_WebResourceResponseReceived(
            &WebResourceResponseReceivedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let request = unsafe { args.Request()? };
                let mut uri = PWSTR::null();
                unsafe { request.Uri(&mut uri)? };
                let uri = take(uri);
                let header = |headers: windows::core::Result<_>, name: PCWSTR| -> String {
                    let headers: ICoreWebView2HttpRequestHeaders = match headers {
                        Ok(h) => h,
                        Err(_) => return String::new(),
                    };
                    let mut value = PWSTR::null();
                    if unsafe { headers.GetHeader(name, &mut value) }.is_ok() { take(value) } else { String::new() }
                };
                let frame = header(unsafe { request.Headers() }, w!("Referer"));
                let mime = match unsafe { args.Response().and_then(|r| r.Headers()) } {
                    Ok(h) => {
                        let mut value = PWSTR::null();
                        if unsafe { h.GetHeader(w!("Content-Type"), &mut value) }.is_ok() { take(value) } else { String::new() }
                    }
                    Err(_) => String::new(),
                };
                let mime = mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
                let media = mime.contains("mpegurl") || mime.contains("dash+xml") || mime.starts_with("video/") || mime.starts_with("audio/");
                if media || played::is_playlist(&uri, &mime) {
                    n(serde_json::json!({ "url": uri, "mime": mime, "frame": frame }).to_string());
                }
                Ok(())
            })),
            &mut token,
        )?;
        view.Navigate(&HSTRING::from(url))?;

        // The view's messages, until the time is up.
        let mut msg = MSG::default();
        while Instant::now() < end.get() {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        let _ = view.Stop();
        let _ = controller.Close();
    }
    Ok(())
}

fn to_win(e: Error) -> windows::core::Error {
    match e {
        Error::WindowsError(e) => e,
        _ => windows::core::Error::from(E_POINTER),
    }
}

/// A string WebView2 handed over, freed once read.
fn take(text: PWSTR) -> String {
    if text.is_null() {
        return String::new();
    }
    let s = unsafe { text.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(text.0 as *const _)) };
    s
}

fn is_playlist(line: &str) -> bool {
    played::parse(line).is_some_and(|s| played::is_playlist(&s.url, &s.mime))
}
