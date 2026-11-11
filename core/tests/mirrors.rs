//! Mirrors (SPECIFICATION.md §7) against several local test servers serving the same file.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use hd_core::transfer::{self, Control, Outcome, Progress, Request, SourceStatus, Tuning};
use hd_mock::{MockFile, MockServer, pattern};

const MIB: usize = 1024 * 1024;

/// The test's own folder (with `files` in it for downloads), removed when the test ends.
fn folder(name: &str) -> common::Scratch {
    let dir = common::Scratch::new(&format!("mirrors-{name}"));
    dir
}

fn fast() -> Tuning {
    Tuning {
        stall_timeout: Duration::from_millis(500),
        retry_delays: vec![Duration::from_millis(50)],
        refusal_wait: Duration::from_millis(200),
        tick: Duration::from_millis(100),
        slow_after: Duration::from_millis(400),
        ..Tuning::default()
    }
}

fn request(url: String, mirrors: Vec<String>, dir: &Path, connections: u32) -> Request {
    Request {
        url,
        mirrors,
        folder: dir.to_path_buf(),
        connections,
        connections_per_server: 32,
        retries: 3,
        state_file: Some(dir.join("state.json")),
        ..Request::default()
    }
}

/// Servers that each serve `data` at `/f.bin`, `rate` bytes per second per response (None: as fast as possible).
async fn servers(n: usize, data: &[u8], rate: Option<u64>) -> Vec<MockServer> {
    let mut out = Vec::new();
    for _ in 0..n {
        let s = MockServer::start().await.unwrap();
        let mut f = MockFile::new(data.to_vec());
        f.rate = rate;
        s.put("/f.bin", f);
        out.push(s);
    }
    out
}

fn spawn(req: Request, tuning: Tuning) -> (Control, Arc<Progress>, tokio::task::JoinHandle<Outcome>) {
    let progress = Progress::new();
    let control = transfer::control();
    let task = tokio::spawn({
        let (progress, control) = (progress.clone(), control.clone());
        async move { transfer::download(&hd_core::http::client(), req, tuning, progress, control).await }
    });
    (control, progress, task)
}

async fn finish(task: tokio::task::JoinHandle<Outcome>) -> Outcome {
    tokio::time::timeout(Duration::from_secs(60), task).await.expect("the download should end").unwrap()
}

async fn wait_for_bytes(progress: &Progress, n: u64) {
    while progress.downloaded.load(Ordering::SeqCst) < n {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn finished(outcome: &Outcome) -> PathBuf {
    match outcome {
        Outcome::Finished(p) => p.clone(),
        other => panic!("expected Finished, got {other:?}"),
    }
}

fn status_of(progress: &Progress, url: &str) -> SourceStatus {
    progress.sources().into_iter().find(|s| s.url == url).unwrap().status
}

#[tokio::test(flavor = "multi_thread")]
async fn connections_spread_over_every_mirror() {
    let data = pattern(16 * MIB, 1);
    let s = servers(3, &data, Some(2 * MIB as u64)).await;
    let dir = folder("spread");
    let req = request(s[0].url("/f.bin"), vec![s[1].url("/f.bin"), s[2].url("/f.bin")], &dir, 12);
    let (_c, progress, task) = spawn(req, fast());
    let outcome = finish(task).await;
    assert_eq!(std::fs::read(finished(&outcome)).unwrap(), data);
    for server in &s {
        let bytes = server.stats("/f.bin").bytes;
        assert!(bytes > 2 * MIB as u64, "every server served a share ({bytes})");
    }
    assert_eq!(progress.sources().len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mirror_with_a_different_size_is_never_used() {
    let data = pattern(4 * MIB, 2);
    let s = servers(1, &data, Some(4 * MIB as u64)).await;
    let other = servers(1, &pattern(4 * MIB + 1, 2), None).await;
    let dir = folder("different");
    let mirror = other[0].url("/f.bin");
    let (_c, progress, task) = spawn(request(s[0].url("/f.bin"), vec![mirror.clone()], &dir, 8), fast());
    let outcome = finish(task).await;
    assert_eq!(std::fs::read(finished(&outcome)).unwrap(), data);
    assert_eq!(status_of(&progress, &mirror), SourceStatus::DifferentFile);
    assert_eq!(other[0].stats("/f.bin").requests, 1, "only the check");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mirror_without_ranges_is_not_used() {
    let data = pattern(4 * MIB, 3);
    let s = servers(1, &data, Some(4 * MIB as u64)).await;
    let plain = MockServer::start().await.unwrap();
    let mut f = MockFile::new(data.clone());
    f.ranges = false;
    plain.put("/f.bin", f);
    let dir = folder("noranges");
    let mirror = plain.url("/f.bin");
    let (_c, progress, task) = spawn(request(s[0].url("/f.bin"), vec![mirror.clone()], &dir, 8), fast());
    assert_eq!(std::fs::read(finished(&finish(task).await)).unwrap(), data);
    assert_eq!(status_of(&progress, &mirror), SourceStatus::NoResumeSupport);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dead_link_with_a_working_mirror_still_downloads() {
    let data = pattern(4 * MIB, 4);
    let s = servers(1, &data, None).await;
    let dead = MockServer::start().await.unwrap();
    let dir = folder("deadlink");
    let link = dead.url("/f.bin");
    let (_c, progress, task) = spawn(request(link.clone(), vec![s[0].url("/f.bin")], &dir, 8), fast());
    let path = finished(&finish(task).await);
    assert_eq!(path, dir.join("f.bin"));
    assert_eq!(std::fs::read(path).unwrap(), data);
    assert_eq!(status_of(&progress, &link), SourceStatus::Failed("404 Not Found".into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_source_that_dies_midway_leaves_its_ranges_to_the_others() {
    let data = pattern(16 * MIB, 5);
    let s = servers(2, &data, Some(2 * MIB as u64)).await;
    let dir = folder("dies");
    let (_c, progress, task) = spawn(request(s[0].url("/f.bin"), vec![s[1].url("/f.bin")], &dir, 8), fast());
    wait_for_bytes(&progress, 2 * MIB as u64).await;
    s[0].remove("/f.bin");
    assert_eq!(std::fs::read(finished(&finish(task).await)).unwrap(), data);
    assert!(matches!(status_of(&progress, &s[0].url("/f.bin")), SourceStatus::Failed(_)));
}

#[tokio::test(flavor = "multi_thread")]
async fn when_every_source_fails_the_download_says_why() {
    let a = MockServer::start().await.unwrap();
    let b = MockServer::start().await.unwrap();
    let dir = folder("allfail");
    let (_c, _p, task) = spawn(request(a.url("/x"), vec![b.url("/x")], &dir, 8), fast());
    assert_eq!(finish(task).await, Outcome::Failed("All sources failed: 404 Not Found".into()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mirror_added_during_the_download_gets_connections() {
    let data = pattern(16 * MIB, 6);
    let s = servers(2, &data, Some(MIB as u64)).await;
    let dir = folder("during");
    let (control, progress, task) = spawn(request(s[0].url("/f.bin"), vec![], &dir, 8), fast());
    wait_for_bytes(&progress, MIB as u64).await;
    control.add_mirrors(vec![s[1].url("/f.bin"), s[0].url("/f.bin")]); // the second is a duplicate: ignored
    assert_eq!(std::fs::read(finished(&finish(task).await)).unwrap(), data);
    assert!(s[1].stats("/f.bin").bytes > MIB as u64, "the new mirror served part of the file");
    assert_eq!(progress.sources().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mirror_added_while_paused_finishes_an_expired_link() {
    let data = pattern(8 * MIB, 7);
    let s = servers(2, &data, Some(MIB as u64)).await;
    let dir = folder("after");
    let req = request(s[0].url("/f.bin"), vec![], &dir, 4);
    let (control, progress, task) = spawn(req.clone(), fast());
    wait_for_bytes(&progress, MIB as u64).await;
    control.pause();
    assert_eq!(finish(task).await, Outcome::Paused);

    s[0].remove("/f.bin"); // the link expired
    let added = transfer::add_mirrors_to_state(&dir.join("state.json"), &[s[1].url("/f.bin"), s[0].url("/f.bin")]).unwrap();
    assert_eq!(added, 1);
    s[1].update("/f.bin", |f| f.rate = None);
    let (_c, _p, task) = spawn(req, fast());
    assert_eq!(std::fs::read(finished(&finish(task).await)).unwrap(), data);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mirror_whose_file_changes_is_dropped() {
    let data = pattern(16 * MIB, 8);
    let s = servers(2, &data, Some(MIB as u64)).await;
    let dir = folder("mirrorchanged");
    let mirror = s[1].url("/f.bin");
    let (_c, progress, task) = spawn(request(s[0].url("/f.bin"), vec![mirror.clone()], &dir, 8), fast());
    while s[1].stats("/f.bin").bytes < MIB as u64 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    s[1].change("/f.bin", pattern(16 * MIB, 9));
    s[0].update("/f.bin", |f| f.rate = Some(8 * MIB as u64));
    assert_eq!(std::fs::read(finished(&finish(task).await)).unwrap(), data);
    assert_eq!(status_of(&progress, &mirror), SourceStatus::FileChanged);
}

#[tokio::test(flavor = "multi_thread")]
async fn slow_connections_hand_their_ranges_to_a_faster_source() {
    let data = pattern(16 * MIB, 10);
    // 1 MiB/s per connection, under the 2.5 MB/s at which connections write nothing in most ticks: slow sources
    // must be found by the bytes received, not written.
    let fast_server = servers(1, &data, Some(MIB as u64)).await;
    let slow_server = servers(1, &data, Some(32 * 1024)).await;
    let dir = folder("slow");
    let slow = slow_server[0].url("/f.bin");
    let started = Instant::now();
    let (_c, progress, task) = spawn(request(fast_server[0].url("/f.bin"), vec![slow.clone()], &dir, 8), fast());
    let mut saw_slow = false;
    while !task.is_finished() {
        saw_slow |= progress.sources().iter().any(|s| s.url == slow && s.status == SourceStatus::Slow);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(std::fs::read(finished(&finish(task).await)).unwrap(), data);
    // Without the handoff, half the file at 32 KiB/s per connection would take minutes.
    assert!(started.elapsed() < Duration::from_secs(20), "{:?}", started.elapsed());
    assert!(saw_slow, "the slow mirror was marked Slow");
}

#[tokio::test(flavor = "multi_thread")]
async fn turning_a_source_off_moves_its_connections() {
    let data = pattern(16 * MIB, 11);
    let s = servers(2, &data, Some(MIB as u64)).await;
    let dir = folder("off");
    let link = s[0].url("/f.bin");
    let (control, progress, task) = spawn(request(link.clone(), vec![s[1].url("/f.bin")], &dir, 8), fast());
    wait_for_bytes(&progress, 2 * MIB as u64).await;
    control.set_source_enabled(&link, false);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let before = s[0].stats("/f.bin").bytes;
    s[1].update("/f.bin", |f| f.rate = Some(8 * MIB as u64));
    assert_eq!(std::fs::read(finished(&finish(task).await)).unwrap(), data);
    // Bytes already in flight when it was turned off still arrive, but nothing new is asked of it.
    assert!(s[0].stats("/f.bin").bytes - before < MIB as u64);
    let info = progress.sources().into_iter().find(|i| i.url == link).unwrap();
    assert!(!info.enabled);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mirror_with_ranges_rescues_a_link_without() {
    let data = pattern(8 * MIB, 12);
    let plain = MockServer::start().await.unwrap();
    let mut f = MockFile::new(data.clone());
    f.ranges = false;
    plain.put("/f.bin", f);
    let s = servers(1, &data, None).await;
    let dir = folder("rescue");
    let (_c, progress, task) = spawn(request(plain.url("/f.bin"), vec![s[0].url("/f.bin")], &dir, 8), fast());
    assert_eq!(std::fs::read(finished(&finish(task).await)).unwrap(), data);
    assert!(progress.resumable.load(Ordering::SeqCst), "the mirror's ranges make it resumable");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mirror_removed_while_running_stops_serving_and_isnt_saved() {
    let data = pattern(16 * MIB, 13);
    let s = servers(2, &data, Some(MIB as u64)).await;
    let dir = folder("remove");
    let mirror = s[1].url("/f.bin");
    let (control, progress, task) = spawn(request(s[0].url("/f.bin"), vec![mirror.clone()], &dir, 8), fast());
    while s[1].stats("/f.bin").bytes < MIB as u64 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // The Download window's detail is published while it runs.
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let conns = progress.connections.lock().unwrap().clone();
        let downloading = conns.iter().filter(|c| c.state == "Downloading").count();
        if downloading >= 2 && !progress.segments.lock().unwrap().is_empty() {
            let c = conns.iter().find(|c| c.state == "Downloading").unwrap();
            assert!(c.end > c.start && c.pos >= c.start && !c.source.is_empty(), "{c:?}");
            break;
        }
        assert!(Instant::now() < until, "{conns:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // Each connection's speed is measured as bytes arrive, not only when its buffer is written.
    while !progress.connections.lock().unwrap().iter().any(|c| c.speed > 0) {
        assert!(Instant::now() < until, "no connection speed");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    control.remove_source(&mirror);
    control.remove_source(&s[0].url("/f.bin")); // the original link can't be removed
    tokio::time::sleep(Duration::from_millis(600)).await;
    let before = s[1].stats("/f.bin").bytes;
    s[0].update("/f.bin", |f| f.rate = None);
    assert_eq!(std::fs::read(finished(&finish(task).await)).unwrap(), data);
    assert!(s[1].stats("/f.bin").bytes - before < MIB as u64, "nothing new asked of the removed mirror");
    let urls: Vec<String> = progress.sources().into_iter().map(|i| i.url).collect();
    assert_eq!(urls, vec![s[0].url("/f.bin")]);
}

/// One file served badly in every way a mirror goes wrong (Herbert: "one bad mirror doesn't stop a file from
/// downloading if the other mirrors are fine"): missing, behind a login, stalling midway on every request, refusing
/// every connection, and a different file of another size.
async fn bad_mirrors(data: &[u8]) -> (Vec<String>, Vec<MockServer>) {
    let mut servers = Vec::new();
    let mut urls = Vec::new();
    let missing = MockServer::start().await.unwrap();
    urls.push(missing.url("/f.bin"));
    servers.push(missing);
    let login = MockServer::start().await.unwrap();
    let mut f = MockFile::new(data.to_vec());
    f.requires = vec![("authorization".into(), "Basic secret".into())];
    login.put("/f.bin", f);
    urls.push(login.url("/f.bin"));
    servers.push(login);
    let stalls = MockServer::start().await.unwrap();
    let mut f = MockFile::new(data.to_vec());
    f.stall_after = Some(64 * 1024);
    stalls.put("/f.bin", f);
    urls.push(stalls.url("/f.bin"));
    servers.push(stalls);
    let refuses = MockServer::start().await.unwrap();
    let mut f = MockFile::new(data.to_vec());
    f.max_connections = Some(0);
    refuses.put("/f.bin", f);
    urls.push(refuses.url("/f.bin"));
    servers.push(refuses);
    let other = MockServer::start().await.unwrap();
    other.put("/f.bin", MockFile::new(pattern(data.len() / 2, 99)));
    urls.push(other.url("/f.bin"));
    servers.push(other);
    (urls, servers)
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_mirrors_never_stop_a_file_a_good_source_serves() {
    let data = pattern(8 * MIB, 11);
    let good = servers(1, &data, None).await;
    let (bad, _keep) = bad_mirrors(&data).await;
    let dir = folder("badmirrors");
    let started = std::time::Instant::now();
    let (_c, progress, task) = spawn(request(good[0].url("/f.bin"), bad.clone(), &dir, 8), fast());
    assert_eq!(std::fs::read(finished(&finish(task).await)).unwrap(), data, "the whole file, right, from the good source");
    assert!(started.elapsed() < Duration::from_secs(20), "it didn't wait on the bad ones: {:?}", started.elapsed());
    // Each bad one is marked, none is used.
    for url in &bad[..2] {
        assert!(matches!(status_of(&progress, url), SourceStatus::Failed(_)), "{url}: {:?}", status_of(&progress, url));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bad_link_with_good_mirrors_still_downloads() {
    let data = pattern(8 * MIB, 12);
    let good = servers(2, &data, None).await;
    let (bad, _keep) = bad_mirrors(&data).await;
    // The stalling one is the link itself; the good ones and the rest of the bad ones are its mirrors.
    let mut mirrors = vec![good[0].url("/f.bin"), bad[0].clone(), bad[1].clone(), good[1].url("/f.bin"), bad[3].clone(), bad[4].clone()];
    mirrors.retain(|m| m != &bad[2]);
    let dir = folder("badlink");
    let (_c, _p, task) = spawn(request(bad[2].clone(), mirrors, &dir, 8), fast());
    assert_eq!(std::fs::read(finished(&finish(task).await)).unwrap(), data);
}

/// The link itself bad in each way, its mirrors good (Herbert: "if the main link is bad but there are good mirrors,
/// the download should ignore the bad link like with the mirrors").
async fn link_bad_in_one_way(kind: &str, data: &[u8]) -> (String, MockServer) {
    let s = MockServer::start().await.unwrap();
    match kind {
        "missing" => {}
        "login" => {
            let mut f = MockFile::new(data.to_vec());
            f.requires = vec![("authorization".into(), "Basic secret".into())];
            s.put("/f.bin", f);
        }
        "stalls" => {
            let mut f = MockFile::new(data.to_vec());
            f.stall_after = Some(64 * 1024);
            s.put("/f.bin", f);
        }
        "refuses" => {
            let mut f = MockFile::new(data.to_vec());
            f.max_connections = Some(0);
            s.put("/f.bin", f);
        }
        "other file" => s.put("/f.bin", MockFile::new(pattern(data.len() / 2, 77))),
        _ => unreachable!(),
    }
    (s.url("/f.bin"), s)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bad_link_in_every_way_is_ignored_when_its_mirrors_are_good() {
    for kind in ["missing", "login", "stalls", "refuses", "other file"] {
        let data = pattern(8 * MIB, 13);
        let good = servers(2, &data, None).await;
        let (link, _keep) = link_bad_in_one_way(kind, &data).await;
        let dir = folder(&format!("badlink-{}", kind.replace(' ', "-")));
        let (_c, _p, task) = spawn(request(link, vec![good[0].url("/f.bin"), good[1].url("/f.bin")], &dir, 8), fast());
        let outcome = tokio::time::timeout(Duration::from_secs(30), finish(task)).await.unwrap_or(Outcome::Failed("still running after 30 s".into()));
        match &outcome {
            Outcome::Finished(path) => {
                let got = std::fs::read(path).unwrap();
                assert!(got == data, "{kind}: got {} bytes, not the good mirrors' {} bytes", got.len(), data.len());
            }
            other => panic!("a link that's {kind}, with two good mirrors: {other:?}"),
        }
    }
}
