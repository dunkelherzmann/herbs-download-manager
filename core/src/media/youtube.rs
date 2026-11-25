//! YouTube links pasted into the app (SPECIFICATION.md §15). YouTube streams a video only through its own player's
//! protocol, which the browser extension's `youtube.js` speaks in YouTube's page. The apps do the same without a
//! browser: they open the video's page out of sight in the system's web engine (in a window nobody sees, so its
//! player really plays), with `youtube.js` and this module's `BRIDGE` in the page. The bridge reports the video once
//! its player streams it; Add Links lists it by its title. Downloading it opens the page again and feeds its pieces
//! to a fed download (`super::feed`) whose id names the video (`feed_id`), as the extension does from its tab.
//! A live video is listed by the HLS playlist its player gives, and recorded like any live stream.

use super::scan::{Found, Kind};

/// The extension's own script, run in the page's world (not an isolated one: it watches the player's requests).
pub const SCRIPT: &str = include_str!("../../../extension/youtube.js");

/// How long the page is given to start streaming its video, in seconds.
pub const WAIT_SECS: u64 = 45;

/// The start of a fed download's id that the app itself feeds: `app-yt_<time>_<video id>`.
const FEED_PREFIX: &str = "app-yt_";

/// Talks to `youtube.js` for the app, and to the app through the message handler `hdmPlayed` (WebKit) or
/// `chrome.webview` (WebView2), one JSON line each:
/// `{"hdm":"video","video":{…describe…},"hls":…}` once the player streams the video (or a live one has its playlist);
/// `{"hdm":"piece","ack":ID,"track":N,"init":B,"data":BASE64}` for each piece of a download, which the app answers
/// with `__hdmYouTube.ack(ID, ok)`; `{"hdm":"done","error":…,"stopped":B}` at its end.
/// The app starts a download with `__hdmYouTube.download(soundOnly, lines)` (lines 0 for the best).
pub const BRIDGE: &str = r#"(() => {
  if (window.__hdmYouTube || window.top !== window) return;
  const post = (m) => { try { const s = JSON.stringify(m);
    if (window.webkit && webkit.messageHandlers && webkit.messageHandlers.hdmPlayed) webkit.messageHandlers.hdmPlayed.postMessage(s);
    else if (window.chrome && chrome.webview) chrome.webview.postMessage(s); } catch (e) {} };
  const response = () => { try {
    const p = document.getElementById('movie_player') || document.getElementById('shorts-player');
    const r = p && p.getPlayerResponse && p.getPlayerResponse();
    return (r && r.videoDetails) ? r : window.ytInitialPlayerResponse; } catch (e) { return null; } };
  const base64 = (buffer) => { const b = new Uint8Array(buffer); let s = '';
    for (let i = 0; i < b.length; i += 0x8000) s += String.fromCharCode.apply(null, b.subarray(i, i + 0x8000));
    return btoa(s); };
  let told = false, downloading = false;
  window.addEventListener('message', (e) => {
    const a = e.source === window && e.data && e.data.herbsDmYouTubeAnswer;
    if (!a) return;
    if (a.type === 'describe' && !told) {
      const video = a.video || {};
      const v = document.querySelector('video');
      const r = response();
      const hls = (r && r.streamingData && r.streamingData.hlsManifestUrl) || '';
      // Its player has played a few seconds of it (by then YouTube gave the page its full token), or it's live (listed
      // by its playlist when the player gave one, else said to wait for the broadcast's end).
      if ((video.ready && v && v.currentTime > 4) || video.live) { told = true; post({ hdm: 'video', video, hls }); }
    } else if (a.type === 'piece') {
      post({ hdm: 'piece', ack: a.ack, track: a.track, init: !!a.init, data: base64(a.data) });
    } else if (a.type === 'done') {
      post({ hdm: 'done', error: a.error || '', stopped: !!a.stopped });
    }
  });
  window.__hdmYouTube = {
    download(soundOnly, lines) {
      if (downloading) return; downloading = true;
      window.postMessage({ herbsDmYouTubeAsk: { type: 'download', id: 'app', options: { soundOnly: !!soundOnly, lines: lines || undefined } } }, location.origin);
    },
    ack(id, ok) { window.postMessage({ herbsDmYouTubeAck: { id, ok: !!ok } }, location.origin); },
  };
  setInterval(() => {
    // The page plays muted; a consent or advert button that only lets the video go on is pressed.
    document.querySelectorAll('video').forEach((v) => { v.muted = true; if (v.paused) { const p = v.play(); if (p && p.catch) p.catch(() => {}); } });
    document.querySelectorAll('.ytp-skip-ad-button, .ytp-ad-skip-button, .ytp-ad-skip-button-modern').forEach((b) => b.click());
    if (!told) window.postMessage({ herbsDmYouTubeAsk: { type: 'describe', id: 'app-describe' } }, location.origin);
  }, 700);
})();"#;

/// The video a YouTube link names: `watch?v=`, `/shorts/`, `/live/`, `/embed/` on YouTube's hosts, or `youtu.be/`.
pub fn video_id(url: &str) -> Option<String> {
    let u = reqwest::Url::parse(url.trim()).ok()?;
    if !matches!(u.scheme(), "http" | "https") {
        return None;
    }
    let host = u.host_str()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let valid = |id: &str| (6..=20).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    let mut path = u.path_segments()?.filter(|s| !s.is_empty());
    let id = if host == "youtu.be" {
        path.next()?.to_string()
    } else if ["youtube.com", "m.youtube.com", "music.youtube.com", "youtube-nocookie.com"].contains(&host) {
        match path.next()? {
            "watch" => u.query_pairs().find(|(k, _)| k == "v")?.1.into_owned(),
            "shorts" | "live" | "embed" | "v" => path.next()?.to_string(),
            _ => return None,
        }
    } else {
        return None;
    };
    valid(&id).then_some(id)
}

/// The page the app opens for a video (the watch page, whatever link named it: its player is the full one).
pub fn watch_url(id: &str) -> String {
    format!("https://www.youtube.com/watch?v={id}")
}

/// A fed download's id for a video the app feeds itself.
pub fn feed_id(video: &str) -> String {
    let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    format!("{FEED_PREFIX}{time:x}_{video}")
}

/// The video a fed download's id names, when the app feeds it itself (not the browser).
pub fn video_of_feed(id: &str) -> Option<String> {
    let rest = id.strip_prefix(FEED_PREFIX)?;
    let (_, video) = rest.split_once('_')?;
    (!video.is_empty()).then(|| video.to_string())
}

/// What the bridge said about the video: one line to list (`None` when the line isn't the bridge's video, or the
/// video can't be downloaded). `lines`: Settings' most lines for a stream (`None` for the best).
pub fn found(line: &str, lines: Option<u32>) -> Option<Found> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v.get("hdm")?.as_str()? != "video" {
        return None;
    }
    let video = v.get("video")?;
    let id = video.get("id")?.as_str()?.to_string();
    let title = video.get("title").and_then(|t| t.as_str()).map(str::trim).filter(|t| !t.is_empty()).unwrap_or("YouTube video");
    let clean: String = title.chars().take(150).map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { '_' } else { c }).collect();
    let name = format!("{}.mp4", clean.trim());
    if video.get("live").and_then(|l| l.as_bool()).unwrap_or(false) {
        // A live video: its HLS playlist, recorded like any live stream.
        let hls = v.get("hls")?.as_str().filter(|h| h.starts_with("https://"))?;
        return Some(Found { url: hls.to_string(), name, size: None, kind: Kind::Stream });
    }
    let size = size_of(video, false, lines.unwrap_or(0));
    Some(Found { url: super::feed::link(&feed_id(&id), size), name, size, kind: Kind::Stream })
}

/// What a download of the video comes to: the picture `youtube.js` picks (the best, or the best within `lines`; the
/// smallest when none is that small) and the sound.
fn size_of(video: &serde_json::Value, sound_only: bool, lines: u32) -> Option<u64> {
    let qualities = video.get("qualities").and_then(|q| q.as_array()).cloned().unwrap_or_default();
    let line_of = |q: &serde_json::Value| q.get("lines").and_then(|l| l.as_u64()).unwrap_or(0);
    let picture = qualities.iter().find(|q| lines == 0 || line_of(q) <= lines as u64).or(qualities.last());
    let picture = if sound_only { 0 } else { picture.and_then(|q| q.get("size")).and_then(|s| s.as_u64()).unwrap_or(0) };
    let size = picture + video.get("sound").and_then(|s| s.as_u64()).unwrap_or(0);
    (size > 0).then_some(size)
}

/// The size a download of the bridge's video comes to (for the fed download's progress).
pub fn size(line: &str, sound_only: bool, lines: u32) -> Option<u64> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    size_of(v.get("video")?, sound_only, lines)
}

/// What Add Links says when a YouTube link gave nothing to list (`line`: the bridge's line, if it said anything).
pub fn why_not(line: Option<&str>) -> String {
    let video = line.and_then(|l| serde_json::from_str::<serde_json::Value>(l).ok()).and_then(|v| v.get("video").cloned());
    match video {
        None => "YouTube video: its page didn't play it; try again, or download it from the browser".into(),
        Some(v) if v.get("live").and_then(|l| l.as_bool()).unwrap_or(false) => "Live YouTube video: it can be downloaded once the broadcast has ended".into(),
        Some(_) => "YouTube video: YouTube gives no MP4 picture and sound for it".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_video_in_youtube_links() {
        for (link, id) in [
            ("https://www.youtube.com/watch?v=QSw8FYSSQrI&list=PLDq3sDRAGvgE", "QSw8FYSSQrI"),
            ("https://youtube.com/shorts/abcDEF12345", "abcDEF12345"),
            ("https://m.youtube.com/watch?feature=share&v=a-b_c123456", "a-b_c123456"),
            ("https://youtu.be/aqz-KE-bpKQ?t=10", "aqz-KE-bpKQ"),
            ("https://www.youtube-nocookie.com/embed/aqz-KE-bpKQ", "aqz-KE-bpKQ"),
            ("https://www.youtube.com/live/aqz-KE-bpKQ", "aqz-KE-bpKQ"),
        ] {
            assert_eq!(video_id(link).as_deref(), Some(id), "{link}");
        }
        for link in ["https://www.youtube.com/@channel", "https://www.youtube.com/", "https://example.com/watch?v=aqz-KE-bpKQ", "https://www.youtube.com/watch?v=a/b"] {
            assert_eq!(video_id(link), None, "{link}");
        }
    }

    #[test]
    fn feed_ids_name_their_video() {
        let id = feed_id("a-b_c123456");
        assert_eq!(video_of_feed(&id).as_deref(), Some("a-b_c123456"));
        assert_eq!(video_of_feed("yt-a-b_c123456-xyz"), None, "the browser's own feeds");
        assert!(super::super::feed::link(&id, None).contains("a-b_c123456"));
    }

    #[test]
    fn lists_the_video_the_bridge_saw() {
        let line = r#"{"hdm":"video","hls":"","video":{"id":"aqz-KE-bpKQ","title":"Big Buck Bunny: 4K","live":false,"sound":30,
            "qualities":[{"lines":2160,"size":700},{"lines":1080,"size":250},{"lines":360,"size":18}]}}"#;
        let best = found(line, None).unwrap();
        assert_eq!((best.size, best.kind), (Some(730), Kind::Stream));
        assert!(best.name.starts_with("Big Buck Bunny") && best.name.ends_with(".mp4"), "{}", best.name);
        assert_eq!(video_of_feed(best.url.strip_prefix("hdm-feed:").unwrap().split('?').next().unwrap()).as_deref(), Some("aqz-KE-bpKQ"));
        assert_eq!(found(line, Some(1080)).unwrap().size, Some(280));
        assert_eq!(found(line, Some(144)).unwrap().size, Some(48), "the smallest when none is that small");
        let live = r#"{"hdm":"video","hls":"https://manifest.googlevideo.com/api/manifest/hls_variant/x","video":{"id":"QSw8FYSSQrI","title":"Rain","live":true}}"#;
        let live = found(live, None).unwrap();
        assert!(live.url.starts_with("https://manifest.googlevideo.com/") && live.size.is_none());
        assert_eq!(found(r#"{"hdm":"piece"}"#, None), None);
        assert_eq!(size(line, true, 0), Some(30));
        assert!(why_not(Some(r#"{"hdm":"video","hls":"","video":{"id":"x","live":true}}"#)).starts_with("Live"));
        assert!(why_not(None).contains("didn't play"));
        assert_eq!(size(line, false, 720), Some(48));
    }
}
