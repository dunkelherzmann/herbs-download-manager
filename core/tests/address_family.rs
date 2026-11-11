//! Links tied to the viewer's address family (SPECIFICATION.md §15): a site that hands the browser links tied to its
//! IPv6 network refuses the same computer over IPv4, and the other way round. The app connects over the family the
//! link names, or, refused over one, tries the other once and remembers it for the host.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use hd_core::http::{Family, named_family, send_matching};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A server on `localhost`, listening on IPv4 and IPv6 at the same port, that answers only over `good` and refuses
/// (403) over the other; it counts the refusals.
struct TwoFamilies {
    port: u16,
    refused: Arc<AtomicUsize>,
}

async fn serve(good: Family) -> Option<TwoFamilies> {
    // localhost must name both families here, or there's nothing to choose between.
    let names: Vec<SocketAddr> = tokio::net::lookup_host(("localhost", 0)).await.ok()?.collect();
    if !names.iter().any(|a| a.is_ipv4()) || !names.iter().any(|a| a.is_ipv6()) {
        eprintln!("localhost doesn't name both IPv4 and IPv6 here: skipped");
        return None;
    }
    for _ in 0..20 {
        let v4 = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.ok()?;
        let port = v4.local_addr().ok()?.port();
        let Ok(v6) = tokio::net::TcpListener::bind((Ipv6Addr::LOCALHOST, port)).await else { continue };
        let refused = Arc::new(AtomicUsize::new(0));
        for (listener, family) in [(v4, Family::V4), (v6, Family::V6)] {
            let refused = refused.clone();
            tokio::spawn(async move {
                while let Ok((mut socket, _)) = listener.accept().await {
                    let refused = refused.clone();
                    tokio::spawn(async move {
                        let mut buf = vec![0u8; 8192];
                        let mut got = 0;
                        while !buf[..got].windows(4).any(|w| w == b"\r\n\r\n") {
                            match socket.read(&mut buf[got..]).await {
                                Ok(0) | Err(_) => return,
                                Ok(n) => got += n,
                            }
                        }
                        let answer = if family == good {
                            "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Type: video/mp4\r\nConnection: close\r\n\r\nhello".to_string()
                        } else {
                            refused.fetch_add(1, Ordering::SeqCst);
                            "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
                        };
                        let _ = socket.write_all(answer.as_bytes()).await;
                    });
                }
            });
        }
        return Some(TwoFamilies { port, refused });
    }
    None
}

/// A JSON Web Token whose claims tie it to `cidr`.
fn token(cidr: &str) -> String {
    use base64::Engine;
    let enc = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
    format!("{}.{}.sig", enc(r#"{"alg":"HS256"}"#), enc(&format!(r#"{{"exp":4102444800,"ip_cidr":"{cidr}"}}"#)))
}

#[test]
fn a_link_names_its_family_by_token_or_address() {
    let url = |s: &str| reqwest::Url::parse(s).unwrap();
    assert_eq!(named_family(&url(&format!("https://a.example/pl/x/master.m3u8?token={}", token("2a0c:5a82:2a15:4700::/64")))), Some(Family::V6));
    assert_eq!(named_family(&url(&format!("https://a.example/v.mp4?t={}", token("185.134.89.245")))), Some(Family::V4));
    assert_eq!(named_family(&url("https://cdn.example/stream2/abc/def:1795584021:185.134.89.245:13631/index.m3u8")), Some(Family::V4));
    assert_eq!(named_family(&url("https://cdn.example/v/2a0c:5a82:2a15:4700::1/index.m3u8")), Some(Family::V6));
    assert_eq!(named_family(&url("https://cdn.example/v/10.0.0.1/index.m3u8")), None, "a private address isn't the viewer's");
    assert_eq!(named_family(&url("https://cdn.example/v/1.2.3/index.m3u8")), None, "a version number isn't an address");
    assert_eq!(named_family(&url("https://cdn.example/a/185.134.89.245/b/2a0c:5a82::1/c")), None, "both families: no telling");
    assert_eq!(named_family(&url("https://cdn.example/v/index.m3u8")), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_tied_to_a_family_is_fetched_over_it() {
    for (good, cidr) in [(Family::V6, "2a0c:5a82:2a15:4700::/64"), (Family::V4, "185.134.89.245/32")] {
        let Some(server) = serve(good).await else { return };
        let url = reqwest::Url::parse(&format!("http://localhost:{}/master.m3u8?token={}", server.port, token(cidr))).unwrap();
        let client = hd_core::http::client();
        let response = send_matching(&client, &url, |c| c.get(url.clone())).await.unwrap();
        assert_eq!(response.status(), 200, "{good:?}");
        assert_eq!(server.refused.load(Ordering::SeqCst), 0, "{good:?}: asked over the family the token names, never the other");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_over_one_family_is_asked_again_over_the_other_and_remembered() {
    for good in [Family::V6, Family::V4] {
        let Some(server) = serve(good).await else { return };
        // Nothing in the link says which family: whichever the system tries first, it ends up with the right one.
        let client = hd_core::http::client();
        for n in 0..3 {
            let url = reqwest::Url::parse(&format!("http://localhost:{}/piece-{n}.ts", server.port)).unwrap();
            let response = send_matching(&client, &url, |c| c.get(url.clone())).await.unwrap();
            assert_eq!(response.status(), 200, "{good:?}, piece {n}");
        }
        // At most the first piece was refused: the family that worked is remembered for the host.
        assert!(server.refused.load(Ordering::SeqCst) <= 1, "{good:?}: {} refusals", server.refused.load(Ordering::SeqCst));
        // Forget it for the next round (the same host, localhost, serves the other family next).
        hd_core::http::forget_families();
    }
}
