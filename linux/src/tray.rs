//! The app's icon in the status area (§2), as on Windows: there while the main window is closed, so it shows the app
//! still runs. A click on it, or Show Herb's Download Manager in its menu, brings the window back; Quit Herb's DM
//! quits (asking first while downloads run). A StatusNotifierItem, which GNOME shows through its AppIndicator
//! extension and KDE itself.

use std::cell::RefCell;

use adw::prelude::*;
use gtk::{gdk_pixbuf, gio, glib};
use ksni::blocking::TrayMethods;

thread_local! {
    static ITEM: RefCell<Option<ksni::blocking::Handle<Item>>> = const { RefCell::new(None) };
}

struct Item {
    icon: Vec<ksni::Icon>,
}

/// Runs one of the app's actions on the main loop (the item answers on its own thread).
fn run(action: &'static str) {
    glib::MainContext::default().invoke(move || {
        if let Some(app) = gio::Application::default() {
            app.activate_action(action, None);
        }
    });
}

impl ksni::Tray for Item {
    fn id(&self) -> String {
        crate::APP_ID.into()
    }
    fn title(&self) -> String {
        "Herb's Download Manager".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icon.clone()
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: "Herb's Download Manager".into(), ..Default::default() }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        run("tray-show");
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem { label: "Show Herb's Download Manager".into(), activate: Box::new(|_| run("tray-show")), ..Default::default() }.into(),
            ksni::MenuItem::Separator,
            StandardItem { label: "Quit Herb's DM".into(), activate: Box::new(|_| run("tray-quit")), ..Default::default() }.into(),
        ]
    }
}

/// The app's icon as the item's pictures (ARGB, big-endian), from the icons the app ships.
fn icon() -> Vec<ksni::Icon> {
    let sources: [&'static [u8]; 2] = [
        include_bytes!("../icons/hicolor/32x32/apps/org.herbsdownloadmanager.app.png"),
        include_bytes!("../icons/hicolor/64x64/apps/org.herbsdownloadmanager.app.png"),
    ];
    sources
        .into_iter()
        .filter_map(|png| {
            let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_static(png));
            let pixbuf = gdk_pixbuf::Pixbuf::from_stream(&stream, gio::Cancellable::NONE).ok()?;
            let (w, h, stride, channels) = (pixbuf.width(), pixbuf.height(), pixbuf.rowstride() as usize, pixbuf.n_channels() as usize);
            let bytes = pixbuf.read_pixel_bytes();
            let mut data = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let p = &bytes[y * stride + x * channels..];
                    let a = if channels == 4 { p[3] } else { 255 };
                    data.extend_from_slice(&[a, p[0], p[1], p[2]]);
                }
            }
            Some(ksni::Icon { width: w, height: h, data })
        })
        .collect()
}

/// Shows the icon while the main window is closed and takes it away when the window is back; adds the actions its
/// menu runs.
pub fn follow(window: &adw::ApplicationWindow) {
    let Some(app) = window.application() else { return };
    let show = gio::SimpleAction::new("tray-show", None);
    let w = window.clone();
    show.connect_activate(move |_, _| w.present());
    app.add_action(&show);
    let quit = gio::SimpleAction::new("tray-quit", None);
    let w = window.clone();
    quit.connect_activate(move |_, _| {
        let _ = WidgetExt::activate_action(&w, "win.quit", None);
    });
    app.add_action(&quit);
    window.connect_visible_notify(|w| set(!w.is_visible()));
}

fn set(on: bool) {
    ITEM.with(|item| {
        let mut item = item.borrow_mut();
        if on && item.is_none() {
            // The Flatpak may not own the item's usual bus name (org.kde.StatusNotifierItem-…): the item is
            // registered by its connection's own name instead, which the status area accepts as well.
            match (Item { icon: icon() }).disable_dbus_name(true).spawn() {
                Ok(handle) => *item = Some(handle),
                Err(e) => eprintln!("No status-area icon: {e}"),
            }
        } else if !on && let Some(handle) = item.take() {
            let _ = handle.shutdown();
        }
    });
}
