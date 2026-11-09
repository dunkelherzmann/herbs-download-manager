//! The HTTP client and what the engine reads from response headers.

use std::time::Duration;

use reqwest::header::{CONTENT_DISPOSITION, CONTENT_RANGE, ETAG, HeaderMap, LAST_MODIFIED};

/// One client for every download: HTTP/1.1 only, so each connection is its own TCP connection (HTTP/2 would fold
/// them into one), TLS with the system's certificates, and proxies from the system settings.
pub fn client() -> reqwest::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .http1_only()
        .user_agent(concat!("HerbsDownloadManager/", env!("CARGO_PKG_VERSION")))
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

/// The file's date on the server (`Last-Modified`, in the usual "Sun, 06 Nov 1994 08:49:37 GMT" form), as seconds
/// since 1970.
pub fn last_modified(headers: &HeaderMap) -> Option<u64> {
    parse_http_date(headers.get(LAST_MODIFIED)?.to_str().ok()?)
}

pub fn parse_http_date(text: &str) -> Option<u64> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    let [_, day, month, year, time, "GMT"] = parts.as_slice() else { return None };
    let month = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"].iter().position(|m| m == month)? as i64 + 1;
    let (day, year): (i64, i64) = (day.parse().ok()?, year.parse().ok()?);
    let hms: Vec<i64> = time.split(':').map(|x| x.parse().ok()).collect::<Option<_>>()?;
    let [h, m, s] = hms.as_slice() else { return None };
    // Days since 1970 for a date in the proleptic Gregorian calendar (Howard Hinnant's days_from_civil).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + h * 3600 + m * 60 + s).ok()
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
        assert_eq!(parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT"), Some(784_111_777));
        assert_eq!(parse_http_date("Sat, 29 Feb 2020 00:00:00 GMT"), Some(1_582_934_400));
        assert_eq!(parse_http_date("yesterday"), None);
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
