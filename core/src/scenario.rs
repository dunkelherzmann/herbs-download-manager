//! Feature tests (macos, linux and windows pending.md): one scenario file, `tests/features.scenario`, run by each app.
//! The app carries out every action through its own command code (the code its buttons and menus run) on its UI
//! thread, through `Host`; this runner, on a background thread, waits on the download list and checks the results
//! and the files, and reports pass or fail per feature.
//!
//! The language, one step per line (`#` starts a comment; `$VAR` is replaced from the variables given):
//!
//! ```text
//! feature NAME                     starts a feature; its steps stop at the first failure
//! add FOLDER [start|queue] URL…    the app's add flow, each link its own download (names in use follow S7)
//! add-as-mirrors FOLDER URL…       Add Links' "Treat as mirrors of one file", started
//! add-sound-only FOLDER URL…       Add Links' "Only the sound of streamed videos", started
//! do COMMAND [ROW] [ARG…]          an app command with ROW selected: resume, pause, restart, delete,
//!                                  delete-files, add-mirror (ARG: links), set-checksum (ARG: algorithm, value),
//!                                  rename (ARG: new name), move (ARG: a folder under $DL), note (ARG: text),
//!                                  start-queue, stop-queue, stop-all, cancel-when-done (ARG: the action the app
//!                                  offers to take now the queue is done; retried for 10 s)
//! setting KEY VALUE                downloads-at-once, speed-limit (bytes a second, 0 for none),
//!                                  when-file-exists (ask, keep-both, skip, replace), look-for-checksum-files (on, off),
//!                                  keep-server-date (on, off), schedule-start and schedule-stop (+MINUTES from the
//!                                  scheduler's now, HH:MM, or off), when-done (nothing, quit, sleep, shut-down),
//!                                  stream-quality (highest, or lines such as 1080), sound-language (a code, or own),
//!                                  subtitles (a code, all, or none)
//! clock +SECONDS                   moves the scheduler's clock forward
//! wait ROW STATE SECONDS           finished, paused, failed, running or waiting
//! expect ROW CHECK [VALUE]         file-size N, file-date SECONDS, sources N, status-contains TEXT,
//!                                  verified-contains TEXT, note TEXT, matches TEXT (the search finds it),
//!                                  not-matches TEXT, gone
//! expect-running N                 how many downloads run now
//! sign-in SITE USER PASSWORD       saves a sign-in (as plain text in the test's own data folder)
//! expect-file PATH exists|absent   a path under $DL
//! sleep SECONDS
//! ```
//!
//! A ROW is `FOLDER/NAME`: the download named NAME saved in $DL/FOLDER. "Quotes" keep spaces in one word.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::manager::{Manager, Row, Status};

/// The app side: runs a command on the UI thread, with these downloads selected, through the app's own code.
pub trait Host: Send + Sync {
    /// `Err` with the reason when the app can't do it.
    fn command(&self, name: &str, ids: &[u64], args: &[String]) -> Result<(), String>;
}

#[derive(Clone, Debug)]
pub struct FeatureResult {
    pub name: String,
    pub passed: bool,
    /// Why it failed (the failing step and what was found).
    pub note: String,
    pub seconds: f64,
}

/// "PASS  Mirrors (3.2 s)" lines and a summary, for the scripts.
pub fn report(results: &[FeatureResult]) -> String {
    let mut out = String::new();
    for r in results {
        out.push_str(&format!("{}  {} ({:.1} s){}\n", if r.passed { "PASS" } else { "FAIL" }, r.name, r.seconds,
                              if r.note.is_empty() { String::new() } else { format!(": {}", r.note) }));
    }
    let failed = results.iter().filter(|r| !r.passed).count();
    out.push_str(&format!("{} features, {} failed\n", results.len(), failed));
    out
}

struct Run<'a> {
    manager: &'a Manager,
    host: &'a dyn Host,
    dl: PathBuf,
}

impl Run<'_> {
    fn row(&self, spec: &str) -> Option<Row> {
        let (folder, name) = spec.rsplit_once('/')?;
        let dir = self.dl.join(folder);
        self.manager.rows().into_iter().find(|r| r.name == name && r.folder == dir)
    }

    fn need_row(&self, spec: &str) -> Result<Row, String> {
        self.row(spec).ok_or_else(|| format!("no download {spec}"))
    }

    fn step(&self, words: &[String]) -> Result<(), String> {
        let arg = |i: usize| words.get(i).cloned().ok_or_else(|| format!("“{}” needs more", words.join(" ")));
        match words[0].as_str() {
            "add" | "add-as-mirrors" | "add-sound-only" => {
                let folder = self.dl.join(arg(1)?);
                std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
                let (mode, links) = match words.get(2).map(String::as_str) {
                    Some("start" | "queue") if words[0] == "add" => (words[2].clone(), &words[3..]),
                    _ => ("start".into(), &words[2..]),
                };
                let mut args = vec![folder.display().to_string(), mode];
                args.extend(links.iter().cloned());
                self.host.command(&words[0], &[], &args)
            }
            "do" => {
                let command = arg(1)?;
                let (ids, rest) = match command.as_str() {
                    "start-queue" | "stop-queue" | "stop-all" => (vec![], &words[2..]),
                    // Resume with nothing selected (starts the waiting downloads).
                    "resume" if words.len() == 2 => (vec![], &words[2..]),
                    // The app sees the queue is done within half a second; give it time to ask.
                    "cancel-when-done" => {
                        let until = Instant::now() + Duration::from_secs(10);
                        loop {
                            match self.host.command(&command, &[], &words[2..]) {
                                Err(_) if Instant::now() < until => std::thread::sleep(Duration::from_millis(200)),
                                result => return result,
                            }
                        }
                    }
                    _ => (vec![self.need_row(&arg(2)?)?.id], &words[3..]),
                };
                // Move To…'s folder picker answers with a full path: the folder under $DL and the file's name.
                if command == "move" {
                    let name = self.need_row(&arg(2)?)?.name;
                    let target = self.dl.join(arg(3)?).join(name);
                    std::fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
                    return self.host.command(&command, &ids, &[target.display().to_string()]);
                }
                self.host.command(&command, &ids, rest)
            }
            "sign-in" => {
                let vault = self.manager.vault();
                if vault.state() == crate::vault::State::NotChosen {
                    vault.use_plain_text()?;
                }
                vault.save_sign_in(crate::vault::SignIn { site: arg(1)?, user: arg(2)?, password: arg(3)?, ..Default::default() })
            }
            "setting" => {
                let (key, value) = (arg(1)?, arg(2)?);
                let mut s = self.manager.settings();
                match key.as_str() {
                    "downloads-at-once" => s.downloads_at_once = value.parse().map_err(|_| "a number".to_string())?,
                    "speed-limit" => s.speed_limit = value.parse::<u64>().ok().filter(|&v| v > 0),
                    "when-file-exists" => {
                        use crate::settings::WhenFileExists::*;
                        s.when_file_exists = match value.as_str() {
                            "keep-both" => KeepBoth,
                            "skip" => Skip,
                            "replace" => Replace,
                            _ => Ask,
                        }
                    }
                    "look-for-checksum-files" => s.look_for_checksum_files = value == "on",
                    "keep-server-date" => s.keep_server_date = value == "on",
                    "schedule-start" | "schedule-stop" => {
                        let minutes = match value.strip_prefix('+') {
                            _ if value == "off" => None,
                            Some(ahead) => {
                                let ahead: i64 = ahead.parse().map_err(|_| "+MINUTES".to_string())?;
                                let t = self.manager.scheduler_now() + ahead * 60;
                                let local = chrono::TimeZone::timestamp_opt(&chrono::Local, t, 0).earliest().ok_or("no such time")?;
                                Some(chrono::Timelike::hour(&local) * 60 + chrono::Timelike::minute(&local))
                            }
                            None => Some(crate::schedule::parse_time(&value).ok_or("HH:MM")?),
                        };
                        if key == "schedule-start" { s.schedule.start = minutes } else { s.schedule.stop = minutes }
                    }
                    "when-done" => s.schedule.when_done = crate::schedule::WhenDone::from_word(&value).ok_or("no such action")?,
                    "stream-quality" => {
                        s.stream_quality = if value == "highest" { None } else { Some(value.parse().map_err(|_| "highest or lines".to_string())?) }
                    }
                    "sound-language" => s.sound_languages = if value == "own" { Vec::new() } else { vec![value] },
                    "subtitles" => {
                        s.subtitle_languages = match value.as_str() {
                            "none" => Vec::new(),
                            "all" => vec!["*".into()],
                            _ => vec![value],
                        }
                    }
                    _ => return Err(format!("no setting {key}")),
                }
                self.manager.set_settings(s);
                Ok(())
            }
            "clock" => {
                let seconds: i64 = arg(1)?.trim_start_matches('+').parse().map_err(|_| "clock +SECONDS".to_string())?;
                self.manager.advance_clock(seconds);
                Ok(())
            }
            "wait" => {
                let (spec, state, secs) = (arg(1)?, arg(2)?, arg(3)?.parse::<f64>().unwrap_or(30.0));
                let until = Instant::now() + Duration::from_secs_f64(secs);
                loop {
                    let row = self.row(&spec);
                    if let Some(r) = &row {
                        let ok = match state.as_str() {
                            "finished" => r.status == Status::Finished,
                            "paused" => r.status == Status::Paused,
                            "failed" => matches!(r.status, Status::Failed(_)),
                            "running" => r.status.is_running(),
                            "waiting" => r.status == Status::Waiting,
                            _ => return Err(format!("no state {state}")),
                        };
                        if ok {
                            return Ok(());
                        }
                        if state == "finished" && matches!(r.status, Status::Failed(_)) {
                            return Err(format!("{spec} failed: {}", r.status_text));
                        }
                    }
                    if Instant::now() > until {
                        return Err(format!("{spec} is {} after {secs} s, not {state}",
                                           row.map(|r| r.status_text).unwrap_or_else(|| "missing".into())));
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            "expect" => {
                let spec = arg(1)?;
                let check = arg(2)?;
                if check == "gone" {
                    return match self.row(&spec) {
                        None => Ok(()),
                        Some(_) => Err(format!("{spec} is still in the list")),
                    };
                }
                let r = self.need_row(&spec)?;
                let value = words[3..].join(" ");
                match check.as_str() {
                    "file-size" => {
                        let size = r.path.as_ref().and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len());
                        (size == value.parse().ok()).then_some(()).ok_or(format!("{spec}'s file is {size:?} bytes, not {value}"))
                    }
                    "sources" => (r.sources_known.to_string() == value).then_some(())
                        .ok_or(format!("{spec} has {} sources, not {value}", r.sources_known)),
                    "status-contains" => r.status_text.contains(&value).then_some(())
                        .ok_or(format!("{spec}'s status is “{}”", r.status_text)),
                    "file-date" => {
                        let secs = r.path.as_ref().and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok())
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs());
                        (secs == value.parse().ok()).then_some(()).ok_or(format!("{spec}'s file is dated {secs:?}, not {value}"))
                    }
                    "note" => (r.note == value).then_some(()).ok_or(format!("{spec}'s note is “{}”", r.note)),
                    "matches" => r.matches(&value).then_some(()).ok_or(format!("searching for “{value}” doesn't find {spec}")),
                    "not-matches" => (!r.matches(&value)).then_some(()).ok_or(format!("searching for “{value}” finds {spec}")),
                    "verified-contains" => {
                        let note = self.manager.detail(r.id).and_then(|d| d.verified).unwrap_or_default();
                        note.contains(&value).then_some(()).ok_or(format!("{spec}'s checksum note is “{note}”"))
                    }
                    _ => Err(format!("no check {check}")),
                }
            }
            "expect-running" => {
                let n = self.manager.rows().iter().filter(|r| r.status.is_running()).count();
                (n.to_string() == arg(1)?).then_some(()).ok_or(format!("{n} downloads run, not {}", words[1]))
            }
            "expect-file" => {
                let path = self.dl.join(arg(1)?);
                let want = arg(2)? == "exists";
                (path.exists() == want).then_some(()).ok_or(format!("{} {}", path.display(), if want { "is missing" } else { "is still there" }))
            }
            "sleep" => {
                std::thread::sleep(Duration::from_secs_f64(arg(1)?.parse().unwrap_or(1.0)));
                Ok(())
            }
            other => Err(format!("no step “{other}”")),
        }
    }
}

/// Words separated by spaces; "double quotes" keep spaces in one word ("f9/small - kept.bin").
fn split_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            c => word.push(c),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// Runs the scenario; `vars` replace `$NAME`s (DL is the downloads folder the rows are found in).
pub fn run(manager: &Manager, host: &dyn Host, script: &str, vars: &[(String, String)]) -> Vec<FeatureResult> {
    let dl = vars.iter().find(|(k, _)| k == "DL").map(|(_, v)| PathBuf::from(v)).unwrap_or_default();
    let run = Run { manager, host, dl };
    let mut results: Vec<FeatureResult> = Vec::new();
    let mut started = Instant::now();
    let mut failed = false;
    for line in script.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut text = line.to_string();
        for (k, v) in vars {
            text = text.replace(&format!("${k}"), v);
        }
        let words = split_words(&text);
        if words[0] == "feature" {
            if let Some(last) = results.last_mut() {
                last.seconds = started.elapsed().as_secs_f64();
            }
            results.push(FeatureResult { name: words[1..].join(" "), passed: true, note: String::new(), seconds: 0.0 });
            started = Instant::now();
            failed = false;
            continue;
        }
        if failed || results.is_empty() {
            continue;
        }
        if let Err(e) = run.step(&words) {
            let last = results.last_mut().unwrap();
            last.passed = false;
            last.note = format!("{line}: {e}");
            failed = true;
        }
    }
    if let Some(last) = results.last_mut() {
        last.seconds = started.elapsed().as_secs_f64();
    }
    results
}

/// A plain host for the core's own test of the runner: every command goes straight to the manager.
pub struct DirectHost(pub Manager);

impl Host for DirectHost {
    fn command(&self, name: &str, ids: &[u64], args: &[String]) -> Result<(), String> {
        let m = &self.0;
        match name {
            "add" | "add-as-mirrors" | "add-sound-only" => {
                let folder = Path::new(&args[0]);
                let start = args[1] == "start";
                let links: Vec<crate::addflow::PendingLink> = match name {
                    "add" => args[2..].iter().map(|u| crate::addflow::PendingLink { url: u.clone(), ..Default::default() }).collect(),
                    "add-sound-only" => args[2..]
                        .iter()
                        .map(|u| crate::addflow::PendingLink { url: u.clone(), stream: true, choice: Some(crate::addflow::sound_only_choice(m)), ..Default::default() })
                        .collect(),
                    _ => vec![crate::addflow::PendingLink { url: args[2].clone(), mirrors: args[3..].to_vec(), ..Default::default() }],
                };
                let mut session = crate::addflow::AddSession::new(m, links, folder, m.settings().connections, start);
                while let crate::addflow::Step::Ask(_) = session.step(m) {
                    session.answer(m, crate::addflow::Choice::KeepBoth, true);
                }
                Ok(())
            }
            "resume" => {
                m.resume(ids);
                Ok(())
            }
            "pause" => {
                m.pause(ids);
                Ok(())
            }
            "restart" => {
                m.restart(ids);
                Ok(())
            }
            "start-queue" => {
                m.start_queue();
                Ok(())
            }
            "stop-queue" => {
                m.stop_queue();
                Ok(())
            }
            "stop-all" => {
                m.pause_all();
                Ok(())
            }
            "delete" => {
                let _ = m.remove(ids, false);
                Ok(())
            }
            "delete-files" => {
                let _ = m.remove(ids, true);
                Ok(())
            }
            "add-mirror" => {
                m.add_mirrors(ids[0], args);
                Ok(())
            }
            "rename" => m.rename_file(ids[0], &args[0]).map(|_| ()),
            "move" => m.move_file(ids[0], Path::new(&args[0])).map(|_| ()),
            "note" => {
                m.set_note(ids[0], &args.join(" "));
                Ok(())
            }
            // With no app to count down, the core's own word that the queue is done stands in for the countdown.
            "cancel-when-done" => match m.take_queue_done() {
                Some(action) if Some(action) == crate::schedule::WhenDone::from_word(args.first().map(String::as_str).unwrap_or("")) => Ok(()),
                other => Err(format!("the queue isn't done with {} to do ({other:?})", args.first().map(String::as_str).unwrap_or("?"))),
            },
            "set-checksum" => {
                let algorithm = crate::checksum::Algorithm::ALL.into_iter().find(|a| a.name().eq_ignore_ascii_case(&args[0]) || a.extension() == args[0])
                    .ok_or("no such algorithm")?;
                m.set_checksum(ids[0], Some(crate::checksum::Checksum::new(algorithm, &args[1])?));
                Ok(())
            }
            other => Err(format!("no command {other}")),
        }
    }
}
