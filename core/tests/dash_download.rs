//! DASH streams downloaded from the list (SPECIFICATION.md §15), on the streams in `tests/media` (made by
//! `media/make-dash.sh`), served by hd-mock: one file per quality with its index as Facebook serves reels, and
//! ffmpeg's two template layouts. Where ffmpeg is installed (this Mac's; the app never uses it), each joined file must
//! decode to the same pictures and sound as ffmpeg's own join of the same video and sound.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use hd_core::manager::{Manager, NewDownload, Status};
use hd_mock::{MockFile, MockServer};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/media")
}

fn serve(s: &MockServer, name: &str) {
    for entry in std::fs::read_dir(fixtures().join(name)).unwrap() {
        let path = entry.unwrap().path();
        s.put(&format!("/{name}/{}", path.file_name().unwrap().to_string_lossy()), MockFile::new(std::fs::read(&path).unwrap()));
    }
}

/// Downloads the stream at `link` through the list; returns the test's folder (removed when it's dropped, so kept
/// while the file is looked at) and the joined file in it.
fn download(test: &str, link: String) -> (common::Scratch, PathBuf) {
    let dir = common::Scratch::new(&format!("dash-{test}"));
    std::fs::create_dir_all(dir.join("files")).unwrap();
    let m = Manager::open(&dir).unwrap();
    let id = m.add(NewDownload { url: link, folder: dir.join("files"), connections: 4, start: true, ..NewDownload::default() });
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        let row = m.rows().into_iter().find(|r| r.id == id).unwrap();
        match row.status {
            Status::Finished => break,
            Status::Failed(e) => panic!("{test} failed: {e}"),
            _ => assert!(Instant::now() < until, "{test} never finished"),
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let path = m.rows().into_iter().find(|r| r.id == id).unwrap().path.unwrap();
    assert!(m.quit(Duration::from_secs(5)));
    drop(m);
    (dir, path)
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
fn one_file_per_quality_as_facebook_serves_reels() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    serve(&s, "dash-ondemand");
    let (_dir, path) = download("ondemand", s.url("/dash-ondemand/manifest.mpd"));
    assert_eq!(path.file_name().unwrap(), "dash-ondemand.mp4");
    assert!(!path.with_file_name("dash-ondemand.mp4.hdparts").exists());
    // The 80x45 quality's file doesn't exist: the best one was taken, never touched.
    let base = fixtures().join("dash-ondemand");
    same_as_ffmpeg(&path, &base.join("video.mp4"), &base.join("audio.mp4"));
}

#[test]
fn numbered_pieces_with_a_timeline() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    serve(&s, "dash-template");
    let (_dir, path) = download("template", s.url("/dash-template/manifest.mpd"));
    // ffmpeg's reference: its own join of the same pieces (init + chunks per track).
    let dir = path.parent().unwrap();
    let cat = |stream: &str, out: &str| {
        let base = fixtures().join("dash-template");
        let mut bytes = std::fs::read(base.join(format!("init-stream{stream}.m4s"))).unwrap();
        for n in 1.. {
            let Ok(chunk) = std::fs::read(base.join(format!("chunk-stream{stream}-{n:05}.m4s"))) else { break };
            bytes.extend(chunk);
        }
        std::fs::write(dir.join(out), bytes).unwrap();
        dir.join(out)
    };
    same_as_ffmpeg(&path, &cat("0", "v.mp4"), &cat("1", "a.mp4"));
}

#[test]
fn numbered_pieces_of_a_fixed_length() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    serve(&s, "dash-number");
    let (_dir, path) = download("number", s.url("/dash-number/manifest.mpd"));
    let bytes = std::fs::read(&path).unwrap();
    let kinds: Vec<[u8; 4]> = hd_core::media::mp4::boxes(&bytes).into_iter().map(|(k, _)| k).collect();
    assert_eq!(kinds, [*b"ftyp", *b"moov", *b"mdat"]);
}
