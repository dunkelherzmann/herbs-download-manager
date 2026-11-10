//! Downloading an HLS stream (SPECIFICATION.md §15): the playlist, its chosen quality and separate sound track, every
//! piece (several at once, within the speed limits, retried like a download's source), then one MP4 joined from them.
//! The pieces wait in `<name>.hdparts` beside the file, so a paused or interrupted stream continues where it was.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use reqwest::Url;

use super::hls::{self, Key, Playlist, Segment};
use super::join::{Joiner, Written, decrypt_aes128};
use super::JoinError;
use crate::limit::Limiter;
use crate::transfer::{self, Control, Outcome, Progress};

/// What a stream download needs.
#[derive(Clone)]
pub struct StreamRequest {
    pub url: String,
    pub folder: PathBuf,
    /// The file's name ("video.mp4").
    pub name: String,
    /// Pieces fetched at once.
    pub connections: u32,
    pub retries: u32,
    pub limits: Vec<Arc<Limiter>>,
    pub vault: Arc<crate::vault::Vault>,
    /// The quality and sound track to take from a master playlist.
    pub choice: super::StreamChoice,
}

/// The most pieces fetched at once: servers that stream rarely like more from one viewer.
const MAX_AT_ONCE: u32 = 6;

/// Whether a link is a stream's playlist or manifest, by its address (`.m3u8`, `.mpd`).
pub fn is_stream_link(url: &str) -> bool {
    Url::parse(url).is_ok_and(|u| {
        let path = u.path().to_ascii_lowercase();
        path.ends_with(".m3u8") || path.ends_with(".mpd")
    })
}

/// A stream's file name from its link: the playlist's name as `.mp4`, or the folder it's in when the playlist has a
/// generic name ("index", "master", "playlist"…), else the site's name.
pub fn stream_name(url: &str) -> String {
    let Ok(u) = Url::parse(url) else { return "video.mp4".into() };
    let segments: Vec<String> = u
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).map(|p| percent_decode(p)).collect())
        .unwrap_or_default();
    let generic = |s: &str| {
        let stem = s.rsplit_once('.').map(|(a, _)| a).unwrap_or(s).to_ascii_lowercase();
        ["index", "master", "playlist", "prog_index", "chunklist", "manifest", "video", "stream", "main", "media"]
            .iter()
            .any(|g| stem == *g || stem.starts_with(&format!("{g}_")) || stem.starts_with(&format!("{g}-")))
            || stem.chars().all(|c| c.is_ascii_digit() || c == 'p')
    };
    let stem_of = |s: &str| s.rsplit_once('.').map(|(a, _)| a.to_string()).unwrap_or_else(|| s.to_string());
    let pick = segments
        .iter()
        .rev()
        .map(|s| stem_of(s))
        .find(|s| !s.is_empty() && !generic(s) && s.len() <= 80)
        .unwrap_or_else(|| u.host_str().unwrap_or("video").trim_start_matches("www.").to_string());
    let clean: String = pick.chars().map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { '_' } else { c }).collect();
    format!("{}.mp4", clean.trim())
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(hex) = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok())
            && let Ok(b) = u8::from_str_radix(hex, 16)
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Where a stream's pieces wait while it downloads.
pub fn parts_folder(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".hdparts");
    dest.with_file_name(name)
}

/// One file to fetch: a piece, or an init section.
#[derive(Clone)]
struct Piece {
    url: Url,
    range: Option<(u64, u64)>,
    /// Its file in the parts folder.
    file: String,
}

enum Fetch {
    Done,
    Stopped(Outcome),
    Failed(String),
}

/// Downloads the stream into `req.folder/req.name`.
pub async fn download(client: &reqwest::Client, req: StreamRequest, progress: Arc<Progress>, control: Control) -> Outcome {
    let stop = control.begin();
    *progress.name.lock().unwrap() = Some(req.name.clone());
    // Pausing keeps the pieces already in (a live recording is saved), so Pause never warns of starting over.
    progress.resumable.store(true, Ordering::SeqCst);
    match run(client, &req, &progress, &stop).await {
        Ok(outcome) => outcome,
        Err(reason) => transfer::stop_outcome(&stop).unwrap_or(Outcome::Failed(reason)),
    }
}

async fn text(client: &reqwest::Client, url: &Url, vault: &crate::vault::Vault) -> Result<String, String> {
    // A manifest the browser extension found in a page, saved by the app (§15).
    if url.scheme() == "file" {
        let path = url.to_file_path().map_err(|_| "Not a valid link".to_string())?;
        return std::fs::read_to_string(&path).map_err(|e| format!("Can't read {}: {e}", path.display()));
    }
    let response = transfer::signed(client.get(url.clone()), url, vault)
        .send()
        .await
        .map_err(|e| transfer::error_text(&e))?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(transfer::LOGIN_NEEDED.into());
    }
    if !status.is_success() {
        return Err(format!("{} {}", status.as_u16(), status.canonical_reason().unwrap_or("")).trim().to_string());
    }
    response.text().await.map_err(|e| transfer::error_text(&e))
}

/// One track of a stream: an HLS playlist, or pieces listed by a DASH manifest.
enum Track {
    Hls(Url),
    Listed(Vec<Segment>),
}

/// A DASH manifest's best video and its best sound, as pieces like HLS's (an init section, then MP4 fragments).
async fn dash_tracks(client: &reqwest::Client, url: &Url, text: &str, sound_only: bool, vault: &crate::vault::Vault) -> Result<Vec<Track>, String> {
    let m = super::dash::parse(text, url)?;
    if m.live {
        return Err("This is a live DASH stream: recording those isn't supported yet".into());
    }
    let mut tracks = Vec::new();
    // Sound only: just the sound's pieces (the picture's too only when the manifest has no separate sound, and it's
    // left out when they're joined).
    let wanted = match (sound_only, m.best_audio()) {
        (true, Some(audio)) => vec![Some(audio)],
        _ => vec![m.best_video(), m.best_audio()],
    };
    for rep in wanted.into_iter().flatten() {
        tracks.push(Track::Listed(dash_pieces(client, rep, vault).await?));
    }
    if tracks.is_empty() {
        return Err("This stream is protected (DRM): it can't be saved".into());
    }
    Ok(tracks)
}

/// A DASH quality's pieces. One file per quality (`SegmentBase`) is read through its index (`sidx`): each
/// subsegment is a piece, after the init section.
async fn dash_pieces(client: &reqwest::Client, rep: &super::dash::Representation, vault: &crate::vault::Vault) -> Result<Vec<Segment>, String> {
    use super::dash::Addressing;
    // DASH ranges are inclusive (first-last); HLS's are (length, offset).
    let hls_range = |r: (u64, u64)| (r.1 - r.0 + 1, r.0);
    let piece = |url: Url, range: Option<(u64, u64)>, map: Option<(Url, Option<(u64, u64)>)>, n: usize| Segment {
        url,
        duration: 0.0,
        range: range.map(hls_range),
        sequence: n as u64,
        key: Key::None,
        map,
        discontinuity: false,
    };
    match &rep.addressing {
        Addressing::Pieces { init, media } => {
            let map = init.clone().map(|(u, r)| (u, r.map(hls_range)));
            Ok(media.iter().enumerate().map(|(n, (u, r))| piece(u.clone(), *r, map.clone(), n)).collect())
        }
        Addressing::Base { url, init, index: Some((first, last)) } => {
            let bytes = fetch_bytes(client, url, Some((last - first + 1, *first)), vault).await?;
            let subsegments = super::dash::sidx_ranges(&bytes, *first).ok_or("The video's index can't be read")?;
            let init = init.unwrap_or((0, first.saturating_sub(1)));
            let map = Some((url.clone(), Some(hls_range(init))));
            Ok(subsegments.into_iter().enumerate().map(|(n, r)| piece(url.clone(), Some(r), map.clone(), n)).collect())
        }
        // No index: the whole file is one piece after its init section (the file itself when none is given).
        Addressing::Base { url, init, index: None } => {
            let map = Some((url.clone(), init.map(hls_range)));
            Ok(vec![piece(url.clone(), None, map, 0)])
        }
    }
}

/// A media playlist's pieces and whether it has ended (a live stream hasn't), after checking it can be downloaded.
async fn media_playlist(client: &reqwest::Client, url: &Url, vault: &crate::vault::Vault) -> Result<(Vec<Segment>, bool), String> {
    match hls::parse(&text(client, url, vault).await?, url)? {
        Playlist::Media { segments, ended } => {
            if segments.iter().any(|s| matches!(s.key, Key::Protected(_))) {
                return Err("This stream is protected (DRM): it can't be saved".into());
            }
            Ok((segments, ended))
        }
        Playlist::Master { .. } => Err("The stream's playlist lists other playlists only".into()),
    }
}

/// A piece's file: its source and media sequence number, so a live playlist's moving window names each piece once.
fn piece_file(source: usize, sequence: u64) -> String {
    format!("{source}-{sequence:012}")
}

/// Waits `seconds`, or less when the download is told to stop.
async fn wait(seconds: f64, stop: &tokio::sync::watch::Receiver<u8>) {
    let mut stop = stop.clone();
    let _ = tokio::time::timeout(Duration::from_secs_f64(seconds.clamp(1.0, 10.0)), stop.changed()).await;
}

async fn run(client: &reqwest::Client, req: &StreamRequest, progress: &Arc<Progress>, stop: &tokio::sync::watch::Receiver<u8>) -> Result<Outcome, String> {
    let url = Url::parse(&req.url).map_err(|_| "Not a valid link".to_string())?;
    // The tracks: HLS playlists (the chosen quality and its sound, read again each round, for live streams), or a DASH manifest's best video and its
    // sound, whose pieces are known at once.
    let first = text(client, &url, &req.vault).await?;
    let mut playlists = Vec::new();
    let sound_only = req.choice.sound_only;
    if first.contains("<MPD") {
        playlists = dash_tracks(client, &url, &first, sound_only, &req.vault).await?;
    } else {
        match hls::parse(&first, &url)? {
            p @ Playlist::Master { .. } => {
                let video = p.pick_variant(req.choice.quality).ok_or("The stream lists no quality to download")?.clone();
                let sound = p.audio_in(&video, &req.choice.sound).and_then(|r| r.url.clone());
                match (sound_only, sound) {
                    // Sound only, as its own playlist: the picture's isn't fetched at all.
                    (true, Some(sound)) => playlists.push(Track::Hls(sound)),
                    // Sound only, inside the variants' pieces: a sound-only variant when the stream lists one,
                    // else the chosen quality's pieces, whose picture is left out when they're joined.
                    (true, None) => playlists.push(Track::Hls(p.sound_variant().unwrap_or(&video).url.clone())),
                    (false, sound) => {
                        playlists.push(Track::Hls(video.url.clone()));
                        playlists.extend(sound.map(Track::Hls));
                    }
                }
            }
            Playlist::Media { .. } => playlists.push(Track::Hls(url.clone())),
        }
    }
    let dest = req.folder.join(&req.name);
    let parts = parts_folder(&dest);
    std::fs::create_dir_all(&parts).map_err(|e| format!("Can't make {}: {e}", parts.display()))?;

    // Every piece seen, per source, in order; a live playlist's window moves on, so new ones are added each round.
    let mut sources: Vec<Vec<Segment>> = vec![Vec::new(); playlists.len()];
    let mut inits: HashMap<(usize, Url, Option<(u64, u64)>), String> = HashMap::new();
    let mut keys: HashMap<Url, Option<[u8; 16]>> = HashMap::new();
    let mut recording = false;
    loop {
        let mut ended = true;
        let mut wanted = Vec::new();
        let mut round_seconds: f64 = 6.0;
        for (si, track) in playlists.iter().enumerate() {
            let (segments, done) = match track {
                Track::Hls(playlist) => media_playlist(client, playlist, &req.vault).await?,
                Track::Listed(segments) => (segments.clone(), true),
            };
            ended &= done;
            if let Some(last) = segments.last() {
                round_seconds = round_seconds.min(last.duration.max(1.0));
            }
            let seen = sources[si].last().map(|s| s.sequence);
            for s in segments.into_iter().filter(|s| seen.is_none_or(|n| s.sequence > n)) {
                if let Some((map_url, map_range)) = &s.map {
                    let n = inits.len();
                    inits.entry((si, map_url.clone(), *map_range)).or_insert_with(|| {
                        let file = format!("{si}-init-{n}");
                        wanted.push(Piece { url: map_url.clone(), range: *map_range, file: file.clone() });
                        file
                    });
                }
                if let Key::Aes128 { url, .. } = &s.key {
                    keys.entry(url.clone()).or_insert(None);
                }
                wanted.push(Piece { url: s.url.clone(), range: s.range, file: piece_file(si, s.sequence) });
                sources[si].push(s);
            }
        }
        recording |= !ended;
        for (key_url, key) in keys.iter_mut().filter(|(_, k)| k.is_none()) {
            let bytes = fetch_bytes(client, key_url, None, &req.vault).await?;
            *key = Some(bytes.as_slice().try_into().map_err(|_| "The stream's key isn't 16 bytes".to_string())?);
        }
        let total: usize = sources.iter().map(Vec::len).sum::<usize>() + inits.len();
        match fetch_all(client, req, progress, stop, &parts, wanted, total, recording).await {
            Ok(()) => {}
            // Pausing a live recording ends it: what was recorded is saved.
            Err(Halt::Stopped(Outcome::Paused)) if recording => break,
            Err(Halt::Stopped(outcome)) => return Ok(outcome),
            Err(Halt::Failed(reason)) => return Err(reason),
        }
        if ended {
            break;
        }
        // A live stream: look for new pieces about once a piece's length from now.
        wait(round_seconds, stop).await;
        match transfer::stop_outcome(stop) {
            None => {}
            Some(Outcome::Paused) => break,
            Some(other) => return Ok(other),
        }
    }

    // Every piece is in (or the recording ended): join them.
    progress.finishing.store(true, Ordering::SeqCst);
    let (dest2, parts2) = (dest.clone(), parts.clone());
    let joined = tokio::task::spawn_blocking(move || join_parts(&sources, &inits, &keys, &parts2, &dest2, sound_only))
        .await
        .map_err(|e| format!("Joining the pieces stopped: {e}"))??;
    progress.size.store(std::fs::metadata(&joined).map(|m| m.len()).unwrap_or(0), Ordering::SeqCst);
    let _ = std::fs::remove_dir_all(&parts);
    Ok(Outcome::Finished(joined))
}

/// Why fetching the pieces stopped early.
enum Halt {
    Stopped(Outcome),
    Failed(String),
}

/// The pieces in the parts folder: how many, and their bytes.
fn count_parts(parts: &Path) -> (usize, u64) {
    std::fs::read_dir(parts)
        .map(|d| {
            d.flatten()
                .filter(|e| !e.file_name().to_string_lossy().ends_with(".tmp") && e.file_name() != "spool")
                .filter_map(|e| e.metadata().ok())
                .fold((0, 0), |(n, b), m| (n + 1, b + m.len()))
        })
        .unwrap_or((0, 0))
}

/// Fetches the pieces not already in the parts folder, several at once.
#[allow(clippy::too_many_arguments)]
async fn fetch_all(
    client: &reqwest::Client,
    req: &StreamRequest,
    progress: &Arc<Progress>,
    stop: &tokio::sync::watch::Receiver<u8>,
    parts: &Path,
    wanted: Vec<Piece>,
    total: usize,
    recording: bool,
) -> Result<(), Halt> {
    // What's already here from an earlier run counts as downloaded.
    let (here, missing): (Vec<Piece>, Vec<Piece>) = wanted.into_iter().partition(|p| parts.join(&p.file).is_file());
    let have: u64 = here.iter().filter_map(|p| std::fs::metadata(parts.join(&p.file)).ok()).map(|m| m.len()).sum();
    progress.downloaded.fetch_add(have, Ordering::SeqCst);
    let at_once = req.connections.clamp(1, MAX_AT_ONCE) as usize;
    let semaphore = Arc::new(tokio::sync::Semaphore::new(at_once));
    let mut tasks = tokio::task::JoinSet::new();
    for piece in missing {
        let (client, req, progress, stop, parts, semaphore) = (client.clone(), req.clone(), progress.clone(), stop.clone(), parts.to_path_buf(), semaphore.clone());
        tasks.spawn(async move {
            let _slot = semaphore.acquire_owned().await;
            progress.active.fetch_add(1, Ordering::SeqCst);
            let result = fetch_piece(&client, &piece, &req, &progress, &stop, &parts).await;
            progress.active.fetch_sub(1, Ordering::SeqCst);
            result
        });
    }
    let mut failure = None;
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(Fetch::Done) => {
                // The size, estimated from the pieces so far (a live recording's keeps growing: none).
                if !recording {
                    let (n, bytes) = count_parts(parts);
                    if n > 0 {
                        progress.size.store(bytes.saturating_mul(total as u64) / n as u64, Ordering::SeqCst);
                    }
                }
            }
            Ok(Fetch::Stopped(outcome)) => {
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
                return Err(Halt::Stopped(outcome));
            }
            Ok(Fetch::Failed(reason)) => {
                failure.get_or_insert(reason);
                tasks.abort_all();
            }
            Err(_) => {}
        }
    }
    if let Some(outcome) = transfer::stop_outcome(stop) {
        return Err(Halt::Stopped(outcome));
    }
    match failure {
        Some(reason) => Err(Halt::Failed(reason)),
        None => Ok(()),
    }
}

async fn fetch_bytes(client: &reqwest::Client, url: &Url, range: Option<(u64, u64)>, vault: &crate::vault::Vault) -> Result<Vec<u8>, String> {
    let mut rb = transfer::signed(client.get(url.clone()), url, vault);
    if let Some((len, off)) = range {
        rb = rb.header(reqwest::header::RANGE, format!("bytes={off}-{}", off + len.max(1) - 1));
    }
    let response = rb.send().await.map_err(|e| transfer::error_text(&e))?;
    if !response.status().is_success() {
        return Err(format!("{} {}", response.status().as_u16(), response.status().canonical_reason().unwrap_or("")).trim().to_string());
    }
    response.bytes().await.map(|b| b.to_vec()).map_err(|e| transfer::error_text(&e))
}

/// Fetches one piece into the parts folder (a temporary name, then its own), retrying like a download's source.
async fn fetch_piece(client: &reqwest::Client, piece: &Piece, req: &StreamRequest, progress: &Progress, stop: &tokio::sync::watch::Receiver<u8>, parts: &Path) -> Fetch {
    let mut tries = 0;
    loop {
        if let Some(outcome) = transfer::stop_outcome(stop) {
            return Fetch::Stopped(outcome);
        }
        let mut got = 0u64;
        let attempt: Result<Vec<u8>, (bool, String)> = async {
            let mut rb = transfer::signed(client.get(piece.url.clone()), &piece.url, &req.vault);
            if let Some((len, off)) = piece.range {
                rb = rb.header(reqwest::header::RANGE, format!("bytes={off}-{}", off + len.max(1) - 1));
            }
            let mut response = tokio::time::timeout(Duration::from_secs(30), rb.send())
                .await
                .map_err(|_| (true, "The server didn't answer".to_string()))?
                .map_err(|e| (true, transfer::error_text(&e)))?;
            let status = response.status();
            if !status.is_success() {
                let retry = status.is_server_error() || status.as_u16() == 408 || status.as_u16() == 429;
                return Err((retry, format!("{} {}", status.as_u16(), status.canonical_reason().unwrap_or("")).trim().to_string()));
            }
            let mut body = Vec::new();
            loop {
                if transfer::stop_outcome(stop).is_some() {
                    return Err((false, "stopped".into()));
                }
                let chunk = tokio::time::timeout(Duration::from_secs(30), response.chunk())
                    .await
                    .map_err(|_| (true, "The server stopped sending".to_string()))?
                    .map_err(|e| (true, transfer::error_text(&e)))?;
                let Some(chunk) = chunk else { break };
                body.extend_from_slice(&chunk);
                got += chunk.len() as u64;
                progress.downloaded.fetch_add(chunk.len() as u64, Ordering::SeqCst);
                let wait = req.limits.iter().map(|l| l.take(chunk.len())).max().unwrap_or_default();
                if !wait.is_zero() {
                    tokio::time::sleep(wait).await;
                }
            }
            Ok(body)
        }
        .await;
        match attempt {
            Ok(body) => {
                let tmp = parts.join(format!("{}.tmp", piece.file));
                let written = std::fs::write(&tmp, &body).and_then(|_| std::fs::rename(&tmp, parts.join(&piece.file)));
                return match written {
                    Ok(()) => Fetch::Done,
                    Err(e) => Fetch::Failed(format!("Can't save a piece in {}: {e}", parts.display())),
                };
            }
            Err((retry, reason)) => {
                progress.downloaded.fetch_sub(got, Ordering::SeqCst);
                if let Some(outcome) = transfer::stop_outcome(stop) {
                    return Fetch::Stopped(outcome);
                }
                tries += 1;
                if !retry || tries > req.retries {
                    return Fetch::Failed(format!("A piece of the stream failed: {reason}"));
                }
                tokio::time::sleep(Duration::from_secs((1u64 << tries.min(4)).min(10))).await;
            }
        }
    }
}

/// Joins the downloaded pieces into `dest` (MP4); a stream MP4 can't hold (AC-3 sound) becomes one `.ts` file.
fn join_parts(
    sources: &[Vec<Segment>],
    inits: &HashMap<(usize, Url, Option<(u64, u64)>), String>,
    keys: &HashMap<Url, Option<[u8; 16]>>,
    parts: &Path,
    dest: &Path,
    sound_only: bool,
) -> Result<PathBuf, String> {
    let read = |file: &str| std::fs::read(parts.join(file)).map_err(|e| format!("A piece is missing: {e}"));
    let decrypted = |s: &Segment, data: Vec<u8>| -> Result<Vec<u8>, String> {
        match &s.key {
            Key::Aes128 { url, iv } => {
                let key = keys.get(url).copied().flatten().ok_or("The stream's key is missing")?;
                decrypt_aes128(&data, &key, &iv.unwrap_or_else(|| hls::sequence_iv(s.sequence))).map_err(|e| e.to_string())
            }
            _ => Ok(data),
        }
    };
    let hdpart = {
        let mut n = dest.file_name().unwrap_or_default().to_os_string();
        n.push(".hdpart");
        dest.with_file_name(n)
    };
    let attempt = || -> Result<Written, JoinError> {
        let mut joiner = Joiner::new(&parts.join("spool"))?;
        if sound_only {
            joiner.sound_only();
        }
        for (si, segments) in sources.iter().enumerate() {
            let source = joiner.source();
            let mut current_map = None;
            for s in segments {
                if let Some((u, r)) = &s.map
                    && current_map != Some((u.clone(), *r))
                {
                    let file = inits.get(&(si, u.clone(), *r)).ok_or_else(|| JoinError::Damaged("An init section is missing".into()))?;
                    joiner.init(source, &read(file).map_err(JoinError::Damaged)?)?;
                    current_map = Some((u.clone(), *r));
                }
                let data = decrypted(s, read(&piece_file(si, s.sequence)).map_err(JoinError::Damaged)?).map_err(JoinError::Damaged)?;
                joiner.piece(source, &data, s.discontinuity)?;
            }
        }
        joiner.finish(&hdpart)
    };
    match attempt() {
        Ok(written) => {
            // MP3 sound is saved as it came, under its own extension.
            let dest = if written == Written::Mp3 { dest.with_extension("mp3") } else { dest.to_path_buf() };
            std::fs::rename(&hdpart, &dest).map_err(|e| format!("Can't name the file {}: {e}", dest.display()))?;
            Ok(dest)
        }
        Err(JoinError::Unsupported(what)) if sound_only => {
            let _ = std::fs::remove_file(&hdpart);
            Err(format!("The sound can't be saved on its own: {what}"))
        }
        Err(JoinError::Unsupported(what)) => {
            // MP4 can't hold it: the video playlist's pieces, decrypted, one after the other, as MPEG-TS.
            let _ = std::fs::remove_file(&hdpart);
            let ts = dest.with_extension("ts");
            let mut out = Vec::new();
            for s in &sources[0] {
                out.extend(decrypted(s, read(&piece_file(0, s.sequence))?)?);
            }
            std::fs::write(&ts, out).map_err(|e| format!("Can't save {}: {e} ({what})", ts.display()))?;
            Ok(ts)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&hdpart);
            Err(format!("The pieces couldn't be joined: {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_links_and_their_names() {
        assert!(is_stream_link("https://cdn.example.com/v/Big%20Trip/master.m3u8?token=1"));
        assert!(!is_stream_link("https://cdn.example.com/v/clip.mp4"));
        assert_eq!(stream_name("https://cdn.example.com/v/Big%20Trip/master.m3u8"), "Big Trip.mp4");
        assert_eq!(stream_name("https://cdn.example.com/shows/episode-4.m3u8"), "episode-4.mp4");
        assert_eq!(stream_name("https://www.example.com/hls/720p/index.m3u8"), "hls.mp4");
        assert_eq!(stream_name("https://www.example.com/index.m3u8"), "example.com.mp4");
        assert_eq!(parts_folder(Path::new("/d/Trip.mp4")), Path::new("/d/Trip.mp4.hdparts"));
    }
}
