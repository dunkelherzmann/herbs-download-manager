//! The download list (SPECIFICATION.md §2.2, §10): every download the user added, saved in `downloads.json`, each
//! running on the engine (`transfer`) when started. The apps ask for `rows()` a few times a second to draw the list.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::format::{self, FileKind};
use crate::limit::Limiter;
use crate::settings::Settings;
use crate::transfer::{self, Control, Outcome, Progress, Request, ResumeState, Tuning};

/// Where a download stands. Only Waiting, Paused, Finished and Failed are saved; a download that was running when
/// the app quit or crashed comes back Paused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Waiting,
    Connecting,
    Downloading,
    /// Every byte is in; the file is being flushed to disk and given its name (instead of sitting at 99%).
    Finishing,
    Paused,
    Finished,
    Failed(String),
}

impl Status {
    pub fn is_running(&self) -> bool {
        matches!(self, Status::Connecting | Status::Downloading | Status::Finishing)
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
    /// Paused by quitting the app while it ran: it starts again at the next launch (§10).
    #[serde(default)]
    pub resume_at_launch: bool,
    /// This download's own speed limit in bytes per second (the Options tab), on top of S5.
    #[serde(default)]
    pub speed_limit: Option<u64>,
    /// The checksum to verify (the Options tab, §6).
    #[serde(default)]
    pub checksum: Option<crate::checksum::Checksum>,
    /// What the check found: "SHA-256 matches (f.bin.sha256)", or for a finished file checked later, a mismatch.
    #[serde(default)]
    pub verified: Option<String>,
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
    /// False when it runs on a source that can't resume: pausing it means starting over.
    pub resumable: bool,
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
    /// The size the server gave when the link was checked, if it was.
    pub size: Option<u64>,
}

/// What `add_or_mirror` did with a link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Added {
    /// A new download with this id.
    New(u64),
    /// The link names the same file as this unfinished download, so it became one of its mirrors.
    MirrorOf(u64),
}

/// What adding a link would do; see `Manager::plan_add`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    New,
    /// The same file as this unfinished download: the link becomes its mirror.
    Mirror(u64),
    /// A different file has the name: the user chooses Skip, Replace or Keep Both.
    Clash(Clash),
}

/// A download or a file in the folder with the name a new download would take, but not the same file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clash {
    /// The download in the list, if the name belongs to one.
    pub id: Option<u64>,
    /// Its file (or the file already in the folder), if there is one.
    pub path: Option<PathBuf>,
    pub size: Option<u64>,
    pub finished: bool,
    /// The name Keep Both gives the new download: "name - kept.ext", then "name - kept 2.ext" and so on.
    pub kept_name: String,
}

/// "report - kept.pdf", "report - kept 2.pdf", …: the first not used in the folder or by a download in the list.
fn kept_name(folder: &Path, name: &str, slots: &[Slot]) -> String {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    let used = |n: &str| {
        let p = folder.join(n);
        p.exists()
            || transfer::part_path(&p).exists()
            || slots.iter().any(|s| s.entry.folder == folder && s.entry.name.as_deref() == Some(n))
    };
    (1..)
        .map(|i| if i == 1 { format!("{stem} - kept{ext}") } else { format!("{stem} - kept {i}{ext}") })
        .find(|n| !used(n))
        .expect("some kept name is free")
}

impl Added {
    pub fn id(self) -> u64 {
        match self {
            Added::New(id) | Added::MirrorOf(id) => id,
        }
    }
}

struct Running {
    /// Restart (§10) was asked while it ran: once stopped, its data is discarded and it starts again.
    restart: std::sync::atomic::AtomicBool,
    control: Control,
    progress: Arc<Progress>,
    meter: Mutex<Meter>,
    /// The download's own speed limit; changing it takes effect at once.
    limiter: Arc<Limiter>,
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
    queue: Mutex<Queue>,
    settings: Mutex<Settings>,
    /// Where Delete puts files in tests; `None` is the system's Trash.
    trash_dir: Mutex<Option<PathBuf>>,
    /// Connection limits learned from servers (host-limits.json).
    learned: Arc<crate::hosts::Learned>,
    /// S5, shared by every download.
    limiter: Arc<Limiter>,
}

/// The queue (§11): whether it starts waiting downloads, and how many may run at once.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Queue {
    running: bool,
    /// Downloads at once, kept here before it moved to `settings.json` (read once, never written).
    #[serde(default, skip_serializing)]
    at_once: Option<u32>,
    /// Quitting: nothing new starts, but `running` is kept for the next launch.
    #[serde(skip)]
    quitting: bool,
}

/// The download list. Cheap to clone; every clone is the same list.
#[derive(Clone)]
pub struct Manager {
    inner: Arc<Inner>,
    /// Owned by the clones the app holds; the downloads' own tasks hold none (`_runtime: None`), so the runtime is
    /// never dropped from inside itself.
    _runtime: Option<Arc<tokio::runtime::Runtime>>,
    handle: tokio::runtime::Handle,
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
                // After a crash the list's numbers are older than the saved resume state: take them from it.
                if entry.status != Status::Finished
                    && let Ok(state) = ResumeState::load(&dir.join("state").join(format!("{}.json", entry.id)))
                {
                    entry.size = state.size.or(entry.size);
                    entry.dest = Some(state.dest);
                    if state.resumable {
                        entry.downloaded = state.segments.iter().map(|g| g.pos - g.start).sum();
                    }
                }
                Slot { entry, running: None }
            })
            .collect();
        let queue = std::fs::read(dir.join("queue.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Queue>(&b).ok())
            .unwrap_or(Queue { running: false, at_once: None, quitting: false });
        let settings = Settings::load(dir).unwrap_or_else(|| {
            let mut s = Settings::default();
            s.downloads_at_once = queue.at_once.unwrap_or(s.downloads_at_once);
            s.clamped()
        });
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(4).build()?;
        let manager = Manager {
            inner: Arc::new(Inner {
                dir: dir.to_path_buf(),
                slots: Mutex::new(slots),
                tuning,
                queue: Mutex::new(queue),
                limiter: Arc::new(Limiter::new(settings.speed_limit)),
                settings: Mutex::new(settings),
                trash_dir: Mutex::new(None),
                learned: Arc::new(crate::hosts::Learned::open(Some(dir))),
            }),
            handle: runtime.handle().clone(),
            _runtime: Some(Arc::new(runtime)),
            client: crate::http::client(),
        };
        manager.save();
        let again: Vec<u64> = manager
            .inner
            .slots
            .lock()
            .unwrap()
            .iter_mut()
            .filter_map(|s| std::mem::take(&mut s.entry.resume_at_launch).then_some(s.entry.id))
            .collect();
        manager.resume(&again);
        manager.pump();
        Ok(manager)
    }

    /// Quitting (§10): pauses every running download, marks it to start again at the next launch, and waits up
    /// to `timeout` for them to save. Returns false if some were still running.
    pub fn quit(&self, timeout: std::time::Duration) -> bool {
        self.inner.queue.lock().unwrap().quitting = true;
        {
            let mut slots = self.inner.slots.lock().unwrap();
            for slot in slots.iter_mut().filter(|s| s.running.is_some()) {
                slot.entry.resume_at_launch = true;
            }
        }
        self.pause_running();
        let stopped = self.wait_idle(timeout);
        self.save();
        stopped
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

    /// What adding a link would do (§7.1 M5, §5.6): become a mirror of an unfinished download of the same file (the
    /// same file name, and the same size when both are known, or the very same link), clash with a different file
    /// of that name in the list or in the folder, or be a new download.
    pub fn plan_add(&self, url: &str, name: Option<&str>, folder: &Path, size: Option<u64>) -> Plan {
        let wanted = name.map(str::to_string).unwrap_or_else(|| name_from_url(url));
        let slots = self.inner.slots.lock().unwrap();
        let name_of = |e: &Entry| e.name.clone().unwrap_or_else(|| name_from_url(&e.url));
        let unfinished = |s: &&Slot| s.entry.status != Status::Finished;
        let uses = |e: &Entry| transfer::same_link(&e.url, url) || e.mirrors.iter().any(|m| transfer::same_link(m, url));
        if let Some(s) = slots.iter().filter(unfinished).find(|s| uses(&s.entry)) {
            return Plan::Mirror(s.entry.id);
        }
        let same_size = |e: &Entry| !matches!((e.size, size), (Some(a), Some(b)) if a != b);
        if let Some(s) = slots.iter().filter(unfinished).find(|s| name_of(&s.entry) == wanted && same_size(&s.entry)) {
            return Plan::Mirror(s.entry.id);
        }
        let kept = || kept_name(folder, &wanted, &slots);
        if let Some(s) = slots.iter().rev().find(|s| s.entry.folder == folder && name_of(&s.entry) == wanted) {
            let e = &s.entry;
            return Plan::Clash(Clash {
                id: Some(e.id),
                path: e.dest.clone(),
                size: e.size,
                finished: e.status == Status::Finished,
                kept_name: kept(),
            });
        }
        let path = folder.join(&wanted);
        if path.exists() {
            let size = std::fs::metadata(&path).ok().map(|m| m.len());
            return Plan::Clash(Clash { id: None, path: Some(path), size, finished: true, kept_name: kept() });
        }
        Plan::New
    }

    /// Adds a link: as a mirror of an unfinished download of the same file, otherwise as a new download (a clash
    /// is added as new; the apps ask first with `plan_add`). Mirrors are checked before use, so a link that turns
    /// out to be another file is dropped as "Different file" without harming the download.
    pub fn add_or_mirror(&self, new: NewDownload) -> Added {
        let plan = self.plan_add(&new.url, new.name.as_deref(), &new.folder, new.size);
        let Plan::Mirror(id) = plan else { return Added::New(self.add(new)) };
        let mut urls = vec![new.url.clone()];
        urls.extend(new.mirrors.iter().cloned());
        self.add_mirrors(id, &urls);
        if new.start {
            self.resume(&[id]);
        }
        Added::MirrorOf(id)
    }

    /// Adds a new download at the top of the list and returns its id.
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
                    size: new.size,
                    downloaded: 0,
                    status: Status::Waiting,
                    connections: new.connections.clamp(1, 32),
                    connections_per_server: new.connections_per_server.clamp(1, 32),
                    retries: new.retries,
                    added: now(),
                    finished: None,
                    resume_at_launch: false,
                    speed_limit: None,
                    checksum: None,
                    verified: None,
                },
                running: None,
            });
            id
        };
        self.save();
        if new.start {
            self.resume(&[id]);
        } else {
            self.pump();
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
            let settings = self.inner.settings.lock().unwrap().clone();
            let limiter = Arc::new(Limiter::new(e.speed_limit));
            let req = Request {
                url: e.url.clone(),
                mirrors: e.mirrors.clone(),
                folder: e.folder.clone(),
                name: e.name.clone(),
                connections: e.connections,
                // S3 and S6 apply to every download from its next start.
                connections_per_server: settings.connections_per_server,
                retries: settings.retries,
                state_file: Some(self.state_file(id)),
                limits: vec![self.inner.limiter.clone(), limiter.clone()],
                look_for_checksum_files: settings.look_for_checksum_files,
                learned: self.inner.learned.clone(),
            };
            let progress = Progress::new();
            progress.downloaded.store(e.downloaded, Ordering::SeqCst);
            *progress.checksum.lock().unwrap() = e.checksum.clone();
            let running = Arc::new(Running {
                restart: std::sync::atomic::AtomicBool::new(false),
                control: transfer::control(),
                progress,
                meter: Mutex::new(Meter { at: Instant::now(), bytes: e.downloaded, speed: 0.0 }),
                limiter,
            });
            slot.entry.status = Status::Connecting;
            slot.running = Some(running.clone());
            (req, running)
        };
        let this = Manager { _runtime: None, ..self.clone() };
        let tuning = self.inner.tuning.clone();
        let client = self.client.clone();
        self.handle.spawn(async move {
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
            // Only a download that quitting paused starts again at launch.
            e.resume_at_launch &= matches!(outcome, Outcome::Paused);
            match outcome {
                Outcome::Finished(path) => {
                    e.verified = running.progress.verified.lock().unwrap().clone();
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
        self.pump();
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

    /// Stop All (T6): stops the queue and pauses every running download.
    pub fn pause_all(&self) {
        self.stop_queue();
        self.pause_running();
    }

    fn pause_running(&self) {
        let ids: Vec<u64> = self.inner.slots.lock().unwrap().iter().filter(|s| s.running.is_some()).map(|s| s.entry.id).collect();
        self.pause(&ids);
    }

    /// Whether any download is running.
    pub fn any_running(&self) -> bool {
        self.inner.slots.lock().unwrap().iter().any(|s| s.running.is_some())
    }

    /// Removes these downloads from the list; with `delete_files`, their file or `.hdpart` file too.
    /// Delete (§10): removes the entries, and with `delete_files` sends their files (the finished file, or the
    /// `.hdpart` of an unfinished one) to the Trash, never erasing them. Returns the files that couldn't go to the
    /// Trash (a volume without one): they are left where they are.
    pub fn remove(&self, ids: &[u64], delete_files: bool) -> Vec<PathBuf> {
        let removed: Vec<Slot> = {
            let mut slots = self.inner.slots.lock().unwrap();
            let (gone, kept): (Vec<Slot>, Vec<Slot>) = slots.drain(..).partition(|s| ids.contains(&s.entry.id));
            *slots = kept;
            gone
        };
        let mut kept = Vec::new();
        for slot in &removed {
            if let Some(r) = &slot.running {
                r.control.pause();
            }
            let _ = std::fs::remove_file(self.state_file(slot.entry.id));
            if delete_files && let Some(dest) = &slot.entry.dest {
                let part = transfer::part_path(dest);
                for file in [Some(part), (slot.entry.status == Status::Finished).then(|| dest.clone())].into_iter().flatten() {
                    if file.exists() && self.trash(&file).is_err() {
                        kept.push(file);
                    }
                }
            }
        }
        self.save();
        self.pump();
        kept
    }

    /// Moves a file to the system's Trash (the Recycle Bin on Windows), or into the test trash folder when one is set.
    fn trash(&self, file: &Path) -> std::io::Result<()> {
        // A running download may still have its file open for a moment after the pause.
        let attempt = || match self.inner.trash_dir.lock().unwrap().clone() {
            Some(dir) => {
                std::fs::create_dir_all(&dir)?;
                let name = file.file_name().ok_or_else(|| std::io::Error::other("no file name"))?;
                std::fs::rename(file, dir.join(name))
            }
            None => trash::delete(file).map_err(std::io::Error::other),
        };
        let mut result = attempt();
        for _ in 0..20 {
            if result.is_ok() || !file.exists() {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
            result = attempt();
        }
        result
    }

    /// Servers' remembered connection limits, by host ("example.com:443"), for Settings.
    pub fn host_limits(&self) -> Vec<(String, u32)> {
        self.inner.learned.all().into_iter().map(|(h, l)| (h, l.connections)).collect()
    }

    /// Settings ▸ Forget: downloads learn every server's limit again.
    pub fn forget_host_limits(&self) {
        self.inner.learned.forget();
    }

    /// Tests and test copies of the app: "the Trash" is this folder, so they never fill the user's real Trash.
    pub fn use_trash_folder(&self, dir: &Path) {
        *self.inner.trash_dir.lock().unwrap() = Some(dir.to_path_buf());
    }

    /// Start Queue (T4): starts waiting downloads, oldest first, while fewer than "Downloads at once" run.
    pub fn start_queue(&self) {
        self.set_queue(|q| q.running = true);
    }

    /// Stop Queue (T5): starts no more waiting downloads; running ones carry on.
    pub fn stop_queue(&self) {
        self.set_queue(|q| q.running = false);
    }

    pub fn queue_running(&self) -> bool {
        self.inner.queue.lock().unwrap().running
    }

    /// Downloads at once (S4, 1–10).
    pub fn set_downloads_at_once(&self, n: u32) {
        let s = Settings { downloads_at_once: n, ..self.settings() };
        self.set_settings(s);
    }

    /// The settings (§9) as saved.
    pub fn settings(&self) -> Settings {
        self.inner.settings.lock().unwrap().clone()
    }

    /// Saves the settings and applies them: the speed limit at once, downloads at once to the queue, connections
    /// per server and retries to each download from its next start.
    pub fn set_settings(&self, s: Settings) {
        let s = s.clamped();
        let _ = s.save(&self.inner.dir);
        self.inner.limiter.set_rate(s.speed_limit);
        *self.inner.settings.lock().unwrap() = s;
        self.pump();
    }

    /// The checksum to verify (the Options tab, §6), `None` for none. A running download checks it before naming
    /// its file; a finished one is checked now, on another thread, and its note says whether it matches.
    pub fn set_checksum(&self, id: u64, checksum: Option<crate::checksum::Checksum>) {
        let finished_file = {
            let mut slots = self.inner.slots.lock().unwrap();
            let Some(slot) = slots.iter_mut().find(|s| s.entry.id == id) else { return };
            slot.entry.checksum = checksum.clone();
            slot.entry.verified = None;
            if let Some(r) = &slot.running {
                *r.progress.checksum.lock().unwrap() = checksum.clone();
            }
            (slot.entry.status == Status::Finished).then(|| slot.entry.dest.clone()).flatten()
        };
        self.save();
        if let (Some(path), Some(expected)) = (finished_file, checksum) {
            let this = Manager { _runtime: None, ..self.clone() };
            std::thread::spawn(move || {
                let name = expected.algorithm.name();
                let note = match crate::checksum::hash_file(&path, expected.algorithm) {
                    Ok(got) if got == expected.value => format!("{name} matches"),
                    Ok(_) => format!("{name} doesn't match: the file is damaged or isn't the expected one"),
                    Err(e) => format!("Can't read the file to check it: {e}"),
                };
                {
                    let mut slots = this.inner.slots.lock().unwrap();
                    if let Some(slot) = slots.iter_mut().find(|s| s.entry.id == id && s.entry.checksum.as_ref() == Some(&expected)) {
                        slot.entry.verified = Some(note);
                    }
                }
                this.save();
            });
        }
    }

    /// One download's own speed limit (the Options tab), in bytes per second; `None` is none. Applies at once.
    pub fn set_speed_limit(&self, id: u64, limit: Option<u64>) {
        let limit = limit.filter(|&l| l > 0);
        {
            let mut slots = self.inner.slots.lock().unwrap();
            let Some(slot) = slots.iter_mut().find(|s| s.entry.id == id) else { return };
            slot.entry.speed_limit = limit;
            if let Some(r) = &slot.running {
                r.limiter.set_rate(limit);
            }
        }
        self.save();
    }

    fn set_queue(&self, change: impl FnOnce(&mut Queue)) {
        let q = {
            let mut q = self.inner.queue.lock().unwrap();
            change(&mut q);
            *q
        };
        if let Ok(text) = serde_json::to_vec_pretty(&q) {
            let tmp = self.inner.dir.join("queue.json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(tmp, self.inner.dir.join("queue.json"));
            }
        }
        self.pump();
    }

    /// Starts the oldest waiting downloads while the queue runs and has room. The queue lock is held throughout,
    /// so two calls can't both fill the same free place.
    fn pump(&self) {
        let q = self.inner.queue.lock().unwrap();
        if !q.running || q.quitting {
            return;
        }
        let next: Vec<u64> = {
            let slots = self.inner.slots.lock().unwrap();
            let busy = slots.iter().filter(|s| s.running.is_some()).count();
            let at_once = self.inner.settings.lock().unwrap().downloads_at_once;
            let free = (at_once as usize).saturating_sub(busy);
            let mut waiting: Vec<&Slot> =
                slots.iter().filter(|s| s.running.is_none() && s.entry.status == Status::Waiting).collect();
            waiting.sort_by_key(|s| s.entry.id);
            waiting.into_iter().take(free).map(|s| s.entry.id).collect()
        };
        for id in &next {
            self.start(*id);
        }
        drop(q);
        if !next.is_empty() {
            self.save();
        }
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
    let mut resumable = true;
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
            // Mirrors just added count at once, before the engine takes them in at its next tick.
            known = (sources.len() as u32).max(known);
            in_use = sources.iter().filter(|s| s.active > 0).count() as u32;
        }
        if active > 0 || downloaded > e.downloaded {
            status = Status::Downloading;
            resumable = p.resumable.load(Ordering::SeqCst);
        }
        if p.finishing.load(Ordering::SeqCst) || size.is_some_and(|s| s > 0 && downloaded >= s) {
            status = Status::Finishing;
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
        resumable,
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

/// What the New Download window shows once the first source answers (A3, A6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    pub name: String,
    pub size: Option<u64>,
    pub resumable: bool,
}

impl Manager {
    /// Asks the server about a link with a one-byte request: the file's name, size and whether it can resume.
    /// Blocks the calling thread (never the app's main thread) for up to 20 seconds.
    pub fn probe(&self, url: &str) -> Result<Probe, String> {
        let parsed = reqwest::Url::parse(url.trim()).map_err(|_| "That isn't a web link".to_string())?;
        let client = self.client.clone();
        self.handle.block_on(async move {
            let sent = client
                .get(parsed)
                .header(reqwest::header::RANGE, "bytes=0-0")
                .header(reqwest::header::ACCEPT_ENCODING, "identity")
                .send();
            let response = tokio::time::timeout(std::time::Duration::from_secs(20), sent)
                .await
                .map_err(|_| "The server didn't answer".to_string())?
                .map_err(|e| transfer::error_text(&e))?;
            let status = response.status();
            if !status.is_success() {
                return Err(format!("{} {}", status.as_u16(), status.canonical_reason().unwrap_or("")).trim().to_string());
            }
            let ranged = status == reqwest::StatusCode::PARTIAL_CONTENT;
            let size = if ranged { crate::http::content_range_total(response.headers()) } else { response.content_length() };
            Ok(Probe {
                name: crate::http::file_name(response.headers(), response.url()),
                size,
                resumable: ranged && size.is_some(),
            })
        })
    }
}

/// The name a new file gets in `folder`: `name` itself, or "name (2).ext"… when taken (§5.1).
pub fn free_name(folder: &Path, name: &str) -> String {
    transfer::unique_path(folder, name).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| name.to_string())
}

/// What the Download window shows (§6): the segments, the connections and the sources.
#[derive(Clone, Debug)]
pub struct Detail {
    pub row: Row,
    pub segments: Vec<crate::segments::Segment>,
    pub connections: Vec<transfer::ConnInfo>,
    pub sources: Vec<transfer::SourceInfo>,
    /// The download's own speed limit, bytes per second.
    pub speed_limit: Option<u64>,
    pub checksum: Option<crate::checksum::Checksum>,
    /// "SHA-256 matches (f.bin.sha256)", or a finished file's mismatch.
    pub verified: Option<String>,
}

impl Manager {
    /// The Download window's view of a download: live while it runs, from its saved state otherwise.
    pub fn detail(&self, id: u64) -> Option<Detail> {
        let slots = self.inner.slots.lock().unwrap();
        let slot = slots.iter().find(|s| s.entry.id == id)?;
        let row = row(slot);
        if let Some(r) = &slot.running {
            let p = &r.progress;
            let mut sources = p.sources();
            if sources.is_empty() {
                sources = saved_sources(&slot.entry, None);
            }
            return Some(Detail {
                row,
                segments: p.segments.lock().unwrap().clone(),
                connections: p.connections.lock().unwrap().clone(),
                sources,
                speed_limit: slot.entry.speed_limit,
                checksum: slot.entry.checksum.clone(),
                verified: slot.entry.verified.clone(),
            });
        }
        let state = ResumeState::load(&self.state_file(id)).ok();
        let segments = match (&state, slot.entry.status == Status::Finished, slot.entry.size) {
            (_, true, Some(size)) => vec![crate::segments::Segment { start: 0, end: size, pos: size, owner: None }],
            (Some(s), _, _) => s.segments.clone(),
            _ => Vec::new(),
        };
        Some(Detail {
            sources: saved_sources(&slot.entry, state.as_ref()),
            row,
            segments,
            connections: Vec::new(),
            speed_limit: slot.entry.speed_limit,
            checksum: slot.entry.checksum.clone(),
            verified: slot.entry.verified.clone(),
        })
    }

    /// Add Mirror… (§7.1): M2 on a running download (checked at once, then used without a pause), M3 on a
    /// stopped one (saved and used when it resumes). Links already among its sources are ignored.
    pub fn add_mirrors(&self, id: u64, urls: &[String]) -> usize {
        let added = {
            let mut slots = self.inner.slots.lock().unwrap();
            let Some(slot) = slots.iter_mut().find(|s| s.entry.id == id) else { return 0 };
            let e = &mut slot.entry;
            let mut added = Vec::new();
            for url in urls.iter().map(|u| u.trim()).filter(|u| reqwest::Url::parse(u).is_ok()) {
                let known = transfer::same_link(&e.url, url) || e.mirrors.iter().any(|m| transfer::same_link(m, url));
                if !known && !added.iter().any(|a: &String| transfer::same_link(a, url)) {
                    added.push(url.to_string());
                }
            }
            e.mirrors.extend(added.iter().cloned());
            match &slot.running {
                Some(r) => r.control.add_mirrors(added.clone()),
                None => {
                    let _ = transfer::add_mirrors_to_state(&self.state_file(id), &added);
                }
            }
            added.len()
        };
        self.save();
        added
    }

    /// Remove (Mirrors tab): drops a mirror; its connections move to the other sources. The original link stays.
    pub fn remove_source(&self, id: u64, url: &str) {
        {
            let mut slots = self.inner.slots.lock().unwrap();
            let Some(slot) = slots.iter_mut().find(|s| s.entry.id == id) else { return };
            if transfer::same_link(&slot.entry.url, url) {
                return;
            }
            slot.entry.mirrors.retain(|m| !transfer::same_link(m, url));
            match &slot.running {
                Some(r) => r.control.remove_source(url),
                None => self.edit_state(id, |s| s.sources.retain(|x| !transfer::same_link(&x.url, url))),
            }
        }
        self.save();
    }

    /// The on/off checkbox of a source (§7.3). At least one source stays on.
    pub fn set_source_enabled(&self, id: u64, url: &str, on: bool) {
        let slots = self.inner.slots.lock().unwrap();
        let Some(slot) = slots.iter().find(|s| s.entry.id == id) else { return };
        match &slot.running {
            Some(r) => r.control.set_source_enabled(url, on),
            None => self.edit_state(id, |s| {
                let others_on = s.sources.iter().any(|x| !transfer::same_link(&x.url, url) && x.enabled);
                if let Some(x) = s.sources.iter_mut().find(|x| transfer::same_link(&x.url, url))
                    && (on || others_on)
                {
                    x.enabled = on;
                }
            }),
        }
    }

    fn edit_state(&self, id: u64, change: impl FnOnce(&mut ResumeState)) {
        let path = self.state_file(id);
        if let Ok(mut state) = ResumeState::load(&path) {
            change(&mut state);
            let _ = state.save(&path);
        }
    }
}

/// The sources of a stopped download: from its saved state, else its link and mirrors (not checked yet).
fn saved_sources(e: &Entry, state: Option<&ResumeState>) -> Vec<transfer::SourceInfo> {
    let info = |url: &str, status: transfer::SourceStatus, enabled: bool| transfer::SourceInfo {
        url: url.to_string(),
        status,
        enabled,
        active: 0,
        speed: 0,
        bytes: 0,
    };
    match state {
        Some(s) if !s.sources.is_empty() => s
            .sources
            .iter()
            .map(|x| {
                let status = match &x.status {
                    transfer::SourceStatus::Working | transfer::SourceStatus::Slow => transfer::SourceStatus::Idle,
                    other => other.clone(),
                };
                info(&x.url, status, x.enabled)
            })
            .collect(),
        _ => std::iter::once(&e.url)
            .chain(e.mirrors.iter())
            .map(|u| info(u, transfer::SourceStatus::Idle, true))
            .collect(),
    }
}
