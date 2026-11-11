//! The HTTP client and what the engine reads from response headers.

use std::time::Duration;

use reqwest::header::{CONTENT_DISPOSITION, CONTENT_RANGE, ETAG, HeaderMap, LAST_MODIFIED};

/// One client for every download: HTTP/1.1 only, so each connection is its own TCP connection (HTTP/2 would fold
/// them into one), TLS with the system's certificates, and proxies from the system settings.
pub fn client() -> reqwest::Client {
    builder().build().expect("the HTTP client's settings are fixed and valid")
}

fn builder() -> reqwest::ClientBuilder {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .http1_only()
        .user_agent(concat!("HerbsDownloadManager/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(30))
}

/// An internet address family: IPv4 or IPv6.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    V4,
    V6,
}

impl Family {
    fn of(ip: &std::net::IpAddr) -> Family {
        if ip.is_ipv4() || matches!(ip, std::net::IpAddr::V6(v6) if v6.to_ipv4_mapped().is_some()) { Family::V4 } else { Family::V6 }
    }

    fn other(self) -> Family {
        if self == Family::V4 { Family::V6 } else { Family::V4 }
    }
}

/// Resolves names to one family's addresses only, so every connection goes over that family.
struct OneFamily(Family);

impl reqwest::dns::Resolve for OneFamily {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let (family, host) = (self.0, name.as_str().to_string());
        Box::pin(async move {
            let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.as_str(), 0)).await?.filter(|a| Family::of(&a.ip()) == family).collect();
            if addrs.is_empty() {
                return Err(format!("{host} has no {} address", if family == Family::V4 { "IPv4" } else { "IPv6" }).into());
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// The client that connects over `family` only.
fn family_client(family: Family) -> reqwest::Client {
    static V4: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    static V6: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    let cell = if family == Family::V4 { &V4 } else { &V6 };
    cell.get_or_init(|| builder().dns_resolver(std::sync::Arc::new(OneFamily(family))).build().expect("the HTTP client's settings are fixed and valid"))
        .clone()
}

/// Hosts whose links work over one family only, learned from a refusal over the other.
fn learned() -> &'static std::sync::Mutex<std::collections::HashMap<String, Family>> {
    static LEARNED: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, Family>>> = std::sync::OnceLock::new();
    LEARNED.get_or_init(Default::default)
}

/// Forgets the families learned for hosts (for tests).
#[doc(hidden)]
pub fn forget_families() {
    learned().lock().unwrap().clear();
}

/// The family a link is tied to, when it names the viewer's address: a token in its query (a JSON Web Token's `ip`
/// or `ip_cidr`), or an address written in its path or query (`…:185.134.89.245:…`). Sites tie a stream's links to
/// the address of the browser that asked for them; the app must then connect over the same family.
pub fn named_family(url: &reqwest::Url) -> Option<Family> {
    use base64::Engine;
    // A token's claims.
    for (_, value) in url.query_pairs() {
        let mut parts = value.split('.');
        if let (Some(_), Some(payload), Some(_)) = (parts.next(), parts.next(), parts.next())
            && let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('='))
            && let Ok(claims) = serde_json::from_slice::<serde_json::Value>(&bytes)
            && let Some(ip) = claims.get("ip_cidr").or(claims.get("ip")).and_then(|v| v.as_str())
            && let Ok(ip) = ip.split('/').next().unwrap_or("").parse::<std::net::IpAddr>()
        {
            return Some(Family::of(&ip));
        }
    }
    // An address written out: public ones only (a private or local one isn't the viewer's address on the internet).
    let text = format!("{}?{}", url.path(), url.query().unwrap_or(""));
    let mut found: Option<Family> = None;
    for piece in text.split(|c: char| !(c.is_ascii_hexdigit() || c == '.' || c == ':')) {
        for candidate in std::iter::once(piece).chain(piece.split(':')) {
            let Ok(ip) = candidate.parse::<std::net::IpAddr>() else { continue };
            let public = match ip {
                std::net::IpAddr::V4(v4) => !(v4.is_private() || v4.is_loopback() || v4.is_link_local() || v4.is_unspecified() || v4.octets()[0] == 0),
                std::net::IpAddr::V6(v6) => !(v6.is_loopback() || v6.is_unspecified()) && (v6.segments()[0] & 0xe000) == 0x2000,
            };
            if !public {
                continue;
            }
            let family = Family::of(&ip);
            if found.is_some_and(|f| f != family) {
                return None; // both families named: no telling
            }
            found = Some(family);
        }
    }
    found
}

/// Whether `host` has an address of `family`.
async fn has_family(host: &str, family: Family) -> bool {
    tokio::net::lookup_host((host, 0)).await.is_ok_and(|mut a| a.any(|a| Family::of(&a.ip()) == family))
}

/// Sends the request `build` makes for `url`, over the address family the link is tied to when it names one (or the
/// one that worked before for its host). A refusal (403) over one family, from a host that has the other, is asked
/// once more over the other, and the family that works is remembered for the host: a site that ties its links to
/// the browser's IPv6 network refuses the same computer over IPv4, and the other way round.
pub async fn send_matching(client: &reqwest::Client, url: &reqwest::Url, build: impl Fn(&reqwest::Client) -> reqwest::RequestBuilder) -> reqwest::Result<reqwest::Response> {
    let host = url.host_str().unwrap_or("").trim_matches(['[', ']']).to_string();
    // A link to an address (not a name) connects there whatever is chosen.
    if host.parse::<std::net::IpAddr>().is_ok() || host.is_empty() {
        return build(client).send().await;
    }
    // What the link itself says wins over what its host taught before.
    let want = named_family(url).or_else(|| learned().lock().unwrap().get(&host).copied());
    let chosen = match want {
        Some(f) if has_family(&host, f).await => Some(f),
        _ => None,
    };
    let response = build(&chosen.map(family_client).unwrap_or_else(|| client.clone())).send().await?;
    if response.status() != reqwest::StatusCode::FORBIDDEN {
        return Ok(response);
    }
    let Some(used) = response.remote_addr().map(|a| Family::of(&a.ip())) else { return Ok(response) };
    let other = used.other();
    if !has_family(&host, other).await {
        return Ok(response);
    }
    match build(&family_client(other)).send().await {
        Ok(again) if again.status().is_success() => {
            learned().lock().unwrap().insert(host, other);
            Ok(again)
        }
        _ => Ok(response),
    }
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
/// An answer that is a web page (`text/html`, XHTML) rather than a file.
pub fn is_page(headers: &HeaderMap) -> bool {
    headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| { let t = t.to_ascii_lowercase(); t.starts_with("text/html") || t.starts_with("application/xhtml") })
}

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
