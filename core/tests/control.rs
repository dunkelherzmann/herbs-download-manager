//! The command line and MCP server (SPECIFICATION.md §12) against a list with its control server running.

mod common;

use std::time::{Duration, Instant};

use hd_core::cli;
use hd_core::control::{self, Request};
use hd_core::manager::Manager;
use hd_mock::{MockFile, MockServer, pattern};
use serde_json::{Value, json};

/// The test's own folder (with `files` in it for downloads), removed when the test ends.
fn folder(name: &str) -> common::Scratch {
    let dir = common::Scratch::new(&format!("control-{name}"));
    std::fs::create_dir_all(dir.join("files")).unwrap();
    dir
}

fn ask(dir: &std::path::Path, args: &[&str]) -> control::Answer {
    let (request, _) = cli::parse(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>()).unwrap();
    control::send(dir, &request).unwrap()
}

fn state_of(dir: &std::path::Path, id: u64) -> String {
    ask(dir, &["list"]).downloads.unwrap().into_iter().find(|d| d.id == id).map(|d| d.state).unwrap_or_default()
}

fn wait_for(dir: &std::path::Path, id: u64, state: &str) {
    let until = Instant::now() + Duration::from_secs(30);
    while state_of(dir, id) != state {
        assert!(Instant::now() < until, "download {id} never became {state}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn the_command_line_adds_lists_notes_pauses_resumes_and_removes() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    let data = pattern(4 * 1024 * 1024, 7);
    let mut slow = MockFile::new(data.clone());
    // Per response: 32 connections at 32 KiB/s take about 4 s, long enough to see it downloading and pause it
    // (at 2 MiB/s a fast machine finished it between two looks).
    slow.rate = Some(32 * 1024);
    s.put("/slow.bin", slow);
    s.put("/small.bin", MockFile::new(pattern(1000, 3)));
    let dir = folder("cli");
    let m = Manager::open(&dir).unwrap();
    m.use_trash_folder(&dir.join("Trash"));
    assert_eq!(control::send(&dir, &Request { command: "list".into(), ..Default::default() }).unwrap_err(), control::NOT_RUNNING);
    control::serve(&m, dir.join("files")).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join(control::FILE)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "only the user can read the token");
    }

    let small = s.url("/small.bin");
    let added = ask(&dir, &["add", &small]);
    assert!(added.ok && added.message.as_deref().unwrap().contains("saving to"), "{added:?}");
    let list = ask(&dir, &["list"]).downloads.unwrap();
    assert_eq!(list.len(), 1);
    let id = list[0].id;
    wait_for(&dir, id, "finished");
    assert_eq!(std::fs::read(dir.join("files/small.bin")).unwrap().len(), 1000);

    assert!(ask(&dir, &["note", &id.to_string(), "for", "the", "laptop"]).ok);
    let found = ask(&dir, &["list", "--search", "LAPTOP"]).downloads.unwrap();
    assert_eq!(found.iter().map(|d| d.note.as_str()).collect::<Vec<_>>(), ["for the laptop"]);
    assert!(ask(&dir, &["list", "--filter", "unfinished"]).downloads.unwrap().is_empty());
    assert!(!ask(&dir, &["list", "--filter", "nonsense"]).ok);

    let other = dir.join("elsewhere");
    let queued = ask(&dir, &["add", &s.url("/slow.bin"), "--queue", "--folder", other.to_str().unwrap()]);
    assert!(queued.message.as_deref().unwrap().contains("waiting in the queue"), "{queued:?}");
    let slow = ask(&dir, &["list", "--search", "slow"]).downloads.unwrap()[0].clone();
    assert_eq!((slow.state.as_str(), slow.folder.as_str()), ("waiting", other.to_str().unwrap()));
    assert!(ask(&dir, &["resume", &slow.id.to_string()]).ok);
    wait_for(&dir, slow.id, "downloading");
    assert!(ask(&dir, &["pause", &slow.id.to_string()]).ok);
    wait_for(&dir, slow.id, "paused");

    // Export Links: each download's name with its link and mirrors under it, as a tree; all, or those asked for.
    s.put("/mirror/slow.bin", MockFile::new(data.clone()));
    m.add_mirrors(slow.id, &[s.url("/mirror/slow.bin")]);
    let tree = ask(&dir, &["export"]).message.unwrap();
    assert_eq!(tree, format!(".\n├── small.bin\n│   └── {small}\n└── slow.bin\n    ├── {}\n    └── {}", s.url("/slow.bin"), s.url("/mirror/slow.bin")));
    let one = ask(&dir, &["export", &slow.id.to_string()]).message.unwrap();
    assert!(one.starts_with(".\n└── slow.bin\n") && !one.contains("small.bin"), "{one}");
    assert_eq!(ask(&dir, &["export", "999999"]).error.as_deref(), Some("No download 999999"));

    let wrong = ask(&dir, &["pause", "999999"]);
    assert_eq!(wrong.error.as_deref(), Some("There's no download with id 999999."));
    assert!(!ask(&dir, &["add", "ftp://example.com/a"]).ok);

    assert!(ask(&dir, &["remove", &id.to_string()]).ok);
    assert!(dir.join("files/small.bin").exists(), "remove without --delete-files keeps the file");
    assert_eq!(ask(&dir, &["list"]).downloads.unwrap().len(), 1);
    let gone = ask(&dir, &["remove", &slow.id.to_string(), "--delete-files"]);
    assert_eq!(gone.message.as_deref(), Some("Removed 1 download from the list and moved their files to the Trash."));
    assert!(ask(&dir, &["list"]).downloads.unwrap().is_empty());
    assert!(std::fs::read_dir(dir.join("Trash")).unwrap().count() > 0, "the partial file went to the Trash");

    // A request with someone else's token is refused.
    let port: Value = serde_json::from_slice(&std::fs::read(dir.join(control::FILE)).unwrap()).unwrap();
    let stream = std::net::TcpStream::connect(("127.0.0.1", port["port"].as_u64().unwrap() as u16)).unwrap();
    std::io::Write::write_all(&mut &stream, b"{\"token\":\"guess\",\"command\":\"list\"}\n").unwrap();
    let mut reply = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(&stream), &mut reply).unwrap();
    assert!(reply.contains("token is wrong") && !reply.contains("slow.bin"), "{reply}");

    assert!(m.quit(Duration::from_secs(5)));
    assert!(!dir.join(control::FILE).exists(), "quitting closes the door");
}

#[test]
fn the_mcp_server_lists_its_tools_and_calls_them() {
    let dir = folder("mcp");
    let m = Manager::open(&dir).unwrap();
    let call = |message: Value| -> Value { serde_json::from_str(&cli::mcp_reply(&message.to_string(), &dir).unwrap()).unwrap() };

    let init = call(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                           "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}}));
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert!(init["result"]["capabilities"]["tools"].is_object());
    assert!(cli::mcp_reply(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string(), &dir).is_none());

    let tools = call(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    let names: Vec<&str> = tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["list_downloads", "export_links", "add_downloads", "pause_downloads", "resume_downloads", "remove_downloads", "scan_page", "set_note"]);

    // Before the app listens, a tool says so instead of failing the protocol.
    let closed = call(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "list_downloads", "arguments": {}}}));
    assert_eq!(closed["result"]["isError"], true);
    assert_eq!(closed["result"]["content"][0]["text"], control::NOT_RUNNING);

    control::serve(&m, dir.join("files")).unwrap();
    let added = call(json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
                            "params": {"name": "add_downloads", "arguments": {"urls": ["http://127.0.0.1:9/a.iso"], "queue": true}}}));
    assert_eq!(added["result"]["isError"], false, "{added}");
    let listed = call(json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "list_downloads", "arguments": {"filter": "paused"}}}));
    let list: Value = serde_json::from_str(listed["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(list[0]["name"], "a.iso");
    let id = list[0]["id"].as_u64().unwrap();
    let noted = call(json!({"jsonrpc": "2.0", "id": 6, "method": "tools/call", "params": {"name": "set_note", "arguments": {"id": id, "note": "iso"}}}));
    assert_eq!(noted["result"]["content"][0]["text"], "Note saved.");
    assert_eq!(m.rows()[0].note, "iso");
    let unknown = call(json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": "format_disk", "arguments": {}}}));
    assert_eq!(unknown["error"]["code"], -32602);
    let nothing = call(json!({"jsonrpc": "2.0", "id": 8, "method": "resources/list"}));
    assert_eq!(nothing["error"]["code"], -32601);
}

#[test]
fn the_command_line_parses_its_words() {
    let parse = |args: &[&str]| cli::parse(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>());
    let (r, json) = parse(&["list", "--filter", "finished", "--json"]).unwrap();
    assert_eq!((r.command.as_str(), r.filter.as_deref(), json), ("list", Some("finished"), true));
    assert_eq!(parse(&["remove", "3", "4", "--delete-files"]).unwrap().0.ids, [3, 4]);
    assert!(parse(&["remove", "3", "--delete-files"]).unwrap().0.delete_files);
    assert_eq!(parse(&["note", "3", ""]).unwrap().0.text, "");
    assert!(parse(&["pause", "x"]).is_err());
    assert!(parse(&["pause", "3", "--queue"]).is_err());
    assert!(parse(&["fly"]).is_err());
    let relative = parse(&["add", "http://a/b", "--folder", "here"]).unwrap().0.folder.unwrap();
    assert!(std::path::Path::new(&relative).is_absolute());
}

/// One native messaging exchange: the length in the computer's byte order, then JSON.
fn framed(messages: &[Value]) -> Vec<u8> {
    let mut out = Vec::new();
    for m in messages {
        let body = m.to_string().into_bytes();
        out.extend((body.len() as u32).to_ne_bytes());
        out.extend(body);
    }
    out
}

fn unframed(mut bytes: &[u8]) -> Vec<Value> {
    let mut out = Vec::new();
    while bytes.len() >= 4 {
        let n = u32::from_ne_bytes(bytes[..4].try_into().unwrap()) as usize;
        out.push(serde_json::from_slice(&bytes[4..4 + n]).unwrap());
        bytes = &bytes[4 + n..];
    }
    out
}

#[test]
fn the_browser_extension_hands_over_a_download_with_its_cookies() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    s.put("/file.bin", MockFile::new(pattern(5000, 9)));
    let dir = folder("native");
    let m = Manager::open(&dir).unwrap();
    let talk = |messages: &[Value]| {
        let mut out = Vec::new();
        assert_eq!(cli::native(&mut framed(messages).as_slice(), &mut out, &dir), 0);
        unframed(&out)
    };

    let closed = talk(&[json!({"command": "ping"})]);
    assert_eq!(closed[0]["error"], control::NOT_RUNNING, "the extension lets the browser keep it");
    // The right-click menu's add asks for the app (§13): hdm tries to open it in the background instead of saying it
    // isn't running (here there's no app beside the test, so opening it fails, saying so).
    let menu = talk(&[
        json!({"command": "add", "urls": [s.url("/file.bin")]}),
        json!({"command": "add", "urls": [s.url("/file.bin")], "open": true}),
    ]);
    assert_eq!(menu[0]["error"], control::NOT_RUNNING, "a plain add leaves a closed app alone");
    assert!(menu[1]["error"].as_str().unwrap().contains("isn't next to hdm"), "{}", menu[1]);

    control::serve(&m, dir.join("files")).unwrap();
    let answers = talk(&[
        json!({"command": "ping"}),
        json!({"command": "add", "urls": [s.url("/missing.bin")], "check": true}),
        json!({"command": "add", "urls": [s.url("/file.bin")], "check": true, "cookies": "session=abc", "referer": "http://127.0.0.1/page"}),
        json!({"command": "remove", "ids": [1]}),
    ]);
    assert_eq!(answers[0]["ok"], true);
    assert!(answers[1]["error"].as_str().unwrap().contains("can't download"), "{}", answers[1]);
    assert_eq!(answers[2]["ok"], true, "{}", answers[2]);
    assert!(answers[3]["error"].as_str().unwrap().contains("can't ask for “remove”"), "only add and ping pass");
    assert_eq!(m.rows().len(), 1, "the missing link wasn't added");
    match m.vault().for_host("127.0.0.1") {
        hd_core::vault::Lookup::Found(sign_in) => {
            assert_eq!((sign_in.cookies.as_str(), sign_in.referer.as_str()), ("session=abc", "http://127.0.0.1/page"))
        }
        _ => panic!("the browser's cookies weren't kept for the site"),
    }
    assert!(m.vault().list().is_empty(), "nor written to sign-ins.json");
    assert!(m.quit(Duration::from_secs(5)));
}

#[test]
fn the_extension_adds_a_stream_by_its_type_named_after_its_page() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    let media = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/media/h264-ts");
    // The playlist at an address that doesn't say it's one; its pieces beside it.
    s.put("/clip/play", MockFile::new(std::fs::read(media.join("index.m3u8")).unwrap()));
    for n in 0..3 {
        s.put(&format!("/clip/s{n}.ts"), MockFile::new(std::fs::read(media.join(format!("s{n}.ts"))).unwrap()));
    }
    let dir = folder("page-stream");
    let m = Manager::open(&dir).unwrap();
    control::serve(&m, dir.join("files")).unwrap();
    let mut out = Vec::new();
    let message = json!({"command": "add", "urls": [s.url("/clip/play")], "check": true, "queue": true, "stream": true,
                         "name": "My Trip: part 1/2 | Videos.mp4"});
    assert_eq!(cli::native(&mut framed(&[message]).as_slice(), &mut out, &dir), 0);
    assert_eq!(unframed(&out)[0]["ok"], true, "{}", unframed(&out)[0]);
    let row = m.rows().into_iter().next().unwrap();
    assert_eq!(row.name, "My Trip part 1 2 Videos.mp4", "a page title made fit for a file name");
    assert_eq!(row.status, hd_core::manager::Status::Waiting, "added to the queue");
    m.start_queue();
    let until = Instant::now() + Duration::from_secs(30);
    while m.rows()[0].status != hd_core::manager::Status::Finished {
        assert!(Instant::now() < until, "never finished: {:?}", m.rows()[0].status);
        std::thread::sleep(Duration::from_millis(50));
    }
    let bytes = std::fs::read(dir.join("files/My Trip part 1 2 Videos.mp4")).unwrap();
    assert_eq!(&bytes[4..8], b"ftyp", "joined into an MP4 although its address had no .m3u8");
    assert!(m.quit(Duration::from_secs(5)));
}

#[test]
fn a_stream_whose_pieces_want_the_browser_downloads_as_the_browser_asked() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    let media = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/media/h264-ts");
    s.put("/guarded/index.m3u8", MockFile::new(std::fs::read(media.join("index.m3u8")).unwrap()));
    // The pieces are only given to a browser on the film's page (as guarded streams do): its User-Agent and Referer.
    for n in 0..3 {
        let mut piece = MockFile::new(std::fs::read(media.join(format!("s{n}.ts"))).unwrap());
        piece.requires = vec![("user-agent".into(), "Mozilla/5.0 (Test) Browser/1".into()), ("referer".into(), "https://films.example/movie/feed".into())];
        s.put(&format!("/guarded/s{n}.ts"), piece);
    }
    let dir = folder("dressed");
    let m = Manager::open(&dir).unwrap();
    control::serve(&m, dir.join("files")).unwrap();
    let add = |extra: Value| {
        let mut message = json!({"command": "add", "urls": [s.url("/guarded/index.m3u8")], "check": true, "stream": true});
        message.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let mut out = Vec::new();
        assert_eq!(cli::native(&mut framed(&[message]).as_slice(), &mut out, &dir), 0);
        unframed(&out)[0].clone()
    };
    let wait = |name: &str| -> hd_core::manager::Status {
        let until = Instant::now() + Duration::from_secs(30);
        loop {
            let row = m.rows().into_iter().find(|r| r.name == name).unwrap();
            if matches!(row.status, hd_core::manager::Status::Finished | hd_core::manager::Status::Failed(_)) || Instant::now() > until {
                return row.status;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    // Plainly: the pieces are refused.
    assert_eq!(add(json!({"name": "plain.mp4"}))["ok"], true);
    assert!(matches!(wait("plain.mp4"), hd_core::manager::Status::Failed(_)), "a guarded piece refused a downloader");
    // (The same link added again would join it as a mirror: it goes first.)
    let plain = m.rows().into_iter().find(|r| r.name == "plain.mp4").unwrap().id;
    m.remove(&[plain], false);
    // As the browser asked: it downloads.
    let answer = add(json!({"name": "dressed.mp4", "user_agent": "Mozilla/5.0 (Test) Browser/1", "referer": "https://films.example/movie/feed"}));
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(wait("dressed.mp4"), hd_core::manager::Status::Finished);
    assert!(m.quit(Duration::from_secs(5)));
}

#[test]
fn a_stream_whose_player_sent_no_referer_goes_without_one() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    let media = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/media/h264-ts");
    s.put("/player/index.m3u8", MockFile::new(std::fs::read(media.join("index.m3u8")).unwrap()));
    // A player in an embedded frame that sends no Referer, only its Origin: its server refuses any page named.
    for n in 0..3 {
        let mut piece = MockFile::new(std::fs::read(media.join(format!("s{n}.ts"))).unwrap());
        piece.requires = vec![("origin".into(), "https://player.films.example".into())];
        piece.forbids = vec!["referer".into()];
        s.put(&format!("/player/s{n}.ts"), piece);
    }
    let dir = folder("no-referer");
    let m = Manager::open(&dir).unwrap();
    control::serve(&m, dir.join("files")).unwrap();
    let add = |extra: Value| {
        let mut message = json!({"command": "add", "urls": [s.url("/player/index.m3u8")], "check": true, "stream": true});
        message.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let mut out = Vec::new();
        assert_eq!(cli::native(&mut framed(&[message]).as_slice(), &mut out, &dir), 0);
        unframed(&out)[0].clone()
    };
    let wait = |name: &str| -> hd_core::manager::Status {
        let until = Instant::now() + Duration::from_secs(30);
        loop {
            let row = m.rows().into_iter().find(|r| r.name == name).unwrap();
            if matches!(row.status, hd_core::manager::Status::Finished | hd_core::manager::Status::Failed(_)) || Instant::now() > until {
                return row.status;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    // The tab's page as Referer (what the extension sent before 1.23): refused, 403.
    assert_eq!(add(json!({"name": "page.mp4", "referer": "https://films.example/watch/1"}))["ok"], true);
    assert!(matches!(wait("page.mp4"), hd_core::manager::Status::Failed(e) if e.contains("403")));
    let page = m.rows().into_iter().find(|r| r.name == "page.mp4").unwrap().id;
    m.remove(&[page], false);
    // Exactly as the player asked: no Referer, its Origin.
    let answer = add(json!({"name": "player.mp4", "referer": "", "origin": "https://player.films.example"}));
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(wait("player.mp4"), hd_core::manager::Status::Finished);
    assert!(m.quit(Duration::from_secs(5)));
}

#[test]
fn a_naming_rules_subfolder_is_made_inside_the_save_folder() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let s = rt.block_on(MockServer::start()).unwrap();
    s.put("/song.mp3", MockFile::new(pattern(3000, 4)));
    let dir = folder("subfolder");
    let m = Manager::open(&dir).unwrap();
    control::serve(&m, dir.join("files")).unwrap();
    let mut out = Vec::new();
    let message = json!({"command": "add", "urls": [s.url("/song.mp3")], "check": true, "name": "Band - Song.mp3",
                         "subfolder": "../Music/Live"});
    assert_eq!(cli::native(&mut framed(&[message]).as_slice(), &mut out, &dir), 0);
    assert_eq!(unframed(&out)[0]["ok"], true, "{}", unframed(&out)[0]);
    let id = m.rows()[0].id;
    let until = Instant::now() + Duration::from_secs(30);
    while m.rows()[0].status != hd_core::manager::Status::Finished {
        assert!(Instant::now() < until, "never finished");
        std::thread::sleep(Duration::from_millis(50));
    }
    let row = m.rows().into_iter().find(|r| r.id == id).unwrap();
    assert_eq!(row.path.as_deref(), Some(dir.join("files/Music/Live/Band - Song.mp3").as_path()), "“..” can't leave the folder");
    assert!(m.quit(Duration::from_secs(5)));
}
