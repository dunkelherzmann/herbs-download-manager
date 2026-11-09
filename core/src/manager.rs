//! The download list (SPECIFICATION.md §2.2, §10): every download the user added, saved in `downloads.json`, each
//! running on the engine (`transfer`) when started. The apps ask for `rows()` a few times a second to draw the list.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::format::{self, FileKind};
use crate::transfer::{self, Control, Outcome, Progress, Request, ResumeState, Tuning};

/// Where a download stands. Only Waiting, Paused, Finished and Failed are saved; a download that was running when
/// the app quit or crashed comes back Paused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Waiting,
    Connecting,
    Downloading,
    Paused,
    Finished,
    Failed(String),
}

impl Status {
    pub fn is_running(&self) -> bool {
        matches!(self, Status::Connecting | Status::Downloading)
    }
}

/// One download as saved.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: u64,
    pub url: String,
    #[serde(default)]
    pub mirrors: Vec<String>,
    pub folder: PathBuf,
    /// The file name, once known (from the user or the server).
    pub name: Option<String>,
    /// The final path, once the first response chose it.
    pub dest: Option<PathBuf>,
    pub size: Option<u64>,
    pub downloaded: u64,
    pub status: Status,
    pub connections: u32,
    pub connections_per_server: u32,
    pub retries: u32,
    /// Seconds since 1970.
    pub added: u64,
    pub finished: Option<u64>,
}

/// What the list shows for one download.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: u64,
    pub name: String,
    pub kind: FileKind,
    pub status: Status,
    /// L2, e.g. "Downloading 47%".
    pub status_text: String,
    pub downloaded: u64,
    pub size: Option<u64>,
    /// 0.0–1.0, `None` while the size is unknown.
    pub fraction: Option<f64>,
    /// Bytes per second, 0 when not running.
    pub speed: u64,
    /// Seconds, when running with a known size and speed.
    pub time_left: Option<u64>,
    pub active_connections: u32,
    pub allowed_connections: u32,
    pub sources_in_use: u32,
    pub sources_known: u32,
    pub added: u64,
    pub finished: Option<u64>,
    pub folder: PathBuf,
    pub url: String,
    pub path: Option<PathBuf>,
    /// While running: each source with connections now, as "host — N connections" for the Connections menu.
    pub live: Vec<String>,
}

/// What `add` needs.
#[derive(Clone, Debug, Default)]
pub struct NewDownload {
    pub url: String,
    pub mirrors: Vec<String>,
    pub folder: PathBuf,
    pub name: Option<String>,
    pub connections: u32,
    pub connections_per_server: u32,
    pub retries: u32,
    /// Start now; otherwise it waits in the queue.
    pub start: bool,
}

struct Running {
    /// Restart (§10) was asked while it ran: once stopped, its data is discarded and it starts again.
    restart: std::sync::atomic::AtomicBool,
    control: Control,
    progress: Arc<Progress>,
    meter: Mutex<Meter>,
}

/// Speed over the last few seconds, smoothed so the number doesn't jump with every packet.
struct Meter {
    at: Instant,
    bytes: u64,
    speed: f64,
}

struct Slot {
    entry: Entry,
    running: Option<Arc<Running>>,
}

struct Inner {
    dir: PathBuf,
    slots: Mutex<Vec<Slot>>,
    tuning: Tuning,
}

/// The download list. Cheap to clone; every clone is the same list.
#[derive(Clone)]
pub struct Manager {
    inner: Arc<Inner>,
    runtime: Arc<tokio::runtime::Runtime>,
    client: reqwest::Client,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Manager {
    /// Opens the list kept in `dir` (created when missing). Downloads that were running come back Paused.
    pub fn open(dir: &Path) -> std::io::Result<Manager> {
        Self::open_with(dir, Tuning::default())
    }

    pub fn open_with(dir: &Path, tuning: Tuning) -> std::io::Result<Manager> {
        std::fs::create_dir_all(dir.join("state"))?;
        let entries: Vec<Entry> = match std::fs::read(dir.join("downloads.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(std::io::Error::other)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        let slots = entries
            .into_iter()
            .map(|mut entry| {
                if entry.status.is_running() {
                    entry.status = Status::Paused;
                }
                Slot { entry, running: None }
            })
            .collect();
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(4).build()?;
        let manager = Manager {
            inner: Arc::new(Inner { dir: dir.to_path_buf(), slots: Mutex::new(slots), tuning }),
            runtime: Arc::new(runtime),
            client: crate::http::client(),
        };
        manager.save();
        Ok(manager)
    }

    fn state_file(&self, id: u64) -> PathBuf {
        self.inner.dir.join("state").join(format!("{id}.json"))
    }

    /// Writes `downloads.json` (temporary file, then rename).
    fn save(&self) {
        let entries: Vec<Entry> = self.inner.slots.lock().unwrap().iter().map(|s| s.entry.clone()).collect();
        let Ok(text) = serde_json::to_vec_pretty(&entries) else { return };
        let path = self.inner.dir.join("downloads.json");
        let tmp = self.inner.dir.join("downloads.json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(tmp, path);
        }
    }

    /// Adds a download at the top of the list and returns its id.
    pub fn add(&self, new: NewDownload) -> u64 {
        let id = {
            let mut slots = self.inner.slots.lock().unwrap();
            let id = slots.iter().map(|s| s.entry.id).max().unwrap_or(0) + 1;
            slots.push(Slot {
                entry: Entry {
                    id,
                    url: new.url,
                    mirrors: new.mirrors,
                    folder: new.folder,
                    name: new.name,
                    dest: None,
                    size: None,
                    downloaded: 0,
                    status: Status::Waiting,
                    connections: new.connections.clamp(1, 32),
                    connections_per_server: new.connections_per_server.clamp(1, 32),
                    retries: new.retries,
                    added: now(),
                    finished: None,
                },
                running: None,
            });
            id
        };
        self.save();
        if new.start {
            self.resume(&[id]);
        }
        id
    }

    /// Starts or continues these downloads (finished and running ones are left alone).
    pub fn resume(&self, ids: &[u64]) {
        for &id in ids {
            self.start(id);
        }
        self.save();
    }

    fn start(&self, id: u64) {
        let (req, running) = {
            let mut slots = self.inner.slots.lock().unwrap();
            let Some(slot) = slots.iter_mut().find(|s| s.entry.id == id) else { return };
            if slot.running.is_some() || slot.entry.status == Status::Finished {
                return;
            }
            let e = &slot.entry;
            let req = Request {
                url: e.url.clone(),
                mirrors: e.mirrors.clone(),
                folder: e.folder.clone(),
                name: e.name.clone(),
                connections: e.connections,
                connections_per_server: e.connections_per_server,
                retries: e.retries,
                state_file: Some(self.state_file(id)),
            };
            let progress = Progress::new();
            progress.downloaded.store(e.downloaded, Ordering::SeqCst);
            let running = Arc::new(Running {
                restart: std::sync::atomic::AtomicBool::new(false),
                control: transfer::control(),
                progress,
                meter: Mutex::new(Meter { at: Instant::now(), bytes: e.downloaded, speed: 0.0 }),
            });
            slot.entry.status = Status::Connecting;
            slot.running = Some(running.clone());
            (req, running)
        };
        let this = self.clone();
        let tuning = self.inner.tuning.clone();
        let client = self.client.clone();
        self.runtime.spawn(async move {
            let outcome =
                transfer::download(&client, req, tuning, running.progress.clone(), running.control.clone()).await;
            this.finished(id, &running, outcome);
        });
    }

    /// The engine is done with a download: record how it ended.
    fn finished(&self, id: u64, running: &Arc<Running>, outcome: Outcome) {
        if running.restart.load(Ordering::SeqCst) {
            {
                let mut slots = self.inner.slots.lock().unwrap();
                if let Some(slot) = slots.iter_mut().find(|s| s.entry.id == id) {
                    slot.running = None;
                    if let Some(dest) = ResumeState::load(&self.state_file(id)).ok().map(|s| s.dest) {
                        slot.entry.dest = Some(dest);
                    }
                }
            }
            self.discard(id);
            self.resume(&[id]);
            return;
        }
        let state = ResumeState::load(&self.state_file(id)).ok();
        {
            let mut slots = self.inner.slots.lock().unwrap();
            let Some(slot) = slots.iter_mut().find(|s| s.entry.id == id) else { return };
            if !slot.running.as_ref().is_some_and(|r| Arc::ptr_eq(r, running)) {
                return; // removed, or started again
            }
            slot.running = None;
            let e = &mut slot.entry;
            e.downloaded = running.progress.downloaded.load(Ordering::SeqCst);
            e.size = running.progress.size().or(e.size);
            if let Some(name) = running.progress.name.lock().unwrap().clone() {
                e.name.get_or_insert(name);
            }
            if let Some(state) = state {
                e.dest = Some(state.dest);
            }
            match outcome {
                Outcome::Finished(path) => {
                    e.name = path.file_name().map(|n| n.to_string_lossy().into_owned()).or(e.name.take());
                    e.dest = Some(path);
                    e.size = Some(e.downloaded);
                    e.status = Status::Finished;
                    e.finished = Some(now());
                }
                Outcome::Paused => e.status = Status::Paused,
                Outcome::Failed(reason) => e.status = Status::Failed(reason),
            }
        }
        self.save();
    }

    /// Restart (§10): discards what was downloaded (and a finished file) and starts again from every source.
    pub fn restart(&self, ids: &[u64]) {
        let mut now_ids = Vec::new();
        {
            let slots = self.inner.slots.lock().unwrap();
            for slot in slots.iter().filter(|s| ids.contains(&s.entry.id)) {
                match &slot.running {
                    Some(r) => {
                        r.restart.store(true, Ordering::SeqCst);
                        r.control.pause();
                    }
                    None => now_ids.push(slot.entry.id),
                }
            }
        }
        for id in now_ids {
            self.discard(id);
        }
        self.resume(ids);
    }

    /// Forgets a stopped download's data: its resume state, its `.hdpart` file and its finished file.
    fn discard(&self, id: u64) {
        let state = self.state_file(id);
        let dest = ResumeState::load(&state).ok().map(|s| s.dest);
        let _ = std::fs::remove_file(&state);
        let mut slots = self.inner.slots.lock().unwrap();
        let Some(slot) = slots.iter_mut().find(|s| s.entry.id == id) else { return };
        let e = &mut slot.entry;
        if let Some(dest) = dest.or(e.dest.take()) {
            let _ = std::fs::remove_file(transfer::part_path(&dest));
            if e.status == Status::Finished {
                let _ = std::fs::remove_file(&dest);
            }
        }
        e.dest = None;
        e.size = None;
        e.downloaded = 0;
        e.finished = None;
        e.status = Status::Waiting;
    }

    /// Connections ▸ Add / Remove Connection: how many connections a download may use (1–32), applied at once
    /// when it runs.
    pub fn set_connections(&self, id: u64, n: u32) {
        let n = n.clamp(1, 32);
        {
            let mut slots = self.inner.slots.lock().unwrap();
            let Some(slot) = slots.iter_mut().find(|s| s.entry.id == id) else { return };
            slot.entry.connections = n;
            if let Some(r) = &slot.running {
                r.control.set_connections(n);
            }
        }
        self.save();
    }

    /// Pauses these downloads, keeping what's downloaded.
    pub fn pause(&self, ids: &[u64]) {
        let slots = self.inner.slots.lock().unwrap();
        for slot in slots.iter().filter(|s| ids.contains(&s.entry.id)) {
            if let Some(r) = &slot.running {
                r.control.pause();
            }
        }
    }

    /// Stop All (T6): pauses every running download.
    pub fn pause_all(&self) {
        let ids: Vec<u64> = self.inner.slots.lock().unwrap().iter().filter(|s| s.running.is_some()).map(|s| s.entry.id).collect();
        self.pause(&ids);
    }

    /// Whether any download is running.
    pub fn any_running(&self) -> bool {
        self.inner.slots.lock().unwrap().iter().any(|s| s.running.is_some())
    }

    /// Removes these downloads from the list; with `delete_files`, their file or `.hdpart` file too.
    pub fn remove(&self, ids: &[u64], delete_files: bool) {
        let removed: Vec<Slot> = {
            let mut slots = self.inner.slots.lock().unwrap();
            let (gone, kept): (Vec<Slot>, Vec<Slot>) = slots.drain(..).partition(|s| ids.contains(&s.entry.id));
            *slots = kept;
            gone
        };
        for slot in &removed {
            if let Some(r) = &slot.running {
                r.control.pause();
            }
            let _ = std::fs::remove_file(self.state_file(slot.entry.id));
            if delete_files && let Some(dest) = &slot.entry.dest {
                let _ = std::fs::remove_file(transfer::part_path(dest));
                if slot.entry.status == Status::Finished {
                    let _ = std::fs::remove_file(dest);
                }
            }
        }
        self.save();
    }

    /// Waits until no download is running (for tests and for quitting).
    pub fn wait_idle(&self, timeout: std::time::Duration) -> bool {
        let until = Instant::now() + timeout;
        while self.any_running() {
            if Instant::now() > until {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        true
    }

    /// The list, newest first.
    pub fn rows(&self) -> Vec<Row> {
        let slots = self.inner.slots.lock().unwrap();
        let mut rows: Vec<Row> = slots.iter().map(row).collect();
        rows.sort_by(|a, b| b.added.cmp(&a.added).then(b.id.cmp(&a.id)));
        rows
    }
}

fn row(slot: &Slot) -> Row {
    let e = &slot.entry;
    let mut status = e.status.clone();
    let (mut downloaded, mut size, mut speed, mut active) = (e.downloaded, e.size, 0u64, 0u32);
    let mut name = e.name.clone();
    let (mut in_use, mut known) = (0u32, 1 + e.mirrors.len() as u32);
    let mut live = Vec::new();
    if let Some(r) = &slot.running {
        let p = &r.progress;
        downloaded = p.downloaded.load(Ordering::SeqCst);
        size = p.size().or(size);
        active = p.active.load(Ordering::SeqCst) as u32;
        if name.is_none() {
            name = p.name.lock().unwrap().clone();
        }
        let sources = p.sources();
        live = sources
            .iter()
            .filter(|s| s.active > 0)
            .map(|s| {
                let host = reqwest::Url::parse(&s.url).ok().and_then(|u| u.host_str().map(|h| match u.port() {
                    Some(port) => format!("{h}:{port}"),
                    None => h.to_string(),
                }));
                let n = s.active;
                format!("{} — {n} connection{}", host.unwrap_or_else(|| s.url.clone()), if n == 1 { "" } else { "s" })
            })
            .collect();
        if !sources.is_empty() {
            known = sources.len() as u32;
            in_use = sources.iter().filter(|s| s.active > 0).count() as u32;
        }
        if active > 0 || downloaded > e.downloaded {
            status = Status::Downloading;
        }
        speed = r.meter.lock().unwrap().sample(downloaded);
    }
    let fraction = size.filter(|&s| s > 0).map(|s| (downloaded as f64 / s as f64).min(1.0));
    let time_left = match (size, speed) {
        (Some(s), v) if v > 0 && slot.running.is_some() => Some(s.saturating_sub(downloaded) / v),
        _ => None,
    };
    let name = name.unwrap_or_else(|| name_from_url(&e.url));
    Row {
        id: e.id,
        kind: FileKind::of(&name),
        status_text: format::status_text(&status, fraction),
        status,
        name,
        downloaded,
        size,
        fraction,
        speed,
        time_left,
        active_connections: active,
        allowed_connections: e.connections,
        sources_in_use: in_use,
        sources_known: known,
        added: e.added,
        finished: e.finished,
        folder: e.folder.clone(),
        url: e.url.clone(),
        path: e.dest.clone(),
        live,
    }
}

fn name_from_url(url: &str) -> String {
    reqwest::Url::parse(url)
        .map(|u| crate::http::file_name(&reqwest::header::HeaderMap::new(), &u))
        .unwrap_or_else(|_| url.to_string())
}

impl Meter {
    /// Bytes per second, averaged over about 3 seconds.
    fn sample(&mut self, bytes: u64) -> u64 {
        let elapsed = self.at.elapsed().as_secs_f64();
        if elapsed >= 0.25 {
            let now = bytes.saturating_sub(self.bytes) as f64 / elapsed;
            let weight = (elapsed / 3.0).min(1.0);
            self.speed = if self.speed == 0.0 { now } else { self.speed + (now - self.speed) * weight };
            self.at = Instant::now();
            self.bytes = bytes;
        }
        self.speed as u64
    }
}
