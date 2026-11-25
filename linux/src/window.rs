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

/// A menu item that shows a shortcut a text field handles itself (Ctrl+Z, Ctrl+X, Ctrl+C), without binding it.
fn item_showing(menu: &gio::Menu, label: &str, action: &str, accel: &str) {
    let i = gio::MenuItem::new(Some(label), Some(&format!("win.{action}")));
    i.set_attribute_value("accel", Some(&accel.to_variant()));
    menu.append_item(&i);
}

/// An item that only says something (Connections ▸ "2 of 8 connections in use"): no action, so it shows dimmed.
fn line(menu: &gio::Menu, label: &str) {
    menu.append_item(&gio::MenuItem::new(Some(label), None));
}

/// The Main Menu (§4): File, Edit, Tasks, View, Window and Help, then Settings, About and Quit. Also returns the
/// View menu, whose first item says Show Sidebar or Hide Sidebar as the sidebar is.
fn main_menu() -> (gio::Menu, gio::Menu) {
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

    // The standard Edit menu, as on the Mac: Undo, Redo, Cut, Copy and Paste act on the text field with the cursor;
    // Paste with none adds the clipboard's links (§5.2) and Select All selects every download.
    let edit = gio::Menu::new();
    let undo = gio::Menu::new();
    item_showing(&undo, "Undo", "undo", "<Control>z");
    item_showing(&undo, "Redo", "redo", "<Control><Shift>z");
    edit.append_section(None, &undo);
    let clipboard = gio::Menu::new();
    item_showing(&clipboard, "Cut", "cut", "<Control>x");
    item_showing(&clipboard, "Copy", "copy", "<Control>c");
    item(&clipboard, "Paste", "paste");
    item(&clipboard, "Select All", "select-all");
    edit.append_section(None, &clipboard);
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
    item(&view, "Hide Sidebar", "show-sidebar");
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
    let front = gio::Menu::new();
    item(&front, "Bring All to Front", "bring-all-to-front");
    window.append_section(None, &front);
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
    (menu, view)
}

/// §4.1: the menu for the right-clicked downloads, as the Mac builds it: items that don't apply are left out
/// (Rename and Move To unless one finished download is selected, Connections for finished ones), the rest are
/// enabled by the window's rules.
fn context_menu(rows: &[Row]) -> gio::Menu {
    let menu = gio::Menu::new();
    let open = gio::Menu::new();
    item(&open, "Open", "open");
    open.append_submenu(Some("Open With"), &open_with_menu(rows));
    item(&open, "Show in Folder", "show-in-folder");
    if let [r] = rows
        && r.status == Status::Finished
    {
        item(&open, "Rename…", "rename-file");
        item(&open, "Move To…", "move-file");
    }
    if rows.len() == 1 {
        item(&open, "Note…", "note");
    }
    menu.append_section(None, &open);
    let run = gio::Menu::new();
    item(&run, "Resume", "resume");
    item(&run, "Pause", "pause");
    item(&run, "Restart…", "restart");
    menu.append_section(None, &run);
    let more = gio::Menu::new();
    item(&more, "Add Mirror…", "add-mirror");
    if !rows.iter().any(|r| r.status == Status::Finished) {
        let connections = gio::Menu::new();
        item(&connections, "Add Connection", "add-connection");
        item(&connections, "Remove Connection", "remove-connection");
        if let [r] = rows {
            let live = gio::Menu::new();
            line(&live, &format!("{} of {} connections in use", r.active_connections, r.allowed_connections));
            for l in &r.live {
                line(&live, l);
            }
            connections.append_section(None, &live);
        }
        more.append_submenu(Some("Connections"), &connections);
    }
    menu.append_section(None, &more);
    let info = gio::Menu::new();
    item(&info, "Copy Link", "copy-link");
    item(&info, "Properties", "properties");
    menu.append_section(None, &info);
    let delete = gio::Menu::new();
    item(&delete, "Delete…", "delete");
    menu.append_section(None, &delete);
    menu
}

/// Open With ▸: the apps that can open the file, the default one first (§4.1).
fn open_with_menu(rows: &[Row]) -> gio::Menu {
    let menu = gio::Menu::new();
    let Some(path) = (match rows {
        [r] => r.path.clone().filter(|p| p.exists()),
        _ => None,
    }) else {
        line(&menu, "No File Yet");
        return menu;
    };
    let (content_type, _) = gio::content_type_guess(Some(&path), None);
    let preferred = gio::AppInfo::default_for_type(&content_type, false);
    let mut apps: Vec<gio::AppInfo> = preferred.iter().cloned().collect();
    for app in gio::AppInfo::all_for_type(&content_type) {
        if !apps.iter().any(|a| a.equal(&app)) {
            apps.push(app);
        }
    }
    for app in apps.iter().take(20) {
        let Some(id) = app.id() else { continue };
        let default = preferred.as_ref().is_some_and(|p| p.equal(app));
        let label = if default { format!("{} (default)", app.display_name()) } else { app.display_name().to_string() };
        let i = gio::MenuItem::new(Some(&label), None);
        i.set_action_and_target_value(Some("win.open-with"), Some(&id.to_variant()));
        i.set_icon(&app.icon().unwrap_or_else(|| gio::ThemedIcon::new("application-x-executable").upcast()));
        menu.append_item(&i);
    }
    // In the Flatpak the sandbox can't list the desktop's apps: the desktop's own app chooser (through its portal)
    // offers them instead.
    if apps.is_empty() {
        item(&menu, "Choose an App…", "open-with-chooser");
    }
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
    /// The text field the Edit menu's Undo, Cut, Copy, Paste and Select All act on: the one with the cursor, kept
    /// while the Main Menu is open. While one has it, the list's Delete and F2 leave the keys to the field.
    text: RefCell<Option<glib::WeakRef<gtk::Text>>>,
    manager: Manager,
    list: DownloadList,
    filter: Cell<Filter>,
    kind: Cell<Option<FileKind>>,
    search: RefCell<String>,
    notice: RefCell<Option<(String, std::time::Instant)>>,
}

pub fn build(app: &adw::Application, manager: Manager) -> adw::ApplicationWindow {
    let (menu_model, view_menu) = main_menu();
    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu_model)
        .tooltip_text("Main Menu: every command, from File, Edit, Tasks, View, Window and Help to Settings and Quit")
        .build();
    menu_button.update_property(&[gtk::accessible::Property::Label("Main Menu")]);
    let header = adw::HeaderBar::new();
    header.pack_start(&menu_button);
    header.set_title_widget(Some(&adw::WindowTitle::new("Herb's Download Manager", "")));
    // The toolbar (§3), right to left from the window controls: T10 Search, T11 Keep on Top, T8 Settings, T7 Delete,
    // T6 Stop All, T5–T4 the queue, T3–T2 Pause and Resume, T1 New Download.
    let tools: Vec<gtk::Widget> = vec![
        tool("system-search-symbolic", "toggle-search", "Search", "Search: show only downloads whose name, link or note contains some text (Ctrl+F)").upcast(),
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
    let mut kind_counts: Vec<Vec<gtk::Label>> = Vec::new();
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
        let mut numbers = Vec::new();
        for kind in FileKind::ALL {
            // Each file type with its symbol and count, as on the Mac (§2.1).
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            line.set_margin_start(40);
            line.append(&gtk::Image::from_icon_name(kind_icon(kind)));
            line.append(&gtk::Label::builder().label(kind.title()).xalign(0.0).hexpand(true).build());
            let number = gtk::Label::new(Some("0"));
            number.add_css_class("dim-label");
            line.append(&number);
            let row = gtk::ListBoxRow::builder().child(&line).visible(false).build();
            sidebar.append(&row);
            kinds.push(row);
            numbers.push(number);
        }
        kind_counts.push(numbers);
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
    crate::explain(&search, "Search downloads", "Show only downloads whose name, link or note contains this text");
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

    // While links are dragged over the window, it dims and says "Drop links here" (§2, §5.4).
    let drop_note = gtk::Label::builder().label("Drop links here").can_target(false).visible(false).build();
    drop_note.add_css_class("hdm-drop");
    let drop_overlay = gtk::Overlay::builder().child(&split).build();
    drop_overlay.add_overlay(&drop_note);
    drop_note.set_hexpand(true);
    drop_note.set_vexpand(true);
    let css = gtk::CssProvider::new();
    css.load_from_string(".hdm-drop { background-color: rgba(0,0,0,0.45); color: white; font-size: 24pt; font-weight: 600; }");
    gtk::style_context_add_provider_for_display(&gdk::Display::default().expect("a display"), &css,
                                                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&drop_overlay));
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
        text: RefCell::default(),
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
            for ((label, kinds), f) in counts.iter().zip(&kind_counts).zip(Filter::ALL) {
                label.set_label(&rows.iter().filter(|r| f.matches(&r.status)).count().to_string());
                for (number, k) in kinds.iter().zip(FileKind::ALL) {
                    number.set_label(&rows.iter().filter(|r| f.matches(&r.status) && r.kind == k).count().to_string());
                }
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
            enable(&window, &ui, &selected, &rows);
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
    // Links dropped on an unfinished download become its mirrors (§7.1 M4), said in the footer.
    {
        let ctx2 = ctx.clone();
        *ui.list.on_drop.borrow_mut() = Some(Rc::new(move |id: u64, links: Vec<String>| {
            // A finished download takes no mirrors: the links open Add Links, as a drop elsewhere does.
            if ctx2.manager.rows().iter().any(|r| r.id == id && r.status == Status::Finished) {
                crate::add::add_links(&ctx2, links);
                return;
            }
            let added = ctx2.manager.add_mirrors(id, &links);
            let name = ctx2.manager.rows().into_iter().find(|r| r.id == id).map(|r| r.name).unwrap_or_else(|| "the download".into());
            (ctx2.tell)(match added {
                0 => format!("“{name}” already uses {}", if links.len() == 1 { "that link" } else { "those links" }),
                1 => format!("Added 1 mirror to “{name}”"),
                n => format!("Added {n} mirrors to “{name}”"),
            });
            (ctx2.refresh)();
        }));
    }
    add_actions(&window, app, &ui, &refresh, &find_bar, &search, &sidebar_scroll, &view_menu, &ctx);
    // Closing the window leaves the app running, so downloads carry on (§2), as the Mac's Dock keeps it: opening the
    // app again brings the window back. Quit asks first while downloads run.
    std::mem::forget(app.hold());
    // Its icon in the status area meanwhile, as on Windows: a click, or Show, brings the window back (§2).
    crate::tray::follow(&window);
    {
        let (split, sidebar_scroll, list) = (split.clone(), sidebar_scroll.clone(), ui.list.clone());
        let save = move |w: &adw::ApplicationWindow| save_layout(w, &split, &sidebar_scroll, &list);
        let save2 = save.clone();
        window.connect_close_request(move |w| {
            save2(w);
            w.set_visible(false);
            glib::Propagation::Stop
        });
        let w = window.clone();
        app.connect_shutdown(move |_| if w.is_visible() { save(&w) });
    }
    load_layout(&window, &split, &sidebar_scroll, &ui.list, &view_menu);
    // The window's shortcuts reach it before the focused field: while a text field (the find bar's, or one in a
    // question over the window, such as Note or Rename) has the cursor, the Edit menu acts on it and Delete and F2
    // stay the field's.
    {
        let (ui, refresh) = (ui.clone(), refresh.clone());
        window.connect_focus_widget_notify(move |w| {
            let focus = gtk::prelude::GtkWindowExt::focus(w);
            match focus {
                Some(f) if f.is::<gtk::Text>() => *ui.text.borrow_mut() = f.downcast::<gtk::Text>().ok().map(|t| t.downgrade()),
                Some(f) if f.ancestor(gtk::Popover::static_type()).is_some() => {}
                _ => *ui.text.borrow_mut() = None,
            }
            let refresh = refresh.clone();
            glib::idle_add_local_once(move || refresh());
        });
    }
    crate::features::install(&ctx, &ui.list);
    CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
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
    // S16, Watch the clipboard: links copied in other apps open Add Links; copies made in this app are left alone.
    {
        // GTK tells of the clipboard's contents when it first learns them too: the text seen last is kept (from the
        // start), and only different text counts as copied.
        let seen: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        {
            let (seen, clipboard) = (seen.clone(), window.clipboard());
            glib::spawn_future_local(async move {
                let text = crate::add::clipboard_text(&clipboard).await;
                seen.borrow_mut().get_or_insert(text);
            });
        }
        let ctx2 = ctx.clone();
        window.clipboard().connect_changed(move |clipboard| {
            let (ctx, clipboard, seen) = (ctx2.clone(), clipboard.clone(), seen.clone());
            glib::spawn_future_local(async move {
                let text = crate::add::clipboard_text(&clipboard).await;
                let new = seen.borrow().as_deref() != Some(text.as_str());
                *seen.borrow_mut() = Some(text.clone());
                // Copies made in this app (its own, or while any of its windows is in front, as the Mac's "app is
                // active") are left alone: Copy Link in a Download window opened Add Links.
                let ours = clipboard.is_local()
                    || ctx.window.application().is_some_and(|a| a.windows().iter().any(|w| w.is_active()));
                if !new || !ctx.manager.settings().watch_clipboard || ours {
                    return;
                }
                let links = hd_core::links::find(&text);
                if !links.is_empty() {
                    crate::add::add_links(&ctx, links);
                }
            });
        });
    }
    // Links, text and link files dropped on the window open Add Links (§5.4); while they're over it, it dims.
    {
        let (ctx2, note) = (ctx.clone(), drop_note.clone());
        let target = crate::add::drop_target(&ctx, move |on| note.set_visible(on), move |links| crate::add::add_links(&ctx2, links));
        view.add_controller(target);
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
    // Double-click (§2.2): a finished download opens its file; an unfinished one, or one whose file is gone, its
    // Download window.
    {
        let (ui, ctx) = (ui.clone(), ctx.clone());
        ui.list.view.clone().connect_activate(move |_, position| {
            let Some(r) = ui.list.row_at(position) else { return };
            match (&r.status, &r.path) {
                (Status::Finished, Some(path)) if path.exists() => {
                    let _ = gio::AppInfo::launch_default_for_uri(&gio::File::for_path(path).uri(), None::<&gio::AppLaunchContext>);
                }
                _ => crate::detail::open(&ctx, r.id, None),
            }
        });
    }
    // Right-click on a download (§4.1): a download that isn't selected becomes the selection first; empty space
    // gets no menu.
    {
        let popover = gtk::PopoverMenu::from_model(None::<&gio::MenuModel>);
        popover.set_parent(&ui.list.view);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);
        let (ui2, refresh2) = (ui.clone(), refresh.clone());
        *ui.list.on_menu.borrow_mut() = Some(Rc::new(move |id: u64, x: f64, y: f64| {
            if !ui2.list.selected().iter().any(|r| r.id == id) {
                ui2.list.select(&[id]);
            }
            refresh2();
            popover.set_menu_model(Some(&context_menu(&ui2.list.selected())));
            popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            popover.popup();
        }));
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

fn can_resume(r: &Row) -> bool {
    matches!(r.status, Status::Waiting | Status::Paused | Status::Failed(_))
}

/// The text field the Edit menu acts on, while it's still on screen.
fn text_field(ui: &Ui) -> Option<gtk::Text> {
    ui.text.borrow().as_ref().and_then(|w| w.upgrade()).filter(|t| t.is_mapped())
}

/// Enables each command as the Mac's window does (§3: buttons that don't apply are disabled).
fn enable(window: &adw::ApplicationWindow, ui: &Ui, selected: &[Row], rows: &[Row]) {
    let set = |name: &str, on: bool| {
        if let Some(a) = window.lookup_action(name).and_downcast::<gio::SimpleAction>() {
            a.set_enabled(on);
        }
    };
    let queue_running = ui.manager.queue_running();
    let typing = text_field(ui).is_some();
    // With nothing selected, Resume starts what is waiting in the queue (oldest first).
    set("resume", if selected.is_empty() {
        !queue_running && rows.iter().any(|r| r.status == Status::Waiting)
    } else {
        selected.iter().any(can_resume)
    });
    set("pause", selected.iter().any(|r| r.status.is_running()));
    set("restart", !selected.is_empty());
    set("stop-all", rows.iter().any(|r| r.status.is_running()) || queue_running);
    set("start-queue", !queue_running);
    set("stop-queue", queue_running);
    set("delete", !selected.is_empty() && !typing);
    set("copy-link", !selected.is_empty());
    // Open, Show in Folder, Rename and Move To need one download whose file is on disk.
    let on_disk = selected.len() == 1 && selected[0].path.as_ref().is_some_and(|p| p.exists());
    set("open", on_disk);
    set("open-with", on_disk);
    set("open-with-chooser", on_disk);
    set("show-in-folder", on_disk);
    set("rename-file", on_disk && !typing);
    set("move-file", on_disk);
    set("note", selected.len() == 1);
    let unfinished = selected.len() == 1 && selected[0].status != Status::Finished;
    set("add-connection", unfinished && selected[0].allowed_connections < 32);
    set("remove-connection", unfinished && selected[0].allowed_connections > 1);
    set("delete-finished", rows.iter().any(|r| r.status == Status::Finished));
    set("delete-unfinished", rows.iter().any(|r| r.status != Status::Finished));
    set("delete-missing", rows.iter().any(|r| r.status == Status::Finished && !r.path.as_ref().is_some_and(|p| p.exists())));
    set("delete-all", !rows.is_empty());
    set("export-links", !rows.is_empty());
    set("add-mirror", selected.len() == 1);
    set("properties", !selected.is_empty());
    set("show-download-window", !selected.is_empty());
    for name in ["undo", "redo", "cut", "copy"] {
        set(name, typing);
    }
}

/// Pausing a download no source can resume asks first, Cancel the default (§10).
fn confirm_pause(window: &adw::ApplicationWindow, rows: Vec<Row>, pause: impl Fn() + 'static) {
    let lost: Vec<&Row> = rows.iter().filter(|r| !r.resumable).collect();
    if lost.is_empty() || !crate::signins::ASKS.get() {
        return pause();
    }
    let heading = if lost.len() == 1 && rows.len() == 1 {
        "This download can't be resumed and will start over. Pause anyway?".to_string()
    } else if lost.len() == 1 {
        format!("“{}” can't be resumed and will start over. Pause anyway?", lost[0].name)
    } else {
        format!("{} of these downloads can't be resumed and will start over. Pause anyway?", lost.len())
    };
    let d = adw::AlertDialog::new(Some(&heading), None);
    d.add_response("cancel", "Cancel");
    d.add_response("pause", "Pause");
    d.set_default_response(Some("cancel"));
    d.set_close_response("cancel");
    d.connect_response(Some("pause"), move |_, _| pause());
    d.present(Some(window));
}

/// Pause for the Download window (§6), with the same question.
pub fn pause_asking(window: &impl IsA<gtk::Widget>, manager: &Manager, id: u64) {
    let Some(r) = manager.rows().into_iter().find(|r| r.id == id) else { return };
    let manager = manager.clone();
    let ask = |heading: &str| {
        let d = adw::AlertDialog::new(Some(heading), None);
        d.add_response("cancel", "Cancel");
        d.add_response("pause", "Pause");
        d.set_default_response(Some("cancel"));
        d.set_close_response("cancel");
        d
    };
    if r.resumable || !crate::signins::ASKS.get() {
        manager.pause(&[id]);
        return;
    }
    let d = ask("This download can't be resumed and will start over. Pause anyway?");
    d.connect_response(Some("pause"), move |_, _| manager.pause(&[id]));
    d.present(Some(window));
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
    view_menu: &gio::Menu,
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
        let selected = ui.list.selected();
        if selected.is_empty() {
            ui.manager.start_queue()
        } else {
            ui.manager.resume(&selected.iter().filter(|r| can_resume(r)).map(|r| r.id).collect::<Vec<_>>())
        }
    });
    {
        let (ui, refresh, w) = (ui.clone(), refresh.clone(), window.clone());
        add("pause", &["<Control>period"], Box::new(move || {
            let rows: Vec<Row> = ui.list.selected().into_iter().filter(|r| r.status.is_running()).collect();
            let ids: Vec<u64> = rows.iter().map(|r| r.id).collect();
            let (ui, refresh) = (ui.clone(), refresh.clone());
            confirm_pause(&w, rows, move || {
                ui.manager.pause(&ids);
                refresh();
            });
        }));
    }
    {
        let (ui, refresh, w) = (ui.clone(), refresh.clone(), window.clone());
        add("stop-all", &["<Control><Alt>period"], Box::new(move || {
            let rows: Vec<Row> = ui.manager.rows().into_iter().filter(|r| r.status.is_running()).collect();
            let (ui, refresh) = (ui.clone(), refresh.clone());
            confirm_pause(&w, rows, move || {
                ui.manager.pause_all();
                refresh();
            });
        }));
    }
    // Restart discards the downloaded data after asking, Cancel the default (§10).
    {
        let (ui, refresh, w) = (ui.clone(), refresh.clone(), window.clone());
        add("restart", &[], Box::new(move || {
            let rows = ui.list.selected();
            if rows.is_empty() {
                return;
            }
            let heading = if rows.len() == 1 { format!("Restart “{}”?", rows[0].name) } else { format!("Restart {} downloads?", rows.len()) };
            let d = adw::AlertDialog::new(Some(&heading),
                                          Some("What was downloaded is discarded and the download starts again from the beginning."));
            d.add_response("cancel", "Cancel");
            d.add_response("restart", "Restart");
            d.set_response_appearance("restart", adw::ResponseAppearance::Destructive);
            d.set_default_response(Some("cancel"));
            d.set_close_response("cancel");
            let (ui, refresh) = (ui.clone(), refresh.clone());
            let ids: Vec<u64> = rows.iter().map(|r| r.id).collect();
            d.connect_response(Some("restart"), move |_, _| {
                ui.manager.restart(&ids);
                refresh();
            });
            d.present(Some(&w));
        }));
    }
    on!("start-queue", &["<Control><Shift>r"], |ui| ui.manager.start_queue());
    on!("stop-queue", &[], |ui| ui.manager.stop_queue());
    on!("add-connection", &[], |ui| for r in ui.list.selected() {
        ui.manager.set_connections(r.id, (r.allowed_connections + 1).min(32));
    });
    on!("remove-connection", &[], |ui| for r in ui.list.selected() {
        ui.manager.set_connections(r.id, r.allowed_connections.saturating_sub(1).max(1));
    });
    // Edit (§4): Select All, Undo, Redo, Cut, Copy and Paste act on the text field with the cursor; with none,
    // Select All selects every download and Paste adds the clipboard's links (§5.2).
    {
        let ui2 = ui.clone();
        add("select-all", &["<Control>a"], Box::new(move || match text_field(&ui2) {
            Some(t) => {
                t.grab_focus();
                t.select_region(0, -1);
            }
            None => {
                ui2.list.selection.select_all();
            }
        }));
        for (name, text_action) in [("undo", "text.undo"), ("redo", "text.redo"), ("cut", "clipboard.cut"), ("copy", "clipboard.copy")] {
            let ui2 = ui.clone();
            add(name, &[], Box::new(move || {
                if let Some(t) = text_field(&ui2) {
                    t.grab_focus();
                    let _ = t.activate_action(text_action, None);
                }
            }));
        }
        let (ui2, ctx2) = (ui.clone(), ctx.clone());
        add("paste", &["<Control>v"], Box::new(move || match text_field(&ui2) {
            Some(t) => {
                t.grab_focus();
                let _ = t.activate_action("clipboard.paste", None);
            }
            None => crate::add::from_clipboard(&ctx2),
        }));
    }
    // Copy Link (the context menu's): the selected downloads' links.
    {
        let (ui, window2) = (ui.clone(), window.clone());
        add("copy-link", &[], Box::new(move || {
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
            if let Some(path) = ui.list.selected().first().and_then(|r| r.path.clone()).filter(|p| p.exists()) {
                gtk::FileLauncher::new(Some(&gio::File::for_path(path))).open_containing_folder(
                    Some(&window2), None::<&gio::Cancellable>, |_| {});
            }
        }));
    }
    {
        let (ui, window2) = (ui.clone(), window.clone());
        add("open-with-chooser", &[], Box::new(move || {
            if let Some(path) = ui.list.selected().first().and_then(|r| r.path.clone()).filter(|p| p.exists()) {
                let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
                launcher.set_always_ask(true);
                launcher.launch(Some(&window2), None::<&gio::Cancellable>, |_| {});
            }
        }));
    }
    // Open With ▸ an app (§4.1): the item carries the app's desktop id.
    {
        let ui = ui.clone();
        let action = gio::SimpleAction::new("open-with", Some(glib::VariantTy::STRING));
        action.connect_activate(move |_, id| {
            let Some(id) = id.and_then(|v| v.get::<String>()) else { return };
            let Some(path) = ui.list.selected().first().and_then(|r| r.path.clone()) else { return };
            if let Some(app) = gio::AppInfo::all().into_iter().find(|a| a.id().as_deref() == Some(id.as_str())) {
                let _ = app.launch(&[gio::File::for_path(path)], None::<&gio::AppLaunchContext>);
            }
        });
        window.add_action(&action);
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
            let picker = gtk::FileDialog::builder().title(format!("Choose where “{}” goes", r.name)).accept_label("Move Here").build();
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
        // The toolbar's Search opens or closes the find bar, as on the Mac.
        let (find_bar, search) = (find_bar.clone(), search.clone());
        add("toggle-search", &[], Box::new(move || {
            if find_bar.is_visible() {
                search.set_text("");
                find_bar.set_visible(false);
            } else {
                find_bar.set_visible(true);
                search.grab_focus();
            }
        }));
    }
    {
        let (sidebar, view_menu) = (sidebar.clone(), view_menu.clone());
        add("show-sidebar", &["<Control><Super>s"], Box::new(move || {
            sidebar.set_visible(!sidebar.is_visible());
            sidebar_title(&view_menu, sidebar.is_visible());
        }));
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
        // Bring All to Front: every window of the app that is open, the one in front last so it stays in front.
        let w = window.clone();
        add("bring-all-to-front", &[], Box::new(move || {
            let Some(app) = w.application() else { return };
            let windows: Vec<gtk::Window> = app.windows().into_iter().filter(|x| x.is_visible()).collect();
            let front = app.active_window();
            for x in windows.iter().rev().filter(|x| Some(*x) != front.as_ref()) {
                x.present();
            }
            if let Some(front) = front.filter(|f| f.is_visible()) {
                front.present();
            }
        }));
        // Quit asks first while downloads run, Cancel the default (§10); they start again at the next launch (the
        // application's shutdown calls the manager's quit).
        let (w, manager) = (window.clone(), ui.manager.clone());
        add("quit", &["<Control>q"], Box::new(move || {
            let Some(app) = w.application() else { return };
            if !manager.any_running() || !crate::signins::ASKS.get() {
                return app.quit();
            }
            w.present();
            let d = adw::AlertDialog::new(Some("Downloads are running. Pause them and quit?"),
                                          Some("They continue the next time you open Herb's Download Manager."));
            d.add_response("cancel", "Cancel");
            d.add_response("quit", "Pause and Quit");
            d.set_default_response(Some("cancel"));
            d.set_close_response("cancel");
            d.connect_response(Some("quit"), move |_, _| app.quit());
            d.present(Some(&w));
        }));
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
                .comments(format!("A download manager\nMany connections · Mirrors · Streams\n\ncore {}", hd_core::version()))
                .website(GITHUB)
                .license_type(gtk::License::MitX11)
                .build();
            about.present(Some(&w));
        }));
    }
    for (name, accels, run) in [
        ("new-download", &["<Control>n"][..], crate::add::new_download_from_clipboard as fn(&crate::add::Ctx)),
        ("import-links", &["<Control><Shift>o"], crate::add::import),
        // Ctrl+V in the main window does the same, through Edit ▸ Paste, when no text field has the cursor.
        ("add-from-clipboard", &["<Control><Shift>v"], crate::add::from_clipboard),
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
            let text = if kept.len() == 1 {
                format!("“{name}” couldn't go to the Trash, so it was left where it is")
            } else {
                format!("{} files couldn't go to the Trash, so they were left where they are", kept.len())
            };
            *ui.notice.borrow_mut() = Some((text, std::time::Instant::now()));
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
    /// What the dialogs need, for files handed to the app (`open_files`).
    static CTX: RefCell<Option<crate::add::Ctx>> = const { RefCell::new(None) };
}

/// Files handed to the app (Open With in the file manager, or dropped on its icon): the links in them open Add Links,
/// as on the Mac (§5.4, §5.7). Files with none are passed over.
pub fn open_files(files: &[gio::File]) {
    let Some(ctx) = CTX.with(|c| c.borrow().clone()) else { return };
    let mut text = String::new();
    for f in files {
        text.push('\n');
        match f.path() {
            Some(p) => text.push_str(&hd_core::links::text_of_file(&p).unwrap_or_default()),
            None => text.push_str(&f.uri()),
        }
    }
    let links = hd_core::links::find(&text);
    if !links.is_empty() {
        crate::add::add_links(&ctx, links);
    }
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


/// The sidebar's symbol for a file type (§2.1), after the Mac's.
fn kind_icon(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Compressed => "package-x-generic-symbolic",
        FileKind::Documents => "x-office-document-symbolic",
        FileKind::Music => "audio-x-generic-symbolic",
        FileKind::Pictures => "image-x-generic-symbolic",
        FileKind::Video => "video-x-generic-symbolic",
        FileKind::Programs => "application-x-executable-symbolic",
        FileKind::Other => "text-x-generic-symbolic",
    }
}

/// View's first item says what it will do: Hide Sidebar while it shows, Show Sidebar while it's hidden.
fn sidebar_title(view_menu: &gio::Menu, shown: bool) {
    view_menu.remove(0);
    view_menu.insert(0, Some(if shown { "Hide Sidebar" } else { "Show Sidebar" }), Some("win.show-sidebar"));
}

fn layout_file() -> std::path::PathBuf {
    crate::data_folder().join("window-layout.txt")
}

/// The window's size, the sidebar's width and whether it shows, the columns' widths, order and which show, and the
/// sort, remembered as the Mac does (§2). A test copy keeps its own, in its data folder.
fn save_layout(window: &adw::ApplicationWindow, split: &gtk::Paned, sidebar: &gtk::ScrolledWindow, list: &DownloadList) {
    let (w, h) = window.default_size();
    let text = format!("size {w} {h}\nmaximized {}\nsidebar {} {}\n{}", window.is_maximized() as u8, split.position(),
                       sidebar.is_visible() as u8, list.layout_text());
    let _ = std::fs::write(layout_file(), text);
}

fn load_layout(window: &adw::ApplicationWindow, split: &gtk::Paned, sidebar: &gtk::ScrolledWindow, list: &DownloadList,
               view_menu: &gio::Menu) {
    let Ok(text) = std::fs::read_to_string(layout_file()) else { return };
    // `--window-size` (a test hook) wins over the remembered size.
    let sized = std::env::args().any(|a| a == "--window-size");
    for line in text.lines() {
        let parts: Vec<&str> = line.split(' ').collect();
        match parts.as_slice() {
            ["size", w, h] if !sized => {
                if let (Ok(w), Ok(h)) = (w.parse::<i32>(), h.parse::<i32>()) && w >= 600 && h >= 300 {
                    window.set_default_size(w, h);
                }
            }
            ["maximized", "1"] if !sized => window.maximize(),
            ["sidebar", width, shown] => {
                if let Ok(width) = width.parse::<i32>() {
                    split.set_position(width.clamp(190, 360));
                }
                sidebar.set_visible(*shown == "1");
                sidebar_title(view_menu, *shown == "1");
            }
            _ => {}
        }
    }
    list.apply_layout(&text);
    for (key, _, _) in COLUMNS.iter().skip(1) {
        if let (Some(action), Some(column)) = (window.lookup_action(&format!("column-{key}")), list.column(key)) {
            action.change_state(&column.is_visible().to_variant());
        }
    }
}
