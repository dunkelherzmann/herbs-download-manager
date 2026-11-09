//! The main window (§2): title bar with the Main Menu (☰) in its left corner, sidebar filters, the download list and
//! the footer. The list refreshes twice a second, as on the Mac.

use adw::prelude::*;
use gtk::{gio, glib};
use hd_core::format::{self, Filter};
use hd_core::manager::{Manager, Row};
use std::cell::Cell;
use std::rc::Rc;

/// The Main Menu (§4): one submenu per menu, then Settings, About and Quit, as on the Mac.
fn main_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    for title in ["File", "Edit", "Tasks", "View", "Window", "Help"] {
        menu.append_submenu(Some(title), &gio::Menu::new());
    }
    let end = gio::Menu::new();
    end.append(Some("Settings…"), Some("app.settings"));
    end.append(Some("About Herb's Download Manager"), Some("app.about"));
    end.append(Some("Quit Herb's DM"), Some("app.quit"));
    menu.append_section(None, &end);
    menu
}

pub fn build(app: &adw::Application, manager: Manager) -> adw::ApplicationWindow {
    let quit = gio::SimpleAction::new("quit", None);
    let app_for_quit = app.clone();
    quit.connect_activate(move |_, _| app_for_quit.quit());
    app.add_action(&quit);
    app.set_accels_for_action("app.quit", &["<Control>q"]);

    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&main_menu())
        .tooltip_text("Main Menu: every command, from File, Edit, Tasks, View, Window and Help to Settings and Quit")
        .build();
    menu_button.update_property(&[gtk::accessible::Property::Label("Main Menu")]);
    let header = adw::HeaderBar::new();
    header.pack_start(&menu_button);
    header.set_title_widget(Some(&adw::WindowTitle::new("Herb's Download Manager", "")));

    // Sidebar: the status filters with their counts (§2.1).
    let sidebar = gtk::ListBox::new();
    sidebar.add_css_class("navigation-sidebar");
    let counts: Vec<gtk::Label> = Filter::ALL
        .iter()
        .map(|filter| {
            let name = gtk::Label::builder().label(filter.title()).xalign(0.0).hexpand(true).build();
            let count = gtk::Label::new(Some("0"));
            count.add_css_class("dim-label");
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.append(&name);
            row.append(&count);
            sidebar.append(&row);
            count
        })
        .collect();
    sidebar.select_row(sidebar.row_at_index(0).as_ref());

    // The download list: one row per download, newest first.
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Multiple);
    list.add_css_class("rich-list");
    let empty = adw::StatusPage::builder().title("No downloads yet").build();
    list.set_placeholder(Some(&empty));
    let scroller = gtk::ScrolledWindow::builder().child(&list).vexpand(true).hexpand(true).build();

    let split = gtk::Paned::new(gtk::Orientation::Horizontal);
    split.set_start_child(Some(&gtk::ScrolledWindow::builder().child(&sidebar).width_request(190).build()));
    split.set_end_child(Some(&scroller));
    split.set_position(190);
    split.set_shrink_start_child(false);

    let footer = gtk::Label::builder().xalign(0.0).margin_start(12).margin_top(4).margin_bottom(4).build();
    footer.add_css_class("caption");

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&split));
    view.add_bottom_bar(&footer);

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Herb's Download Manager")
        .default_width(1100)
        .default_height(640)
        .content(&view)
        .build();

    let filter = Rc::new(Cell::new(Filter::All));
    let refresh = {
        let (list, footer, filter, manager) = (list.clone(), footer.clone(), filter.clone(), manager.clone());
        move || {
            let rows = manager.rows();
            for (label, f) in counts.iter().zip(Filter::ALL) {
                label.set_label(&rows.iter().filter(|r| f.matches(&r.status)).count().to_string());
            }
            show_rows(&list, rows.iter().filter(|r| filter.get().matches(&r.status)));
            let active = rows.iter().filter(|r| r.status.is_running()).count();
            let mut parts = vec![format!("{} selected", list.selected_rows().len()), format!("{active} active")];
            let speed: u64 = rows.iter().map(|r| r.speed).sum();
            if speed > 0 {
                parts.push(format::speed_text(speed));
            }
            parts.push(if manager.queue_running() { "Queue running".into() } else { "Queue stopped".into() });
            footer.set_label(&parts.join("  ·  "));
        }
    };
    refresh();
    {
        let refresh = refresh.clone();
        sidebar.connect_row_selected(move |_, row| {
            if let Some(row) = row {
                filter.set(Filter::ALL[row.index().max(0) as usize]);
                refresh();
            }
        });
    }
    glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
        refresh();
        glib::ControlFlow::Continue
    });
    window
}

/// Rebuilds the list's rows: name, status, size and speed.
fn show_rows<'a>(list: &gtk::ListBox, rows: impl Iterator<Item = &'a Row>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    for r in rows {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        line.set_margin_top(6);
        line.set_margin_bottom(6);
        let name = gtk::Label::builder().label(&r.name).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::Middle).build();
        name.set_tooltip_text(Some(&r.url));
        let status = gtk::Label::builder().label(&r.status_text).xalign(0.0).width_chars(16).build();
        let size = gtk::Label::builder().label(format::size_text(r.downloaded, r.size, r.status == hd_core::manager::Status::Finished)).xalign(0.0).width_chars(18).build();
        let speed = gtk::Label::builder().label(format::speed_text(r.speed)).xalign(0.0).width_chars(10).build();
        for w in [&name, &status, &size, &speed] {
            line.append(w);
        }
        line.update_property(&[gtk::accessible::Property::Label(&format!("{}, {}", r.name, r.status_text))]);
        list.append(&line);
    }
}

/// Saves the window as a PNG a second after it appears (the test hook behind `--capture`), then quits.
pub fn capture_then_quit(app: &adw::Application, window: &adw::ApplicationWindow, path: std::path::PathBuf) {
    let (app, window) = (app.clone(), window.clone());
    glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
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
