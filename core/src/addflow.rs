//! Adding links (SPECIFICATION.md §5, §5.6, §7.1 M5) as one batch, for hosts written in Rust (Linux, Windows; the Mac
//! has the same flow in Swift). Links are added one at a time, so links to the same file gather into one download and
//! its mirrors. When a different file already has a link's name, the session stops with a question (Skip, Replace or
//! Keep Both, "Do the same for the rest" when more follow) unless Settings ▸ When a file exists answers it; the host
//! asks, calls `answer`, and steps again. A summary for the footer comes at the end.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::manager::{Added, Clash, Manager, NewDownload, Plan};
use crate::settings::WhenFileExists;

/// A link about to be added, with what its server said when it was checked.
#[derive(Clone, Debug, Default)]
pub struct PendingLink {
    pub url: String,
    pub mirrors: Vec<String>,
    pub name: Option<String>,
    pub size: Option<u64>,
    /// The early download New Download started for this link (`Manager::start_early`).
    pub early: Option<u64>,
    /// A streamed video's playlist (§15), when the address doesn't say so (the browser extension saw its type).
    pub stream: bool,
    /// A stream's quality and sound track, chosen in the extension; `None` takes Settings'.
    pub choice: Option<crate::media::StreamChoice>,
    /// How the browser asked for it (the extension): its User-Agent and page.
    pub dress: Option<crate::transfer::Dress>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Skip,
    Replace,
    KeepBoth,
}

/// What to ask (§5.6): "A different “report.pdf” is already in Downloads…".
#[derive(Clone, Debug)]
pub struct Question {
    pub name: String,
    pub clash: Clash,
    pub new_size: Option<u64>,
    pub folder: PathBuf,
    /// More links follow, so "Do the same for the rest" is offered.
    pub more: bool,
}

impl Question {
    pub fn title(&self) -> String {
        let place = match (self.clash.id, self.clash.finished) {
            (None, _) => format!("in {}", self.folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            (Some(_), true) => "in the list".into(),
            (Some(_), false) => "in the list, not yet finished".into(),
        };
        format!("A different “{}” is already {place}.", self.name)
    }

    pub fn body(&self) -> String {
        let sizes = match (self.clash.size, self.new_size) {
            (Some(a), Some(b)) if a != b => {
                format!(" The one there is {}; the new one is {}.", crate::format::bytes(a), crate::format::bytes(b))
            }
            (Some(a), Some(_)) => format!(" Both are {}, but they may differ.", crate::format::bytes(a)),
            _ => String::new(),
        };
        format!(
            "Skip the new link, replace the existing {} (it goes to the Trash), or keep both, the new one as “{}”.{sizes}",
            if self.clash.finished || self.clash.id.is_none() { "file" } else { "download" },
            self.clash.kept_name
        )
    }
}

pub enum Step {
    Ask(Question),
    Done(Summary),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub new: usize,
    pub mirrored: usize,
    pub skipped: usize,
    pub kept: usize,
    pub replaced: usize,
    pub links: usize,
}

impl Summary {
    /// The footer's line ("3 downloads added, 2 links added as mirrors"), or `None` for a single plain add.
    pub fn text(&self) -> Option<String> {
        let mut parts = Vec::new();
        let plural = |n: usize, one: &str, many: String| if n == 1 { one.to_string() } else { many };
        if self.new > 0 {
            parts.push(plural(self.new, "1 download added", format!("{} downloads added", self.new)));
        }
        if self.mirrored > 0 {
            parts.push(plural(self.mirrored, "1 link added as a mirror", format!("{} links added as mirrors", self.mirrored)));
        }
        if self.kept > 0 {
            parts.push(format!("{} kept under a new name", self.kept));
        }
        if self.replaced > 0 {
            parts.push(format!("{} replaced", self.replaced));
        }
        if self.skipped > 0 {
            parts.push(format!("{} skipped", self.skipped));
        }
        (!parts.is_empty() && (self.links > 1 || self.mirrored > 0 || self.skipped > 0)).then(|| parts.join(", "))
    }
}

pub struct AddSession {
    links: Vec<PendingLink>,
    next: usize,
    folder: PathBuf,
    connections: u32,
    start: bool,
    remembered: Option<Choice>,
    /// What this batch did with each file (name and size): its other links follow, as mirrors of the download just
    /// added (even under a kept name), or skipped with it.
    decided: HashMap<String, Option<u64>>,
    waiting: Option<(PendingLink, Clash, String)>,
    summary: Summary,
}

/// Sound only, with Settings' quality and sound language (S13, S14): what the add windows' "Only the sound" gives.
pub fn sound_only_choice(manager: &Manager) -> crate::media::StreamChoice {
    crate::media::StreamChoice { sound_only: true, ..manager.settings().stream_choice() }
}

fn name_of(link: &PendingLink) -> String {
    let name = plain_name_of(link);
    if link.choice.as_ref().is_some_and(|c| c.sound_only) { crate::media::sound_only_name(&name) } else { name }
}

fn plain_name_of(link: &PendingLink) -> String {
    link.name.clone().unwrap_or_else(|| {
        if link.stream || crate::media::download::is_stream_link(&link.url) {
            return crate::media::download::stream_name(&link.url);
        }
        reqwest::Url::parse(&link.url)
            .ok()
            .and_then(|u| u.path_segments().and_then(|mut s| s.next_back().map(str::to_string)))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| link.url.clone())
    })
}

impl AddSession {
    /// `start`: Download (now) or Add (to the queue). Settings ▸ When a file exists, when not Ask, answers every
    /// question up front.
    pub fn new(manager: &Manager, links: Vec<PendingLink>, folder: &Path, connections: u32, start: bool) -> AddSession {
        let remembered = match manager.settings().when_file_exists {
            WhenFileExists::Ask => None,
            WhenFileExists::KeepBoth => Some(Choice::KeepBoth),
            WhenFileExists::Skip => Some(Choice::Skip),
            WhenFileExists::Replace => Some(Choice::Replace),
        };
        let summary = Summary { links: links.len(), ..Summary::default() };
        AddSession {
            links,
            next: 0,
            folder: folder.to_path_buf(),
            connections,
            start,
            remembered,
            decided: HashMap::new(),
            waiting: None,
            summary,
        }
    }

    /// Adds links until one needs a question, or all are done.
    pub fn step(&mut self, manager: &Manager) -> Step {
        while self.waiting.is_none() && self.next < self.links.len() {
            let link = self.links[self.next].clone();
            self.next += 1;
            let name = name_of(&link);
            let key = format!("{name}\u{0}{}", link.size.map(|s| s.to_string()).unwrap_or_default());
            if let Some(earlier) = self.decided.get(&key) {
                if let Some(token) = link.early {
                    manager.cancel_early(token);
                }
                match earlier {
                    Some(id) => {
                        let mut urls = vec![link.url.clone()];
                        urls.extend(link.mirrors.iter().cloned());
                        manager.add_mirrors(*id, &urls);
                        self.summary.mirrored += 1;
                    }
                    None => self.summary.skipped += 1,
                }
                continue;
            }
            match manager.plan_add(&link.url, link.name.as_deref(), &self.folder, link.size) {
                Plan::Clash(clash) => match self.remembered {
                    Some(choice) => self.apply(manager, link, clash, key, choice),
                    None => {
                        let question = Question {
                            name: name.clone(),
                            clash: clash.clone(),
                            new_size: link.size,
                            folder: self.folder.clone(),
                            more: self.next < self.links.len(),
                        };
                        self.waiting = Some((link, clash, key));
                        return Step::Ask(question);
                    }
                },
                _ => self.add(manager, link, None, key),
            }
        }
        Step::Done(self.summary)
    }

    /// The answer to the question `step` returned; `for_rest` is "Do the same for the rest". Call `step` again.
    pub fn answer(&mut self, manager: &Manager, choice: Choice, for_rest: bool) {
        if for_rest {
            self.remembered = Some(choice);
        }
        if let Some((link, clash, key)) = self.waiting.take() {
            self.apply(manager, link, clash, key, choice);
        }
    }

    fn apply(&mut self, manager: &Manager, link: PendingLink, clash: Clash, key: String, choice: Choice) {
        match choice {
            Choice::Skip => {
                if let Some(token) = link.early {
                    manager.cancel_early(token);
                }
                self.summary.skipped += 1;
                self.decided.insert(key, None);
            }
            Choice::KeepBoth => {
                self.summary.kept += 1;
                self.add(manager, link, Some(clash.kept_name), key);
            }
            Choice::Replace => {
                // The old file goes to the Trash, so replacing can be undone; an unfinished download's part file goes
                // with its entry.
                if let Some(path) = &clash.path
                    && path.exists()
                {
                    let _ = trash::delete(path);
                }
                if let Some(id) = clash.id {
                    let _ = manager.remove(&[id], !clash.finished);
                }
                self.summary.replaced += 1;
                self.add(manager, link, None, key);
            }
        }
    }

    fn add(&mut self, manager: &Manager, link: PendingLink, name: Option<String>, key: String) {
        let s = manager.settings();
        let added = manager.add_or_mirror(NewDownload {
            url: link.url,
            mirrors: link.mirrors,
            folder: self.folder.clone(),
            name: name.or(link.name),
            connections: self.connections,
            connections_per_server: s.connections_per_server,
            retries: s.retries,
            start: self.start,
            size: link.size,
            early: link.early,
            stream: link.stream,
            choice: link.choice,
            dress: link.dress,
        });
        match added {
            Added::MirrorOf(_) => self.summary.mirrored += 1,
            Added::New(_) => self.summary.new += 1,
        }
        self.decided.insert(key, Some(added.id()));
    }
}
