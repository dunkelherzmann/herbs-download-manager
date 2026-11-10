//! Feature tests (tests/features.scenario, run by core/src/scenario.rs): every step runs on the GTK main thread through
//! the app's own code, the code its buttons, menus and dialogs run. `--features SCRIPT OUT NAME=VALUE…` runs the
//! scenario on a background thread, writes the report to OUT and quits.

use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};

use adw::prelude::*;
use gtk::glib;
use hd_core::addflow::PendingLink;

use crate::add::Ctx;
use crate::list::DownloadList;

/// Add flows still running (they're asynchronous on Linux); a step waits for them before it counts as done.
pub static PENDING_ADDS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static APP: RefCell<Option<(Ctx, DownloadList)>> = const { RefCell::new(None) };
}

/// The main window's context, for the steps (called when the window is built).
pub fn install(ctx: &Ctx, list: &DownloadList) {
    APP.with(|a| *a.borrow_mut() = Some((ctx.clone(), list.clone())));
}

struct Host;

impl hd_core::scenario::Host for Host {
    fn command(&self, name: &str, ids: &[u64], args: &[String]) -> Result<(), String> {
        let (tx, rx) = std::sync::mpsc::channel();
        let (name, ids, args) = (name.to_string(), ids.to_vec(), args.to_vec());
        glib::MainContext::default().invoke(move || {
            let _ = tx.send(on_main(&name, &ids, &args));
        });
        let result = rx.recv().map_err(|_| "the app stopped".to_string())?;
        while PENDING_ADDS.load(Ordering::SeqCst) > 0 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        result
    }
}

fn on_main(name: &str, ids: &[u64], args: &[String]) -> Result<(), String> {
    APP.with(|a| {
        let a = a.borrow();
        let (ctx, list) = a.as_ref().ok_or("no main window")?;
        // The list's rows are refreshed twice a second: read them now, so a command sees each row's current state
        // (and is enabled for it).
        (ctx.refresh)();
        list.select(ids);
        (ctx.refresh)();
        let action = |n: &str| WidgetExt::activate_action(&ctx.window, &format!("win.{n}"), None).map_err(|e| e.to_string());
        match name {
            "add" | "add-as-mirrors" => {
                // New Download and Add Links end in add_all; Add Links' "Treat as mirrors" makes one link with mirrors.
                let links = &args[2..];
                let pending: Vec<PendingLink> = if name == "add" {
                    links.iter().map(|u| PendingLink { url: u.clone(), ..Default::default() }).collect()
                } else {
                    vec![PendingLink { url: links[0].clone(), mirrors: links[1..].to_vec(), ..Default::default() }]
                };
                crate::add::add_all(ctx, pending, args[0].clone().into(), ctx.manager.settings().connections, args[1] == "start");
                Ok(())
            }
            "resume" | "pause" | "restart" | "start-queue" | "stop-queue" | "stop-all" => action(name),
            // Delete's question answered Remove from List, or Move to Trash Too.
            "delete" | "delete-files" => {
                let _ = ctx.manager.remove(ids, name == "delete-files");
                Ok(())
            }
            // Add Mirror's dialog, Add.
            "add-mirror" => {
                ctx.manager.add_mirrors(ids[0], args);
                Ok(())
            }
            // Rename…'s dialog and Move To…'s folder picker, answered.
            "rename" => ctx.manager.rename_file(ids[0], &args[0]).map(|_| ()),
            "move" => ctx.manager.move_file(ids[0], std::path::Path::new(&args[0])).map(|_| ()),
            // Note…'s dialog, answered.
            "note" => {
                ctx.manager.set_note(ids[0], &args.join(" "));
                Ok(())
            }
            // The Options tab's checksum field.
            "set-checksum" => {
                let algorithm = hd_core::checksum::Algorithm::ALL
                    .into_iter()
                    .find(|a| a.extension() == args[0].to_lowercase() || a.name().eq_ignore_ascii_case(&args[0]))
                    .ok_or("no such algorithm")?;
                ctx.manager.set_checksum(ids[0], Some(hd_core::checksum::Checksum::new(algorithm, &args[1])?));
                Ok(())
            }
            other => Err(format!("no command {other}")),
        }
    })
}

/// `--features SCRIPT OUT NAME=VALUE…`
pub fn run_if_asked(app: &adw::Application, manager: &hd_core::manager::Manager) {
    let args: Vec<String> = std::env::args().collect();
    let Some(i) = args.iter().position(|a| a == "--features") else { return };
    let (Some(script), Some(out)) = (args.get(i + 1).and_then(|p| std::fs::read_to_string(p).ok()), args.get(i + 2).cloned()) else {
        return;
    };
    let vars: Vec<(String, String)> =
        args[i + 3..].iter().filter_map(|p| p.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))).collect();
    crate::signins::ASKS.set(false);
    let manager = manager.clone();
    let app = app.clone();
    let (tx, rx) = std::sync::mpsc::channel::<bool>();
    std::thread::spawn(move || {
        let results = hd_core::scenario::run(&manager, &Host, &script, &vars);
        let _ = std::fs::write(&out, hd_core::scenario::report(&results));
        let _ = tx.send(results.iter().all(|r| r.passed));
    });
    glib::timeout_add_local(std::time::Duration::from_millis(200), move || match rx.try_recv() {
        Ok(passed) => {
            app.quit();
            std::process::exit(if passed { 0 } else { 1 });
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(_) => glib::ControlFlow::Break,
    });
}
