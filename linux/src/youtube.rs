//! A YouTube video's page, opened out of sight (SPECIFICATION.md §15) with the browser extension's YouTube script and
//! the core's bridge in it: `describe` waits until its player streams the video and returns the bridge's line about
//! it, for Add Links; `feed_wanted` downloads it the extension's way, handing each piece to the fed download that
//! asked. As on the Mac (YouTubePage.swift).

use gtk::glib;
use gtk::prelude::*;
use hd_core::media::{feed, youtube};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;
use webkit6::prelude::*;

const DIDNT_START: &str = "YouTube's page didn't start the video: try again, or download it from the browser.";

/// One page and what it is for.
struct Page {
    view: webkit6::WebView,
    window: gtk::Window,
    content: webkit6::UserContentManager,
    timeout: RefCell<Option<glib::SourceId>>,
    /// Add Links waits for the bridge's line: `Some(None)` while waiting, `Some(Some(line))` once told.
    described: RefCell<Option<Option<String>>>,
    /// The fed download it streams.
    wanted: RefCell<Option<feed::Wanted>>,
    open: std::cell::Cell<bool>,
}

thread_local! {
    /// The pages feeding downloads, kept until each is done.
    static FEEDING: RefCell<HashMap<String, Rc<Page>>> = RefCell::default();
}

/// The bridge's line about the video the link names, or `None` when its page never streamed it.
pub async fn describe(url: &str) -> Option<String> {
    let id = youtube::video_id(url)?;
    let page = Page::open(&youtube::watch_url(&id), None);
    *page.described.borrow_mut() = Some(None);
    while page.open.get() {
        glib::timeout_future(Duration::from_millis(100)).await;
    }
    page.described.take().flatten()
}

/// Starts feeding each fed download that has just started and needs its page (called with each refresh).
pub fn feed_wanted() {
    for want in feed::wanted() {
        if FEEDING.with(|f| f.borrow().contains_key(&want.feed)) {
            continue;
        }
        let (id, address) = (want.feed.clone(), want.page.clone());
        let page = Page::open(&address, Some(want));
        if page.open.get() {
            FEEDING.with(|f| f.borrow_mut().insert(id, page));
        }
    }
}

impl Page {
    fn open(address: &str, wanted: Option<feed::Wanted>) -> Rc<Page> {
        let content = webkit6::UserContentManager::new();
        for script in [youtube::SCRIPT, youtube::BRIDGE] {
            // In the page's own world (youtube.js watches the player's requests), its main frame, at its start.
            content.add_script(&webkit6::UserScript::new(
                script,
                webkit6::UserContentInjectedFrames::TopFrame,
                webkit6::UserScriptInjectionTime::Start,
                &[],
                &[],
            ));
        }
        content.register_script_message_handler("hdmPlayed", None);
        let view = crate::played::hidden_view(&content);
        let window = crate::played::hidden_window(&view);
        let page = Rc::new(Page {
            view,
            window,
            content,
            timeout: RefCell::default(),
            described: RefCell::default(),
            wanted: RefCell::new(wanted),
            open: std::cell::Cell::new(true),
        });
        let weak = Rc::downgrade(&page);
        page.content.connect_script_message_received(Some("hdmPlayed"), move |_, value| {
            if let Some(page) = weak.upgrade() {
                page.message(&value.to_str());
            }
        });
        let weak: Weak<Page> = Rc::downgrade(&page);
        let timeout = glib::timeout_add_local_once(Duration::from_secs(youtube::WAIT_SECS), move || {
            if let Some(page) = weak.upgrade() {
                page.timeout.take();
                page.finish(Some(DIDNT_START.into()));
            }
        });
        *page.timeout.borrow_mut() = Some(timeout);
        page.view.load_uri(address);
        page
    }

    fn run(&self, script: &str) {
        self.view.evaluate_javascript(script, None, None, None::<&gtk::gio::Cancellable>, |_| {});
    }

    fn message(&self, line: &str) {
        let Ok(json) = serde_json::from_str::<serde_json::Value>(line) else { return };
        let Some(kind) = json.get("hdm").and_then(|k| k.as_str()) else { return };
        let want = self.wanted.borrow().clone();
        match kind {
            "video" => {
                if matches!(*self.described.borrow(), Some(None)) {
                    *self.described.borrow_mut() = Some(Some(line.to_string()));
                    self.finish(None);
                } else if let Some(want) = want {
                    if let Some(id) = self.timeout.take() {
                        id.remove();
                    }
                    if let Some(size) = youtube::size(line, want.sound_only, want.lines) {
                        feed::expect(&want.feed, size);
                    }
                    self.run(&format!("__hdmYouTube.download({}, {})", want.sound_only, want.lines));
                }
            }
            "piece" => {
                let Some(want) = want else { return };
                let (Some(ack), Some(data)) = (json.get("ack").and_then(|a| a.as_str()), json.get("data").and_then(|d| d.as_str())) else {
                    return;
                };
                let track = json.get("track").and_then(|t| t.as_u64()).unwrap_or(0) as usize;
                let init = json.get("init").and_then(|i| i.as_bool()).unwrap_or(false);
                let ok = feed::put_base64(&want.feed, track, init, data).is_ok();
                let quoted = serde_json::to_string(ack).unwrap_or_else(|_| "\"\"".into());
                self.run(&format!("__hdmYouTube.ack({quoted}, {ok})"));
                // Paused, cancelled or gone: the page stops at the ack and the download needs nothing more.
                if !ok {
                    self.finish(None);
                }
            }
            "done" => {
                let Some(want) = want else { return };
                let error = json.get("error").and_then(|e| e.as_str()).filter(|e| !e.is_empty()).map(str::to_string);
                let stopped = json.get("stopped").and_then(|s| s.as_bool()).unwrap_or(false);
                if error.is_none() && !stopped {
                    let _ = feed::end(&want.feed, None);
                }
                self.finish(error);
            }
            _ => {}
        }
    }

    /// Closes the page; a fed download that hasn't ended is told why (`failed`), or stays stopped (`None`).
    fn finish(&self, failed: Option<String>) {
        if !self.open.replace(false) {
            return;
        }
        if let Some(id) = self.timeout.take() {
            id.remove();
        }
        if let Some(want) = self.wanted.take() {
            if let Some(failed) = failed {
                let _ = feed::end(&want.feed, Some(failed));
            }
            // Let go of once this handler is done.
            glib::idle_add_local_once(move || {
                FEEDING.with(|f| f.borrow_mut().remove(&want.feed));
            });
        }
        self.view.stop_loading();
        self.content.unregister_script_message_handler("hdmPlayed", None);
        self.window.destroy();
    }
}

