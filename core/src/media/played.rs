//! A page whose player builds its video's address only while it runs (§15): when reading the page finds nothing, each
//! app opens it out of sight, muted, in the system's own web engine for a few seconds and notes what its frames fetch
//! or play. These rules pick what of that is worth checking; the scan then checks it like any other address.

/// Something a page fetched or played: its address, its type (or "element" for what a `<video>` was given), and the
/// address of the frame that asked for it (sent as the Referer when it is checked).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Seen {
    pub url: String,
    pub mime: String,
    pub frame: String,
}

/// How long a page is given, and how much longer once a playlist turned up (its other qualities and sound follow).
pub const WAIT_SECS: u64 = 15;
pub const AFTER_PLAYLIST_MS: u64 = 1500;
/// The most addresses checked from one page's run.
const PICK_MAX: usize = 20;

/// The script put into every frame of the page (WebKit on the Mac and Linux; WebView2 on Windows watches the network
/// instead): it reports what `fetch` and XMLHttpRequest got with a media type or a playlist's address, and what media
/// elements are given to play, and starts the page's players muted.
pub const CATCH_SCRIPT: &str = r#"(() => {
  if (window.__hdmCatch) return; window.__hdmCatch = 1;
  const send = (url, mime) => { try {
    if (!url || /^(blob|data|about|javascript):/i.test(url)) return;
    const m = JSON.stringify({ url: new URL(url, location.href).href, mime: (mime || '').toLowerCase(), frame: location.href });
    if (window.webkit && webkit.messageHandlers && webkit.messageHandlers.hdmPlayed) webkit.messageHandlers.hdmPlayed.postMessage(m);
    else if (window.chrome && chrome.webview) chrome.webview.postMessage(m);
  } catch (e) {} };
  const look = (url, type) => { type = (type || '').toLowerCase();
    if (/mpegurl|dash\+xml|^video\/|^audio\//.test(type) || /\.(m3u8|mpd)(\?|#|$)/i.test(url || '')) send(url, type); };
  const f = window.fetch;
  if (f) window.fetch = function () { const p = f.apply(this, arguments);
    p.then(r => { try { look(r.url, r.headers.get('content-type')); } catch (e) {} }, () => {}); return p; };
  const open = XMLHttpRequest.prototype.open;
  XMLHttpRequest.prototype.open = function (method, url) {
    this.addEventListener('load', () => { try { look(this.responseURL || String(url), this.getResponseHeader('content-type')); } catch (e) {} });
    return open.apply(this, arguments); };
  const d = Object.getOwnPropertyDescriptor(HTMLMediaElement.prototype, 'src');
  if (d && d.set) Object.defineProperty(HTMLMediaElement.prototype, 'src', { configurable: true, get: d.get,
    set(v) { send(String(v), 'element'); return d.set.call(this, v); } });
  const sa = Element.prototype.setAttribute;
  Element.prototype.setAttribute = function (n, v) {
    if (/^src$/i.test(n) && (this instanceof HTMLMediaElement || this instanceof HTMLSourceElement)) send(String(v), 'element');
    return sa.apply(this, arguments); };
  setInterval(() => {
    document.querySelectorAll('video, audio, source').forEach(e => {
      if (e.src) send(e.src, 'element');
      if (e instanceof HTMLMediaElement) { e.muted = true; if (e.paused) { const p = e.play(); if (p && p.catch) p.catch(() => {}); } }
    });
    try { if (window.jwplayer) { const j = window.jwplayer(); if (j && j.getState && j.getState() !== 'playing') { if (j.setMute) j.setMute(true); if (j.play) j.play(); } } } catch (e) {}
  }, 1000);
})();"#;

/// A playlist or manifest, by its type or its address.
pub fn is_playlist(url: &str, mime: &str) -> bool {
    let mime = mime.to_ascii_lowercase();
    let path = url.split(['?', '#']).next().unwrap_or("").to_ascii_lowercase();
    mime.contains("mpegurl") || mime.contains("dash+xml") || path.ends_with(".m3u8") || path.ends_with(".mpd")
}

/// What a run saw, as one line of JSON from the catch script (`{"url":…,"mime":…,"frame":…}`).
pub fn parse(json: &str) -> Option<Seen> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let seen = Seen { url: s("url"), mime: s("mime"), frame: s("frame") };
    (seen.url.starts_with("http://") || seen.url.starts_with("https://")).then_some(seen)
}

/// What is worth checking, with the Referer to check it with: the playlists when there are any (the pieces they
/// fetched are left out), otherwise what media elements were given and media the frames fetched, without stream
/// pieces. Each address once.
pub fn picked(seen: &[Seen]) -> Vec<(String, String)> {
    let piece = |s: &Seen| {
        let path = s.url.split(['?', '#']).next().unwrap_or("").to_ascii_lowercase();
        s.mime.contains("mp2t") || [".ts", ".m4s", ".m4f", ".cmfv", ".cmfa", ".aac"].iter().any(|e| path.ends_with(e))
    };
    let lists: Vec<&Seen> = seen.iter().filter(|s| is_playlist(&s.url, &s.mime)).collect();
    let chosen: Vec<&Seen> = if lists.is_empty() { seen.iter().filter(|s| !piece(s)).collect() } else { lists };
    let mut done = std::collections::HashSet::new();
    chosen
        .into_iter()
        .filter(|s| done.insert(s.url.clone()))
        .take(PICK_MAX)
        .map(|s| (s.url.clone(), s.frame.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(url: &str, mime: &str) -> Seen {
        Seen { url: url.into(), mime: mime.into(), frame: "https://player.example.com/play/1".into() }
    }

    #[test]
    fn playlists_win_over_their_pieces() {
        let all = [
            seen("https://p.example.com/m3u8/abc", "application/vnd.apple.mpegurl"),
            seen("https://p.example.com/segs/abc/000.html", "video/mp4"),
            seen("https://p.example.com/m3u8/abc", "application/vnd.apple.mpegurl"),
        ];
        assert_eq!(picked(&all), vec![("https://p.example.com/m3u8/abc".to_string(), "https://player.example.com/play/1".to_string())]);
    }

    #[test]
    fn without_a_playlist_media_counts_but_pieces_do_not() {
        let all = [seen("https://cdn.example.com/v/clip.mp4", "element"), seen("https://cdn.example.com/a/1.ts", "video/mp2t"), seen("https://cdn.example.com/a/2.m4s", "video/mp4")];
        assert_eq!(picked(&all).len(), 1);
        assert_eq!(picked(&all)[0].0, "https://cdn.example.com/v/clip.mp4");
    }

    #[test]
    fn parses_the_script_lines() {
        let s = parse(r#"{"url":"https://p.example.com/m3u8/abc","mime":"application/vnd.apple.mpegurl","frame":"https://p.example.com/play/abc"}"#).unwrap();
        assert!(is_playlist(&s.url, &s.mime));
        assert_eq!(parse(r#"{"url":"blob:https://x/1","mime":"","frame":""}"#), None);
        assert_eq!(parse("not json"), None);
    }
}
