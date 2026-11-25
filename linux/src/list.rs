//! The download list (§2.2): a ColumnView with the Mac's ten columns (L1–L10), all sortable, every one but Name can
//! be hidden from the header's menu. Each item is a download's id; the cells read the latest rows from a shared map
//! and are updated in place twice a second, so the selection and scroll position never jump.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use hd_core::format::{self, FileKind, Filter};
use hd_core::manager::{Row, Status};

/// Column ids, titles and their initial widths; L1 (Name) can't be hidden.
/// Their widths fit the window's first size (1100 wide: the list is 910) with every shown column whole; Name takes
/// what's left (bugs.md: Date Added was cut off past the edge).
pub const COLUMNS: [(&str, &str, i32); 11] = [
    ("name", "Name", 160),
    ("note", "Note", 80),
    ("status", "Status", 100),
    ("size", "Size", 100),
    ("speed", "Speed", 66),
    ("time-left", "Time Left", 74),
    ("connections", "Connections", 88),
    ("mirrors", "Mirrors", 62),
    ("added", "Date Added", 150),
    ("finished", "Date Finished", 150),
    ("location", "Location", 200),
];

/// Shown until the user hides them, as on the Mac (Date Finished and Location start hidden).
pub fn shown_at_first(key: &str) -> bool {
    !matches!(key, "finished" | "location")
}

type Cells = Rc<RefCell<HashMap<(u64, &'static str), glib::WeakRef<gtk::Widget>>>>;
/// What links dropped on a download do (they become its mirrors, §7.1 M4), set by the window.
pub type RowDrop = Rc<RefCell<Option<Rc<dyn Fn(u64, Vec<String>)>>>>;
/// What a right-click on a download does (§4.1): its id, and the point in the list's coordinates.
pub type RowMenu = Rc<RefCell<Option<Rc<dyn Fn(u64, f64, f64)>>>>;

#[derive(Clone)]
pub struct DownloadList {
    pub view: gtk::ColumnView,
    pub selection: gtk::MultiSelection,
    store: gio::ListStore,
    sorter: gtk::Sorter,
    rows: Rc<RefCell<HashMap<u64, Row>>>,
    cells: Cells,
    order: Rc<RefCell<Vec<u64>>>,
    pub on_drop: RowDrop,
    pub on_menu: RowMenu,
}

/// "Today at 10:48", "Yesterday at 09:02", or "3 Nov 2026 at 17:20".
pub fn date_text(seconds: Option<u64>) -> String {
    let Some(s) = seconds else { return String::new() };
    let Ok(when) = glib::DateTime::from_unix_local(s as i64) else { return String::new() };
    let Ok(now) = glib::DateTime::now_local() else { return String::new() };
    let time = when.format("%H:%M").map(|t| t.to_string()).unwrap_or_default();
    let day = |d: &glib::DateTime| (d.year(), d.day_of_year());
    if day(&when) == day(&now) {
        format!("Today at {time}")
    } else if now.add_days(-1).map(|y| day(&y) == day(&when)).unwrap_or(false) {
        format!("Yesterday at {time}")
    } else {
        format!("{} at {time}", when.format("%-d %b %Y").map(|t| t.to_string()).unwrap_or_default())
    }
}

/// "1 Oct 2026 at 01:59", as the Mac's Download window writes dates (never "Today").
pub fn full_date_text(seconds: u64) -> String {
    glib::DateTime::from_unix_local(seconds as i64).ok().and_then(|d| d.format("%-d %b %Y at %H:%M").ok())
        .map(|t| t.to_string()).unwrap_or_default()
}

/// A column's text for a row (L2–L10).
fn cell_text(column: &str, r: &Row) -> String {
    let finished = r.status == Status::Finished;
    match column {
        "status" => r.status_text.clone(),
        "size" => format::size_text(r.downloaded, r.size, finished),
        "speed" => format::speed_text(r.speed),
        "time-left" => format::time_left_text(r.time_left),
        "connections" => r.connections_text.clone(),
        "mirrors" => r.mirrors_text.clone(),
        "added" => date_text(Some(r.added)),
        "finished" => date_text(r.finished),
        "location" => tilde(&r.folder),
        "note" => r.note.clone(),
        _ => String::new(),
    }
}

/// A folder as the Mac shows it: the home folder as ~.
pub fn tilde(path: &std::path::Path) -> String {
    match path.strip_prefix(glib::home_dir()) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// A cell's right-click: selects its download when it isn't selected and opens the menu (§4.1).
fn row_menu_gesture(cell: &gtk::Widget, cells: Cells, on_menu: RowMenu, view: glib::WeakRef<gtk::ColumnView>) -> gtk::GestureClick {
    let click = gtk::GestureClick::builder().button(gtk::gdk::BUTTON_SECONDARY).build();
    let cell = cell.downgrade();
    click.connect_pressed(move |g, _, x, y| {
        let (Some(cell), Some(view)) = (cell.upgrade(), view.upgrade()) else { return };
        let id = cells.borrow().iter().find(|(_, w)| w.upgrade().as_ref() == Some(&cell)).map(|((id, _), _)| *id);
        let (Some(id), Some(run)) = (id, on_menu.borrow().clone()) else { return };
        g.set_state(gtk::EventSequenceState::Claimed);
        let point = cell.compute_point(&view, &gtk::graphene::Point::new(x as f32, y as f32));
        let (px, py) = point.map(|p| (p.x() as f64, p.y() as f64)).unwrap_or((x, y));
        run(id, px, py);
    });
    click
}

/// The file type's icon from the desktop's icon theme, by its name's content type.
fn file_icon(name: &str) -> gio::Icon {
    let (content_type, _) = gio::content_type_guess(Some(name), None);
    gio::content_type_get_symbolic_icon(&content_type)
}

/// Hover text on each column header: its title.
fn header_hover(view: &gtk::ColumnView) {
    let mut child = view.first_child();
    while let Some(c) = child {
        if c.css_name() == "header" {
            let mut title = c.first_child();
            while let Some(t) = title {
                let mut stack = vec![t.clone()];
                while let Some(w) = stack.pop() {
                    if let Some(l) = w.downcast_ref::<gtk::Label>() {
                        t.set_tooltip_text(Some(&l.label()).filter(|s| !s.is_empty()).map(|s| s.as_str()));
                        break;
                    }
                    let mut g = w.first_child();
                    while let Some(x) = g {
                        g = x.next_sibling();
                        stack.push(x);
                    }
                }
                title = t.next_sibling();
            }
        }
        child = c.next_sibling();
    }
}

/// A cell's drop target: links (text, web-page text with its links, or files holding links) dropped on it go to its
/// download's mirrors.
fn row_drop_target(cell: &gtk::Widget, cells: Cells, on_drop: RowDrop) -> gtk::DropTargetAsync {
    let cell = cell.downgrade();
    crate::add::links_drop(|_| {}, move |links| {
        let Some(cell) = cell.upgrade() else { return };
        let id = cells.borrow().iter().find(|(_, w)| w.upgrade().as_ref() == Some(&cell)).map(|((id, _), _)| *id);
        let (Some(id), Some(run)) = (id, on_drop.borrow().clone()) else { return };
        if !links.is_empty() {
            run(id, links);
        }
    })
}

impl DownloadList {
    pub fn new() -> DownloadList {
        let store = gio::ListStore::new::<gtk::StringObject>();
        let rows: Rc<RefCell<HashMap<u64, Row>>> = Rc::default();
        let cells: Cells = Rc::default();
        let on_drop: RowDrop = Rc::default();
        let on_menu: RowMenu = Rc::default();
        let view = gtk::ColumnView::builder().show_column_separators(true).reorderable(true).build();
        let weak_view = view.downgrade();
        view.add_css_class("data-table");
        let id_of = |o: &glib::Object| o.downcast_ref::<gtk::StringObject>().and_then(|s| s.string().parse::<u64>().ok());
        let header_menu = gio::Menu::new();

        for (key, title, width) in COLUMNS {
            let factory = gtk::SignalListItemFactory::new();
            let (cells_d, on_drop_d, on_menu_d, view_d) = (cells.clone(), on_drop.clone(), on_menu.clone(), weak_view.clone());
            factory.connect_setup(move |_, item| {
                let item = item.downcast_ref::<gtk::ListItem>().unwrap();
                if key == "name" {
                    let icon = gtk::Image::new();
                    let label = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::Middle).build();
                    let bar = gtk::ProgressBar::builder().visible(false).build();
                    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
                    text.append(&label);
                    text.append(&bar);
                    text.set_valign(gtk::Align::Center);
                    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    line.append(&icon);
                    line.append(&text);
                    item.set_child(Some(&line));
                } else {
                    item.set_child(Some(&gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).build()));
                }
                // Links dropped on a download become its mirrors (§7.1 M4); elsewhere they open Add Links.
                if let Some(child) = item.child() {
                    child.add_controller(row_drop_target(&child, cells_d.clone(), on_drop_d.clone()));
                    child.add_controller(row_menu_gesture(&child, cells_d.clone(), on_menu_d.clone(), view_d.clone()));
                }
            });
            let (rows_b, cells_b) = (rows.clone(), cells.clone());
            factory.connect_bind(move |_, item| {
                let item = item.downcast_ref::<gtk::ListItem>().unwrap();
                let (Some(id), Some(child)) = (item.item().as_ref().and_then(id_of), item.child()) else { return };
                if let Some(r) = rows_b.borrow().get(&id) {
                    show_cell(key, &child, r);
                }
                cells_b.borrow_mut().insert((id, key), child.downgrade());
            });
            let cells_u = cells.clone();
            factory.connect_unbind(move |_, item| {
                let item = item.downcast_ref::<gtk::ListItem>().unwrap();
                if let Some(id) = item.item().as_ref().and_then(id_of) {
                    cells_u.borrow_mut().remove(&(id, key));
                }
            });
            let column = gtk::ColumnViewColumn::builder().title(title).factory(&factory).resizable(true).fixed_width(width).build();
            column.set_id(Some(key));
            column.set_visible(shown_at_first(key));
            if key == "name" {
                column.set_expand(true);
            } else {
                header_menu.append(Some(title), Some(&format!("win.column-{key}")));
            }
            let rows_s = rows.clone();
            let sort_key = key;
            column.set_sorter(Some(&gtk::CustomSorter::new(move |a, b| {
                let rows = rows_s.borrow();
                let (Some(a), Some(b)) = (id_of(a).and_then(|i| rows.get(&i)), id_of(b).and_then(|i| rows.get(&i))) else {
                    return gtk::Ordering::Equal;
                };
                compare(sort_key, a, b).into()
            })));
            view.append_column(&column);
        }
        for c in 0..view.columns().n_items() {
            if let Some(column) = view.columns().item(c).and_downcast::<gtk::ColumnViewColumn>() {
                column.set_header_menu(Some(&header_menu));
            }
        }
        let sorter = view.sorter().unwrap();
        let sorted = gtk::SortListModel::new(Some(store.clone()), Some(sorter.clone()));
        let selection = gtk::MultiSelection::new(Some(sorted));
        view.set_model(Some(&selection));
        // Each column's title on hover over its header, as on the Mac (GTK has no call for it: the headers are found
        // once shown, and again when columns move).
        view.connect_map(|v| header_hover(v));
        {
            let weak = view.downgrade();
            view.columns().connect_items_changed(move |_, _, _, _| {
                let weak = weak.clone();
                glib::idle_add_local_once(move || if let Some(v) = weak.upgrade() { header_hover(&v) });
            });
        }
        let list = DownloadList { view, selection, store, sorter, rows, cells, order: Rc::default(), on_drop, on_menu };
        // Date Added, newest first (L8's default).
        if let Some(added) = list.column("added") {
            list.view.sort_by_column(Some(&added), gtk::SortType::Descending);
        }
        list
    }

    /// Shows `rows` filtered by status, file type and search text, updating cells in place when the same downloads
    /// are listed and rebuilding the list only when they change.
    pub fn show(&self, all: &[Row], filter: Filter, kind: Option<FileKind>, search: &str) -> usize {
        let shown: Vec<&Row> = all
            .iter()
            .filter(|r| filter.matches(&r.status))
            .filter(|r| kind.is_none_or(|k| r.kind == k))
            .filter(|r| r.matches(search))
            .collect();
        *self.rows.borrow_mut() = shown.iter().map(|r| (r.id, (*r).clone())).collect();
        let ids: Vec<u64> = shown.iter().map(|r| r.id).collect();
        if *self.order.borrow() != ids {
            let items: Vec<gtk::StringObject> = ids.iter().map(|id| gtk::StringObject::new(&id.to_string())).collect();
            self.store.splice(0, self.store.n_items(), &items);
            *self.order.borrow_mut() = ids;
        } else {
            for ((id, key), cell) in self.cells.borrow().iter() {
                if let (Some(widget), Some(r)) = (cell.upgrade(), self.rows.borrow().get(id)) {
                    show_cell(key, &widget, r);
                }
            }
            // Keep a sort by a changing column (speed, status…) current.
            if let Some(column) = self.view.sorter().and_downcast::<gtk::ColumnViewSorter>().and_then(|s| s.primary_sort_column())
                && !matches!(column.id().as_deref(), Some("name" | "added" | "finished" | "location" | "note"))
            {
                self.sorter.changed(gtk::SorterChange::Different);
            }
        }
        shown.len()
    }

    /// The selected downloads' rows, in list order.
    pub fn selected(&self) -> Vec<Row> {
        let bits = self.selection.selection();
        let rows = self.rows.borrow();
        (0..bits.size())
            .filter_map(|i| self.selection.item(bits.nth(i as u32)))
            .filter_map(|o| o.downcast_ref::<gtk::StringObject>().and_then(|s| s.string().parse::<u64>().ok()))
            .filter_map(|id| rows.get(&id).cloned())
            .collect()
    }

    /// Selects these downloads (the feature tests' way to point a command at them).
    pub fn select(&self, ids: &[u64]) {
        self.selection.unselect_all();
        for position in 0..self.selection.n_items() {
            let id = self.selection.item(position).and_downcast::<gtk::StringObject>().and_then(|s| s.string().parse::<u64>().ok());
            if id.is_some_and(|id| ids.contains(&id)) {
                self.selection.select_item(position, false);
            }
        }
    }

    /// The row at a list position (for double-click).
    pub fn row_at(&self, position: u32) -> Option<Row> {
        let id = self.selection.item(position)?.downcast_ref::<gtk::StringObject>()?.string().parse::<u64>().ok()?;
        self.rows.borrow().get(&id).cloned()
    }

    /// A column by its key.
    pub fn column(&self, key: &str) -> Option<gtk::ColumnViewColumn> {
        let columns = self.view.columns();
        (0..columns.n_items())
            .filter_map(|c| columns.item(c).and_downcast::<gtk::ColumnViewColumn>())
            .find(|c| c.id().as_deref() == Some(key))
    }

    /// The columns' layout, as one line per column in their order: `key width shown`, then `sort key ascending`.
    pub fn layout_text(&self) -> String {
        let columns = self.view.columns();
        let mut out = String::new();
        for c in (0..columns.n_items()).filter_map(|c| columns.item(c).and_downcast::<gtk::ColumnViewColumn>()) {
            out.push_str(&format!("column {} {} {}\n", c.id().unwrap_or_default(), c.fixed_width(), c.is_visible() as u8));
        }
        if let Some(sorter) = self.view.sorter().and_downcast::<gtk::ColumnViewSorter>()
            && let Some(column) = sorter.primary_sort_column()
        {
            let ascending = sorter.primary_sort_order() == gtk::SortType::Ascending;
            out.push_str(&format!("sort {} {}\n", column.id().unwrap_or_default(), ascending as u8));
        }
        out
    }

    /// Puts back a layout saved by `layout_text`; returns the columns shown, for View ▸ Columns' checkmarks.
    pub fn apply_layout(&self, text: &str) {
        let mut position = 0;
        for line in text.lines() {
            let parts: Vec<&str> = line.split(' ').collect();
            match parts.as_slice() {
                ["column", key, width, shown] => {
                    if let Some(c) = self.column(key) {
                        if let Ok(w) = width.parse::<i32>() && w > 20 {
                            c.set_fixed_width(w);
                        }
                        if *key != "name" {
                            c.set_visible(*shown == "1");
                        }
                        self.view.remove_column(&c);
                        self.view.insert_column(position, &c);
                        position += 1;
                    }
                }
                ["sort", key, ascending] => {
                    if let Some(c) = self.column(key) {
                        let order = if *ascending == "1" { gtk::SortType::Ascending } else { gtk::SortType::Descending };
                        self.view.sort_by_column(Some(&c), order);
                    }
                }
                _ => {}
            }
        }
    }

    pub fn set_column_visible(&self, key: &str, visible: bool) {
        let columns = self.view.columns();
        for c in 0..columns.n_items() {
            if let Some(column) = columns.item(c).and_downcast::<gtk::ColumnViewColumn>()
                && column.id().as_deref() == Some(key)
            {
                column.set_visible(visible);
            }
        }
    }
}

fn compare(key: &str, a: &Row, b: &Row) -> std::cmp::Ordering {
    match key {
        "name" => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        "status" => a.status_text.cmp(&b.status_text),
        "size" => a.size.cmp(&b.size).then(a.downloaded.cmp(&b.downloaded)),
        "speed" => a.speed.cmp(&b.speed),
        "time-left" => a.time_left.cmp(&b.time_left),
        "connections" => a.connections_sort.cmp(&b.connections_sort),
        "mirrors" => a.mirrors_sort.cmp(&b.mirrors_sort),
        "finished" => a.finished.cmp(&b.finished),
        "location" => a.folder.cmp(&b.folder),
        "note" => a.note.to_lowercase().cmp(&b.note.to_lowercase()),
        _ => a.added.cmp(&b.added).then(a.id.cmp(&b.id)),
    }
}

/// Fills one cell: L1 is the icon, name and progress bar; the rest are text.
fn show_cell(key: &str, widget: &gtk::Widget, r: &Row) {
    if key == "name" {
        let Some(line) = widget.downcast_ref::<gtk::Box>() else { return };
        let icon = line.first_child().and_downcast::<gtk::Image>();
        let text = line.last_child().and_downcast::<gtk::Box>();
        if let Some(icon) = icon {
            icon.set_from_gicon(&file_icon(&r.name));
        }
        if let Some(text) = text {
            if let Some(label) = text.first_child().and_downcast::<gtk::Label>() {
                label.set_label(&r.name);
                label.set_tooltip_text(Some(&r.url));
            }
            if let Some(bar) = text.last_child().and_downcast::<gtk::ProgressBar>() {
                let unfinished = r.status != Status::Finished && (r.fraction.is_some() || r.status.is_running());
                bar.set_visible(unfinished);
                match r.fraction {
                    Some(f) => bar.set_fraction(f),
                    None if unfinished => bar.pulse(),
                    None => {}
                }
            }
        }
        line.update_property(&[gtk::accessible::Property::Label(&format!("{}, {}", r.name, r.status_text))]);
    } else if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        let text = cell_text(key, r);
        // Status, Location and Note can be cut short in their columns: the whole text shows on hover, as on the Mac.
        if matches!(key, "status" | "location" | "note") {
            label.set_tooltip_text(Some(&text).filter(|t| !t.is_empty()).map(|t| t.as_str()));
        }
        // A failed download's status is red (§2.2).
        if key == "status" && matches!(r.status, Status::Failed(_)) {
            label.add_css_class("error");
        } else {
            label.remove_css_class("error");
        }
        label.set_label(&text);
    }
}
