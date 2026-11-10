//! Scanning a web page for its files and media (SPECIFICATION.md §15), on a page served by hd-mock with one of every
//! kind of link: what's listed must be the real files only, checked, named and sized.

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
    let dir = std::env::temp_dir().join(format!("hd-scan-{}", std::process::id()));
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
    assert_eq!(m.scan_page(&s.url("/clip.mp4")).unwrap_err(), "That link is a file, not a web page");
}

#[test]
fn hdm_scans_a_page_and_adds_what_it_found() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    s.put("/a.zip", typed(pattern(40_000, 1), &["Content-Type: application/zip"]));
    s.put("/p.html", typed(b"<title>Files</title><a href=\"/a.zip\">a</a> <a href=\"/b.zip\">b</a>".to_vec(), &["Content-Type: text/html"]));
    let dir = std::env::temp_dir().join(format!("hd-scan-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
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
