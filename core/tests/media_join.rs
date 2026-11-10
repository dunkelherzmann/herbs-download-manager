//! Joining HLS pieces into MP4 (SPECIFICATION.md §15), on the streams in `tests/media` (made by `media/make.sh`).
//! Where ffmpeg is installed (this Mac's Homebrew one; the app never uses it), each MP4 is held to ffmpeg's own join of
//! the same stream (`-c copy`, no re-encoding): every decoded frame and sound block the same, at the same times. That proves nothing was
//! lost or changed, the "as good as ffmpeg" bar Herbert set (enhancements.md).

use std::path::{Path, PathBuf};
use std::process::Command;

use hd_core::media::hls::{self, Key, Playlist};
use hd_core::media::join::{Joiner, decrypt_aes128};
use reqwest::Url;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/media")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hd-media-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn read(url: &Url) -> Vec<u8> {
    std::fs::read(url.to_file_path().unwrap()).unwrap()
}

/// Reads a media playlist into the joiner, as the download does once every piece is in.
fn add_playlist(joiner: &mut Joiner, url: &Url) {
    let text = String::from_utf8(read(url)).unwrap();
    let Playlist::Media { segments, ended } = hls::parse(&text, url).unwrap() else { panic!("a media playlist") };
    assert!(ended);
    let source = joiner.source();
    let mut map = None;
    for s in segments {
        if s.map.is_some() && s.map != map {
            joiner.init(source, &read(&s.map.as_ref().unwrap().0)).unwrap();
            map = s.map.clone();
        }
        let mut data = read(&s.url);
        if let Key::Aes128 { url, iv } = &s.key {
            let key: [u8; 16] = read(url).try_into().unwrap();
            data = decrypt_aes128(&data, &key, &iv.unwrap_or_else(|| hls::sequence_iv(s.sequence))).unwrap();
        }
        joiner.piece(source, &data, s.discontinuity).unwrap();
    }
}

/// Joins `name`'s playlist in a scratch folder of the test's own (`test`), so tests running at once never share one.
fn join(test: &str, name: &str, playlist: &str) -> PathBuf {
    let dir = scratch(test);
    let url = Url::from_file_path(fixtures().join(name).join(playlist)).unwrap();
    let mut joiner = Joiner::new(&dir.join("spool")).unwrap();
    let text = String::from_utf8(read(&url)).unwrap();
    match hls::parse(&text, &url).unwrap() {
        p @ Playlist::Master { .. } => {
            let video = p.best_variant().unwrap().clone();
            add_playlist(&mut joiner, &video.url);
            if let Some(sound) = p.audio_for(&video) {
                add_playlist(&mut joiner, sound.url.as_ref().unwrap());
            }
        }
        Playlist::Media { .. } => add_playlist(&mut joiner, &url),
    }
    let out = dir.join("joined.mp4");
    joiner.finish(&out).unwrap();
    out
}

fn ffmpeg() -> Option<PathBuf> {
    ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg", "/usr/bin/ffmpeg"].iter().map(PathBuf::from).find(|p| p.exists())
}

/// Every decoded frame and sound block: (stream, presentation time in seconds, size, checksum), per stream.
fn decoded(ffmpeg: &Path, input: &Path) -> Vec<Vec<(f64, String)>> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error"]);
    if input.extension().is_some_and(|e| e == "m3u8") {
        cmd.args(["-allowed_extensions", "ALL", "-protocol_whitelist", "file,crypto,data"]);
    }
    let out = cmd
        .arg("-i")
        .arg(input)
        .args(["-map", "0:v?", "-map", "0:a?", "-f", "framemd5", "-"])
        .output()
        .unwrap();
    assert!(out.status.success(), "ffmpeg: {}", String::from_utf8_lossy(&out.stderr));
    let mut tb: Vec<f64> = Vec::new();
    let mut streams: Vec<Vec<(f64, String)>> = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some(rest) = line.strip_prefix("#tb ") {
            let (_, frac) = rest.split_once(": ").unwrap();
            let (n, d) = frac.split_once('/').unwrap();
            tb.push(n.parse::<f64>().unwrap() / d.parse::<f64>().unwrap());
            streams.push(Vec::new());
            continue;
        }
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').map(str::trim).collect();
        let s: usize = f[0].parse().unwrap();
        let pts: f64 = f[2].parse::<f64>().unwrap() * tb[s];
        streams[s].push((pts, format!("{} {}", f[4], f[5])));
    }
    streams
}

/// Ours against ffmpeg's reading of the original: the same frames in the same order, the same spacing in time.
fn same_as_ffmpeg(name: &str, playlist: &str) {
    let joined = join(name, name, playlist);
    let Some(ffmpeg) = ffmpeg() else {
        eprintln!("no ffmpeg here: {name} joined but not compared");
        return;
    };
    let ours = decoded(&ffmpeg, &joined);
    // ffmpeg's own join, copying without re-encoding: the alternative to ours.
    let reference = joined.with_file_name("ffmpeg.mp4");
    let made = Command::new(&ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-allowed_extensions", "ALL", "-protocol_whitelist", "file,crypto,data", "-i"])
        .arg(fixtures().join(name).join(playlist))
        .args(["-map", "0:v?", "-map", "0:a?", "-c", "copy"])
        .arg(&reference)
        .output()
        .unwrap();
    assert!(made.status.success(), "ffmpeg's join: {}", String::from_utf8_lossy(&made.stderr));
    let theirs = decoded(&ffmpeg, &reference);
    assert_eq!(ours.len(), theirs.len(), "{name}: as many streams");
    for (s, (a, b)) in ours.iter().zip(&theirs).enumerate() {
        assert_eq!(a.len(), b.len(), "{name}: stream {s} has {} frames, ffmpeg's {}", a.len(), b.len());
        let hashes = |v: &Vec<(f64, String)>| v.iter().map(|(_, h)| h.clone()).collect::<Vec<_>>();
        let (ha, hb) = (hashes(a), hashes(b));
        if let Some(i) = ha.iter().zip(&hb).position(|(x, y)| x != y) {
            let same_elsewhere = ha.iter().filter(|h| hb.contains(h)).count();
            panic!("{name}: stream {s}'s frame {i} of {} differs ({} vs ffmpeg's {}); {same_elsewhere} of ours appear in ffmpeg's",
                   ha.len(), ha[i], hb[i]);
        }
        // Times relative to the stream's start, within a millisecond.
        let (a0, b0) = (a[0].0, b[0].0);
        for (i, ((ta, _), (tb, _))) in a.iter().zip(b).enumerate() {
            assert!(((ta - a0) - (tb - b0)).abs() < 0.001, "{name}: stream {s} frame {i} at {} s, ffmpeg's at {} s", ta - a0, tb - b0);
        }
    }
    // And the streams start together as they did (sound against picture).
    if ours.len() == 2 {
        let offset = |v: &Vec<Vec<(f64, String)>>| v[1][0].0 - v[0][0].0;
        assert!((offset(&ours) - offset(&theirs)).abs() < 0.001, "{name}: sound moved against the picture: {} s, ffmpeg {} s", offset(&ours), offset(&theirs));
    }
}

#[test]
fn h264_and_aac_in_ts_pieces() {
    same_as_ffmpeg("h264-ts", "index.m3u8");
}

#[test]
fn h264_and_aac_in_mp4_fragments() {
    same_as_ffmpeg("h264-fmp4", "index.m3u8");
}

#[test]
fn hevc_in_ts_pieces() {
    same_as_ffmpeg("hevc-ts", "index.m3u8");
}

#[test]
fn mp3_sound_in_ts_pieces() {
    same_as_ffmpeg("mp3-ts", "index.m3u8");
}

#[test]
fn a_separate_sound_track_joins_the_video() {
    same_as_ffmpeg("split", "master.m3u8");
}

#[test]
fn aes_128_encrypted_pieces() {
    same_as_ffmpeg("aes", "index.m3u8");
}

#[test]
fn the_joined_file_starts_with_its_index() {
    let joined = join("index-first", "h264-ts", "index.m3u8");
    let bytes = std::fs::read(&joined).unwrap();
    let kinds: Vec<[u8; 4]> = hd_core::media::mp4::boxes(&bytes).into_iter().map(|(k, _)| k).collect();
    assert_eq!(kinds, [*b"ftyp", *b"moov", *b"mdat"], "moov before mdat: it plays before it's read to the end");
}
