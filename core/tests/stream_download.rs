//! Downloading streams from the list (SPECIFICATION.md §15): the test streams in `tests/media`, served by hd-mock, added
//! as links like any other download. The joined file must be exactly what joining the pieces directly gives.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use hd_core::manager::{Manager, NewDownload, Status};
use hd_core::media::join::Joiner;
use hd_mock::{MockFile, MockServer};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/media")
}

/// The test's own folder (with `files` in it for downloads), removed when the test ends.
fn folder(name: &str) -> common::Scratch {
    let dir = common::Scratch::new(&format!("stream-{name}"));
    std::fs::create_dir_all(dir.join("files")).unwrap();
    dir
}

/// Serves every file of a fixture stream at `/<name>/<file>`, each at `rate` bytes a second if given.
fn serve(s: &MockServer, name: &str, rate: Option<u64>) {
    for entry in std::fs::read_dir(fixtures().join(name)).unwrap() {
        let path = entry.unwrap().path();
        let mut file = MockFile::new(std::fs::read(&path).unwrap());
        file.rate = rate;
        s.put(&format!("/{name}/{}", path.file_name().unwrap().to_string_lossy()), file);
    }
}

/// The MP4 the joiner makes from the fixture's own files: what the download must end up with.
fn joined_directly(name: &str, dir: &Path) -> Vec<u8> {
    let base = fixtures().join(name);
    let mut j = Joiner::new(&dir.join("direct-spool")).unwrap();
    let source = j.source();
    let mut n = 0;
    while base.join(format!("s{n}.ts")).exists() {
        j.piece(source, &std::fs::read(base.join(format!("s{n}.ts"))).unwrap(), false).unwrap();
        n += 1;
    }
    let out = dir.join("direct.mp4");
    j.finish(&out).unwrap();
    std::fs::read(out).unwrap()
}

/// The video pieces fully downloaded into a parts folder.
fn pieces_in(parts: &Path) -> usize {
    std::fs::read_dir(parts)
        .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("0-") && !n.ends_with(".tmp")).count())
        .unwrap_or(0)
}

fn status(m: &Manager, id: u64) -> Status {
    m.rows().into_iter().find(|r| r.id == id).unwrap().status
}

fn wait_for(m: &Manager, id: u64, what: &str, ok: impl Fn(&Status) -> bool) {
    let until = Instant::now() + Duration::from_secs(30);
    while !ok(&status(m, id)) {
        assert!(Instant::now() < until, "never {what}: {:?}", status(m, id));
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn add(m: &Manager, url: String, dir: &Path) -> u64 {
    m.add(NewDownload { url, folder: dir.join("files"), connections: 4, start: true, ..NewDownload::default() })
}

#[test]
fn a_stream_link_downloads_its_pieces_and_joins_them() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    serve(&s, "h264-ts", None);
    let dir = folder("vod");
    let m = Manager::open(&dir).unwrap();
    let id = add(&m, s.url("/h264-ts/index.m3u8"), &dir);
    let row = m.rows().into_iter().find(|r| r.id == id).unwrap();
    assert_eq!(row.name, "h264-ts.mp4", "named after the playlist's folder, as MP4");
    wait_for(&m, id, "finished", |s| *s == Status::Finished);
    let path = dir.join("files/h264-ts.mp4");
    assert_eq!(std::fs::read(&path).unwrap(), joined_directly("h264-ts", &dir), "the same file as joining the pieces directly");
    assert!(!dir.join("files/h264-ts.mp4.hdparts").exists(), "the pieces are gone once joined");
    let row = m.rows().into_iter().find(|r| r.id == id).unwrap();
    assert_eq!(row.size, Some(std::fs::metadata(&path).unwrap().len()), "the list shows the file's size, not the pieces'");
    assert!(m.quit(Duration::from_secs(5)));
}

#[test]
fn a_paused_stream_continues_with_the_pieces_it_has() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    // Slow pieces: about a second each, so it's paused part way.
    serve(&s, "h264-ts", Some(60_000));
    let dir = folder("pause");
    let m = Manager::open(&dir).unwrap();
    let id = add(&m, s.url("/h264-ts/index.m3u8"), &dir);
    let parts = dir.join("files/h264-ts.mp4.hdparts");
    let until = Instant::now() + Duration::from_secs(30);
    while std::fs::read_dir(&parts).map(|d| d.flatten().filter(|e| !e.file_name().to_string_lossy().ends_with(".tmp")).count()).unwrap_or(0) == 0 {
        assert!(Instant::now() < until, "no piece arrived");
        std::thread::sleep(Duration::from_millis(20));
    }
    m.pause(&[id]);
    wait_for(&m, id, "paused", |s| *s == Status::Paused);
    let kept: Vec<String> = std::fs::read_dir(&parts).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| !n.ends_with(".tmp")).collect();
    assert!(!kept.is_empty() && kept.len() < 3, "paused part way: {kept:?}");
    serve(&s, "h264-ts", None);
    m.resume(&[id]);
    wait_for(&m, id, "finished", |s| *s == Status::Finished);
    assert_eq!(std::fs::read(dir.join("files/h264-ts.mp4")).unwrap(), joined_directly("h264-ts", &dir));
    assert!(m.quit(Duration::from_secs(5)));
}

#[test]
fn a_live_stream_is_recorded_until_it_ends() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    serve(&s, "h264-ts", None);
    // A live playlist: one piece, then two, then all three and the end.
    let playlist = |n: usize, end: bool| {
        let mut p = String::from("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n");
        for i in 0..n {
            p.push_str(&format!("#EXTINF:2.0,\ns{i}.ts\n"));
        }
        if end {
            p.push_str("#EXT-X-ENDLIST\n");
        }
        MockFile::new(p.into_bytes())
    };
    s.put("/h264-ts/live.m3u8", playlist(1, false));
    let dir = folder("live");
    let m = Manager::open(&dir).unwrap();
    let id = add(&m, s.url("/h264-ts/live.m3u8"), &dir);
    let parts = dir.join("files/live.mp4.hdparts");
    let pieces = |n: usize| pieces_in(&parts) >= n;
    let until = Instant::now() + Duration::from_secs(30);
    while !pieces(1) {
        assert!(Instant::now() < until, "the first piece never came");
        std::thread::sleep(Duration::from_millis(20));
    }
    s.put("/h264-ts/live.m3u8", playlist(2, false));
    while !pieces(2) {
        assert!(Instant::now() < until, "the second piece never came");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(status(&m, id).is_running(), "still recording");
    s.put("/h264-ts/live.m3u8", playlist(3, true));
    wait_for(&m, id, "finished", |s| *s == Status::Finished);
    assert_eq!(std::fs::read(dir.join("files/live.mp4")).unwrap(), joined_directly("h264-ts", &dir), "every piece, in order");
    assert!(m.quit(Duration::from_secs(5)));
}

#[test]
fn pausing_a_live_recording_saves_what_it_has() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    serve(&s, "h264-ts", None);
    s.put("/h264-ts/live.m3u8", MockFile::new(b"#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:2.0,\ns0.ts\n#EXTINF:2.0,\ns1.ts\n".to_vec()));
    let dir = folder("live-pause");
    let m = Manager::open(&dir).unwrap();
    let id = add(&m, s.url("/h264-ts/live.m3u8"), &dir);
    let parts = dir.join("files/live.mp4.hdparts");
    let until = Instant::now() + Duration::from_secs(30);
    while pieces_in(&parts) < 2 {
        assert!(Instant::now() < until, "the pieces never came");
        std::thread::sleep(Duration::from_millis(20));
    }
    m.pause(&[id]);
    wait_for(&m, id, "finished", |s| *s == Status::Finished);
    let saved = std::fs::read(dir.join("files/live.mp4")).unwrap();
    assert!(saved.len() > 10_000 && &saved[4..8] == b"ftyp", "the recording so far is an MP4");
    assert!(m.quit(Duration::from_secs(5)));
}

#[test]
fn a_protected_stream_is_refused() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    s.put("/drm/index.m3u8", MockFile::new(
        b"#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://key\",KEYFORMAT=\"com.apple.streamingkeydelivery\"\n#EXTINF:4,\na.ts\n#EXT-X-ENDLIST\n".to_vec(),
    ));
    let dir = folder("drm");
    let m = Manager::open(&dir).unwrap();
    let id = add(&m, s.url("/drm/index.m3u8"), &dir);
    wait_for(&m, id, "failed", |s| matches!(s, Status::Failed(_)));
    let Status::Failed(reason) = status(&m, id) else { unreachable!() };
    assert!(reason.contains("protected (DRM)"), "{reason}");
    assert!(m.quit(Duration::from_secs(5)));
}

/// A master playlist over two fixtures: 1080p (the HEVC stream) and 360p (the H.264 one).
fn serve_master(s: &MockServer) {
    serve(s, "h264-ts", None);
    serve(s, "hevc-ts", None);
    s.put("/show/master.m3u8", MockFile::new(
        b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=5000000,RESOLUTION=1920x1080\n/hevc-ts/index.m3u8\n\
          #EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\n/h264-ts/index.m3u8\n".to_vec(),
    ));
}

#[test]
fn a_stream_downloads_the_quality_chosen_for_it_or_in_settings() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    serve_master(&s);
    let dir = folder("quality");
    let m = Manager::open(&dir).unwrap();
    // Chosen for it (the extension's picker): the 360p version.
    let chosen = hd_core::media::StreamChoice { quality: Some(480), ..Default::default() };
    let id = m.add(NewDownload { url: s.url("/show/master.m3u8"), folder: dir.join("files"), name: Some("chosen.mp4".into()), connections: 4,
                                 start: true, stream: true, choice: Some(chosen), ..NewDownload::default() });
    wait_for(&m, id, "finished", |s| *s == Status::Finished);
    assert_eq!(std::fs::read(dir.join("files/chosen.mp4")).unwrap(), joined_directly("h264-ts", &dir), "the 360p version");

    // Settings' quality (S13), fixed when the download is added: changing it afterwards doesn't change the download.
    let mut settings = m.settings();
    settings.stream_quality = Some(360);
    m.set_settings(settings.clone());
    let id = m.add(NewDownload { url: s.url("/show/master.m3u8"), folder: dir.join("files"), name: Some("settings.mp4".into()), connections: 4,
                                 start: false, stream: true, ..NewDownload::default() });
    settings.stream_quality = None;
    m.set_settings(settings);
    m.resume(&[id]);
    wait_for(&m, id, "finished", |s| *s == Status::Finished);
    assert_eq!(std::fs::read(dir.join("files/settings.mp4")).unwrap(), joined_directly("h264-ts", &dir), "Settings' 360p at the time it was added");
    assert!(m.quit(Duration::from_secs(5)));
}

/// The sound alone, as the joiner makes it from the playlist's own pieces (`<prefix>s0.ts`…).
fn sound_directly(name: &str, prefix: &str, dir: &Path) -> Vec<u8> {
    let base = fixtures().join(name);
    let mut j = Joiner::new(&dir.join("direct-sound-spool")).unwrap();
    j.sound_only();
    let source = j.source();
    let mut n = 0;
    while base.join(format!("{prefix}s{n}.ts")).exists() {
        j.piece(source, &std::fs::read(base.join(format!("{prefix}s{n}.ts"))).unwrap(), false).unwrap();
        n += 1;
    }
    let out = dir.join("direct-sound");
    j.finish(&out).unwrap();
    std::fs::read(out).unwrap()
}

#[test]
fn sound_only_downloads_just_the_sound_track() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    // The picture's playlist is there but none of its pieces: fetching one would fail the download.
    for entry in std::fs::read_dir(fixtures().join("split")).unwrap() {
        let path = entry.unwrap().path();
        let file = path.file_name().unwrap().to_string_lossy().to_string();
        if !file.starts_with("0_") {
            s.put(&format!("/split/{file}"), MockFile::new(std::fs::read(&path).unwrap()));
        }
    }
    serve(&s, "mp3-ts", None);
    let dir = folder("sound-only");
    let m = Manager::open(&dir).unwrap();
    let sound_only = Some(hd_core::media::StreamChoice { sound_only: true, ..Default::default() });

    // A separate sound track: only its pieces, saved as .m4a under the stream's name.
    let id = m.add(NewDownload { url: s.url("/split/master.m3u8"), folder: dir.join("files"), connections: 4, start: true,
                                 choice: sound_only.clone(), ..NewDownload::default() });
    wait_for(&m, id, "finished", |s| *s == Status::Finished);
    let row = m.rows().into_iter().find(|r| r.id == id).unwrap();
    assert_eq!(row.name, "split.m4a");
    assert_eq!(std::fs::read(dir.join("files/split.m4a")).unwrap(), sound_directly("split", "1_", &dir));

    // MP3 sound in the video's own pieces: its frames as an .mp3 file, whatever name it was added under.
    let id = m.add(NewDownload { url: s.url("/mp3-ts/index.m3u8"), folder: dir.join("files"), name: Some("song.mp4".into()),
                                 connections: 4, start: true, choice: sound_only, ..NewDownload::default() });
    wait_for(&m, id, "finished", |s| *s == Status::Finished);
    let row = m.rows().into_iter().find(|r| r.id == id).unwrap();
    assert_eq!(row.path.as_deref(), Some(dir.join("files/song.mp3").as_path()));
    assert_eq!(row.name, "song.mp3");
    assert_eq!(std::fs::read(dir.join("files/song.mp3")).unwrap(), sound_directly("mp3-ts", "", &dir));
    assert!(!dir.join("files/song.m4a").exists());
    assert!(m.quit(Duration::from_secs(5)));
}

/// The language of each subtitle track (`sbtl` handler) in an MP4, as ffprobe would list them.
fn subtitle_languages(mp4: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = mp4[at..].windows(4).position(|w| w == b"sbtl").map(|i| i + at) {
        // mdhd comes before hdlr in mdia: its language is the 2 bytes after the version-1 duration.
        let mdhd = mp4[..i].windows(4).rposition(|w| w == b"mdhd").unwrap();
        let packed = u16::from_be_bytes([mp4[mdhd + 4 + 4 + 28], mp4[mdhd + 4 + 4 + 29]]);
        out.push((0..3).rev().map(|k| (((packed >> (5 * k)) & 0x1f) as u8 + 0x60) as char).collect());
        at = i + 4;
    }
    out
}

#[test]
fn a_stream_keeps_the_subtitles_chosen_for_it() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    serve(&s, "split", None);
    serve(&s, "subs", None);
    let dir = folder("subtitles");
    let m = Manager::open(&dir).unwrap();
    let add = |name: &str, subtitles: &[&str]| {
        let choice = hd_core::media::StreamChoice { subtitles: subtitles.iter().map(|l| l.to_string()).collect(), ..Default::default() };
        let id = m.add(NewDownload { url: s.url("/subs/master.m3u8"), folder: dir.join("files"), name: Some(name.into()), connections: 4,
                                     start: true, stream: true, choice: Some(choice), ..NewDownload::default() });
        wait_for(&m, id, "finished", |s| *s == Status::Finished);
        std::fs::read(dir.join("files").join(name)).unwrap()
    };
    assert_eq!(subtitle_languages(&add("none.mp4", &[])), Vec::<String>::new(), "S15's default: none");
    assert_eq!(subtitle_languages(&add("en.mp4", &["en"])), ["eng"]);
    assert_eq!(subtitle_languages(&add("all.mp4", &["*"])), ["eng", "deu"], "all, forced ones left out");
    assert!(m.quit(Duration::from_secs(5)));
}
