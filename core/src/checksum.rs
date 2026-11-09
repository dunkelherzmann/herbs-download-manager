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

/// A checksum the server announces in its answer's headers (enhancements.md), the strongest it gives:
/// `Repr-Digest: sha-256=:BASE64:` and Google's `x-goog-hash: md5=BASE64` cover the whole file even in a partial
/// answer; `Digest: SHA-256=BASE64` and `Content-MD5: BASE64` cover only the body sent, so they count only when
/// `whole_body` (the server sent the whole file).
pub fn announced(headers: &reqwest::header::HeaderMap, whole_body: bool) -> Option<Checksum> {
    use base64::Engine;
    let decode = |v: &str| base64::engine::general_purpose::STANDARD.decode(v.trim().trim_matches(':')).ok();
    let hex = |b: Vec<u8>| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    let algorithm = |name: &str| match name.trim().to_ascii_lowercase().as_str() {
        "md5" => Some(Algorithm::Md5),
        "sha" | "sha1" | "sha-1" => Some(Algorithm::Sha1),
        "sha-256" | "sha256" => Some(Algorithm::Sha256),
        "sha-512" | "sha512" => Some(Algorithm::Sha512),
        _ => None,
    };
    let mut found: Vec<Checksum> = Vec::new();
    let mut take = |header: &str, pairs: bool| {
        for value in headers.get_all(header).iter().filter_map(|v| v.to_str().ok()) {
            let items: Vec<(&str, &str)> = if pairs {
                value.split(',').filter_map(|p| p.split_once('=')).collect()
            } else {
                vec![("md5", value)]
            };
            for (name, encoded) in items {
                if let (Some(a), Some(bytes)) = (algorithm(name), decode(encoded))
                    && let Ok(c) = Checksum::new(a, &hex(bytes))
                {
                    found.push(c);
                }
            }
        }
    };
    take("repr-digest", true);
    take("x-goog-hash", true);
    if whole_body {
        take("digest", true);
        take("content-md5", false);
    }
    found.into_iter().max_by_key(|c| c.algorithm.hex_len())
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
    fn checksums_the_server_announces() {
        use reqwest::header::{HeaderMap, HeaderValue};
        let mut h = HeaderMap::new();
        // "test": MD5 CY9rzUYh03PK3k6DJie09g==, SHA-256 n4bQgYhMfWWaL+qgxVrQFaO/TxsrC4Is0V1sFbDwCgg=
        h.insert("content-md5", HeaderValue::from_static("CY9rzUYh03PK3k6DJie09g=="));
        assert_eq!(announced(&h, false), None, "Content-MD5 covers only the body sent");
        assert_eq!(announced(&h, true).unwrap().value, "098f6bcd4621d373cade4e832627b4f6");
        h.insert("repr-digest", HeaderValue::from_static("sha-256=:n4bQgYhMfWWaL+qgxVrQFaO/TxsrC4Is0V1sFbDwCgg=:"));
        let c = announced(&h, false).unwrap();
        assert_eq!((c.algorithm, c.value.as_str()), (Algorithm::Sha256, A));
        let mut g = HeaderMap::new();
        g.insert("x-goog-hash", HeaderValue::from_static("crc32c=n03x6A==, md5=CY9rzUYh03PK3k6DJie09g=="));
        assert_eq!(announced(&g, false).unwrap().algorithm, Algorithm::Md5);
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
