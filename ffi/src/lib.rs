//! The Swift-facing API of hd-core, generated into Swift by UniFFI for the macOS app.
//! The Windows and Linux apps are Rust and call hd-core directly.

uniffi::setup_scaffolding!();

use hd_core::settings;

#[derive(uniffi::Enum)]
pub enum WhenFileExists {
    Ask,
    NumberedName,
    Replace,
}

#[derive(uniffi::Record)]
pub struct Settings {
    pub save_to: Option<String>,
    pub connections: u32,
    pub connections_per_server: u32,
    pub downloads_at_once: u32,
    pub speed_limit: Option<u64>,
    pub retries: u32,
    pub when_file_exists: WhenFileExists,
    pub look_for_checksum_files: bool,
    pub notify_when_finished: bool,
}

impl From<settings::Settings> for Settings {
    fn from(s: settings::Settings) -> Self {
        Settings {
            save_to: s.save_to,
            connections: s.connections,
            connections_per_server: s.connections_per_server,
            downloads_at_once: s.downloads_at_once,
            speed_limit: s.speed_limit,
            retries: s.retries,
            when_file_exists: match s.when_file_exists {
                settings::WhenFileExists::Ask => WhenFileExists::Ask,
                settings::WhenFileExists::NumberedName => WhenFileExists::NumberedName,
                settings::WhenFileExists::Replace => WhenFileExists::Replace,
            },
            look_for_checksum_files: s.look_for_checksum_files,
            notify_when_finished: s.notify_when_finished,
        }
    }
}

#[uniffi::export]
pub fn core_version() -> String {
    hd_core::version().to_string()
}

#[uniffi::export]
pub fn default_settings() -> Settings {
    settings::Settings::default().into()
}

/// The choices in the connections picker (1 to 32).
#[uniffi::export]
pub fn connection_choices() -> Vec<u32> {
    settings::CONNECTIONS.collect()
}

// --- The download list ------------------------------------------------------------------------------------------

use hd_core::format::{self, FileKind as CoreKind, Filter as CoreFilter};
use hd_core::manager::{self, Manager};

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Compressed,
    Documents,
    Music,
    Pictures,
    Video,
    Programs,
    Other,
}

impl From<CoreKind> for FileKind {
    fn from(k: CoreKind) -> Self {
        match k {
            CoreKind::Compressed => FileKind::Compressed,
            CoreKind::Documents => FileKind::Documents,
            CoreKind::Music => FileKind::Music,
            CoreKind::Pictures => FileKind::Pictures,
            CoreKind::Video => FileKind::Video,
            CoreKind::Programs => FileKind::Programs,
            CoreKind::Other => FileKind::Other,
        }
    }
}

impl From<FileKind> for CoreKind {
    fn from(k: FileKind) -> Self {
        CoreKind::ALL.into_iter().find(|c| FileKind::from(*c) == k).unwrap_or(CoreKind::Other)
    }
}

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Downloading,
    Paused,
    Finished,
    Unfinished,
}

impl From<Filter> for CoreFilter {
    fn from(f: Filter) -> Self {
        match f {
            Filter::All => CoreFilter::All,
            Filter::Downloading => CoreFilter::Downloading,
            Filter::Paused => CoreFilter::Paused,
            Filter::Finished => CoreFilter::Finished,
            Filter::Unfinished => CoreFilter::Unfinished,
        }
    }
}

#[derive(uniffi::Enum, Clone, PartialEq, Eq)]
pub enum DownloadStatus {
    Waiting,
    Connecting,
    Downloading,
    Paused,
    Finished,
    Failed { reason: String },
}

impl From<manager::Status> for DownloadStatus {
    fn from(s: manager::Status) -> Self {
        match s {
            manager::Status::Waiting => DownloadStatus::Waiting,
            manager::Status::Connecting => DownloadStatus::Connecting,
            manager::Status::Downloading => DownloadStatus::Downloading,
            manager::Status::Paused => DownloadStatus::Paused,
            manager::Status::Finished => DownloadStatus::Finished,
            manager::Status::Failed(reason) => DownloadStatus::Failed { reason },
        }
    }
}

/// One line of the download list, with its texts ready to show.
#[derive(uniffi::Record)]
pub struct DownloadRow {
    pub id: u64,
    pub name: String,
    pub kind: FileKind,
    pub status: DownloadStatus,
    pub status_text: String,
    pub downloaded: u64,
    pub size: Option<u64>,
    pub size_text: String,
    /// 0–1, absent while the size is unknown.
    pub fraction: Option<f64>,
    pub speed: u64,
    pub speed_text: String,
    pub time_left: Option<u64>,
    pub time_left_text: String,
    pub active_connections: u32,
    pub allowed_connections: u32,
    pub sources_in_use: u32,
    pub sources_known: u32,
    /// Seconds since 1970.
    pub added: u64,
    pub finished: Option<u64>,
    pub folder: String,
    pub url: String,
    pub path: Option<String>,
    /// While running: "host — N connections" per source in use.
    pub live: Vec<String>,
    /// False when pausing it means starting over (no source can resume).
    pub resumable: bool,
}

impl From<manager::Row> for DownloadRow {
    fn from(r: manager::Row) -> Self {
        let done = r.status == manager::Status::Finished;
        DownloadRow {
            id: r.id,
            kind: r.kind.into(),
            status_text: r.status_text,
            status: r.status.into(),
            size_text: format::size_text(r.downloaded, r.size, done),
            speed_text: format::speed_text(r.speed),
            time_left_text: format::time_left_text(r.time_left),
            name: r.name,
            downloaded: r.downloaded,
            size: r.size,
            fraction: r.fraction,
            speed: r.speed,
            time_left: r.time_left,
            active_connections: r.active_connections,
            allowed_connections: r.allowed_connections,
            sources_in_use: r.sources_in_use,
            sources_known: r.sources_known,
            added: r.added,
            finished: r.finished,
            folder: r.folder.to_string_lossy().into_owned(),
            url: r.url,
            path: r.path.map(|p| p.to_string_lossy().into_owned()),
            live: r.live,
            resumable: r.resumable,
        }
    }
}

#[derive(Debug, uniffi::Error, thiserror::Error)]
pub enum CoreError {
    #[error("{message}")]
    Io { message: String },
    /// A message to show as it is ("404 Not Found", "That isn't a web link").
    #[error("{message}")]
    Message { message: String },
}

/// What the server says about a link (New Download's Name and Size, A3 and A6).
#[derive(uniffi::Record)]
pub struct LinkInfo {
    pub name: String,
    pub size: Option<u64>,
    pub resumable: bool,
}

/// The download list, kept in a folder (the app's data folder).
#[derive(uniffi::Object)]
pub struct DownloadManager {
    inner: Manager,
}

#[uniffi::export]
impl DownloadManager {
    #[uniffi::constructor]
    pub fn open(data_dir: String) -> Result<std::sync::Arc<Self>, CoreError> {
        Manager::open(std::path::Path::new(&data_dir))
            .map(|inner| std::sync::Arc::new(DownloadManager { inner }))
            .map_err(|e| CoreError::Io { message: e.to_string() })
    }

    /// Adds a link and returns its id; `start` runs it at once, otherwise it waits in the queue.
    pub fn add(&self, url: String, mirrors: Vec<String>, folder: String, name: Option<String>, connections: u32, start: bool) -> u64 {
        let s = settings::Settings::default();
        self.inner.add(manager::NewDownload {
            url,
            mirrors,
            folder: folder.into(),
            name,
            connections,
            connections_per_server: s.connections_per_server,
            retries: s.retries,
            start,
        })
    }

    pub fn resume(&self, ids: Vec<u64>) {
        self.inner.resume(&ids);
    }

    pub fn pause(&self, ids: Vec<u64>) {
        self.inner.pause(&ids);
    }

    /// Stop All: stops the queue and pauses every running download.
    pub fn pause_all(&self) {
        self.inner.pause_all();
    }

    /// Start Queue: starts waiting downloads, oldest first, while fewer than "Downloads at once" run.
    pub fn start_queue(&self) {
        self.inner.start_queue();
    }

    /// Stop Queue: starts no more waiting downloads; running ones carry on.
    pub fn stop_queue(&self) {
        self.inner.stop_queue();
    }

    pub fn queue_running(&self) -> bool {
        self.inner.queue_running()
    }

    /// Downloads at once (1–10).
    pub fn set_downloads_at_once(&self, n: u32) {
        self.inner.set_downloads_at_once(n);
    }

    pub fn any_running(&self) -> bool {
        self.inner.any_running()
    }

    /// Quitting: pauses running downloads, marks them to start again at the next launch, and waits up to
    /// `seconds` for them to save. False if some were still running.
    pub fn quit(&self, seconds: f64) -> bool {
        self.inner.quit(std::time::Duration::from_secs_f64(seconds))
    }

    /// Waits up to `seconds` for every download to stop; false if some still run.
    pub fn wait_idle(&self, seconds: f64) -> bool {
        self.inner.wait_idle(std::time::Duration::from_secs_f64(seconds))
    }

    /// Asks the server for the link's name, size and resume support. Blocks: call it off the main thread.
    pub fn probe(&self, url: String) -> Result<LinkInfo, CoreError> {
        self.inner
            .probe(&url)
            .map(|p| LinkInfo { name: p.name, size: p.size, resumable: p.resumable })
            .map_err(|message| CoreError::Message { message })
    }

    /// The Download window's view (§6), or nothing if the download was removed.
    pub fn detail(&self, id: u64) -> Option<DownloadDetail> {
        let d = self.inner.detail(id)?;
        let original = d.row.url.clone();
        Some(DownloadDetail {
            segments: d.segments.iter().map(|s| SegmentInfo { start: s.start, end: s.end, pos: s.pos, active: s.owner.is_some() }).collect(),
            connections: d
                .connections
                .into_iter()
                .map(|c| ConnectionRow {
                    id: c.id as u32,
                    host: reqwest_host(&c.source),
                    range_text: match (c.start, c.end) {
                        (_, 0) => String::new(),
                        (0, end) => format!("0–{}", format::bytes(end)),
                        (start, end) => format!("{}–{}", format::bytes(start), format::bytes(end)),
                    },
                    downloaded_text: if c.bytes > 0 { format::bytes(c.bytes) } else { String::new() },
                    speed_text: format::speed_text(c.speed),
                    state: c.state,
                })
                .collect(),
            sources: d
                .sources
                .into_iter()
                .map(|s| SourceRow {
                    is_original: s.url == original,
                    status_text: s.status.text(),
                    url: s.url,
                    enabled: s.enabled,
                    active: s.active as u32,
                    speed_text: format::speed_text(s.speed),
                    bytes_text: if s.bytes > 0 { format::bytes(s.bytes) } else { String::new() },
                })
                .collect(),
            row: d.row.into(),
        })
    }

    /// Add Mirror… (§7.1): used at once when running, saved for later otherwise. Returns how many were new.
    pub fn add_mirrors(&self, id: u64, urls: Vec<String>) -> u32 {
        self.inner.add_mirrors(id, &urls) as u32
    }

    /// Remove a mirror (the original link stays).
    pub fn remove_source(&self, id: u64, url: String) {
        self.inner.remove_source(id, &url);
    }

    /// A source's on/off checkbox; at least one stays on.
    pub fn set_source_enabled(&self, id: u64, url: String, enabled: bool) {
        self.inner.set_source_enabled(id, &url, enabled);
    }

    /// Discards the downloaded data and starts again (§10).
    pub fn restart(&self, ids: Vec<u64>) {
        self.inner.restart(&ids);
    }

    /// How many connections a download may use (1–32), applied at once when running.
    pub fn set_connections(&self, id: u64, connections: u32) {
        self.inner.set_connections(id, connections);
    }

    pub fn remove(&self, ids: Vec<u64>, delete_files: bool) {
        self.inner.remove(&ids, delete_files);
    }

    /// The whole list, newest first.
    pub fn rows(&self) -> Vec<DownloadRow> {
        self.inner.rows().into_iter().map(Into::into).collect()
    }
}

/// Whether a row shows under a sidebar filter.
#[uniffi::export]
pub fn filter_matches(filter: Filter, status: DownloadStatus) -> bool {
    let s = match status {
        DownloadStatus::Waiting => manager::Status::Waiting,
        DownloadStatus::Connecting => manager::Status::Connecting,
        DownloadStatus::Downloading => manager::Status::Downloading,
        DownloadStatus::Paused => manager::Status::Paused,
        DownloadStatus::Finished => manager::Status::Finished,
        DownloadStatus::Failed { reason } => manager::Status::Failed(reason),
    };
    CoreFilter::from(filter).matches(&s)
}

#[uniffi::export]
pub fn filter_title(filter: Filter) -> String {
    CoreFilter::from(filter).title().to_string()
}

#[uniffi::export]
pub fn file_kind_title(kind: FileKind) -> String {
    CoreKind::from(kind).title().to_string()
}

/// "2.4 MB/s" for the footer's total speed.
#[uniffi::export]
pub fn speed_text(bytes_per_second: u64) -> String {
    format::speed_text(bytes_per_second)
}

/// Every link in pasted or dropped text or HTML (§5.2–§5.4).
#[uniffi::export]
pub fn find_links(text: String) -> Vec<String> {
    hd_core::links::find(&text)
}

/// The links a batch pattern like `https://e.com/img^(###1-100).jpg` makes (§5.5).
#[uniffi::export]
pub fn expand_batch(pattern: String) -> Result<Vec<String>, CoreError> {
    hd_core::links::expand_batch(&pattern).map_err(|message| CoreError::Message { message })
}

/// The name a new file gets in a folder: the name itself, or "name (2).ext"… when taken.
#[uniffi::export]
pub fn free_name(folder: String, name: String) -> String {
    manager::free_name(std::path::Path::new(&folder), &name)
}

/// "4.0 GB".
#[uniffi::export]
pub fn bytes_text(bytes: u64) -> String {
    format::bytes(bytes)
}

fn reqwest_host(url: &str) -> String {
    url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or(url).to_string()
}

/// One range of the file for the segments bar: `start..end`, bytes up to `pos` in; `active` while a connection
/// works on it.
#[derive(uniffi::Record)]
pub struct SegmentInfo {
    pub start: u64,
    pub end: u64,
    pub pos: u64,
    pub active: bool,
}

/// One line of the Connections tab.
#[derive(uniffi::Record)]
pub struct ConnectionRow {
    pub id: u32,
    pub host: String,
    pub range_text: String,
    pub downloaded_text: String,
    pub speed_text: String,
    pub state: String,
}

/// One line of the Mirrors tab (§7.3).
#[derive(uniffi::Record)]
pub struct SourceRow {
    pub url: String,
    pub is_original: bool,
    pub enabled: bool,
    pub status_text: String,
    pub active: u32,
    pub speed_text: String,
    pub bytes_text: String,
}

/// What the Download window shows.
#[derive(uniffi::Record)]
pub struct DownloadDetail {
    pub row: DownloadRow,
    pub segments: Vec<SegmentInfo>,
    pub connections: Vec<ConnectionRow>,
    pub sources: Vec<SourceRow>,
}
