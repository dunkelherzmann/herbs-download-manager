//! Checksums (§6 Options, §9 S8): a download's expected MD5, SHA-1, SHA-256 or SHA-512, verified over the whole file
//! before it gets its final name; and checksum files (`<link>.sha256` and the like) published next to a link.

use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Algorithm {
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

impl Algorithm {
    pub const ALL: [Algorithm; 4] = [Algorithm::Md5, Algorithm::Sha1, Algorithm::Sha256, Algorithm::Sha512];

    pub fn name(self) -> &'static str {
        match self {
            Algorithm::Md5 => "MD5",
            Algorithm::Sha1 => "SHA-1",
            Algorithm::Sha256 => "SHA-256",
            Algorithm::Sha512 => "SHA-512",
        }
    }

    /// Hex digits in a value.
    pub fn hex_len(self) -> usize {
        match self {
            Algorithm::Md5 => 32,
            Algorithm::Sha1 => 40,
            Algorithm::Sha256 => 64,
            Algorithm::Sha512 => 128,
        }
    }

    /// The extension of its checksum files (`.sha256`).
    pub fn extension(self) -> &'static str {
        match self {
            Algorithm::Md5 => "md5",
            Algorithm::Sha1 => "sha1",
            Algorithm::Sha256 => "sha256",
            Algorithm::Sha512 => "sha512",
        }
    }

    /// The algorithm a pasted value's length implies.
    pub fn of_value(value: &str) -> Option<Algorithm> {
        let v = value.trim();
        if !v.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        Algorithm::ALL.into_iter().find(|a| a.hex_len() == v.len())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checksum {
    pub algorithm: Algorithm,
    /// Lowercase hex.
    pub value: String,
}

impl Checksum {
    /// A checksum from a pasted value: hex of the algorithm's length (spaces and case ignored).
    pub fn new(algorithm: Algorithm, value: &str) -> Result<Checksum, String> {
        let v: String = value.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
        if v.len() != algorithm.hex_len() || !v.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("A {} value is {} hexadecimal digits", algorithm.name(), algorithm.hex_len()));
        }
        Ok(Checksum { algorithm, value: v })
    }
}

/// The file's hash as lowercase hex. Reads it in 1 MiB pieces; call it off the async threads.
pub fn hash_file(path: &Path, algorithm: Algorithm) -> std::io::Result<String> {
    use sha2::Digest;
    fn run<D: sha2::Digest>(mut hasher: D, path: &Path) -> std::io::Result<String> {
        let mut file = std::fs::File::open(path)?;
        let mut buffer = vec![0u8; 1 << 20];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
    }
    match algorithm {
        Algorithm::Md5 => run(md5::Md5::new(), path),
        Algorithm::Sha1 => run(sha1::Sha1::new(), path),
        Algorithm::Sha256 => run(sha2::Sha256::new(), path),
        Algorithm::Sha512 => run(sha2::Sha512::new(), path),
    }
}

/// The value for `file_name` in a checksum file: `hash  name` lines (sha256sum and the like, one or many files), BSD
/// `SHA256 (name) = hash` lines, or a bare hash. A line naming another file is skipped.
pub fn parse(text: &str, algorithm: Algorithm, file_name: &str) -> Option<String> {
    let is_hash = |t: &str| t.len() == algorithm.hex_len() && t.chars().all(|c| c.is_ascii_hexdigit());
    let mut first = None;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        // BSD style.
        if let Some((left, right)) = line.split_once(") = ")
            && let Some((_, name)) = left.split_once(" (")
        {
            let value = right.trim();
            if is_hash(value) {
                if name == file_name {
                    return Some(value.to_ascii_lowercase());
                }
                continue;
            }
        }
        let mut words = line.split_whitespace();
        let Some(value) = words.next().filter(|w| is_hash(w)) else { continue };
        match words.next().map(|n| n.trim_start_matches('*')) {
            Some(name) if name.rsplit(['/', '\\']).next() == Some(file_name) => return Some(value.to_ascii_lowercase()),
            Some(_) => continue,
            None => {
                first.get_or_insert(value.to_ascii_lowercase());
            }
        }
    }
    first
}

/// Looks for a checksum file next to the link (`<link>.sha256`, then `.sha512`, `.sha1`, `.md5`) and returns the
/// value for `file_name` with the file it came from.
pub async fn look_for(client: &reqwest::Client, url: &str, file_name: &str) -> Option<(Checksum, String)> {
    let base = url.split(['?', '#']).next().unwrap_or(url);
    for algorithm in [Algorithm::Sha256, Algorithm::Sha512, Algorithm::Sha1, Algorithm::Md5] {
        let link = format!("{base}.{}", algorithm.extension());
        let Ok(Ok(response)) = tokio::time::timeout(std::time::Duration::from_secs(10), client.get(&link).send()).await else {
            continue;
        };
        if !response.status().is_success() || response.content_length().is_some_and(|n| n > 64 * 1024) {
            continue;
        }
        let Ok(Ok(body)) = tokio::time::timeout(std::time::Duration::from_secs(10), response.text()).await else { continue };
        if body.len() > 64 * 1024 {
            continue;
        }
        if let Some(value) = parse(&body, algorithm, file_name) {
            let from = link.rsplit('/').next().unwrap_or(&link).to_string();
            return Some((Checksum { algorithm, value }, from));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
    const B: &str = "60303ae22b998861bce3b28f33eec1be758a213c86c93c076dbe9f558c11c752";

    #[test]
    fn checksum_files_in_every_common_form() {
        assert_eq!(parse(&format!("{A}\n"), Algorithm::Sha256, "f.bin").as_deref(), Some(A));
        assert_eq!(parse(&format!("{B}  other.bin\n{A}  f.bin\n"), Algorithm::Sha256, "f.bin").as_deref(), Some(A));
        assert_eq!(parse(&format!("{A} *dir/f.bin"), Algorithm::Sha256, "f.bin").as_deref(), Some(A));
        assert_eq!(parse(&format!("SHA256 (other.bin) = {B}\nSHA256 (f.bin) = {A}"), Algorithm::Sha256, "f.bin").as_deref(), Some(A));
        assert_eq!(parse(&format!("{B}  other.bin"), Algorithm::Sha256, "f.bin"), None);
        assert_eq!(parse("<html>not found</html>", Algorithm::Sha256, "f.bin"), None);
    }

    #[test]
    fn values_are_checked_and_files_hashed() {
        assert_eq!(Algorithm::of_value(A), Some(Algorithm::Sha256));
        assert!(Checksum::new(Algorithm::Md5, A).is_err());
        let dir = std::env::temp_dir().join(format!("hd-checksum-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("t"), b"test").unwrap();
        assert_eq!(hash_file(&dir.join("t"), Algorithm::Sha256).unwrap(), A);
        assert_eq!(hash_file(&dir.join("t"), Algorithm::Md5).unwrap(), "098f6bcd4621d373cade4e832627b4f6");
        assert_eq!(hash_file(&dir.join("t"), Algorithm::Sha1).unwrap(), "a94a8fe5ccb19ba61c4c0873d391e987982fbbd3");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
