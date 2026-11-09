//! Downloads one file from one or more sources over up to 32 connections (SPECIFICATION.md §7, §8, §10).
//!
//! A download has sources: the link it was added with and any mirrors serving the same file. The first answer
//! (`Range: bytes=0-`) gives the size, whether ranges work and that source's validator; every other source is checked
//! (`Range: bytes=0-0`) before it gets connections. Connections go to the working source with the fewest
//! connections on its host, and each takes the second half of the largest remaining range (see `segments`). Data
//! goes to `<name>.hdpart`, written in place at its offset, and the file gets its final name once every byte is in.

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use reqwest::header::{ACCEPT_ENCODING, IF_RANGE, RANGE};
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, watch};

use crate::http;
use crate::segments::{Segment, Segments};

/// What to download and how.
#[derive(Clone, Debug, Default)]
pub struct Request {
    pub url: String,
    /// Other links to the same file (M1).
    pub mirrors: Vec<String>,
    pub folder: PathBuf,
    /// The file name; `None` takes it from the server.
    pub name: Option<String>,
    /// Connections for this download (1–32).
    pub connections: u32,
    /// Connections to any one server (1–32).
    pub connections_per_server: u32,
    /// Consecutive failures before a source counts as failed.
    pub retries: u32,
    /// Where the resume state is kept (`None`: not saved).
    pub state_file: Option<PathBuf>,
    /// Speed limits this download shares (all downloads', its own); every received chunk is paid to each.
    pub limits: Vec<Arc<crate::limit::Limiter>>,
}

/// Timings and sizes the spec fixes; tests shorten them.
#[derive(Clone, Debug)]
pub struct Tuning {
    /// No range is split into halves smaller than this.
    pub min_split: u64,
    /// A connection that receives nothing for this long is closed and its range goes back to the pool.
    pub stall_timeout: Duration,
    /// Waits before retrying a failed source: 5 s, 10, 20, 40, then 60 s.
    pub retry_delays: Vec<Duration>,
    /// After a server turns away a connection, the wait before trying one more.
    pub refusal_wait: Duration,
    /// Each connection writes to disk in pieces of this size.
    pub write_buffer: usize,
    /// How often the resume state is saved while downloading.
    pub save_interval: Duration,
    /// How often speeds are measured, mirrors added and slow connections looked for.
    pub tick: Duration,
    /// A connection under a quarter of the average speed for this long hands its range to a faster source.
    pub slow_after: Duration,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            min_split: 256 * 1024,
            stall_timeout: Duration::from_secs(20),
            retry_delays: [5, 10, 20, 40, 60].map(Duration::from_secs).to_vec(),
            refusal_wait: Duration::from_secs(60),
            write_buffer: 256 * 1024,
            save_interval: Duration::from_secs(5),
            tick: Duration::from_secs(1),
            slow_after: Duration::from_secs(15),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Finished(PathBuf),
    Paused,
    Failed(String),
}

/// A source's state, as the Mirrors tab shows it (§7.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceStatus {
    Checking,
    Working,
    Idle,
    Slow,
    Failed(String),
    DifferentFile,
    FileChanged,
    NoResumeSupport,
}

impl SourceStatus {
    /// Never used again in this run.
    fn is_dead(&self) -> bool {
        matches!(
            self,
            SourceStatus::Failed(_) | SourceStatus::DifferentFile | SourceStatus::FileChanged | SourceStatus::NoResumeSupport
        )
    }

    pub fn text(&self) -> String {
        match self {
            SourceStatus::Checking => "Checking".into(),
            SourceStatus::Working => "Working".into(),
            SourceStatus::Idle => "Idle".into(),
            SourceStatus::Slow => "Slow".into(),
            SourceStatus::Failed(reason) => format!("Failed: {reason}"),
            SourceStatus::DifferentFile => "Different file".into(),
            SourceStatus::FileChanged => "File changed".into(),
            SourceStatus::NoResumeSupport => "No resume support".into(),
        }
    }
}

/// One source in the resume state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSource {
    pub url: String,
    /// This server's own ETag or Last-Modified, sent back as `If-Range`.
    pub validator: Option<String>,
    pub enabled: bool,
    pub status: SourceStatus,
}

/// One source, live, for the Mirrors tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceInfo {
    pub url: String,
    pub status: SourceStatus,
    pub enabled: bool,
    pub active: usize,
    /// Bytes per second.
    pub speed: u64,
    pub bytes: u64,
}

/// Live numbers for the UI; read them at any time.
#[derive(Debug, Default)]
pub struct Progress {
    /// Total size, or `u64::MAX` while unknown.
    pub size: AtomicU64,
    pub downloaded: AtomicU64,
    /// Connections receiving data now.
    pub active: AtomicUsize,
    /// The most connections that received data at once.
    pub peak: AtomicUsize,
    pub resumable: AtomicBool,
    /// Connections allowed now.
    pub allowed: AtomicUsize,
    /// The ranges and who works on them, refreshed every tick (the Download window's segments bar).
    pub segments: Mutex<Vec<Segment>>,
    /// One line per connection in use, refreshed every tick (the Connections tab).
    pub connections: Mutex<Vec<ConnInfo>>,
    pub name: Mutex<Option<String>>,
    pub sources: Mutex<Vec<SourceInfo>>,
    /// Every byte is in: the file is being flushed to disk and given its name.
    pub finishing: AtomicBool,
}

/// A connection as the Connections tab shows it (§6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnInfo {
    pub id: usize,
    /// The source's link, empty while it has none.
    pub source: String,
    /// The range it works on: `start..end`, with bytes up to `pos` written. All 0 when it has none.
    pub start: u64,
    pub end: u64,
    pub pos: u64,
    /// Bytes this connection received.
    pub bytes: u64,
    pub speed: u64,
    /// Connecting, Downloading, Idle, Retrying in 5 s.
    pub state: String,
}

impl Progress {
    pub fn new() -> Arc<Self> {
        let p = Progress::default();
        p.size.store(u64::MAX, Ordering::SeqCst);
        Arc::new(p)
    }

    pub fn size(&self) -> Option<u64> {
        Some(self.size.load(Ordering::SeqCst)).filter(|&s| s != u64::MAX)
    }

    pub fn sources(&self) -> Vec<SourceInfo> {
        self.sources.lock().unwrap().clone()
    }
}

enum Command {
    AddMirrors(Vec<String>),
    Enable(String, bool),
    SetConnections(usize),
    Remove(String),
}

/// Steers a running download from another task or thread: pause it, add mirrors (M2), turn sources on or off.
#[derive(Clone)]
pub struct Control {
    stop: Arc<watch::Sender<u8>>,
    inbox: Arc<Mutex<Vec<Command>>>,
}

/// Connection slots per download; `allowed` says how many are in use.
const MAX_CONNECTIONS: usize = 32;

const RUN: u8 = 0;
const PAUSE: u8 = 1;
const ABORT: u8 = 2;

impl Control {
    pub fn pause(&self) {
        let _ = self.stop.send(PAUSE);
    }

    /// Adds mirrors to the running download; each is checked at once and then gets connections.
    pub fn add_mirrors(&self, urls: Vec<String>) {
        self.inbox.lock().unwrap().push(Command::AddMirrors(urls));
    }

    /// Turns a source on or off; its connections move to the others. The last source on stays on.
    /// Changes how many connections the running download may use (1–32); extra ones finish their write, hand
    /// their range back and close.
    pub fn set_connections(&self, n: u32) {
        self.inbox.lock().unwrap().push(Command::SetConnections(n.clamp(1, 32) as usize));
    }

    /// Removes a mirror from the running download: its connections move to the other sources and it isn't saved.
    /// The original link can only be turned off.
    pub fn remove_source(&self, url: &str) {
        self.inbox.lock().unwrap().push(Command::Remove(url.to_string()));
    }

    pub fn set_source_enabled(&self, url: &str, enabled: bool) {
        self.inbox.lock().unwrap().push(Command::Enable(url.to_string(), enabled));
    }
}

pub fn control() -> Control {
    let (tx, _) = watch::channel(RUN);
    Control { stop: Arc::new(tx), inbox: Arc::new(Mutex::new(Vec::new())) }
}

/// Everything needed to continue a download after a pause, a quit, a crash or a power loss, saved as JSON.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeState {
    /// The link the download was added with; it identifies the download.
    pub url: String,
    /// The final file; the data is in `<dest>.hdpart` until it is complete.
    pub dest: PathBuf,
    pub size: Option<u64>,
    /// False when no source has ranges or the size is unknown: such a download starts over.
    pub resumable: bool,
    pub segments: Vec<Segment>,
    pub sources: Vec<SavedSource>,
}

impl ResumeState {
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let text = std::fs::read(path)?;
        serde_json::from_slice(&text).map_err(std::io::Error::other)
    }

    /// Writes to a temporary file and renames it, so a crash never leaves half a state file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(&tmp, path)
    }
}

/// Adds mirrors to a paused, waiting or failed download (M3); they are checked when it resumes. Mirrors equal to
/// an existing source are ignored. Returns how many were added.
pub fn add_mirrors_to_state(state_file: &Path, urls: &[String]) -> std::io::Result<usize> {
    let mut state = ResumeState::load(state_file)?;
    let mut added = 0;
    for url in urls.iter().map(|u| u.trim()).filter(|u| !u.is_empty()) {
        if reqwest::Url::parse(url).is_ok() && !state.sources.iter().any(|s| same_link(&s.url, url)) {
            state.sources.push(SavedSource { url: url.to_string(), validator: None, enabled: true, status: SourceStatus::Checking });
            added += 1;
        }
    }
    state.save(state_file)?;
    Ok(added)
}

pub(crate) fn same_link(a: &str, b: &str) -> bool {
    match (reqwest::Url::parse(a), reqwest::Url::parse(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

enum Problem {
    /// The connection should go back to the pool and reconnect at once.
    Stalled,
    /// Far slower than the others: its range goes to a faster source.
    Slow,
    /// Its source was turned off.
    Moved,
    /// Paused or aborted.
    Stopped,
    /// A failure that may pass: counted against the source's retries.
    Transient(String),
    /// The server turned away this connection (429, 503).
    Refused,
    /// The file on this server isn't the one being downloaded any more.
    FileChanged,
    /// Retrying won't help (404, 403…).
    Permanent(String),
}

struct Source {
    url: reqwest::Url,
    text: String,
    host: String,
    validator: Mutex<Option<String>>,
    status: Mutex<SourceStatus>,
    enabled: AtomicBool,
    active: AtomicUsize,
    failures: AtomicU32,
    retry_at: Mutex<Option<Instant>>,
    /// When it was last marked Slow; the mark stays at least `slow_after`, so it doesn't flicker.
    slow_since: Mutex<Option<Instant>>,
    bytes: AtomicU64,
    last_bytes: AtomicU64,
    speed: AtomicU64,
    /// Removed while running: off, never saved or shown.
    removed: AtomicBool,
}

impl Source {
    fn new(saved: SavedSource) -> Option<Arc<Source>> {
        let url = reqwest::Url::parse(&saved.url).ok()?;
        let host = format!("{}:{}", url.host_str().unwrap_or(""), url.port_or_known_default().unwrap_or(0));
        Some(Arc::new(Source {
            url,
            text: saved.url,
            host,
            validator: Mutex::new(saved.validator),
            status: Mutex::new(saved.status),
            enabled: AtomicBool::new(saved.enabled),
            active: AtomicUsize::new(0),
            failures: AtomicU32::new(0),
            retry_at: Mutex::new(None),
            slow_since: Mutex::new(None),
            bytes: AtomicU64::new(0),
            last_bytes: AtomicU64::new(0),
            speed: AtomicU64::new(0),
            removed: AtomicBool::new(false),
        }))
    }

    fn status(&self) -> SourceStatus {
        self.status.lock().unwrap().clone()
    }

    fn set_status(&self, status: SourceStatus) {
        *self.status.lock().unwrap() = status;
    }

    fn saved(&self) -> SavedSource {
        SavedSource {
            url: self.text.clone(),
            validator: self.validator.lock().unwrap().clone(),
            enabled: self.enabled.load(Ordering::SeqCst),
            status: self.status(),
        }
    }
}

/// After a server turns connections away: at most `cap` to that host until `until`, then one more.
struct HostLimit {
    cap: usize,
    until: Instant,
}

/// A connection's numbers, for finding slow ones.
#[derive(Default)]
struct ConnStat {
    /// The source it is using, or `usize::MAX`.
    source: AtomicUsize,
    bytes: AtomicU64,
    slow_ticks: AtomicU32,
    handoff: AtomicBool,
    /// Asked to reconnect, so its range moves to a source with fewer connections.
    moving: AtomicBool,
    /// A task is working in this slot.
    running: AtomicBool,
    speed: AtomicU64,
    /// Bytes received (counted as they arrive, before they're written), for its speed.
    received: AtomicU64,
    last_received: AtomicU64,
    /// IDLE, CONNECTING, DOWNLOADING or WAITING.
    phase: AtomicU8,
}

const IDLE: u8 = 0;
const CONNECTING: u8 = 1;
const DOWNLOADING: u8 = 2;
const WAITING: u8 = 3;

enum Pick {
    Use(usize),
    Wait,
    NoneLeft,
}

struct Job {
    /// Connections this download may use now (Control::set_connections changes it).
    allowed: AtomicUsize,
    /// Tasks of connections added while running.
    extra: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    client: reqwest::Client,
    tuning: Tuning,
    retries: u32,
    per_server: usize,
    limits: Vec<Arc<crate::limit::Limiter>>,
    sequential: bool,
    /// What gets saved; `segments` and `sources` are filled in at each save.
    state: ResumeState,
    state_file: Option<PathBuf>,
    file: File,
    segs: Mutex<Segments>,
    sources: RwLock<Vec<Arc<Source>>>,
    hosts: Mutex<HashMap<String, HostLimit>>,
    pick_lock: Mutex<()>,
    refusal_gate: tokio::sync::Mutex<()>,
    conns: Vec<ConnStat>,
    progress: Arc<Progress>,
    changed: Notify,
    inbox: Arc<Mutex<Vec<Command>>>,
    last_reason: Mutex<Option<String>>,
    fatal: Mutex<Option<String>>,
    stop_tx: watch::Sender<u8>,
    stop: watch::Receiver<u8>,
}

/// Runs a download to the end, a pause or a failure.
///
/// With `req.state_file`, the resume state is saved there every `save_interval` while downloading, on pause and on
/// failure, and removed when the download finishes; if the file already holds a state for this link, the download
/// continues from it (or starts over when it can't resume, or when the `.hdpart` file no longer matches).
pub async fn download(client: &reqwest::Client, req: Request, tuning: Tuning, progress: Arc<Progress>, control: Control) -> Outcome {
    control.stop.send_replace(RUN);
    let stop = control.stop.subscribe();
    if reqwest::Url::parse(&req.url).is_err() {
        return Outcome::Failed("Not a valid link".into());
    }
    let max_conns = req.connections.clamp(1, 32) as usize;
    let saved = req.state_file.as_deref().and_then(|p| ResumeState::load(p).ok()).filter(|s| s.url == req.url);

    // The sources: saved ones first (with their validators and states), then the link and mirrors from the request.
    let mut sources: Vec<SavedSource> = saved.as_ref().map(|s| s.sources.clone()).unwrap_or_default();
    for url in std::iter::once(&req.url).chain(req.mirrors.iter()) {
        let url = url.trim();
        if reqwest::Url::parse(url).is_ok() && !sources.iter().any(|s| same_link(&s.url, url)) {
            sources.push(SavedSource { url: url.to_string(), validator: None, enabled: true, status: SourceStatus::Checking });
        }
    }

    // The app shows the sources from the start, while the first answer is awaited.
    *progress.sources.lock().unwrap() = sources
        .iter()
        .map(|s| SourceInfo { url: s.url.clone(), status: s.status.clone(), enabled: s.enabled, active: 0, speed: 0, bytes: 0 })
        .collect();

    // Continue saved ranges when the .hdpart file is still the one they describe.
    if let Some(state) = &saved
        && let Some(size) = state.size
        && state.resumable
        && Segments::valid_for(&state.segments, size)
    {
        let part = part_path(&state.dest);
        let file = std::fs::OpenOptions::new().read(true).write(true).open(&part);
        if let Ok(file) = file
            && file.metadata().is_ok_and(|m| m.len() == size)
        {
            // A source that failed gets another chance; one with a validator was already checked.
            for s in &mut sources {
                s.status = match &s.status {
                    st if st.is_dead() && !matches!(st, SourceStatus::Failed(_)) => st.clone(),
                    _ if s.validator.is_some() => SourceStatus::Working,
                    _ => SourceStatus::Checking,
                };
            }
            let segs = Segments::from_saved(state.segments.clone(), tuning.min_split);
            progress.downloaded.store(segs.downloaded(), Ordering::SeqCst);
            let job = Job::new(client, &req, tuning, state.clone(), sources, file, segs, progress, &control, false, max_conns);
            return run(job, None, max_conns, stop).await;
        }
    }

    // A fresh start: ask the sources in turn for the whole file until one answers. One with ranges and a size is
    // preferred, so a mirror can save a download whose link has no ranges.
    let mut attempt = 0usize;
    let (first_idx, first) = loop {
        let mut plain: Option<(usize, reqwest::Response)> = None;
        let mut reason = None;
        for (i, s) in sources.iter_mut().enumerate() {
            if !s.enabled || s.status.is_dead() {
                continue;
            }
            let Ok(url) = reqwest::Url::parse(&s.url) else { continue };
            let sent = client.get(url).header(RANGE, "bytes=0-").header(ACCEPT_ENCODING, "identity").send();
            let result = tokio::select! {
                r = sent => r,
                _ = stopped(stop.clone()) => return outcome_for_stop(&stop),
            };
            match result {
                Ok(r) if r.status() == StatusCode::PARTIAL_CONTENT && http::content_range_total(r.headers()).is_some() => {
                    plain = Some((i, r));
                    break;
                }
                Ok(r) if r.status().is_success() => {
                    if plain.is_none() {
                        plain = Some((i, r));
                    }
                }
                Ok(r) => match status_problem(r.status()) {
                    Problem::Permanent(msg) => {
                        s.status = SourceStatus::Failed(msg.clone());
                        reason = Some(msg);
                    }
                    Problem::Transient(msg) => reason = Some(msg),
                    _ => reason = Some(format!("{} {}", r.status().as_u16(), r.status().canonical_reason().unwrap_or(""))),
                },
                Err(e) => reason = Some(error_text(&e)),
            }
        }
        if let Some(found) = plain {
            break found;
        }
        let reason = reason.unwrap_or_else(|| "No source to download from".into());
        let alive = sources.iter().any(|s| s.enabled && !s.status.is_dead());
        if !alive || attempt >= req.retries as usize {
            return Outcome::Failed(failure_text(sources.len(), &reason));
        }
        if !sleep_unless_stopped(retry_delay(&tuning, attempt), &stop).await {
            return outcome_for_stop(&stop);
        }
        attempt += 1;
    };

    let ranges = first.status() == StatusCode::PARTIAL_CONTENT;
    let size = if ranges { http::content_range_total(first.headers()) } else { first.content_length() };
    let resumable = ranges && size.is_some();
    sources[first_idx].validator = http::validator(first.headers());
    sources[first_idx].status = SourceStatus::Working;
    // The other sources are checked before they get connections.
    for (i, s) in sources.iter_mut().enumerate() {
        if i != first_idx && !s.status.is_dead() {
            s.status = SourceStatus::Checking;
            s.validator = None;
        }
    }
    // Starting over keeps the name the download already had.
    let claimed = match &saved {
        Some(state) => File::create(part_path(&state.dest)).map(|f| (state.dest.clone(), f)),
        None => {
            let name = req.name.clone().unwrap_or_else(|| http::file_name(first.headers(), first.url()));
            claim_path(&req.folder, &name)
        }
    };
    let (dest, file) = match claimed {
        Ok(c) => c,
        Err(e) => return Outcome::Failed(format!("Can't create the file in {}: {e}", req.folder.display())),
    };
    if let Some(size) = size
        && let Err(e) = file.set_len(size)
    {
        return Outcome::Failed(format!("Can't make room for the file: {e}"));
    }
    // Ranged downloads are split over connections; without ranges or a size, one connection reads to the end.
    let segs = if resumable { Segments::new(size.unwrap_or(0), tuning.min_split) } else { Segments::new(u64::MAX, u64::MAX) };
    progress.downloaded.store(0, Ordering::SeqCst);
    let state = ResumeState { url: req.url.clone(), dest, size, resumable, segments: Vec::new(), sources: Vec::new() };
    let job = Job::new(client, &req, tuning, state, sources, file, segs, progress, &control, !resumable, max_conns);
    run(job, Some((first_idx, first)), max_conns, stop).await
}

fn failure_text(sources: usize, reason: &str) -> String {
    if sources > 1 { format!("All sources failed: {reason}") } else { reason.to_string() }
}

async fn run(job: Arc<Job>, first: Option<(usize, reqwest::Response)>, max_conns: usize, stop: watch::Receiver<u8>) -> Outcome {
    {
        let p = &job.progress;
        *p.name.lock().unwrap() = job.state.dest.file_name().map(|n| n.to_string_lossy().into_owned());
        p.size.store(job.state.size.unwrap_or(u64::MAX), Ordering::SeqCst);
        p.resumable.store(job.state.resumable, Ordering::SeqCst);
    }
    job.save();
    job.publish_sources();
    let count = job.sources.read().unwrap().len();
    for i in 0..count {
        job.spawn_check_if_needed(i);
    }

    let mut tasks = Vec::new();
    let mut next_id = 0;
    if let Some((src, response)) = first {
        job.sources.read().unwrap()[src].active.fetch_add(1, Ordering::SeqCst);
        job.segs.lock().unwrap().take(0).expect("a new download has one free segment");
        job.conns[0].running.store(true, Ordering::SeqCst);
        tasks.push(tokio::spawn(connection(job.clone(), 0, Some((src, response)))));
        next_id = 1;
    }
    let conns = if job.sequential { 1 } else { max_conns };
    for id in next_id..conns {
        job.conns[id].running.store(true, Ordering::SeqCst);
        tasks.push(tokio::spawn(connection(job.clone(), id, None)));
    }

    // Saves the resume state, measures speeds and handles commands until the connections are done.
    let (done_tx, done_rx) = watch::channel(false);
    let saver = tokio::spawn({
        let job = job.clone();
        let mut done = done_rx.clone();
        async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(job.tuning.save_interval) => job.save(),
                    _ = done.changed() => break,
                }
            }
        }
    });
    let monitor = tokio::spawn({
        let job = job.clone();
        let mut done = done_rx;
        async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(job.tuning.tick) => job.tick(),
                    _ = done.changed() => break,
                }
            }
        }
    });
    for t in tasks {
        let _ = t.await;
    }
    // Connections added while running (Control::set_connections).
    loop {
        let next = job.extra.lock().unwrap().pop();
        match next {
            Some(t) => {
                let _ = t.await;
            }
            None => break,
        }
    }
    let _ = done_tx.send(true);
    let _ = saver.await;
    let _ = monitor.await;
    job.publish_sources();

    if let Some(msg) = job.fatal.lock().unwrap().clone() {
        job.save();
        return Outcome::Failed(msg);
    }
    if *stop.borrow() == PAUSE {
        job.save();
        return Outcome::Paused;
    }
    let complete = job.sequential || job.segs.lock().unwrap().is_complete();
    if !complete {
        job.save();
        return Outcome::Failed("The download stopped before every byte was in".into());
    }
    job.progress.finishing.store(true, Ordering::SeqCst);
    let size = job.state.size;
    if job.sequential {
        let got = job.progress.downloaded.load(Ordering::SeqCst);
        if size.is_some_and(|s| s != got) {
            return Outcome::Failed(format!("The server sent {got} of {} bytes", size.unwrap_or(0)));
        }
        let _ = job.file.set_len(got);
    }
    if let Err(e) = job.file.sync_all() {
        return Outcome::Failed(format!("Can't write the file: {e}"));
    }
    let dest = job.state.dest.clone();
    match std::fs::rename(part_path(&dest), &dest) {
        Ok(()) => {
            if let Some(path) = &job.state_file {
                let _ = std::fs::remove_file(path);
            }
            Outcome::Finished(dest)
        }
        Err(e) => Outcome::Failed(format!("Can't rename the finished file: {e}")),
    }
}

/// One connection: picks a source, works on a range until it is done, then takes another, until nothing is left.
async fn connection(job: Arc<Job>, id: usize, first: Option<(usize, reqwest::Response)>) {
    work(&job, id, first).await;
    job.release(id);
    job.conns[id].running.store(false, Ordering::SeqCst);
    job.changed.notify_waiters();
}

async fn work(job: &Arc<Job>, id: usize, mut first: Option<(usize, reqwest::Response)>) {
    let job = job.clone();
    let mut avoid: Option<usize> = None;
    loop {
        if job.is_stopped() || id >= job.allowed() {
            break;
        }
        let (src, response) = match first.take() {
            Some(found) => found,
            None => {
                if job.segs.lock().unwrap().owned_by(id).is_none() && job.segs.lock().unwrap().take(id).is_none() {
                    break;
                }
                job.conns[id].phase.store(CONNECTING, Ordering::SeqCst);
                let src = match job.pick(avoid) {
                    Pick::Use(src) => src,
                    Pick::Wait => {
                        job.conns[id].phase.store(WAITING, Ordering::SeqCst);
                        job.release(id);
                        job.wait_for_change().await;
                        continue;
                    }
                    Pick::NoneLeft => {
                        job.fail_no_sources();
                        break;
                    }
                };
                match job.open(id, src).await {
                    Ok(r) => (src, r),
                    Err(p) => {
                        job.unuse(src);
                        if !job.handle(id, src, p, &mut avoid).await {
                            break;
                        }
                        continue;
                    }
                }
            }
        };
        let stat = &job.conns[id];
        stat.source.store(src, Ordering::SeqCst);
        stat.handoff.store(false, Ordering::SeqCst);
        stat.moving.store(false, Ordering::SeqCst);
        stat.slow_ticks.store(0, Ordering::SeqCst);
        stat.phase.store(DOWNLOADING, Ordering::SeqCst);
        job.progress.active.fetch_add(1, Ordering::SeqCst);
        job.progress.peak.fetch_max(job.progress.active.load(Ordering::SeqCst), Ordering::SeqCst);
        let result = job.stream(id, src, response).await;
        stat.phase.store(IDLE, Ordering::SeqCst);
        job.progress.active.fetch_sub(1, Ordering::SeqCst);
        stat.source.store(usize::MAX, Ordering::SeqCst);
        job.unuse(src);
        match result {
            Ok(()) => {
                avoid = None;
                let source = job.source(src);
                let settled = source.slow_since.lock().unwrap().is_none_or(|t| t.elapsed() >= job.tuning.slow_after);
                if source.status() == SourceStatus::Slow && settled {
                    source.set_status(SourceStatus::Working);
                }
                if job.sequential {
                    break;
                }
            }
            Err(p) => {
                if !job.handle(id, src, p, &mut avoid).await {
                    break;
                }
            }
        }
    }
    job.release(id);
    job.changed.notify_waiters();
}

impl Job {
    #[allow(clippy::too_many_arguments)]
    fn new(
        client: &reqwest::Client,
        req: &Request,
        tuning: Tuning,
        state: ResumeState,
        sources: Vec<SavedSource>,
        file: File,
        segs: Segments,
        progress: Arc<Progress>,
        control: &Control,
        sequential: bool,
        max_conns: usize,
    ) -> Arc<Job> {
        let (stop_tx, _) = watch::channel(RUN);
        let stop = merged_stop(control.stop.subscribe(), &stop_tx);
        let allowed = if sequential { 1 } else { max_conns.clamp(1, MAX_CONNECTIONS) };
        progress.allowed.store(allowed, Ordering::SeqCst);
        Arc::new(Job {
            allowed: AtomicUsize::new(allowed),
            extra: Mutex::new(Vec::new()),
            client: client.clone(),
            tuning,
            retries: req.retries,
            per_server: req.connections_per_server.clamp(1, 32) as usize,
            limits: req.limits.clone(),
            sequential,
            state,
            state_file: req.state_file.clone(),
            file,
            segs: Mutex::new(segs),
            sources: RwLock::new(sources.into_iter().filter_map(Source::new).collect()),
            hosts: Mutex::new(HashMap::new()),
            pick_lock: Mutex::new(()),
            refusal_gate: tokio::sync::Mutex::new(()),
            conns: (0..MAX_CONNECTIONS).map(|_| ConnStat { source: AtomicUsize::new(usize::MAX), ..Default::default() }).collect(),
            progress,
            changed: Notify::new(),
            inbox: control.inbox.clone(),
            last_reason: Mutex::new(None),
            fatal: Mutex::new(None),
            stop,
            stop_tx,
        })
    }

    fn source(&self, idx: usize) -> Arc<Source> {
        self.sources.read().unwrap()[idx].clone()
    }

    /// Saves the resume state, after making sure every byte it counts is on disk.
    fn save(&self) {
        let Some(path) = &self.state_file else { return };
        let segments = self.segs.lock().unwrap().all().to_vec();
        let _ = self.file.sync_data();
        let mut state = self.state.clone();
        state.segments = if state.resumable { segments } else { Vec::new() };
        state.sources = self.sources.read().unwrap().iter().filter(|s| !s.removed.load(Ordering::SeqCst)).map(|s| s.saved()).collect();
        let _ = state.save(path);
    }

    fn allowed(&self) -> usize {
        self.allowed.load(Ordering::SeqCst)
    }

    fn is_stopped(&self) -> bool {
        *self.stop.borrow() != RUN
    }

    fn release(&self, id: usize) {
        let mut segs = self.segs.lock().unwrap();
        if let Some(idx) = segs.owned_by(id) {
            segs.release(idx);
        }
    }

    fn unuse(&self, src: usize) {
        self.source(src).active.fetch_sub(1, Ordering::SeqCst);
        self.changed.notify_waiters();
    }

    fn fail(&self, msg: String) {
        self.fatal.lock().unwrap().get_or_insert(msg);
        let _ = self.stop_tx.send(ABORT);
    }

    fn fail_no_sources(&self) {
        let count = self.sources.read().unwrap().len();
        let reason = self.last_reason.lock().unwrap().clone().unwrap_or_else(|| "No source is working".into());
        self.fail(failure_text(count, &reason));
    }

    /// Marks a source as never to be used again in this run, remembering why.
    fn kill(&self, src: usize, status: SourceStatus) {
        // The download's failure message gives the plain reason ("404 Not Found"), not the Mirrors tab's text.
        let reason = match &status {
            SourceStatus::Failed(reason) => reason.clone(),
            SourceStatus::FileChanged => "File changed on the server".into(),
            other => other.text(),
        };
        *self.last_reason.lock().unwrap() = Some(reason);
        self.source(src).set_status(status);
        self.changed.notify_waiters();
    }

    /// Chooses a source for a new connection: a working, enabled one whose host has room, with the fewest
    /// connections on its host, then the best speed. Slow sources and `avoid` come last.
    fn pick(&self, avoid: Option<usize>) -> Pick {
        let _guard = self.pick_lock.lock().unwrap();
        let sources = self.sources.read().unwrap();
        let now = Instant::now();
        let mut per_host: HashMap<&str, usize> = HashMap::new();
        for s in sources.iter() {
            *per_host.entry(s.host.as_str()).or_default() += s.active.load(Ordering::SeqCst);
        }
        // While mirrors are still being checked, no source takes more than its fair share, so the connections
        // left over go to the mirrors once they are confirmed instead of all piling onto the first link.
        let live = sources.iter().filter(|s| s.enabled.load(Ordering::SeqCst) && !s.status().is_dead()).count();
        let checking = sources.iter().any(|s| s.enabled.load(Ordering::SeqCst) && s.status() == SourceStatus::Checking);
        let share = if checking && !self.sequential { self.allowed().div_ceil(live.max(1)) } else { usize::MAX };
        let mut hosts = self.hosts.lock().unwrap();
        let mut alive = false;
        type Key = (bool, usize, std::cmp::Reverse<u64>);
        let mut best: Option<(usize, Key)> = None;
        for (i, s) in sources.iter().enumerate() {
            let status = s.status();
            if !s.enabled.load(Ordering::SeqCst) || status.is_dead() {
                continue;
            }
            alive = true;
            if status == SourceStatus::Checking || s.retry_at.lock().unwrap().is_some_and(|t| t > now) {
                continue;
            }
            let on_host = per_host[s.host.as_str()];
            if on_host >= self.per_server || s.active.load(Ordering::SeqCst) >= share {
                continue;
            }
            if let Some(limit) = hosts.get(&s.host)
                && on_host >= limit.cap
                && now < limit.until
            {
                continue;
            }
            let key = (status == SourceStatus::Slow || Some(i) == avoid, on_host, std::cmp::Reverse(s.speed.load(Ordering::SeqCst)));
            if best.as_ref().is_none_or(|(_, k)| key < *k) {
                best = Some((i, key));
            }
        }
        match best {
            Some((i, _)) => {
                let s = &sources[i];
                // Past a refusal's wait, this connection is the one more that gets tried.
                if let Some(limit) = hosts.get_mut(&s.host)
                    && per_host[s.host.as_str()] >= limit.cap
                {
                    limit.cap += 1;
                    limit.until = now + self.tuning.refusal_wait;
                }
                s.active.fetch_add(1, Ordering::SeqCst);
                Pick::Use(i)
            }
            None if alive => Pick::Wait,
            None => Pick::NoneLeft,
        }
    }

    /// After a refusal: no more connections to the host than are receiving data now, until the wait is over.
    fn limit_host(&self, host: &str) {
        let sources = self.sources.read().unwrap();
        let accepted = self
            .conns
            .iter()
            .filter_map(|c| sources.get(c.source.load(Ordering::SeqCst)))
            .filter(|s| s.host == host)
            .count()
            .max(1);
        let now = Instant::now();
        let mut hosts = self.hosts.lock().unwrap();
        let cap = match hosts.get(host) {
            Some(limit) if limit.until > now => limit.cap.min(accepted),
            _ => accepted,
        };
        hosts.insert(host.to_string(), HostLimit { cap, until: now + self.tuning.refusal_wait });
    }

    async fn wait_for_change(&self) {
        let mut stop = self.stop.clone();
        tokio::select! {
            _ = self.changed.notified() => {}
            _ = tokio::time::sleep(self.tuning.tick) => {}
            _ = stopped_ref(&mut stop) => {}
        }
    }

    /// Deals with a problem; returns whether the connection should keep going.
    async fn handle(&self, id: usize, src: usize, problem: Problem, avoid: &mut Option<usize>) -> bool {
        let source = self.source(src);
        if !matches!(problem, Problem::Stopped) {
            self.release(id);
        }
        match problem {
            Problem::Stopped => false,
            Problem::Stalled | Problem::Moved => true,
            Problem::Slow => {
                source.set_status(SourceStatus::Slow);
                *source.slow_since.lock().unwrap() = Some(Instant::now());
                self.publish_sources();
                *avoid = Some(src);
                true
            }
            Problem::Refused => {
                // Keep the connections the host accepted (those receiving data now); one more is tried after the wait.
                self.limit_host(&source.host);
                // A refused connection waits its turn, so a busy server sees at most one new try per wait.
                let _gate = self.refusal_gate.lock().await;
                sleep_unless_stopped(self.tuning.refusal_wait, &self.stop).await
            }
            Problem::FileChanged => {
                self.kill(src, SourceStatus::FileChanged);
                true
            }
            Problem::Permanent(msg) => {
                self.kill(src, SourceStatus::Failed(msg));
                true
            }
            Problem::Transient(msg) => {
                // Connections to one source that drop together (a server restart, a file being replaced) are one
                // failure: while the source waits for its retry, more drops don't count again.
                let mut retry_at = source.retry_at.lock().unwrap();
                if retry_at.is_some_and(|t| t > Instant::now()) {
                    return true;
                }
                let failures = source.failures.fetch_add(1, Ordering::SeqCst) + 1;
                if failures > self.retries {
                    self.kill(src, SourceStatus::Failed(msg));
                } else {
                    *self.last_reason.lock().unwrap() = Some(msg);
                    let delay = retry_delay(&self.tuning, failures as usize - 1);
                    *retry_at = Some(Instant::now() + delay);
                }
                true
            }
        }
    }

    /// Opens a request on `src` for the rest of the range this connection owns.
    async fn open(&self, id: usize, src: usize) -> Result<reqwest::Response, Problem> {
        let source = self.source(src);
        let mut req = self.client.get(source.url.clone()).header(ACCEPT_ENCODING, "identity");
        let (start, end) = if self.sequential {
            // Without ranges a failed download starts over.
            let mut segs = self.segs.lock().unwrap();
            *segs = Segments::new(u64::MAX, u64::MAX);
            segs.take(id);
            self.progress.downloaded.store(0, Ordering::SeqCst);
            (0, u64::MAX)
        } else {
            let segs = self.segs.lock().unwrap();
            let s = segs.get(segs.owned_by(id).ok_or(Problem::Stopped)?);
            (s.pos, s.end)
        };
        if !self.sequential {
            req = req.header(RANGE, format!("bytes={start}-{}", end - 1));
            if let Some(v) = source.validator.lock().unwrap().as_ref() {
                req = req.header(IF_RANGE, v);
            }
        }
        let mut stop = self.stop.clone();
        let response = tokio::select! {
            r = req.send() => r.map_err(|e| Problem::Transient(error_text(&e)))?,
            _ = stopped_ref(&mut stop) => return Err(Problem::Stopped),
        };
        let status = response.status();
        if self.sequential {
            return if status.is_success() { Ok(response) } else { Err(status_problem(status)) };
        }
        match status {
            StatusCode::PARTIAL_CONTENT if http::content_range_start(response.headers()) == Some(start) => Ok(response),
            StatusCode::PARTIAL_CONTENT => Err(Problem::Transient("The server sent the wrong range".into())),
            // A full answer to a range request with If-Range: the file is no longer the same.
            StatusCode::OK => Err(Problem::FileChanged),
            other => Err(status_problem(other)),
        }
    }

    /// Reads a response into the file until the connection's range is done.
    async fn stream(&self, id: usize, src: usize, mut response: reqwest::Response) -> Result<(), Problem> {
        let source = self.source(src);
        let stat = &self.conns[id];
        let mut buf: Vec<u8> = Vec::with_capacity(self.tuning.write_buffer);
        let mut stop = self.stop.clone();
        loop {
            let next = tokio::select! {
                r = tokio::time::timeout(self.tuning.stall_timeout, response.chunk()) => r,
                _ = stopped_ref(&mut stop) => {
                    self.flush(id, &source, &mut buf)?;
                    return Err(Problem::Stopped);
                }
            };
            match next {
                Err(_) => {
                    self.flush(id, &source, &mut buf)?;
                    return Err(Problem::Stalled);
                }
                Ok(Err(e)) => {
                    self.flush(id, &source, &mut buf)?;
                    return Err(Problem::Transient(error_text(&e)));
                }
                Ok(Ok(None)) => {
                    let done = self.flush(id, &source, &mut buf)?;
                    return if done || self.sequential {
                        Ok(())
                    } else {
                        Err(Problem::Transient("The server closed the connection early".into()))
                    };
                }
                Ok(Ok(Some(bytes))) => {
                    stat.received.fetch_add(bytes.len() as u64, Ordering::SeqCst);
                    buf.extend_from_slice(&bytes);
                    let wait = self
                        .limits
                        .iter()
                        .map(|l| l.take(bytes.len()))
                        .max()
                        .unwrap_or_default();
                    if !wait.is_zero() {
                        tokio::select! {
                            _ = tokio::time::sleep(wait) => {}
                            _ = stopped_ref(&mut stop) => {
                                self.flush(id, &source, &mut buf)?;
                                return Err(Problem::Stopped);
                            }
                        }
                    }
                    let (pos, end) = self.position(id).ok_or(Problem::Stopped)?;
                    if (buf.len() >= self.tuning.write_buffer || pos + buf.len() as u64 >= end)
                        && self.flush(id, &source, &mut buf)?
                    {
                        return Ok(());
                    }
                    if !self.sequential {
                        if stat.handoff.load(Ordering::SeqCst) {
                            self.flush(id, &source, &mut buf)?;
                            return Err(Problem::Slow);
                        }
                        if !source.enabled.load(Ordering::SeqCst) || stat.moving.load(Ordering::SeqCst) || id >= self.allowed() {
                            self.flush(id, &source, &mut buf)?;
                            return Err(Problem::Moved);
                        }
                    }
                }
            }
        }
    }

    fn position(&self, id: usize) -> Option<(u64, u64)> {
        let segs = self.segs.lock().unwrap();
        let s = segs.get(segs.owned_by(id)?);
        Some((s.pos, s.end))
    }

    /// Writes the buffer at the connection's position, never past its range's end (which another connection may
    /// have moved), and returns whether the range is done.
    fn flush(&self, id: usize, source: &Source, buf: &mut Vec<u8>) -> Result<bool, Problem> {
        let mut segs = self.segs.lock().unwrap();
        let Some(idx) = segs.owned_by(id) else {
            buf.clear();
            return Ok(true);
        };
        let s = segs.get(idx);
        let n = (buf.len() as u64).min(s.remaining()) as usize;
        if n > 0 {
            write_at(&self.file, &buf[..n], s.pos).map_err(|e| Problem::Permanent(format!("Can't write the file: {e}")))?;
            segs.advance(idx, n as u64);
            self.progress.downloaded.fetch_add(n as u64, Ordering::SeqCst);
            source.bytes.fetch_add(n as u64, Ordering::SeqCst);
            source.failures.store(0, Ordering::SeqCst);
            self.conns[id].bytes.fetch_add(n as u64, Ordering::SeqCst);
        }
        buf.clear();
        let done = segs.get(idx).remaining() == 0;
        if done {
            segs.release(idx);
        }
        Ok(done)
    }

    /// Checks a source before it gets connections (§7.2), unless it is already checked or dead.
    fn spawn_check_if_needed(self: &Arc<Self>, idx: usize) {
        let source = self.source(idx);
        if source.status() != SourceStatus::Checking || !source.enabled.load(Ordering::SeqCst) {
            return;
        }
        let job = self.clone();
        tokio::spawn(async move { job.check(idx).await });
    }

    async fn check(&self, idx: usize) {
        let source = self.source(idx);
        let mut attempt = 0usize;
        loop {
            let req = self.client.get(source.url.clone()).header(RANGE, "bytes=0-0").header(ACCEPT_ENCODING, "identity");
            let mut stop = self.stop.clone();
            let result = tokio::select! {
                r = req.send() => r,
                _ = stopped_ref(&mut stop) => return,
            };
            let problem = match result {
                Ok(r) if r.status() == StatusCode::PARTIAL_CONTENT || (r.status().is_success() && self.sequential) => {
                    let total = if r.status() == StatusCode::PARTIAL_CONTENT {
                        http::content_range_total(r.headers())
                    } else {
                        r.content_length()
                    };
                    if self.state.size.is_some() && total != self.state.size {
                        self.kill(idx, SourceStatus::DifferentFile);
                    } else {
                        *source.validator.lock().unwrap() = http::validator(r.headers());
                        source.set_status(SourceStatus::Working);
                        self.changed.notify_waiters();
                    }
                    return;
                }
                Ok(r) if r.status().is_success() => {
                    self.kill(idx, SourceStatus::NoResumeSupport);
                    return;
                }
                Ok(r) => status_problem(r.status()),
                Err(e) => Problem::Transient(error_text(&e)),
            };
            match problem {
                Problem::Transient(msg) | Problem::Permanent(msg) if attempt >= self.retries as usize => {
                    self.kill(idx, SourceStatus::Failed(msg));
                    return;
                }
                Problem::Permanent(msg) => {
                    self.kill(idx, SourceStatus::Failed(msg));
                    return;
                }
                _ => {}
            }
            if !sleep_unless_stopped(retry_delay(&self.tuning, attempt), &self.stop).await {
                return;
            }
            attempt += 1;
        }
    }

    /// Runs every `tick`: handles commands, measures speeds, hands slow connections' ranges to faster sources.
    fn tick(self: &Arc<Self>) {
        let commands: Vec<Command> = std::mem::take(&mut *self.inbox.lock().unwrap());
        for command in commands {
            match command {
                Command::AddMirrors(urls) => {
                    for url in urls.iter().map(|u| u.trim()).filter(|u| !u.is_empty()) {
                        let idx = {
                            let mut sources = self.sources.write().unwrap();
                            if sources.iter().any(|s| same_link(&s.text, url)) {
                                continue;
                            }
                            let saved = SavedSource { url: url.to_string(), validator: None, enabled: true, status: SourceStatus::Checking };
                            let Some(source) = Source::new(saved) else { continue };
                            sources.push(source);
                            sources.len() - 1
                        };
                        self.spawn_check_if_needed(idx);
                    }
                }
                Command::Remove(url) => {
                    let sources = self.sources.read().unwrap();
                    if let Some(s) = sources.iter().skip(1).find(|s| same_link(&s.text, &url)) {
                        s.removed.store(true, Ordering::SeqCst);
                        s.enabled.store(false, Ordering::SeqCst);
                    }
                    drop(sources);
                    self.changed.notify_waiters();
                }
                Command::SetConnections(n) => {
                    if self.sequential {
                        continue;
                    }
                    self.allowed.store(n, Ordering::SeqCst);
                    self.progress.allowed.store(n, Ordering::SeqCst);
                    for id in 0..n {
                        if !self.conns[id].running.swap(true, Ordering::SeqCst) {
                            let task = tokio::spawn(connection(self.clone(), id, None));
                            self.extra.lock().unwrap().push(task);
                        }
                    }
                    self.changed.notify_waiters();
                }
                Command::Enable(url, on) => {
                    let sources = self.sources.read().unwrap();
                    let others_on = sources.iter().filter(|s| !same_link(&s.text, &url) && s.enabled.load(Ordering::SeqCst)).count();
                    if let Some(s) = sources.iter().find(|s| same_link(&s.text, &url))
                        && (on || others_on > 0)
                    {
                        s.enabled.store(on, Ordering::SeqCst);
                        if on && matches!(s.status(), SourceStatus::Failed(_)) {
                            s.failures.store(0, Ordering::SeqCst);
                            s.set_status(SourceStatus::Working);
                        }
                    }
                    drop(sources);
                    let count = self.sources.read().unwrap().len();
                    for i in 0..count {
                        self.spawn_check_if_needed(i);
                    }
                }
            }
        }

        let secs = self.tuning.tick.as_secs_f64().max(0.001);
        for s in self.sources.read().unwrap().iter() {
            let now = s.bytes.load(Ordering::SeqCst);
            let delta = now - s.last_bytes.swap(now, Ordering::SeqCst);
            let rate = (delta as f64 / secs) as u64;
            let old = s.speed.load(Ordering::SeqCst);
            s.speed.store(if old == 0 { rate } else { (old + rate) / 2 }, Ordering::SeqCst);
        }

        // Slow connections: under a quarter of the average for `slow_after`, when another source could take over.
        let mut rates = Vec::new();
        for (id, c) in self.conns.iter().enumerate() {
            // Bytes received this tick, not written: writes come in `write_buffer` pieces, so below about 2.5 MB/s
            // a connection writes nothing in most ticks, the average drops to 0 and no slow source was ever found.
            let got = c.received.load(Ordering::SeqCst);
            let delta = got - c.last_received.swap(got, Ordering::SeqCst);
            let rate = (delta as f64 / secs) as u64;
            let old = c.speed.load(Ordering::SeqCst);
            let idle = c.source.load(Ordering::SeqCst) == usize::MAX;
            c.speed.store(if idle { 0 } else if old == 0 { rate } else { (old * 2 + rate) / 3 }, Ordering::SeqCst);
            let src = c.source.load(Ordering::SeqCst);
            if src != usize::MAX {
                rates.push((id, src, delta));
            }
        }
        let usable = self
            .sources
            .read()
            .unwrap()
            .iter()
            .filter(|s| s.enabled.load(Ordering::SeqCst) && matches!(s.status(), SourceStatus::Working | SourceStatus::Idle))
            .count();
        if rates.len() >= 2 && usable >= 2 && !self.sequential {
            let average = rates.iter().map(|r| r.2).sum::<u64>() / rates.len() as u64;
            let needed = (self.tuning.slow_after.as_secs_f64() / secs).ceil() as u32;
            for (id, src, delta) in rates {
                let c = &self.conns[id];
                if delta * 4 < average {
                    let ticks = c.slow_ticks.fetch_add(1, Ordering::SeqCst) + 1;
                    let elsewhere = self.sources.read().unwrap().iter().enumerate().any(|(i, s)| {
                        i != src && s.enabled.load(Ordering::SeqCst) && s.status() == SourceStatus::Working
                    });
                    if ticks >= needed && elsewhere {
                        c.handoff.store(true, Ordering::SeqCst);
                    }
                } else {
                    c.slow_ticks.store(0, Ordering::SeqCst);
                }
            }
        }
        self.rebalance();
        self.publish_sources();
        self.changed.notify_waiters();
    }

    /// Moves one connection a tick from the busiest working source to the least busy one when they differ by two or
    /// more, so a mirror that joins mid-download (or comes back) gets its share.
    fn rebalance(&self) {
        if self.sequential {
            return;
        }
        let sources = self.sources.read().unwrap();
        let now = Instant::now();
        let hosts = self.hosts.lock().unwrap();
        let open = |s: &Source| {
            s.enabled.load(Ordering::SeqCst)
                && s.status() == SourceStatus::Working
                && s.retry_at.lock().unwrap().is_none_or(|t| t <= now)
                && hosts.get(&s.host).is_none_or(|l| l.until <= now)
        };
        let usable: Vec<usize> = (0..sources.len()).filter(|&i| open(&sources[i])).collect();
        let active = |i: usize| sources[i].active.load(Ordering::SeqCst);
        let (Some(&low), Some(&high)) = (usable.iter().min_by_key(|&&i| active(i)), usable.iter().max_by_key(|&&i| active(i)))
        else {
            return;
        };
        if active(high) < active(low) + 2 || active(low) >= self.per_server {
            return;
        }
        if self.conns.iter().any(|c| c.moving.load(Ordering::SeqCst)) {
            return;
        }
        if let Some(c) = self.conns.iter().find(|c| c.source.load(Ordering::SeqCst) == high) {
            c.moving.store(true, Ordering::SeqCst);
        }
    }

    fn publish_sources(&self) {
        let infos = self
            .sources
            .read()
            .unwrap()
            .iter()
            .filter(|s| !s.removed.load(Ordering::SeqCst))
            .map(|s| {
                let active = s.active.load(Ordering::SeqCst);
                let status = match s.status() {
                    SourceStatus::Working if active == 0 => SourceStatus::Idle,
                    other => other,
                };
                SourceInfo {
                    url: s.text.clone(),
                    status,
                    enabled: s.enabled.load(Ordering::SeqCst),
                    active,
                    speed: s.speed.load(Ordering::SeqCst),
                    bytes: s.bytes.load(Ordering::SeqCst),
                }
            })
            .collect();
        *self.progress.sources.lock().unwrap() = infos;
        self.publish_detail();
    }

    /// The segments and the connections in use, for the Download window.
    fn publish_detail(&self) {
        let segs = self.segs.lock().unwrap().clone();
        let sources = self.sources.read().unwrap();
        let now = Instant::now();
        let retry = sources
            .iter()
            .filter(|s| s.enabled.load(Ordering::SeqCst) && !s.status().is_dead())
            .filter_map(|s| *s.retry_at.lock().unwrap())
            .filter(|&t| t > now)
            .min();
        let allowed = self.allowed();
        let mut conns = Vec::new();
        for (id, c) in self.conns.iter().enumerate() {
            if !c.running.load(Ordering::SeqCst) && id >= allowed {
                continue;
            }
            let src = c.source.load(Ordering::SeqCst);
            let source = sources.get(src).map(|s| s.text.clone()).unwrap_or_default();
            let (start, end, pos) = segs.owned_by(id).map(|i| segs.get(i)).map_or((0, 0, 0), |g| (g.start, g.end, g.pos));
            let state = match c.phase.load(Ordering::SeqCst) {
                _ if !c.running.load(Ordering::SeqCst) => "Idle".to_string(),
                CONNECTING => "Connecting".to_string(),
                DOWNLOADING => "Downloading".to_string(),
                WAITING => match retry {
                    Some(t) => format!("Retrying in {} s", (t - now).as_secs().max(1)),
                    None => "Idle".to_string(),
                },
                _ => "Idle".to_string(),
            };
            conns.push(ConnInfo {
                id,
                source,
                start,
                end: if end == u64::MAX { 0 } else { end },
                pos,
                bytes: c.bytes.load(Ordering::SeqCst),
                speed: c.speed.load(Ordering::SeqCst),
                state,
            });
        }
        drop(sources);
        *self.progress.segments.lock().unwrap() = if self.sequential { Vec::new() } else { segs.all().to_vec() };
        *self.progress.connections.lock().unwrap() = conns;
    }
}

#[cfg(unix)]
fn write_at(file: &File, data: &[u8], offset: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(file, data, offset)
}

#[cfg(windows)]
fn write_at(file: &File, mut data: &[u8], mut offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !data.is_empty() {
        let n = file.seek_write(data, offset)?;
        data = &data[n..];
        offset += n as u64;
    }
    Ok(())
}

fn status_problem(status: StatusCode) -> Problem {
    let text = format!("{} {}", status.as_u16(), status.canonical_reason().unwrap_or("")).trim().to_string();
    match status.as_u16() {
        429 | 503 => Problem::Refused,
        408 | 500..=599 => Problem::Transient(text),
        _ => Problem::Permanent(text),
    }
}

pub(crate) fn error_text(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "The server didn't answer in time".into()
    } else if e.is_connect() {
        "Can't connect to the server".into()
    } else {
        "The connection was lost".into()
    }
}

fn retry_delay(tuning: &Tuning, attempt: usize) -> Duration {
    tuning.retry_delays.get(attempt).or(tuning.retry_delays.last()).copied().unwrap_or_default()
}

async fn stopped(mut rx: watch::Receiver<u8>) {
    let _ = rx.wait_for(|v| *v != RUN).await;
}

async fn stopped_ref(rx: &mut watch::Receiver<u8>) {
    let _ = rx.wait_for(|v| *v != RUN).await;
}

/// Sleeps, or returns false early if the download stops.
async fn sleep_unless_stopped(d: Duration, stop: &watch::Receiver<u8>) -> bool {
    let mut rx = stop.clone();
    tokio::select! {
        _ = tokio::time::sleep(d) => *stop.borrow() == RUN,
        _ = stopped_ref(&mut rx) => false,
    }
}

fn outcome_for_stop(stop: &watch::Receiver<u8>) -> Outcome {
    if *stop.borrow() == PAUSE { Outcome::Paused } else { Outcome::Failed("Stopped".into()) }
}

/// One receiver that fires when the user pauses or the job aborts itself.
fn merged_stop(user: watch::Receiver<u8>, job: &watch::Sender<u8>) -> watch::Receiver<u8> {
    let (tx, rx) = watch::channel(*user.borrow());
    let mut job_rx = job.subscribe();
    let mut user = user;
    tokio::spawn(async move {
        loop {
            tokio::select! {
                r = user.changed() => if r.is_err() { break },
                r = job_rx.changed() => if r.is_err() { break },
            }
            let v = (*user.borrow()).max(*job_rx.borrow());
            if tx.send(v).is_err() {
                break;
            }
        }
    });
    rx
}

/// `<name>.hdpart` next to the destination.
pub fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".hdpart");
    PathBuf::from(s)
}

/// The destination, or "name (2).ext", "name (3).ext"… when that name is taken (by a file or a partial download).
/// Picks a free name like `unique_path` and creates its `.hdpart` file in the same step, so two downloads of the
/// same file starting together never pick the same name: one that loses the race takes the next number. Once its
/// `.hdpart` exists no other download can claim the name, so checking the finished name after creating it is enough.
fn claim_path(folder: &Path, name: &str) -> std::io::Result<(PathBuf, File)> {
    // On Windows a file that another download is deleting or renaming lingers "delete pending" for a moment, and
    // creating that name meanwhile fails with access denied rather than "already exists". That passes within
    // milliseconds; an access denied that lasts is a folder that can't be written, and is reported as such.
    let mut denied = 0;
    for _ in 0..1000 {
        let dest = unique_path(folder, name);
        match std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(part_path(&dest)) {
            // Another download may have finished under this name after it was picked: then try the next.
            Ok(_) if dest.exists() => {
                let _ = std::fs::remove_file(part_path(&dest));
                continue;
            }
            Ok(file) => return Ok((dest, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && cfg!(windows) && denied < 50 => {
                denied += 1;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("no free file name"))
}

pub fn unique_path(folder: &Path, name: &str) -> PathBuf {
    let taken = |p: &Path| p.exists() || part_path(p).exists();
    let first = folder.join(name);
    if !taken(&first) {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    (2..)
        .map(|i| folder.join(format!("{stem} ({i}){ext}")))
        .find(|p| !taken(p))
        .expect("some numbered name is free")
}
