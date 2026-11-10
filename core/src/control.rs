//! The command line's and the MCP server's way into the running app (SPECIFICATION.md §12). The app listens on
//! 127.0.0.1 at a port of its choosing and writes the port and a secret token to `control.json` in its data folder,
//! which only the user can read; `hdm` reads them to connect. One JSON request per line, one JSON answer per line.
//! Nothing listens beyond this computer, and a request without the token is refused.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::addflow::{AddSession, Choice, PendingLink, Step};
use crate::format::Filter;
use crate::manager::{Manager, Row, Status};

/// The file the app writes and `hdm` reads.
pub const FILE: &str = "control.json";
/// What `hdm` says when the app isn't open.
pub const NOT_RUNNING: &str = "Herb's Download Manager isn't running. Open it, then try again.";
/// The longest request line read (a batch of links is far shorter).
const MAX_LINE: u64 = 4 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Door {
    port: u16,
    token: String,
}

/// A request: the command and what it needs.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Request {
    pub token: String,
    /// list, add, pause, resume, remove or note.
    pub command: String,
    pub ids: Vec<u64>,
    pub urls: Vec<String>,
    /// add: the folder to save to (S1 when absent).
    pub folder: Option<String>,
    /// add: wait in the queue instead of starting now.
    pub queue: bool,
    /// note: the text ("" removes it).
    pub text: String,
    /// list: all, downloading, paused, finished or unfinished.
    pub filter: Option<String>,
    /// list: only downloads whose name, link or note contains this.
    pub search: String,
    /// remove: move the files to the Trash too.
    pub delete_files: bool,
    /// add, from the browser extension (§13): the page the link was on, sent with each request to its site.
    pub referer: Option<String>,
    /// add, from the browser extension: the browser's cookies for the link (a Cookie header). Kept for this session
    /// only, as an unremembered sign-in for the link's site (§9.1); never written to disk.
    pub cookies: Option<String>,
    /// add: check each link first, and add nothing if one doesn't work, so the browser can keep the download.
    pub check: bool,
    /// add, from the browser extension: the link is a streamed video's playlist (§15), whatever its address says.
    pub stream: bool,
    /// add, one link: the file's name (the extension names a stream after its page).
    pub name: Option<String>,
    /// add, from the browser extension: a DASH manifest a page carries in its data (Facebook's reels) instead of a
    /// link. Saved in the app's data folder and downloaded from there; `urls` may then be empty.
    pub manifest: Option<String>,
}

/// One download as `list` reports it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Download {
    pub id: u64,
    pub name: String,
    /// waiting, connecting, downloading, finishing, paused, finished or failed.
    pub state: String,
    /// As the list's Status column shows it ("Downloading 47%", "Failed (…)").
    pub status: String,
    pub downloaded: u64,
    pub size: Option<u64>,
    /// Bytes per second.
    pub speed: u64,
    pub url: String,
    pub folder: String,
    pub path: Option<String>,
    pub note: String,
    /// Seconds since 1970.
    pub added: u64,
    pub finished: Option<u64>,
}

impl From<&Row> for Download {
    fn from(r: &Row) -> Self {
        let state = match &r.status {
            Status::Waiting => "waiting",
            Status::Connecting => "connecting",
            Status::Downloading => "downloading",
            Status::Finishing => "finishing",
            Status::Paused => "paused",
            Status::Finished => "finished",
            Status::Failed(_) => "failed",
        };
        Download {
            id: r.id,
            name: r.name.clone(),
            state: state.into(),
            status: r.status_text.clone(),
            downloaded: r.downloaded,
            size: r.size,
            speed: r.speed,
            url: r.url.clone(),
            folder: r.folder.display().to_string(),
            path: r.path.as_ref().map(|p| p.display().to_string()),
            note: r.note.clone(),
            added: r.added,
            finished: r.finished,
        }
    }
}

/// The answer: `ok` with a message or downloads, or the error.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Answer {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloads: Option<Vec<Download>>,
    /// scan: the page's files and media.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found: Option<Vec<FoundFile>>,
    /// The app isn't running (the browser extension then offers to open it).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub not_running: bool,
}

/// One file `scan` found on a page (§15).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FoundFile {
    pub url: String,
    pub name: String,
    pub size: Option<u64>,
    /// "Video stream", "Video", "Sound", "Picture", "Archive", "Document", "Program" or "File".
    pub kind: String,
    /// A stream: add it with `stream` and `name`, so it's joined into one MP4 under this name.
    pub stream: bool,
}

impl Answer {
    /// What `send` failing becomes: the app isn't running, or another problem.
    pub fn failed(e: String) -> Answer {
        Answer { not_running: e == NOT_RUNNING, error: Some(e), ..Default::default() }
    }

    fn error(e: impl Into<String>) -> Answer {
        Answer { error: Some(e.into()), ..Default::default() }
    }
    fn message(m: impl Into<String>) -> Answer {
        Answer { ok: true, message: Some(m.into()), ..Default::default() }
    }
}

/// The data folder the app uses: `$HD_DATA_DIR` (tests), or the platform's own. On Linux the Flatpak keeps it under
/// `~/.var/app`, so a command run outside the sandbox finds it there.
pub fn default_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("HD_DATA_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Herb's Download Manager")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("Herb's Download Manager")
    } else {
        let mut candidates = Vec::new();
        if let Some(xdg) = std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()) {
            candidates.push(PathBuf::from(xdg).join("herbs-download-manager"));
        }
        candidates.push(home.join(".var/app/org.herbsdownloadmanager.app/data/herbs-download-manager"));
        candidates.push(home.join(".local/share/herbs-download-manager"));
        candidates.iter().find(|d| d.join(FILE).exists()).or_else(|| candidates.iter().find(|d| d.exists()))
            .unwrap_or(&candidates[0]).clone()
    }
}

/// Starts listening for `hdm` and writes `control.json` to the list's data folder. `downloads` is the Downloads
/// folder, used when Settings ▸ Save to (S1) is unset or gone.
pub fn serve(manager: &Manager, downloads: PathBuf) -> std::io::Result<()> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    let mut secret = [0u8; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut secret)
        .map_err(|_| std::io::Error::other("no random numbers"))?;
    let token: String = secret.iter().map(|b| format!("{b:02x}")).collect();
    write_private(&manager.data_dir().join(FILE), &serde_json::to_vec(&Door { port, token: token.clone() })?)?;
    let manager = manager.detached();
    std::thread::Builder::new().name("hdm control".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            let (manager, token, downloads) = (manager.clone(), token.clone(), downloads.clone());
            let _ = std::thread::Builder::new().name("hdm request".into()).spawn(move || {
                let _ = answer_requests(stream, &manager, &token, &downloads);
            });
        }
    })?;
    Ok(())
}

/// Written so only the user can read it (the token lets anyone who has it control the app).
/// Keeps a DASH manifest from a page as `manifests/<sha256>.mpd` in the data folder; returns its `file:` link.
fn save_manifest(manager: &Manager, text: &str) -> Result<String, String> {
    if !text.contains("<MPD") || text.len() > 4 * 1024 * 1024 {
        return Err("That isn't a DASH manifest".into());
    }
    let digest = ring::digest::digest(&ring::digest::SHA256, text.as_bytes());
    let name: String = digest.as_ref()[..12].iter().map(|b| format!("{b:02x}")).collect();
    let dir = manager.data_dir().join("manifests");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{name}.mpd"));
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    reqwest::Url::from_file_path(&path).map(|u| u.to_string()).map_err(|_| "The manifest couldn't be kept".into())
}

/// A name from a page's title made fit for a file: no folder separators or characters Windows refuses, at most 120
/// characters.
fn safe_name(name: &str) -> String {
    let clean: String = name.chars().map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { ' ' } else { c }).collect();
    let words: Vec<&str> = clean.split_whitespace().collect();
    let mut out = words.join(" ");
    if out.chars().count() > 120 {
        let (stem, ext) = out.rsplit_once('.').map(|(a, b)| (a.to_string(), format!(".{b}"))).unwrap_or((out.clone(), String::new()));
        out = stem.chars().take(120 - ext.chars().count()).collect::<String>().trim_end().to_string() + &ext;
    }
    out.trim_matches('.').trim().to_string()
}

/// The browser's cookies and page for the links' sites, as sign-ins for this session (they win over saved ones, whose
/// user name and password they keep).
fn browser_sign_ins(manager: &Manager, r: &Request) {
    let (cookies, referer) = (r.cookies.clone().unwrap_or_default(), r.referer.clone().unwrap_or_default());
    if cookies.is_empty() && referer.is_empty() {
        return;
    }
    let vault = manager.vault();
    let mut hosts: Vec<String> = r.urls.iter().filter_map(|u| reqwest::Url::parse(u).ok()?.host_str().map(str::to_string)).collect();
    hosts.dedup();
    for host in hosts {
        let mut sign_in = match vault.for_host(&host) {
            crate::vault::Lookup::Found(s) => s,
            _ => crate::vault::SignIn::default(),
        };
        sign_in.site = host;
        sign_in.cookies = cookies.clone();
        sign_in.referer = referer.clone();
        vault.remember_for_session(sign_in);
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(&tmp)?.write_all(bytes)?;
    std::fs::rename(&tmp, path)
}

fn answer_requests(stream: TcpStream, manager: &Manager, token: &str, downloads: &Path) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(60)))?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        if (&mut reader).take(MAX_LINE).read_line(&mut line)? == 0 {
            return Ok(());
        }
        let answer = match serde_json::from_str::<Request>(&line) {
            Ok(r) if same_token(&r.token, token) => {
                handle(manager, downloads, &r)
            }
            Ok(_) => Answer::error("The request's token is wrong."),
            Err(e) => Answer::error(format!("Not a request: {e}")),
        };
        let wrong_token = answer.error.as_deref() == Some("The request's token is wrong.");
        let mut out = serde_json::to_vec(&answer)?;
        out.push(b'\n');
        writer.write_all(&out)?;
        if wrong_token {
            return Ok(());
        }
    }
}

/// Compares in constant time, so the time an answer takes says nothing about how much of a guess was right.
fn same_token(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Carries out one request on the list, as the app's own commands do.
pub fn handle(manager: &Manager, downloads: &Path, r: &Request) -> Answer {
    let rows = manager.rows();
    let known = |ids: &[u64]| -> Result<(), String> {
        if ids.is_empty() {
            return Err("Give the id of at least one download (hdm list shows them).".into());
        }
        match ids.iter().find(|id| !rows.iter().any(|r| r.id == **id)) {
            Some(id) => Err(format!("There's no download with id {id}.")),
            None => Ok(()),
        }
    };
    let count = |n: usize| if n == 1 { "1 download".to_string() } else { format!("{n} downloads") };
    match r.command.as_str() {
        "scan" => {
            let [url] = r.urls.as_slice() else { return Answer::error("Give the address of one web page to scan.") };
            // From the browser extension: the page is read with the browser's cookies.
            browser_sign_ins(manager, &r);
            match manager.scan_page(url) {
                Err(e) => Answer::error(format!("Can't scan {url}: {e}")),
                Ok(scan) => Answer {
                    ok: true,
                    message: Some(scan.title),
                    found: Some(
                        scan.found
                            .into_iter()
                            .map(|f| FoundFile { stream: f.kind == crate::media::scan::Kind::Stream, kind: f.kind.word().into(), url: f.url, name: f.name, size: f.size })
                            .collect(),
                    ),
                    ..Default::default()
                },
            }
        }
        "list" => {
            let filter = match r.filter.as_deref().unwrap_or("all") {
                "all" => Filter::All,
                "downloading" => Filter::Downloading,
                "paused" => Filter::Paused,
                "finished" => Filter::Finished,
                "unfinished" => Filter::Unfinished,
                other => return Answer::error(format!("No filter “{other}”: use all, downloading, paused, finished or unfinished.")),
            };
            let list = rows.iter().filter(|row| filter.matches(&row.status) && row.matches(&r.search)).map(Download::from).collect();
            Answer { ok: true, downloads: Some(list), ..Default::default() }
        }
        "add" => {
            // A manifest from a page: kept in the data folder, and its file is the download's link.
            let mut r = r.clone();
            if let Some(text) = r.manifest.take() {
                match save_manifest(manager, &text) {
                    Ok(link) => {
                        r.urls = vec![link];
                        r.stream = true;
                        r.check = false;
                    }
                    Err(e) => return Answer::error(e),
                }
            }
            if r.urls.is_empty() {
                return Answer::error("Give at least one link to download.");
            }
            let saved = |u: &reqwest::Url| u.scheme() == "file" && u.path().contains("/manifests/");
            if let Some(bad) = r.urls.iter().find(|u| !reqwest::Url::parse(u).is_ok_and(|u| matches!(u.scheme(), "http" | "https") || saved(&u))) {
                return Answer::error(format!("“{bad}” isn't an http or https link."));
            }
            let folder = match &r.folder {
                Some(f) => {
                    let f = PathBuf::from(f);
                    if !f.is_absolute() {
                        return Answer::error(format!("Give the folder as a full path, not “{}”.", f.display()));
                    }
                    if let Err(e) = std::fs::create_dir_all(&f) {
                        return Answer::error(format!("Can't use the folder {}: {e}", f.display()));
                    }
                    f
                }
                None => {
                    let f = manager.settings().save_to.map(PathBuf::from).filter(|p| p.is_dir()).unwrap_or_else(|| downloads.to_path_buf());
                    if let Err(e) = std::fs::create_dir_all(&f) {
                        return Answer::error(format!("Can't use the folder {}: {e}", f.display()));
                    }
                    f
                }
            };
            browser_sign_ins(manager, &r);
            if r.check {
                for url in &r.urls {
                    if let Err(e) = manager.probe(url) {
                        return Answer::error(format!("Herb's Download Manager can't download “{url}”: {e}"));
                    }
                }
            }
            // A name given for one link, made safe for a file name.
            let name = r.name.as_deref().filter(|_| r.urls.len() == 1).map(safe_name).filter(|n| !n.is_empty());
            let links = r.urls.iter().map(|u| PendingLink { url: u.clone(), name: name.clone(), stream: r.stream, ..Default::default() }).collect();
            let mut session = AddSession::new(manager, links, &folder, manager.settings().connections, !r.queue);
            loop {
                match session.step(manager) {
                    // A name already in use: keep both, never replacing a file without asking.
                    Step::Ask(_) => session.answer(manager, Choice::KeepBoth, false),
                    Step::Done(summary) => {
                        let text = summary.text().unwrap_or_else(|| "1 download added".into());
                        let where_ = if r.queue { "waiting in the queue" } else { "started" };
                        return Answer::message(format!("{text} ({where_}), saving to {}.", folder.display()));
                    }
                }
            }
        }
        "pause" | "resume" => {
            if let Err(e) = known(&r.ids) {
                return Answer::error(e);
            }
            if r.command == "pause" {
                manager.pause(&r.ids);
                Answer::message(format!("Paused {}.", count(r.ids.len())))
            } else {
                manager.resume(&r.ids);
                Answer::message(format!("Resumed {}.", count(r.ids.len())))
            }
        }
        "remove" => {
            if let Err(e) = known(&r.ids) {
                return Answer::error(e);
            }
            // Files that couldn't go to the Trash (a volume without one) are left where they are.
            let left = manager.remove(&r.ids, r.delete_files);
            if r.delete_files && !left.is_empty() {
                let names: Vec<String> = left.iter().map(|p| p.display().to_string()).collect();
                Answer::message(format!("Removed {} from the list. These files couldn't go to the Trash and were left where they are: {}",
                                        count(r.ids.len()), names.join(", ")))
            } else if r.delete_files {
                Answer::message(format!("Removed {} from the list and moved their files to the Trash.", count(r.ids.len())))
            } else {
                Answer::message(format!("Removed {} from the list; the files stay where they are.", count(r.ids.len())))
            }
        }
        "note" => {
            if let Err(e) = known(&r.ids) {
                return Answer::error(e);
            }
            if r.ids.len() != 1 {
                return Answer::error("Give one download's id.");
            }
            manager.set_note(r.ids[0], &r.text);
            Answer::message(if r.text.trim().is_empty() { "Note removed." } else { "Note saved." })
        }
        other => Answer::error(format!("No command “{other}”.")),
    }
}

/// Sends a request to the app whose data folder is `dir` and returns its answer; `Err` when it can't be reached.
pub fn send(dir: &Path, request: &Request) -> Result<Answer, String> {
    let door: Door = std::fs::read(dir.join(FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).ok_or(NOT_RUNNING)?;
    let address = (Ipv4Addr::LOCALHOST, door.port).into();
    let stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).map_err(|_| NOT_RUNNING)?;
    stream.set_read_timeout(Some(Duration::from_secs(120))).map_err(|e| e.to_string())?;
    let mut line = serde_json::to_vec(&Request { token: door.token, ..request.clone() }).map_err(|e| e.to_string())?;
    line.push(b'\n');
    (&stream).write_all(&line).map_err(|_| NOT_RUNNING)?;
    let mut reply = String::new();
    BufReader::new(&stream).take(MAX_LINE * 16).read_line(&mut reply).map_err(|e| format!("No answer from the app: {e}"))?;
    serde_json::from_str(&reply).map_err(|_| NOT_RUNNING.to_string())
}

/// Removes `control.json` (the app is quitting).
pub fn close(manager: &Manager) {
    let _ = std::fs::remove_file(manager.data_dir().join(FILE));
}

/// The app's program, beside `hdm`: in the Mac app bundle, the Flatpak (or a Linux install), or the Windows folder.
fn app_program() -> Result<PathBuf, String> {
    let hdm = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = hdm.parent().ok_or("hdm has no folder")?;
    let names: &[&str] = if cfg!(windows) {
        &["HerbsDownloadManager.exe", "Herbs Download Manager.exe"]
    } else if cfg!(target_os = "macos") {
        &["HerbsDownloadManager"]
    } else {
        &["herbs-download-manager"]
    };
    names.iter().map(|n| dir.join(n)).find(|p| p.is_file()).ok_or_else(|| format!("Herb's Download Manager isn't next to hdm in {}", dir.display()))
}

/// Opens the app (the browser extension's "Open and Download"), apart from `hdm` so it keeps running when the browser
/// closes `hdm`, and waits up to `wait` for it to answer. The app gets hdm's environment (a test copy's data folder).
pub fn open_app(dir: &Path, wait: Duration) -> Result<(), String> {
    if send(dir, &Request { command: "list".into(), filter: Some("downloading".into()), ..Default::default() }).is_ok() {
        return Ok(());
    }
    let program = app_program()?;
    let mut command = std::process::Command::new(&program);
    command.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A session of its own: closing hdm's doesn't close the app.
        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    let started = {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x8;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        // Out of the browser's job too, when it allows that, so closing the browser doesn't close the app.
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
        command.spawn().or_else(|_| {
            command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
            command.spawn()
        })
    };
    #[cfg(not(windows))]
    let started = command.spawn();
    started.map_err(|e| format!("Herb's Download Manager couldn't be opened: {e}"))?;
    let until = std::time::Instant::now() + wait;
    while std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_millis(250));
        if send(dir, &Request { command: "list".into(), filter: Some("downloading".into()), ..Default::default() }).is_ok() {
            return Ok(());
        }
    }
    Err("Herb's Download Manager was opened but didn't answer in time".into())
}
