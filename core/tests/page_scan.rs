//! Scanning a web page for its files and media (SPECIFICATION.md §15), on a page served by hd-mock with one of every
//! kind of link: what's listed must be the real files only, checked, named and sized.

mod common;

use hd_core::manager::Manager;
use hd_core::media::scan::Kind;
use hd_mock::{MockFile, MockServer, pattern};

fn typed(data: Vec<u8>, headers: &[&str]) -> MockFile {
    let mut f = MockFile::new(data);
    f.headers = headers.iter().map(|h| h.to_string()).collect();
    f
}

#[test]
fn a_page_lists_its_files_and_streams_never_its_site() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    let split = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/media/split");
    for name in ["master.m3u8", "0.m3u8", "1.m3u8"] {
        s.put(&format!("/clip/{name}"), typed(std::fs::read(split.join(name)).unwrap(), &["Content-Type: application/vnd.apple.mpegurl"]));
    }
    s.put("/files/guide.pdf", typed(pattern(20_000, 1), &["Content-Type: application/pdf"]));
    s.put("/files/tiny.png", typed(pattern(2_000, 2), &["Content-Type: image/png"]));
    s.put("/photo.jpg", typed(pattern(80_000, 3), &["Content-Type: image/jpeg"]));
    s.put("/clip.mp4", typed(pattern(200_000, 4), &["Content-Type: video/mp4"]));
    s.put("/download/123", typed(pattern(30_000, 5), &["Content-Disposition: attachment; filename=\"report.zip\""]));
    s.put("/s.css", typed(b"body{}".to_vec(), &["Content-Type: text/css"]));
    s.put("/app.js", typed(b"x()".to_vec(), &["Content-Type: application/javascript"]));
    s.put("/about", typed(b"<html>about</html>".to_vec(), &["Content-Type: text/html"]));
    let base = s.url("");
    let page = format!(
        r#"<html><head><title>Holiday videos</title><link rel="stylesheet" href="/s.css"><script src="/app.js"></script></head>
        <body><a href="files/guide.pdf">Guide</a> <img src="files/tiny.png"> <a href="/photo.jpg"><img src="files/tiny.png"></a>
        <video src="clip.mp4"></video> <a href="/download/123">Report</a> <a href="/about">About</a> <a href="/other.html">More</a>
        <a href="missing.zip">Gone</a>
        <script>var p = {{"hls": "{b}\/clip\/master.m3u8", "low": "{b}\/clip\/0.m3u8"}};</script></body></html>"#,
        b = base.trim_end_matches('/').replace('/', "\\/")
    );
    s.put("/holiday.html", typed(page.into_bytes(), &["Content-Type: text/html; charset=utf-8"]));
    let dir = common::Scratch::new("scan");
    let m = Manager::open(&dir).unwrap();

    assert!(m.probe(&s.url("/holiday.html")).unwrap().page, "probe says it's a web page");
    assert!(!m.probe(&s.url("/clip.mp4")).unwrap().page);
    let scan = m.scan_page(&s.url("/holiday.html")).unwrap();
    assert_eq!(scan.title, "Holiday videos");
    let listed: Vec<(&str, Option<u64>, Kind)> = scan.found.iter().map(|f| (f.name.as_str(), f.size, f.kind)).collect();
    assert_eq!(
        listed,
        [
            ("guide.pdf", Some(20_000), Kind::Document),
            ("photo.jpg", Some(80_000), Kind::Image),
            ("clip.mp4", Some(200_000), Kind::Video),
            ("report.zip", Some(30_000), Kind::Other),
            ("Holiday videos.mp4", None, Kind::Stream),
        ],
        "no icon, style, script, page or broken link; the stream once, named after the page, without its quality playlist"
    );
    assert!(scan.found[4].url.ends_with("/clip/master.m3u8"));
    // A link that is itself a file is what's found.
    let direct = m.scan_page(&s.url("/clip.mp4")).unwrap();
    assert_eq!(direct.found.iter().map(|f| (f.name.as_str(), f.kind)).collect::<Vec<_>>(), [("clip.mp4", Kind::Video)]);
}

/// What JDownloader's crawler and its plugins find on pages built by players, done generically (§15): the page read
/// as a browser reads it, embedded players opened with their page as referer, addresses hidden by escapes,
/// entities, percent-encoding, base64 or a packed script, a player's media named by its key, DASH, and links that are
/// themselves streams.
#[test]
fn a_scan_finds_what_players_hide() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    let base = s.url("").trim_end_matches('/').to_string();
    let ua = hd_core::media::scan::BROWSER_UA;
    let split = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/media/split");
    for name in ["master.m3u8", "0.m3u8", "1.m3u8"] {
        s.put(&format!("/hls/{name}"), typed(std::fs::read(split.join(name)).unwrap(), &["Content-Type: application/vnd.apple.mpegurl"]));
    }
    // A stream whose address names no type, sent as plain text.
    s.put("/live", typed(std::fs::read(split.join("0.m3u8")).unwrap(), &["Content-Type: text/plain"]));
    let dash = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/media/dash-template/manifest.mpd")).unwrap();
    s.put("/dash/show.mpd", typed(dash, &["Content-Type: application/dash+xml"]));
    for (path, seed) in [("/v/escaped.mp4", 1u8), ("/v/entity.mp4", 2), ("/v/encoded.mp4", 3), ("/v/b64.mp4", 4), ("/media/key/77", 5), ("/given.mp4", 6)] {
        s.put(path, typed(pattern(120_000 + seed as usize, seed), &["Content-Type: video/mp4"]));
    }
    // The player page answers only with its page as referer and a browser's User-Agent; it packs its setup.
    let words = format!("jwplayer|v|setup|file|{base}/hls/master.m3u8");
    let packed = format!(r#"<script>eval(function(p,a,c,k,e,d){{while(c--)if(k[c])p=p.replace(new RegExp('\\b'+c+'\\b','g'),k[c]);return p}}('0("1").2({{3:"4"}})',10,5,'{words}'.split('|'),0,{{}}))</script>"#);
    let mut player = typed(packed.into_bytes(), &["Content-Type: text/html"]);
    player.requires = vec![("referer".into(), s.url("/watch")), ("user-agent".into(), ua.into())];
    s.put("/embed/1", player);
    let b64 = base64_of(&format!("{base}/v/b64.mp4"));
    let enc = format!("{base}/v/encoded.mp4").replace(':', "%3A").replace('/', "%2F");
    let page = format!(
        r#"<html><head><title>Trip</title></head><body><iframe src="/embed/1"></iframe>
        <script>var c = {{"src": "{u}\u002Fv\u002Fescaped.mp4", "video_url": "{base}/media/key/77", "x": "{base}/stream/live"}};
        var d = "{base}/dash/show.mpd";</script><a data-a="{ent}">x</a> <a href="/go?u={enc}">y</a> <div data-b="{b64}"></div></body></html>"#,
        u = base.replace('/', "\\u002F"),
        ent = format!("{base}/v/entity.mp4").replace('/', "&#47;"),
    );
    let mut watch = typed(page.into_bytes(), &["Content-Type: text/html"]);
    watch.requires = vec![("user-agent".into(), ua.into())];
    s.put("/watch", watch);
    let dir = common::Scratch::new("scan-deep");
    let m = Manager::open(&dir).unwrap();
    let scan = m.scan_page(&s.url("/watch")).unwrap();
    let mut urls: Vec<String> = scan.found.iter().map(|f| f.url.trim_start_matches(&base).to_string()).collect();
    urls.sort();
    assert_eq!(
        urls,
        ["/dash/show.mpd", "/hls/master.m3u8", "/media/key/77", "/v/b64.mp4", "/v/encoded.mp4", "/v/entity.mp4", "/v/escaped.mp4"],
        "{:?}",
        scan.found
    );
    assert!(scan.found.iter().filter(|f| f.url.ends_with(".m3u8") || f.url.ends_with(".mpd")).all(|f| f.kind == Kind::Stream));
    assert!(scan.found.iter().any(|f| f.name == "77.mp4"), "a file named by its key gets its type's extension");
    // A link that is itself a stream, whatever its address says.
    let live = m.scan_page(&s.url("/live")).unwrap();
    assert_eq!(live.found.len(), 1);
    assert_eq!(live.found[0].kind, Kind::Stream);

    // The page as the browser shows it (after its scripts ran), handed over by the extension: not read again.
    let given = hd_core::media::scan::Given {
        pages: vec![hd_core::media::scan::GivenPage { url: s.url("/app"), html: format!(r#"<title>App</title><video src="{base}/given.mp4"></video>"#) }],
        resources: vec![],
        user_agent: Some(ua.into()),
    };
    let app = m.scan_page_given(&s.url("/app"), given).unwrap();
    assert_eq!((app.title.as_str(), app.found.iter().map(|f| f.name.as_str()).collect::<Vec<_>>()), ("App", vec!["given.mp4"]));
    assert_eq!(s.stats("/app").requests, 0, "the handed-over page isn't fetched");
    assert!(m.quit(std::time::Duration::from_secs(5)));
}

fn base64_of(text: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(text)
}

#[test]
fn hdm_scans_a_page_and_adds_what_it_found() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    s.put("/a.zip", typed(pattern(40_000, 1), &["Content-Type: application/zip"]));
    s.put("/p.html", typed(b"<title>Files</title><a href=\"/a.zip\">a</a> <a href=\"/b.zip\">b</a>".to_vec(), &["Content-Type: text/html"]));
    let dir = common::Scratch::new("scan-cli");
    let m = Manager::open(&dir).unwrap();
    hd_core::control::serve(&m, dir.join("files")).unwrap();
    let (request, _) = hd_core::cli::parse(&["scan".into(), s.url("/p.html"), "--add".into()]).unwrap();
    let answer = hd_core::control::send(&dir, &request).unwrap();
    let found = answer.found.unwrap();
    assert_eq!(found.len(), 1, "the broken link isn't listed");
    assert_eq!((found[0].name.as_str(), found[0].kind.as_str(), found[0].size), ("a.zip", "Archive", Some(40_000)));
    assert_eq!(answer.message.as_deref(), Some("Files"));
    let table = hd_core::cli::found_table("Files", &found);
    assert!(table.starts_with("Files\n") && table.contains("Archive") && table.contains("a.zip"), "{table}");
    // The browser extension's "Find Files on This Page", through hdm's native messaging side.
    let message = serde_json::json!({"command": "scan", "urls": [s.url("/p.html")], "cookies": "s=1"}).to_string().into_bytes();
    let mut input = (message.len() as u32).to_ne_bytes().to_vec();
    input.extend(message);
    let mut out = Vec::new();
    assert_eq!(hd_core::cli::native(&mut input.as_slice(), &mut out, &dir), 0);
    let native: serde_json::Value = serde_json::from_slice(&out[4..]).unwrap();
    assert_eq!(native["found"][0]["name"], "a.zip", "{native}");
    let mcp = hd_core::cli::mcp_reply(&serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "scan_page", "arguments": {"url": s.url("/p.html")}}}).to_string(), &dir).unwrap();
    assert!(mcp.contains("a.zip") && mcp.contains("\"isError\":false"), "{mcp}");
    assert!(m.quit(std::time::Duration::from_secs(5)));
}
