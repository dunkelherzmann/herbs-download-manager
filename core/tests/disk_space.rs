//! Names and disk space (SPECIFICATION.md §5.6, §9): a name no system takes is fitted, and a download stops before
//! its disk is full, keeping what it has, and finishes when there's room again. Its own process: the pretended
//! free space is global.

mod common;

use std::time::{Duration, Instant};

use hd_core::manager::{Manager, NewDownload, Status};
use hd_mock::{MockFile, MockServer, pattern};

fn wait(m: &Manager, id: u64, what: &str, ok: impl Fn(&Status) -> bool) -> Status {
    let until = Instant::now() + Duration::from_secs(40);
    loop {
        let s = m.rows().into_iter().find(|r| r.id == id).unwrap().status;
        if ok(&s) {
            return s;
        }
        assert!(Instant::now() < until, "never {what}: {s:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_download_stops_before_the_disk_is_full_and_names_are_fitted() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    let data = pattern(40 * 1024 * 1024, 7);
    let mut slow = MockFile::new(data.clone());
    slow.rate = Some(512 * 1024);
    // A name too long for any system, with characters some refuse.
    let long = format!("{}:part?.bin", "x".repeat(300));
    slow.headers = vec![format!("Content-Disposition: attachment; filename=\"{long}\"")];
    s.put("/f.bin", slow);
    let dir = common::Scratch::new("disk-space");
    std::fs::create_dir_all(dir.join("files")).unwrap();
    let m = Manager::open(&dir).unwrap();
    let id = m.add(NewDownload { url: s.url("/f.bin"), folder: dir.join("files"), connections: 4, start: true, ..Default::default() });
    wait(&m, id, "downloading", |s| *s == Status::Downloading);
    // The disk fills up meanwhile: within a few seconds the download stops, saying why.
    hd_core::disk::pretend_free_space(Some(50 * 1024 * 1024));
    let stopped = wait(&m, id, "stopped", |s| matches!(s, Status::Failed(_)));
    assert!(matches!(&stopped, Status::Failed(r) if r.starts_with("The disk is nearly full")), "{stopped:?}");
    let kept = m.rows()[0].downloaded;
    assert!(kept > 0, "what was downloaded is kept");
    // Room again: Resume finishes it, from where it stopped.
    hd_core::disk::pretend_free_space(None);
    s.update("/f.bin", |f| f.rate = None);
    m.resume(&[id]);
    wait(&m, id, "finished", |s| *s == Status::Finished);
    let row = m.rows().remove(0);
    let path = row.path.unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), data);
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.len() <= 230 && name.ends_with(".bin") && !name.contains(':') && !name.contains('?'), "{name}");
    // A stream on a nearly full disk stops before its first piece, saying why.
    let media = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/media/h264-ts");
    for entry in std::fs::read_dir(&media).unwrap() {
        let path = entry.unwrap().path();
        s.put(&format!("/h264-ts/{}", path.file_name().unwrap().to_string_lossy()), MockFile::new(std::fs::read(&path).unwrap()));
    }
    hd_core::disk::pretend_free_space(Some(50 * 1024 * 1024));
    let id = m.add(NewDownload { url: s.url("/h264-ts/index.m3u8"), folder: dir.join("files"), connections: 4, start: true, ..Default::default() });
    let stopped = wait(&m, id, "stopped", |s| matches!(s, Status::Failed(_)));
    assert!(matches!(&stopped, Status::Failed(r) if r.contains("The disk is nearly full")), "{stopped:?}");
    hd_core::disk::pretend_free_space(None);
    m.resume(&[id]);
    wait(&m, id, "finished", |s| *s == Status::Finished);
    assert!(m.quit(Duration::from_secs(5)));
}
