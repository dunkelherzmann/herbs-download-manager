//! Adding downloads (§5): New Download (Ctrl+N), Add Links (several links, from the clipboard, a drop, Import or
//! Batch), Batch Download (Ctrl+Shift+N) and Import Links (Ctrl+Shift+O). Every add goes through the core's
//! `AddSession`, which gathers mirrors and asks about a name in use (§5.6) with the dialog drawn here.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use hd_core::addflow::{AddSession, Choice, PendingLink, Step};
use hd_core::format;
use hd_core::manager::Manager;

/// What the dialogs need from the main window.
#[derive(Clone)]
pub struct Ctx {
    pub manager: Manager,
    pub window: adw::ApplicationWindow,
    /// A note in the footer for a few seconds.
    pub tell: Rc<dyn Fn(String)>,
    pub refresh: Rc<dyn Fn()>,
    /// The Add Links window while it is open: later drops join it (§5.3).
    pub open_links: Rc<RefCell<Option<AddLinks>>>,
}

/// Adds the links, asking about names in use, then sums up in the footer.
pub fn add_all(ctx: &Ctx, links: Vec<PendingLink>, folder: PathBuf, connections: u32, start: bool) {
    let ctx = ctx.clone();
    crate::features::PENDING_ADDS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    glib::spawn_future_local(async move {
        let mut session = AddSession::new(&ctx.manager, links, &folder, connections, start);
        loop {
            match session.step(&ctx.manager) {
                Step::Done(summary) => {
                    if let Some(text) = summary.text() {
                        (ctx.tell)(text);
                    }
                    break;
                }
                Step::Ask(q) => {
                    let dialog = adw::AlertDialog::new(Some(&q.title()), Some(&q.body()));
                    dialog.add_response("skip", "Skip");
                    dialog.add_response("replace", "Replace");
                    dialog.add_response("keep", "Keep Both");
                    dialog.set_response_appearance("replace", adw::ResponseAppearance::Destructive);
                    dialog.set_response_appearance("keep", adw::ResponseAppearance::Suggested);
                    dialog.set_default_response(Some("keep"));
                    dialog.set_close_response("skip");
                    let rest = gtk::CheckButton::with_label("Do the same for the rest");
                    if q.more {
                        dialog.set_extra_child(Some(&rest));
                    }
                    let answer = dialog.choose_future(Some(&ctx.window)).await;
                    let choice = match answer.as_str() {
                        "replace" => Choice::Replace,
                        "keep" => Choice::KeepBoth,
                        _ => Choice::Skip,
                    };
                    session.answer(&ctx.manager, choice, rest.is_active());
                }
            }
        }
        (ctx.refresh)();
        crate::features::PENDING_ADDS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    });
}

fn label(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(1.0).build()
}

/// A folder row: the path and Choose….
fn folder_row(window: &impl IsA<gtk::Window>, folder: Rc<RefCell<PathBuf>>) -> gtk::Box {
    let path = gtk::Label::builder().label(folder.borrow().display().to_string()).xalign(0.0).hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::Middle).build();
    let choose = gtk::Button::builder().label("Choose…").tooltip_text("Choose the folder the file is saved to").build();
    let window = window.clone().upcast::<gtk::Window>();
    let path2 = path.clone();
    choose.connect_clicked(move |_| {
        let dialog = gtk::FileDialog::builder().title("Save to").initial_folder(&gio::File::for_path(&*folder.borrow())).build();
        let (folder, path) = (folder.clone(), path2.clone());
        dialog.select_folder(Some(&window), None::<&gio::Cancellable>, move |result| {
            if let Ok(dir) = result
                && let Some(p) = dir.path()
            {
                path.set_label(&p.display().to_string());
                *folder.borrow_mut() = p;
            }
        });
    });
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&path);
    row.append(&choose);
    row
}

fn buttons(cancel: &gtk::Button, add: &gtk::Button, download: &gtk::Button) -> gtk::Box {
    download.add_css_class("suggested-action");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_halign(gtk::Align::End);
    row.append(cancel);
    row.append(add);
    row.append(download);
    cancel.set_tooltip_text(Some("Close without adding anything (Esc)"));
    add.set_tooltip_text(Some("Add: put it in the queue; it starts when the queue runs"));
    download.set_tooltip_text(Some("Download: start now"));
    row
}

fn dialog_window(ctx: &Ctx, title: &str, width: i32, height: i32) -> adw::Window {
    adw::Window::builder()
        .title(title)
        .transient_for(&ctx.window)
        .modal(false)
        .default_width(width)
        .default_height(height)
        .build()
}

fn with_header(window: &adw::Window, body: &impl IsA<gtk::Widget>) {
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(body));
    window.set_content(Some(&view));
}

/// New Download (§5.1).
pub fn new_download(ctx: &Ctx, link: Option<String>) {
    let window = dialog_window(ctx, "New Download", 560, 420);
    let link_entry = gtk::Entry::builder().hexpand(true).placeholder_text("https://…").build();
    crate::explain(&link_entry, "Link", "The address of the file to download");
    if let Some(l) = link {
        link_entry.set_text(&l);
    }
    let mirrors = gtk::TextView::builder().wrap_mode(gtk::WrapMode::None).build();
    crate::explain(&mirrors, "Mirrors", "Other servers with the same file, one link per line (optional)");
    mirrors.set_tooltip_text(Some("Other servers with the same file, one link per line (optional)"));
    let mirrors_scroll = gtk::ScrolledWindow::builder().child(&mirrors).min_content_height(70).has_frame(true).build();
    let name = gtk::Entry::builder().hexpand(true).build();
    crate::explain(&name, "Name", "The file's name, from the server; change it to save under another name");
    let folder = Rc::new(RefCell::new(crate::save_folder(&ctx.manager)));
    let connections = gtk::SpinButton::with_range(1.0, 32.0, 1.0);
    crate::explain(&connections, "Connections", "Connections for this download (1–32)");
    connections.set_value(ctx.manager.settings().connections as f64);
    connections.set_tooltip_text(Some("Connections for this download (1–32)"));
    let size = gtk::Label::builder().xalign(0.0).label("").build();
    size.add_css_class("dim-label");

    let grid = gtk::Grid::builder().row_spacing(10).column_spacing(10).margin_top(16).margin_bottom(16).margin_start(16).margin_end(16).build();
    for (row, (title, widget)) in [
        ("Link:", link_entry.clone().upcast::<gtk::Widget>()),
        ("Mirrors:", mirrors_scroll.upcast()),
        ("Name:", name.clone().upcast()),
        ("Save to:", folder_row(&window, folder.clone()).upcast()),
        ("Connections:", connections.clone().upcast()),
        ("Size:", size.clone().upcast()),
    ]
    .into_iter()
    .enumerate()
    {
        grid.attach(&label(title), 0, row as i32, 1, 1);
        grid.attach(&widget, 1, row as i32, 1, 1);
    }
    let (cancel, add, download) = (gtk::Button::with_label("Cancel"), gtk::Button::with_label("Add"), gtk::Button::with_label("Download"));
    grid.attach(&buttons(&cancel, &add, &download), 0, 6, 2, 1);
    with_header(&window, &grid);

    // A6: ask the server about the link once it stops changing.
    let probed: Rc<RefCell<Option<u64>>> = Rc::default();
    // The link is a web page: Download and Add open Add Links with the files found on it instead (§15).
    let page: Rc<Cell<bool>> = Rc::default();
    let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    // The download started while this window is open (enhancements.md), once the link proved resumable: Download or
    // Add adopts it, Cancel discards it.
    let early: Rc<Early> = Rc::new(Early { manager: ctx.manager.clone(), token: Cell::new(None), timer: RefCell::new(None) });
    {
        let (manager, name, size, probed, pending) = (ctx.manager.clone(), name.clone(), size.clone(), probed.clone(), pending.clone());
        let folder = folder.clone();
        let early = early.clone();
        let page = page.clone();
        link_entry.connect_changed(move |entry| {
            page.set(false);
            let page = page.clone();
            if let Some(id) = pending.borrow_mut().take() {
                id.remove();
            }
            early.drop_it();
            let early = early.clone();
            let url = entry.text().trim().to_string();
            let (manager, name, size, probed, pending2) = (manager.clone(), name.clone(), size.clone(), probed.clone(), pending.clone());
            let folder = folder.clone();
            *pending.borrow_mut() = Some(glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
                pending2.borrow_mut().take();
                if hd_core::links::find(&url).is_empty() {
                    size.set_label("");
                    return;
                }
                size.set_label("Checking…");
                let url2 = url.clone();
                glib::spawn_future_local(async move {
                    let result = gio::spawn_blocking(move || manager.probe(&url)).await;
                    match result {
                        Ok(Ok(p)) if p.page => {
                            page.set(true);
                            size.set_label("A web page: Download lists the files and media on it");
                        }
                        Ok(Ok(p)) => {
                            if name.text().is_empty() {
                                name.set_text(&p.name);
                            }
                            *probed.borrow_mut() = p.size;
                            let mut text = match (p.size, p.resumable) {
                                (Some(s), true) => format!("{}, resumable", format::bytes(s)),
                                (Some(s), false) => format!("{}, can't resume", format::bytes(s)),
                                (None, _) => "size unknown, can't resume".into(),
                            };
                            // Said now rather than when the download fails (free space, FAT32's 4 GB limit).
                            if let Some(problem) = p.size.and_then(|s| hd_core::disk::space_problem(&folder.borrow(), s)) {
                                text = format!("{text} — {problem}");
                            } else if p.resumable {
                                early.start(&url2, &folder.borrow(), &p.name, &size, text.clone());
                            }
                            size.set_label(&text);
                        }
                        Ok(Err(e)) => size.set_label(&e),
                        Err(_) => size.set_label(""),
                    }
                });
            }));
        });
    }
    link_entry.emit_by_name::<()>("changed", &[]);

    let early_for_close = early.clone();
    let submit = {
        let (ctx, window) = (ctx.clone(), window.clone());
        move |start: bool| {
            let url = link_entry.text().trim().to_string();
            if hd_core::links::find(&url).is_empty() {
                size.set_label("Enter a web link (http, https or ftp)");
                link_entry.grab_focus();
                return;
            }
            if page.get() {
                // Its files, listed to tick, in Add Links.
                window.close();
                add_links(&ctx, vec![url]);
                return;
            }
            let buffer = mirrors.buffer();
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
            let link = PendingLink {
                url,
                mirrors: hd_core::links::find(&text),
                name: Some(name.text().trim().to_string()).filter(|n| !n.is_empty()),
                size: *probed.borrow(),
                // The download already under way is adopted (moved to the chosen folder and name), not started again.
                early: early.take(),
                stream: false,
            };
            add_all(&ctx, vec![link], folder.borrow().clone(), connections.value() as u32, start);
            window.close();
        }
    };
    let w = window.clone();
    cancel.connect_clicked(move |_| w.close());
    // Closed without adding: what was fetched is discarded.
    window.connect_destroy(move |_| early_for_close.drop_it());
    let s = submit.clone();
    add.connect_clicked(move |_| s(false));
    download.connect_clicked(move |_| submit(true));
    window.set_default_widget(Some(&download));
    window.present();
}

/// One row of Add Links.
struct Item {
    url: String,
    name: String,
    size: Option<u64>,
    size_text: String,
    check: gtk::CheckButton,
    row: gtk::ListBoxRow,
    name_label: gtk::Label,
    size_label: gtk::Label,
    /// A stream a page scan found: added as one (§15).
    stream: bool,
}

/// Add Links (§5.3): the links, each with a checkbox, name, size and link; a filter; Check All and Uncheck All;
/// "Treat as mirrors of one file"; Save to; Cancel, Add, Download.
#[derive(Clone)]
pub struct AddLinks {
    window: adw::Window,
    list: gtk::ListBox,
    items: Rc<RefCell<Vec<Item>>>,
    summary: gtk::Label,
    mirrors: gtk::CheckButton,
    manager: Manager,
}

impl AddLinks {
    /// Adds rows for links not listed yet and checks each with its server, four at a time.
    pub fn add(&self, links: Vec<String>) {
        let known: Vec<String> = self.items.borrow().iter().map(|i| i.url.clone()).collect();
        let mut fresh = Vec::new();
        for url in links {
            if known.contains(&url) || fresh.contains(&url) {
                continue;
            }
            fresh.push(url);
        }
        for url in &fresh {
            self.append_row(url, reqwest_name(url), None, "Checking…", false, true);
        }
        self.window.set_title(Some(&title(self.items.borrow().len())));
        self.update();
        let queue = Rc::new(RefCell::new(fresh));
        for _ in 0..4 {
            let (this, queue) = (self.clone(), queue.clone());
            glib::spawn_future_local(async move {
                loop {
                    let Some(url) = ({
                        let mut q = queue.borrow_mut();
                        if q.is_empty() { None } else { Some(q.remove(0)) }
                    }) else { break };
                    let manager = this.manager.clone();
                    let u = url.clone();
                    let result = gio::spawn_blocking(move || manager.probe(&u)).await;
                    // A web page: what's on it takes its place (§15).
                    if let Ok(Ok(p)) = &result
                        && p.page
                    {
                        this.scan_page(&url).await;
                        continue;
                    }
                    for item in this.items.borrow_mut().iter_mut().filter(|i| i.url == url) {
                        match &result {
                            Ok(Ok(p)) => {
                                item.name = p.name.clone();
                                item.size = p.size;
                                item.size_text = p.size.map(format::bytes).unwrap_or_else(|| "Unknown".into());
                            }
                            Ok(Err(e)) => item.size_text = e.clone(),
                            Err(_) => item.size_text = String::new(),
                        }
                        item.name_label.set_label(&item.name);
                        item.size_label.set_label(&item.size_text);
                    }
                    this.update();
                }
            });
        }
    }

    /// A row for one link, checked or not, with what's known of it so far.
    fn append_row(&self, url: &str, name: String, size: Option<u64>, size_text: &str, stream: bool, checked: bool) {
        let check = gtk::CheckButton::builder().active(checked).build();
        check.set_tooltip_text(Some("Download this link"));
        check.update_property(&[gtk::accessible::Property::Label(&format!("Download {name}"))]);
        let name_label = gtk::Label::builder().label(&name).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::Middle).build();
        let size_label = gtk::Label::builder().label(size_text).width_chars(14).xalign(0.0).build();
        let link = gtk::Label::builder().label(url).xalign(0.0).width_chars(30).max_width_chars(30).ellipsize(gtk::pango::EllipsizeMode::Middle).build();
        link.add_css_class("dim-label");
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        line.append(&check);
        line.append(&name_label);
        line.append(&size_label);
        line.append(&link);
        let row = gtk::ListBoxRow::builder().child(&line).build();
        self.list.append(&row);
        let this = self.clone();
        check.connect_toggled(move |_| this.update());
        self.items.borrow_mut().push(Item { url: url.to_string(), name, size, size_text: size_text.into(), check, row, name_label, size_label, stream });
    }

    /// Replaces a web page's row with the files and media found on it, each checked already; a page with nothing on
    /// it, or that couldn't be read, stays as an unchecked row that says so.
    async fn scan_page(&self, url: &str) {
        for item in self.items.borrow_mut().iter_mut().filter(|i| i.url == url) {
            item.size_text = "Scanning page…".into();
            item.size_label.set_label(&item.size_text);
        }
        let (manager, u) = (self.manager.clone(), url.to_string());
        let scanned = gio::spawn_blocking(move || manager.scan_page(&u)).await.unwrap_or_else(|_| Err("The scan stopped".into()));
        match scanned {
            Ok(scan) if !scan.found.is_empty() => {
                let page_row = self.items.borrow().iter().find(|i| i.url == url).map(|i| i.row.clone());
                if let Some(row) = page_row {
                    self.list.remove(&row);
                }
                self.items.borrow_mut().retain(|i| i.url != url);
                let known: Vec<String> = self.items.borrow().iter().map(|i| i.url.clone()).collect();
                for f in scan.found.into_iter().filter(|f| !known.contains(&f.url)) {
                    let text = match f.size {
                        Some(n) => format!("{}, {}", format::bytes(n), f.kind.word()),
                        None => f.kind.word().to_string(),
                    };
                    self.append_row(&f.url, f.name, f.size, &text, f.kind == hd_core::media::scan::Kind::Stream, true);
                }
                self.window.set_title(Some(&title(self.items.borrow().len())));
            }
            result => {
                let text = match result {
                    Ok(_) => "Web page: no files found".to_string(),
                    Err(e) => format!("Web page: {e}"),
                };
                for item in self.items.borrow_mut().iter_mut().filter(|i| i.url == url) {
                    item.check.set_active(false);
                    item.size_text = text.clone();
                    item.size_label.set_label(&text);
                }
            }
        }
        self.update();
    }

    fn checked(&self) -> Vec<PendingLink> {
        self.items
            .borrow()
            .iter()
            .filter(|i| i.check.is_active())
            .map(|i| PendingLink { url: i.url.clone(), name: Some(i.name.clone()), size: i.size, stream: i.stream, ..Default::default() })
            .collect()
    }

    /// "Treat as mirrors" is offered when the checked links share a name or a size; the line under the list says
    /// how they will be added.
    fn update(&self) {
        let checked = self.checked();
        let n = checked.len();
        let names: std::collections::HashSet<&String> = checked.iter().filter_map(|l| l.name.as_ref()).collect();
        let sizes: std::collections::HashSet<u64> = checked.iter().filter_map(|l| l.size).collect();
        let can = n > 1 && (names.len() == 1 || (sizes.len() == 1 && checked.iter().all(|l| l.size.is_some())));
        self.mirrors.set_sensitive(can);
        if !can {
            self.mirrors.set_active(false);
        }
        let files: std::collections::HashSet<String> =
            checked.iter().map(|l| format!("{}\u{0}{:?}", l.name.clone().unwrap_or_default(), l.size)).collect();
        self.summary.set_label(&if self.mirrors.is_active() {
            format!("1 download from {n} sources")
        } else if n == 1 {
            "1 link checked".into()
        } else if files.len() < n {
            format!("{n} links checked: {} download{} with mirrors", files.len(), if files.len() == 1 { "" } else { "s" })
        } else {
            format!("{n} links checked")
        });
    }
}

fn title(n: usize) -> String {
    if n == 1 { "Add 1 Link".into() } else { format!("Add {n} Links") }
}

fn reqwest_name(url: &str) -> String {
    url.trim_end_matches('/').rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(url).to_string()
}

/// Opens Add Links with the links, or adds them to the one already open.
pub fn add_links(ctx: &Ctx, links: Vec<String>) {
    if links.is_empty() {
        (ctx.tell)("No links found".into());
        return;
    }
    if let Some(open) = ctx.open_links.borrow().clone() {
        open.add(links);
        open.window.present();
        return;
    }
    let window = dialog_window(ctx, &title(links.len()), 760, 460);
    let filter = gtk::SearchEntry::builder().placeholder_text("Filter").hexpand(true).build();
    crate::explain(&filter, "Filter", "Show only links whose name or address contains this text");
    let all = gtk::Button::with_label("Check All");
    all.set_tooltip_text(Some("Tick every link shown"));
    let none = gtk::Button::with_label("Uncheck All");
    none.set_tooltip_text(Some("Untick every link shown"));
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    top.append(&filter);
    top.append(&all);
    top.append(&none);
    let list = gtk::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk::SelectionMode::None);
    let scroll = gtk::ScrolledWindow::builder().child(&list).vexpand(true).build();
    let mirrors = gtk::CheckButton::with_label("Treat as mirrors of one file");
    mirrors.set_tooltip_text(Some("Add the checked links as one download, each a source of the same file"));
    let folder = Rc::new(RefCell::new(crate::save_folder(&ctx.manager)));
    let save = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    save.append(&gtk::Label::new(Some("Save to:")));
    save.append(&folder_row(&window, folder.clone()));
    let summary = gtk::Label::builder().xalign(0.0).hexpand(true).build();
    summary.add_css_class("dim-label");
    let (cancel, add, download) = (gtk::Button::with_label("Cancel"), gtk::Button::with_label("Add"), gtk::Button::with_label("Download"));
    let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bottom.append(&summary);
    bottom.append(&buttons(&cancel, &add, &download));
    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    for m in [&top.clone().upcast::<gtk::Widget>(), &scroll.clone().upcast(), &mirrors.clone().upcast(), &save.clone().upcast(), &bottom.clone().upcast()] {
        body.append(m);
    }
    body.set_margin_top(12);
    body.set_margin_bottom(12);
    body.set_margin_start(12);
    body.set_margin_end(12);
    with_header(&window, &body);

    let this = AddLinks { window: window.clone(), list, items: Rc::default(), summary, mirrors: mirrors.clone(), manager: ctx.manager.clone() };
    {
        let this2 = this.clone();
        mirrors.connect_toggled(move |_| this2.update());
        let this2 = this.clone();
        all.connect_clicked(move |_| this2.items.borrow().iter().filter(|i| i.row.is_visible()).for_each(|i| i.check.set_active(true)));
        let this2 = this.clone();
        none.connect_clicked(move |_| this2.items.borrow().iter().filter(|i| i.row.is_visible()).for_each(|i| i.check.set_active(false)));
        let this2 = this.clone();
        filter.connect_search_changed(move |f| {
            let needle = f.text().to_lowercase();
            for i in this2.items.borrow().iter() {
                i.row.set_visible(needle.is_empty() || i.name.to_lowercase().contains(&needle) || i.url.to_lowercase().contains(&needle));
            }
        });
    }
    // Links dropped on the window join the list (§5.3).
    window.add_controller(drop_target(ctx, {
        let this = this.clone();
        move |links| this.add(links)
    }));
    let submit = {
        let (ctx, this) = (ctx.clone(), this.clone());
        move |start: bool| {
            let mut links = this.checked();
            if links.is_empty() {
                return;
            }
            if this.mirrors.is_active() {
                let first = links.remove(0);
                links = vec![PendingLink {
                    mirrors: links.iter().map(|l| l.url.clone()).collect(),
                    size: first.size.or(links.iter().find_map(|l| l.size)),
                    ..first
                }];
            }
            add_all(&ctx, links, folder.borrow().clone(), ctx.manager.settings().connections, start);
            this.window.close();
        }
    };
    let w = window.clone();
    cancel.connect_clicked(move |_| w.close());
    let s = submit.clone();
    add.connect_clicked(move |_| s(false));
    download.connect_clicked(move |_| submit(true));
    {
        let open = ctx.open_links.clone();
        window.connect_close_request(move |_| {
            open.borrow_mut().take();
            glib::Propagation::Proceed
        });
    }
    *ctx.open_links.borrow_mut() = Some(this.clone());
    this.add(links);
    window.present();
}

/// Links from a drop: link text, or the links inside dropped text files (§5.4, §5.7).
pub fn drop_target(ctx: &Ctx, on_links: impl Fn(Vec<String>) + 'static) -> gtk::DropTarget {
    let target = gtk::DropTarget::new(glib::Type::INVALID, gdk::DragAction::COPY);
    target.set_types(&[gdk::FileList::static_type(), glib::GString::static_type()]);
    let tell = ctx.tell.clone();
    target.connect_drop(move |_, value, _, _| {
        let mut text = String::new();
        if let Ok(files) = value.get::<gdk::FileList>() {
            for f in files.files() {
                if let Some(p) = f.path() {
                    text.push('\n');
                    text.push_str(&hd_core::links::text_of_file(&p).unwrap_or_default());
                } else {
                    text.push('\n');
                    text.push_str(&f.uri());
                }
            }
        } else if let Ok(s) = value.get::<String>() {
            text = s;
        }
        let links = hd_core::links::find(&text);
        if links.is_empty() {
            tell("No links found in what was dropped".into());
            return false;
        }
        on_links(links);
        true
    });
    target
}

/// Batch Download (§5.5): a link with a range placeholder, a preview, then Add Links.
pub fn batch(ctx: &Ctx) {
    let window = dialog_window(ctx, "Batch Download", 560, 220);
    let pattern = gtk::Entry::builder().hexpand(true).placeholder_text("https://example.com/img^(###1-100).jpg").build();
    crate::explain(&pattern, "Link pattern", "^(###1-100) counts with 3 digits; H after a range for hexadecimal; a-z for letters");
    pattern.set_tooltip_text(Some("^(###1-100) counts with 3 digits; H after a range for hexadecimal; a-z for letters"));
    let preview = gtk::Label::builder().xalign(0.0).wrap(true).build();
    preview.add_css_class("dim-label");
    let next = gtk::Button::with_label("Next");
    next.set_tooltip_text(Some("Show these links in Add Links"));
    next.add_css_class("suggested-action");
    next.set_sensitive(false);
    let cancel = gtk::Button::with_label("Cancel");
    cancel.set_tooltip_text(Some("Close without adding anything (Esc)"));
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_halign(gtk::Align::End);
    row.append(&cancel);
    row.append(&next);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    body.set_margin_top(16);
    body.set_margin_bottom(16);
    body.set_margin_start(16);
    body.set_margin_end(16);
    body.append(&pattern);
    body.append(&preview);
    body.append(&row);
    with_header(&window, &body);
    let links: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        let (links, next) = (links.clone(), next.clone());
        pattern.connect_changed(move |p| match hd_core::links::expand_batch(p.text().trim()) {
            Ok(l) if !l.is_empty() => {
                preview.set_label(&format!("{} links: {} … {}", l.len(), l[0], l[l.len() - 1]));
                next.set_sensitive(true);
                *links.borrow_mut() = l;
            }
            Ok(_) => preview.set_label(""),
            Err(e) => {
                preview.set_label(&e);
                next.set_sensitive(false);
            }
        });
    }
    let w = window.clone();
    cancel.connect_clicked(move |_| w.close());
    let (ctx2, w) = (ctx.clone(), window.clone());
    next.connect_clicked(move |_| {
        let l = links.borrow().clone();
        w.close();
        add_links(&ctx2, l);
    });
    window.set_default_widget(Some(&next));
    window.present();
}

/// Import Links (§5.7): the links in one or more text files.
pub fn import(ctx: &Ctx) {
    let dialog = gtk::FileDialog::builder().title("Import Links").accept_label("Import").build();
    let ctx = ctx.clone();
    dialog.open_multiple(Some(&ctx.window.clone()), None::<&gio::Cancellable>, move |result| {
        let Ok(files) = result else { return };
        let mut text = String::new();
        let mut unreadable = 0;
        for i in 0..files.n_items() {
            if let Some(path) = files.item(i).and_downcast::<gio::File>().and_then(|f| f.path()) {
                match hd_core::links::text_of_file(&path) {
                    Some(t) => {
                        text.push('\n');
                        text.push_str(&t);
                    }
                    None => unreadable += 1,
                }
            }
        }
        let links = hd_core::links::find(&text);
        if links.is_empty() {
            (ctx.tell)(if unreadable > 0 { "That file isn't text, so it has no links".into() } else { "No links found".into() });
            return;
        }
        add_links(&ctx, links);
    });
}

/// Add from Clipboard (§5.2).
pub fn from_clipboard(ctx: &Ctx) {
    let ctx = ctx.clone();
    glib::spawn_future_local(async move {
        let text = ctx.window.clipboard().read_text_future().await.ok().flatten().map(|t| t.to_string()).unwrap_or_default();
        add_links(&ctx, hd_core::links::find(&text));
    });
}

/// The first link in the clipboard, for New Download's Link field (§5.1 A1).
pub fn new_download_from_clipboard(ctx: &Ctx) {
    let ctx = ctx.clone();
    glib::spawn_future_local(async move {
        let text = ctx.window.clipboard().read_text_future().await.ok().flatten().map(|t| t.to_string()).unwrap_or_default();
        new_download(&ctx, hd_core::links::find(&text).into_iter().next());
    });
}

/// New Download's early download (enhancements.md: start while New Download is open).
struct Early {
    manager: Manager,
    token: Cell<Option<u64>>,
    timer: RefCell<Option<glib::SourceId>>,
}

impl Early {
    /// Starts fetching into the window's folder; the size line shows how much is in.
    fn start(&self, url: &str, folder: &std::path::Path, name: &str, size: &gtk::Label, base: String) {
        self.drop_it();
        let token = self.manager.start_early(url, &[], folder, Some(name));
        self.token.set(Some(token));
        let (manager, size) = (self.manager.clone(), size.clone());
        *self.timer.borrow_mut() = Some(glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
            match manager.early_progress(token) {
                Some((got, total)) => {
                    if got > 0 {
                        let whole = total.is_some_and(|t| got >= t);
                        size.set_label(&if whole {
                            format!("{base} — already downloaded")
                        } else {
                            format!("{base} — {} downloaded already", format::bytes(got))
                        });
                    }
                    glib::ControlFlow::Continue
                }
                None => glib::ControlFlow::Break,
            }
        }));
    }

    fn stop_timer(&self) {
        if let Some(id) = self.timer.borrow_mut().take() {
            id.remove();
        }
    }

    /// Hands the early download over to the add (it is adopted there).
    fn take(&self) -> Option<u64> {
        self.stop_timer();
        self.token.take()
    }

    /// Discards it, off the UI thread (stopping waits for it to save).
    fn drop_it(&self) {
        self.stop_timer();
        if let Some(token) = self.token.take() {
            let manager = self.manager.clone();
            gio::spawn_blocking(move || manager.cancel_early(token));
        }
    }
}
