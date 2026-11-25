//! A page whose player builds its video's address only while it runs (SPECIFICATION.md §15): opened out of sight,
//! muted, in a WebKitGTK view of its own that keeps nothing, for a few seconds, noting what its frames fetch or play.
//! What it saw goes back to the core's page scan.

use gtk::glib;
use hd_core::media::played;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};
use webkit6::prelude::*;

/// What the page at `url` fetched or played, one line of the catch script each, after `played::WAIT_SECS` (or
/// `played::AFTER_PLAYLIST_MS` after the first playlist).
pub async fn run(url: &str) -> Vec<String> {
    let content = webkit6::UserContentManager::new();
    content.add_script(&webkit6::UserScript::new(
        played::CATCH_SCRIPT,
        webkit6::UserContentInjectedFrames::AllFrames,
        webkit6::UserScriptInjectionTime::Start,
        &[],
        &[],
    ));
    content.register_script_message_handler("hdmPlayed", None);
    let view = hidden_view(&content);
    // In a window nobody sees: outside one, its player wouldn't really play.
    let window = hidden_window(&view);

    let seen = Rc::new(RefCell::new(Vec::<String>::new()));
    let end = Rc::new(Cell::new(Instant::now() + Duration::from_secs(played::WAIT_SECS)));
    {
        let (seen, end) = (seen.clone(), end.clone());
        content.connect_script_message_received(Some("hdmPlayed"), move |_, value| {
            let line = value.to_str().to_string();
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
        });
    }
    view.load_uri(url);
    while Instant::now() < end.get() {
        glib::timeout_future(Duration::from_millis(100)).await;
    }
    view.stop_loading();
    content.unregister_script_message_handler("hdmPlayed", None);
    window.destroy();
    seen.take()
}

/// A web view for a page run out of sight: it keeps nothing, plays muted without a click, and can't download.
pub fn hidden_view(content: &webkit6::UserContentManager) -> webkit6::WebView {
    let view = webkit6::WebView::builder()
        .network_session(&webkit6::NetworkSession::new_ephemeral())
        .user_content_manager(content)
        .build();
    if let Some(settings) = webkit6::prelude::WebViewExt::settings(&view) {
        settings.set_media_playback_requires_user_gesture(false);
    }
    view.set_is_muted(true);
    view.set_size_request(960, 540);
    // The page can't download anything.
    view.connect_decide_policy(|_, decision, kind| {
        if kind == webkit6::PolicyDecisionType::Response
            && let Some(d) = decision.downcast_ref::<webkit6::ResponsePolicyDecision>()
            && !d.is_mime_type_supported()
        {
            decision.ignore();
            return true;
        }
        false
    });
    view
}

/// A window nobody sees, holding `view`: a web view that isn't in a shown window doesn't play its video (YouTube's
/// then gives no full token) and slows its timers, as on the Mac. It is shown without a frame, fully transparent,
/// and lets every click through (an empty input region); the window that had the keyboard keeps it.
pub fn hidden_window(view: &webkit6::WebView) -> gtk::Window {
    use gtk::prelude::*;
    let front = gtk::Window::list_toplevels().into_iter().filter_map(|w| w.downcast::<gtk::Window>().ok()).find(|w| w.is_active());
    let window = gtk::Window::builder()
        .decorated(false)
        .resizable(false)
        .focusable(false)
        .can_focus(false)
        .can_target(false)
        .default_width(960)
        .default_height(540)
        .title("")
        .build();
    window.add_css_class("hdm-hidden-page");
    window.set_opacity(0.0);
    window.set_child(Some(view));
    window.connect_realize(|w| {
        if let Some(surface) = w.surface() {
            surface.set_input_region(Some(&gtk::cairo::Region::create()));
        }
    });
    window.connect_map(|w| {
        if let Some(surface) = w.surface() {
            surface.set_input_region(Some(&gtk::cairo::Region::create()));
        }
    });
    window.present();
    // The window that had the keyboard gets it back.
    if let Some(front) = front {
        glib::idle_add_local_once(move || front.present());
    }
    window
}

fn is_playlist(line: &str) -> bool {
    played::parse(line).is_some_and(|s| played::is_playlist(&s.url, &s.mime))
}
