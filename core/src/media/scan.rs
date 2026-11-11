//! Scanning a web page for what can be downloaded (SPECIFICATION.md §15): the files and media it links to or plays,
//! never its pages, scripts, styles or other bits of the site. Every candidate is checked with its server, so what's
//! listed is a real file with its name and size.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use reqwest::Url;

/// What a found file is, for the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// An HLS playlist: downloaded as a stream and joined into MP4.
    Stream,
    Video,
    Audio,
    Image,
    Archive,
    Document,
    Program,
    Other,
}

impl Kind {
    pub fn word(self) -> &'static str {
        match self {
            Kind::Stream => "Video stream",
            Kind::Video => "Video",
            Kind::Audio => "Sound",
            Kind::Image => "Picture",
            Kind::Archive => "Archive",
            Kind::Document => "Document",
            Kind::Program => "Program",
            Kind::Other => "File",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub url: String,
    pub name: String,
    pub size: Option<u64>,
    pub kind: Kind,
}

/// The page itself: its title and what it found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scan {
    pub title: String,
    pub found: Vec<Found>,
}

/// The longest page read.
const PAGE_MAX: usize = 8 * 1024 * 1024;
/// The most pages read for one scan: the page and the players and frames it embeds, two levels deep.
const FRAMES_MAX: usize = 12;
const FRAME_DEPTH: usize = 2;
/// The most addresses taken from a player's setup (`"file":`, `"hls":`…) that name no file type.
const MEDIA_VALUES_MAX: usize = 60;

/// The User-Agent pages are read with when the browser didn't give its own: sites send other programs a bare page
/// without its player, or none at all.
#[cfg(target_os = "macos")]
pub const BROWSER_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
#[cfg(target_os = "windows")]
pub const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// What the browser extension hands over with a scan: the page as the browser shows it (every frame, after its
/// scripts ran), the addresses it already fetched, and the browser's User-Agent.
#[derive(Clone, Debug, Default)]
pub struct Given {
    pub pages: Vec<GivenPage>,
    pub resources: Vec<String>,
    pub user_agent: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct GivenPage {
    pub url: String,
    pub html: String,
}
/// The most candidates checked on one page.
const CHECK_MAX: usize = 400;
/// Checked at once, and at once on one server.
const AT_ONCE: usize = 8;
const PER_HOST: usize = 4;
/// Smaller pictures are icons and thumbnails; smaller other files are bits of the site.
const PICTURE_MIN: u64 = 50 * 1024;
const FILE_MIN: u64 = 10 * 1024;

const PAGE_EXT: &[&str] = &["html", "htm", "xhtml", "shtml", "php", "asp", "aspx", "jsp", "cgi", "pl"];
const SITE_EXT: &[&str] = &["js", "mjs", "css", "json", "map", "woff", "woff2", "ttf", "otf", "eot", "ico", "svg", "webmanifest", "xml", "rss", "atom", "txt"];
/// Stream pieces: listed as their stream, never one by one.
const PIECE_EXT: &[&str] = &["ts", "m4s", "m4f", "cmfv", "cmfa"];

fn kind_of_ext(ext: &str) -> Option<Kind> {
    Some(match ext {
        "m3u8" | "mpd" => Kind::Stream,
        "mp4" | "m4v" | "webm" | "mkv" | "mov" | "avi" | "wmv" | "flv" | "f4v" | "3gp" | "ogv" | "mpg" | "mpeg" | "m2ts" | "mts" | "vob"
        | "rmvb" | "divx" => Kind::Video,
        "mp3" | "m4a" | "m4b" | "aac" | "ogg" | "oga" | "opus" | "flac" | "wav" | "wma" | "aiff" | "mka" | "ac3" | "ape" => Kind::Audio,
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif" | "heic" | "tif" | "tiff" | "bmp" => Kind::Image,
        "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "iso" | "dmg" | "img" => Kind::Archive,
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp" | "epub" | "rtf" | "csv" => Kind::Document,
        "exe" | "msi" | "pkg" | "deb" | "rpm" | "appimage" | "apk" | "flatpakref" | "torrent" => Kind::Program,
        _ => return None,
    })
}

fn kind_of_type(mime: &str) -> Option<Kind> {
    let m = mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    Some(match m.as_str() {
        _ if m.contains("mpegurl") || m == "application/dash+xml" => Kind::Stream,
        _ if m.starts_with("video/") => Kind::Video,
        _ if m.starts_with("audio/") || m == "application/ogg" => Kind::Audio,
        "image/svg+xml" => return None,
        _ if m.starts_with("image/") => Kind::Image,
        "application/pdf" | "application/msword" | "application/epub+zip" => Kind::Document,
        _ if m.contains("officedocument") || m.contains("opendocument") => Kind::Document,
        "application/zip" | "application/x-7z-compressed" | "application/vnd.rar" | "application/x-rar-compressed" | "application/gzip"
        | "application/x-tar" | "application/x-xz" | "application/x-bzip2" | "application/zstd" | "application/x-iso9660-image"
        | "application/x-apple-diskimage" => Kind::Archive,
        "application/x-msdownload" | "application/x-msi" | "application/vnd.debian.binary-package" | "application/x-rpm"
        | "application/vnd.android.package-archive" | "application/x-bittorrent" => Kind::Program,
        "application/octet-stream" | "binary/octet-stream" | "application/x-download" | "application/force-download" => Kind::Other,
        _ => return None, // pages, scripts, styles, data
    })
}

/// The usual extension of a file type, for names that have none.
fn ext_of_type(mime: &str) -> Option<&'static str> {
    Some(match mime.split(';').next().unwrap_or("").trim() {
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/quicktime" => "mov",
        "video/x-matroska" => "mkv",
        "audio/mpeg" => "mp3",
        "audio/mp4" | "audio/x-m4a" => "m4a",
        "audio/ogg" => "ogg",
        "audio/flac" => "flac",
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "application/pdf" => "pdf",
        "application/zip" => "zip",
        _ => return None,
    })
}

fn extension(url: &Url) -> String {
    let last = url.path_segments().and_then(|mut s| s.next_back()).unwrap_or("");
    last.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

/// HTML entities that turn up in links (`&amp;`), and JSON's escaped slashes.
fn unescape(s: &str) -> String {
    s.replace("\\/", "/").replace("&amp;", "&").replace("&#38;", "&").replace("&#x2F;", "/").replace("&#47;", "/").replace("&quot;", "")
}

/// The page's title, for naming its streams.
fn title_of(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let Some(start) = lower.find("<title") else { return String::new() };
    let Some(open_end) = lower[start..].find('>').map(|i| start + i + 1) else { return String::new() };
    let Some(end) = lower[open_end..].find("</title>").map(|i| open_end + i) else { return String::new() };
    let raw = &html[open_end..end];
    let text = raw.replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every address the page mentions: in its attributes (`href`, `src`, `data-src`, `content`, `srcset`…) and anywhere
/// in its text (scripts and JSON name players' streams), in the order they appear.
pub fn links_in(html: &str, page: &Url) -> Vec<Url> {
    let base = {
        let lower = html.to_ascii_lowercase();
        lower.find("<base ").and_then(|i| {
            let tag = &html[i..i + lower[i..].find('>').unwrap_or(0)];
            attribute_values(tag).into_iter().next().and_then(|v| page.join(&unescape(&v)).ok())
        })
    }
    .unwrap_or_else(|| page.clone());
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut keep = |text: &str| {
        let text = unescape(text.trim());
        if text.is_empty() || text.starts_with('#') || text.starts_with("javascript:") || text.starts_with("mailto:") || text.starts_with("data:") {
            return;
        }
        let Ok(mut url) = base.join(&text) else { return };
        if !matches!(url.scheme(), "http" | "https") {
            return;
        }
        url.set_fragment(None);
        if url.as_str() != page.as_str() && seen.insert(url.as_str().to_string()) {
            out.push(url);
        }
    };
    // Attribute values of tags.
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        let after = &rest[lt + 1..];
        let end = after.find('>').unwrap_or(after.len());
        for v in attribute_values(&after[..end]) {
            for part in v.split(',') {
                // srcset: "a.jpg 1x, b.jpg 2x" → the addresses.
                if let Some(first) = part.split_whitespace().next() {
                    keep(first);
                }
            }
        }
        rest = &after[end..];
    }
    // Paths in quotes in scripts and data ("clip/index.m3u8", '/v/trip.mp4'): only those ending in a file type the
    // scan knows, since a script's strings are mostly other things.
    for quote in ['"', '\'', '`'] {
        for (n, piece) in html.split(quote).enumerate() {
            let piece = piece.replace("\\/", "/");
            if n % 2 == 1 && piece.len() < 2048 && !piece.contains(char::is_whitespace) && !piece.contains('<') {
                let path = piece.split(['?', '#']).next().unwrap_or("");
                let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
                if path.contains('.') && kind_of_ext(&ext).is_some() {
                    keep(&piece);
                }
            }
        }
    }
    // Addresses written out in scripts and data.
    let text = html.replace("\\/", "/");
    let mut i = 0;
    while let Some(at) = text[i..].find("http").map(|p| p + i) {
        let tail = &text[at..];
        if tail.starts_with("https://") || tail.starts_with("http://") {
            let len = tail.find(|c: char| c.is_whitespace() || "\"'<>`\\()[]{}".contains(c)).unwrap_or(tail.len());
            keep(&tail[..len]);
            i = at + len.max(4);
        } else {
            i = at + 4;
        }
    }
    out
}

/// The values of a tag's link-like attributes.
fn attribute_values(tag: &str) -> Vec<String> {
    const NAMES: &[&str] = &["href", "src", "data-src", "data-href", "data-url", "data-video", "data-stream", "content", "srcset", "data-srcset", "poster", "action"];
    let lower = tag.to_ascii_lowercase();
    let mut out = Vec::new();
    for name in NAMES {
        let mut from = 0;
        while let Some(p) = lower[from..].find(name).map(|p| p + from) {
            from = p + name.len();
            // A whole attribute name: preceded by a space and followed by "=".
            if p > 0 && !lower.as_bytes()[p - 1].is_ascii_whitespace() {
                continue;
            }
            let after = lower[from..].trim_start();
            if !after.starts_with('=') {
                continue;
            }
            let value_start = tag.len() - after.len() + 1;
            let v = tag[value_start..].trim_start();
            let value = match v.chars().next() {
                Some(q @ ('"' | '\'')) => v[1..].split(q).next().unwrap_or(""),
                _ => v.split(|c: char| c.is_whitespace()).next().unwrap_or(""),
            };
            out.push(value.to_string());
        }
    }
    out
}

/// What a candidate's server says about it.
struct Answer {
    kind: Option<Kind>,
    name: String,
    size: Option<u64>,
    /// A playlist's quality and sound playlists, to leave out of the list (a master lists them).
    variants: Vec<String>,
    is_playlist: bool,
}

/// How each request of a scan introduces itself: the browser's User-Agent, and the page the address was found on.
#[derive(Clone)]
struct Asking {
    user_agent: String,
}

impl Asking {
    fn get(&self, client: &reqwest::Client, url: &Url, vault: &crate::vault::Vault, referer: Option<&str>) -> reqwest::RequestBuilder {
        let mut rb = crate::transfer::signed(client.get(url.clone()), url, vault).header(reqwest::header::USER_AGENT, &self.user_agent);
        if let Some(r) = referer.filter(|r| !r.is_empty()) {
            rb = rb.header(reqwest::header::REFERER, r);
        }
        rb
    }
}

/// A playlist's text, read whole (playlists are small).
async fn text_of(rb: reqwest::RequestBuilder) -> Option<String> {
    tokio::time::timeout(Duration::from_secs(15), async {
        let response = rb.send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        let bytes = response.bytes().await.ok()?;
        (bytes.len() <= 4 * 1024 * 1024).then(|| String::from_utf8_lossy(&bytes).into_owned())
    })
    .await
    .ok()?
}

/// Reads a stream's playlist or manifest to be sure it is one, to find the qualities a master lists, and to leave out
/// a protected (DRM) one, which can't be saved.
fn read_stream(answer: &mut Answer, text: &str, url: &Url) -> bool {
    if text.trim_start().starts_with("#EXTM3U") {
        match super::hls::parse(text, url) {
            Ok(super::hls::Playlist::Master { variants, audio, .. }) => {
                answer.variants = variants.iter().map(|v| v.url.to_string()).chain(audio.iter().filter_map(|a| a.url.as_ref().map(|u| u.to_string()))).collect();
            }
            Ok(super::hls::Playlist::Media { segments, .. }) if segments.iter().any(|s| matches!(s.key, super::hls::Key::Protected(_))) => return false,
            Ok(_) => {}
            Err(_) => return false,
        }
    } else if text.contains("<MPD") {
        match super::dash::parse(text, url) {
            Ok(m) if !m.protected => {}
            _ => return false,
        }
    } else {
        return false;
    }
    answer.kind = Some(Kind::Stream);
    answer.is_playlist = true;
    true
}

async fn check(client: &reqwest::Client, url: &Url, vault: &crate::vault::Vault, referer: &str, asking: &Asking) -> Option<Answer> {
    let rb = asking.get(client, url, vault, Some(referer)).header(reqwest::header::RANGE, "bytes=0-0").header(reqwest::header::ACCEPT_ENCODING, "identity");
    let response = tokio::time::timeout(Duration::from_secs(15), rb.send()).await.ok()?.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let headers = response.headers().clone();
    let mime = headers.get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
    let ranged = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    let size = if ranged { crate::http::content_range_total(&headers) } else { response.content_length() };
    let name = crate::http::file_name(&headers, response.url());
    let final_url = response.url().clone();
    drop(response);
    let attachment = headers.get(reqwest::header::CONTENT_DISPOSITION).and_then(|v| v.to_str().ok()).is_some_and(|v| v.to_ascii_lowercase().contains("attachment"));
    let by_ext = kind_of_ext(&extension(&final_url)).or_else(|| kind_of_ext(&extension(url)));
    let mut kind = kind_of_type(&mime);
    // A server that types everything as octet-stream: the address says better.
    if kind == Some(Kind::Other) && by_ext.is_some() {
        kind = by_ext;
    }
    if kind.is_none() && attachment {
        kind = by_ext.or(Some(Kind::Other));
    }
    let texty = mime.is_empty() || mime.starts_with("text/") || mime.contains("json") || mime.contains("xml") || mime.contains("octet-stream");
    let html = mime.contains("html");
    // Something big that isn't text is a file even when its server names no type (as JDownloader takes it).
    if kind.is_none() && !texty && size.is_some_and(|s| s > 2 * 1024 * 1024) {
        kind = by_ext.or(Some(Kind::Other));
    }
    // A name its address gave without a type ("77"): the server's type adds it ("77.mp4").
    let name = match ext_of_type(&mime) {
        Some(ext) if !name.contains('.') => format!("{name}.{ext}"),
        _ => name,
    };
    let mut answer = Answer { kind, name, size, variants: Vec::new(), is_playlist: false };
    // A playlist or manifest, by its type or address, or text that may be one whatever its address says.
    let maybe_stream = kind == Some(Kind::Stream) || by_ext == Some(Kind::Stream) || (!html && texty && size.is_none_or(|s| s < 4 * 1024 * 1024) && (kind.is_none() || kind == Some(Kind::Other)));
    if maybe_stream {
        let text = text_of(asking.get(client, url, vault, Some(referer))).await;
        let is_stream = text.is_some_and(|t| read_stream(&mut answer, &t, url));
        if !is_stream && (kind == Some(Kind::Stream) || by_ext == Some(Kind::Stream)) {
            return None;
        }
        if !is_stream && kind == Some(Kind::Other) && mime.starts_with("text/") {
            return None;
        }
    }
    Some(answer)
}

/// A page read for the scan: its address after redirects and its text.
enum Read {
    Page(Url, String),
    /// Not a web page (a file or a stream): checked as one.
    Other(Url),
}

async fn read_page(client: &reqwest::Client, url: &Url, vault: &crate::vault::Vault, referer: Option<&str>, asking: &Asking, wait: u64) -> Result<Read, String> {
    let rb = asking.get(client, url, vault, referer).header(reqwest::header::ACCEPT, "text/html,application/xhtml+xml,*/*;q=0.8");
    let response = tokio::time::timeout(Duration::from_secs(wait), rb.send())
        .await
        .map_err(|_| "The server didn't answer".to_string())?
        .map_err(|e| crate::transfer::error_text(&e))?;
    if !response.status().is_success() {
        return Err(format!("{} {}", response.status().as_u16(), response.status().canonical_reason().unwrap_or("")).trim().to_string());
    }
    let at = response.url().clone();
    let mime = response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
    let unknown = mime.is_empty() || mime.starts_with("text/plain");
    if !mime.contains("html") && !unknown {
        return Ok(Read::Other(at));
    }
    if response.content_length().is_some_and(|l| l as usize > PAGE_MAX) {
        return Ok(Read::Other(at));
    }
    let bytes = tokio::time::timeout(Duration::from_secs(wait), response.bytes())
        .await
        .map_err(|_| "The page took too long".to_string())?
        .map_err(|e| crate::transfer::error_text(&e))?;
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(PAGE_MAX)]).into_owned();
    // A server that names no type: it's a page if it reads like one.
    if unknown && !text.trim_start().starts_with('<') {
        return Ok(Read::Other(at));
    }
    Ok(Read::Page(at, text))
}

/// The address a page sends its visitor on to (`<meta http-equiv="refresh" content="0; url=…">`).
fn refresh_target(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let at = lower.find("http-equiv=\"refresh\"").or_else(|| lower.find("http-equiv='refresh'")).or_else(|| lower.find("http-equiv=refresh"))?;
    let start = lower[..at].rfind('<')?;
    let end = start + lower[start..].find('>')?;
    let tag = &lower[start..end];
    let url_at = tag.find("url=")? + 4;
    let raw = &html[start + url_at..end];
    let value = raw.trim_start_matches(['\'', '"', ' ']).split(['\'', '"']).next()?.trim();
    (!value.is_empty()).then(|| super::scan_text::decode_entities(value))
}

fn is_web(u: &Url) -> bool {
    matches!(u.scheme(), "http" | "https")
}

/// Reads the page at `url` and checks what it mentions; blocks until done (call it off the main thread).
pub async fn scan(client: &reqwest::Client, url: &str, vault: Arc<crate::vault::Vault>, given: Given) -> Result<Scan, String> {
    let start = Url::parse(url.trim()).map_err(|_| "That isn't a web link".to_string())?;
    let asking = Asking { user_agent: given.user_agent.clone().filter(|a| !a.trim().is_empty()).unwrap_or_else(|| BROWSER_UA.to_string()) };
    // The pages to look through, with how deep in the page each one is embedded.
    let mut pages: Vec<(Url, String, usize)> = given
        .pages
        .iter()
        .filter_map(|p| Some((Url::parse(&p.url).ok().filter(is_web)?, p.html.chars().take(PAGE_MAX).collect::<String>(), 0)))
        .collect();
    if pages.is_empty() {
        match read_page(client, &start, &vault, None, &asking, 20).await? {
            Read::Page(at, html) => pages.push((at, html, 0)),
            // A link to a file or a stream: it is what's found.
            Read::Other(at) => {
                let answer = check(client, &at, &vault, "", &asking).await;
                return match answer.filter(|a| a.kind.is_some()) {
                    Some(a) => {
                        let kind = a.kind.unwrap_or(Kind::Other);
                        let name = if kind == Kind::Stream { super::download::stream_name(at.as_str()) } else { a.name };
                        Ok(Scan { title: name.clone(), found: vec![Found { url: at.to_string(), name, size: if kind == Kind::Stream { None } else { a.size }, kind }] })
                    }
                    None => Err("Nothing at that link can be downloaded".into()),
                };
            }
        }
    }
    let title = pages.iter().map(|(_, html, _)| title_of(html)).find(|t| !t.is_empty()).unwrap_or_default();
    // Embedded players and frames the browser didn't hand over (or the app read the page itself), and a page that
    // only sends its visitor on: read too, with the page that embeds them as their referer.
    let mut read: HashSet<String> = pages.iter().map(|(u, _, _)| u.as_str().to_string()).collect();
    let mut i = 0;
    while i < pages.len() && pages.len() < FRAMES_MAX {
        let (base, html, depth) = pages[i].clone();
        i += 1;
        if depth >= FRAME_DEPTH {
            continue;
        }
        let mut targets: Vec<Url> = super::scan_text::frame_sources(&html).into_iter().chain(refresh_target(&html)).filter_map(|f| base.join(&f).ok()).filter(is_web).collect();
        targets.retain(|t| kind_of_ext(&extension(t)).is_none() && read.insert(t.as_str().to_string()));
        targets.truncate(FRAMES_MAX - pages.len());
        let mut tasks = tokio::task::JoinSet::new();
        for t in targets {
            let (client, vault, asking, referer) = (client.clone(), vault.clone(), asking.clone(), base.to_string());
            tasks.spawn(async move { read_page(&client, &t, &vault, Some(&referer), &asking, 12).await });
        }
        while let Some(done) = tasks.join_next().await {
            if let Ok(Ok(Read::Page(at, text))) = done {
                read.insert(at.as_str().to_string());
                pages.push((at, text, depth + 1));
            }
        }
    }
    // Candidates, each with the page it was found on: everything but pages, the site's own scripts and styles, and
    // stream pieces. Links with no file type that stay on the site are its other pages, so only addresses with a type
    // the scan knows, download paths, and what a player's setup names as its media are asked about.
    let skipped = |u: &Url| {
        let ext = extension(u);
        PAGE_EXT.contains(&ext.as_str()) || SITE_EXT.contains(&ext.as_str()) || PIECE_EXT.contains(&ext.as_str())
    };
    let page_urls: HashSet<String> = pages.iter().map(|(u, _, _)| u.as_str().to_string()).collect();
    let mut seen = HashSet::new();
    let mut candidates: Vec<(Url, String)> = Vec::new();
    let mut media_named = 0;
    for (base, html, _) in &pages {
        let text = super::scan_text::expand(html);
        for u in links_in(&text, base) {
            if !skipped(&u) && (kind_of_ext(&extension(&u)).is_some() || extension(&u).is_empty() && looks_like_download(&u)) && !page_urls.contains(u.as_str()) && seen.insert(u.to_string()) {
                candidates.push((u, base.to_string()));
            }
        }
        for v in super::scan_text::media_values(&super::scan_text::unescape_js(html)) {
            let Ok(mut u) = base.join(&v) else { continue };
            u.set_fragment(None);
            if media_named < MEDIA_VALUES_MAX && !skipped(&u) && !page_urls.contains(u.as_str()) && seen.insert(u.to_string()) {
                media_named += 1;
                candidates.push((u, base.to_string()));
            }
        }
    }
    // What the page already fetched (from the browser): its media files and streams.
    let referer = pages.first().map(|(u, _, _)| u.to_string()).unwrap_or_default();
    for r in &given.resources {
        let Ok(mut u) = Url::parse(r) else { continue };
        u.set_fragment(None);
        if is_web(&u) && !skipped(&u) && kind_of_ext(&extension(&u)).is_some() && seen.insert(u.to_string()) {
            candidates.push((u, referer.clone()));
        }
    }
    candidates.truncate(CHECK_MAX);
    let semaphore = Arc::new(tokio::sync::Semaphore::new(AT_ONCE));
    let hosts: Arc<std::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Semaphore>>>> = Arc::default();
    let mut tasks = tokio::task::JoinSet::new();
    for (i, (url, referer)) in candidates.into_iter().enumerate() {
        let (client, vault, asking, semaphore, hosts) = (client.clone(), vault.clone(), asking.clone(), semaphore.clone(), hosts.clone());
        tasks.spawn(async move {
            let host = hosts.lock().unwrap().entry(url.host_str().unwrap_or("").to_string()).or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(PER_HOST))).clone();
            let _a = semaphore.acquire_owned().await;
            let _b = host.acquire_owned().await;
            (i, url.clone(), check(&client, &url, &vault, &referer, &asking).await)
        });
    }
    let mut answers = Vec::new();
    while let Some(Ok(done)) = tasks.join_next().await {
        answers.push(done);
    }
    answers.sort_by_key(|(i, _, _)| *i);
    let hidden: HashSet<String> = answers.iter().filter_map(|(_, _, a)| a.as_ref()).flat_map(|a| a.variants.iter().cloned()).collect();
    let streams = answers.iter().filter(|(_, u, a)| a.as_ref().is_some_and(|a| a.is_playlist) && !hidden.contains(u.as_str())).count();
    let mut found = Vec::new();
    let mut names = HashSet::new();
    for (_, url, answer) in answers {
        let Some(a) = answer else { continue };
        let Some(kind) = a.kind else { continue };
        if hidden.contains(url.as_str()) {
            continue;
        }
        let min = match kind {
            Kind::Stream => 0,
            Kind::Image => PICTURE_MIN,
            _ => FILE_MIN,
        };
        if a.size.is_some_and(|s| s < min) {
            continue;
        }
        // A page's one stream is named after the page; several, after their own playlists.
        let name = if kind == Kind::Stream {
            if streams == 1 && !title.is_empty() { format!("{}.mp4", file_safe(&title)) } else { super::download::stream_name(url.as_str()) }
        } else {
            a.name
        };
        // The same file under two addresses (a thumbnail link and its picture) is listed once.
        if !names.insert((name.clone(), a.size)) {
            continue;
        }
        found.push(Found { url: url.to_string(), name, size: if kind == Kind::Stream { None } else { a.size }, kind });
    }
    Ok(Scan { title, found })
}

/// A link with no extension that may still be a file: a download path ("/download/…", "?file=…", "/get/…").
fn looks_like_download(u: &Url) -> bool {
    let s = u.as_str().to_ascii_lowercase();
    ["download", "/get/", "/file/", "/files/", "attachment", "?file=", "&file=", "/dl/", "/media/", "/video/", "/audio/"].iter().any(|k| s.contains(k))
}

fn file_safe(s: &str) -> String {
    let clean: String = s.chars().map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { ' ' } else { c }).collect();
    let words = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    words.chars().take(120).collect::<String>().trim().trim_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_mentions_its_links_in_tags_and_scripts() {
        let page = Url::parse("https://example.com/videos/trip.html").unwrap();
        let html = r#"<html><head><title>Big &amp; Small</title><link rel="stylesheet" href="/s.css">
            <meta property="og:video" content="https://cdn.example.com/v/trip.mp4"></head><body>
            <a href="files/guide.pdf">Guide</a> <a href='#top'>Top</a> <a href="mailto:x@y">Mail</a>
            <img srcset="a.jpg 1x, b.jpg 2x"><video src=clip.webm></video>
            <script>player({"hls":"https:\/\/cdn.example.com\/live\/master.m3u8","x":1}); const s = 'media/song.mp3', t = "a b.mp4";</script></body></html>"#;
        let links: Vec<String> = links_in(html, &page).iter().map(|u| u.to_string()).collect();
        for want in [
            "https://example.com/s.css",
            "https://cdn.example.com/v/trip.mp4",
            "https://example.com/videos/files/guide.pdf",
            "https://example.com/videos/a.jpg",
            "https://example.com/videos/b.jpg",
            "https://example.com/videos/clip.webm",
            "https://cdn.example.com/live/master.m3u8",
            "https://example.com/videos/media/song.mp3",
        ] {
            assert!(links.contains(&want.to_string()), "{want} not in {links:?}");
        }
        assert!(!links.iter().any(|l| l.starts_with("mailto") || l.ends_with("#top") || l.contains("a%20b")), "{links:?}");
        assert_eq!(title_of(html), "Big & Small");
    }

    #[test]
    fn kinds_by_type_and_extension() {
        assert_eq!(kind_of_type("application/vnd.apple.mpegurl"), Some(Kind::Stream));
        assert_eq!(kind_of_type("video/mp4"), Some(Kind::Video));
        assert_eq!(kind_of_type("text/html; charset=utf-8"), None);
        assert_eq!(kind_of_type("application/javascript"), None);
        assert_eq!(kind_of_type("image/svg+xml"), None);
        assert_eq!(kind_of_ext("pdf"), Some(Kind::Document));
        assert_eq!(kind_of_ext("js"), None);
    }
}
