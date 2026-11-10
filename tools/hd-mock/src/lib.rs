//! A small HTTP/1.1 server for Herb's Download Manager tests.
//!
//! Each file it serves can turn range support on or off, change its contents (and so its ETag), stall after a
//! number of bytes, limit its speed, or refuse connections over a limit with 503. Several servers on different
//! ports act as mirrors of the same file. Every response closes its connection, so each request is one connection.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

/// How one served file behaves.
#[derive(Clone, Debug)]
pub struct MockFile {
    pub data: Arc<Vec<u8>>,
    /// Answer `Range` requests with 206 (otherwise send the whole file with 200).
    pub ranges: bool,
    /// Send `Content-Length` (otherwise the size is unknown to the client).
    pub content_length: bool,
    /// Stop sending (without closing) after this many body bytes in each response.
    pub stall_after: Option<u64>,
    /// Stall only the first this many requests (0: every request).
    pub stall_first: u64,
    /// Bytes per second for each response.
    pub rate: Option<u64>,
    /// Refuse requests with 503 while this many are already being answered.
    pub max_connections: Option<usize>,
    /// The file's ETag; changes whenever the contents change.
    pub etag: String,
    /// Send the ETag (otherwise the server gives no validator, and If-Range is ignored).
    pub validators: bool,
    /// Header values a request must carry (name in lowercase, value), or it gets 401: a login, cookies, a referer.
    pub requires: Vec<(String, String)>,
    /// More header lines to send ("Repr-Digest: sha-256=:…:").
    pub headers: Vec<String>,
}

impl MockFile {
    pub fn new(data: Vec<u8>) -> Self {
        let etag = etag_for(&data, 1);
        MockFile {
            data: Arc::new(data),
            ranges: true,
            content_length: true,
            stall_after: None,
            stall_first: 0,
            rate: None,
            max_connections: None,
            etag,
            validators: true,
            requires: Vec::new(),
            headers: Vec::new(),
        }
    }
}

/// Bytes that differ at every offset, so a misplaced range shows up in a comparison.
pub fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| ((i as u64).wrapping_mul(2_654_435_761) >> 13) as u8 ^ seed)
        .collect()
}

fn etag_for(data: &[u8], generation: u64) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in data.iter().step_by((data.len() / 4096).max(1)) {
        h = (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("\"{:x}-{:x}-{}\"", data.len(), h, generation)
}

#[derive(Default)]
struct Stats {
    requests: AtomicU64,
    active: AtomicUsize,
    refused: AtomicU64,
    bytes: AtomicU64,
}

struct State {
    files: Mutex<HashMap<String, MockFile>>,
    stats: Mutex<HashMap<String, Arc<Stats>>>,
    generation: AtomicU64,
}

impl State {
    fn stats(&self, path: &str) -> Arc<Stats> {
        self.stats.lock().unwrap().entry(path.to_string()).or_default().clone()
    }
}

/// Counters for one path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PathStats {
    pub requests: u64,
    pub active: usize,
    pub refused: u64,
    pub bytes: u64,
}

/// A running server; it stops when dropped.
pub struct MockServer {
    addr: SocketAddr,
    state: Arc<State>,
    stop: watch::Sender<bool>,
}

impl MockServer {
    /// Starts on 127.0.0.1 at a free port.
    pub async fn start() -> std::io::Result<Self> {
        Self::bind("127.0.0.1:0").await
    }

    pub async fn bind(addr: &str) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let addr = listener.local_addr()?;
        let state = Arc::new(State {
            files: Mutex::new(HashMap::new()),
            stats: Mutex::new(HashMap::new()),
            generation: AtomicU64::new(1),
        });
        let (stop, mut stopped) = watch::channel(false);
        let st = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stopped.changed() => break,
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { continue };
                        let st = st.clone();
                        let stop_rx = stopped.clone();
                        tokio::spawn(async move { let _ = serve(stream, st, stop_rx).await; });
                    }
                }
            }
        });
        Ok(MockServer { addr, state, stop })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    pub fn put(&self, path: &str, file: MockFile) {
        self.state.files.lock().unwrap().insert(path.to_string(), file);
    }

    /// Stops serving a file: responses under way are cut and later requests get 404.
    pub fn remove(&self, path: &str) {
        self.state.files.lock().unwrap().remove(path);
    }

    /// Replaces a file's contents and gives it a new ETag; responses under way are cut.
    pub fn change(&self, path: &str, data: Vec<u8>) {
        let generation = self.state.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let mut files = self.state.files.lock().unwrap();
        if let Some(f) = files.get_mut(path) {
            f.etag = etag_for(&data, generation);
            f.data = Arc::new(data);
        }
    }

    /// Changes how a file behaves, keeping its contents.
    pub fn update(&self, path: &str, edit: impl FnOnce(&mut MockFile)) {
        if let Some(f) = self.state.files.lock().unwrap().get_mut(path) {
            edit(f);
        }
    }

    pub fn stats(&self, path: &str) -> PathStats {
        let s = self.state.stats(path);
        PathStats {
            requests: s.requests.load(Ordering::SeqCst),
            active: s.active.load(Ordering::SeqCst),
            refused: s.refused.load(Ordering::SeqCst),
            bytes: s.bytes.load(Ordering::SeqCst),
        }
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

struct Request {
    method: String,
    path: String,
    headers: HashMap<String, String>,
}

async fn read_request(stream: &mut TcpStream) -> std::io::Result<Option<Request>> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() > 64 * 1024 {
            return Ok(None);
        }
    }
    let text = String::from_utf8_lossy(&buf);
    let mut lines = text.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let method = first.next().unwrap_or("").to_string();
    let path = first.next().unwrap_or("/").to_string();
    let headers = lines
        .take_while(|l| !l.is_empty())
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    Ok(Some(Request { method, path, headers }))
}

/// Parses `bytes=a-b`, `bytes=a-` and `bytes=-n` into an inclusive range, or `Err(())` when it can't be satisfied.
fn parse_range(value: &str, len: u64) -> Option<Result<(u64, u64), ()>> {
    let spec = value.strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let range = match (a.trim(), b.trim()) {
        ("", n) => {
            let n: u64 = n.parse().ok()?;
            if n == 0 || len == 0 {
                return Some(Err(()));
            }
            (len.saturating_sub(n), len - 1)
        }
        (a, "") => (a.parse().ok()?, len.saturating_sub(1)),
        (a, b) => (a.parse().ok()?, b.parse::<u64>().ok()?.min(len.saturating_sub(1))),
    };
    if range.0 >= len || range.0 > range.1 {
        return Some(Err(()));
    }
    Some(Ok(range))
}

async fn serve(mut stream: TcpStream, state: Arc<State>, mut stop: watch::Receiver<bool>) -> std::io::Result<()> {
    let Some(req) = read_request(&mut stream).await? else { return Ok(()) };
    // An exact address first, else the path without its query (signed links: "video.mp4?oh=…&oe=…").
    let file = {
        let files = state.files.lock().unwrap();
        files.get(&req.path).or_else(|| files.get(req.path.split('?').next().unwrap_or(""))).cloned()
    };
    let stats = state.stats(&req.path);
    let number = stats.requests.fetch_add(1, Ordering::SeqCst) + 1;

    let Some(file) = file else {
        return respond_simple(&mut stream, "404 Not Found").await;
    };
    if req.method != "GET" && req.method != "HEAD" {
        return respond_simple(&mut stream, "405 Method Not Allowed").await;
    }
    if let Some(max) = file.max_connections
        && stats.active.load(Ordering::SeqCst) >= max {
            stats.refused.fetch_add(1, Ordering::SeqCst);
            return respond_simple(&mut stream, "503 Service Unavailable").await;
        }
    stats.active.fetch_add(1, Ordering::SeqCst);
    let mut file = file;
    if file.stall_first > 0 && number > file.stall_first {
        file.stall_after = None;
    }
    let result = send_file(&mut stream, &req, &file, &stats, &state, &mut stop).await;
    stats.active.fetch_sub(1, Ordering::SeqCst);
    result
}

async fn send_file(
    stream: &mut TcpStream,
    req: &Request,
    file: &MockFile,
    stats: &Stats,
    state: &State,
    stop: &mut watch::Receiver<bool>,
) -> std::io::Result<()> {
    let len = file.data.len() as u64;
    if !file.requires.iter().all(|(name, value)| req.headers.get(name.as_str()).is_some_and(|v| v == value)) {
        let head = "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"mock\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        return stream.write_all(head.as_bytes()).await;
    }
    // A Range with an If-Range that no longer matches gets the whole file, as RFC 9110 says.
    let if_range_ok = !file.validators || req.headers.get("if-range").is_none_or(|v| *v == file.etag);
    let range = match req.headers.get("range") {
        Some(v) if file.ranges && if_range_ok => parse_range(v, len),
        _ => None,
    };
    let (status, start, end) = match range {
        Some(Err(())) => {
            let head = format!(
                "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{len}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            return stream.write_all(head.as_bytes()).await;
        }
        Some(Ok((a, b))) => ("206 Partial Content", a, b + 1),
        None => ("200 OK", 0, len),
    };
    let mut head = format!("HTTP/1.1 {status}\r\nConnection: close\r\n");
    if file.validators {
        head.push_str(&format!("ETag: {}\r\n", file.etag));
    }
    for line in &file.headers {
        head.push_str(line);
        head.push_str("\r\n");
    }
    // A file's own type when it gives one (a page, a playlist), else a file's.
    if !file.headers.iter().any(|h| h.to_ascii_lowercase().starts_with("content-type:")) {
        head.push_str("Content-Type: application/octet-stream\r\n");
    }
    if file.ranges {
        head.push_str("Accept-Ranges: bytes\r\n");
    }
    if status.starts_with("206") {
        head.push_str(&format!("Content-Range: bytes {start}-{}/{len}\r\n", end - 1));
    }
    if file.content_length || status.starts_with("206") {
        head.push_str(&format!("Content-Length: {}\r\n", end - start));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    if req.method == "HEAD" {
        return Ok(());
    }

    let body = &file.data[start as usize..end as usize];
    let chunk = match file.rate {
        Some(rate) => (rate / 20).max(1) as usize,
        None => 64 * 1024,
    };
    let mut sent: u64 = 0;
    for piece in body.chunks(chunk) {
        // A file removed or replaced while it is being sent cuts the responses already under way.
        let same = {
            let files = state.files.lock().unwrap();
            files.get(&req.path).or_else(|| files.get(req.path.split('?').next().unwrap_or(""))).is_some_and(|f| f.etag == file.etag)
        };
        if !same {
            return Err(std::io::Error::new(std::io::ErrorKind::ConnectionAborted, "the file went away"));
        }
        let piece = match file.stall_after {
            Some(limit) if sent + piece.len() as u64 > limit => &piece[..(limit - sent) as usize],
            _ => piece,
        };
        stream.write_all(piece).await?;
        sent += piece.len() as u64;
        stats.bytes.fetch_add(piece.len() as u64, Ordering::SeqCst);
        if file.stall_after.is_some_and(|limit| sent >= limit) {
            // Keep the connection open and silent until the server stops or the client gives up.
            let mut probe = [0u8; 1];
            tokio::select! {
                _ = stop.changed() => {}
                _ = stream.read(&mut probe) => {}
            }
            return Ok(());
        }
        if file.rate.is_some() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    Ok(())
}

async fn respond_simple(stream: &mut TcpStream, status: &str) -> std::io::Result<()> {
    let head = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    stream.write_all(head.as_bytes()).await
}

/// The feature tests' files (tests/features.scenario), the same on three servers (a link and two mirrors):
/// small.bin (4 MiB), slow.bin (16 MiB at 512 KiB/s per response), sum.bin with a right sum.bin.sha256 and bad.bin with
/// a wrong bad.bin.sha256. On ports P, P+1 and P+2, or on free ones.
pub async fn feature_servers(port: Option<u16>) -> std::io::Result<Vec<MockServer>> {
    use sha2::Digest;
    let small = pattern(4 << 20, 1);
    let slow = pattern(16 << 20, 2);
    let sum = pattern(2 << 20, 3);
    let hex = |d: &[u8]| sha2::Sha256::digest(d).iter().map(|b| format!("{b:02x}")).collect::<String>();
    let mut servers = Vec::new();
    for i in 0..3u16 {
        let server = match port {
            Some(p) => MockServer::bind(&format!("127.0.0.1:{}", p + i)).await?,
            None => MockServer::start().await?,
        };
        server.put("/small.bin", MockFile::new(small.clone()));
        let mut s = MockFile::new(slow.clone());
        s.rate = Some(512 * 1024);
        server.put("/slow.bin", s);
        server.put("/sum.bin", MockFile::new(sum.clone()));
        server.put("/sum.bin.sha256", MockFile::new(format!("{}  sum.bin\n", hex(&sum)).into_bytes()));
        server.put("/bad.bin", MockFile::new(sum.clone()));
        // Needs the user name herb and the password test-password.
        let mut login = MockFile::new(sum.clone());
        login.requires = vec![("authorization".into(), "Basic aGVyYjp0ZXN0LXBhc3N3b3Jk".into())];
        server.put("/login.bin", login);
        let mut dated = MockFile::new(sum.clone());
        dated.headers = vec!["Last-Modified: Sun, 06 Nov 1994 08:49:37 GMT".into()];
        server.put("/dated.bin", dated);
        // A streamed video (SPECIFICATION.md §15): core/tests/media/h264-ts, at /clip/.
        for (name, bytes) in [
            ("index.m3u8", &include_bytes!("../../../core/tests/media/h264-ts/index.m3u8")[..]),
            ("s0.ts", &include_bytes!("../../../core/tests/media/h264-ts/s0.ts")[..]),
            ("s1.ts", &include_bytes!("../../../core/tests/media/h264-ts/s1.ts")[..]),
            ("s2.ts", &include_bytes!("../../../core/tests/media/h264-ts/s2.ts")[..]),
        ] {
            server.put(&format!("/clip/{name}"), MockFile::new(bytes.to_vec()));
        }
        // A DASH video as Facebook serves reels (SPECIFICATION.md §15): core/tests/media/dash-ondemand, at /reel/.
        for (name, bytes) in [
            ("manifest.mpd", &include_bytes!("../../../core/tests/media/dash-ondemand/manifest.mpd")[..]),
            ("video.mp4", &include_bytes!("../../../core/tests/media/dash-ondemand/video.mp4")[..]),
            ("audio.mp4", &include_bytes!("../../../core/tests/media/dash-ondemand/audio.mp4")[..]),
        ] {
            server.put(&format!("/reel/{name}"), MockFile::new(bytes.to_vec()));
        }
        server.put("/bad.bin.sha256", MockFile::new(format!("{}  bad.bin\n", "0".repeat(64)).into_bytes()));
        servers.push(server);
    }
    Ok(servers)
}

/// small.bin's SHA-256, for the scenario's checksum set by hand.
pub fn feature_small_sha256() -> String {
    use sha2::Digest;
    sha2::Sha256::digest(pattern(4 << 20, 1)).iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn get(server: &MockServer, path: &str, headers: &[(&str, &str)]) -> (String, String, Vec<u8>) {
        let mut s = TcpStream::connect(server.addr()).await.unwrap();
        let mut req = format!("GET {path} HTTP/1.1\r\nHost: test\r\n");
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");
        s.write_all(req.as_bytes()).await.unwrap();
        let mut out = Vec::new();
        s.read_to_end(&mut out).await.unwrap();
        let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8_lossy(&out[..split]).to_string();
        let status = head.lines().next().unwrap().to_string();
        (status, head, out[split + 4..].to_vec())
    }

    #[tokio::test]
    async fn serves_whole_file_and_ranges() {
        let server = MockServer::start().await.unwrap();
        let data = pattern(100_000, 7);
        server.put("/f.bin", MockFile::new(data.clone()));

        let (status, head, body) = get(&server, "/f.bin", &[]).await;
        assert!(status.contains("200"), "{status}");
        assert!(head.contains("Accept-Ranges: bytes"));
        assert_eq!(body, data);

        let (status, head, body) = get(&server, "/f.bin", &[("Range", "bytes=10-19")]).await;
        assert!(status.contains("206"), "{status}");
        assert!(head.contains("Content-Range: bytes 10-19/100000"));
        assert_eq!(body, data[10..20]);

        let (_, _, body) = get(&server, "/f.bin", &[("Range", "bytes=-5")]).await;
        assert_eq!(body, data[99_995..]);

        let (status, _, _) = get(&server, "/f.bin", &[("Range", "bytes=200000-")]).await;
        assert!(status.contains("416"), "{status}");

        let (status, _, _) = get(&server, "/missing", &[]).await;
        assert!(status.contains("404"), "{status}");
        assert_eq!(server.stats("/f.bin").requests, 4);
    }

    #[tokio::test]
    async fn ranges_off_sends_whole_file() {
        let server = MockServer::start().await.unwrap();
        let mut file = MockFile::new(pattern(1000, 1));
        file.ranges = false;
        server.put("/f", file);
        let (status, head, body) = get(&server, "/f", &[("Range", "bytes=0-0")]).await;
        assert!(status.contains("200"), "{status}");
        assert!(!head.contains("Accept-Ranges"));
        assert_eq!(body.len(), 1000);
    }

    #[tokio::test]
    async fn changed_file_gets_new_etag_and_if_range_falls_back() {
        let server = MockServer::start().await.unwrap();
        server.put("/f", MockFile::new(pattern(1000, 1)));
        let (_, head, _) = get(&server, "/f", &[]).await;
        let etag = head.lines().find_map(|l| l.strip_prefix("ETag: ")).unwrap().to_string();

        let (status, _, _) = get(&server, "/f", &[("Range", "bytes=0-9"), ("If-Range", &etag)]).await;
        assert!(status.contains("206"));

        server.change("/f", pattern(1000, 2));
        let (status, head, body) = get(&server, "/f", &[("Range", "bytes=0-9"), ("If-Range", &etag)]).await;
        assert!(status.contains("200"), "{status}");
        assert!(!head.contains(&etag));
        assert_eq!(body, pattern(1000, 2));
    }

    #[tokio::test]
    async fn stalls_and_refuses_over_the_limit() {
        let server = MockServer::start().await.unwrap();
        let mut file = MockFile::new(pattern(100_000, 3));
        file.stall_after = Some(1000);
        file.max_connections = Some(1);
        server.put("/f", file);

        let mut first = TcpStream::connect(server.addr()).await.unwrap();
        first.write_all(b"GET /f HTTP/1.1\r\nHost: t\r\n\r\n").await.unwrap();
        let mut got = vec![0u8; 4096];
        let mut total = 0;
        let read = tokio::time::timeout(Duration::from_millis(500), async {
            loop {
                let n = first.read(&mut got).await.unwrap();
                total += n;
                if n == 0 {
                    break;
                }
            }
        })
        .await;
        assert!(read.is_err(), "a stalled response must not end");
        assert!(total > 1000 && total < 2000, "{total}");
        assert_eq!(server.stats("/f").active, 1);

        let (status, _, _) = get(&server, "/f", &[]).await;
        assert!(status.contains("503"), "{status}");
        assert_eq!(server.stats("/f").refused, 1);
    }

    #[test]
    fn range_parsing() {
        assert_eq!(parse_range("bytes=0-9", 100), Some(Ok((0, 9))));
        assert_eq!(parse_range("bytes=90-", 100), Some(Ok((90, 99))));
        assert_eq!(parse_range("bytes=-10", 100), Some(Ok((90, 99))));
        assert_eq!(parse_range("bytes=50-500", 100), Some(Ok((50, 99))));
        assert_eq!(parse_range("bytes=100-", 100), Some(Err(())));
        assert_eq!(parse_range("bytes=0-1,5-6", 100), None);
        assert_eq!(parse_range("items=0-1", 100), None);
    }
}
