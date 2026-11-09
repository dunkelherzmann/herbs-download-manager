//! Settings (§9, Ctrl+,): applied and saved (in the core's settings.json) as soon as something changes, as on the
//! Mac. S8 (checksum files) arrives with checksum verification.

use adw::prelude::*;
use gtk::gio;
use hd_core::settings::{Settings, WhenFileExists};

use crate::add::Ctx;

const EXISTS: [(&str, WhenFileExists); 4] = [
    ("Ask", WhenFileExists::Ask),
    ("Keep both", WhenFileExists::KeepBoth),
    ("Skip the new link", WhenFileExists::Skip),
    ("Replace the existing file", WhenFileExists::Replace),
];

fn spin(title: &str, subtitle: &str, low: f64, high: f64, value: u32) -> adw::SpinRow {
    let row = adw::SpinRow::with_range(low, high, 1.0);
    row.set_title(title);
    row.set_subtitle(subtitle);
    row.set_value(value as f64);
    row
}

pub fn open(ctx: &Ctx) {
    let s = ctx.manager.settings();
    let page = adw::PreferencesPage::new();
    let downloads = adw::PreferencesGroup::builder().title("Downloads").build();
    let connections = adw::PreferencesGroup::builder().title("Connections").build();
    let behaviour = adw::PreferencesGroup::builder().title("When adding and finishing").build();
    page.add(&downloads);
    page.add(&connections);
    page.add(&behaviour);
    page.add(&crate::signins::group(ctx));

    // S1: Save downloads to.
    let folder = adw::ActionRow::builder().title("Save downloads to").subtitle(crate::save_folder(&ctx.manager).display().to_string()).build();
    let choose = gtk::Button::builder().label("Choose…").valign(gtk::Align::Center).tooltip_text("Choose the folder new downloads are saved to").build();
    let reset = gtk::Button::builder().label("Use Downloads").valign(gtk::Align::Center).tooltip_text("Save new downloads to your Downloads folder").build();
    folder.add_suffix(&choose);
    folder.add_suffix(&reset);
    downloads.add(&folder);
    // S4, S5.
    let at_once = spin("Downloads at once", "Downloads the queue runs at the same time", 1.0, 10.0, s.downloads_at_once);
    downloads.add(&at_once);
    let limit = adw::ExpanderRow::builder().title("Speed limit").subtitle("All downloads together").show_enable_switch(true)
        .enable_expansion(s.speed_limit.is_some()).build();
    let limit_value = adw::SpinRow::with_range(1.0, 1_000_000.0, 50.0);
    limit_value.set_title("Limit to");
    let unit = adw::ComboRow::builder().title("Unit").model(&gtk::StringList::new(&["KB/s", "MB/s"])).build();
    match s.speed_limit {
        Some(l) if l >= 1_000_000 && l % 100_000 == 0 => {
            unit.set_selected(1);
            limit_value.set_value(l as f64 / 1_000_000.0);
        }
        Some(l) => limit_value.set_value(l as f64 / 1_000.0),
        None => limit_value.set_value(500.0),
    }
    limit.add_row(&limit_value);
    limit.add_row(&unit);
    crate::explain_row(&limit, "Limit the speed of all downloads together");
    downloads.add(&limit);

    // S2, S3, S6.
    let per_download = spin("Connections per download", "The most a new download uses (New Download can choose another)", 1.0, 32.0, s.connections);
    let per_server = spin("Connections per server", "To any one server, from a download's next start", 1.0, 32.0, s.connections_per_server);
    let retries = spin("Retries per source", "Times a link or mirror is tried again before it counts as failed", 0.0, 20.0, s.retries);
    connections.add(&per_download);
    connections.add(&per_server);
    connections.add(&retries);

    // S7, S9.
    let exists = adw::ComboRow::builder().title("When a file exists").subtitle("A different file already has a new download's name")
        .model(&gtk::StringList::new(&EXISTS.map(|e| e.0))).build();
    exists.set_selected(EXISTS.iter().position(|e| e.1 == s.when_file_exists).unwrap_or(0) as u32);
    let notify = adw::SwitchRow::builder().title("Show a notification when a download finishes").active(s.notify_when_finished).build();
    let checksum_files = adw::SwitchRow::builder().title("Look for checksum files next to the link")
        .subtitle("With no checksum set, check the file with the site's file.sha256 (or .sha512, .sha1, .md5), if it has one")
        .active(s.look_for_checksum_files).build();
    crate::explain_row(&exists, "What happens when a different file already has a new download's name");
    crate::explain_row(&notify, "Show a notification each time a download finishes");
    crate::explain_row(&checksum_files, "With no checksum set, check the file with the site's file.sha256 (or .sha512, .sha1, .md5)");
    let server_date = adw::SwitchRow::builder().title("Keep the server's date for finished files")
        .subtitle("The date the server gives the file, instead of when it finished downloading")
        .active(s.keep_server_date).build();
    crate::explain_row(&server_date, "A finished file gets the date the server gives it (Last-Modified) instead of the time it finished downloading");
    behaviour.add(&exists);
    behaviour.add(&checksum_files);
    behaviour.add(&server_date);
    behaviour.add(&notify);

    // Remembered connection limits: servers that turned away extra connections, for a week.
    let limits = adw::ActionRow::builder().title("Server limits").build();
    let forget = gtk::Button::builder().label("Forget").valign(gtk::Align::Center).build();
    forget.set_tooltip_text(Some("Forget every server's remembered limit; downloads learn them again"));
    limits.add_suffix(&forget);
    let show_limits = {
        let (manager, limits, forget) = (ctx.manager.clone(), limits.clone(), forget.clone());
        move || {
            let all = manager.host_limits();
            let (summary, detail) = hd_core::hosts::describe(&all);
            limits.set_subtitle(&summary);
            crate::explain_row(&limits, &detail);
            forget.set_sensitive(!all.is_empty());
        }
    };
    show_limits();
    {
        let manager = ctx.manager.clone();
        forget.connect_clicked(move |_| {
            manager.forget_host_limits();
            show_limits();
        });
    }
    connections.add(&limits);

    let save = {
        let (manager, limit, limit_value, unit) = (ctx.manager.clone(), limit.clone(), limit_value.clone(), unit.clone());
        let (at_once, per_download, per_server, retries, exists, notify, checksum_files) = (at_once.clone(), per_download.clone(),
            per_server.clone(), retries.clone(), exists.clone(), notify.clone(), checksum_files.clone());
        let server_date = server_date.clone();
        move || {
            let s = Settings {
                downloads_at_once: at_once.value() as u32,
                speed_limit: limit.enables_expansion()
                    .then(|| (limit_value.value() * if unit.selected() == 1 { 1_000_000.0 } else { 1_000.0 }) as u64),
                connections: per_download.value() as u32,
                connections_per_server: per_server.value() as u32,
                retries: retries.value() as u32,
                when_file_exists: EXISTS[(exists.selected() as usize).min(3)].1,
                notify_when_finished: notify.is_active(),
                look_for_checksum_files: checksum_files.is_active(),
                keep_server_date: server_date.is_active(),
                ..manager.settings()
            };
            manager.set_settings(s);
        }
    };
    for row in [&at_once, &per_download, &per_server, &retries, &limit_value] {
        let save = save.clone();
        row.connect_value_notify(move |_| save());
    }
    for row in [&unit, &exists] {
        let save = save.clone();
        row.connect_selected_notify(move |_| save());
    }
    {
        let save = save.clone();
        limit.connect_enable_expansion_notify(move |_| save());
    }
    {
        let save = save.clone();
        checksum_files.connect_active_notify(move |_| save());
    }
    {
        let save = save.clone();
        server_date.connect_active_notify(move |_| save());
    }
    notify.connect_active_notify(move |_| save());

    let dialog = adw::PreferencesDialog::builder().title("Settings").build();
    dialog.add(&page);
    {
        let (manager, folder2) = (ctx.manager.clone(), folder.clone());
        let dialog2 = dialog.clone();
        choose.connect_clicked(move |_| {
            let picker = gtk::FileDialog::builder().title("Save downloads to").build();
            let (manager, folder) = (manager.clone(), folder2.clone());
            let root = dialog2.root().and_downcast::<gtk::Window>();
            picker.select_folder(root.as_ref(), None::<&gio::Cancellable>, move |result| {
                if let Ok(dir) = result
                    && let Some(path) = dir.path()
                {
                    manager.set_settings(Settings { save_to: Some(path.display().to_string()), ..manager.settings() });
                    folder.set_subtitle(&path.display().to_string());
                }
            });
        });
    }
    {
        let manager = ctx.manager.clone();
        reset.connect_clicked(move |_| {
            manager.set_settings(Settings { save_to: None, ..manager.settings() });
            folder.set_subtitle(&crate::downloads_folder().display().to_string());
        });
    }
    dialog.present(Some(&ctx.window));
}
