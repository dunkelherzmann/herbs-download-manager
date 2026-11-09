//! The download list (SPECIFICATION.md §2.2, §10) against the local test server.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use hd_core::manager::{Added, Clash, Manager, NewDownload, Plan, Status};
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
    assert!(row.resumable);
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
    assert_eq!(again.rows()[0].size, Some(data.len() as u64), "numbers come from the saved state");
    assert!(again.rows()[0].downloaded > 0);
    s.update("/f.bin", |f| f.rate = None);
    again.resume(&[id]);
    wait(&again, id, |s| *s == Status::Finished);
    assert_eq!(std::fs::read(again.rows()[0].path.clone().unwrap()).unwrap(), data);
}

#[test]
fn delete_removes_the_entry_and_optionally_sends_the_file_to_the_trash() {
    let data = pattern(MIB, 4);
    let (_rt, s) = server(&data, None);
    let dir = folder("delete");
    let m = Manager::open(&dir).unwrap();
    m.use_trash_folder(&dir.join("Trash"));
    let a = m.add(new(s.url("/f.bin"), &dir, true));
    wait(&m, a, |s| *s == Status::Finished);
    let b = m.add(new(s.url("/f.bin"), &dir, true));
    wait(&m, b, |s| *s == Status::Finished);
    let rows = m.rows();
    assert_eq!(rows[0].id, b, "newest first");
    let (pa, pb) = (rows[1].path.clone().unwrap(), rows[0].path.clone().unwrap());
    assert_ne!(pa, pb);
    assert!(m.remove(&[a], false).is_empty());
    assert!(pa.exists());
    // Deleting the file sends it to the Trash: gone from the folder, whole in the Trash.
    assert!(m.remove(&[b], true).is_empty());
    assert!(!pb.exists());
    assert_eq!(std::fs::read(dir.join("Trash").join(pb.file_name().unwrap())).unwrap(), data);
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

#[test]
fn quitting_pauses_and_the_next_launch_continues() {
    let data = pattern(8 * MIB, 7);
    let (_rt, s) = server(&data, Some(MIB as u64));
    let dir = folder("quit-resume");
    let m = Manager::open(&dir).unwrap();
    let running = m.add(new(s.url("/f.bin"), &dir, true));
    let waiting = m.add(new(s.url("/f.bin"), &dir, false));
    wait(&m, running, |s| *s == Status::Downloading);
    std::thread::sleep(Duration::from_millis(300));
    assert!(m.quit(Duration::from_secs(10)));
    assert_eq!(m.rows().iter().find(|r| r.id == running).unwrap().status, Status::Paused);
    drop(m);
    s.update("/f.bin", |f| f.rate = None);
    let again = Manager::open(&dir).unwrap();
    wait(&again, running, |s| *s == Status::Finished);
    assert_eq!(again.rows().iter().find(|r| r.id == waiting).unwrap().status, Status::Waiting, "only what was running");
    drop(again);
    // Started once: the next launch doesn't start it again.
    let third = Manager::open(&dir).unwrap();
    assert!(!third.any_running());
}

#[test]
fn a_download_without_ranges_says_it_cant_resume() {
    let data = pattern(8 * MIB, 8);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    let mut f = MockFile::new(data);
    f.ranges = false;
    f.rate = Some(MIB as u64);
    s.put("/f.bin", f);
    let dir = folder("no-ranges");
    let m = Manager::open(&dir).unwrap();
    let id = m.add(new(s.url("/f.bin"), &dir, true));
    wait(&m, id, |s| *s == Status::Downloading);
    assert!(!m.rows()[0].resumable);
    m.pause(&[id]);
    wait(&m, id, |s| *s == Status::Paused);
}

#[test]
fn probing_a_link_gives_its_name_size_and_resume_support() {
    let data = pattern(3 * MIB, 9);
    let (_rt, s) = server(&data, None);
    let mut plain = MockFile::new(data.clone());
    plain.ranges = false;
    s.put("/plain.iso", plain);
    let dir = folder("probe");
    let m = Manager::open(&dir).unwrap();
    let p = m.probe(&s.url("/f.bin")).unwrap();
    assert_eq!((p.name.as_str(), p.size, p.resumable), ("f.bin", Some(data.len() as u64), true));
    let p = m.probe(&s.url("/plain.iso")).unwrap();
    assert_eq!((p.name.as_str(), p.size, p.resumable), ("plain.iso", Some(data.len() as u64), false));
    assert_eq!(m.probe(&s.url("/missing")).unwrap_err(), "404 Not Found");
    assert_eq!(m.probe("not a link").unwrap_err(), "That isn't a web link");
    std::fs::write(dir.join("files/f.bin"), b"x").unwrap();
    assert_eq!(hd_core::manager::free_name(&dir.join("files"), "f.bin"), "f (2).bin");
}

#[test]
fn mirrors_are_added_and_removed_before_during_and_after() {
    let data = pattern(16 * MIB, 10);
    let (_rt, a) = server(&data, Some(MIB as u64));
    let (_rt2, b) = server(&data, Some(MIB as u64));
    let (_rt3, c) = server(&data, None);
    let dir = folder("mirrors");
    let m = Manager::open(&dir).unwrap();
    let id = m.add(new(a.url("/f.bin"), &dir, true));
    wait(&m, id, |s| *s == Status::Downloading);
    // M2: added while running, used without a pause; a repeat of the link is ignored.
    assert_eq!(m.add_mirrors(id, &[b.url("/f.bin"), a.url("/f.bin")]), 1);
    let until = Instant::now() + Duration::from_secs(20);
    while b.stats("/f.bin").bytes < MIB as u64 {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(20));
    }
    let d = m.detail(id).unwrap();
    assert_eq!(d.sources.len(), 2);
    assert!(!d.segments.is_empty());
    m.pause(&[id]);
    wait(&m, id, |s| *s == Status::Paused);
    // Stopped: the saved state gives the detail; M3 adds c; Remove drops b; the original stays.
    let d = m.detail(id).unwrap();
    assert!(d.connections.is_empty() && !d.segments.is_empty());
    assert_eq!(m.add_mirrors(id, &[c.url("/f.bin")]), 1);
    m.remove_source(id, &b.url("/f.bin"));
    m.remove_source(id, &a.url("/f.bin"));
    m.set_source_enabled(id, &a.url("/f.bin"), false);
    let urls: Vec<(String, bool)> = m.detail(id).unwrap().sources.into_iter().map(|s| (s.url, s.enabled)).collect();
    assert_eq!(urls, vec![(a.url("/f.bin"), false), (c.url("/f.bin"), true)]);
    // Bytes already in flight at the pause may still be counted; new requests would not be.
    std::thread::sleep(Duration::from_millis(300));
    let before = b.stats("/f.bin").requests;
    m.resume(&[id]);
    wait(&m, id, |s| *s == Status::Finished);
    assert_eq!(std::fs::read(m.rows()[0].path.clone().unwrap()).unwrap(), data);
    assert_eq!(b.stats("/f.bin").requests, before, "the removed mirror isn't asked again");
    let finished = m.detail(id).unwrap();
    assert_eq!(finished.segments.len(), 1);
}

fn running(m: &Manager) -> usize {
    m.rows().iter().filter(|r| r.status.is_running()).count()
}

#[test]
fn the_queue_runs_waiting_downloads_a_few_at_a_time_oldest_first() {
    let data = pattern(2 * MIB, 7);
    let (_rt, s) = server(&data, Some(4 * MIB as u64));
    let dir = folder("queue");
    let m = Manager::open(&dir).unwrap();
    m.set_downloads_at_once(2);
    let ids: Vec<u64> = (0..4).map(|_| m.add(new(s.url("/f.bin"), &dir, false))).collect();
    assert_eq!(running(&m), 0, "nothing starts while the queue is stopped");
    m.start_queue();
    assert!(m.queue_running());
    let rows = m.rows();
    let started: Vec<u64> = rows.iter().filter(|r| r.status.is_running()).map(|r| r.id).collect();
    assert_eq!(started.len(), 2);
    assert!(started.contains(&ids[0]) && started.contains(&ids[1]), "oldest first: {started:?}");
    let until = Instant::now() + Duration::from_secs(30);
    while m.rows().iter().any(|r| r.status != Status::Finished) {
        assert!(running(&m) <= 2, "never more than downloads at once");
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
    // A download added while the queue runs starts by itself.
    let late = m.add(new(s.url("/f.bin"), &dir, false));
    wait(&m, late, |s| *s == Status::Finished);
}

#[test]
fn stopping_the_queue_lets_running_downloads_finish_and_starts_no_more() {
    let data = pattern(2 * MIB, 8);
    let (_rt, s) = server(&data, Some(2 * MIB as u64));
    let dir = folder("queue-stop");
    let m = Manager::open(&dir).unwrap();
    m.set_downloads_at_once(1);
    let a = m.add(new(s.url("/f.bin"), &dir, false));
    let b = m.add(new(s.url("/f.bin"), &dir, false));
    m.start_queue();
    m.stop_queue();
    wait(&m, a, |s| *s == Status::Finished);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(m.rows().into_iter().find(|r| r.id == b).unwrap().status, Status::Waiting);
    // Download/Resume ignores the queue.
    m.resume(&[b]);
    wait(&m, b, |s| *s == Status::Finished);
}

#[test]
fn stop_all_stops_the_queue_and_a_running_queue_is_remembered() {
    let data = pattern(4 * MIB, 9);
    let (_rt, s) = server(&data, Some(MIB as u64));
    let dir = folder("queue-launch");
    {
        let m = Manager::open(&dir).unwrap();
        m.set_downloads_at_once(1);
        m.add(new(s.url("/f.bin"), &dir, false));
        m.add(new(s.url("/f.bin"), &dir, false));
        m.start_queue();
        assert_eq!(running(&m), 1);
        assert!(m.quit(Duration::from_secs(10)));
        assert_eq!(running(&m), 0, "quitting starts nothing new");
    }
    let m = Manager::open(&dir).unwrap();
    assert!(m.queue_running(), "the queue runs again after launch");
    assert_eq!(running(&m), 1);
    m.pause_all();
    assert!(m.wait_idle(Duration::from_secs(10)));
    assert!(!m.queue_running());
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(running(&m), 0, "Stop All starts nothing from the queue");
}

#[test]
fn a_link_to_a_file_already_in_the_list_becomes_its_mirror() {
    let data = pattern(4 * MIB, 11);
    let (_rt, a) = server(&data, None);
    let (_rt2, b) = server(&data, None);
    let dir = folder("match");
    let m = Manager::open(&dir).unwrap();
    let id = m.add(new(a.url("/f.bin"), &dir, false));

    // Same file name on another server: a mirror, not a second download.
    assert_eq!(m.add_or_mirror(new(b.url("/f.bin"), &dir, false)), Added::MirrorOf(id));
    let rows = m.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].sources_known, 2);

    // Another name, or the same name with another size, is a download of its own.
    assert!(matches!(m.add_or_mirror(new(b.url("/other.bin"), &dir, false)), Added::New(_)));
    m.resume(&[id]);
    wait(&m, id, |s| *s == Status::Finished);
    let sized = NewDownload { size: Some(1), ..new(b.url("/x/f.bin"), &dir, false) };
    assert!(matches!(m.add_or_mirror(sized), Added::New(_)), "a finished download takes no mirrors");
    let late = m.rows().into_iter().find(|r| r.url.ends_with("/x/f.bin")).unwrap().id;
    let other_size = NewDownload { size: Some(2), ..new(a.url("/y/f.bin"), &dir, false) };
    assert!(matches!(m.add_or_mirror(other_size), Added::New(_)), "sizes differ");
    let unknown = NewDownload { size: None, ..new(a.url("/z/f.bin"), &dir, false) };
    assert_eq!(m.add_or_mirror(unknown), Added::MirrorOf(late), "an unknown size matches by name, oldest first");
}

#[test]
fn a_different_file_with_the_same_name_is_a_clash_with_a_kept_name() {
    let data = pattern(MIB, 12);
    let (_rt, s) = server(&data, None);
    let dir = folder("clash");
    let files = dir.join("files");
    let m = Manager::open(&dir).unwrap();
    assert_eq!(m.plan_add(&s.url("/f.bin"), None, &files, None), Plan::New);

    let id = m.add(new(s.url("/f.bin"), &dir, true));
    wait(&m, id, |s| *s == Status::Finished);
    // The same link again, or the same name from elsewhere: a finished download is never added to, so it clashes.
    let Plan::Clash(c) = m.plan_add("http://other.example/f.bin", None, &files, Some(5)) else { panic!() };
    assert_eq!(
        c,
        Clash { id: Some(id), path: Some(files.join("f.bin")), size: Some(MIB as u64), finished: true, kept_name: "f - kept.bin".into() }
    );

    // A file already in the folder clashes too, and kept names count up past the ones in use.
    std::fs::write(files.join("notes.txt"), b"hi").unwrap();
    std::fs::write(files.join("notes - kept.txt"), b"hi").unwrap();
    let Plan::Clash(c) = m.plan_add("http://a.example/notes.txt", None, &files, None) else { panic!() };
    assert_eq!((c.id, c.size, c.kept_name.as_str()), (None, Some(2), "notes - kept 2.txt"));
    // A name without an extension.
    std::fs::write(files.join("README"), b"x").unwrap();
    let Plan::Clash(c) = m.plan_add("http://a.example/README", None, &files, None) else { panic!() };
    assert_eq!(c.kept_name, "README - kept");

    // Keep Both: added under the kept name, which the next clash then skips.
    let kept = m.add(NewDownload { name: Some("f - kept.bin".into()), ..new("http://127.0.0.1:9/f.bin".into(), &dir, false) });
    // A link the kept download already uses (as its link or a mirror) is that download, whatever its name.
    m.add_mirrors(kept, &["http://c.example/f.bin".to_string()]);
    assert_eq!(m.plan_add("http://c.example/f.bin", None, &files, None), Plan::Mirror(kept));
    let Plan::Clash(c) = m.plan_add("http://b.example/f.bin", None, &files, Some(7)) else { panic!() };
    assert_eq!((c.id, c.kept_name.as_str()), (Some(id), "f - kept 2.bin"));
}

#[test]
fn settings_are_saved_and_an_old_queue_limit_carries_over() {
    use hd_core::settings::{Settings, WhenFileExists};
    let dir = folder("settings");
    // Before settings.json, downloads at once lived in queue.json.
    std::fs::write(
        dir.join("queue.json"),
        r#"{"running": false, "at_once": 7}"#,
    )
    .unwrap();
    let m = Manager::open(&dir).unwrap();
    assert_eq!(m.settings().downloads_at_once, 7);
    let s = Settings {
        save_to: Some("/tmp/elsewhere".into()),
        connections: 8,
        retries: 0,
        speed_limit: Some(2 * MIB as u64),
        when_file_exists: WhenFileExists::Replace,
        notify_when_finished: false,
        ..m.settings()
    };
    m.set_settings(s.clone());
    drop(m);
    let again = Manager::open(&dir).unwrap();
    assert_eq!(again.settings(), s);
    again.set_downloads_at_once(99);
    assert_eq!(again.settings().downloads_at_once, 10);
}

/// 512 KiB at a limit of 256 KiB/s takes about two seconds, however many connections share it.
fn timed_download(
    dir: &std::path::Path,
    url: String,
    limit_all: Option<u64>,
    limit_one: Option<u64>,
) -> Duration {
    let m = Manager::open(dir).unwrap();
    m.set_settings(hd_core::settings::Settings {
        speed_limit: limit_all,
        ..m.settings()
    });
    let id = m.add(new(url, dir, false));
    m.set_speed_limit(id, limit_one);
    let t = Instant::now();
    m.resume(&[id]);
    wait(&m, id, |s| *s == Status::Finished);
    t.elapsed()
}

#[test]
fn the_speed_limit_for_all_downloads_holds() {
    let data = pattern(MIB / 2, 21);
    let (_rt, s) = server(&data, None);
    let took = timed_download(
        &folder("limit-all"),
        s.url("/f.bin"),
        Some(256 * 1024),
        None,
    );
    assert!(
        took >= Duration::from_millis(1500) && took < Duration::from_secs(6),
        "took {took:?}"
    );
}

#[test]
fn one_download_s_own_limit_holds_and_can_be_lifted() {
    let data = pattern(MIB / 2, 22);
    let (_rt, s) = server(&data, None);
    let took = timed_download(
        &folder("limit-one"),
        s.url("/f.bin"),
        None,
        Some(256 * 1024),
    );
    assert!(
        took >= Duration::from_millis(1500) && took < Duration::from_secs(6),
        "took {took:?}"
    );
    let took = timed_download(&folder("limit-none"), s.url("/f.bin"), None, None);
    assert!(
        took < Duration::from_millis(1500),
        "took {took:?} without a limit"
    );
}

#[test]
fn an_add_session_gathers_mirrors_and_asks_about_a_name_in_use() {
    use hd_core::addflow::{AddSession, Choice, PendingLink, Step};
    let first = pattern(MIB, 31);
    let other = pattern(2 * MIB, 32);
    let (_rt1, s1) = server(&first, None);
    let (_rt2, s2) = server(&other, None);
    let (_rt3, s3) = server(&other, None);
    let dir = folder("addflow");
    let m = Manager::open(&dir).unwrap();
    let id = m.add(new(s1.url("/f.bin"), &dir, true));
    wait(&m, id, |s| *s == Status::Finished);
    let link = |url: String, size: u64| PendingLink { url, name: Some("f.bin".into()), size: Some(size), ..Default::default() };
    // A different f.bin (other size) from two servers: one question, then its second link follows the answer.
    let links = vec![link(s2.url("/f.bin"), other.len() as u64), link(s3.url("/f.bin"), other.len() as u64)];
    let mut session = AddSession::new(&m, links, &dir.join("files"), 4, false);
    let Step::Ask(question) = session.step(&m) else { panic!("expected a question") };
    assert_eq!(question.clash.kept_name, "f - kept.bin");
    assert!(question.more);
    assert!(question.title().contains("A different “f.bin” is already in the list"), "{}", question.title());
    session.answer(&m, Choice::KeepBoth, false);
    let Step::Done(summary) = session.step(&m) else { panic!("expected the end") };
    assert_eq!((summary.new, summary.kept, summary.mirrored), (1, 1, 1));
    assert_eq!(summary.text().as_deref(), Some("1 download added, 1 link added as a mirror, 1 kept under a new name"));
    let rows = m.rows();
    let kept = rows.iter().find(|r| r.name == "f - kept.bin").expect("kept under a new name");
    assert_eq!(kept.sources_known, 2, "the second link became its mirror");

    // Skip, for the rest: nothing more is added.
    let links = vec![link(s2.url("/f.bin?again"), 5), link(s3.url("/f.bin?again"), 6)];
    let mut session = AddSession::new(&m, links, &dir.join("files"), 4, false);
    let Step::Ask(_) = session.step(&m) else { panic!("expected a question") };
    session.answer(&m, Choice::Skip, true);
    let Step::Done(summary) = session.step(&m) else { panic!("expected the end") };
    assert_eq!((summary.new, summary.skipped), (0, 2));
    assert_eq!(m.rows().len(), 2);
}

#[test]
fn link_files_are_read_as_text_and_binary_files_are_refused() {
    let dir = folder("linkfiles");
    let utf16: Vec<u8> = [0xFF, 0xFE].into_iter().chain("see https://example.com/a.zip".encode_utf16().flat_map(|u| u.to_le_bytes())).collect();
    std::fs::write(dir.join("links.txt"), &utf16).unwrap();
    std::fs::write(dir.join("photo.png"), [0x89, b'P', b'N', b'G', 0, 0, 0]).unwrap();
    let text = hd_core::links::text_of_file(&dir.join("links.txt")).unwrap();
    assert_eq!(hd_core::links::find(&text), vec!["https://example.com/a.zip".to_string()]);
    assert_eq!(hd_core::links::text_of_file(&dir.join("photo.png")), None);
}

fn sha256_of(data: &[u8], dir: &std::path::Path) -> String {
    let path = dir.join("expected.bin");
    std::fs::write(&path, data).unwrap();
    hd_core::checksum::hash_file(&path, hd_core::checksum::Algorithm::Sha256).unwrap()
}

#[test]
fn a_checksum_is_verified_before_the_file_gets_its_name() {
    use hd_core::checksum::{Algorithm, Checksum};
    let data = pattern(MIB, 41);
    let (_rt, s) = server(&data, None);
    let dir = folder("checksum");
    let good = sha256_of(&data, &dir);
    let m = Manager::open(&dir).unwrap();
    // Right: finished, and the note says so.
    let id = m.add(new(s.url("/f.bin"), &dir, false));
    m.set_checksum(id, Some(Checksum::new(Algorithm::Sha256, &good.to_uppercase()).unwrap()));
    m.resume(&[id]);
    wait(&m, id, |s| *s == Status::Finished);
    assert_eq!(m.detail(id).unwrap().verified.as_deref(), Some("SHA-256 matches"));
    // Wrong: failed, with the data kept for Restart and no file under the final name.
    let wrong = Checksum::new(Algorithm::Sha256, &"0".repeat(64)).unwrap();
    s.put("/f2.bin", MockFile::new(data.clone()));
    let id = m.add(new(s.url("/f2.bin"), &dir, false));
    m.set_checksum(id, Some(wrong.clone()));
    m.resume(&[id]);
    let status = wait(&m, id, |s| matches!(s, Status::Failed(_)));
    assert!(matches!(&status, Status::Failed(r) if r.contains("SHA-256 checksum doesn't match")), "{status:?}");
    // Set on a finished file: checked at once.
    let first = m.rows().into_iter().find(|r| r.status == Status::Finished).unwrap().id;
    m.set_checksum(first, Some(wrong));
    let until = Instant::now() + Duration::from_secs(10);
    while m.detail(first).unwrap().verified.is_none() {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(m.detail(first).unwrap().verified.unwrap().contains("doesn't match"));
}

#[test]
fn a_checksum_file_next_to_the_link_is_found_and_used() {
    let data = pattern(MIB, 42);
    let (_rt, s) = server(&data, None);
    let dir = folder("checksum-file");
    let good = sha256_of(&data, &dir);
    s.put("/f.bin.sha256", MockFile::new(format!("{good}  f.bin\n").into_bytes()));
    s.put("/g.bin", MockFile::new(data.clone()));
    s.put("/g.bin.sha256", MockFile::new(format!("{}  g.bin\n", "1".repeat(64)).into_bytes()));
    let m = Manager::open(&dir).unwrap();
    let id = m.add(new(s.url("/f.bin"), &dir, true));
    wait(&m, id, |s| *s == Status::Finished);
    assert_eq!(m.detail(id).unwrap().verified.as_deref(), Some("SHA-256 matches (f.bin.sha256)"));
    let id = m.add(new(s.url("/g.bin"), &dir, true));
    let status = wait(&m, id, |s| matches!(s, Status::Failed(_)));
    assert!(matches!(&status, Status::Failed(r) if r.contains("(g.bin.sha256)")), "{status:?}");
}

#[test]
fn the_feature_scenario_passes_against_the_core() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let servers = rt.block_on(hd_mock::feature_servers(None)).unwrap();
    let dir = folder("scenario");
    let m = Manager::open(&dir).unwrap();
    m.use_trash_folder(&dir.join("Trash"));
    let base = |i: usize| servers[i].url("").trim_end_matches('/').to_string();
    let vars: Vec<(String, String)> = vec![
        ("L".into(), base(0)),
        ("M1".into(), base(1)),
        ("M2".into(), base(2)),
        ("DL".into(), dir.join("files").display().to_string()),
        ("SMALL_SHA256".into(), hd_mock::feature_small_sha256()),
    ];
    let script = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/features.scenario")).unwrap();
    let results = hd_core::scenario::run(&m, &hd_core::scenario::DirectHost(m.clone()), &script, &vars);
    let report = hd_core::scenario::report(&results);
    assert!(results.len() >= 11 && results.iter().all(|r| r.passed), "{report}");
}
