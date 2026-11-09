//! Downloads one file from one source over up to N connections (SPECIFICATION.md §7.4, §8).
//!
//! The first connection asks for the whole file (`Range: bytes=0-`); its answer gives the size, whether ranges
//! work and the validator. When ranges work, the other connections open and each takes the second half of the
//! largest remaining range (see `segments`). Data goes to `<name>.hdpart`, written in place at its offset, and
//! the file gets its final name once every byte is in.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{ACCEPT_ENCODING, IF_RANGE, RANGE};
use tokio::sync::watch;

use crate::http;
use crate::segments::{Segment, Segments};
use serde::{Deserialize, Serialize};

/// What to download and how.
#[derive(Clone, Debug)]
pub struct Request {
    pub url: String,
    pub folder: PathBuf,
    /// The file name; `None` takes it from the server.
    pub name: Option<String>,
    /// Connections for this download (1–32).
    pub connections: u32,
    /// Connections to any one server (1–32).
    pub connections_per_server: u32,
    /// Consecutive failures before the source counts as failed.
    pub retries: u32,
    /// Where the resume state is kept (`None`: not saved).
    pub state_file: Option<PathBuf>,
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
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Finished(PathBuf),
    Paused,
    Failed(String),
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
    pub name: Mutex<Option<String>>,
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
}

/// Pauses a running download from another task or thread.
#[derive(Clone)]
pub struct Control(Arc<watch::Sender<u8>>);

const RUN: u8 = 0;
const PAUSE: u8 = 1;
const ABORT: u8 = 2;

impl Control {
    pub fn pause(&self) {
        let _ = self.0.send(PAUSE);
    }
}

pub fn control() -> (Control, watch::Receiver<u8>) {
    let (tx, rx) = watch::channel(RUN);
    (Control(Arc::new(tx)), rx)
}

enum Problem {
    /// The connection should go back to the pool and reconnect at once.
    Stalled,
    /// Paused or aborted.
    Stopped,
    /// A failure that may pass: counted against the source's retries.
    Transient(String),
    /// The server turned away this connection (429, 503).
    Refused,
    /// The file on the server isn't the one being downloaded any more.
    FileChanged,
    /// Retrying won't help (404, 403…).
    Permanent(String),
}

struct Job {
    client: reqwest::Client,
    url: reqwest::Url,
    tuning: Tuning,
    retries: u32,
    validator: Option<String>,
    /// What gets saved; its `segments` are filled in from `segs` at each save.
    state: ResumeState,
    state_file: Option<PathBuf>,
    file: File,
    segs: Mutex<Segments>,
    progress: Arc<Progress>,
    failures: AtomicU32,
    fatal: Mutex<Option<String>>,
    stop_tx: watch::Sender<u8>,
    stop: watch::Receiver<u8>,
    refusal_gate: tokio::sync::Mutex<()>,
}

/// Everything needed to continue a download after a pause, a quit, a crash or a power loss, saved as JSON.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeState {
    pub url: String,
    /// The final file; the data is in `<dest>.hdpart` until it is complete.
    pub dest: PathBuf,
    pub size: Option<u64>,
    pub validator: Option<String>,
    /// False when the server has no ranges or no size: such a download starts over.
    pub resumable: bool,
    pub segments: Vec<Segment>,
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

/// Where a download starts from.
enum Start {
    /// A new download: the first response is already open.
    Fresh(reqwest::Response),
    /// Continuing saved ranges: no first response; every connection opens its own.
    Resume,
}

/// Runs a download to the end, a pause or a failure.
///
/// With `req.state_file`, the resume state is saved there every `save_interval` while downloading, on pause and on
/// failure, and removed when the download finishes; if the file already holds a state for this link, the download
/// continues from it (or starts over when it can't resume, or when the `.hdpart` file no longer matches).
pub async fn download(
    client: &reqwest::Client,
    req: Request,
    tuning: Tuning,
    progress: Arc<Progress>,
    stop: watch::Receiver<u8>,
) -> Outcome {
    let Ok(url) = reqwest::Url::parse(&req.url) else {
        return Outcome::Failed("Not a valid link".into());
    };
    let max_conns = req.connections.clamp(1, 32).min(req.connections_per_server.clamp(1, 32)) as usize;
    let saved = req.state_file.as_deref().and_then(|p| ResumeState::load(p).ok()).filter(|s| s.url == req.url);

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
            let segs = Segments::from_saved(state.segments.clone(), tuning.min_split);
            progress.downloaded.store(segs.downloaded(), Ordering::SeqCst);
            let job = Job::new(client, url, &req, tuning, state.clone(), file, segs, progress, &stop);
            return run(job, Start::Resume, max_conns, false, stop).await;
        }
    }

    // The first connection: the whole file, which also tells the size, range support and validator.
    let mut attempt = 0;
    let first = loop {
        let sent = client.get(url.clone()).header(RANGE, "bytes=0-").header(ACCEPT_ENCODING, "identity").send();
        let result = tokio::select! {
            r = sent => r,
            _ = stopped(stop.clone()) => return outcome_for_stop(&stop),
        };
        let problem = match result {
            Ok(r) if r.status().is_success() => break r,
            Ok(r) => status_problem(r.status()),
            Err(e) => Problem::Transient(error_text(&e)),
        };
        match problem {
            Problem::Permanent(msg) => return Outcome::Failed(msg),
            Problem::Transient(msg) if attempt >= req.retries => return Outcome::Failed(msg),
            _ => {}
        }
        if !sleep_unless_stopped(retry_delay(&tuning, attempt as usize), &stop).await {
            return outcome_for_stop(&stop);
        }
        attempt += 1;
    };

    let ranges = first.status() == StatusCode::PARTIAL_CONTENT;
    let size = if ranges { http::content_range_total(first.headers()) } else { first.content_length() };
    let resumable = ranges && size.is_some();
    // Starting over keeps the name the download already had.
    let dest = match &saved {
        Some(state) => state.dest.clone(),
        None => {
            let name = req.name.clone().unwrap_or_else(|| http::file_name(first.headers(), first.url()));
            unique_path(&req.folder, &name)
        }
    };
    let part = part_path(&dest);
    let file = match File::create(&part) {
        Ok(f) => f,
        Err(e) => return Outcome::Failed(format!("Can't create {}: {e}", part.display())),
    };
    if let Some(size) = size
        && let Err(e) = file.set_len(size)
    {
        return Outcome::Failed(format!("Can't make room for the file: {e}"));
    }
    // Ranged downloads are split over connections; without ranges or a size, one connection reads to the end.
    let segs = if resumable {
        Segments::new(size.unwrap_or(0), tuning.min_split)
    } else {
        Segments::new(u64::MAX, u64::MAX)
    };
    progress.downloaded.store(0, Ordering::SeqCst);
    let state = ResumeState {
        url: req.url.clone(),
        dest,
        size,
        validator: http::validator(first.headers()),
        resumable,
        segments: Vec::new(),
    };
    let job = Job::new(client, url, &req, tuning, state, file, segs, progress, &stop);
    run(job, Start::Fresh(first), max_conns, !resumable, stop).await
}

async fn run(job: Arc<Job>, start: Start, max_conns: usize, sequential: bool, stop: watch::Receiver<u8>) -> Outcome {
    {
        let p = &job.progress;
        *p.name.lock().unwrap() = job.state.dest.file_name().map(|n| n.to_string_lossy().into_owned());
        p.size.store(job.state.size.unwrap_or(u64::MAX), Ordering::SeqCst);
        p.resumable.store(job.state.resumable, Ordering::SeqCst);
    }
    job.save();

    let mut tasks = Vec::new();
    let mut first_id = 0;
    if let Start::Fresh(first) = start {
        let idx = job.segs.lock().unwrap().take(0).expect("a new download has one free segment");
        tasks.push(tokio::spawn(connection(job.clone(), 0, Some((idx, first)), sequential)));
        first_id = 1;
    }
    if !sequential {
        for id in first_id..max_conns {
            tasks.push(tokio::spawn(connection(job.clone(), id, None, false)));
        }
    }

    // Saves the resume state every few seconds until the connections are done.
    let (done_tx, mut done_rx) = watch::channel(false);
    let saver = tokio::spawn({
        let job = job.clone();
        async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(job.tuning.save_interval) => job.save(),
                    _ = done_rx.changed() => break,
                }
            }
        }
    });
    for t in tasks {
        let _ = t.await;
    }
    let _ = done_tx.send(true);
    let _ = saver.await;

    if let Some(msg) = job.fatal.lock().unwrap().clone() {
        job.save();
        return Outcome::Failed(msg);
    }
    if *stop.borrow() == PAUSE {
        job.save();
        return Outcome::Paused;
    }
    let complete = sequential || job.segs.lock().unwrap().is_complete();
    if !complete {
        job.save();
        return Outcome::Failed("The download stopped before every byte was in".into());
    }
    let size = job.state.size;
    if sequential {
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

/// One connection: works on a range until it is done, then takes another, until nothing is left to take.
async fn connection(job: Arc<Job>, id: usize, mut first: Option<(usize, reqwest::Response)>, sequential: bool) {
    let mut attempt = 0;
    loop {
        if job.is_stopped() {
            break;
        }
        let response = match first.take() {
            Some((_, r)) => r,
            None => {
                if job.segs.lock().unwrap().owned_by(id).is_none() && job.segs.lock().unwrap().take(id).is_none() {
                    break;
                }
                match job.open(id, sequential).await {
                    Ok(r) => r,
                    Err(p) => {
                        if !job.handle(id, p, &mut attempt).await {
                            break;
                        }
                        continue;
                    }
                }
            }
        };
        job.progress.active.fetch_add(1, Ordering::SeqCst);
        job.progress.peak.fetch_max(job.progress.active.load(Ordering::SeqCst), Ordering::SeqCst);
        let result = job.stream(id, response, sequential).await;
        job.progress.active.fetch_sub(1, Ordering::SeqCst);
        match result {
            Ok(()) => {
                attempt = 0;
                if sequential {
                    break;
                }
            }
            Err(p) => {
                if !job.handle(id, p, &mut attempt).await {
                    break;
                }
            }
        }
    }
    job.release(id);
}

impl Job {
    #[allow(clippy::too_many_arguments)]
    fn new(
        client: &reqwest::Client,
        url: reqwest::Url,
        req: &Request,
        tuning: Tuning,
        state: ResumeState,
        file: File,
        segs: Segments,
        progress: Arc<Progress>,
        stop: &watch::Receiver<u8>,
    ) -> Arc<Job> {
        let (stop_tx, _) = watch::channel(RUN);
        Arc::new(Job {
            client: client.clone(),
            url,
            tuning,
            retries: req.retries,
            validator: state.validator.clone(),
            state,
            state_file: req.state_file.clone(),
            file,
            segs: Mutex::new(segs),
            progress,
            failures: AtomicU32::new(0),
            fatal: Mutex::new(None),
            stop: merged_stop(stop.clone(), &stop_tx),
            stop_tx,
            refusal_gate: tokio::sync::Mutex::new(()),
        })
    }

    /// Saves the resume state, after making sure every byte it counts is on disk.
    fn save(&self) {
        let Some(path) = &self.state_file else { return };
        let segments = self.segs.lock().unwrap().all().to_vec();
        let _ = self.file.sync_data();
        let mut state = self.state.clone();
        state.segments = if state.resumable { segments } else { Vec::new() };
        let _ = state.save(path);
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

    fn fail(&self, msg: String) {
        self.fatal.lock().unwrap().get_or_insert(msg);
        let _ = self.stop_tx.send(ABORT);
    }

    /// Deals with a problem; returns whether the connection should keep going.
    async fn handle(&self, id: usize, problem: Problem, attempt: &mut usize) -> bool {
        match problem {
            Problem::Stopped => false,
            Problem::Stalled => {
                // The range goes back to the pool; this connection reconnects and takes the largest free range.
                self.release(id);
                true
            }
            Problem::Refused => {
                self.release(id);
                // One connection at a time waits and tries again; the others queue behind it.
                let _gate = self.refusal_gate.lock().await;
                sleep_unless_stopped(self.tuning.refusal_wait, &self.stop).await
            }
            Problem::FileChanged => {
                self.fail("File changed on the server".into());
                false
            }
            Problem::Permanent(msg) => {
                self.fail(msg);
                false
            }
            Problem::Transient(msg) => {
                self.release(id);
                let failures = self.failures.fetch_add(1, Ordering::SeqCst) + 1;
                if failures > self.retries {
                    self.fail(msg);
                    return false;
                }
                let delay = retry_delay(&self.tuning, *attempt);
                *attempt += 1;
                sleep_unless_stopped(delay, &self.stop).await
            }
        }
    }

    /// Opens a request for the rest of the range this connection owns.
    async fn open(&self, id: usize, sequential: bool) -> Result<reqwest::Response, Problem> {
        let mut req = self.client.get(self.url.clone()).header(ACCEPT_ENCODING, "identity");
        let (start, end) = if sequential {
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
        if !sequential {
            req = req.header(RANGE, format!("bytes={start}-{}", end - 1));
            if let Some(v) = &self.validator {
                req = req.header(IF_RANGE, v);
            }
        }
        let mut stop = self.stop.clone();
        let response = tokio::select! {
            r = req.send() => r.map_err(|e| Problem::Transient(error_text(&e)))?,
            _ = stopped_ref(&mut stop) => return Err(Problem::Stopped),
        };
        let status = response.status();
        if sequential {
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
    async fn stream(&self, id: usize, mut response: reqwest::Response, sequential: bool) -> Result<(), Problem> {
        let mut buf: Vec<u8> = Vec::with_capacity(self.tuning.write_buffer);
        let mut stop = self.stop.clone();
        loop {
            let next = tokio::select! {
                r = tokio::time::timeout(self.tuning.stall_timeout, response.chunk()) => r,
                _ = stopped_ref(&mut stop) => {
                    self.flush(id, &mut buf)?;
                    return Err(Problem::Stopped);
                }
            };
            match next {
                Err(_) => {
                    self.flush(id, &mut buf)?;
                    return Err(Problem::Stalled);
                }
                Ok(Err(e)) => {
                    self.flush(id, &mut buf)?;
                    return Err(Problem::Transient(error_text(&e)));
                }
                Ok(Ok(None)) => {
                    let done = self.flush(id, &mut buf)?;
                    return if done || sequential {
                        Ok(())
                    } else {
                        Err(Problem::Transient("The server closed the connection early".into()))
                    };
                }
                Ok(Ok(Some(bytes))) => {
                    buf.extend_from_slice(&bytes);
                    let (pos, end) = self.position(id).ok_or(Problem::Stopped)?;
                    if (buf.len() >= self.tuning.write_buffer || pos + buf.len() as u64 >= end)
                        && self.flush(id, &mut buf)? {
                            return Ok(());
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
    fn flush(&self, id: usize, buf: &mut Vec<u8>) -> Result<bool, Problem> {
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
            self.failures.store(0, Ordering::SeqCst);
        }
        buf.clear();
        let done = segs.get(idx).remaining() == 0;
        if done {
            segs.release(idx);
        }
        Ok(done)
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

fn error_text(e: &reqwest::Error) -> String {
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

/// The destination, or "name (1).ext", "name (2).ext"… when that name is taken (by a file or a partial download).
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
    (1..)
        .map(|i| folder.join(format!("{stem} ({i}){ext}")))
        .find(|p| !taken(p))
        .expect("some numbered name is free")
}
