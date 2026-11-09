//! The HTTP client and what the engine reads from response headers.

use std::time::Duration;

use reqwest::header::{CONTENT_DISPOSITION, CONTENT_RANGE, ETAG, HeaderMap, LAST_MODIFIED};

/// One client for every download: HTTP/1.1 only, so each connection is its own TCP connection (HTTP/2 would fold
/// them into one), TLS with the system's certificates, and proxies from the system settings.
pub fn client() -> reqwest::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .http1_only()
        .user_agent(concat!("HerbsDownloader/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
        .build()
        .expect("the HTTP client's settings are fixed and valid")
}

/// The total size in `Content-Range: bytes a-b/total`, when the server knows it.
pub fn content_range_total(headers: &HeaderMap) -> Option<u64> {
    let v = headers.get(CONTENT_RANGE)?.to_str().ok()?;
    v.rsplit_once('/')?.1.trim().parse().ok()
}

/// The first byte in `Content-Range: bytes a-b/total`.
pub fn content_range_start(headers: &HeaderMap) -> Option<u64> {
    let v = headers.get(CONTENT_RANGE)?.to_str().ok()?;
    v.trim().strip_prefix("bytes ")?.split('-').next()?.trim().parse().ok()
}

/// What identifies this version of the file: the ETag (unless weak), else Last-Modified. Sent back in `If-Range`.
pub fn validator(headers: &HeaderMap) -> Option<String> {
    if let Some(e) = headers.get(ETAG).and_then(|v| v.to_str().ok())
        && !e.starts_with("W/") {
            return Some(e.to_string());
        }
    headers.get(LAST_MODIFIED).and_then(|v| v.to_str().ok()).map(str::to_string)
}

/// The file name from `Content-Disposition`, else the last part of the URL's path, else "download".
pub fn file_name(headers: &HeaderMap, url: &reqwest::Url) -> String {
    let from_header = headers
        .get(CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .and_then(disposition_name);
    let from_url = || {
        url.path_segments()
            .and_then(|mut s| s.next_back().map(str::to_string))
            .map(|s| percent_decode(&s))
            .filter(|s| !s.is_empty())
    };
    let name = from_header.or_else(from_url).unwrap_or_default();
    let clean = sanitize(&name);
    if clean.is_empty() { "download".to_string() } else { clean }
}

fn disposition_name(value: &str) -> Option<String> {
    let mut plain = None;
    for part in value.split(';').map(str::trim) {
        let Some((key, val)) = part.split_once('=') else { continue };
        match key.trim().to_ascii_lowercase().as_str() {
            "filename*" => {
                // RFC 5987: charset'language'percent-encoded
                let encoded = val.trim().splitn(3, '\'').nth(2)?;
                return Some(percent_decode(encoded));
            }
            "filename" => plain = Some(val.trim().trim_matches('"').to_string()),
            _ => {}
        }
    }
    plain
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Keeps a name safe as one file name on every platform.
fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() { '_' } else { c })
        .collect();
    cleaned.trim().trim_start_matches('.').trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    fn headers(pairs: &[(reqwest::header::HeaderName, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(k.clone(), HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn reads_content_range() {
        let h = headers(&[(CONTENT_RANGE, "bytes 100-199/5000")]);
        assert_eq!(content_range_total(&h), Some(5000));
        assert_eq!(content_range_start(&h), Some(100));
        assert_eq!(content_range_total(&headers(&[(CONTENT_RANGE, "bytes 0-9/*")])), None);
    }

    #[test]
    fn prefers_strong_etag() {
        assert_eq!(validator(&headers(&[(ETAG, "\"abc\""), (LAST_MODIFIED, "x")])), Some("\"abc\"".into()));
        assert_eq!(validator(&headers(&[(ETAG, "W/\"abc\""), (LAST_MODIFIED, "x")])), Some("x".into()));
    }

    #[test]
    fn names_files() {
        let url = reqwest::Url::parse("https://example.com/files/My%20File.zip?x=1").unwrap();
        assert_eq!(file_name(&HeaderMap::new(), &url), "My File.zip");
        let h = headers(&[(CONTENT_DISPOSITION, "attachment; filename=\"report.pdf\"")]);
        assert_eq!(file_name(&h, &url), "report.pdf");
        let h = headers(&[(CONTENT_DISPOSITION, "attachment; filename=\"a.pdf\"; filename*=UTF-8''r%C3%A9sum%C3%A9.pdf")]);
        assert_eq!(file_name(&h, &url), "résumé.pdf");
        let h = headers(&[(CONTENT_DISPOSITION, "attachment; filename=\"../../etc/passwd\"")]);
        assert_eq!(file_name(&h, &url), "_.._etc_passwd");
        let root = reqwest::Url::parse("https://example.com/").unwrap();
        assert_eq!(file_name(&HeaderMap::new(), &root), "download");
    }
}
