//! Settings (§9, Ctrl+,): applied and saved (in the core's settings.json) as soon as something changes, as on the
//! Mac. S8 (checksum files) arrives with checksum verification.

use adw::prelude::*;
use gtk::{gio, glib};
use hd_core::schedule::WhenDone;
use hd_core::settings::{Settings, WhenFileExists};

use crate::add::Ctx;

const EXISTS: [(&str, WhenFileExists); 4] = [
    ("Ask", WhenFileExists::Ask),
    ("Keep both", WhenFileExists::KeepBoth),
    ("Skip the new link", WhenFileExists::Skip),
    ("Replace the existing file", WhenFileExists::Replace),
];

const DONE: [(&str, WhenDone); 4] =
    [("Do nothing", WhenDone::Nothing), ("Quit", WhenDone::Quit), ("Sleep", WhenDone::Sleep), ("Shut down", WhenDone::ShutDown)];

/// A time row for the scheduler: a switch, then hours and minutes. `None` when switched off.
struct TimeRow {
    row: adw::ActionRow,
    on: gtk::Switch,
    hours: gtk::SpinButton,
    minutes: gtk::SpinButton,
}

impl TimeRow {
    fn new(title: &str, subtitle: &str, value: Option<u32>, fallback: u32) -> TimeRow {
        let row = adw::ActionRow::builder().title(title).subtitle(subtitle).build();
        let minutes_now = value.unwrap_or(fallback);
        let spin = |high: f64, v: u32, what: &str| {
            let s = gtk::SpinButton::with_range(0.0, high, 1.0);
            s.set_value(v as f64);
            s.set_wrap(true);
            s.set_valign(gtk::Align::Center);
            s.set_width_chars(2);
            s.update_property(&[gtk::accessible::Property::Label(&format!("{title}: {what}"))]);
            // Two digits, as a clock shows them.
            s.connect_output(|s| {
                s.set_text(&format!("{:02}", s.value() as u32));
                glib::Propagation::Stop
            });
            s
        };
        let hours = spin(23.0, minutes_now / 60, "hours");
        let minutes = spin(59.0, minutes_now % 60, "minutes");
        let on = gtk::Switch::builder().active(value.is_some()).valign(gtk::Align::Center).build();
        on.update_property(&[gtk::accessible::Property::Label(title)]);
        let time = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        time.append(&hours);
        time.append(&gtk::Label::new(Some(":")));
        time.append(&minutes);
        on.bind_property("active", &time, "sensitive").sync_create().build();
        row.add_suffix(&time);
        row.add_suffix(&on);
        TimeRow { row, on, hours, minutes }
    }

    /// Reads the row's time later, from the save closure.
    fn value_source(&self) -> impl Fn() -> Option<u32> + Clone + 'static {
        let (on, hours, minutes) = (self.on.clone(), self.hours.clone(), self.minutes.clone());
        move || on.is_active().then(|| hours.value() as u32 * 60 + minutes.value() as u32)
    }
}

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
    let scheduler = adw::PreferencesGroup::builder().title("Scheduler")
        .description("Start and stop the queue at set times").build();
    page.add(&scheduler);
    page.add(&streams_group(ctx));
    page.add(&crate::signins::group(ctx));
    // §13: the extension that hands the browsers' downloads to the app.
    let browsers = adw::PreferencesGroup::builder().title("Browser extension")
        .description("Chrome, Edge, Brave and Vivaldi: Extensions ▸ Developer mode ▸ Load Unpacked, and choose the folder. Firefox: about:debugging ▸ This Firefox ▸ Load Temporary Add-on, and choose manifest.json in it.").build();
    let extension = adw::ActionRow::builder().title("Chrome, Edge, Brave, Vivaldi and Firefox").build();
    let show = gtk::Button::builder().label("Show Extension Folder").valign(gtk::Align::Center)
        .tooltip_text("Open the folder with the extension, to load it in the browser").build();
    extension.add_suffix(&show);
    browsers.add(&extension);
    page.add(&browsers);
    {
        let window = ctx.window.clone();
        show.connect_clicked(move |_| match hd_core::browser::extension_folder(&crate::data_folder()) {
            Ok(folder) => gtk::FileLauncher::new(Some(&gio::File::for_path(folder))).launch(Some(&window), None::<&gio::Cancellable>, |_| {}),
            Err(e) => eprintln!("The extension can't be written: {e}"),
        });
    }

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

    // Q1–Q4 (§11.1).
    let start = TimeRow::new("Start the queue at", "On the days chosen below", s.schedule.start, 60);
    let stop = TimeRow::new("Stop the queue at", "Running downloads wait for the next start", s.schedule.stop, 7 * 60);
    crate::explain_row(&stop.row, "Stop the queue at this time; running downloads pause and wait in the queue for the next start. A time before the start time is the next morning's");
    let days_row = adw::ActionRow::builder().title("On").build();
    let days_box = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    let days: Vec<gtk::CheckButton> = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].iter().zip(s.schedule.days)
        .map(|(name, on)| {
            let b = gtk::CheckButton::builder().label(*name).active(on).build();
            days_box.append(&b);
            b
        })
        .collect();
    days_row.add_suffix(&days_box);
    let when_done = adw::ComboRow::builder().title("When the queue is done")
        .subtitle("After a minute's countdown you can cancel").model(&gtk::StringList::new(&DONE.map(|d| d.0))).build();
    when_done.set_selected(DONE.iter().position(|d| d.1 == s.schedule.when_done).unwrap_or(0) as u32);
    crate::explain_row(&when_done, "Once the running queue has nothing left to download: quit the app, or put the computer to sleep or shut it down");
    let enable_days = {
        let (start_on, stop_on, days_box) = (start.on.clone(), stop.on.clone(), days_box.clone());
        move || days_box.set_sensitive(start_on.is_active() || stop_on.is_active())
    };
    enable_days();
    scheduler.add(&start.row);
    scheduler.add(&stop.row);
    scheduler.add(&days_row);
    scheduler.add(&when_done);

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
        let (start, stop, days, when_done) = (start.value_source(), stop.value_source(), days.clone(), when_done.clone());
        move || {
            let mut schedule = manager.settings().schedule;
            schedule.start = start();
            schedule.stop = stop();
            for (i, b) in days.iter().enumerate() {
                schedule.days[i] = b.is_active();
            }
            schedule.when_done = DONE[(when_done.selected() as usize).min(3)].1;
            let s = Settings {
                schedule,
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
    for row in [&start, &stop] {
        for spin in [&row.hours, &row.minutes] {
            let save = save.clone();
            spin.connect_value_changed(move |_| save());
        }
        let (save, enable_days) = (save.clone(), enable_days.clone());
        row.on.connect_active_notify(move |_| {
            enable_days();
            save();
        });
    }
    for b in &days {
        let save = save.clone();
        b.connect_toggled(move |_| save());
    }
    for row in [&unit, &exists, &when_done] {
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

/// S13 and S14 (§15): what a stream added from now on downloads, unless the browser extension chose for it. Each row
/// saves its own setting, so a hand-edited list of several sound languages survives other changes.
fn streams_group(ctx: &Ctx) -> adw::PreferencesGroup {
    let s = ctx.manager.settings();
    let group = adw::PreferencesGroup::builder().title("Streamed video")
        .description("The browser extension can choose the quality and sound for each video").build();
    let qualities: Vec<Option<u32>> = std::iter::once(None).chain(hd_core::media::QUALITIES.iter().copied().map(Some)).collect();
    let names: Vec<String> = qualities.iter().map(|q| q.map(|q| format!("{q}p")).unwrap_or_else(|| "Highest".into())).collect();
    let quality = adw::ComboRow::builder().title("Video stream quality").subtitle("The best one up to this size, or the smallest when every one is bigger")
        .model(&gtk::StringList::new(&names.iter().map(String::as_str).collect::<Vec<_>>())).build();
    quality.set_selected(qualities.iter().position(|q| *q == s.stream_quality).unwrap_or(0) as u32);
    crate::explain_row(&quality, "The quality a streamed video is downloaded in: the best one up to this size, or the smallest when every one is bigger");
    let mut languages: Vec<(String, String)> = hd_core::media::LANGUAGES.iter().map(|(c, n)| (c.to_string(), n.to_string())).collect();
    // One not offered here (a hand-edited settings file) is shown as its code.
    if let Some(first) = s.sound_languages.first().filter(|f| !languages.iter().any(|(c, _)| c == *f)) {
        languages.push((first.clone(), first.clone()));
    }
    let sound_names: Vec<&str> = std::iter::once("The stream's own").chain(languages.iter().map(|(_, n)| n.as_str())).collect();
    let sound = adw::ComboRow::builder().title("Sound language").subtitle("For a stream with sound in several languages")
        .model(&gtk::StringList::new(&sound_names)).build();
    sound.set_selected(s.sound_languages.first().and_then(|f| languages.iter().position(|(c, _)| c == f)).map(|i| i as u32 + 1).unwrap_or(0));
    crate::explain_row(&sound, "For a stream with sound in several languages: the one to download. When the stream doesn't have it, its own sound is taken");
    {
        let manager = ctx.manager.clone();
        quality.connect_selected_notify(move |row| {
            let mut s = manager.settings();
            s.stream_quality = qualities.get(row.selected() as usize).copied().flatten();
            manager.set_settings(s);
        });
    }
    {
        let manager = ctx.manager.clone();
        sound.connect_selected_notify(move |row| {
            let mut s = manager.settings();
            let i = row.selected() as usize;
            s.sound_languages = if i == 0 { Vec::new() } else { languages.get(i - 1).map(|(c, _)| vec![c.clone()]).unwrap_or_default() };
            manager.set_settings(s);
        });
    }
    // S15: None, All, or a language (one not offered here shown as its code).
    let mut sub_languages: Vec<(String, String)> = hd_core::media::LANGUAGES.iter().map(|(c, n)| (c.to_string(), n.to_string())).collect();
    if let Some(first) = s.subtitle_languages.first().filter(|f| *f != "*" && !sub_languages.iter().any(|(c, _)| c == *f)) {
        sub_languages.push((first.clone(), first.clone()));
    }
    let sub_names: Vec<&str> = ["None", "All"].into_iter().chain(sub_languages.iter().map(|(_, n)| n.as_str())).collect();
    let subtitles = adw::ComboRow::builder().title("Subtitles").subtitle("Kept in the file as tracks a player can turn on")
        .model(&gtk::StringList::new(&sub_names)).build();
    subtitles.set_selected(match s.subtitle_languages.first().map(String::as_str) {
        None => 0,
        Some("*") => 1,
        Some(code) => sub_languages.iter().position(|(c, _)| c == code).map(|i| i as u32 + 2).unwrap_or(0),
    });
    crate::explain_row(&subtitles, "Subtitles a streamed video's file keeps, as tracks a player can turn on: none, every language the stream has, or one language");
    {
        let manager = ctx.manager.clone();
        subtitles.connect_selected_notify(move |row| {
            let mut s = manager.settings();
            s.subtitle_languages = match row.selected() as usize {
                0 => Vec::new(),
                1 => vec!["*".into()],
                i => sub_languages.get(i - 2).map(|(c, _)| vec![c.clone()]).unwrap_or_default(),
            };
            manager.set_settings(s);
        });
    }
    group.add(&quality);
    group.add(&sound);
    group.add(&subtitles);
    group
}

