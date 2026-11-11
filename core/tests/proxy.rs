//! The system's proxy (SPECIFICATION.md §9): when the app sets how to find it, every request to the internet goes
//! through the proxy it names, and this computer's own servers are reached directly. Its own process, since the
//! proxy setting is global.

use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A stand-in proxy: answers any request itself and keeps the request lines it got.
async fn fake_proxy() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let kept = seen.clone();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let kept = kept.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or("").to_string();
                kept.lock().unwrap().push(head);
                let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\nConnection: close\r\n\r\nvia the proxy").await;
            });
        }
    });
    (port, seen)
}

#[tokio::test(flavor = "multi_thread")]
async fn requests_go_through_the_proxy_the_system_names() {
    let (port, seen) = fake_proxy().await;
    hd_core::http::set_proxy_resolver(Arc::new(move |url| {
        // As a desktop's settings would answer: a proxy for this site, direct for others.
        (url.host_str() == Some("files.example.test")).then(|| format!("http://127.0.0.1:{port}"))
    }));
    let client = hd_core::http::client();
    let body = client.get("http://files.example.test/report.pdf").send().await.unwrap().text().await.unwrap();
    assert_eq!(body, "via the proxy");
    assert_eq!(seen.lock().unwrap().as_slice(), ["GET http://files.example.test/report.pdf HTTP/1.1"], "asked in proxy form");
    // This computer's own servers are reached directly, whatever the setting says.
    let (local_port, direct) = fake_proxy().await;
    hd_core::http::set_proxy_resolver(Arc::new(move |_| Some(format!("http://127.0.0.1:{port}"))));
    let client = hd_core::http::client();
    client.get(format!("http://127.0.0.1:{local_port}/x")).send().await.unwrap();
    assert_eq!(direct.lock().unwrap().as_slice(), ["GET /x HTTP/1.1"], "reached directly, not in proxy form");
    assert_eq!(seen.lock().unwrap().len(), 1, "the proxy wasn't asked for a local server");
}
