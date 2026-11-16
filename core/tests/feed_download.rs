//! Streams the browser fetches itself and hands over piece by piece (SPECIFICATION.md §15, YouTube's): a download
//! with an `hdm-feed:` link takes each track's start section and pieces as the extension sends them, then joins
//! them. Fed here with `tests/media/dash-ondemand`, whose files are laid out as YouTube's are (a start section and
//! its index, then fragments); where ffmpeg is installed, the joined file must decode as ffmpeg's join does.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use hd_core::manager::{Manager, NewDownload, Status};
use hd_core::media::feed;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/media")
}

/// A file's top-level boxes split as the browser receives them: its start section (everything before the first
/// fragment), then each fragment (`moof` with its `mdat`).
fn pieces(file: &[u8]) -> (Vec<u8>, Vec<Vec<u8>>) {
    let mut boxes = Vec::new();
    let mut i = 0;
    while i + 8 <= file.len() {
        let size = u32::from_be_bytes(file[i..i + 4].try_into().unwrap()) as usize;
        boxes.push((&file[i + 4..i + 8], &file[i..i + size]));
        i += size;
    }
    let first = boxes.iter().position(|(k, _)| *k == b"moof").unwrap();
    let init = boxes[..first].iter().flat_map(|(_, b)| b.to_vec()).collect();
    let mut out: Vec<Vec<u8>> = Vec::new();
    for (k, b) in &boxes[first..] {
        if *k == b"moof" {
            out.push(b.to_vec());
        } else {
            out.last_mut().unwrap().extend_from_slice(b);
        }
    }
    (init, out)
}

/// Hands a piece over as the extension does: again while the download hasn't started yet.
fn put(id: &str, track: usize, init: bool, data: &[u8]) {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        match feed::put(id, track, init, data) {
            Ok(()) => return,
            Err(e) if e == feed::WAITING && Instant::now() < until => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => panic!("piece refused: {e}"),
        }
    }
}

fn wait(m: &Manager, id: u64, until: impl Fn(&Status) -> bool) -> Status {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let status = m.rows().into_iter().find(|r| r.id == id).unwrap().status;
        if until(&status) {
            return status;
        }
        assert!(Instant::now() < deadline, "still {status:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn ffmpeg() -> Option<PathBuf> {
    ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg", "/usr/bin/ffmpeg"].iter().map(PathBuf::from).find(|p| p.exists())
}

/// Each stream's decoded frames: (presentation seconds, size and checksum).
fn decoded(ffmpeg: &Path, input: &Path) -> Vec<Vec<(f64, String)>> {
    let out = Command::new(ffmpeg).args(["-hide_banner", "-loglevel", "error", "-i"]).arg(input)
        .args(["-map", "0:v?", "-map", "0:a?", "-f", "framemd5", "-"]).output().unwrap();
    assert!(out.status.success(), "ffmpeg: {}", String::from_utf8_lossy(&out.stderr));
    let (mut tb, mut streams): (Vec<f64>, Vec<Vec<(f64, String)>>) = (Vec::new(), Vec::new());
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some(rest) = line.strip_prefix("#tb ") {
            let (n, d) = rest.split_once(": ").unwrap().1.split_once('/').unwrap();
            tb.push(n.parse::<f64>().unwrap() / d.parse::<f64>().unwrap());
            streams.push(Vec::new());
        } else if !line.starts_with('#') && !line.trim().is_empty() {
            let f: Vec<&str> = line.split(',').map(str::trim).collect();
            let s: usize = f[0].parse().unwrap();
            streams[s].push((f[2].parse::<f64>().unwrap() * tb[s], format!("{} {}", f[4], f[5])));
        }
    }
    streams
}

/// Ours against ffmpeg's join of `video` and `audio` (copying, no re-encoding).
fn same_as_ffmpeg(ours: &Path, video: &Path, audio: &Path) {
    let Some(ffmpeg) = ffmpeg() else {
        eprintln!("no ffmpeg here: {} joined but not compared", ours.display());
        return;
    };
    let reference = ours.with_file_name("ffmpeg.mp4");
    let made = Command::new(&ffmpeg).args(["-hide_banner", "-loglevel", "error", "-y", "-i"]).arg(video).arg("-i").arg(audio)
        .args(["-map", "0:v", "-map", "1:a", "-c", "copy"]).arg(&reference).output().unwrap();
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
    let (a, b) = (decoded(&ffmpeg, ours), decoded(&ffmpeg, &reference));
    assert_eq!(a.len(), 2, "picture and sound in one file");
    for s in 0..2 {
        let hashes = |v: &Vec<(f64, String)>| v.iter().map(|(_, h)| h.clone()).collect::<Vec<_>>();
        assert_eq!(hashes(&a[s]), hashes(&b[s]), "stream {s}'s frames differ from ffmpeg's join");
        for (i, ((ta, _), (tb, _))) in a[s].iter().zip(&b[s]).enumerate() {
            assert!(((ta - a[s][0].0) - (tb - b[s][0].0)).abs() < 0.001, "stream {s} frame {i}");
        }
    }
    // Sound against picture: as the source files place them, which is how a DASH player plays them. (ffmpeg's join
    // of two separate files starts each at zero instead: with a video whose first picture comes 0.08 s in (B-frames,
    // no edit list), that moves the sound 80 ms against it. Ours keeps the source's timing.)
    let start = |f: &Path| -> f64 {
        let out = Command::new(ffmpeg.with_file_name("ffprobe")).args(["-v", "error", "-show_entries", "stream=start_time", "-of", "csv=p=0"]).arg(f).output().unwrap();
        String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
    };
    let source = start(audio) - start(video);
    let ours_offset = a[1][0].0 - a[0][0].0;
    assert!((ours_offset - source).abs() < 0.001, "sound moved against the picture: {ours_offset} s, the source's {source} s");
}

#[test]
fn a_fed_stream_is_joined_from_the_pieces_the_browser_sends() {
    let dir = common::Scratch::new("feed-joined");
    std::fs::create_dir_all(dir.join("files")).unwrap();
    let m = Manager::open(&dir).unwrap();
    let base = fixtures().join("dash-ondemand");
    let (video, audio) = (std::fs::read(base.join("video.mp4")).unwrap(), std::fs::read(base.join("audio.mp4")).unwrap());
    let size = (video.len() + audio.len()) as u64;
    let id = m.add(NewDownload { url: feed::link("feed-test-1", Some(size)), folder: dir.join("files"), name: Some("A Video.mp4".into()), stream: true, start: true, ..NewDownload::default() });
    // As the browser receives them: the two tracks' pieces interleaved, each track in order.
    let (vi, vp) = pieces(&video);
    let (ai, ap) = pieces(&audio);
    put("feed-test-1", 0, true, &vi);
    put("feed-test-1", 1, true, &ai);
    for i in 0..vp.len().max(ap.len()) {
        if let Some(p) = vp.get(i) {
            put("feed-test-1", 0, false, p);
        }
        if let Some(p) = ap.get(i) {
            put("feed-test-1", 1, false, p);
        }
    }
    feed::end("feed-test-1", None).unwrap();
    match wait(&m, id, |s| matches!(s, Status::Finished | Status::Failed(_))) {
        Status::Finished => {}
        other => panic!("{other:?}"),
    }
    let row = m.rows().into_iter().find(|r| r.id == id).unwrap();
    assert_eq!((row.connections_text.as_str(), row.mirrors_text.as_str()), ("1", "1 of 1"), "the page's stream is its one connection");
    let path = row.path.unwrap();
    assert_eq!(path.file_name().unwrap(), "A Video.mp4");
    assert!(!path.with_file_name("A Video.mp4.hdparts").exists(), "the pieces go once joined");
    same_as_ffmpeg(&path, &base.join("video.mp4"), &base.join("audio.mp4"));
    assert!(m.quit(Duration::from_secs(5)));
}

#[test]
fn a_fed_stream_is_never_a_mirror_of_another_download() {
    let dir = common::Scratch::new("feed-own");
    std::fs::create_dir_all(dir.join("files")).unwrap();
    let m = Manager::open(&dir).unwrap();
    let new = |id: &str| NewDownload { url: feed::link(id, Some(10)), folder: dir.join("files"), name: Some("Same Name.mp4".into()), stream: true, ..NewDownload::default() };
    let first = m.add(new("feed-own-1"));
    match m.add_or_mirror(new("feed-own-2")) {
        hd_core::manager::Added::New(second) => assert_ne!(first, second),
        other => panic!("{other:?}"),
    }
    assert!(m.quit(Duration::from_secs(5)));
}

#[test]
fn a_fed_stream_fails_with_the_browsers_reason_and_stops_taking_pieces_when_paused() {
    let dir = common::Scratch::new("feed-stopped");
    std::fs::create_dir_all(dir.join("files")).unwrap();
    let m = Manager::open(&dir).unwrap();
    let failing = m.add(NewDownload { url: feed::link("feed-test-2", None), folder: dir.join("files"), name: Some("B.mp4".into()), stream: true, start: true, ..NewDownload::default() });
    put("feed-test-2", 0, false, b"not yet joined");
    feed::end("feed-test-2", Some("YouTube refused the stream (403).".into())).unwrap();
    match wait(&m, failing, |s| matches!(s, Status::Failed(_))) {
        Status::Failed(e) => assert!(e.contains("YouTube refused"), "{e}"),
        _ => unreachable!(),
    }
    let paused = m.add(NewDownload { url: feed::link("feed-test-3", None), folder: dir.join("files"), name: Some("C.mp4".into()), stream: true, start: true, ..NewDownload::default() });
    put("feed-test-3", 0, false, b"a piece");
    m.pause(&[paused]);
    wait(&m, paused, |s| matches!(s, Status::Paused));
    assert_eq!(feed::put("feed-test-3", 0, false, b"more"), Err(feed::STOPPED.into()), "the browser is told to stop sending");
    assert!(m.quit(Duration::from_secs(5)));
}
