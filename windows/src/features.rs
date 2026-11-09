//! Feature tests (tests/features.scenario, run by core/src/scenario.rs): every step runs on the main thread through the
//! app's own code, the code its toolbar, menus and dialogs run. `--features SCRIPT OUT NAME=VALUE…` runs the scenario
//! on a background thread, writes the report to OUT and quits.

use hd_core::addflow::PendingLink;
use hd_core::manager::Manager;
use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Sent to the main window with a `Request` to carry out on its thread.
pub const WM_FEATURE: u32 = WM_APP + 20;

pub struct Request {
    pub name: String,
    pub ids: Vec<u64>,
    pub args: Vec<String>,
    pub result: Result<(), String>,
}

struct Host {
    window: isize,
}

impl hd_core::scenario::Host for Host {
    fn command(&self, name: &str, ids: &[u64], args: &[String]) -> Result<(), String> {
        let mut request = Request { name: name.into(), ids: ids.to_vec(), args: args.to_vec(), result: Err("not run".into()) };
        // SendMessage from another thread waits until the main thread has handled it.
        unsafe {
            SendMessageW(HWND(self.window as *mut _), WM_FEATURE, Some(WPARAM(0)), Some(LPARAM(&mut request as *mut _ as isize)));
        }
        request.result
    }
}

/// Carries out one step on the main thread (called from the main window's WM_FEATURE).
pub fn on_main(hwnd: HWND, request: &mut Request) {
    let Some(ctx) = crate::app::feature_ctx() else {
        request.result = Err("no main window".into());
        return;
    };
    crate::app::select_ids(&request.ids);
    let (name, ids, args) = (request.name.as_str(), &request.ids, &request.args);
    request.result = match name {
        "add" | "add-as-mirrors" => {
            // New Download and Add Links end in add_all; Add Links' "Treat as mirrors" makes one link with mirrors.
            let links = &args[2..];
            let pending: Vec<PendingLink> = if name == "add" {
                links.iter().map(|u| PendingLink { url: u.clone(), ..Default::default() }).collect()
            } else {
                vec![PendingLink { url: links[0].clone(), mirrors: links[1..].to_vec(), ..Default::default() }]
            };
            let note = crate::add::add_all(hwnd, &ctx.manager, pending, args[0].clone().into(), ctx.manager.settings().connections, args[1] == "start");
            (ctx.done)(note);
            Ok(())
        }
        "resume" | "pause" | "restart" | "start-queue" | "stop-queue" | "stop-all" => crate::app::run_command(hwnd, name),
        // Delete's question answered Remove from List, or Move to Recycle Bin Too.
        "delete" | "delete-files" => {
            let _ = ctx.manager.remove(ids, name == "delete-files");
            Ok(())
        }
        // Add Mirror's window, Add.
        "add-mirror" => {
            ctx.manager.add_mirrors(ids[0], args);
            Ok(())
        }
        // The Options tab's checksum field.
        "set-checksum" => hd_core::checksum::Algorithm::ALL
            .into_iter()
            .find(|a| a.extension() == args[0].to_lowercase() || a.name().eq_ignore_ascii_case(&args[0]))
            .ok_or_else(|| "no such algorithm".to_string())
            .and_then(|a| hd_core::checksum::Checksum::new(a, &args[1]))
            .map(|c| ctx.manager.set_checksum(ids[0], Some(c))),
        other => Err(format!("no command {other}")),
    };
}

/// `--features SCRIPT OUT NAME=VALUE…`
pub fn run_if_asked(window: HWND, manager: &Manager) {
    let args: Vec<String> = std::env::args().collect();
    let Some(i) = args.iter().position(|a| a == "--features") else { return };
    // The scenario gives sign-ins itself; a prompt would wait for a person.
    crate::signins::ASKS.set(false);
    let (Some(script), Some(out)) = (args.get(i + 1).and_then(|p| std::fs::read_to_string(p).ok()), args.get(i + 2).cloned()) else {
        return;
    };
    let vars: Vec<(String, String)> =
        args[i + 3..].iter().filter_map(|p| p.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))).collect();
    let manager = manager.clone();
    let host = Host { window: window.0 as isize };
    std::thread::spawn(move || {
        let results = hd_core::scenario::run(&manager, &host, &script, &vars);
        let _ = std::fs::write(&out, hd_core::scenario::report(&results));
        std::process::exit(if results.iter().all(|r| r.passed) { 0 } else { 1 });
    });
}
