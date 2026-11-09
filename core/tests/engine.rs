//! The download engine against the local test server (tools/hd-mock).

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use hd_core::transfer::{self, Outcome, Progress, Request, Tuning};
use hd_mock::{MockFile, MockServer, pattern};

const MIB: usize = 1024 * 1024;

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hd-engine-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn request(url: String, dir: &std::path::Path, connections: u32) -> Request {
    Request { url, folder: dir.to_path_buf(), name: None, connections, connections_per_server: 32, retries: 5 }
}

fn fast() -> Tuning {
    Tuning {
        stall_timeout: Duration::from_millis(500),
        retry_delays: vec![Duration::from_millis(50)],
        refusal_wait: Duration::from_millis(200),
        ..Tuning::default()
    }
}

async fn run(req: Request, tuning: Tuning) -> (Outcome, std::sync::Arc<Progress>) {
    let progress = Progress::new();
    let (_control, stop) = transfer::control();
    let outcome = tokio::time::timeout(
        Duration::from_secs(60),
        transfer::download(&hd_core::http::client(), req, tuning, progress.clone(), stop),
    )
    .await
    .expect("the download should end");
    (outcome, progress)
}

fn finished(outcome: &Outcome) -> PathBuf {
    match outcome {
        Outcome::Finished(p) => p.clone(),
        other => panic!("expected Finished, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn one_connection_downloads_the_whole_file() {
    let server = MockServer::start().await.unwrap();
    let data = pattern(3 * MIB + 17, 1);
    server.put("/one.bin", MockFile::new(data.clone()));
    let dir = folder("one");
    let (outcome, progress) = run(request(server.url("/one.bin"), &dir, 1), fast()).await;
    let path = finished(&outcome);
    assert_eq!(path, dir.join("one.bin"));
    assert_eq!(std::fs::read(&path).unwrap(), data);
    assert!(!transfer::part_path(&path).exists(), "the .hdpart file is renamed");
    assert_eq!(server.stats("/one.bin").requests, 1);
    assert_eq!(progress.size(), Some(data.len() as u64));
    assert!(progress.resumable.load(Ordering::SeqCst));
}

#[tokio::test(flavor = "multi_thread")]
async fn many_connections_split_the_file_and_put_it_together() {
    let server = MockServer::start().await.unwrap();
    let data = pattern(24 * MIB + 5, 2);
    let mut file = MockFile::new(data.clone());
    file.rate = Some(8 * MIB as u64); // slow enough that every connection gets a share
    server.put("/many.bin", file);
    let dir = folder("many");
    let (outcome, progress) = run(request(server.url("/many.bin"), &dir, 32), fast()).await;
    let path = finished(&outcome);
    assert_eq!(std::fs::read(&path).unwrap(), data);
    let peak = progress.peak.load(Ordering::SeqCst);
    assert!(peak > 16, "up to 32 connections at once, saw {peak}");
    assert!(peak <= 32);
    assert!(server.stats("/many.bin").requests >= 32);
}

#[tokio::test(flavor = "multi_thread")]
async fn small_files_are_not_split_below_256_kib() {
    let server = MockServer::start().await.unwrap();
    let data = pattern(700 * 1024, 3);
    let mut file = MockFile::new(data.clone());
    file.rate = Some(1024 * 1024);
    server.put("/small.bin", file);
    let dir = folder("small");
    let (outcome, progress) = run(request(server.url("/small.bin"), &dir, 32), fast()).await;
    assert_eq!(std::fs::read(finished(&outcome)).unwrap(), data);
    assert!(progress.peak.load(Ordering::SeqCst) <= 2, "700 KiB splits once at most");
}

#[tokio::test(flavor = "multi_thread")]
async fn without_ranges_one_connection_reads_to_the_end() {
    let server = MockServer::start().await.unwrap();
    let data = pattern(2 * MIB, 4);
    let mut file = MockFile::new(data.clone());
    file.ranges = false;
    server.put("/plain.bin", file);
    let dir = folder("plain");
    let (outcome, progress) = run(request(server.url("/plain.bin"), &dir, 32), fast()).await;
    assert_eq!(std::fs::read(finished(&outcome)).unwrap(), data);
    assert_eq!(server.stats("/plain.bin").requests, 1);
    assert!(!progress.resumable.load(Ordering::SeqCst));
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_size_reads_until_the_server_closes() {
    let server = MockServer::start().await.unwrap();
    let data = pattern(MIB + 3, 5);
    let mut file = MockFile::new(data.clone());
    file.ranges = false;
    file.content_length = false;
    server.put("/stream", file);
    let dir = folder("stream");
    let (outcome, progress) = run(request(server.url("/stream"), &dir, 8), fast()).await;
    assert_eq!(std::fs::read(finished(&outcome)).unwrap(), data);
    assert_eq!(progress.size(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stalled_connection_hands_its_range_back() {
    let server = MockServer::start().await.unwrap();
    let data = pattern(4 * MIB, 6);
    let mut file = MockFile::new(data.clone());
    file.stall_after = Some(100 * 1024);
    file.stall_first = 3; // the first three connections go silent
    server.put("/stall.bin", file);
    let dir = folder("stall");
    let started = Instant::now();
    let (outcome, _) = run(request(server.url("/stall.bin"), &dir, 4), fast()).await;
    assert_eq!(std::fs::read(finished(&outcome)).unwrap(), data);
    assert!(started.elapsed() < Duration::from_secs(20));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_refuses_extra_connections_keeps_the_ones_it_accepted() {
    let server = MockServer::start().await.unwrap();
    let data = pattern(8 * MIB, 7);
    let mut file = MockFile::new(data.clone());
    file.max_connections = Some(3);
    file.rate = Some(4 * MIB as u64);
    server.put("/limited.bin", file);
    let dir = folder("limited");
    let (outcome, progress) = run(request(server.url("/limited.bin"), &dir, 16), fast()).await;
    assert_eq!(std::fs::read(finished(&outcome)).unwrap(), data);
    assert!(progress.peak.load(Ordering::SeqCst) <= 3);
    let refused = server.stats("/limited.bin").refused;
    assert!(refused >= 1, "the server turned some away");
    assert!(refused < 40, "refused connections wait instead of hammering ({refused})");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_that_changes_on_the_server_fails_the_download() {
    let server = MockServer::start().await.unwrap();
    let mut file = MockFile::new(pattern(4 * MIB, 8));
    file.stall_after = Some(MIB as u64); // the first connection goes silent after 1 MiB…
    file.stall_first = 1;
    server.put("/changing.bin", file);
    let dir = folder("changing");
    let req = request(server.url("/changing.bin"), &dir, 1);
    let progress = Progress::new();
    let (_c, stop) = transfer::control();
    let client = hd_core::http::client();
    let task = tokio::spawn({
        let progress = progress.clone();
        async move { transfer::download(&client, req, fast(), progress, stop).await }
    });
    while progress.downloaded.load(Ordering::SeqCst) < 512 * 1024 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // …and before it reconnects the file changes, so the reconnect's If-Range no longer matches.
    server.change("/changing.bin", pattern(4 * MIB, 9));
    let outcome = tokio::time::timeout(Duration::from_secs(30), task).await.unwrap().unwrap();
    assert_eq!(outcome, Outcome::Failed("File changed on the server".into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_files_fail_at_once_with_the_reason() {
    let server = MockServer::start().await.unwrap();
    let dir = folder("missing");
    let started = Instant::now();
    let (outcome, _) = run(request(server.url("/nope.bin"), &dir, 8), Tuning::default()).await;
    assert_eq!(outcome, Outcome::Failed("404 Not Found".into()));
    assert!(started.elapsed() < Duration::from_secs(2), "no retries for a 404");
}

#[tokio::test(flavor = "multi_thread")]
async fn pausing_stops_every_connection() {
    let server = MockServer::start().await.unwrap();
    let mut file = MockFile::new(pattern(16 * MIB, 10));
    file.rate = Some(MIB as u64);
    server.put("/pause.bin", file);
    let dir = folder("pause");
    let req = request(server.url("/pause.bin"), &dir, 8);
    let progress = Progress::new();
    let (control, stop) = transfer::control();
    let client = hd_core::http::client();
    let task = tokio::spawn({
        let progress = progress.clone();
        async move { transfer::download(&client, req, fast(), progress, stop).await }
    });
    while progress.downloaded.load(Ordering::SeqCst) < 256 * 1024 {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    control.pause();
    let outcome = tokio::time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
    assert_eq!(outcome, Outcome::Paused);
    assert_eq!(progress.active.load(Ordering::SeqCst), 0);
    assert!(dir.join("pause.bin.hdpart").exists(), "the partial file stays for resuming");
}

#[test]
fn taken_names_get_a_number() {
    let dir = folder("names");
    std::fs::write(dir.join("a.zip"), b"x").unwrap();
    std::fs::write(dir.join("a (1).zip.hdpart"), b"x").unwrap();
    assert_eq!(transfer::unique_path(&dir, "a.zip"), dir.join("a (2).zip"));
    assert_eq!(transfer::unique_path(&dir, "b.zip"), dir.join("b.zip"));
}
