//! The scheduler (SPECIFICATION.md §11.1) on a real list: its clock is moved forward instead of waiting for it.

use std::time::{Duration, Instant};

use chrono::{Local, TimeZone, Timelike};
use hd_core::manager::{Manager, NewDownload, Status};
use hd_core::schedule::{Schedule, WhenDone};
use hd_mock::{MockFile, MockServer, pattern};

fn folder(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("hd-schedule-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("files")).unwrap();
    dir
}

/// Minutes after midnight, `ahead` minutes from now.
fn minutes_from_now(ahead: u32) -> u32 {
    let now = Local::now();
    (now.hour() * 60 + now.minute() + ahead) % (24 * 60)
}

fn status(m: &Manager, id: u64) -> Status {
    m.rows().into_iter().find(|r| r.id == id).unwrap().status
}

fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(30);
    while !ok() {
        assert!(Instant::now() < until, "{what} never happened");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn the_schedule_starts_and_stops_the_queue_and_says_when_its_done() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    let mut slow = MockFile::new(pattern(8 * 1024 * 1024, 5));
    slow.rate = Some(1024 * 1024);
    s.put("/slow.bin", slow);
    s.put("/small.bin", MockFile::new(pattern(1000, 3)));
    let dir = folder("window");
    let m = Manager::open(&dir).unwrap();
    let add = |path: &str| {
        m.add(NewDownload { url: s.url(path), folder: dir.join("files"), start: false, ..NewDownload::default() })
    };
    let slow_id = add("/slow.bin");
    assert_eq!(status(&m, slow_id), Status::Waiting);

    // Times as the scheduler's clock reads them: the start of this minute, and the minute of day `ahead` minutes on.
    // The clock is moved to exact times, so however long a step takes, it lands between the same two events.
    let base = m.scheduler_now();
    let minute = base - base.rem_euclid(60);
    let at = |ahead: i64| {
        let t = Local.timestamp_opt(minute + ahead * 60, 0).unwrap();
        t.hour() * 60 + t.minute()
    };
    let go_to = |t: i64| m.advance_clock(t - m.scheduler_now());
    let mut settings = m.settings();
    settings.schedule = Schedule { start: Some(at(2)), stop: Some(at(4)), when_done: WhenDone::Sleep, ..Schedule::default() };
    m.set_settings(settings);
    assert!(m.queue_text().unwrap().starts_with("Queue stopped, 1 waiting, starts at "), "{:?}", m.queue_text());
    go_to(minute + 90);
    assert!(!m.queue_running(), "nothing fires before the start time");

    go_to(minute + 150);
    assert!(m.queue_running(), "the start time started the queue");
    wait_until("the slow download running", || status(&m, slow_id) == Status::Downloading);
    let text = m.queue_text().unwrap();
    assert!(text.starts_with("Queue running, stops at ") && text.ends_with(", then sleeps"), "{text}");

    go_to(minute + 270);
    assert!(!m.queue_running(), "the stop time stopped the queue");
    wait_until("the slow download back in the queue", || status(&m, slow_id) == Status::Waiting);
    assert!(m.rows()[0].downloaded > 0, "what it downloaded is kept");
    assert_eq!(m.take_queue_done(), None, "a stopped queue isn't done");

    // The next day's start carries on with it; once the queue is empty, it's done, once.
    let small_id = add("/small.bin");
    go_to(minute + 24 * 3600 + 150);
    assert!(m.queue_running());
    let mut done = None;
    wait_until("the queue done", || {
        done = done.or(m.take_queue_done());
        done.is_some()
    });
    assert_eq!(done, Some(WhenDone::Sleep));
    assert_eq!((status(&m, slow_id), status(&m, small_id)), (Status::Finished, Status::Finished));
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(m.take_queue_done(), None, "it says so once");
    assert!(m.quit(Duration::from_secs(5)));
}

#[test]
fn a_start_missed_while_the_app_was_closed_fires_at_launch() {
    let dir = folder("closed");
    let m = Manager::open(&dir).unwrap();
    let mut settings = m.settings();
    // Half an hour ago: already past when it was set, so it doesn't fire now.
    settings.schedule = Schedule { start: Some(minutes_from_now(24 * 60 - 30)), ..Schedule::default() };
    m.set_settings(settings);
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!m.queue_running());
    assert!(m.quit(Duration::from_secs(5)));
    drop(m);

    // As if the app had quit an hour ago: queue.json says when the scheduler last looked.
    let path = dir.join("queue.json");
    let mut q: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    q["checked"] = serde_json::json!(q["checked"].as_i64().unwrap() - 3600);
    std::fs::write(&path, q.to_string()).unwrap();
    let back = Manager::open(&dir).unwrap();
    assert!(back.queue_running(), "the start it missed while closed started the queue at launch");
    assert!(back.quit(Duration::from_secs(5)));
}

#[test]
fn a_queue_that_finishes_within_a_second_is_still_done() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    s.put("/small.bin", MockFile::new(pattern(1000, 3)));
    let dir = folder("quick");
    let m = Manager::open(&dir).unwrap();
    let mut settings = m.settings();
    settings.schedule.when_done = WhenDone::Quit;
    m.set_settings(settings);
    m.start_queue();
    m.add(NewDownload { url: s.url("/small.bin"), folder: dir.join("files"), start: false, ..NewDownload::default() });
    wait_until("the queue done", || m.take_queue_done() == Some(WhenDone::Quit));
    assert!(m.quit(Duration::from_secs(5)));
}
