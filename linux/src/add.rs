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
use hd_core::links::{LinkColumn, LinkRow};
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
                    // As on the Mac, Esc gives no answer: one of the three buttons must be chosen.
                    dialog.set_can_close(false);
                    let rest = gtk::CheckButton::with_label("Do the same for the rest");
                    if q.more {
                        dialog.set_extra_child(Some(&rest));
                    }
                    // The question goes over the main window, brought back if it was closed (§2).
                    ctx.window.present();
                    let answer = dialog.clone().choose_future(Some(&ctx.window)).await;
                    // A dialog that can't be closed stays up after its answer unless closed this way.
                    dialog.force_close();
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

/// A folder row: the path (as ~/…) and Choose…; `changed` runs when another folder is chosen.
fn folder_row(window: &impl IsA<gtk::Window>, folder: Rc<RefCell<PathBuf>>, files: &str, changed: impl Fn() + 'static) -> gtk::Box {
    let path = gtk::Label::builder().label(crate::list::tilde(&folder.borrow())).xalign(0.0).hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::Middle).build();
    path.set_tooltip_text(Some(&format!("The folder {files} saved to; Choose… picks another")));
    path.update_property(&[gtk::accessible::Property::Label("Save to")]);
    let choose = gtk::Button::builder().label("Choose…").tooltip_text(format!("Choose the folder {files} saved to")).build();
    let window = window.clone().upcast::<gtk::Window>();
    let path2 = path.clone();
    let changed = Rc::new(changed);
    choose.connect_clicked(move |_| {
        let dialog = gtk::FileDialog::builder().title("Save to").initial_folder(&gio::File::for_path(&*folder.borrow())).build();
        let (folder, path, changed) = (folder.clone(), path2.clone(), changed.clone());
        dialog.select_folder(Some(&window), None::<&gio::Cancellable>, move |result| {
            if let Ok(dir) = result
                && let Some(p) = dir.path()
            {
                path.set_label(&crate::list::tilde(&p));
                *folder.borrow_mut() = p;
                changed();
            }
        });
    });
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&path);
    row.append(&choose);
    row
}

/// Cancel, Add and Download; `what` is "it" (New Download) or "them" (Add Links).
fn buttons(cancel: &gtk::Button, add: &gtk::Button, download: &gtk::Button, what: &str) -> gtk::Box {
    download.add_css_class("suggested-action");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_halign(gtk::Align::End);
    row.append(cancel);
    row.append(add);
    row.append(download);
    cancel.set_tooltip_text(Some("Close without adding anything (Esc)"));
    add.set_tooltip_text(Some(&format!("Add: put {what} in the queue without starting {what}")));
    download.set_tooltip_text(Some(&format!("Download: start {what} now")));
    row
}

fn dialog_window(ctx: &Ctx, title: &str, width: i32, height: i32) -> adw::Window {
    let window = adw::Window::builder()
        .title(title)
        .transient_for(&ctx.window)
        .modal(false)
        .default_width(width)
        .default_height(height)
        .build();
    // Esc closes it, as Cancel does (its hover text says so).
    let keys = gtk::ShortcutController::new();
    keys.add_shortcut(gtk::Shortcut::new(gtk::ShortcutTrigger::parse_string("Escape"), Some(gtk::NamedAction::new("window.close"))));
    window.add_controller(keys);
    window
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
    let link_entry = gtk::Entry::builder().hexpand(true).placeholder_text("https://example.com/file.zip").build();
    crate::explain(&link_entry, "Link", "Link: the address of the file to download");
    if let Some(l) = link {
        link_entry.set_text(&l);
    }
    let mirrors = gtk::TextView::builder().wrap_mode(gtk::WrapMode::None).build();
    crate::explain(&mirrors, "Mirrors", "Mirrors: other servers with the same file, one address per line (optional)");
    mirrors.set_tooltip_text(Some("Mirrors: other servers with the same file, one address per line (optional)"));
    let mirrors_scroll = gtk::ScrolledWindow::builder().child(&mirrors).min_content_height(70).has_frame(true).build();
    let name = gtk::Entry::builder().hexpand(true).placeholder_text("From the server").build();
    crate::explain(&name, "Name", "Name: the file name to save as");
    let folder = Rc::new(RefCell::new(crate::save_folder(&ctx.manager)));
    let connections = gtk::SpinButton::with_range(1.0, 32.0, 1.0);
    crate::explain(&connections, "Connections", "Connections: how many parts of the file download at once (1–32)");
    connections.set_value(ctx.manager.settings().connections as f64);
    connections.set_tooltip_text(Some("Connections: how many parts of the file download at once (1–32)"));
    let size = gtk::Label::builder().xalign(0.0).label("").build();
    size.add_css_class("dim-label");
    let sound = gtk::CheckButton::with_label("Only the sound");
    sound.set_tooltip_text(Some("For a streamed video: save just its sound (.m4a, or .mp3), as the site sends it"));
    sound.set_sensitive(false);

    let grid = gtk::Grid::builder().row_spacing(10).column_spacing(10).margin_top(16).margin_bottom(16).margin_start(16).margin_end(16).build();
    for (row, (title, widget)) in [
        ("Link:", link_entry.clone().upcast::<gtk::Widget>()),
        ("Mirrors:", mirrors_scroll.upcast()),
        ("Name:", name.clone().upcast()),
        ("Save to:", folder_row(&window, folder.clone(), "the file is", || {}).upcast()),
        ("Connections:", connections.clone().upcast()),
        ("Size:", size.clone().upcast()),
        ("Stream:", sound.clone().upcast()),
    ]
    .into_iter()
    .enumerate()
    {
        grid.attach(&label(title), 0, row as i32, 1, 1);
        grid.attach(&widget, 1, row as i32, 1, 1);
    }
    let (cancel, add, download) = (gtk::Button::with_label("Cancel"), gtk::Button::with_label("Add"), gtk::Button::with_label("Download"));
    grid.attach(&buttons(&cancel, &add, &download, "it"), 0, 7, 2, 1);
    with_header(&window, &grid);

    // A6: ask the server about the link once it stops changing.
    let probed: Rc<RefCell<Option<u64>>> = Rc::default();
    // The link is a web page: Download and Add open Add Links with the files found on it instead (§15).
    let page: Rc<Cell<bool>> = Rc::default();
    let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
    // The download started while this window is open (enhancements.md), once the link proved resumable: Download or
    // Add adopts it, Cancel discards it.
    let early: Rc<Early> = Rc::new(Early { manager: ctx.manager.clone(), token: Cell::new(None), timer: RefCell::new(None) });
    // The user typed a name: the server's no longer replaces it (it does each time the link changes otherwise).
    let typed: Rc<Cell<bool>> = Rc::default();
    let filling: Rc<Cell<bool>> = Rc::default();
    {
        let (typed, filling) = (typed.clone(), filling.clone());
        name.connect_changed(move |n| if !filling.get() { typed.set(!n.text().is_empty()) });
    }
    {
        let (manager, name, size, probed, pending) = (ctx.manager.clone(), name.clone(), size.clone(), probed.clone(), pending.clone());
        let folder = folder.clone();
        let early = early.clone();
        let page = page.clone();
        let sound = sound.clone();
        let (typed, filling) = (typed.clone(), filling.clone());
        link_entry.connect_changed(move |entry| {
            let (typed, filling) = (typed.clone(), filling.clone());
            page.set(false);
            // Sound only is for streams; a whole video file isn't split.
            let stream = hd_core::media::download::is_stream_link(entry.text().trim());
            sound.set_sensitive(stream);
            if !stream {
                sound.set_active(false);
            }
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
                            if !typed.get() {
                                filling.set(true);
                                name.set_text(&p.name);
                                filling.set(false);
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
            let sound_only = sound.is_sensitive() && sound.is_active();
            let link = PendingLink {
                url,
                mirrors: hd_core::links::find(&text),
                name: Some(name.text().trim().to_string()).filter(|n| !n.is_empty()),
                size: *probed.borrow(),
                // The download already under way is adopted (moved to the chosen folder and name), not started again.
                early: if sound_only { None } else { early.take() },
                stream: sound_only,
                choice: sound_only.then(|| hd_core::addflow::sound_only_choice(&ctx.manager)),
                dress: None,
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
    folder_label: gtk::Label,
    /// A stream a page scan found: added as one (§15).
    stream: bool,
}

/// Add Links (§5.3): the links, each with a checkbox, name, size, folder and link; a filter; Check All and Uncheck All;
/// "Treat as mirrors of one file"; Save to; Cancel, Add, Download.
#[derive(Clone)]
pub struct AddLinks {
    window: adw::Window,
    list: gtk::ListBox,
    items: Rc<RefCell<Vec<Item>>>,
    summary: gtk::Label,
    mirrors: gtk::CheckButton,
    /// "Only the sound of streamed videos".
    sound: gtk::CheckButton,
    manager: Manager,
    /// The column the rows are sorted by, and whether A to Z (smallest first); none until a header is clicked.
    sort: Rc<Cell<Option<(LinkColumn, bool)>>>,
    /// The headers' sort arrows: Name, Size, Folder, Link.
    arrows: Rc<Vec<(LinkColumn, gtk::Image)>>,
    /// Each header as wide as its column's cells: the checkbox, Size, Folder and Link.
    widths: Rc<[gtk::SizeGroup; 4]>,
    /// Where the files are saved (Save to).
    folder: Rc<RefCell<PathBuf>>,
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
        // Nothing new (every link listed already): a beep, as on the Mac.
        if fresh.is_empty() {
            RootExt::display(&self.window).beep();
            return;
        }
        for url in &fresh {
            self.append_row(url, reqwest_name(url), None, "Checking…", hd_core::media::download::is_stream_link(url), true);
        }
        self.window.set_title(Some(&title(self.items.borrow().len())));
        self.resort();
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
                    // Rows to untick, once the list is let go (a checkbox's handler reads the list).
                    let mut untick = Vec::new();
                    for item in this.items.borrow_mut().iter_mut().filter(|i| i.url == url) {
                        match &result {
                            Ok(Ok(p)) => {
                                item.name = p.name.clone();
                                item.size = p.size;
                                item.size_text = p.size.map(format::bytes).unwrap_or_else(|| "Unknown".into());
                            }
                            // Offline, or behind a sign-in: said so, and left out unless ticked again.
                            Ok(Err(e)) => {
                                item.size_text = format::offline_text(e);
                                untick.push(item.check.clone());
                            }
                            Err(_) => item.size_text = String::new(),
                        }
                        item.name_label.set_label(&item.name);
                        item.size_label.set_label(&item.size_text);
                    }
                    for check in untick {
                        check.set_active(false);
                    }
                    // A name or size found changes the order when the list is sorted.
                    this.resort();
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
        let folder_label = gtk::Label::builder().label(crate::list::tilde(&self.folder.borrow())).xalign(0.0).width_chars(16)
            .max_width_chars(16).ellipsize(gtk::pango::EllipsizeMode::End).build();
        let link = gtk::Label::builder().label(url).xalign(0.0).width_chars(30).max_width_chars(30).ellipsize(gtk::pango::EllipsizeMode::Middle).build();
        link.add_css_class("dim-label");
        // Each cell's whole text on hover, as on the Mac: a name or link cut short can be read.
        for l in [&name_label, &size_label, &folder_label, &link] {
            l.set_tooltip_text(Some(&l.label()).filter(|t| !t.is_empty()).map(|t| t.as_str()));
            l.connect_label_notify(|l| l.set_tooltip_text(Some(&l.label()).filter(|t| !t.is_empty()).map(|t| t.as_str())));
        }
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        line.append(&check);
        line.append(&name_label);
        line.append(&size_label);
        line.append(&folder_label);
        line.append(&link);
        for (group, cell) in self.widths.iter().zip([check.upcast_ref::<gtk::Widget>(), size_label.upcast_ref(), folder_label.upcast_ref(), link.upcast_ref()]) {
            group.add_widget(cell);
        }
        let row = gtk::ListBoxRow::builder().child(&line).build();
        self.list.append(&row);
        let this = self.clone();
        check.connect_toggled(move |_| this.update());
        self.items.borrow_mut().push(Item { url: url.to_string(), name, size, size_text: size_text.into(), check, row, name_label, size_label, folder_label, stream });
    }

    /// Replaces a web page's row with the files and media found on it, each checked already; a page with nothing on
    /// it, or that couldn't be read, stays as an unchecked row that says so.
    async fn scan_page(&self, url: &str) {
        for item in self.items.borrow_mut().iter_mut().filter(|i| i.url == url) {
            item.size_text = "Scanning page…".into();
            item.size_label.set_label(&item.size_text);
        }
        let (manager, u) = (self.manager.clone(), url.to_string());
        let mut scanned = gio::spawn_blocking(move || manager.scan_page(&u)).await.unwrap_or_else(|_| Err("The scan stopped".into()));
        if matches!(&scanned, Ok(scan) if scan.found.is_empty()) {
            // Nothing in its text: its player may build the address only while it runs.
            for item in self.items.borrow_mut().iter_mut().filter(|i| i.url == url) {
                item.size_text = "Looking at the page's player…".into();
                item.size_label.set_label(&item.size_text);
            }
            let seen = crate::played::run(url).await;
            if !seen.is_empty() {
                let (manager, u) = (self.manager.clone(), url.to_string());
                scanned = gio::spawn_blocking(move || {
                    let seen: Vec<_> = seen.iter().filter_map(|j| hd_core::media::played::parse(j)).collect();
                    let given = hd_core::media::scan::Given { played: hd_core::media::played::picked(&seen), ..Default::default() };
                    manager.scan_page_given(&u, given)
                })
                .await
                .unwrap_or_else(|_| Err("The scan stopped".into()));
            }
        }
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
                self.resort();
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

    /// A header was clicked: sort by its column, or reverse the order when it is the column sorted by already.
    fn sort_by(&self, column: LinkColumn) {
        let ascending = !matches!(self.sort.get(), Some((c, true)) if c == column);
        self.sort.set(Some((column, ascending)));
        for (c, arrow) in self.arrows.iter() {
            arrow.set_visible(*c == column);
            arrow.set_icon_name(Some(if ascending { "pan-up-symbolic" } else { "pan-down-symbolic" }));
        }
        self.resort();
    }

    /// Puts the rows in the order of the column sorted by (the core's `links::sort_order`, sizes by bytes), which is
    /// also the order they are added in.
    fn resort(&self) {
        let Some((column, ascending)) = self.sort.get() else { return };
        let mut items = self.items.borrow_mut();
        let folder = crate::list::tilde(&self.folder.borrow());
        let rows: Vec<LinkRow> = items.iter().map(|i| LinkRow { name: &i.name, size: i.size, folder: &folder, url: &i.url }).collect();
        let order = hd_core::links::sort_order(&rows, column, ascending);
        let mut slots: Vec<Option<Item>> = items.drain(..).map(Some).collect();
        items.extend(order.into_iter().filter_map(|i| slots[i].take()));
        for item in items.iter() {
            // The same row widgets move, so their checkboxes and the filter's hiding stay as they were.
            self.list.remove(&item.row);
            self.list.append(&item.row);
        }
    }

    fn checked(&self) -> Vec<PendingLink> {
        self.items
            .borrow()
            .iter()
            .filter(|i| i.check.is_active())
            .map(|i| PendingLink {
                url: i.url.clone(),
                name: Some(i.name.clone()),
                size: i.size,
                stream: i.stream,
                choice: (i.stream && self.sound.is_active()).then(|| hd_core::addflow::sound_only_choice(&self.manager)),
                ..Default::default()
            })
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
        // Only for streams; a whole video file isn't split.
        let streams = checked.iter().any(|l| l.stream);
        self.sound.set_sensitive(streams);
        if !streams {
            self.sound.set_active(false);
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
    // Over the main window, brought back first if it was closed, as on the Mac.
    ctx.window.present();
    if let Some(open) = ctx.open_links.borrow().clone() {
        open.add(links);
        open.window.present();
        return;
    }
    let window = dialog_window(ctx, &title(links.len()), 760, 460);
    let filter = gtk::SearchEntry::builder().placeholder_text("Filter").hexpand(true).build();
    crate::explain(&filter, "Filter", "Filter: show links whose name or address contains the text");
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
    // Column headers over the rows, lined up with their checkbox, name, size, folder and link: click one to sort by it.
    let headers = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let widths: Rc<[gtk::SizeGroup; 4]> = Rc::new(std::array::from_fn(|_| gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal)));
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    widths[0].add_widget(&spacer);
    headers.append(&spacer);
    let mut header_buttons = Vec::new();
    let mut arrows = Vec::new();
    for (column, name, group) in [(LinkColumn::Name, "Name", None), (LinkColumn::Size, "Size", Some(1)), (LinkColumn::Folder, "Folder", Some(2)),
                                  (LinkColumn::Link, "Link", Some(3))] {
        let text = gtk::Label::builder().label(name).build();
        text.add_css_class("heading");
        let arrow = gtk::Image::from_icon_name("pan-up-symbolic");
        arrow.set_visible(false);
        let inner = gtk::Box::builder().spacing(4).halign(gtk::Align::Start).build();
        inner.append(&text);
        inner.append(&arrow);
        let button = gtk::Button::builder().child(&inner).css_classes(["flat"]).build();
        // As wide as the column's cells below; Name takes the rest.
        match group {
            Some(g) => widths[g].add_widget(&button),
            None => button.set_hexpand(true),
        }
        crate::explain(&button, name, &format!("Sort by {}; click again for the reverse order", name.to_lowercase()));
        headers.append(&button);
        header_buttons.push((column, button));
        arrows.push((column, arrow));
    }
    let table = gtk::Box::new(gtk::Orientation::Vertical, 4);
    table.append(&headers);
    table.append(&list);
    let scroll = gtk::ScrolledWindow::builder().child(&table).vexpand(true).build();
    let mirrors = gtk::CheckButton::with_label("Treat as mirrors of one file");
    mirrors.set_tooltip_text(Some("Combine the checked links into one download that uses each of them as a source"));
    let sound = gtk::CheckButton::with_label("Only the sound of streamed videos");
    sound.set_tooltip_text(Some("Save just the sound of each checked stream (.m4a, or .mp3), as the site sends it"));
    let folder = Rc::new(RefCell::new(crate::save_folder(&ctx.manager)));
    let shown: Rc<std::cell::OnceCell<AddLinks>> = Rc::default();
    let save = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    save.append(&gtk::Label::new(Some("Save to:")));
    // Another folder shows in every row's Folder cell.
    save.append(&folder_row(&window, folder.clone(), "the files are", {
        let shown = shown.clone();
        move || {
            let Some(this) = shown.get() else { return };
            let text = crate::list::tilde(&this.folder.borrow());
            this.items.borrow().iter().for_each(|i| i.folder_label.set_label(&text));
            this.resort();
        }
    }));
    let summary = gtk::Label::builder().xalign(0.0).hexpand(true)
        .tooltip_text("Links to the same file (same name and size) are added as one download and its mirrors").build();
    summary.add_css_class("dim-label");
    let (cancel, add, download) = (gtk::Button::with_label("Cancel"), gtk::Button::with_label("Add"), gtk::Button::with_label("Download"));
    let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bottom.append(&summary);
    bottom.append(&buttons(&cancel, &add, &download, "them"));
    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    for m in [&top.clone().upcast::<gtk::Widget>(), &scroll.clone().upcast(), &mirrors.clone().upcast(), &sound.clone().upcast(), &save.clone().upcast(), &bottom.clone().upcast()] {
        body.append(m);
    }
    body.set_margin_top(12);
    body.set_margin_bottom(12);
    body.set_margin_start(12);
    body.set_margin_end(12);
    // While links are dragged over it, it dims and says "Drop links to add them", as on the Mac.
    let note = gtk::Label::builder().label("Drop links to add them").can_target(false).visible(false).hexpand(true).vexpand(true).build();
    note.add_css_class("hdm-drop");
    let overlay = gtk::Overlay::builder().child(&body).build();
    overlay.add_overlay(&note);
    with_header(&window, &overlay);

    let this = AddLinks {
        window: window.clone(),
        list,
        items: Rc::default(),
        summary,
        mirrors: mirrors.clone(),
        sound: sound.clone(),
        manager: ctx.manager.clone(),
        sort: Rc::default(),
        arrows: Rc::new(arrows),
        widths,
        folder: folder.clone(),
    };
    let _ = shown.set(this.clone());
    for (column, button) in header_buttons {
        let this2 = this.clone();
        button.connect_clicked(move |_| this2.sort_by(column));
    }
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
    window.add_controller(drop_target(ctx, move |on| note.set_visible(on), {
        let this = this.clone();
        move |links| this.add(links)
    }));
    let submit = {
        let (ctx, this) = (ctx.clone(), this.clone());
        move |start: bool| {
            let mut links = this.checked();
            // Nothing ticked: a beep, as on the Mac.
            if links.is_empty() {
                RootExt::display(&this.window).beep();
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
    // Test hook: `--sort-links name|size|folder|link [desc]` sorts as clicks on that header do.
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--sort-links") {
        let column = match args.get(i + 1).map(String::as_str) {
            Some("size") => LinkColumn::Size,
            Some("folder") => LinkColumn::Folder,
            Some("link") => LinkColumn::Link,
            _ => LinkColumn::Name,
        };
        this.sort_by(column);
        if args.get(i + 2).is_some_and(|a| a == "desc") {
            this.sort_by(column);
        }
    }
    window.present();
}

/// What a drop carries, as text to find links in: link text, the links inside dropped text files (§5.4, §5.7), and
/// for text dragged from a web page its HTML, read as a paste reads it (§5.2): the links behind the text, relative
/// ones made whole against the page.
pub async fn drop_text(drop: &gdk::Drop) -> String {
    let types = drop.formats().union_deserialize_types();
    let mut text = String::new();
    let files = if types.contains_type(gdk::FileList::static_type()) {
        drop.read_value_future(gdk::FileList::static_type(), glib::Priority::DEFAULT).await.ok().and_then(|v| v.get::<gdk::FileList>().ok())
    } else {
        None
    };
    if let Some(files) = files {
        for f in files.files() {
            text.push('\n');
            match f.path() {
                Some(p) => text.push_str(&hd_core::links::text_of_file(&p).unwrap_or_default()),
                None => text.push_str(&f.uri()),
            }
        }
    } else if types.contains_type(glib::GString::static_type())
        && let Ok(v) = drop.read_value_future(glib::GString::static_type(), glib::Priority::DEFAULT).await
        && let Ok(s) = v.get::<String>()
    {
        text = s;
    }
    if drop.formats().contain_mime_type("text/html")
        && let Some(html) = drop_bytes(drop, "text/html").await
    {
        let mut base = None;
        for mime in SOURCE_URL {
            if drop.formats().contain_mime_type(mime) && let Some(b) = drop_bytes(drop, mime).await {
                base = Some(b);
                break;
            }
        }
        text.push('\n');
        text.push_str(&page_html(&html, base.as_deref()));
    }
    text
}

/// The data of one type in a drop.
async fn drop_bytes(drop: &gdk::Drop, mime: &str) -> Option<Vec<u8>> {
    let (stream, _) = drop.read_future(&[mime], glib::Priority::DEFAULT).await.ok()?;
    read_all(stream).await
}

async fn read_all(stream: gio::InputStream) -> Option<Vec<u8>> {
    let out = gio::MemoryOutputStream::new_resizable();
    out.splice_future(&stream, gio::OutputStreamSpliceFlags::CLOSE_SOURCE | gio::OutputStreamSpliceFlags::CLOSE_TARGET,
                      glib::Priority::DEFAULT).await.ok()?;
    Some(out.steal_as_bytes().to_vec())
}

/// Where browsers say a copy or a drag came from: Chromium browsers (Chrome, Edge) as `chromium/x-source-url`,
/// Firefox as `text/x-moz-url-priv`.
const SOURCE_URL: [&str; 2] = ["chromium/x-source-url", "text/x-moz-url-priv"];

/// A page's HTML as text (UTF-8, or UTF-16 with its byte-order mark, as Firefox may send it), its relative links
/// made whole against the page it came from.
fn page_html(html: &[u8], source: Option<&[u8]>) -> String {
    let utf16 = |bytes: &[u8], le: bool| {
        let units: Vec<u16> = bytes.chunks_exact(2)
            .map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
            .collect();
        String::from_utf16_lossy(&units)
    };
    let html = match html {
        [0xff, 0xfe, rest @ ..] => utf16(rest, true),
        [0xfe, 0xff, rest @ ..] => utf16(rest, false),
        _ => String::from_utf8_lossy(html).into_owned(),
    };
    match source.and_then(hd_core::links::copied_from) {
        Some(base) => hd_core::links::absolute_html(&html, &base),
        None => html,
    }
}

/// Links dropped on a widget (§5.4, §5.7): `hover` says whether what is dragged over it has links (for a "Drop links
/// here" note); `on_links` gets the links dropped, none when it had none.
pub fn links_drop(hover: impl Fn(bool) + 'static, on_links: impl Fn(Vec<String>) + 'static) -> gtk::DropTargetAsync {
    let formats = gdk::ContentFormats::builder()
        .add_type(gdk::FileList::static_type())
        .add_type(glib::GString::static_type())
        .add_mime_type("text/html")
        .build()
        .union_deserialize_mime_types();
    let target = gtk::DropTargetAsync::new(Some(formats), gdk::DragAction::COPY);
    let (hover, on_links) = (Rc::new(hover), Rc::new(on_links));
    {
        let hover = hover.clone();
        target.connect_drag_enter(move |_, drop, _, _| {
            let (hover, drop) = (hover.clone(), drop.clone());
            glib::spawn_future_local(async move {
                let links = hd_core::links::find(&drop_text(&drop).await);
                hover(!links.is_empty());
            });
            gdk::DragAction::COPY
        });
    }
    {
        let hover = hover.clone();
        target.connect_drag_leave(move |_, _| hover(false));
    }
    target.connect_drop(move |_, drop, _, _| {
        let (hover, on_links, drop) = (hover.clone(), on_links.clone(), drop.clone());
        glib::spawn_future_local(async move {
            let links = hd_core::links::find(&drop_text(&drop).await);
            drop.finish(if links.is_empty() { gdk::DragAction::empty() } else { gdk::DragAction::COPY });
            hover(false);
            on_links(links);
        });
        true
    });
    target
}

/// Links dropped on a window open or join Add Links; a drop with none says so in the footer.
pub fn drop_target(ctx: &Ctx, hover: impl Fn(bool) + 'static, on_links: impl Fn(Vec<String>) + 'static) -> gtk::DropTargetAsync {
    let tell = ctx.tell.clone();
    links_drop(hover, move |links| {
        if links.is_empty() {
            tell("No links found in what was dropped".into());
        } else {
            on_links(links);
        }
    })
}

/// Batch Download (§5.5): a link with a range placeholder, the help, a preview of the first and last links and the
/// count, then Add Links.
pub fn batch(ctx: &Ctx) {
    let window = dialog_window(ctx, "Batch Download", 600, 230);
    let pattern = gtk::Entry::builder().hexpand(true).placeholder_text("https://example.com/img^(###1-100).jpg").build();
    crate::explain(&pattern, "Link pattern", "Batch link: a link with a range placeholder");
    let help = gtk::Label::builder().xalign(0.0).wrap(true).label(
        "Put a range where the number or letters change: ^(###1-100) pads to 3 digits, ^(0-ffH) counts in \
         hexadecimal, ^(a-z) or ^(AA-BZ) in letters. Separate several ranges with commas.").build();
    help.add_css_class("dim-label");
    help.add_css_class("caption");
    let preview = gtk::Label::builder().xalign(0.0).wrap(true).selectable(true).build();
    preview.add_css_class("dim-label");
    preview.add_css_class("monospace");
    let next = gtk::Button::with_label("Continue…");
    next.set_tooltip_text(Some("Show the links in the Add Links window"));
    next.add_css_class("suggested-action");
    next.set_sensitive(false);
    let cancel = gtk::Button::with_label("Cancel");
    cancel.set_tooltip_text(Some("Close without adding anything (Esc)"));
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_halign(gtk::Align::End);
    row.append(&cancel);
    row.append(&next);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.set_margin_top(20);
    body.set_margin_bottom(20);
    body.set_margin_start(20);
    body.set_margin_end(20);
    body.append(&pattern);
    body.append(&help);
    body.append(&preview);
    body.append(&row);
    with_header(&window, &body);
    let links: Rc<RefCell<Vec<String>>> = Rc::default();
    {
        let (links, next) = (links.clone(), next.clone());
        pattern.connect_changed(move |p| {
            let text = p.text().trim().to_string();
            preview.remove_css_class("error");
            preview.add_css_class("dim-label");
            if text.is_empty() {
                preview.set_label("");
                next.set_sensitive(false);
                links.borrow_mut().clear();
                return;
            }
            match hd_core::links::expand_batch(&text) {
                Ok(l) if !l.is_empty() => {
                    preview.set_label(&if l.len() == 1 {
                        format!("1 link: {}", l[0])
                    } else {
                        format!("{} links\nFirst: {}\nLast:  {}", l.len(), l[0], l[l.len() - 1])
                    });
                    next.set_sensitive(true);
                    *links.borrow_mut() = l;
                }
                Ok(_) => {
                    preview.set_label("");
                    next.set_sensitive(false);
                    links.borrow_mut().clear();
                }
                Err(e) => {
                    // A mistake in the pattern is red, as on the Mac.
                    preview.set_label(&e);
                    preview.remove_css_class("dim-label");
                    preview.add_css_class("error");
                    next.set_sensitive(false);
                    links.borrow_mut().clear();
                }
            }
        });
    }
    let w = window.clone();
    cancel.connect_clicked(move |_| w.close());
    let (ctx2, w) = (ctx.clone(), window.clone());
    next.connect_clicked(move |_| {
        let l = links.borrow().clone();
        if l.is_empty() {
            return;
        }
        w.close();
        add_links(&ctx2, l);
    });
    pattern.set_activates_default(true);
    window.set_default_widget(Some(&next));
    window.present();
}

/// Import Links (§5.7): the links in one or more text files.
pub fn import(ctx: &Ctx) {
    // The Mac's line under the picker's title, here as the name of the files it shows (GTK's picker has no such line).
    let files = gtk::FileFilter::new();
    files.set_name(Some("Files with links: plain text with or without an extension, Markdown, rich text, HTML…"));
    files.add_pattern("*");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&files);
    let dialog = gtk::FileDialog::builder().title("Import Links").accept_label("Import").filters(&filters).default_filter(&files).build();
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
        let _ = unreadable;
        let links = hd_core::links::find(&text);
        if links.is_empty() {
            let heading = match files.n_items() {
                1 => {
                    let name = files.item(0).and_downcast::<gio::File>().and_then(|f| f.basename())
                        .map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    format!("“{name}” has no links.")
                }
                _ => "These files have no links.".into(),
            };
            alert(&ctx.window, &heading, "Import Links reads text files (with or without an extension), Markdown, rich text \
                   and HTML, and finds the web addresses (http, https or ftp) in them.");
            return;
        }
        add_links(&ctx, links);
    });
}

/// A message over the window, with OK, as the Mac's alerts (§4).
pub fn alert(window: &impl IsA<gtk::Widget>, heading: &str, body: &str) {
    let d = adw::AlertDialog::new(Some(heading), Some(body).filter(|b| !b.is_empty()));
    d.add_response("ok", "OK");
    d.present(Some(window));
}

/// The clipboard's text, with its HTML when it has some (links behind a copied page's text, §5.2), the HTML's
/// relative links made whole against the page it was copied from: Chromium browsers (Chrome, Edge) put its address
/// beside the copy as `chromium/x-source-url`, Firefox as `text/x-moz-url-priv`.
pub async fn clipboard_text(clipboard: &gdk::Clipboard) -> String {
    let mut text = clipboard.read_text_future().await.ok().flatten().map(|t| t.to_string()).unwrap_or_default();
    if let Some(html) = clipboard_bytes(clipboard, "text/html").await {
        let mut base = None;
        for mime in SOURCE_URL {
            if let Some(b) = clipboard_bytes(clipboard, mime).await {
                base = Some(b);
                break;
            }
        }
        text.push('\n');
        text.push_str(&page_html(&html, base.as_deref()));
    }
    text
}

/// The clipboard's data of one type, if it has that type.
async fn clipboard_bytes(clipboard: &gdk::Clipboard, mime: &str) -> Option<Vec<u8>> {
    if !clipboard.formats().contain_mime_type(mime) {
        return None;
    }
    let (stream, _) = clipboard.read_future(&[mime], glib::Priority::DEFAULT).await.ok()?;
    read_all(stream).await
}

/// Export Links (§5.8): these downloads (all with none), saved as a text file with each file's name and its link and
/// mirrors under it, as a tree. Import Links reads it back.
pub fn export(ctx: &Ctx, ids: Vec<u64>) {
    let date = glib::DateTime::now_local().ok().and_then(|d| d.format("%Y-%m-%d").ok()).map(|d| d.to_string()).unwrap_or_default();
    // What is saved, as the Mac's line under the picker's title says it.
    let what = match ids.len() {
        0 => "Every download's links, under its file's name".to_string(),
        1 => "The selected download's links".to_string(),
        n => format!("The {n} selected downloads' links"),
    };
    let text = gtk::FileFilter::new();
    text.set_name(Some(&what));
    text.add_mime_type("text/plain");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&text);
    let dialog = gtk::FileDialog::builder()
        .title("Export Links")
        .filters(&filters)
        .default_filter(&text)
        .accept_label("Export")
        .initial_name(format!("{}.txt", format!("Herb's DM links {date}").trim_end()))
        .build();
    let ctx = ctx.clone();
    dialog.save(Some(&ctx.window.clone()), None::<&gio::Cancellable>, move |result| {
        let Some(path) = result.ok().and_then(|f| f.path()) else { return };
        if let Err(e) = std::fs::write(&path, ctx.manager.links_tree(&ids)) {
            alert(&ctx.window, "The links couldn't be saved.", &e.to_string());
        }
    });
}

/// Add from Clipboard (§5.2).
pub fn from_clipboard(ctx: &Ctx) {
    let ctx = ctx.clone();
    glib::spawn_future_local(async move {
        let links = hd_core::links::find(&clipboard_text(&ctx.window.clipboard()).await);
        if links.is_empty() {
            alert(&ctx.window, "The clipboard has no links.", "Copy one or more web addresses (http, https or ftp), then try again.");
            return;
        }
        add_links(&ctx, links);
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
