//! The Swift-facing API of hd-core, generated into Swift by UniFFI for the macOS app.
//! The Windows and Linux apps are Rust and call hd-core directly.

uniffi::setup_scaffolding!();

use hd_core::schedule;
use hd_core::settings;

/// The scheduler's "When the queue is done" (§11.1 Q4).
#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq)]
pub enum WhenDone {
    Nothing,
    Quit,
    Sleep,
    ShutDown,
}

impl From<schedule::WhenDone> for WhenDone {
    fn from(w: schedule::WhenDone) -> Self {
        match w {
            schedule::WhenDone::Nothing => WhenDone::Nothing,
            schedule::WhenDone::Quit => WhenDone::Quit,
            schedule::WhenDone::Sleep => WhenDone::Sleep,
            schedule::WhenDone::ShutDown => WhenDone::ShutDown,
        }
    }
}

impl From<WhenDone> for schedule::WhenDone {
    fn from(w: WhenDone) -> Self {
        match w {
            WhenDone::Nothing => schedule::WhenDone::Nothing,
            WhenDone::Quit => schedule::WhenDone::Quit,
            WhenDone::Sleep => schedule::WhenDone::Sleep,
            WhenDone::ShutDown => schedule::WhenDone::ShutDown,
        }
    }
}

#[derive(uniffi::Enum)]
pub enum WhenFileExists {
    Ask,
    KeepBoth,
    Skip,
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
    /// S16: links copied in other apps open Add Links.
    pub watch_clipboard: bool,
    pub keep_server_date: bool,
    /// The scheduler (§11.1): start and stop times in minutes after midnight, `None` for off.
    pub schedule_start: Option<u32>,
    pub schedule_stop: Option<u32>,
    /// Seven days, Monday first.
    pub schedule_days: Vec<bool>,
    pub when_done: WhenDone,
    /// S13: the most lines of a stream's picture (1080 for 1080p); `None` is Highest.
    pub stream_quality: Option<u32>,
    /// S14: sound languages in order; empty is the stream's own sound.
    pub sound_languages: Vec<String>,
    /// S15: subtitle languages kept in a stream's file; empty for none, `*` for all.
    pub subtitle_languages: Vec<String>,
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
                settings::WhenFileExists::KeepBoth => WhenFileExists::KeepBoth,
                settings::WhenFileExists::Skip => WhenFileExists::Skip,
                settings::WhenFileExists::Replace => WhenFileExists::Replace,
            },
            look_for_checksum_files: s.look_for_checksum_files,
            notify_when_finished: s.notify_when_finished,
            watch_clipboard: s.watch_clipboard,
            keep_server_date: s.keep_server_date,
            schedule_start: s.schedule.start,
            schedule_stop: s.schedule.stop,
            schedule_days: s.schedule.days.to_vec(),
            when_done: s.schedule.when_done.into(),
            stream_quality: s.stream_quality,
            sound_languages: s.sound_languages,
            subtitle_languages: s.subtitle_languages,
        }
    }
}

impl From<Settings> for settings::Settings {
    fn from(s: Settings) -> Self {
        settings::Settings {
            save_to: s.save_to.filter(|p| !p.is_empty()),
            connections: s.connections,
            connections_per_server: s.connections_per_server,
            downloads_at_once: s.downloads_at_once,
            speed_limit: s.speed_limit,
            retries: s.retries,
            when_file_exists: match s.when_file_exists {
                WhenFileExists::Ask => settings::WhenFileExists::Ask,
                WhenFileExists::KeepBoth => settings::WhenFileExists::KeepBoth,
                WhenFileExists::Skip => settings::WhenFileExists::Skip,
                WhenFileExists::Replace => settings::WhenFileExists::Replace,
            },
            look_for_checksum_files: s.look_for_checksum_files,
            notify_when_finished: s.notify_when_finished,
            watch_clipboard: s.watch_clipboard,
            keep_server_date: s.keep_server_date,
            schedule: schedule::Schedule {
                start: s.schedule_start,
                stop: s.schedule_stop,
                days: std::array::from_fn(|i| s.schedule_days.get(i).copied().unwrap_or(true)),
                when_done: s.when_done.into(),
            },
            stream_quality: s.stream_quality,
            sound_languages: s.sound_languages,
            subtitle_languages: s.subtitle_languages,
        }
    }
}

/// What Add Links says of a link that didn't answer as a file: "Needs a sign-in", or "Offline (404 Not Found)".
#[uniffi::export]
pub fn offline_text(error: String) -> String {
    hd_core::format::offline_text(&error)
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

/// A stream's file name when only its sound is saved: the name given (else the stream's own, from its link), with
/// `.mp4` made `.m4a`.
#[uniffi::export]
pub fn sound_only_name(name: Option<String>, url: String) -> String {
    hd_core::media::sound_only_name(&name.unwrap_or_else(|| hd_core::media::download::stream_name(&url)))
}

/// Whether a link is a stream's playlist or manifest by its address (`.m3u8`, `.mpd`).
#[uniffi::export]
pub fn is_stream_link(url: String) -> bool {
    hd_core::media::download::is_stream_link(&url)
}

/// S13's choices besides Highest, largest first (2160 … 360).
#[uniffi::export]
pub fn stream_quality_choices() -> Vec<u32> {
    hd_core::media::QUALITIES.to_vec()
}

/// A sound language S14 offers: its code (`de`) and its name in its own language (`Deutsch`).
#[derive(uniffi::Record)]
pub struct SoundLanguage {
    pub code: String,
    pub name: String,
}

#[uniffi::export]
pub fn sound_language_choices() -> Vec<SoundLanguage> {
    hd_core::media::LANGUAGES.iter().map(|(code, name)| SoundLanguage { code: code.to_string(), name: name.to_string() }).collect()
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

/// What adding a link would do (see `DownloadManager::plan_add`).
#[derive(uniffi::Enum)]
pub enum AddPlan {
    New,
    /// The same file as this unfinished download: the link becomes its mirror.
    Mirror { id: u64 },
    /// A different file has the name: ask Skip, Replace or Keep Both (as `kept_name`).
    Clash { id: Option<u64>, path: Option<String>, size: Option<u64>, finished: bool, kept_name: String },
}

/// What adding a link did: a new download, or a mirror of the download with this id.
#[derive(uniffi::Record)]
pub struct AddResult {
    pub id: u64,
    pub as_mirror: bool,
}

#[derive(uniffi::Enum, Clone, PartialEq, Eq)]
pub enum DownloadStatus {
    Waiting,
    Connecting,
    Downloading,
    /// Every byte is in; the file is being flushed to disk and given its name.
    Finishing,
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
            manager::Status::Finishing => DownloadStatus::Finishing,
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
    /// A failed download's host that asks for a user name and password.
    pub needs_login: Option<String>,
    /// The user's note, "" for none.
    pub note: String,
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
            needs_login: r.needs_login,
            note: r.note,
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
    /// A web page: Add Links scans it for its files (`scan_page`).
    pub page: bool,
}

/// One file a page scan found (§15).
#[derive(uniffi::Record)]
pub struct FoundLink {
    pub url: String,
    pub name: String,
    pub size: Option<u64>,
    /// "Video stream", "Video", "Sound", "Picture"…
    pub kind: String,
    /// Added as a stream (joined into one MP4 under `name`).
    pub stream: bool,
}

#[derive(uniffi::Record)]
pub struct PageScan {
    pub title: String,
    pub found: Vec<FoundLink>,
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

    /// Adds a link; `start` runs it at once, otherwise it waits in the queue. A link to the same file as an
    /// unfinished download (same name, and same size when both are known) becomes that download's mirror.
    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &self,
        url: String,
        mirrors: Vec<String>,
        folder: String,
        name: Option<String>,
        size: Option<u64>,
        connections: u32,
        start: bool,
        early: Option<u64>,
        stream: bool,
        sound_only: bool,
    ) -> AddResult {
        let s = self.inner.settings();
        // Sound only: Settings' quality and sound language, the picture left out.
        let choice = sound_only.then(|| hd_core::media::StreamChoice { sound_only: true, ..s.stream_choice() });
        let added = self.inner.add_or_mirror(manager::NewDownload {
            url,
            mirrors,
            folder: folder.into(),
            name,
            connections,
            connections_per_server: s.connections_per_server,
            retries: s.retries,
            start,
            size,
            early,
            stream,
            choice,
            dress: None,
        });
        AddResult { id: added.id(), as_mirror: matches!(added, manager::Added::MirrorOf(_)) }
    }

    /// What adding a link would do: become a mirror of an unfinished download of the same file, clash with a
    /// different file of that name in the list or the folder, or be a new download.
    pub fn plan_add(&self, url: String, name: Option<String>, folder: String, size: Option<u64>) -> AddPlan {
        match self.inner.plan_add(&url, name.as_deref(), std::path::Path::new(&folder), size) {
            manager::Plan::New => AddPlan::New,
            manager::Plan::Mirror(id) => AddPlan::Mirror { id },
            manager::Plan::Clash(c) => AddPlan::Clash {
                id: c.id,
                path: c.path.map(|p| p.to_string_lossy().into_owned()),
                size: c.size,
                finished: c.finished,
                kept_name: c.kept_name,
            },
        }
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

    /// Export Links (§5.8): these downloads (all with none), each name with its link and mirrors under it, as a tree.
    pub fn links_tree(&self, ids: Vec<u64>) -> String {
        self.inner.links_tree(&ids)
    }

    /// Once, when the running queue has finished everything: the scheduler's "When the queue is done" action.
    pub fn take_queue_done(&self) -> Option<WhenDone> {
        self.inner.take_queue_done().map(Into::into)
    }

    /// Registers `hdm` (at `hdm_path`) as the browser extension's native messaging host with the browsers found
    /// (§13). Returns the problems, if any.
    pub fn register_browser_host(&self, hdm_path: String) -> Vec<String> {
        hd_core::browser::register(std::path::Path::new(&hdm_path), self.inner.data_dir())
    }

    /// The browser extension, unpacked in the data folder, for Load Unpacked.
    pub fn extension_folder(&self) -> Result<String, CoreError> {
        hd_core::browser::extension_folder(self.inner.data_dir()).map(|p| p.display().to_string()).map_err(|e| CoreError::Io { message: e.to_string() })
    }

    /// The footer's words for the queue: "Queue running, 2 waiting, stops at 07:00, then sleeps".
    pub fn queue_text(&self) -> Option<String> {
        self.inner.queue_text()
    }

    /// Downloads at once (1–10).
    pub fn set_downloads_at_once(&self, n: u32) {
        self.inner.set_downloads_at_once(n);
    }

    /// The settings (§9) as saved in the data folder.
    pub fn settings(&self) -> Settings {
        self.inner.settings().into()
    }

    /// Saves and applies the settings; numbers out of range are brought into it.
    pub fn set_settings(&self, settings: Settings) {
        self.inner.set_settings(settings.into());
    }

    /// The checksum to verify (§6), or `None` to clear it. Returns why a value is refused ("A SHA-256 value is 64
    /// hexadecimal digits"), or `None` when it was set.
    pub fn set_checksum(&self, id: u64, algorithm: Option<ChecksumAlgorithm>, value: String) -> Option<String> {
        match algorithm {
            None => {
                self.inner.set_checksum(id, None);
                None
            }
            Some(a) => match hd_core::checksum::Checksum::new(a.into(), &value) {
                Ok(c) => {
                    self.inner.set_checksum(id, Some(c));
                    None
                }
                Err(e) => Some(e),
            },
        }
    }

    /// One download's own speed limit in bytes per second (`None`: no limit). Applies at once.
    pub fn set_speed_limit(&self, id: u64, limit: Option<u64>) {
        self.inner.set_speed_limit(id, limit);
    }

    pub fn any_running(&self) -> bool {
        self.inner.any_running()
    }

    /// Quitting: pauses running downloads, marks them to start again at the next launch, and waits up to
    /// `seconds` for them to save. False if some were still running.
    /// Lets `hdm` (the command line and MCP server) control this list while the app runs (§12). `downloads` is the
    /// Downloads folder, used when Settings ▸ Save to is unset.
    pub fn start_control_server(&self, downloads: String) -> Result<(), CoreError> {
        hd_core::control::serve(&self.inner, downloads.into()).map_err(|e| CoreError::Io { message: e.to_string() })
    }

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
            .map(|p| LinkInfo { name: p.name, size: p.size, resumable: p.resumable, page: p.page })
            .map_err(|message| CoreError::Message { message })
    }

    /// Reads a web page and lists its files and media, each checked. Blocks: call it off the main thread.
    pub fn scan_page(&self, url: String) -> Result<PageScan, CoreError> {
        let scan = self.inner.scan_page(&url).map_err(|message| CoreError::Message { message })?;
        Ok(PageScan {
            title: scan.title,
            found: scan
                .found
                .into_iter()
                .map(|f| FoundLink {
                    stream: f.kind == hd_core::media::scan::Kind::Stream,
                    kind: f.kind.word().into(),
                    url: f.url,
                    name: f.name,
                    size: f.size,
                })
                .collect(),
        })
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
            speed_limit: d.speed_limit,
            checksum_algorithm: d.checksum.as_ref().map(|c| c.algorithm.into()),
            checksum_value: d.checksum.as_ref().map(|c| c.value.clone()),
            verified: d.verified,
            row: d.row.into(),
        })
    }

    /// Add Mirror… (§7.1): used at once when running, saved for later otherwise. Returns how many were new.
    pub fn add_mirrors(&self, id: u64, urls: Vec<String>) -> u32 {
        self.inner.add_mirrors(id, &urls) as u32
    }

    /// Make Main Link: a mirror becomes the download's link, the old link a mirror; false if `url` isn't a mirror.
    pub fn make_main(&self, id: u64, url: String) -> bool {
        self.inner.make_main(id, &url)
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

    /// Delete: with `delete_files`, the files go to the Trash. Returns the paths that couldn't (left in place).
    pub fn remove(&self, ids: Vec<u64>, delete_files: bool) -> Vec<String> {
        self.inner.remove(&ids, delete_files).into_iter().map(|p| p.to_string_lossy().into_owned()).collect()
    }

    /// Tests and test copies: Delete moves files into this folder instead of the system's Trash.
    pub fn use_trash_folder(&self, dir: String) {
        self.inner.use_trash_folder(std::path::Path::new(&dir));
    }

    /// The whole list, newest first.
    pub fn rows(&self) -> Vec<DownloadRow> {
        self.inner.rows().into_iter().map(Into::into).collect()
    }
}

/// Whether a row shows under a sidebar filter.
/// The search (T10): whether the row's name, link or note contains `text`, ignoring case.
#[uniffi::export]
pub fn search_matches(row: &DownloadRow, text: String) -> bool {
    manager::search_matches(&text, &[&row.name, &row.url, &row.note])
}

#[uniffi::export]
pub fn filter_matches(filter: Filter, status: DownloadStatus) -> bool {
    let s = match status {
        DownloadStatus::Waiting => manager::Status::Waiting,
        DownloadStatus::Connecting => manager::Status::Connecting,
        DownloadStatus::Downloading => manager::Status::Downloading,
        DownloadStatus::Finishing => manager::Status::Finishing,
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

/// A column of Add Links' list that its rows can be sorted by.
#[derive(uniffi::Enum, Clone, Copy)]
pub enum LinkColumn {
    Name,
    Size,
    Folder,
    Link,
}

/// What Add Links sorts one row by.
#[derive(uniffi::Record)]
pub struct LinkSortRow {
    pub name: String,
    pub size: Option<u64>,
    pub folder: String,
    pub url: String,
}

/// Add Links' rows sorted by a column, as indexes into `rows` (`hd_core::links::sort_order`).
#[uniffi::export]
pub fn link_sort_order(rows: Vec<LinkSortRow>, column: LinkColumn, ascending: bool) -> Vec<u32> {
    use hd_core::links::{LinkColumn as C, LinkRow};
    let rows: Vec<LinkRow> =
        rows.iter().map(|r| LinkRow { name: &r.name, size: r.size, folder: &r.folder, url: &r.url }).collect();
    let column = match column {
        LinkColumn::Name => C::Name,
        LinkColumn::Size => C::Size,
        LinkColumn::Folder => C::Folder,
        LinkColumn::Link => C::Link,
    };
    hd_core::links::sort_order(&rows, column, ascending).into_iter().map(|i| i as u32).collect()
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
    /// The download's own speed limit, bytes per second.
    pub speed_limit: Option<u64>,
    pub checksum_algorithm: Option<ChecksumAlgorithm>,
    pub checksum_value: Option<String>,
    /// "SHA-256 matches (f.bin.sha256)", or a mismatch found on a finished file.
    pub verified: Option<String>,
}

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq)]
pub enum ChecksumAlgorithm {
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

impl From<ChecksumAlgorithm> for hd_core::checksum::Algorithm {
    fn from(a: ChecksumAlgorithm) -> Self {
        match a {
            ChecksumAlgorithm::Md5 => Self::Md5,
            ChecksumAlgorithm::Sha1 => Self::Sha1,
            ChecksumAlgorithm::Sha256 => Self::Sha256,
            ChecksumAlgorithm::Sha512 => Self::Sha512,
        }
    }
}

impl From<hd_core::checksum::Algorithm> for ChecksumAlgorithm {
    fn from(a: hd_core::checksum::Algorithm) -> Self {
        match a {
            hd_core::checksum::Algorithm::Md5 => Self::Md5,
            hd_core::checksum::Algorithm::Sha1 => Self::Sha1,
            hd_core::checksum::Algorithm::Sha256 => Self::Sha256,
            hd_core::checksum::Algorithm::Sha512 => Self::Sha512,
        }
    }
}

/// The algorithm a pasted checksum's length implies (32 hex digits: MD5, 40: SHA-1, 64: SHA-256, 128: SHA-512).
#[uniffi::export]
pub fn checksum_algorithm_of(value: String) -> Option<ChecksumAlgorithm> {
    hd_core::checksum::Algorithm::of_value(&value).map(Into::into)
}

// --- The system's proxy (§9.2) ----------------------------------------------------------------------------------

/// The app's way to find the system's proxy for an address (the Mac asks CFNetwork, which runs automatic setup
/// scripts): its URL ("http://proxy:8080", "socks5://…"), or `None` / "direct://" to connect directly.
#[uniffi::export(with_foreign)]
pub trait ProxyFinder: Send + Sync {
    fn proxy_for(&self, url: String) -> Option<String>;
}

/// Sets how each address's proxy is found. Call before `DownloadManager.open`, whose client is made once.
#[uniffi::export]
pub fn set_proxy_finder(finder: std::sync::Arc<dyn ProxyFinder>) {
    hd_core::http::set_proxy_resolver(std::sync::Arc::new(move |url| finder.proxy_for(url.to_string())));
}

// --- Feature tests -----------------------------------------------------------------------------------------------

/// The app side of a feature test (core/src/scenario.rs): runs a command through the app's own code on its main thread,
/// with these downloads selected. Returns why it couldn't, or `None`.
#[uniffi::export(with_foreign)]
pub trait ScenarioHost: Send + Sync {
    fn command(&self, name: String, ids: Vec<u64>, args: Vec<String>) -> Option<String>;
}

struct HostAdapter(std::sync::Arc<dyn ScenarioHost>);

impl hd_core::scenario::Host for HostAdapter {
    fn command(&self, name: &str, ids: &[u64], args: &[String]) -> Result<(), String> {
        match self.0.command(name.to_string(), ids.to_vec(), args.to_vec()) {
            None => Ok(()),
            Some(e) => Err(e),
        }
    }
}

#[derive(uniffi::Record)]
pub struct ScenarioVar {
    pub name: String,
    pub value: String,
}

#[uniffi::export]
impl DownloadManager {
    /// Runs a feature-test scenario and returns its report ("PASS  Download a file (0.1 s)" lines and a summary).
    /// Blocks: call it off the main thread, which the host's commands need.
    pub fn run_scenario(&self, host: std::sync::Arc<dyn ScenarioHost>, script: String, vars: Vec<ScenarioVar>) -> String {
        let vars: Vec<(String, String)> = vars.into_iter().map(|v| (v.name, v.value)).collect();
        let results = hd_core::scenario::run(&self.inner, &HostAdapter(host), &script, &vars);
        hd_core::scenario::report(&results)
    }
}

/// Why a file of `size` bytes won't fit in `folder` (not enough space, or a FAT32 drive and 4 GB or more), or `None`.
#[uniffi::export]
pub fn space_problem(folder: String, size: u64) -> Option<String> {
    hd_core::disk::space_problem(std::path::Path::new(&folder), size)
}

#[derive(uniffi::Record)]
pub struct HostLimitRow {
    pub host: String,
    pub connections: u32,
}

#[uniffi::export]
impl DownloadManager {
    /// Servers' remembered connection limits (Settings).
    pub fn host_limits(&self) -> Vec<HostLimitRow> {
        self.inner.host_limits().into_iter().map(|(host, connections)| HostLimitRow { host, connections }).collect()
    }

    /// Settings ▸ Forget.
    pub fn forget_host_limits(&self) {
        self.inner.forget_host_limits();
    }
}

#[derive(uniffi::Record)]
pub struct HostLimitsText {
    pub summary: String,
    pub detail: String,
}

/// Settings' line for the remembered limits and its hover text.
#[uniffi::export]
pub fn host_limits_text(limits: Vec<HostLimitRow>) -> HostLimitsText {
    let pairs: Vec<(String, u32)> = limits.into_iter().map(|r| (r.host, r.connections)).collect();
    let (summary, detail) = hd_core::hosts::describe(&pairs);
    HostLimitsText { summary, detail }
}

// --- Sign-ins (links that need a login) ------------------------------------------------------------------------------

#[derive(uniffi::Enum, Clone, Copy, PartialEq, Eq)]
pub enum SignInStore {
    /// No choice made yet: the first save asks how to protect sign-ins.
    NotChosen,
    Plain,
    Locked,
    Unlocked,
}

#[derive(uniffi::Record)]
pub struct SignInRow {
    pub site: String,
    pub user: String,
    pub referer: String,
    /// `None` while locked.
    pub password: Option<String>,
    pub cookies: Option<String>,
    pub encrypted: bool,
}

#[derive(uniffi::Record)]
pub struct NewSignIn {
    pub site: String,
    pub user: String,
    pub password: String,
    pub cookies: String,
    pub referer: String,
}

impl From<NewSignIn> for hd_core::vault::SignIn {
    fn from(s: NewSignIn) -> Self {
        hd_core::vault::SignIn { site: s.site, user: s.user, password: s.password, cookies: s.cookies, referer: s.referer }
    }
}

fn message(r: Result<(), String>) -> Result<(), CoreError> {
    r.map_err(|message| CoreError::Message { message })
}

#[uniffi::export]
impl DownloadManager {
    pub fn sign_in_store(&self) -> SignInStore {
        match self.inner.vault().state() {
            hd_core::vault::State::NotChosen => SignInStore::NotChosen,
            hd_core::vault::State::Plain => SignInStore::Plain,
            hd_core::vault::State::Locked => SignInStore::Locked,
            hd_core::vault::State::Unlocked => SignInStore::Unlocked,
        }
    }

    /// Creates the Administrator Password; every sign-in is encrypted with it. Takes about a second: call it off the
    /// main thread.
    pub fn create_admin_password(&self, password: String) -> Result<(), CoreError> {
        message(self.inner.vault().create_password(&password))
    }

    /// Changes it (unlocked only); an empty one removes it and stores sign-ins as plain text.
    pub fn change_admin_password(&self, password: String) -> Result<(), CoreError> {
        message(self.inner.vault().change_password(&password))
    }

    /// The explicit, confirmed choice to store sign-ins as plain text.
    pub fn store_sign_ins_as_plain_text(&self) -> Result<(), CoreError> {
        message(self.inner.vault().use_plain_text())
    }

    /// False when the password is wrong. Takes about a second.
    pub fn unlock_sign_ins(&self, password: String) -> bool {
        self.inner.vault().unlock(&password)
    }

    pub fn lock_sign_ins(&self) {
        self.inner.vault().lock();
    }

    pub fn sign_ins(&self) -> Vec<SignInRow> {
        self.inner
            .vault()
            .list()
            .into_iter()
            .map(|l| SignInRow { site: l.site, user: l.user, referer: l.referer, password: l.password, cookies: l.cookies, encrypted: l.encrypted })
            .collect()
    }

    pub fn save_sign_in(&self, sign_in: NewSignIn) -> Result<(), CoreError> {
        message(self.inner.vault().save_sign_in(sign_in.into()))
    }

    /// Uses a sign-in until the app quits, without saving it.
    pub fn use_sign_in_for_session(&self, sign_in: NewSignIn) {
        self.inner.vault().remember_for_session(sign_in.into());
    }

    pub fn delete_sign_in(&self, site: String) -> Result<(), CoreError> {
        message(self.inner.vault().delete(&site))
    }

    /// Starts again the downloads that failed for want of a sign-in to `site` ("" for every site).
    pub fn resume_waiting_for_login(&self, site: String) {
        let ids = self.inner.waiting_for_login(&site);
        self.inner.resume(&ids);
    }
}

/// The site a link or typed text belongs to ("https://www.example.com/a" → "example.com").
#[uniffi::export]
pub fn site_of(text: String) -> String {
    hd_core::vault::site_of(&text)
}

// --- Finished downloads ---------------------------------------------------------------------------------------------

#[uniffi::export]
impl DownloadManager {
    /// Rename…: a finished download's file gets a new name in its folder; returns the new path.
    pub fn rename_file(&self, id: u64, name: String) -> Result<String, CoreError> {
        self.inner.rename_file(id, &name).map(|p| p.display().to_string()).map_err(|message| CoreError::Message { message })
    }

    /// Note…: the download's own note; blank removes it.
    pub fn set_note(&self, id: u64, note: String) {
        self.inner.set_note(id, &note);
    }

    /// Move To…: a finished download's file goes to `target` (a full path); returns it.
    pub fn move_file(&self, id: u64, target: String) -> Result<String, CoreError> {
        self.inner
            .move_file(id, std::path::Path::new(&target))
            .map(|p| p.display().to_string())
            .map_err(|message| CoreError::Message { message })
    }
}

// --- Starting while New Download is open ------------------------------------------------------------------------------

#[derive(uniffi::Record)]
pub struct EarlyProgress {
    pub downloaded: u64,
    pub size: Option<u64>,
}

#[uniffi::export]
impl DownloadManager {
    /// Starts fetching the link while New Download is open, into `folder`; returns a token for `add` or `cancel_early`.
    pub fn start_early(&self, url: String, mirrors: Vec<String>, folder: String, name: Option<String>) -> u64 {
        self.inner.start_early(&url, &mirrors, std::path::Path::new(&folder), name.as_deref())
    }

    pub fn early_progress(&self, token: u64) -> Option<EarlyProgress> {
        self.inner.early_progress(token).map(|(downloaded, size)| EarlyProgress { downloaded, size })
    }

    /// Cancel in New Download: what was fetched is discarded.
    pub fn cancel_early(&self, token: u64) {
        self.inner.cancel_early(token);
    }
}
