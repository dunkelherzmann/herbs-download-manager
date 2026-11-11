//! What the integration tests share: a scratch folder that goes when its test ends, passed or failed.

use std::path::{Path, PathBuf};

/// A folder in the system's temp folder, `hd-<label>-<pid>`, made empty; removed again when dropped (at the end of
/// the test, or as a failing test unwinds). They were left behind before: 46 GB of them filled a Windows disk
/// (2026-11-11). Declare it before what uses it (a `Manager`), so that goes first.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(label: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("hd-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
}

impl std::ops::Deref for Scratch {
    type Target = PathBuf;
    fn deref(&self) -> &PathBuf {
        &self.0
    }
}

impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Files a stopped download still holds open (Windows) can keep it a moment: tried a few times.
        for _ in 0..5 {
            if std::fs::remove_dir_all(&self.0).is_ok() || !self.0.exists() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}
