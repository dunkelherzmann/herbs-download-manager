//! Joining HLS pieces into MP4 (SPECIFICATION.md §15), on the streams in `tests/media` (made by `media/make.sh`).
//! Where ffmpeg is installed (this Mac's Homebrew one; the app never uses it), each MP4 is held to ffmpeg's own join of
//! the same stream (`-c copy`, no re-encoding): every decoded frame and sound block the same, at the same times. That proves nothing was
//! lost or changed, the "as good as ffmpeg" bar Herbert set (enhancements.md).

use std::path::{Path, PathBuf};
use std::process::Command;

use hd_core::media::hls::{self, Key, Playlist};
use hd_core::media::join::{Joiner, Written, decrypt_aes128};
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
    join_as(test, name, playlist, false).0
}

/// `join`, or with `sound_only` only the sound (from the separate sound playlist when there is one, as the download
/// does): the file and what was written.
fn join_as(test: &str, name: &str, playlist: &str, sound_only: bool) -> (PathBuf, Written) {
    let dir = scratch(test);
    let url = Url::from_file_path(fixtures().join(name).join(playlist)).unwrap();
    let mut joiner = Joiner::new(&dir.join("spool")).unwrap();
    if sound_only {
        joiner.sound_only();
    }
    let text = String::from_utf8(read(&url)).unwrap();
    match hls::parse(&text, &url).unwrap() {
        p @ Playlist::Master { .. } => {
            let video = p.best_variant().unwrap().clone();
            let sound = p.audio_for(&video).and_then(|r| r.url.clone());
            if !(sound_only && sound.is_some()) {
                add_playlist(&mut joiner, &video.url);
            }
            if let Some(sound) = sound {
                add_playlist(&mut joiner, &sound);
            }
        }
        Playlist::Media { .. } => add_playlist(&mut joiner, &url),
    }
    let out = dir.join("joined.mp4");
    let written = joiner.finish(&out).unwrap();
    (out, written)
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

/// Sound only: ours holds just the sound, frame for frame what ffmpeg copies out of the stream with `-vn -c copy`
/// (MP3 sound as an .mp3 file, AAC as MP4 sound).
fn sound_only_same_as_ffmpeg(name: &str, playlist: &str, expected: Written) {
    let (joined, written) = join_as(&format!("{name}-sound"), name, playlist, true);
    assert_eq!(written, expected, "{name}: written as");
    let Some(ffmpeg) = ffmpeg() else {
        eprintln!("no ffmpeg here: {name}'s sound saved but not compared");
        return;
    };
    let ours = decoded(&ffmpeg, &joined);
    let reference = joined.with_file_name(if expected == Written::Mp3 { "ffmpeg.mp3" } else { "ffmpeg.m4a" });
    let made = Command::new(&ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-allowed_extensions", "ALL", "-protocol_whitelist", "file,crypto,data", "-i"])
        .arg(fixtures().join(name).join(playlist))
        .args(["-map", "0:a:0", "-vn", "-c", "copy"])
        .arg(&reference)
        .output()
        .unwrap();
    assert!(made.status.success(), "ffmpeg's copy: {}", String::from_utf8_lossy(&made.stderr));
    let theirs = decoded(&ffmpeg, &reference);
    assert_eq!(ours.len(), 1, "{name}: one stream, the sound");
    assert_eq!(theirs.len(), 1);
    let hashes = |v: &Vec<(f64, String)>| v.iter().map(|(_, h)| h.clone()).collect::<Vec<_>>();
    let (a, b) = (hashes(&ours[0]), hashes(&theirs[0]));
    assert_eq!(a.len(), b.len(), "{name}: as many sound blocks");
    // ffmpeg's .mp3 marks the encoder's lead-in to be skipped, so its first block decodes shorter; every other is
    // the same. (Counted, not listed: a list of hundreds of checksums is unreadable in a failure.)
    let skip = usize::from(expected == Written::Mp3);
    let differ = a.iter().zip(&b).skip(skip).filter(|(x, y)| x != y).count();
    assert_eq!(differ, 0, "{name}: {differ} of {} sound blocks differ", a.len());
}

#[test]
fn sound_only_from_ts_pieces_is_mp4_sound() {
    sound_only_same_as_ffmpeg("h264-ts", "index.m3u8", Written::Mp4);
}

#[test]
fn sound_only_from_mp4_fragments() {
    sound_only_same_as_ffmpeg("h264-fmp4", "index.m3u8", Written::Mp4);
}

#[test]
fn sound_only_in_mp3_is_an_mp3_file() {
    sound_only_same_as_ffmpeg("mp3-ts", "index.m3u8", Written::Mp3);
}

#[test]
fn sound_only_takes_the_separate_sound_track() {
    sound_only_same_as_ffmpeg("split", "master.m3u8", Written::Mp4);
}

#[test]
fn sound_only_of_a_stream_with_no_sound_says_so() {
    let dir = scratch("no-sound");
    let url = Url::from_file_path(fixtures().join("split/0.m3u8")).unwrap();
    let mut joiner = Joiner::new(&dir.join("spool")).unwrap();
    joiner.sound_only();
    add_playlist(&mut joiner, &url);
    let err = joiner.finish(&dir.join("out.m4a")).unwrap_err();
    assert_eq!(err.to_string(), "This stream has no sound to save");
}

/// The subtitles a master offers for `languages`, read from their WebVTT files, as the download does.
fn subtitles_of(url: &Url, languages: &[&str]) -> Vec<hd_core::media::subtitles::Subtitles> {
    let text = String::from_utf8(read(url)).unwrap();
    let p = hls::parse(&text, url).unwrap();
    let video = p.best_variant().unwrap().clone();
    let languages: Vec<String> = languages.iter().map(|l| l.to_string()).collect();
    p.subtitles_in(&video, &languages)
        .into_iter()
        .map(|r| {
            let list = r.url.clone().unwrap();
            let Playlist::Media { segments, .. } = hls::parse(&String::from_utf8(read(&list)).unwrap(), &list).unwrap() else { panic!() };
            let mut subs = hd_core::media::subtitles::Subtitles { language: r.language.clone().unwrap_or_default(), cues: Vec::new(), offset: None };
            for s in segments {
                let (cues, offset) = hd_core::media::subtitles::parse_vtt(&String::from_utf8(read(&s.url)).unwrap()).unwrap();
                subs.cues.extend(cues);
                subs.offset = subs.offset.or(offset);
            }
            subs
        })
        .collect()
}

/// A subtitle track of `file` as SRT, by ffmpeg.
fn srt(ffmpeg: &Path, file: &Path, index: usize) -> String {
    let out = Command::new(ffmpeg).args(["-hide_banner", "-loglevel", "error", "-i"]).arg(file)
        .args(["-map", &format!("0:s:{index}"), "-f", "srt", "-"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn subtitles_join_the_video_as_labelled_tracks() {
    let dir = scratch("subs");
    let master = Url::from_file_path(fixtures().join("subs/master.m3u8")).unwrap();
    let mut joiner = Joiner::new(&dir.join("spool")).unwrap();
    let text = String::from_utf8(read(&master)).unwrap();
    let p = hls::parse(&text, &master).unwrap();
    let video = p.best_variant().unwrap().clone();
    add_playlist(&mut joiner, &video.url);
    add_playlist(&mut joiner, p.audio_for(&video).unwrap().url.as_ref().unwrap());
    let subs = subtitles_of(&master, &["de", "en"]);
    assert_eq!(subs.iter().map(|s| s.language.as_str()).collect::<Vec<_>>(), ["de", "en"], "in S15's order; forced left out");
    joiner.subtitles(subs);
    let out = dir.join("joined.mp4");
    joiner.finish(&out).unwrap();
    let Some(ffmpeg) = ffmpeg() else {
        eprintln!("no ffmpeg here: subtitles joined but not compared");
        return;
    };
    let probe = Command::new(ffmpeg.with_file_name("ffprobe")).args(["-v", "error", "-show_entries", "stream=codec_name:stream_tags=language", "-of", "csv=p=0"])
        .arg(&out).output().unwrap();
    let streams = String::from_utf8_lossy(&probe.stdout).to_string();
    assert_eq!(streams.lines().collect::<Vec<_>>(), ["h264,und", "aac,und", "mov_text,deu", "mov_text,eng"], "{streams}");
    // English carries X-TIMESTAMP-MAP (MPEGTS 133200 is the picture's first time): cue 0.5 s shows 0.5 s into the
    // picture, which starts 23 ms into the file, after the sound. (ffmpeg's own HLS reader isn't the reference here:
    // it drops the first WebVTT piece and mistimes the second.)
    let english = srt(&ffmpeg, &out, 1);
    assert_eq!(english, "1\n00:00:00,523 --> 00:00:02,023\nHello, world\n\n2\n00:00:03,023 --> 00:00:05,023\nSecond line & more\n\n");
    // German has no map: its times count from the stream's start.
    assert_eq!(srt(&ffmpeg, &out, 0), "1\n00:00:01,000 --> 00:00:04,000\nHallo, Welt\n\n");
}
