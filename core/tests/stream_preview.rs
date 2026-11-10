//! Previews for the browser extension's list (SPECIFICATION.md §15): `hdm`'s native messaging side answers `preview`
//! by itself with one picture of the stream's smallest quality, as a tiny MP4, and the stream's length.

use std::path::{Path, PathBuf};
use std::process::Command;

use hd_mock::{MockFile, MockServer};
use serde_json::{Value, json};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/media")
}

fn serve(s: &MockServer, name: &str) {
    for entry in std::fs::read_dir(fixtures().join(name)).unwrap() {
        let path = entry.unwrap().path();
        s.put(&format!("/{name}/{}", path.file_name().unwrap().to_string_lossy()), MockFile::new(std::fs::read(&path).unwrap()));
    }
}

/// One message through `hdm`'s native messaging side, and its answer.
fn ask(message: Value) -> Value {
    let dir = std::env::temp_dir().join(format!("hd-preview-{}", std::process::id()));
    let body = message.to_string().into_bytes();
    let mut input = (body.len() as u32).to_ne_bytes().to_vec();
    input.extend(body);
    let mut out = Vec::new();
    assert_eq!(hd_core::cli::native(&mut input.as_slice(), &mut out, &dir), 0);
    serde_json::from_slice(&out[4..]).unwrap()
}

/// The preview's MP4 (checked: one picture, and well under the browser's 1 MB message limit) and its length.
fn preview(message: Value, test: &str) -> (Vec<u8>, Option<f64>) {
    let answer = ask(message);
    assert_eq!(answer["ok"], true, "{answer}");
    let mp4 = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, answer["preview"]["mp4"].as_str().unwrap()).unwrap();
    assert!(mp4.len() < 200_000, "{test}: {} bytes", mp4.len());
    let kinds: Vec<[u8; 4]> = hd_core::media::mp4::boxes(&mp4).into_iter().map(|(k, _)| k).collect();
    assert_eq!(kinds, [*b"ftyp", *b"moov", *b"mdat"], "{test}");
    if let Some(ffmpeg) = ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg", "/usr/bin/ffmpeg"].iter().find(|p| Path::new(p).exists()) {
        let file = std::env::temp_dir().join(format!("hd-preview-{}-{test}.mp4", std::process::id()));
        std::fs::write(&file, &mp4).unwrap();
        let out = Command::new(ffmpeg).args(["-hide_banner", "-loglevel", "error", "-i"]).arg(&file)
            .args(["-map", "0", "-f", "framemd5", "-"]).output().unwrap();
        assert!(out.status.success(), "{test}: {}", String::from_utf8_lossy(&out.stderr));
        let frames = String::from_utf8_lossy(&out.stdout).lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()).count();
        assert_eq!(frames, 1, "{test}: one picture, no sound");
        let _ = std::fs::remove_file(file);
    }
    (mp4, answer["preview"]["seconds"].as_f64())
}

#[test]
fn a_stream_is_previewed_by_one_picture_of_its_smallest_quality() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    serve(&s, "h264-ts");
    serve(&s, "hevc-ts");
    serve(&s, "dash-template");
    // The master needs the browser's cookie: the preview is fetched as the page would.
    let mut master = MockFile::new(
        b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080\n/hevc-ts/index.m3u8\n\
          #EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\n/h264-ts/index.m3u8\n".to_vec(),
    );
    master.requires = vec![("cookie".into(), "s=1".into())];
    s.put("/show/master.m3u8", master);

    let refused = ask(json!({"command": "preview", "urls": [s.url("/show/master.m3u8")]}));
    assert_eq!(refused["ok"], false, "without the cookie the master isn't given");
    let (shown, seconds) = preview(json!({"command": "preview", "urls": [s.url("/show/master.m3u8")], "cookies": "s=1"}), "master");
    assert_eq!(seconds, Some(6.0), "three 2-second pieces");
    let (smallest, _) = preview(json!({"command": "preview", "urls": [s.url("/h264-ts/index.m3u8")]}), "h264");
    assert_eq!(shown, smallest, "the master's smallest quality: the 360p one");

    let (_, seconds) = preview(json!({"command": "preview", "urls": [s.url("/dash-template/manifest.mpd")]}), "dash");
    assert_eq!(seconds, Some(6.0));
    // A manifest the page carries, as Facebook's reels: its pieces' addresses are absolute.
    let text = std::fs::read_to_string(fixtures().join("dash-template/manifest.mpd")).unwrap();
    let base = s.url("/dash-template/");
    let text = text.replacen("<Period", &format!("<BaseURL>{base}</BaseURL><Period"), 1);
    preview(json!({"command": "preview", "manifest": text}), "page-dash");
}
