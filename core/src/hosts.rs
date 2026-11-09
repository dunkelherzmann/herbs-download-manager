//! Remembered connection limits per server (enhancements.md): when a server turns away extra connections, the most it
//! accepted is kept, in `host-limits.json`, so later downloads start within it instead of each learning it again.
//! A limit is forgotten after a week, or from Settings.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

const WEEK: u64 = 7 * 24 * 60 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limit {
    /// Connections the server accepted at once.
    pub connections: u32,
    /// Seconds since 1970 when it was learned.
    pub learned: u64,
}

#[derive(Debug)]
pub struct Learned {
    file: Option<PathBuf>,
    limits: Mutex<HashMap<String, Limit>>,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Learned {
    /// The limits kept in `dir`; `None` keeps them in memory only (tests).
    pub fn open(dir: Option<&Path>) -> Learned {
        let file = dir.map(|d| d.join("host-limits.json"));
        let limits = file
            .as_ref()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|b| serde_json::from_slice::<HashMap<String, Limit>>(&b).ok())
            .unwrap_or_default();
        let learned = Learned { file, limits: Mutex::new(limits) };
        learned.prune();
        learned
    }

    fn prune(&self) {
        let cutoff = now().saturating_sub(WEEK);
        self.limits.lock().unwrap().retain(|_, l| l.learned >= cutoff);
    }

    fn save(&self) {
        let Some(file) = &self.file else { return };
        let Ok(text) = serde_json::to_vec_pretty(&*self.limits.lock().unwrap()) else { return };
        let tmp = file.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(tmp, file);
        }
    }

    /// The connections `host` accepts, when known and less than a week old.
    pub fn get(&self, host: &str) -> Option<u32> {
        let cutoff = now().saturating_sub(WEEK);
        self.limits.lock().unwrap().get(host).filter(|l| l.learned >= cutoff).map(|l| l.connections)
    }

    /// A server turned away connections beyond `connections`.
    pub fn learn(&self, host: &str, connections: u32) {
        let changed = {
            let mut limits = self.limits.lock().unwrap();
            let new = Limit { connections: connections.max(1), learned: now() };
            limits.insert(host.to_string(), new) != Some(new)
        };
        if changed {
            self.save();
        }
    }

    /// Every remembered limit, by host.
    pub fn all(&self) -> Vec<(String, Limit)> {
        self.prune();
        let mut v: Vec<(String, Limit)> = self.limits.lock().unwrap().iter().map(|(h, l)| (h.clone(), *l)).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    /// Settings ▸ Forget: every download learns again.
    pub fn forget(&self) {
        self.limits.lock().unwrap().clear();
        self.save();
    }
}

impl Default for Learned {
    fn default() -> Self {
        Learned::open(None)
    }
}

/// Settings' line for the remembered limits ("2 servers limit connections") and its hover text, one server a line.
pub fn describe(limits: &[(String, u32)]) -> (String, String) {
    let summary = match limits.len() {
        0 => "No server has limited connections".to_string(),
        1 => "1 server limits connections".to_string(),
        n => format!("{n} servers limit connections"),
    };
    let detail = if limits.is_empty() {
        "Servers that turn away extra connections are remembered here for a week".to_string()
    } else {
        limits.iter().map(|(h, n)| format!("{h}: {n} connection{}", if *n == 1 { "" } else { "s" })).collect::<Vec<_>>().join("\n")
    };
    (summary, detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_are_kept_between_runs_and_forgotten() {
        let dir = std::env::temp_dir().join(format!("hd-hosts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = Learned::open(Some(&dir));
        a.learn("example.com:443", 2);
        assert_eq!(Learned::open(Some(&dir)).get("example.com:443"), Some(2));
        a.forget();
        assert_eq!(Learned::open(Some(&dir)).get("example.com:443"), None);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
