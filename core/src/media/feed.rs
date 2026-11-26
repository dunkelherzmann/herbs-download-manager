//! Streams the browser fetches itself and hands over piece by piece (SPECIFICATION.md §15): YouTube streams a video
//! only through its own player's protocol, so the browser extension downloads it in YouTube's page and sends each
//! piece here. Such a download's link is `hdm-feed:<id>?size=<bytes>`; while it runs, `put` stores each piece of
//! each track (0 the picture, 1 the sound) in its `.hdparts` folder, in order, and `end` says it's complete, after
//! which the tracks are joined into one MP4 like any stream's. A whole file a page holds (a video it decrypted itself,
//! as WhatsApp Web plays them) is fed the same way under an id starting with `WHOLE`: its pieces, all on track 0, are
//! the file as it is, saved without joining.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use super::join::{Joiner, Written};
use crate::transfer::{Outcome, Progress};

/// The scheme of a fed download's link.
pub const SCHEME: &str = "hdm-feed:";

/// How long a fed download waits for the browser's next piece before it fails: the tab was closed, or the browser
/// stopped (a resumed download, which only its page can feed again).
const SILENCE: Duration = Duration::from_secs(90);

/// The start of a fed download's id whose pieces are one whole file, kept as it is.
pub const WHOLE: &str = "file-";

/// The browser's answer when a piece can't be taken.
pub const STOPPED: &str = "stopped";
pub const WAITING: &str = "waiting";

struct Feed {
    parts: PathBuf,
    received: AtomicU64,
    /// What the download asked for (its sound only, the picture's most lines), for the app that feeds it itself.
    choice: super::StreamChoice,
    /// Whether the app took it to feed (`wanted`).
    claimed: std::sync::atomic::AtomicBool,
    /// The size the feeder expects, once it knows (0 when not told).
    expected: AtomicU64,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// Each track's pieces' lengths, in order; its start section (init) apart.
    lengths: Vec<Vec<u64>>,
    has_init: Vec<bool>,
    ended: bool,
    failed: Option<String>,
    stopped: bool,
    last: Option<Instant>,
}

static FEEDS: LazyLock<Mutex<HashMap<String, Arc<Feed>>>> = LazyLock::new(Default::default);

/// Whether a link is a fed download's.
pub fn is_feed_link(url: &str) -> bool {
    url.starts_with(SCHEME)
}

/// A fed download's link for the browser's id and the size it expects.
pub fn link(id: &str, size: Option<u64>) -> String {
    let id: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(64).collect();
    match size {
        Some(s) => format!("{SCHEME}{id}?size={s}"),
        None => format!("{SCHEME}{id}"),
    }
}

fn parse(url: &str) -> (String, Option<u64>) {
    let rest = url.strip_prefix(SCHEME).unwrap_or(url);
    let (id, query) = rest.split_once('?').unwrap_or((rest, ""));
    let size = query.split('&').find_map(|p| p.strip_prefix("size=")).and_then(|s| s.parse().ok());
    (id.to_string(), size)
}

fn track_file(parts: &Path, track: usize, init: bool) -> PathBuf {
    parts.join(if init { format!("feed-{track}-init") } else { format!("feed-{track}") })
}

/// A fed download the app feeds itself: a video it opens out of sight (`super::youtube`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wanted {
    pub feed: String,
    /// The page to open.
    pub page: String,
    pub sound_only: bool,
    /// The picture's most lines (0 for the best).
    pub lines: u32,
}

/// The app's own fed downloads that have started and that nothing feeds yet, each handed out once per start.
pub fn wanted() -> Vec<Wanted> {
    let feeds = FEEDS.lock().unwrap();
    let mut out = Vec::new();
    for (id, feed) in feeds.iter() {
        let Some(video) = super::youtube::video_of_feed(id) else { continue };
        if feed.claimed.swap(true, Ordering::SeqCst) {
            continue;
        }
        out.push(Wanted { feed: id.clone(), page: super::youtube::watch_url(&video), sound_only: feed.choice.sound_only, lines: feed.choice.quality.unwrap_or(0) });
    }
    out
}

/// The size the feeder will send in all, once it knows (a sound-only download is smaller than the size listed).
pub fn expect(id: &str, size: u64) {
    if let Some(feed) = FEEDS.lock().unwrap().get(id) {
        feed.expected.store(size, Ordering::SeqCst);
    }
}

/// Stores one piece of a running fed download. Errors: `STOPPED` (paused, cancelled or gone: the browser stops),
/// `WAITING` (not started yet: the browser asks again shortly), or what went wrong.
pub fn put(id: &str, track: usize, init: bool, data: &[u8]) -> Result<(), String> {
    if track > 3 {
        return Err("No such track".into());
    }
    let Some(feed) = FEEDS.lock().unwrap().get(id).cloned() else { return Err(WAITING.into()) };
    let mut state = feed.state.lock().unwrap();
    if state.stopped || state.failed.is_some() {
        return Err(STOPPED.into());
    }
    if state.lengths.len() <= track {
        state.lengths.resize(track + 1, Vec::new());
        state.has_init.resize(track + 1, false);
    }
    let path = track_file(&feed.parts, track, init);
    let mut file = std::fs::OpenOptions::new().create(true).append(!init).write(true).truncate(init).open(&path).map_err(|e| format!("Can't keep a piece: {e}"))?;
    file.write_all(data).map_err(|e| format!("Can't keep a piece: {e}"))?;
    if init {
        state.has_init[track] = true;
    } else {
        state.lengths[track].push(data.len() as u64);
    }
    state.last = Some(Instant::now());
    feed.received.fetch_add(data.len() as u64, Ordering::SeqCst);
    Ok(())
}

/// `put` with the piece in base64, as a page sends it.
pub fn put_base64(id: &str, track: usize, init: bool, data: &str) -> Result<(), String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.as_bytes()).map_err(|e| format!("A piece came broken: {e}"))?;
    put(id, track, init, &bytes)
}

/// The browser has sent every piece (`failed`: it couldn't, and why).
pub fn end(id: &str, failed: Option<String>) -> Result<(), String> {
    let Some(feed) = FEEDS.lock().unwrap().get(id).cloned() else { return Err(STOPPED.into()) };
    let mut state = feed.state.lock().unwrap();
    if state.stopped {
        return Err(STOPPED.into());
    }
    match failed {
        Some(reason) => state.failed = Some(reason),
        None => state.ended = true,
    }
    Ok(())
}

/// Runs a fed download into `folder/name`: waits for the browser's pieces, then joins them.
pub async fn run(url: &str, folder: &Path, name: &str, choice: &super::StreamChoice, progress: &Progress, stop: &tokio::sync::watch::Receiver<u8>) -> Outcome {
    let (id, size) = parse(url);
    let dest = folder.join(name);
    let parts = super::download::parts_folder(&dest);
    // A fed download can't continue where it stopped: its pieces come from the page, which starts again.
    let _ = std::fs::remove_dir_all(&parts);
    if let Err(e) = std::fs::create_dir_all(&parts) {
        return Outcome::Failed(format!("Can't make {}: {e}", parts.display()));
    }
    progress.resumable.store(false, Ordering::SeqCst);
    // The page streams it to the app as one connection.
    progress.allowed.store(1, Ordering::SeqCst);
    if let Some(s) = size {
        progress.size.store(s, Ordering::SeqCst);
    }
    let feed = Arc::new(Feed {
        parts: parts.clone(),
        received: AtomicU64::new(0),
        choice: choice.clone(),
        claimed: Default::default(),
        expected: AtomicU64::new(0),
        state: Mutex::new(State { last: Some(Instant::now()), ..Default::default() }) });
    FEEDS.lock().unwrap().insert(id.clone(), feed.clone());
    let forget = || {
        FEEDS.lock().unwrap().remove(&id);
    };
    loop {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let received = feed.received.load(Ordering::SeqCst);
        if received > 0 && progress.active.swap(1, Ordering::SeqCst) == 0 {
            progress.peak.fetch_max(1, Ordering::SeqCst);
        }
        progress.downloaded.store(received, Ordering::SeqCst);
        let expected = feed.expected.load(Ordering::SeqCst);
        if expected > 0 {
            progress.size.store(expected, Ordering::SeqCst);
        }
        // Paused or cancelled: the feed stays known as stopped, so the browser's next piece is told to stop.
        if let Some(outcome) = crate::transfer::stop_outcome(stop) {
            feed.state.lock().unwrap().stopped = true;
            let _ = std::fs::remove_dir_all(&parts);
            return outcome;
        }
        let state = feed.state.lock().unwrap();
        if let Some(reason) = &state.failed {
            let reason = reason.clone();
            drop(state);
            forget();
            let _ = std::fs::remove_dir_all(&parts);
            return Outcome::Failed(reason);
        }
        if state.ended {
            break;
        }
        if state.last.is_some_and(|t| t.elapsed() > SILENCE) {
            drop(state);
            feed.state.lock().unwrap().stopped = true;
            let _ = std::fs::remove_dir_all(&parts);
            return Outcome::Failed("The browser stopped sending it: download it again from its page".into());
        }
    }
    forget();
    progress.active.store(0, Ordering::SeqCst);
    progress.finishing.store(true, Ordering::SeqCst);
    let (lengths, has_init) = {
        let s = feed.state.lock().unwrap();
        (s.lengths.clone(), s.has_init.clone())
    };
    let whole = id.starts_with(WHOLE);
    let joined = tokio::task::spawn_blocking(move || if whole { keep_whole(&parts, &dest, &lengths) } else { join(&parts, &dest, &lengths, &has_init) }).await;
    match joined {
        Ok(Ok(path)) => Outcome::Finished(path),
        Ok(Err(e)) => Outcome::Failed(e),
        Err(e) => Outcome::Failed(format!("The pieces couldn't be joined: {e}")),
    }
}

/// Keeps a whole file's pieces (track 0, already stored in order) as `dest`, then removes the pieces' folder.
fn keep_whole(parts: &Path, dest: &Path, lengths: &[Vec<u64>]) -> Result<PathBuf, String> {
    if lengths.len() != 1 || lengths[0].is_empty() {
        return Err("Nothing was received".into());
    }
    std::fs::rename(track_file(parts, 0, false), dest).map_err(|e| format!("Can't name the file {}: {e}", dest.display()))?;
    let _ = std::fs::remove_dir_all(parts);
    Ok(dest.to_path_buf())
}

/// Joins the stored tracks into `dest` (an `.m4a` when there's only sound), then removes the pieces.
fn join(parts: &Path, dest: &Path, lengths: &[Vec<u64>], has_init: &[bool]) -> Result<PathBuf, String> {
    use std::io::Read;
    let hdpart = {
        let mut n = dest.file_name().unwrap_or_default().to_os_string();
        n.push(".hdpart");
        dest.with_file_name(n)
    };
    let attempt = || -> Result<Written, String> {
        let mut joiner = Joiner::new(&parts.join("spool")).map_err(|e| e.to_string())?;
        let tracks: Vec<usize> = (0..lengths.len()).filter(|&t| !lengths[t].is_empty()).collect();
        if tracks.is_empty() {
            return Err("Nothing was received".into());
        }
        if tracks == [1] {
            joiner.sound_only();
        }
        for &t in &tracks {
            let source = joiner.source();
            if has_init.get(t).copied().unwrap_or(false) {
                let init = std::fs::read(track_file(parts, t, true)).map_err(|e| format!("A start section is missing: {e}"))?;
                joiner.init(source, &init).map_err(|e| e.to_string())?;
            }
            let mut file = std::fs::File::open(track_file(parts, t, false)).map_err(|e| format!("Pieces are missing: {e}"))?;
            for &len in &lengths[t] {
                let mut piece = vec![0u8; len as usize];
                file.read_exact(&mut piece).map_err(|e| format!("A piece is missing: {e}"))?;
                joiner.piece(source, &piece, false).map_err(|e| e.to_string())?;
            }
        }
        joiner.finish(&hdpart).map_err(|e| e.to_string())
    };
    match attempt() {
        Ok(_) => {
            std::fs::rename(&hdpart, dest).map_err(|e| format!("Can't name the file {}: {e}", dest.display()))?;
            let _ = std::fs::remove_dir_all(parts);
            Ok(dest.to_path_buf())
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
    fn links_carry_the_id_and_size() {
        assert_eq!(link("ab-1_2", Some(42)), "hdm-feed:ab-1_2?size=42");
        assert_eq!(link("a/b?c", None), "hdm-feed:abc");
        assert_eq!(parse("hdm-feed:ab?size=42"), ("ab".to_string(), Some(42)));
        assert!(is_feed_link("hdm-feed:x") && !is_feed_link("https://x"));
    }

    #[test]
    fn a_piece_for_a_download_not_running_waits() {
        assert_eq!(put("nobody", 0, false, b"x"), Err(WAITING.into()));
        assert_eq!(end("nobody", None), Err(STOPPED.into()));
    }
}
