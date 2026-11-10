//! `hdm`, the command line (SPECIFICATION.md §12): lists, adds, pauses, resumes and removes downloads and sets notes in
//! the running app, through `control`. `hdm mcp` is an MCP server on standard input and output with the same commands
//! as tools, for AI assistants. Each app ships it: in the Mac app's bundle, beside the Windows .exe, and in the
//! Flatpak (`flatpak run --command=hdm org.herbsdownloadmanager.app`).

use std::io::{BufRead, Read, Write};
use std::path::Path;

use serde_json::{Value, json};

use crate::control::{self, Answer, Request};
use crate::format;

pub const USAGE: &str = "\
hdm controls Herb's Download Manager while it runs.

  hdm list [--filter all|downloading|paused|finished|unfinished] [--search TEXT] [--json]
  hdm add LINK… [--folder FOLDER] [--queue] [--quality highest|1080|720…] [--sound LANGUAGE]
  hdm pause ID…
  hdm resume ID…
  hdm remove ID… [--delete-files]
  hdm note ID TEXT…           (an empty TEXT removes the note)
  hdm scan PAGE [--json] [--add]   lists a web page's files and media; --add puts them all in the queue
  hdm mcp                     an MCP server on standard input and output, for AI assistants

add starts the downloads now, or with --queue leaves them waiting in the queue; they go to the folder set in
Settings unless --folder gives another. For a streamed video, --quality and --sound (a language code such as de)
choose its quality and sound track instead of Settings'. remove takes downloads off the list and leaves their files, or with
--delete-files moves the files to the Trash too. IDs are the numbers hdm list shows.
";

/// Runs `hdm` with its arguments (without the program name); returns the exit code.
pub fn main(args: Vec<String>) -> i32 {
    let dir = control::default_data_dir();
    // A browser starts hdm as its extension's native messaging host (§13): Chrome and its kin pass the extension's
    // origin, Firefox the host's manifest and the extension's id.
    let from_browser = args.first().is_some_and(|a| a.starts_with("chrome-extension://"))
        || args.get(1).is_some_and(|a| a == crate::browser::FIREFOX_ID);
    if from_browser || args.first().is_some_and(|a| a == "native") {
        return native(&mut std::io::stdin().lock(), &mut std::io::stdout(), &dir);
    }
    match args.first().map(String::as_str) {
        None | Some("help" | "--help" | "-h") => {
            print!("{USAGE}");
            if args.is_empty() { 2 } else { 0 }
        }
        Some("mcp") => {
            let stdin = std::io::stdin();
            let mut out = std::io::stdout();
            for line in stdin.lock().lines() {
                let Ok(line) = line else { break };
                if let Some(reply) = mcp_reply(&line, &dir) {
                    let _ = writeln!(out, "{reply}");
                    let _ = out.flush();
                }
            }
            0
        }
        Some(_) => match parse(&args) {
            Err(e) => {
                eprintln!("hdm: {e}\n\n{USAGE}");
                2
            }
            Ok((request, as_json)) => match control::send(&dir, &request) {
                Err(e) | Ok(Answer { ok: false, error: Some(e), .. }) => {
                    eprintln!("hdm: {e}");
                    1
                }
                Ok(Answer { found: Some(found), message, .. }) => {
                    if as_json {
                        println!("{}", serde_json::to_string_pretty(&found).unwrap_or_default());
                    } else {
                        print!("{}", found_table(message.as_deref().unwrap_or(""), &found));
                    }
                    // --add: every file into the queue, a stream under its name.
                    if request.queue {
                        let mut failed = 0;
                        for f in &found {
                            let add = Request { command: "add".into(), urls: vec![f.url.clone()], queue: true, stream: f.stream, name: Some(f.name.clone()), referer: Some(request.urls[0].clone()), ..Default::default() };
                            if !matches!(control::send(&dir, &add), Ok(Answer { ok: true, .. })) {
                                failed += 1;
                            }
                        }
                        println!("Added {} to the queue{}.", found.len() - failed, if failed > 0 { format!(" ({failed} couldn't be added)") } else { String::new() });
                    }
                    0
                }
                Ok(answer) => {
                    match (&answer.downloads, as_json) {
                        (Some(list), true) => println!("{}", serde_json::to_string_pretty(list).unwrap_or_default()),
                        (Some(list), false) => print!("{}", table(list)),
                        _ => println!("{}", answer.message.unwrap_or_default()),
                    }
                    0
                }
            },
        },
    }
}

/// The request a command line asks for, and whether `--json` was given.
pub fn parse(args: &[String]) -> Result<(Request, bool), String> {
    let mut r = Request { command: args[0].clone(), ..Default::default() };
    let mut json = false;
    let mut words = Vec::new();
    let mut i = 1;
    while i < args.len() {
        let arg = args[i].clone();
        let mut value = |name: &str| -> Result<String, String> {
            i += 1;
            args.get(i).cloned().ok_or(format!("{name} needs a value"))
        };
        match arg.as_str() {
            "--filter" if r.command == "list" => r.filter = Some(value("--filter")?),
            "--search" if r.command == "list" => r.search = value("--search")?,
            "--json" if r.command == "list" || r.command == "scan" => json = true,
            "--add" if r.command == "scan" => r.queue = true,
            "--folder" if r.command == "add" => r.folder = Some(absolute(&value("--folder")?)),
            "--queue" if r.command == "add" => r.queue = true,
            "--quality" if r.command == "add" => r.quality = Some(value("--quality")?),
            "--sound" if r.command == "add" => r.sound = Some(value("--sound")?),
            "--delete-files" if r.command == "remove" => r.delete_files = true,
            flag if flag.starts_with("--") => return Err(format!("{} doesn't take {flag}", r.command)),
            word => words.push(word.to_string()),
        }
        i += 1;
    }
    let ids = |words: &[String]| -> Result<Vec<u64>, String> {
        words.iter().map(|w| w.parse().map_err(|_| format!("“{w}” isn't a download's id"))).collect()
    };
    match r.command.as_str() {
        "list" if words.is_empty() => {}
        "add" => r.urls = words,
        "scan" if words.len() == 1 => r.urls = words,
        "pause" | "resume" | "remove" => r.ids = ids(&words)?,
        "note" if !words.is_empty() => {
            r.ids = ids(&words[..1])?;
            r.text = words[1..].join(" ");
        }
        "list" | "note" => return Err(format!("{} doesn't take those words", r.command)),
        "scan" => return Err("scan takes the address of one web page".into()),
        other => return Err(format!("no command “{other}”")),
    }
    Ok((r, json))
}

/// A folder given on the command line, as a full path from where hdm runs.
fn absolute(folder: &str) -> String {
    let path = Path::new(folder);
    if path.is_absolute() {
        return folder.to_string();
    }
    std::env::current_dir().map(|d| d.join(path).display().to_string()).unwrap_or_else(|_| folder.to_string())
}

/// `hdm list`'s table: id, status, size and name, with the note after a dash.
pub fn table(list: &[control::Download]) -> String {
    if list.is_empty() {
        return "No downloads.\n".into();
    }
    let rows: Vec<[String; 4]> = list
        .iter()
        .map(|d| {
            let size = format::size_text(d.downloaded, d.size, d.state == "finished");
            let name = if d.note.is_empty() { d.name.clone() } else { format!("{} — {}", d.name, d.note) };
            // A failure's reason can be long: the table keeps its start (list --json has all of it).
            let status = if d.status.chars().count() > 40 { format!("{}…", d.status.chars().take(39).collect::<String>()) } else { d.status.clone() };
            [d.id.to_string(), status, size, name]
        })
        .collect();
    let header = ["ID".to_string(), "STATUS".into(), "SIZE".into(), "NAME".into()];
    let width = |c: usize| rows.iter().chain([&header]).map(|r| r[c].chars().count()).max().unwrap_or(0);
    let (w0, w1, w2) = (width(0), width(1), width(2));
    let mut out = String::new();
    for r in [&header].into_iter().chain(&rows) {
        out.push_str(&format!("{:>w0$}  {:<w1$}  {:>w2$}  {}\n", r[0], r[1], r[2], r[3]));
    }
    out
}

/// `hdm scan`'s table: kind, size and name, then the link.
pub fn found_table(title: &str, found: &[control::FoundFile]) -> String {
    let mut out = if title.is_empty() { String::new() } else { format!("{title}\n") };
    if found.is_empty() {
        out.push_str("No files or media found on this page.\n");
        return out;
    }
    let size = |f: &control::FoundFile| f.size.map(format::bytes).unwrap_or_else(|| "–".into());
    let (wk, ws) = (found.iter().map(|f| f.kind.chars().count()).max().unwrap_or(0), found.iter().map(|f| size(f).chars().count()).max().unwrap_or(0));
    for f in found {
        out.push_str(&format!("{:<wk$}  {:>ws$}  {}  {}\n", f.kind, size(f), f.name, f.url));
    }
    out
}

const PROTOCOLS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// The MCP tools: name, description, input schema, read-only, destructive.
fn tools() -> Value {
    let ids = json!({"type": "array", "items": {"type": "integer"}, "minItems": 1,
                     "description": "The downloads' ids, from list_downloads"});
    json!([
        {"name": "list_downloads", "title": "List downloads",
         "description": "Lists the downloads in Herb's Download Manager: id, name, state, status text, bytes downloaded, size, speed, link, folder, file path and note.",
         "inputSchema": {"type": "object", "properties": {
             "filter": {"type": "string", "enum": ["all", "downloading", "paused", "finished", "unfinished"], "description": "Which downloads (default all)"},
             "search": {"type": "string", "description": "Only downloads whose name, link or note contains this text"}}},
         "annotations": {"readOnlyHint": true}},
        {"name": "add_downloads", "title": "Add downloads",
         "description": "Adds http or https links as downloads. Links to the same file become one download with mirrors. They start now unless queue is true.",
         "inputSchema": {"type": "object", "required": ["urls"], "properties": {
             "urls": {"type": "array", "items": {"type": "string"}, "minItems": 1, "description": "The links to download"},
             "folder": {"type": "string", "description": "Full path of the folder to save to (default: the one set in Settings)"},
             "queue": {"type": "boolean", "description": "Leave them waiting in the queue instead of starting now"},
             "quality": {"type": "string", "description": "A streamed video's quality: highest, or the most lines such as 1080 or 720 (default: Settings')"},
             "sound": {"type": "string", "description": "A streamed video's sound language, such as en or de (default: Settings')"}}},
         "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": true}},
        {"name": "pause_downloads", "title": "Pause downloads", "description": "Pauses downloads, keeping what's downloaded.",
         "inputSchema": {"type": "object", "required": ["ids"], "properties": {"ids": ids}},
         "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true}},
        {"name": "resume_downloads", "title": "Resume downloads", "description": "Resumes paused, waiting or failed downloads.",
         "inputSchema": {"type": "object", "required": ["ids"], "properties": {"ids": ids}},
         "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": true}},
        {"name": "remove_downloads", "title": "Remove downloads",
         "description": "Removes downloads from the list. Their files stay, unless delete_files is true: then they go to the Trash.",
         "inputSchema": {"type": "object", "required": ["ids"], "properties": {"ids": ids,
             "delete_files": {"type": "boolean", "description": "Move the downloaded files to the Trash too"}}},
         "annotations": {"readOnlyHint": false, "destructiveHint": true}},
        {"name": "scan_page", "title": "Scan a web page for files",
         "description": "Reads a web page and lists the files and media it links to or plays (videos, streams, sounds, pictures, documents, archives, programs), each checked with its server: name, size, kind and link. Pages, scripts and styles are left out. Add one with add_downloads.",
         "inputSchema": {"type": "object", "required": ["url"], "properties": {"url": {"type": "string", "description": "The web page's address"}}},
         "annotations": {"readOnlyHint": true, "openWorldHint": true}},
        {"name": "set_note", "title": "Set a download's note",
         "description": "Sets the note shown in the download's Note column and found by the search; an empty note removes it.",
         "inputSchema": {"type": "object", "required": ["id", "note"], "properties": {
             "id": {"type": "integer", "description": "The download's id"},
             "note": {"type": "string", "description": "The note, one line"}}},
         "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true}}
    ])
}

/// The request an MCP tool call makes.
fn tool_request(name: &str, a: &Value) -> Result<Request, String> {
    let text = |k: &str| a.get(k).and_then(Value::as_str).map(str::to_string);
    let flag = |k: &str| a.get(k).and_then(Value::as_bool).unwrap_or(false);
    let ids = || -> Vec<u64> { a.get("ids").and_then(Value::as_array).map(|v| v.iter().filter_map(Value::as_u64).collect()).unwrap_or_default() };
    Ok(match name {
        "list_downloads" => Request { command: "list".into(), filter: text("filter"), search: text("search").unwrap_or_default(), ..Default::default() },
        "add_downloads" => Request {
            command: "add".into(),
            urls: a.get("urls").and_then(Value::as_array).map(|v| v.iter().filter_map(|u| u.as_str().map(str::to_string)).collect()).unwrap_or_default(),
            folder: text("folder"),
            queue: flag("queue"),
            quality: text("quality"),
            sound: text("sound"),
            ..Default::default()
        },
        "pause_downloads" => Request { command: "pause".into(), ids: ids(), ..Default::default() },
        "resume_downloads" => Request { command: "resume".into(), ids: ids(), ..Default::default() },
        "remove_downloads" => Request { command: "remove".into(), ids: ids(), delete_files: flag("delete_files"), ..Default::default() },
        "scan_page" => Request { command: "scan".into(), urls: text("url").into_iter().collect(), ..Default::default() },
        "set_note" => Request {
            command: "note".into(),
            ids: a.get("id").and_then(Value::as_u64).into_iter().collect(),
            text: text("note").unwrap_or_default(),
            ..Default::default()
        },
        other => return Err(format!("No tool “{other}”.")),
    })
}

/// The browser extension's side (§13): native messaging, one message at a time, each a 32-bit length in the
/// computer's byte order followed by that many bytes of JSON. Only `add`, `scan`, `ping` and `open` (which opens the
/// app) are passed on to the app, which
/// answers as for the command line ({ok, error, message}). Ends when the browser closes the pipe.
pub fn native(input: &mut impl Read, output: &mut impl Write, dir: &Path) -> i32 {
    /// Browsers send at most 4 GB, but a link and its cookies are far less.
    const MAX: usize = 1024 * 1024;
    loop {
        let mut size = [0u8; 4];
        if input.read_exact(&mut size).is_err() {
            return 0;
        }
        let size = u32::from_ne_bytes(size) as usize;
        if size > MAX {
            return 1;
        }
        let mut body = vec![0u8; size];
        if input.read_exact(&mut body).is_err() {
            return 1;
        }
        let answer = match serde_json::from_slice::<Request>(&body) {
            Err(_) => Answer { error: Some("The browser's message wasn't understood.".into()), ..Default::default() },
            Ok(r) if r.command == "ping" => match control::send(dir, &Request { command: "list".into(), filter: Some("downloading".into()), ..Default::default() }) {
                Ok(_) => Answer { ok: true, message: Some(crate::version().into()), ..Default::default() },
                Err(e) => Answer::failed(e),
            },
            // The extension's "Open and Download": the app, opened and answering.
            Ok(r) if r.command == "open" => match control::open_app(dir, std::time::Duration::from_secs(30)) {
                Ok(()) => Answer { ok: true, message: Some("Herb's Download Manager is open.".into()), ..Default::default() },
                Err(e) => Answer { error: Some(e), ..Default::default() },
            },
            Ok(r) if r.command == "add" => {
                let request = Request { command: "add".into(), ids: vec![], text: String::new(), delete_files: false, ..r };
                control::send(dir, &request).unwrap_or_else(Answer::failed)
            }
            // The extension's "Find Files on This Page": the app reads the page with the browser's cookies.
            Ok(r) if r.command == "scan" => {
                let request = Request { command: "scan".into(), urls: r.urls, cookies: r.cookies, referer: r.referer, ..Default::default() };
                control::send(dir, &request).unwrap_or_else(Answer::failed)
            }
            Ok(r) => Answer { error: Some(format!("The browser can't ask for “{}”.", r.command)), ..Default::default() },
        };
        let text = serde_json::to_vec(&answer).unwrap_or_default();
        if output.write_all(&(text.len() as u32).to_ne_bytes()).and_then(|_| output.write_all(&text)).and_then(|_| output.flush()).is_err() {
            return 1;
        }
    }
}

/// Answers one MCP message (JSON-RPC 2.0, one per line) for the app whose data folder is `dir`; `None` for
/// notifications, which get no answer.
pub fn mcp_reply(line: &str, dir: &Path) -> Option<String> {
    let reply = |id: &Value, result: Value| Some(json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string());
    let failure = |id: &Value, code: i64, message: &str| {
        Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}).to_string())
    };
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        return failure(&Value::Null, -32700, "Not JSON");
    };
    let id = message.get("id")?.clone();
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    match message.get("method").and_then(Value::as_str).unwrap_or("") {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
            let version = PROTOCOLS.iter().find(|v| **v == asked).unwrap_or(&PROTOCOLS[0]);
            reply(&id, json!({
                "protocolVersion": version,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "herbs-download-manager", "title": "Herb's Download Manager", "version": crate::version()},
                "instructions": "Controls Herb's Download Manager while it runs: list, add, pause, resume and remove downloads, and set their notes."
            }))
        }
        "ping" => reply(&id, json!({})),
        "tools/list" => reply(&id, json!({"tools": tools()})),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
            let request = match tool_request(name, &arguments) {
                Ok(r) => r,
                Err(e) => return failure(&id, -32602, &e),
            };
            let (text, is_error) = match control::send(dir, &request) {
                Err(e) => (e, true),
                Ok(Answer { ok: false, error, .. }) => (error.unwrap_or_default(), true),
                Ok(Answer { downloads: Some(list), .. }) => (serde_json::to_string_pretty(&list).unwrap_or_default(), false),
                Ok(Answer { found: Some(found), .. }) => (serde_json::to_string_pretty(&found).unwrap_or_default(), false),
                Ok(answer) => (answer.message.unwrap_or_default(), false),
            };
            reply(&id, json!({"content": [{"type": "text", "text": text}], "isError": is_error}))
        }
        _ => failure(&id, -32601, "Unknown method"),
    }
}
