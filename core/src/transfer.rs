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
use crate::segments::Segments;

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
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            min_split: 256 * 1024,
            stall_timeout: Duration::from_secs(20),
            retry_delays: [5, 10, 20, 40, 60].map(Duration::from_secs).to_vec(),
            refusal_wait: Duration::from_secs(60),
            write_buffer: 256 * 1024,
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
    file: File,
    segs: Mutex<Segments>,
    progress: Arc<Progress>,
    failures: AtomicU32,
    fatal: Mutex<Option<String>>,
    stop_tx: watch::Sender<u8>,
    stop: watch::Receiver<u8>,
    refusal_gate: tokio::sync::Mutex<()>,
}

/// Runs a download to the end, a pause or a failure.
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
    let validator = http::validator(first.headers());
    let name = req.name.clone().unwrap_or_else(|| http::file_name(first.headers(), first.url()));
    let dest = unique_path(&req.folder, &name);
    let part = part_path(&dest);
    *progress.name.lock().unwrap() = dest.file_name().map(|n| n.to_string_lossy().into_owned());
    progress.size.store(size.unwrap_or(u64::MAX), Ordering::SeqCst);
    progress.resumable.store(ranges && size.is_some(), Ordering::SeqCst);

    let file = match File::create(&part) {
        Ok(f) => f,
        Err(e) => return Outcome::Failed(format!("Can't create {}: {e}", part.display())),
    };
    if let Some(size) = size
        && let Err(e) = file.set_len(size) {
            return Outcome::Failed(format!("Can't make room for the file: {e}"));
        }

    // Ranged downloads are split over connections; without ranges or a size, one connection reads to the end.
    let segs = match (ranges, size) {
        (true, Some(size)) => Segments::new(size, tuning.min_split),
        _ => Segments::new(u64::MAX, u64::MAX),
    };
    let (stop_tx, _) = watch::channel(RUN);
    let job = Arc::new(Job {
        client: client.clone(),
        url,
        tuning,
        retries: req.retries,
        validator,
        file,
        segs: Mutex::new(segs),
        progress,
        failures: AtomicU32::new(0),
        fatal: Mutex::new(None),
        stop: merged_stop(stop.clone(), &stop_tx),
        stop_tx,
        refusal_gate: tokio::sync::Mutex::new(()),
    });

    let sequential = !(ranges && size.is_some());
    let first_idx = job.segs.lock().unwrap().take(0).expect("a new download has one free segment");
    let mut tasks = vec![tokio::spawn(connection(job.clone(), 0, Some((first_idx, first)), sequential))];
    if !sequential {
        for id in 1..max_conns {
            tasks.push(tokio::spawn(connection(job.clone(), id, None, false)));
        }
    }
    for t in tasks {
        let _ = t.await;
    }

    if let Some(msg) = job.fatal.lock().unwrap().clone() {
        return Outcome::Failed(msg);
    }
    if *stop.borrow() == PAUSE {
        return Outcome::Paused;
    }
    let complete = if sequential { true } else { job.segs.lock().unwrap().is_complete() };
    if !complete {
        return Outcome::Failed("The download stopped before every byte was in".into());
    }
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
    match std::fs::rename(&part, &dest) {
        Ok(()) => Outcome::Finished(dest),
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
