//! The main window (§2–§4): the Main Menu (☰) in the title bar's left corner, the toolbar, the sidebar filters with
//! their file-type groups, the download list, the find bar and the footer. Every command is a window action with the
//! Mac's shortcut (Ctrl for ⌘); the list refreshes twice a second, as on the Mac.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use hd_core::format::{self, FileKind, Filter};
use hd_core::manager::{Manager, Row, Status};

use crate::list::{COLUMNS, DownloadList};

const GITHUB: &str = "https://github.com/dunkelherzmann/herbs-download-manager";

/// Commands that arrive with their own windows later in `linux/pending.md`; their items show, disabled.
/// Keep on Top can't be done on Linux (GTK 4, Wayland); its item shows, disabled, with a note in its hover text.
const LATER: [&str; 1] = ["keep-on-top"];

fn item(menu: &gio::Menu, label: &str, action: &str) {
    menu.append(Some(label), Some(&format!("win.{action}")));
}

/// The Main Menu (§4): File, Edit, Tasks, View, Window and Help, then Settings, About and Quit.
fn main_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let file = gio::Menu::new();
    item(&file, "New Download…", "new-download");
    item(&file, "Import Links…", "import-links");
    item(&file, "Add from Clipboard", "add-from-clipboard");
    item(&file, "Batch Download…", "batch-download");
    item(&file, "Export Links…", "export-links");
    let close = gio::Menu::new();
    item(&close, "Close Window", "close");
    item(&close, "Exit", "quit");
    file.append_section(None, &close);
    menu.append_submenu(Some("File"), &file);

    let edit = gio::Menu::new();
    item(&edit, "Copy Link", "copy-link");
    item(&edit, "Select All", "select-all");
    let find = gio::Menu::new();
    item(&find, "Find", "find");
    edit.append_section(None, &find);
    menu.append_submenu(Some("Edit"), &edit);

    let tasks = gio::Menu::new();
    let run = gio::Menu::new();
    item(&run, "Resume", "resume");
    item(&run, "Pause", "pause");
    tasks.append_section(None, &run);
    let queue = gio::Menu::new();
    item(&queue, "Start Queue", "start-queue");
    item(&queue, "Stop Queue", "stop-queue");
    tasks.append_section(None, &queue);
    let all = gio::Menu::new();
    item(&all, "Stop All", "stop-all");
    tasks.append_section(None, &all);
    let one = gio::Menu::new();
    item(&one, "Add Mirror…", "add-mirror");
    item(&one, "Properties", "properties");
    tasks.append_section(None, &one);
    let file = gio::Menu::new();
    item(&file, "Rename…", "rename-file");
    item(&file, "Move To…", "move-file");
    item(&file, "Note…", "note");
    tasks.append_section(None, &file);
    let delete = gio::Menu::new();
    item(&delete, "Selected", "delete");
    item(&delete, "All Finished", "delete-finished");
    item(&delete, "All Unfinished", "delete-unfinished");
    item(&delete, "All Missing Files", "delete-missing");
    item(&delete, "Entire List", "delete-all");
    let delete_section = gio::Menu::new();
    delete_section.append_submenu(Some("Delete"), &delete);
    tasks.append_section(None, &delete_section);
    menu.append_submenu(Some("Tasks"), &tasks);

    let view = gio::Menu::new();
    item(&view, "Show Sidebar", "show-sidebar");
    item(&view, "Keep on Top", "keep-on-top");
    let columns = gio::Menu::new();
    for (key, title, _) in COLUMNS.iter().skip(1) {
        item(&columns, title, &format!("column-{key}"));
    }
    view.append_submenu(Some("Columns"), &columns);
    item(&view, "Show Download Window", "show-download-window");
    menu.append_submenu(Some("View"), &view);

    let window = gio::Menu::new();
    item(&window, "Minimize", "minimize");
    item(&window, "Zoom", "zoom");
    menu.append_submenu(Some("Window"), &window);

    let help = gio::Menu::new();
    item(&help, "Herb's Download Manager Help", "help");
    menu.append_submenu(Some("Help"), &help);

    let end = gio::Menu::new();
    item(&end, "Settings…", "settings");
    item(&end, "About Herb's Download Manager", "about");
    menu.append_section(None, &end);
    let quit = gio::Menu::new();
    item(&quit, "Quit Herb's DM", "quit");
    menu.append_section(None, &quit);
    menu
}

/// §4.1: the menu for right-clicked downloads.
fn context_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let open = gio::Menu::new();
    item(&open, "Open", "open");
    item(&open, "Show in Folder", "show-in-folder");
    item(&open, "Rename…", "rename-file");
    item(&open, "Move To…", "move-file");
    item(&open, "Note…", "note");
    menu.append_section(None, &open);
    let run = gio::Menu::new();
    item(&run, "Resume", "resume");
    item(&run, "Pause", "pause");
    item(&run, "Restart", "restart");
    menu.append_section(None, &run);
    let more = gio::Menu::new();
    item(&more, "Add Mirror…", "add-mirror");
    let connections = gio::Menu::new();
    item(&connections, "Add Connection", "add-connection");
    item(&connections, "Remove Connection", "remove-connection");
    more.append_submenu(Some("Connections"), &connections);
    menu.append_section(None, &more);
    let info = gio::Menu::new();
    item(&info, "Copy Link", "copy-link");
    item(&info, "Properties", "properties");
    menu.append_section(None, &info);
    let delete = gio::Menu::new();
    item(&delete, "Delete", "delete");
    menu.append_section(None, &delete);
    menu
}

/// A flat toolbar button for a window action, with its hover text and screen-reader name (§3).
fn tool(icon: &str, action: &str, label: &str, help: &str) -> gtk::Button {
    let b = gtk::Button::builder().icon_name(icon).action_name(format!("win.{action}")).tooltip_text(help).build();
    b.add_css_class("flat");
    b.update_property(&[gtk::accessible::Property::Label(label)]);
    b
}

fn separator() -> gtk::Separator {
    let s = gtk::Separator::new(gtk::Orientation::Vertical);
    s.set_margin_top(10);
    s.set_margin_bottom(10);
    s
}

struct Ui {
    manager: Manager,
    list: DownloadList,
    filter: Cell<Filter>,
    kind: Cell<Option<FileKind>>,
    search: RefCell<String>,
    notice: RefCell<Option<(String, std::time::Instant)>>,
}

pub fn build(app: &adw::Application, manager: Manager) -> adw::ApplicationWindow {
    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&main_menu())
        .tooltip_text("Main Menu: every command, from File, Edit, Tasks, View, Window and Help to Settings and Quit")
        .build();
    menu_button.update_property(&[gtk::accessible::Property::Label("Main Menu")]);
    let header = adw::HeaderBar::new();
    header.pack_start(&menu_button);
    header.set_title_widget(Some(&adw::WindowTitle::new("Herb's Download Manager", "")));
    // The toolbar (§3), right to left from the window controls: T10 Search, T11 Keep on Top, T8 Settings, T7 Delete,
    // T6 Stop All, T5–T4 the queue, T3–T2 Pause and Resume, T1 New Download.
    let tools: Vec<gtk::Widget> = vec![
        tool("system-search-symbolic", "find", "Search", "Search: show only downloads whose name, link or note contains some text (Ctrl+F)").upcast(),
        tool("view-pin-symbolic", "keep-on-top", "Keep on Top",
             "Keep on Top: Linux desktops don't let an app keep itself above others; use the title bar's right-click menu ▸ Always on Top").upcast(),
        tool("emblem-system-symbolic", "settings", "Settings", "Settings (Ctrl+,)").upcast(),
        separator().upcast(),
        tool("user-trash-symbolic", "delete", "Delete", "Delete: remove the selected downloads from the list (Delete)").upcast(),
        separator().upcast(),
        tool("process-stop-symbolic", "stop-all", "Stop All", "Stop All: stop the queue and pause every running download (Ctrl+Alt+.)").upcast(),
        separator().upcast(),
        tool("media-playback-stop-symbolic", "stop-queue", "Stop Queue", "Stop Queue: stop starting new items from the queue").upcast(),
        tool("media-seek-forward-symbolic", "start-queue", "Start Queue", "Start Queue: download waiting items one after another (Ctrl+Shift+R)").upcast(),
        separator().upcast(),
        tool("media-playback-pause-symbolic", "pause", "Pause", "Pause: stop the selected downloads, keeping what's downloaded (Ctrl+.)").upcast(),
        tool("media-playback-start-symbolic", "resume", "Resume", "Resume: continue the selected downloads, or with none selected start the waiting ones (Ctrl+R)").upcast(),
        separator().upcast(),
        tool("list-add-symbolic", "new-download", "New Download", "New Download: add a link to download (Ctrl+N)").upcast(),
    ];
    for t in &tools {
        header.pack_end(t);
    }

    // Sidebar (§2.1): each status filter with its count, and under it the file-type groups (shown by its arrow).
    let sidebar = gtk::ListBox::new();
    sidebar.add_css_class("navigation-sidebar");
    let mut counts = Vec::new();
    let mut groups: Vec<Vec<gtk::ListBoxRow>> = Vec::new();
    for filter in Filter::ALL {
        let icon = match filter {
            Filter::All => "folder-symbolic",
            Filter::Downloading => "go-down-symbolic",
            Filter::Paused => "media-playback-pause-symbolic",
            Filter::Finished => "object-select-symbolic",
            Filter::Unfinished => "document-open-recent-symbolic",
        };
        let arrow = gtk::Button::builder().icon_name("pan-end-symbolic").build();
        crate::explain(&arrow, &format!("{} file types", filter.title()), "Show or hide the file types under this filter");
        arrow.add_css_class("flat");
        arrow.add_css_class("circular");
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        line.append(&arrow);
        line.append(&gtk::Image::from_icon_name(icon));
        line.append(&gtk::Label::builder().label(filter.title()).xalign(0.0).hexpand(true).build());
        let count = gtk::Label::new(Some("0"));
        count.add_css_class("dim-label");
        line.append(&count);
        sidebar.append(&line);
        counts.push(count);
        let mut kinds = Vec::new();
        for kind in FileKind::ALL {
            let row = gtk::ListBoxRow::builder()
                .child(&gtk::Label::builder().label(kind.title()).xalign(0.0).margin_start(52).build())
                .visible(false)
                .build();
            sidebar.append(&row);
            kinds.push(row);
        }
        let shown = kinds.clone();
        arrow.connect_clicked(move |b| {
            let open = !shown[0].is_visible();
            shown.iter().for_each(|r| r.set_visible(open));
            b.set_icon_name(if open { "pan-down-symbolic" } else { "pan-end-symbolic" });
        });
        groups.push(kinds);
    }
    sidebar.select_row(sidebar.row_at_index(0).as_ref());
    let sidebar_scroll = gtk::ScrolledWindow::builder().child(&sidebar).width_request(190).build();

    let list = DownloadList::new();
    let empty = adw::StatusPage::builder().title("No downloads yet").vexpand(true).build();
    let list_scroll = gtk::ScrolledWindow::builder().child(&list.view).vexpand(true).hexpand(true).build();
    let stack = gtk::Stack::new();
    stack.add_named(&list_scroll, Some("list"));
    stack.add_named(&empty, Some("empty"));

    // The find bar (§2), at the top right of the list, as in Herb's Terminal.
    let search = gtk::SearchEntry::builder().placeholder_text("Search downloads").width_chars(28).build();
    search.update_property(&[gtk::accessible::Property::Label("Search downloads")]);
    let done = gtk::Button::builder().label("Done").tooltip_text("Done: hide the search and show every download (Esc)").build();
    let find_bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    find_bar.append(&search);
    find_bar.append(&done);
    find_bar.add_css_class("toolbar");
    find_bar.add_css_class("osd");
    find_bar.set_halign(gtk::Align::End);
    find_bar.set_valign(gtk::Align::Start);
    find_bar.set_margin_top(34);
    find_bar.set_margin_end(12);
    find_bar.set_visible(false);
    let overlay = gtk::Overlay::builder().child(&stack).build();
    overlay.add_overlay(&find_bar);

    let split = gtk::Paned::new(gtk::Orientation::Horizontal);
    split.set_start_child(Some(&sidebar_scroll));
    split.set_end_child(Some(&overlay));
    split.set_position(190);
    split.set_shrink_start_child(false);

    let footer = gtk::Label::builder().xalign(0.0).margin_start(12).margin_top(4).margin_bottom(4).build();
    footer.add_css_class("caption");

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&split));
    view.add_bottom_bar(&footer);
    view.set_bottom_bar_style(adw::ToolbarStyle::Raised);
    view.set_top_bar_style(adw::ToolbarStyle::Raised);

    // Test hook: `--window-size WIDTHxHEIGHT`, so a picture can show a whole dialog (Settings is taller than 640).
    let size = std::env::args().skip_while(|a| a != "--window-size").nth(1)
        .and_then(|v| v.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))))
        .unwrap_or((1100, 640));
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Herb's Download Manager")
        .default_width(size.0)
        .default_height(size.1)
        .content(&view)
        .build();

    let ui = Rc::new(Ui {
        manager,
        list: list.clone(),
        filter: Cell::new(Filter::All),
        kind: Cell::new(None),
        search: RefCell::default(),
        notice: RefCell::default(),
    });

    let finished_before: Rc<RefCell<Option<std::collections::HashSet<u64>>>> = Rc::default();
    let refresh: Rc<dyn Fn()> = {
        let (ui, stack, footer, window) = (ui.clone(), stack.clone(), footer.clone(), window.clone());
        Rc::new(move || {
            let rows = ui.manager.rows();
            // S9: a notification for each download that finished since the last refresh.
            let finished: std::collections::HashSet<u64> = rows.iter().filter(|r| r.status == Status::Finished).map(|r| r.id).collect();
            if let Some(before) = finished_before.borrow().as_ref()
                && ui.manager.settings().notify_when_finished
                && let Some(app) = window.application()
            {
                for r in rows.iter().filter(|r| finished.contains(&r.id) && !before.contains(&r.id)) {
                    let n = gio::Notification::new("Download finished");
                    n.set_body(Some(&r.name));
                    // Open and Show in Folder (enhancements.md); clicking the notification shows the file too.
                    n.add_button_with_target_value("Open", "app.open-file", Some(&r.id.to_variant()));
                    n.add_button_with_target_value("Show in Folder", "app.show-file", Some(&r.id.to_variant()));
                    n.set_default_action_and_target_value("app.show-file", Some(&r.id.to_variant()));
                    app.send_notification(Some(&format!("finished-{}", r.id)), &n);
                }
            }
            *finished_before.borrow_mut() = Some(finished);
            for (label, f) in counts.iter().zip(Filter::ALL) {
                label.set_label(&rows.iter().filter(|r| f.matches(&r.status)).count().to_string());
            }
            let shown = ui.list.show(&rows, ui.filter.get(), ui.kind.get(), &ui.search.borrow());
            stack.set_visible_child_name(if shown > 0 { "list" } else { "empty" });
            if let Some(page) = stack.child_by_name("empty").and_downcast::<adw::StatusPage>() {
                page.set_title(if rows.is_empty() { "No downloads yet" } else { "No downloads match" });
            }
            let selected = ui.list.selected();
            let active: Vec<&Row> = rows.iter().filter(|r| r.status.is_running()).collect();
            keep_awake(&window, !active.is_empty());
            let mut parts = vec![format!("{} selected", selected.len()), format!("{} active", active.len())];
            let speed: u64 = active.iter().map(|r| r.speed).sum();
            if speed > 0 {
                parts.push(format::speed_text(speed));
            }
            parts.extend(ui.manager.queue_text());
            let mut notice = ui.notice.borrow_mut();
            if notice.as_ref().is_some_and(|(_, at)| at.elapsed().as_secs() >= 6) {
                *notice = None;
            }
            if let Some((text, _)) = notice.as_ref() {
                parts.push(text.clone());
            }
            footer.set_label(&parts.join("  ·  "));
            enable(&window, &selected, &rows);
            crate::queuedone::check(&ui.manager, &window);
        })
    };

    let ctx = crate::add::Ctx {
        manager: ui.manager.clone(),
        window: window.clone(),
        tell: {
            let ui = ui.clone();
            Rc::new(move |text: String| *ui.notice.borrow_mut() = Some((text, std::time::Instant::now())))
        },
        refresh: refresh.clone(),
        open_links: Rc::default(),
    };
    add_actions(&window, app, &ui, &refresh, &find_bar, &search, &sidebar_scroll, &ctx);
    // Quit (or closing the window) while downloads run asks first, with Cancel the default (§10); they start again
    // at the next launch (the application's shutdown calls the manager's quit).
    {
        let (manager, app2) = (ui.manager.clone(), app.clone());
        let confirmed = std::rc::Rc::new(std::cell::Cell::new(false));
        window.connect_close_request(move |w| {
            if confirmed.get() || !manager.any_running() || !crate::signins::ASKS.get() {
                return glib::Propagation::Proceed;
            }
            let d = adw::AlertDialog::new(Some("Downloads are running. Pause them and quit?"),
                                          Some("They continue the next time you open Herb's Download Manager."));
            d.add_response("cancel", "Cancel");
            d.add_response("quit", "Pause and Quit");
            d.set_default_response(Some("cancel"));
            d.set_close_response("cancel");
            let (confirmed2, app3) = (confirmed.clone(), app2.clone());
            d.connect_response(Some("quit"), move |_, _| {
                confirmed2.set(true);
                app3.quit();
            });
            d.present(Some(w));
            glib::Propagation::Stop
        });
    }
    crate::features::install(&ctx, &ui.list);
    // Test hook: `--show new-download|add-links|batch [LINK…]` opens that window over the main one.
    {
        let args: Vec<String> = std::env::args().collect();
        if let Some(i) = args.iter().position(|a| a == "--show") {
            let links: Vec<String> = args[i + 2..].iter().take_while(|a| !a.starts_with("--")).cloned().collect();
            let (ctx, what) = (ctx.clone(), args.get(i + 1).cloned().unwrap_or_default());
            glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || match what.as_str() {
                "new-download" => crate::add::new_download(&ctx, links.into_iter().next()),
                "add-links" => crate::add::add_links(&ctx, links),
                "batch" => crate::add::batch(&ctx),
                "settings" => crate::settings::open(&ctx),
                "download-window" => {
                    if let Some(r) = ctx.manager.rows().first() {
                        crate::detail::open(&ctx, r.id, links.first().map(String::as_str));
                    }
                }
                _ => {}
            });
        }
    }
    // Links, text and link files dropped on the window open Add Links (§5.4).
    {
        let ctx2 = ctx.clone();
        view.add_controller(crate::add::drop_target(&ctx, move |links| crate::add::add_links(&ctx2, links)));
    }
    {
        let (ui, refresh, sidebar2) = (ui.clone(), refresh.clone(), sidebar.clone());
        sidebar.connect_row_selected(move |_, row| {
            let Some(row) = row else { return };
            let i = row.index().max(0) as usize;
            // Rows run: filter, its 7 file types, next filter…
            let (f, k) = (i / 8, i % 8);
            ui.filter.set(Filter::ALL[f.min(4)]);
            ui.kind.set(if k == 0 { None } else { Some(FileKind::ALL[k - 1]) });
            let _ = &sidebar2;
            refresh();
        });
    }
    {
        let (ui, refresh) = (ui.clone(), refresh.clone());
        search.connect_search_changed(move |entry| {
            *ui.search.borrow_mut() = entry.text().to_string();
            refresh();
        });
    }
    {
        let (find_bar2, search2) = (find_bar.clone(), search.clone());
        let close = move || {
            search2.set_text("");
            find_bar2.set_visible(false);
        };
        let c = close.clone();
        done.connect_clicked(move |_| c());
        search.connect_stop_search(move |_| close());
    }
    {
        let refresh = refresh.clone();
        ui.list.selection.connect_selection_changed(move |_, _, _| refresh());
    }
    // Double-click (§2.2): a finished download opens its file; an unfinished one its Download window.
    {
        let (ui, ctx) = (ui.clone(), ctx.clone());
        ui.list.view.clone().connect_activate(move |_, position| {
            let Some(r) = ui.list.row_at(position) else { return };
            match (&r.status, &r.path) {
                (Status::Finished, Some(path)) => {
                    let _ = gio::AppInfo::launch_default_for_uri(&gio::File::for_path(path).uri(), None::<&gio::AppLaunchContext>);
                }
                _ => crate::detail::open(&ctx, r.id, None),
            }
        });
    }
    // Right-click (§4.1).
    {
        let popover = gtk::PopoverMenu::from_model(Some(&context_menu()));
        popover.set_parent(&ui.list.view);
        popover.set_has_arrow(false);
        let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        click.connect_pressed(move |_, _, x, y| {
            popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.popup();
        });
        ui.list.view.add_controller(click);
    }
    refresh();
    glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
        refresh();
        // A download that met a server asking for a login gets a prompt for one.
        crate::signins::ask_if_needed(&ctx, &ctx.manager.rows());
        glib::ControlFlow::Continue
    });
    window
}

/// Enables each command for the selection (§3: buttons that don't apply are disabled).
fn enable(window: &adw::ApplicationWindow, selected: &[Row], rows: &[Row]) {
    let set = |name: &str, on: bool| {
        if let Some(a) = window.lookup_action(name).and_downcast::<gio::SimpleAction>() {
            a.set_enabled(on);
        }
    };
    let running = selected.iter().any(|r| r.status.is_running());
    let resumable = selected.iter().any(|r| matches!(r.status, Status::Waiting | Status::Paused | Status::Failed(_)));
    let unfinished = selected.iter().any(|r| r.status != Status::Finished);
    // With nothing selected, Resume starts what is waiting in the queue (oldest first).
    set("resume", resumable || (selected.is_empty() && rows.iter().any(|r| r.status == Status::Waiting)));
    set("pause", running);
    set("restart", unfinished);
    set("stop-all", rows.iter().any(|r| r.status.is_running()));
    set("delete", !selected.is_empty());
    set("copy-link", !selected.is_empty());
    set("open", selected.len() == 1 && selected[0].status == Status::Finished);
    set("show-in-folder", selected.len() == 1);
    let finished_file = selected.len() == 1 && selected[0].status == Status::Finished && selected[0].path.as_ref().is_some_and(|p| p.exists());
    set("rename-file", finished_file);
    set("move-file", finished_file);
    set("note", selected.len() == 1);
    set("add-connection", selected.len() == 1 && unfinished && selected[0].allowed_connections < 32);
    set("remove-connection", selected.len() == 1 && unfinished && selected[0].allowed_connections > 1);
    set("delete-finished", rows.iter().any(|r| r.status == Status::Finished));
    set("delete-unfinished", rows.iter().any(|r| r.status != Status::Finished));
    set("delete-all", !rows.is_empty());
    set("export-links", !rows.is_empty());
    set("add-mirror", selected.len() == 1);
    set("properties", !selected.is_empty());
    set("show-download-window", !selected.is_empty());
}

#[allow(clippy::too_many_arguments)]
fn add_actions(
    window: &adw::ApplicationWindow,
    app: &adw::Application,
    ui: &Rc<Ui>,
    refresh: &Rc<dyn Fn()>,
    find_bar: &gtk::Box,
    search: &gtk::SearchEntry,
    sidebar: &gtk::ScrolledWindow,
    ctx: &crate::add::Ctx,
) {
    let add = |name: &str, accels: &[&str], run: Box<dyn Fn()>| {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(move |_, _| run());
        window.add_action(&action);
        if !accels.is_empty() {
            app.set_accels_for_action(&format!("win.{name}"), accels);
        }
    };
    let ids = |ui: &Rc<Ui>| ui.list.selected().iter().map(|r| r.id).collect::<Vec<_>>();
    macro_rules! on {
        ($name:expr, $accels:expr, |$ui:ident| $body:expr) => {{
            let ($ui, refresh) = (ui.clone(), refresh.clone());
            add($name, $accels, Box::new(move || {
                $body;
                refresh();
            }));
        }};
    }
    on!("resume", &["<Control>r"], |ui| {
        let ids = ids(&ui);
        if ids.is_empty() { ui.manager.start_queue() } else { ui.manager.resume(&ids) }
    });
    on!("pause", &["<Control>period"], |ui| ui.manager.pause(&ids(&ui)));
    on!("restart", &[], |ui| ui.manager.restart(&ids(&ui)));
    on!("start-queue", &["<Control><Shift>r"], |ui| ui.manager.start_queue());
    on!("stop-queue", &[], |ui| ui.manager.stop_queue());
    on!("stop-all", &["<Control><Alt>period"], |ui| ui.manager.pause_all());
    on!("add-connection", &[], |ui| for r in ui.list.selected() {
        ui.manager.set_connections(r.id, (r.allowed_connections + 1).min(32));
    });
    on!("remove-connection", &[], |ui| for r in ui.list.selected() {
        ui.manager.set_connections(r.id, r.allowed_connections.saturating_sub(1).max(1));
    });
    on!("select-all", &["<Control>a"], |ui| ui.list.selection.select_all());
    // The app's shortcuts are seen before the focused widget, so while the find bar's field has the cursor, the
    // ones a text field has too (Ctrl+A, Ctrl+C, Ctrl+V) are switched off and the field selects, copies and pastes.
    {
        let focus = gtk::EventControllerFocus::new();
        let set = |window: &adw::ApplicationWindow, on: bool| {
            for name in ["select-all", "copy-link", "add-from-clipboard"] {
                if let Some(action) = window.lookup_action(name).and_then(|a| a.downcast::<gio::SimpleAction>().ok()) {
                    action.set_enabled(on);
                }
            }
        };
        let w = window.clone();
        focus.connect_enter(move |_| set(&w, false));
        let w = window.clone();
        focus.connect_leave(move |_| set(&w, true));
        search.add_controller(focus);
    }
    {
        let (ui, window2) = (ui.clone(), window.clone());
        add("copy-link", &["<Control>c"], Box::new(move || {
            let links: Vec<String> = ui.list.selected().into_iter().map(|r| r.url).collect();
            window2.clipboard().set_text(&links.join("\n"));
        }));
    }
    {
        let ui = ui.clone();
        add("open", &[], Box::new(move || {
            if let Some(path) = ui.list.selected().first().and_then(|r| r.path.clone()) {
                let _ = gio::AppInfo::launch_default_for_uri(&gio::File::for_path(path).uri(), None::<&gio::AppLaunchContext>);
            }
        }));
    }
    {
        let (ui, window2) = (ui.clone(), window.clone());
        add("show-in-folder", &[], Box::new(move || {
            if let Some(r) = ui.list.selected().first() {
                let target = r.path.clone().unwrap_or_else(|| r.folder.clone());
                gtk::FileLauncher::new(Some(&gio::File::for_path(target))).open_containing_folder(
                    Some(&window2), None::<&gio::Cancellable>, |_| {});
            }
        }));
    }
    // Note…: the download's own note (§4.1).
    {
        let (ui, refresh, window2) = (ui.clone(), refresh.clone(), window.clone());
        add("note", &[], Box::new(move || {
            let Some(r) = ui.list.selected().into_iter().next() else { return };
            let d = adw::AlertDialog::new(Some(&format!("Note on “{}”", r.name)),
                                          Some("Shown in the Note column and found by the search. Leave it blank to remove it."));
            let field = gtk::Entry::builder().text(&r.note).placeholder_text("What it's for, where it came from…").activates_default(true).build();
            crate::explain(&field, "Note", "Your note on this download");
            d.set_extra_child(Some(&field));
            d.add_response("cancel", "Cancel");
            d.add_response("save", "Save");
            d.set_response_appearance("save", adw::ResponseAppearance::Suggested);
            d.set_default_response(Some("save"));
            let (ui2, refresh2) = (ui.clone(), refresh.clone());
            d.connect_response(Some("save"), move |_, _| {
                ui2.manager.set_note(r.id, &field.text());
                refresh2();
            });
            d.present(Some(&window2));
        }));
    }
    // Rename… and Move To… a finished download's file (enhancements.md).
    {
        let (ui, refresh, window2) = (ui.clone(), refresh.clone(), window.clone());
        add("rename-file", &["F2"], Box::new(move || {
            let Some(r) = ui.list.selected().into_iter().next().filter(|r| r.status == Status::Finished) else { return };
            let d = adw::AlertDialog::new(Some(&format!("Rename “{}”", r.name)), Some("The file keeps its folder and stays in the list."));
            let field = gtk::Entry::builder().text(&r.name).activates_default(true).build();
            crate::explain(&field, "New name", "The file's new name");
            d.set_extra_child(Some(&field));
            d.add_response("cancel", "Cancel");
            d.add_response("rename", "Rename");
            d.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
            d.set_default_response(Some("rename"));
            // The name without its extension is selected, as in Files.
            let stem = std::path::Path::new(&r.name).file_stem().map(|s| s.to_string_lossy().chars().count()).unwrap_or(0);
            field.connect_map(move |f| f.select_region(0, stem as i32));
            let (ui2, refresh2, window3) = (ui.clone(), refresh.clone(), window2.clone());
            d.connect_response(Some("rename"), move |_, _| {
                if field.text() != r.name.as_str()
                    && let Err(e) = ui2.manager.rename_file(r.id, &field.text())
                {
                    tell(&window3, &e);
                }
                refresh2();
            });
            d.present(Some(&window2));
        }));
    }
    {
        let (ui, refresh, window2) = (ui.clone(), refresh.clone(), window.clone());
        add("move-file", &[], Box::new(move || {
            let Some(r) = ui.list.selected().into_iter().next().filter(|r| r.status == Status::Finished) else { return };
            let picker = gtk::FileDialog::builder().title(format!("Move “{}” To", r.name)).accept_label("Move Here").build();
            if let Some(folder) = r.path.as_ref().and_then(|p| p.parent()) {
                picker.set_initial_folder(Some(&gio::File::for_path(folder)));
            }
            let (ui2, refresh2, window3) = (ui.clone(), refresh.clone(), window2.clone());
            picker.select_folder(Some(&window2), None::<&gio::Cancellable>, move |result| {
                if let Ok(dir) = result
                    && let Some(folder) = dir.path()
                    && let Err(e) = ui2.manager.move_file(r.id, &folder.join(&r.name))
                {
                    tell(&window3, &e);
                }
                refresh2();
            });
        }));
    }
    // Delete (§10): the selection, or one of Tasks ▸ Delete's groups.
    let delete = |name: &str, accels: &[&str], pick: fn(&Rc<Ui>) -> Vec<Row>, files_gone: bool| {
        let (ui, refresh, window2) = (ui.clone(), refresh.clone(), window.clone());
        add(name, accels, Box::new(move || confirm_delete(&window2, &ui, &refresh, pick(&ui), files_gone)));
    };
    delete("delete", &["Delete"], |ui| ui.list.selected(), false);
    delete("delete-finished", &[], |ui| ui.manager.rows().into_iter().filter(|r| r.status == Status::Finished).collect(), false);
    delete("delete-unfinished", &[], |ui| ui.manager.rows().into_iter().filter(|r| r.status != Status::Finished).collect(), false);
    delete("delete-missing", &[], |ui| {
        ui.manager.rows().into_iter().filter(|r| r.status == Status::Finished && !r.path.as_ref().is_some_and(|p| p.exists())).collect()
    }, true);
    delete("delete-all", &[], |ui| ui.manager.rows(), false);

    {
        let (find_bar, search) = (find_bar.clone(), search.clone());
        add("find", &["<Control>f"], Box::new(move || {
            find_bar.set_visible(true);
            search.grab_focus();
        }));
    }
    {
        let sidebar = sidebar.clone();
        add("show-sidebar", &["<Control><Super>s"], Box::new(move || sidebar.set_visible(!sidebar.is_visible())));
    }
    for (key, _, _) in COLUMNS.iter().skip(1) {
        let action = gio::SimpleAction::new_stateful(&format!("column-{key}"), None, &crate::list::shown_at_first(key).to_variant());
        let list = ui.list.clone();
        action.connect_activate(move |a, _| {
            let on = !a.state().and_then(|s| s.get::<bool>()).unwrap_or(true);
            a.set_state(&on.to_variant());
            list.set_column_visible(key, on);
        });
        window.add_action(&action);
    }
    {
        let w = window.clone();
        add("close", &["<Control>w"], Box::new(move || w.close()));
        let w = window.clone();
        add("minimize", &["<Control>m"], Box::new(move || w.minimize()));
        let w = window.clone();
        add("zoom", &[], Box::new(move || if w.is_maximized() { w.unmaximize() } else { w.maximize() }));
        // Quitting goes through the window's close, which asks first while downloads run.
        let w = window.clone();
        add("quit", &["<Control>q"], Box::new(move || w.close()));
        // Help (F1): the guide, HELP.md, on the project's page.
        add("help", &["F1"], Box::new(|| {
            let _ = gio::AppInfo::launch_default_for_uri(&format!("{GITHUB}/blob/main/HELP.md"), None::<&gio::AppLaunchContext>);
        }));
        let w = window.clone();
        add("about", &[], Box::new(move || {
            let about = adw::AboutDialog::builder()
                .application_name("Herb's Download Manager")
                .application_icon(crate::APP_ID)
                .developer_name("dunkelherzmann")
                .version(hd_core::version())
                .website(GITHUB)
                .license_type(gtk::License::MitX11)
                .build();
            about.present(Some(&w));
        }));
    }
    for (name, accels, run) in [
        ("new-download", &["<Control>n"][..], crate::add::new_download_from_clipboard as fn(&crate::add::Ctx)),
        ("import-links", &["<Control><Shift>o"], crate::add::import),
        // Ctrl+V too, in the main window: a text field with the cursor takes its own paste first.
        ("add-from-clipboard", &["<Control><Shift>v", "<Control>v"], crate::add::from_clipboard),
        ("batch-download", &["<Control><Shift>n"], crate::add::batch),
    ] {
        let ctx = ctx.clone();
        add(name, accels, Box::new(move || run(&ctx)));
    }
    {
        // The selected downloads, or all with none selected.
        let (ui, ctx) = (ui.clone(), ctx.clone());
        add("export-links", &["<Control><Shift>e"], Box::new(move || crate::add::export(&ctx, ui.list.selected().iter().map(|r| r.id).collect())));
    }
    {
        let (ui, ctx) = (ui.clone(), ctx.clone());
        add("add-mirror", &["<Control><Alt>m"], Box::new(move || {
            if let [r] = ui.list.selected().as_slice() {
                crate::detail::ask_mirrors(&ctx, r.id);
            }
        }));
    }
    {
        let (ui, ctx) = (ui.clone(), ctx.clone());
        add("properties", &["<Control>i"], Box::new(move || {
            for r in ui.list.selected() {
                crate::detail::open(&ctx, r.id, Some("info"));
            }
        }));
    }
    {
        let (ui, ctx) = (ui.clone(), ctx.clone());
        add("show-download-window", &["<Control>o"], Box::new(move || {
            for r in ui.list.selected() {
                crate::detail::open(&ctx, r.id, None);
            }
        }));
    }
    {
        let ctx = ctx.clone();
        add("settings", &["<Control>comma"], Box::new(move || crate::settings::open(&ctx)));
    }
    add("keep-on-top", &["<Control><Alt>t"], Box::new(|| {}));
    for name in LATER {
        if let Some(a) = window.lookup_action(name).and_downcast::<gio::SimpleAction>() {
            a.set_enabled(false);
        }
    }
}

/// Delete asks whether to remove only the list entries (the default) or to move the files to the Trash as well (§10).
fn confirm_delete(window: &adw::ApplicationWindow, ui: &Rc<Ui>, refresh: &Rc<dyn Fn()>, rows: Vec<Row>, files_gone: bool) {
    if rows.is_empty() {
        return;
    }
    let one = rows.len() == 1;
    let heading = if one { format!("Delete “{}”?", rows[0].name) } else { format!("Delete {} downloads?", rows.len()) };
    let body = if files_gone {
        "Their files are no longer on disk; only the list entries are removed.".to_string()
    } else {
        format!("Remove {} from the list only, or also move {} to the Trash? Files in the Trash can be put back until it's emptied.",
                if one { "it" } else { "them" }, if one { "its file" } else { "their files" })
    };
    let dialog = adw::AlertDialog::new(Some(&heading), Some(&body));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("list", "Remove from List");
    if !files_gone {
        dialog.add_response("trash", "Move to Trash Too");
        dialog.set_response_appearance("trash", adw::ResponseAppearance::Destructive);
    }
    dialog.set_default_response(Some("list"));
    dialog.set_close_response("cancel");
    let (ui, refresh) = (ui.clone(), refresh.clone());
    dialog.connect_response(None, move |_, response| {
        let ids: Vec<u64> = rows.iter().map(|r| r.id).collect();
        let kept = match response {
            "list" => ui.manager.remove(&ids, false),
            "trash" => ui.manager.remove(&ids, true),
            _ => return,
        };
        if let Some(first) = kept.first() {
            let name = first.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            *ui.notice.borrow_mut() = Some((format!("“{name}” couldn't go to the Trash, so it was left where it is"), std::time::Instant::now()));
        }
        refresh();
    });
    dialog.present(Some(window));
}

/// Saves the window as a PNG a moment after it appears (the test hook behind `--capture`), then quits.
pub fn capture_then_quit(app: &adw::Application, window: &adw::ApplicationWindow, path: std::path::PathBuf) {
    let (app, window) = (app.clone(), window.clone());
    glib::timeout_add_local_once(std::time::Duration::from_millis(2500), move || {
        // The frontmost window: a dialog the test opened, else the main window.
        let window: gtk::Window = gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Window>().ok())
            .filter(|w| w.is_visible() && w.upcast_ref::<gtk::Widget>() != window.upcast_ref::<gtk::Widget>())
            .next_back()
            .unwrap_or_else(|| window.clone().upcast());
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let (w, h) = (window.width() as f64, window.height() as f64);
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, w, h);
        let saved = snapshot
            .to_node()
            .zip(window.native().and_then(|n| n.renderer()))
            .map(|(node, renderer)| renderer.render_texture(&node, None).save_to_png(&path));
        if !matches!(saved, Some(Ok(()))) {
            eprintln!("capture failed: {saved:?}");
        }
        app.quit();
    });
}

/// A short message over the window.
fn tell(window: &adw::ApplicationWindow, text: &str) {
    let d = adw::AlertDialog::new(Some(text), None);
    d.add_response("ok", "OK");
    d.present(Some(window));
}

thread_local! {
    /// The inhibit cookie held while downloads run (0: none).
    static AWAKE: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// The computer doesn't suspend while a download runs (SPECIFICATION.md §10): the app asks the desktop (through its
/// portal, in the Flatpak) not to suspend while any download is active, and lets go when none is.
fn keep_awake(window: &adw::ApplicationWindow, downloading: bool) {
    let Some(app) = window.application() else { return };
    let held = AWAKE.get();
    if downloading && held == 0 {
        AWAKE.set(app.inhibit(Some(window), gtk::ApplicationInhibitFlags::SUSPEND, Some("Downloads are running")));
    } else if !downloading && held != 0 {
        app.uninhibit(held);
        AWAKE.set(0);
    }
}

