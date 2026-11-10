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
pub const COLUMNS: [(&str, &str, i32); 11] = [
    ("name", "Name", 200),
    ("note", "Note", 160),
    ("status", "Status", 120),
    ("size", "Size", 140),
    ("speed", "Speed", 80),
    ("time-left", "Time Left", 80),
    ("connections", "Connections", 90),
    ("mirrors", "Mirrors", 70),
    ("added", "Date Added", 130),
    ("finished", "Date Finished", 130),
    ("location", "Location", 200),
];

/// Shown until the user hides them, as on the Mac (Date Finished and Location start hidden).
pub fn shown_at_first(key: &str) -> bool {
    !matches!(key, "finished" | "location")
}

type Cells = Rc<RefCell<HashMap<(u64, &'static str), glib::WeakRef<gtk::Widget>>>>;

#[derive(Clone)]
pub struct DownloadList {
    pub view: gtk::ColumnView,
    pub selection: gtk::MultiSelection,
    store: gio::ListStore,
    sorter: gtk::Sorter,
    rows: Rc<RefCell<HashMap<u64, Row>>>,
    cells: Cells,
    order: Rc<RefCell<Vec<u64>>>,
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

/// A column's text for a row (L2–L10).
fn cell_text(column: &str, r: &Row) -> String {
    let finished = r.status == Status::Finished;
    match column {
        "status" => r.status_text.clone(),
        "size" => format::size_text(r.downloaded, r.size, finished),
        "speed" => format::speed_text(r.speed),
        "time-left" => format::time_left_text(r.time_left),
        "connections" if !finished => format!("{} / {}", r.active_connections, r.allowed_connections),
        "mirrors" => format!("{} / {}", r.sources_in_use, r.sources_known),
        "added" => date_text(Some(r.added)),
        "finished" => date_text(r.finished),
        "location" => r.folder.display().to_string(),
        "note" => r.note.clone(),
        _ => String::new(),
    }
}

/// The file type's icon from the desktop's icon theme, by its name's content type.
fn file_icon(name: &str) -> gio::Icon {
    let (content_type, _) = gio::content_type_guess(Some(name), None);
    gio::content_type_get_symbolic_icon(&content_type)
}

impl DownloadList {
    pub fn new() -> DownloadList {
        let store = gio::ListStore::new::<gtk::StringObject>();
        let rows: Rc<RefCell<HashMap<u64, Row>>> = Rc::default();
        let cells: Cells = Rc::default();
        let view = gtk::ColumnView::builder().show_column_separators(true).reorderable(true).build();
        view.add_css_class("data-table");
        let id_of = |o: &glib::Object| o.downcast_ref::<gtk::StringObject>().and_then(|s| s.string().parse::<u64>().ok());
        let header_menu = gio::Menu::new();

        for (key, title, width) in COLUMNS {
            let factory = gtk::SignalListItemFactory::new();
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
        // Date Added, newest first (L8's default).
        if let Some(added) = view.columns().item(7).and_downcast::<gtk::ColumnViewColumn>() {
            view.sort_by_column(Some(&added), gtk::SortType::Descending);
        }
        DownloadList { view, selection, store, sorter, rows, cells, order: Rc::default() }
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
        "connections" => a.active_connections.cmp(&b.active_connections),
        "mirrors" => a.sources_known.cmp(&b.sources_known),
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
        // A long note is cut short in its column: the whole of it shows on hover.
        if key == "note" {
            label.set_tooltip_text(Some(&text).filter(|t| !t.is_empty()).map(|t| t.as_str()));
        }
        label.set_label(&text);
    }
}
