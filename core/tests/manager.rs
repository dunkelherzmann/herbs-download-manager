//! The download list (SPECIFICATION.md §2.2, §10) against the local test server.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use hd_core::manager::{Manager, NewDownload, Status};
use hd_mock::{MockFile, MockServer, pattern};

const MIB: usize = 1024 * 1024;

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hd-manager-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("files")).unwrap();
    dir
}

/// A test server on its own runtime, so the manager's runtime is never dropped inside another one.
fn server(data: &[u8], rate: Option<u64>) -> (tokio::runtime::Runtime, MockServer) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    let mut f = MockFile::new(data.to_vec());
    f.rate = rate;
    s.put("/f.bin", f);
    (rt, s)
}

fn new(url: String, dir: &std::path::Path, start: bool) -> NewDownload {
    NewDownload { url, folder: dir.join("files"), connections: 4, connections_per_server: 32, retries: 2, start, ..Default::default() }
}

fn wait(m: &Manager, id: u64, what: impl Fn(&Status) -> bool) -> Status {
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        let status = m.rows().into_iter().find(|r| r.id == id).unwrap().status;
        if what(&status) {
            return status;
        }
        assert!(Instant::now() < until, "still {status:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_download_runs_to_the_end_and_is_remembered() {
    let data = pattern(3 * MIB, 1);
    let (_rt, s) = server(&data, None);
    let dir = folder("finish");
    let m = Manager::open(&dir).unwrap();
    let id = m.add(new(s.url("/f.bin"), &dir, true));
    wait(&m, id, |s| *s == Status::Finished);
    let row = m.rows().remove(0);
    assert_eq!(row.name, "f.bin");
    assert_eq!(row.status_text, "Finished");
    assert_eq!(row.size, Some(data.len() as u64));
    assert!(row.finished.is_some());
    assert_eq!(std::fs::read(row.path.unwrap()).unwrap(), data);
    drop(m);
    let again = Manager::open(&dir).unwrap();
    assert_eq!(again.rows().len(), 1);
    assert_eq!(again.rows()[0].status, Status::Finished);
}

#[test]
fn a_download_added_without_starting_waits() {
    let dir = folder("waits");
    let m = Manager::open(&dir).unwrap();
    let id = m.add(new("http://127.0.0.1:9/never.zip".into(), &dir, false));
    let row = m.rows().remove(0);
    assert_eq!((row.id, row.status, row.name.as_str()), (id, Status::Waiting, "never.zip"));
    assert_eq!(row.sources_known, 1);
}

#[test]
fn pause_keeps_the_bytes_and_resume_finishes() {
    let data = pattern(8 * MIB, 2);
    let (_rt, s) = server(&data, Some(MIB as u64));
    let dir = folder("pause");
    let m = Manager::open(&dir).unwrap();
    let id = m.add(new(s.url("/f.bin"), &dir, true));
    wait(&m, id, |s| *s == Status::Downloading);
    let row = m.rows().remove(0);
    assert!(row.status_text.starts_with("Downloading "), "{}", row.status_text);
    assert_eq!(row.allowed_connections, 4);
    std::thread::sleep(Duration::from_millis(600));
    m.pause(&[id]);
    wait(&m, id, |s| *s == Status::Paused);
    let paused = m.rows().remove(0);
    assert!(paused.downloaded > 0 && paused.status_text.starts_with("Paused "));
    s.update("/f.bin", |f| f.rate = None);
    m.resume(&[id]);
    wait(&m, id, |s| *s == Status::Finished);
    assert_eq!(std::fs::read(m.rows()[0].path.clone().unwrap()).unwrap(), data);
}

#[test]
fn downloads_running_at_quit_come_back_paused() {
    let data = pattern(8 * MIB, 3);
    let (_rt, s) = server(&data, Some(MIB as u64));
    let dir = folder("quit");
    let m = Manager::open(&dir).unwrap();
    let id = m.add(new(s.url("/f.bin"), &dir, true));
    wait(&m, id, |s| *s == Status::Downloading);
    std::thread::sleep(Duration::from_millis(300));
    // What a crash leaves: the list saved as running.
    let text = std::fs::read_to_string(dir.join("downloads.json")).unwrap().replace("\"Connecting\"", "\"Downloading\"");
    m.pause_all();
    assert!(m.wait_idle(Duration::from_secs(10)));
    drop(m);
    std::fs::write(dir.join("downloads.json"), text).unwrap();
    let again = Manager::open(&dir).unwrap();
    assert_eq!(again.rows()[0].status, Status::Paused);
    s.update("/f.bin", |f| f.rate = None);
    again.resume(&[id]);
    wait(&again, id, |s| *s == Status::Finished);
    assert_eq!(std::fs::read(again.rows()[0].path.clone().unwrap()).unwrap(), data);
}

#[test]
fn delete_removes_the_entry_and_optionally_the_file() {
    let data = pattern(MIB, 4);
    let (_rt, s) = server(&data, None);
    let dir = folder("delete");
    let m = Manager::open(&dir).unwrap();
    let a = m.add(new(s.url("/f.bin"), &dir, true));
    wait(&m, a, |s| *s == Status::Finished);
    let b = m.add(new(s.url("/f.bin"), &dir, true));
    wait(&m, b, |s| *s == Status::Finished);
    let rows = m.rows();
    assert_eq!(rows[0].id, b, "newest first");
    let (pa, pb) = (rows[1].path.clone().unwrap(), rows[0].path.clone().unwrap());
    assert_ne!(pa, pb);
    m.remove(&[a], false);
    assert!(pa.exists());
    m.remove(&[b], true);
    assert!(!pb.exists());
    assert!(m.rows().is_empty());
}

#[test]
fn restart_discards_the_data_and_downloads_again() {
    let data = pattern(4 * MIB, 5);
    let (_rt, s) = server(&data, None);
    let dir = folder("restart");
    let m = Manager::open(&dir).unwrap();
    let id = m.add(new(s.url("/f.bin"), &dir, true));
    wait(&m, id, |s| *s == Status::Finished);
    let first = m.rows()[0].path.clone().unwrap();
    let before = s.stats("/f.bin").bytes;
    m.restart(&[id]);
    wait(&m, id, |s| *s == Status::Finished);
    let row = m.rows().remove(0);
    assert_eq!(row.path.as_ref(), Some(&first), "the same file name, not a numbered copy");
    assert_eq!(std::fs::read(&first).unwrap(), data);
    assert!(s.stats("/f.bin").bytes - before >= data.len() as u64, "downloaded again");
    // While running: it stops, discards and starts over.
    s.update("/f.bin", |f| f.rate = Some(MIB as u64));
    m.restart(&[id]);
    wait(&m, id, |s| *s == Status::Downloading);
    std::thread::sleep(Duration::from_millis(500));
    m.restart(&[id]);
    std::thread::sleep(Duration::from_millis(300));
    s.update("/f.bin", |f| f.rate = None);
    wait(&m, id, |s| *s == Status::Finished);
    assert_eq!(std::fs::read(m.rows()[0].path.clone().unwrap()).unwrap(), data);
    assert_eq!(std::fs::read_dir(dir.join("files")).unwrap().count(), 1, "no leftovers");
}

#[test]
fn connections_change_while_running_and_are_remembered() {
    let data = pattern(16 * MIB, 6);
    let (_rt, s) = server(&data, Some(MIB as u64));
    let dir = folder("connections");
    let m = Manager::open(&dir).unwrap();
    let id = m.add(new(s.url("/f.bin"), &dir, true));
    let until = Instant::now() + Duration::from_secs(20);
    while m.rows()[0].active_connections < 4 {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(20));
    }
    m.set_connections(id, 7);
    while m.rows()[0].active_connections < 7 {
        assert!(Instant::now() < until, "{}", m.rows()[0].active_connections);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(m.rows()[0].allowed_connections, 7);
    // The sources list is refreshed on the engine's 1-second tick.
    while !m.rows()[0].live.first().is_some_and(|l| l.ends_with("— 7 connections")) {
        assert!(Instant::now() < until, "{:?}", m.rows()[0].live);
        std::thread::sleep(Duration::from_millis(50));
    }
    m.pause(&[id]);
    wait(&m, id, |s| *s == Status::Paused);
    drop(m);
    let again = Manager::open(&dir).unwrap();
    assert_eq!(again.rows()[0].allowed_connections, 7);
}
