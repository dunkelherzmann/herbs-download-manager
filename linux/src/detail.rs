//! The Download window (§6): the file's segments, its numbers, Pause/Resume and Close, and the Info, Connections,
//! Mirrors and Options tabs. One window per download; it refreshes twice a second and closing it never stops the
//! download. Also Add Mirror… (§7.1).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use hd_core::format;
use hd_core::manager::{Detail, Manager, Status};

use crate::add::Ctx;

thread_local! {
    static OPEN: RefCell<HashMap<u64, adw::Window>> = RefCell::new(HashMap::new());
}

/// Opens (or brings forward) a download's window, on `tab` ("info", "connections", "mirrors", "options").
pub fn open(ctx: &Ctx, id: u64, tab: Option<&str>) {
    if let Some(w) = OPEN.with(|o| o.borrow().get(&id).cloned()) {
        if let Some(tab) = tab
            && let Some(stack) = w.content().and_then(|c| find_stack(&c))
        {
            stack.set_visible_child_name(tab);
        }
        w.present();
        return;
    }
    let Some(first) = ctx.manager.detail(id) else { return };
    let window = adw::Window::builder().title(&first.row.name).transient_for(&ctx.window).default_width(680).default_height(470).build();

    let bar = gtk::DrawingArea::builder().content_height(18).hexpand(true).build();
    crate::explain(&bar, "Downloaded parts of the file", "The file's ranges: filled parts are downloaded, gaps are still to fetch");
    let percent = gtk::Label::builder().xalign(0.0).build();
    percent.add_css_class("title-3");
    let numbers = gtk::Label::builder().xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).build();
    numbers.add_css_class("dim-label");
    let toggle = gtk::Button::with_label("Pause");
    let close = gtk::Button::with_label("Close");
    close.set_tooltip_text(Some("Close this window; the download keeps going"));
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    header.append(&percent);
    header.append(&numbers);
    header.append(&toggle);
    header.append(&close);

    // Info.
    let info_grid = gtk::Grid::builder().row_spacing(8).column_spacing(12).margin_top(12).margin_start(12).margin_end(12).build();
    let mut info = HashMap::new();
    for (i, key) in ["Link", "Folder", "Size", "Resumable", "Date Added", "Date Finished", "Checksum"].iter().enumerate() {
        info_grid.attach(&gtk::Label::builder().label(format!("{key}:")).xalign(1.0).build(), 0, i as i32, 1, 1);
        // One line, shortened in the middle as on the Mac, the whole value on hover: a long link that wrapped at
        // any character made the window taller than the screen and pushed the tabs' buttons out of sight.
        let value = gtk::Label::builder().xalign(0.0).selectable(true).ellipsize(gtk::pango::EllipsizeMode::Middle).hexpand(true).build();
        value.connect_label_notify(|l| l.set_tooltip_text(Some(&l.label())));
        info_grid.attach(&value, 1, i as i32, 1, 1);
        info.insert(*key, value);
    }

    // Connections.
    let conn_list = gtk::ListBox::new();
    conn_list.add_css_class("boxed-list");
    conn_list.set_selection_mode(gtk::SelectionMode::None);
    let plus = gtk::Button::builder().icon_name("list-add-symbolic").tooltip_text("Add a connection now").build();
    let minus = gtk::Button::builder().icon_name("list-remove-symbolic").tooltip_text("Remove a connection now").build();
    let conn_buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    conn_buttons.append(&plus);
    conn_buttons.append(&minus);
    let conn_page = page(&conn_list, &conn_buttons, &[("Source", 22), ("Range", 14), ("Downloaded", 14), ("Speed", 14), ("State", 14)]);

    // Mirrors (§7.3).
    let src_list = gtk::ListBox::new();
    src_list.add_css_class("boxed-list");
    let add_mirror = gtk::Button::with_label("Add Mirror…");
    add_mirror.set_tooltip_text(Some("Add Mirror: another server with the same file (Ctrl+Alt+M)"));
    let remove_mirror = gtk::Button::with_label("Remove");
    remove_mirror.set_tooltip_text(Some("Remove the selected mirror; the original link can only be turned off"));
    let make_main = gtk::Button::with_label("Make Main Link");
    make_main.set_tooltip_text(Some(
        "Make Main Link: the selected mirror becomes the download's link, and the link becomes a mirror. What was downloaded is kept",
    ));
    make_main.update_property(&[gtk::accessible::Property::Label("Make the selected mirror the main link")]);
    let copy_link = gtk::Button::with_label("Copy Link");
    copy_link.set_tooltip_text(Some("Copy the selected source's address (Ctrl+C)"));
    let src_buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    src_buttons.append(&add_mirror);
    src_buttons.append(&copy_link);
    src_buttons.append(&make_main);
    src_buttons.append(&remove_mirror);
    let src_page = page(&src_list, &src_buttons,
                        &[("", 3), ("Address", 34), ("State", 14), ("Connections", 6), ("Speed", 10), ("Served", 10)]);

    // Options.
    let connections = gtk::SpinButton::with_range(1.0, 32.0, 1.0);
    connections.update_property(&[gtk::accessible::Property::Label("Connections")]);
    connections.set_tooltip_text(Some("Connections for this download (1–32), applied at once"));
    let limit_on = gtk::CheckButton::with_label("Limit to");
    let limit = gtk::SpinButton::with_range(1.0, 1_000_000.0, 50.0);
    let unit = gtk::DropDown::from_strings(&["KB/s", "MB/s"]);
    if let Some(l) = first.speed_limit {
        limit_on.set_active(true);
        let mb = l >= 1_000_000 && l % 100_000 == 0;
        unit.set_selected(mb as u32);
        limit.set_value(l as f64 / if mb { 1_000_000.0 } else { 1_000.0 });
    } else {
        limit.set_value(500.0);
    }
    limit_on.set_tooltip_text(Some("Limit the download speed of this download"));
    crate::explain(&limit, "Speed limit", "The most this download may download each second");
    crate::explain(&unit, "Speed limit unit", "Kilobytes or megabytes a second");
    let limit_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    limit_row.append(&limit_on);
    limit_row.append(&limit);
    limit_row.append(&unit);
    let options = gtk::Grid::builder().row_spacing(10).column_spacing(12).margin_top(12).margin_start(12).build();
    options.attach(&gtk::Label::builder().label("Connections:").xalign(1.0).build(), 0, 0, 1, 1);
    options.attach(&connections, 1, 0, 1, 1);
    options.attach(&gtk::Label::builder().label("Speed limit:").xalign(1.0).build(), 0, 1, 1, 1);
    options.attach(&limit_row, 1, 1, 1, 1);
    // §6: the checksum to verify before the file gets its name; a pasted value picks its algorithm by length.
    let algorithms = gtk::DropDown::from_strings(&["None", "MD5", "SHA-1", "SHA-256", "SHA-512"]);
    algorithms.set_tooltip_text(Some("The kind of checksum: MD5, SHA-1, SHA-256 or SHA-512"));
    let value = gtk::Entry::builder().placeholder_text("Paste the expected value").width_chars(48).build();
    value.set_tooltip_text(Some("The value the site publishes for this file; it's checked once every byte is in"));
    let note = gtk::Label::builder().xalign(0.0).build();
    note.add_css_class("dim-label");
    let index_of = |a: Option<hd_core::checksum::Algorithm>| a.map(|a| 1 + hd_core::checksum::Algorithm::ALL.iter().position(|x| *x == a).unwrap_or(0) as u32).unwrap_or(0);
    algorithms.set_selected(index_of(first.checksum.as_ref().map(|c| c.algorithm)));
    if let Some(c) = &first.checksum {
        value.set_text(&c.value);
    }
    let checksum_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    checksum_row.append(&algorithms);
    checksum_row.append(&value);
    options.attach(&gtk::Label::builder().label("Checksum:").xalign(1.0).build(), 0, 2, 1, 1);
    options.attach(&checksum_row, 1, 2, 1, 1);
    options.attach(&note, 1, 3, 1, 1);
    let apply_checksum = {
        let (manager, algorithms, value, note) = (ctx.manager.clone(), algorithms.clone(), value.clone(), note.clone());
        move |guess: bool| {
            let text = value.text().trim().to_string();
            if guess && let Some(a) = hd_core::checksum::Algorithm::of_value(&text) {
                algorithms.set_selected(index_of(Some(a)));
            }
            let chosen = (algorithms.selected() as usize).checked_sub(1).map(|i| hd_core::checksum::Algorithm::ALL[i]);
            match chosen.filter(|_| !text.is_empty()) {
                None => {
                    manager.set_checksum(id, None);
                    note.set_label("");
                }
                Some(a) => match hd_core::checksum::Checksum::new(a, &text) {
                    Ok(c) => {
                        manager.set_checksum(id, Some(c));
                        note.set_label("Checked once every byte is in, before the file gets its name");
                    }
                    Err(e) => note.set_label(&e),
                },
            }
        }
    };
    let a = apply_checksum.clone();
    value.connect_activate(move |_| a(true));
    let a = apply_checksum.clone();
    value.connect_changed(move |_| a(true));
    algorithms.connect_selected_notify(move |_| apply_checksum(false));

    let stack = adw::ViewStack::new();
    // Each tab as tall as its own page, so the lists' buttons (Add Mirror, Make Main Link, +, −) stay in sight.
    stack.set_vhomogeneous(false);
    stack.add_titled_with_icon(&info_grid, Some("info"), "Info", "dialog-information-symbolic");
    stack.add_titled_with_icon(&conn_page, Some("connections"), "Connections", "network-transmit-receive-symbolic");
    stack.add_titled_with_icon(&src_page, Some("mirrors"), "Mirrors", "folder-remote-symbolic");
    stack.add_titled_with_icon(&options, Some("options"), "Options", "emblem-system-symbolic");
    let switcher = adw::ViewSwitcher::builder().stack(&stack).policy(adw::ViewSwitcherPolicy::Wide).build();

    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    body.set_margin_top(12);
    body.set_margin_bottom(12);
    body.set_margin_start(12);
    body.set_margin_end(12);
    body.append(&bar);
    body.append(&header);
    body.append(&switcher);
    body.append(&stack);
    stack.set_vexpand(true);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&body));
    window.set_content(Some(&view));

    let detail: Rc<RefCell<Detail>> = Rc::new(RefCell::new(first));
    {
        let detail = detail.clone();
        bar.set_draw_func(move |area, cr, w, h| draw_segments(area, cr, w as f64, h as f64, &detail.borrow()));
    }
    let manager = ctx.manager.clone();
    let id_ = id;
    {
        let (manager, detail, w) = (manager.clone(), detail.clone(), window.clone());
        // Pausing a download that can't be resumed asks first, as the main window's Pause does (§10).
        toggle.connect_clicked(move |_| {
            if detail.borrow().row.status.is_running() { crate::window::pause_asking(&w, &manager, id_) } else { manager.resume(&[id_]) }
        });
    }
    {
        let w = window.clone();
        close.connect_clicked(move |_| w.close());
    }
    {
        let (manager, detail) = (manager.clone(), detail.clone());
        plus.connect_clicked(move |_| manager.set_connections(id_, (detail.borrow().row.allowed_connections + 1).min(32)));
    }
    {
        let (manager, detail) = (manager.clone(), detail.clone());
        minus.connect_clicked(move |_| manager.set_connections(id_, detail.borrow().row.allowed_connections.saturating_sub(1).max(1)));
    }
    {
        let manager = manager.clone();
        connections.connect_value_changed(move |s| manager.set_connections(id_, s.value() as u32));
    }
    let apply_limit = {
        let (manager, limit_on, limit, unit) = (manager.clone(), limit_on.clone(), limit.clone(), unit.clone());
        move || {
            let bytes = limit_on.is_active().then(|| (limit.value() * if unit.selected() == 1 { 1_000_000.0 } else { 1_000.0 }) as u64);
            manager.set_speed_limit(id_, bytes);
            limit.set_sensitive(limit_on.is_active());
            unit.set_sensitive(limit_on.is_active());
        }
    };
    apply_limit();
    let a = apply_limit.clone();
    limit_on.connect_toggled(move |_| a());
    let a = apply_limit.clone();
    limit.connect_value_changed(move |_| a());
    unit.connect_selected_notify(move |_| apply_limit());
    {
        let ctx = ctx.clone();
        add_mirror.connect_clicked(move |_| ask_mirrors(&ctx, id_));
    }
    // Copy Link, Make Main Link and Remove act on a source by its place in the list: the selected one for the
    // buttons, the right-clicked one for the source's menu (§7.3).
    let source_action = {
        let (manager, src_list, detail) = (manager.clone(), src_list.clone(), detail.clone());
        Rc::new(move |what: &str, index: i32| {
            let d = detail.borrow();
            let Some(s) = usize::try_from(index).ok().and_then(|i| d.sources.get(i)) else { return };
            let original = s.url == d.row.url;
            match what {
                "copy" => src_list.clipboard().set_text(&s.url),
                "main" if !original => {
                    manager.make_main(id_, &s.url);
                }
                "remove" if !original => {
                    manager.remove_source(id_, &s.url);
                }
                _ => {}
            }
        })
    };
    {
        let (a, src_list) = (source_action.clone(), src_list.clone());
        make_main.connect_clicked(move |_| a("main", src_list.selected_row().map(|r| r.index()).unwrap_or(-1)));
    }
    {
        let menu = gtk::gio::Menu::new();
        menu.append(Some("Copy Link"), Some("source.copy"));
        menu.append(Some("Make Main Link"), Some("source.main"));
        menu.append(Some("Remove"), Some("source.remove"));
        // Parented on the page, not the list: a list box treats every child as a row.
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.set_parent(&src_page);
        {
            let popover = popover.clone();
            window.connect_destroy(move |_| popover.unparent());
        }
        popover.set_has_arrow(false);
        let clicked = Rc::new(std::cell::Cell::new(-1));
        let group = gtk::gio::SimpleActionGroup::new();
        for what in ["copy", "main", "remove"] {
            let action = gtk::gio::SimpleAction::new(what, None);
            let (a, clicked) = (source_action.clone(), clicked.clone());
            action.connect_activate(move |_, _| a(what, clicked.get()));
            group.add_action(&action);
        }
        src_page.insert_action_group("source", Some(&group));
        let click = gtk::GestureClick::builder().button(gtk::gdk::BUTTON_SECONDARY).build();
        let (list, detail, page) = (src_list.clone(), detail.clone(), src_page.clone());
        click.connect_pressed(move |_, _, x, y| {
            let Some(row) = list.row_at_y(y as i32) else { return };
            clicked.set(row.index());
            let d = detail.borrow();
            let original = d.sources.get(row.index() as usize).is_some_and(|s| s.url == d.row.url);
            for what in ["main", "remove"] {
                if let Some(a) = group.lookup_action(what).and_downcast::<gtk::gio::SimpleAction>() {
                    a.set_enabled(!original);
                }
            }
            let at = list.compute_point(&page, &gtk::graphene::Point::new(x as f32, y as f32))
                .map(|p| (p.x() as i32, p.y() as i32)).unwrap_or((x as i32, y as i32));
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(at.0, at.1, 1, 1)));
            popover.popup();
        });
        src_list.add_controller(click);
    }
    // Copy Link: the selected source's address, the main link's too (Ctrl+C on the list does the same).
    {
        let (src_list2, a) = (src_list.clone(), source_action.clone());
        let copy = move || a("copy", src_list2.selected_row().map(|r| r.index()).unwrap_or(-1));
        let copy2 = copy.clone();
        copy_link.connect_clicked(move |_| copy2());
        let keys = gtk::ShortcutController::new();
        keys.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string("<Control>c"),
            Some(gtk::CallbackAction::new(move |_, _| {
                copy();
                gtk::glib::Propagation::Stop
            })),
        ));
        src_list.add_controller(keys);
    }
    {
        let (a, src_list) = (source_action.clone(), src_list.clone());
        remove_mirror.connect_clicked(move |_| a("remove", src_list.selected_row().map(|r| r.index()).unwrap_or(-1)));
    }
    // Remove and Make Main Link need a selected mirror (not the original link), Copy Link any selected source.
    let enable_sources = {
        let (src_list, detail, remove_mirror, make_main, copy_link) =
            (src_list.clone(), detail.clone(), remove_mirror.clone(), make_main.clone(), copy_link.clone());
        move || {
            let d = detail.borrow();
            let selected = src_list.selected_row().and_then(|r| d.sources.get(r.index() as usize));
            let mirror = selected.is_some_and(|s| s.url != d.row.url);
            remove_mirror.set_sensitive(mirror);
            make_main.set_sensitive(mirror);
            copy_link.set_sensitive(selected.is_some());
        }
    };
    {
        let e = enable_sources.clone();
        src_list.connect_row_selected(move |_, _| e());
    }

    let limit_row2 = limit_row.clone();
    let refresh = {
        let (manager, window, detail, bar) = (manager.clone(), window.clone(), detail.clone(), bar.clone());
        move || -> bool {
            let Some(d) = manager.detail(id_) else {
                window.close();
                return false;
            };
            let r = &d.row;
            window.set_title(Some(&r.name));
            let finished = r.status == Status::Finished;
            percent.set_label(&match r.fraction {
                Some(f) => format!("{}%", (f * 100.0).floor() as u32),
                None if finished => "100%".into(),
                None => String::new(),
            });
            let left = format::time_left_text(r.time_left);
            numbers.set_label(
                &[format::size_text(r.downloaded, r.size, finished), format::speed_text(r.speed),
                  if left.is_empty() { String::new() } else { format!("{left} left") }, r.status_text.clone()]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("  ·  "),
            );
            toggle.set_label(if r.status.is_running() { "Pause" } else { "Resume" });
            toggle.set_sensitive(r.status.is_running() || matches!(r.status, Status::Waiting | Status::Paused | Status::Failed(_)));
            toggle.set_tooltip_text(Some(if r.status.is_running() { "Pause: stop, keeping what's downloaded (Ctrl+.)" } else { "Resume: continue (Ctrl+R)" }));
            info["Link"].set_label(&r.url);
            info["Folder"].set_label(&crate::list::tilde(&r.folder));
            info["Size"].set_label(&r.size.map(format::bytes).unwrap_or_else(|| "Unknown".into()));
            info["Resumable"].set_label(if r.resumable { "Yes" } else { "No: pausing starts it over" });
            info["Date Added"].set_label(&crate::list::full_date_text(r.added));
            info["Checksum"].set_label(&d.verified.clone().unwrap_or_else(|| match &d.checksum {
                Some(c) => format!("{}, checked when the download finishes", c.algorithm.name()),
                None => "—".into(),
            }));
            info["Date Finished"].set_label(&r.finished.map(crate::list::full_date_text).unwrap_or_else(|| "—".into()));
            plus.set_sensitive(!finished && r.allowed_connections < 32);
            minus.set_sensitive(!finished && r.allowed_connections > 1);
            connections.set_sensitive(!finished);
            limit_row2.set_sensitive(!finished);
            if connections.value() as u32 != r.allowed_connections && !connections.has_focus() {
                connections.set_value(r.allowed_connections as f64);
            }
            refill(&conn_list, d.connections.iter().map(|c| {
                let range = match (c.start, c.end) {
                    (_, 0) => String::new(),
                    (s, e) => format!("{}–{}", format::bytes(s), format::bytes(e)),
                };
                vec![host(&c.source), range, if c.bytes > 0 { format::bytes(c.bytes) } else { String::new() }, format::speed_text(c.speed), c.state.clone()]
            }));
            refill_sources(&src_list, &d, &manager, id_);
            *detail.borrow_mut() = d;
            enable_sources();
            bar.queue_draw();
            true
        }
    };
    refresh();
    let tick = glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
        if refresh() { glib::ControlFlow::Continue } else { glib::ControlFlow::Break }
    });
    let tick = Rc::new(RefCell::new(Some(tick)));
    window.connect_close_request(move |_| {
        if let Some(t) = tick.borrow_mut().take() {
            t.remove();
        }
        OPEN.with(|o| o.borrow_mut().remove(&id_));
        glib::Propagation::Proceed
    });
    if let Some(tab) = tab {
        stack.set_visible_child_name(tab);
    }
    OPEN.with(|o| o.borrow_mut().insert(id, window.clone()));
    window.present();
}

fn find_stack(widget: &gtk::Widget) -> Option<adw::ViewStack> {
    if let Some(s) = widget.downcast_ref::<adw::ViewStack>() {
        return Some(s.clone());
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(s) = find_stack(&c) {
            return Some(s);
        }
        child = c.next_sibling();
    }
    None
}

/// A list page: the columns' titles over the list (as the Mac's table headers), the list, then its buttons.
fn page(list: &gtk::ListBox, buttons: &gtk::Box, titles: &[(&str, i32)]) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.set_margin_top(12);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    head.set_margin_start(12);
    for (title, chars) in titles {
        let l = gtk::Label::builder().label(*title).xalign(0.0).width_chars(*chars).build();
        l.add_css_class("caption-heading");
        l.add_css_class("dim-label");
        head.append(&l);
    }
    b.append(&head);
    b.append(&gtk::ScrolledWindow::builder().child(list).vexpand(true).build());
    b.append(buttons);
    b
}

fn host(url: &str) -> String {
    url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or(url).to_string()
}

/// Rewrites a list's rows of text cells, keeping their number when it doesn't change.
fn refill(list: &gtk::ListBox, rows: impl Iterator<Item = Vec<String>>) {
    let rows: Vec<Vec<String>> = rows.collect();
    let mut n = 0;
    while list.row_at_index(n).is_some() {
        n += 1;
    }
    if n as usize != rows.len() {
        // Rows only: the sources' right-click menu is a child of the list too.
        list.remove_all();
        for cells in &rows {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            line.set_margin_top(4);
            line.set_margin_bottom(4);
            for (i, _) in cells.iter().enumerate() {
                line.append(&gtk::Label::builder().xalign(0.0).width_chars(if i == 0 { 22 } else { 14 }).ellipsize(gtk::pango::EllipsizeMode::End).build());
            }
            list.append(&line);
        }
    }
    for (i, cells) in rows.iter().enumerate() {
        let Some(line) = list.row_at_index(i as i32).and_then(|r| r.child()) else { continue };
        let mut child = line.first_child();
        for text in cells {
            if let Some(label) = child.as_ref().and_then(|c| c.downcast_ref::<gtk::Label>()) {
                show_text(label, text);
            }
            child = child.and_then(|c| c.next_sibling());
        }
    }
}

/// §7.3: on/off checkbox, address, state, active connections, speed and bytes served.
fn refill_sources(list: &gtk::ListBox, d: &Detail, manager: &Manager, id: u64) {
    let mut n = 0;
    while list.row_at_index(n).is_some() {
        n += 1;
    }
    if n as usize != d.sources.len() {
        let selected = list.selected_row().map(|r| r.index());
        // Rows only: the sources' right-click menu is a child of the list too.
        list.remove_all();
        for s in &d.sources {
            let check = gtk::CheckButton::new();
            let url = s.url.clone();
            let m = manager.clone();
            check.connect_toggled(move |c| m.set_source_enabled(id, &url, c.is_active()));
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            line.set_margin_top(4);
            line.set_margin_bottom(4);
            line.append(&check);
            for chars in [34, 14, 6, 10, 10] {
                line.append(&gtk::Label::builder().xalign(0.0).width_chars(chars).ellipsize(gtk::pango::EllipsizeMode::Middle).build());
            }
            list.append(&line);
        }
        if let Some(i) = selected {
            list.select_row(list.row_at_index(i).as_ref());
        }
    }
    for (i, s) in d.sources.iter().enumerate() {
        let Some(line) = list.row_at_index(i as i32).and_then(|r| r.child()) else { continue };
        let mut child = line.first_child();
        if let Some(check) = child.as_ref().and_then(|c| c.downcast_ref::<gtk::CheckButton>()) {
            if check.is_active() != s.enabled {
                check.set_active(s.enabled);
            }
            check.set_tooltip_text(Some(if s.enabled { "Turn this source off; its connections move to the others" } else { "Turn this source on" }));
            check.update_property(&[gtk::accessible::Property::Label(&format!("Use {}", s.url))]);
        }
        let original = if s.url == d.row.url { " (original)" } else { "" };
        for text in [format!("{}{original}", s.url), s.status.text(), if s.active > 0 { s.active.to_string() } else { String::new() }, format::speed_text(s.speed),
                     if s.bytes > 0 { format::bytes(s.bytes) } else { String::new() }] {
            child = child.and_then(|c| c.next_sibling());
            if let Some(label) = child.as_ref().and_then(|c| c.downcast_ref::<gtk::Label>()) {
                show_text(label, &text);
            }
        }
    }
}

/// A list cell's text, also its hover text (a cut-off link can be read whole); failures are red, as on the Mac.
fn show_text(label: &gtk::Label, text: &str) {
    label.set_label(text);
    label.set_tooltip_text(Some(text).filter(|t| !t.is_empty()));
    if text.starts_with("Failed") || text == "Different file" || text == "File changed" {
        label.add_css_class("error");
    } else {
        label.remove_css_class("error");
    }
}

/// The header's bar, as the Mac draws it: a finished download all green; otherwise each segment's downloaded part
/// green once the segment is complete, in the accent colour while a connection works on it and grey while it waits,
/// with a thin line where each segment starts; the gaps still to fetch left empty.
fn draw_segments(area: &gtk::DrawingArea, cr: &gtk::cairo::Context, w: f64, h: f64, d: &Detail) {
    let rgb = |c: gtk::gdk::RGBA| cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
    let named = |name: &str, fallback: &str| {
        #[allow(deprecated)]
        area.style_context().lookup_color(name).unwrap_or_else(|| gtk::gdk::RGBA::parse(fallback).expect("a colour"))
    };
    let (green, accent, grey) = (named("success_bg_color", "#26a269"), named("accent_bg_color", "#3584e4"), gtk::gdk::RGBA::parse("#9a9996").expect("a colour"));
    let r = 4.0_f64.min(h / 2.0);
    cr.new_sub_path();
    cr.arc(w - r, r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(w - r, h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(r, h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(r, r, r, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
    cr.close_path();
    cr.set_source_rgba(0.5, 0.5, 0.5, 0.25);
    let _ = cr.fill_preserve();
    cr.clip();
    let size = d.row.size.unwrap_or(0);
    if size == 0 {
        return;
    }
    if d.row.status == Status::Finished {
        rgb(green);
        cr.rectangle(0.0, 0.0, w, h);
        let _ = cr.fill();
        return;
    }
    let scale = w / size as f64;
    for s in d.segments.iter().filter(|s| s.pos > s.start) {
        rgb(if s.pos >= s.end { green } else if s.owner.is_some() { accent } else { grey });
        cr.rectangle(s.start as f64 * scale, 0.0, ((s.pos - s.start) as f64 * scale).max(1.0), h);
        let _ = cr.fill();
    }
    // The window's background, see-through, where each segment after the first starts.
    let mut bg = named("window_bg_color", "#ffffff");
    bg.set_alpha(0.6);
    rgb(bg);
    for s in d.segments.iter().skip(1) {
        cr.rectangle(s.start as f64 * scale, 0.0, 1.0, h);
        let _ = cr.fill();
    }
}

/// Add Mirror… (§7.1 M2, M3): one address per line.
pub fn ask_mirrors(ctx: &Ctx, id: u64) {
    let Some(name) = ctx.manager.detail(id).map(|d| d.row.name) else { return };
    let dialog = adw::AlertDialog::new(Some("Add Mirror"), Some("Other servers with the same file, one address per line. A running download starts using them at once."));
    let text = gtk::TextView::builder().wrap_mode(gtk::WrapMode::None).build();
    let scroll = gtk::ScrolledWindow::builder().child(&text).min_content_height(90).min_content_width(420).has_frame(true).build();
    dialog.set_extra_child(Some(&scroll));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("add", "Add");
    dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("add"));
    let ctx2 = ctx.clone();
    dialog.connect_response(Some("add"), move |_, _| {
        let buffer = text.buffer();
        let links = hd_core::links::find(&buffer.text(&buffer.start_iter(), &buffer.end_iter(), false));
        if links.is_empty() {
            return;
        }
        let added = ctx2.manager.add_mirrors(id, &links);
        (ctx2.tell)(if added == 0 {
            format!("“{name}” already uses {}", if links.len() == 1 { "that link" } else { "those links" })
        } else {
            format!("Added {added} mirror{} to “{name}”", if added == 1 { "" } else { "s" })
        });
        (ctx2.refresh)();
    });
    let window = ctx.manager.detail(id).and_then(|_| OPEN.with(|o| o.borrow().get(&id).cloned()));
    match window {
        Some(w) => dialog.present(Some(&w)),
        None => dialog.present(Some(&ctx.window)),
    }
}
