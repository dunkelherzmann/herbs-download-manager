//! Links that need a login (enhancements.md), as on the Mac: asking for a site's user name and password when a
//! download meets a server that wants one, the Administrator Password that encrypts saved sign-ins (as in Herb's
//! Terminal), and Settings' Sign-ins group.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use adw::prelude::*;
use hd_core::vault::{SignIn, State};

use crate::add::Ctx;

thread_local! {
    /// Off while the feature tests run: the scenario gives sign-ins itself.
    pub static ASKS: Cell<bool> = const { Cell::new(true) };
    static ASKING: Cell<bool> = const { Cell::new(false) };
    /// Downloads already asked about, so a cancelled prompt isn't shown again until the download runs again.
    static ASKED: RefCell<HashSet<u64>> = RefCell::new(HashSet::new());
}

type Then = Rc<dyn Fn()>;

fn entry(placeholder: &str, value: &str, help: &str) -> gtk::Entry {
    let e = gtk::Entry::builder().placeholder_text(placeholder).text(value).width_chars(32).build();
    crate::explain(&e, placeholder, help);
    e
}

fn password(placeholder: &str, help: &str) -> gtk::PasswordEntry {
    let e = gtk::PasswordEntry::builder().placeholder_text(placeholder).show_peek_icon(true).build();
    crate::explain(&e, placeholder, help);
    e
}

fn column(widgets: &[&gtk::Widget]) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    for w in widgets {
        b.append(*w);
    }
    b
}

fn alert(heading: &str, body: &str) -> adw::AlertDialog {
    adw::AlertDialog::new(Some(heading), Some(body))
}

fn tell(ctx: &Ctx, text: &str) {
    let d = alert(text, "");
    d.add_response("ok", "OK");
    d.present(Some(&ctx.window));
}

/// After each refresh: a download that failed on a server asking for a user name and password gets a prompt.
pub fn ask_if_needed(ctx: &Ctx, rows: &[hd_core::manager::Row]) {
    let waiting: HashSet<u64> = rows.iter().filter(|r| r.needs_login.is_some()).map(|r| r.id).collect();
    ASKED.with(|a| a.borrow_mut().retain(|id| waiting.contains(id)));
    if !ASKS.get() || ASKING.get() {
        return;
    }
    let Some(row) = rows.iter().find(|r| r.needs_login.is_some() && !ASKED.with(|a| a.borrow().contains(&r.id))) else { return };
    ASKED.with(|a| a.borrow_mut().insert(row.id));
    ASKING.set(true);
    ask_for_login(ctx, row.needs_login.as_deref().unwrap_or(""), Rc::new(|| ASKING.set(false)));
}

/// With a saved sign-in that's locked, asks for the Administrator Password; otherwise for the site's user name and
/// password. Then starts the waiting downloads again; `done` runs either way.
pub fn ask_for_login(ctx: &Ctx, host: &str, done: Then) {
    let vault = ctx.manager.vault();
    let site = hd_core::vault::site_of(host);
    let saved = vault.list().into_iter().find(|s| hd_core::vault::covers(&s.site, host));
    if let Some(s) = &saved
        && s.password.is_none()
    {
        let (ctx2, site2, done2) = (ctx.clone(), s.site.clone(), done.clone());
        let detail = format!("A download from {host} needs your saved sign-in. Enter the Administrator Password to use it.");
        unlock(ctx, Some(&detail), Rc::new(move || {
            resume(&ctx2, &site2);
            done2();
        }), done);
        return;
    }
    let body = match &saved {
        None => "Sign in to download the file.".to_string(),
        Some(s) => format!("The saved sign-in for {} wasn't accepted. Sign in again.", s.site),
    };
    let d = alert(&format!("{host} asks for a user name and password"), &body);
    let user = entry("User name", saved.as_ref().map(|s| s.user.as_str()).unwrap_or(""), "The user name for this site");
    let pass = password("Password", "The password for this site");
    let remember = gtk::CheckButton::with_label(&format!("Remember for {site}"));
    remember.set_active(true);
    crate::explain(&remember, &format!("Remember for {site}"),
                   &format!("Save this sign-in in the app (encrypted with the Administrator Password) for later downloads from {site}"));
    d.set_extra_child(Some(&column(&[user.upcast_ref(), pass.upcast_ref(), remember.upcast_ref()])));
    d.add_response("cancel", "Cancel");
    d.add_response("sign-in", "Sign In");
    d.set_response_appearance("sign-in", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("sign-in"));
    let ctx2 = ctx.clone();
    d.connect_response(None, move |_, response| {
        if response != "sign-in" {
            done();
            return;
        }
        let sign = SignIn {
            site: saved.as_ref().map(|s| s.site.clone()).unwrap_or_else(|| site.clone()),
            user: user.text().into(),
            password: pass.text().into(),
            cookies: saved.as_ref().and_then(|s| s.cookies.clone()).unwrap_or_default(),
            referer: saved.as_ref().map(|s| s.referer.clone()).unwrap_or_default(),
        };
        let vault_site = sign.site.clone();
        if remember.is_active() {
            let (ctx3, done3, sign2) = (ctx2.clone(), done.clone(), sign.clone());
            ready_to_save(&ctx2, Rc::new(move || {
                if let Err(e) = ctx3.manager.vault().save_sign_in(sign2.clone()) {
                    tell(&ctx3, &e);
                    ctx3.manager.vault().remember_for_session(sign2.clone());
                }
                resume(&ctx3, &vault_site);
                done3();
            }), {
                let (ctx3, done3) = (ctx2.clone(), done.clone());
                Rc::new(move || {
                    ctx3.manager.vault().remember_for_session(sign.clone());
                    resume(&ctx3, &sign.site);
                    done3();
                })
            });
        } else {
            ctx2.manager.vault().remember_for_session(sign.clone());
            resume(&ctx2, &sign.site);
            done();
        }
    });
    d.present(Some(&ctx.window));
}

fn resume(ctx: &Ctx, site: &str) {
    ctx.manager.resume(&ctx.manager.waiting_for_login(site));
    (ctx.refresh)();
}

/// Before saving: a protection is chosen, and encrypted sign-ins are unlocked. Runs `then`, or `otherwise` when the
/// user backs out.
pub fn ready_to_save(ctx: &Ctx, then: Then, otherwise: Then) {
    match ctx.manager.vault().state() {
        State::NotChosen => explain_setup(ctx, then, otherwise),
        State::Locked => unlock(ctx, None, then, otherwise),
        State::Plain | State::Unlocked => then(),
    }
}

/// The first-use explanation.
pub fn explain_setup(ctx: &Ctx, then: Then, otherwise: Then) {
    let d = alert(
        "Create an Administrator Password",
        "Herb's Download Manager keeps saved sign-ins inside the app. To encrypt them, create an Administrator Password; \
         you'll enter it once each time you open the app, when a download first needs a saved sign-in.\n\n\
         Without one, saved sign-ins (passwords and cookies included) are stored as plain text, which is a security \
         risk: anyone or anything that can read your files could read them.",
    );
    d.add_response("later", "Not Now");
    d.add_response("plain", "Store as Plain Text");
    d.add_response("create", "Create Administrator Password…");
    d.set_response_appearance("plain", adw::ResponseAppearance::Destructive);
    d.set_response_appearance("create", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("create"));
    d.set_close_response("later");
    let ctx2 = ctx.clone();
    d.connect_response(None, move |_, response| match response {
        "create" => create_or_modify(&ctx2, then.clone(), otherwise.clone()),
        "plain" => {
            // The risky choice is never one click away.
            let c = alert(
                "Store saved sign-ins as plain text?",
                "Passwords and cookies you save will be readable by anyone or anything that can read your files. You can \
                 create an Administrator Password later in Settings ▸ Sign-ins.",
            );
            c.add_response("cancel", "Cancel");
            c.add_response("plain", "Store as Plain Text");
            c.set_response_appearance("plain", adw::ResponseAppearance::Destructive);
            c.set_default_response(Some("cancel"));
            let (ctx3, then3, otherwise3) = (ctx2.clone(), then.clone(), otherwise.clone());
            c.connect_response(None, move |_, r| {
                if r == "plain" && ctx3.manager.vault().use_plain_text().is_ok() {
                    then3();
                } else {
                    otherwise3();
                }
            });
            c.present(Some(&ctx2.window));
        }
        _ => otherwise(),
    });
    d.present(Some(&ctx.window));
}

/// Creates the Administrator Password, or changes it after checking the current one (blank removes it).
pub fn create_or_modify(ctx: &Ctx, then: Then, otherwise: Then) {
    create_or_modify_with(ctx, None, then, otherwise);
}

fn create_or_modify_with(ctx: &Ctx, message: Option<String>, then: Then, otherwise: Then) {
    let vault = ctx.manager.vault();
    let modifying = matches!(vault.state(), State::Locked | State::Unlocked);
    let info = if modifying {
        "Saved sign-ins are encrypted again with the new password.\n\nLeave both new password fields blank to remove the \
         Administrator Password: saved sign-ins are then stored as plain text, unprotected."
    } else {
        "Saved sign-ins will be encrypted with this password. It can't be recovered if you forget it; you would have to \
         delete your sign-ins and enter them again."
    };
    let body = match &message {
        Some(m) => format!("{m}\n\n{info}"),
        None => info.to_string(),
    };
    let d = alert(if modifying { "Change Administrator Password" } else { "Create Administrator Password" }, &body);
    let current = password("Current password", "The Administrator Password you use now");
    let new = password(if modifying { "New password (blank to remove)" } else { "New password (at least 8 characters)" },
                       "The new Administrator Password");
    let confirm = password("Confirm new password", "The new Administrator Password again");
    let fields: Vec<&gtk::Widget> = if modifying {
        vec![current.upcast_ref(), new.upcast_ref(), confirm.upcast_ref()]
    } else {
        vec![new.upcast_ref(), confirm.upcast_ref()]
    };
    d.set_extra_child(Some(&column(&fields)));
    d.add_response("cancel", "Cancel");
    d.add_response("ok", if modifying { "Change Password" } else { "Create Password" });
    d.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("ok"));
    let ctx2 = ctx.clone();
    d.connect_response(None, move |_, response| {
        if response != "ok" {
            otherwise();
            return;
        }
        let vault = ctx2.manager.vault();
        let (old, value) = (current.text().to_string(), new.text().to_string());
        let again = |m: &str| create_or_modify_with(&ctx2, Some(m.to_string()), then.clone(), otherwise.clone());
        if value != confirm.text().as_str() {
            return again("The new passwords don't match.");
        }
        if modifying && !vault.unlock(&old) {
            return again("The current password is wrong.");
        }
        let result = if modifying { vault.change_password(&value) } else { vault.create_password(&value) };
        match result {
            Ok(()) => then(),
            Err(e) => again(&e),
        }
    });
    d.present(Some(&ctx.window));
}

/// Asks for the Administrator Password to unlock saved sign-ins for this session.
pub fn unlock(ctx: &Ctx, detail: Option<&str>, then: Then, otherwise: Then) {
    unlock_with(ctx, detail.unwrap_or("Enter the Administrator Password to use your saved sign-ins.").to_string(), then, otherwise);
}

fn unlock_with(ctx: &Ctx, body: String, then: Then, otherwise: Then) {
    if ctx.manager.vault().state() != State::Locked {
        return then();
    }
    let d = alert("Unlock Saved Sign-ins", &body);
    let field = password("Administrator Password", "The Administrator Password that encrypts saved sign-ins");
    d.set_extra_child(Some(&field));
    d.add_response("cancel", "Cancel");
    d.add_response("unlock", "Unlock");
    d.set_response_appearance("unlock", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("unlock"));
    let ctx2 = ctx.clone();
    d.connect_response(None, move |_, response| {
        if response != "unlock" {
            return otherwise();
        }
        if ctx2.manager.vault().unlock(&field.text()) {
            then();
        } else {
            unlock_with(&ctx2, "That isn't the Administrator Password.".into(), then.clone(), otherwise.clone());
        }
    });
    d.present(Some(&ctx.window));
}

/// Settings ▸ Sign-ins: one row per saved site (Edit, Delete), and Add, Lock or Unlock and the Administrator Password.
pub fn group(ctx: &Ctx) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Sign-ins").build();
    let rows: Rc<RefCell<Vec<gtk::Widget>>> = Rc::default();
    let rebuild: Rc<RefCell<Option<Then>>> = Rc::default();
    let fill: Then = {
        let (ctx, group, rows, rebuild) = (ctx.clone(), group.clone(), rows.clone(), rebuild.clone());
        Rc::new(move || {
            for r in rows.borrow_mut().drain(..) {
                group.remove(&r);
            }
            let vault = ctx.manager.vault();
            let state = vault.state();
            group.set_description(Some(match state {
                State::NotChosen => "Sign-ins are asked for when a download needs one. Create an Administrator Password to encrypt the ones you save.",
                State::Plain => "Saved sign-ins are stored as plain text. Create an Administrator Password to encrypt them.",
                State::Locked => "Saved sign-ins are encrypted and locked. Unlock them to see or change passwords.",
                State::Unlocked => "Saved sign-ins are encrypted with the Administrator Password and unlocked until the app quits.",
            }));
            let again = rebuild.borrow().clone().unwrap();
            // The group's buttons.
            let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let add = gtk::Button::with_label("Add…");
            crate::explain(&add, "Add a sign-in", "Add a site's user name, password, cookies or referer");
            let lock = gtk::Button::with_label(if state == State::Unlocked { "Lock" } else { "Unlock…" });
            lock.set_sensitive(matches!(state, State::Locked | State::Unlocked));
            crate::explain(&lock, if state == State::Unlocked { "Lock sign-ins" } else { "Unlock sign-ins" },
                           "Unlock saved sign-ins with the Administrator Password, or lock them again");
            let admin = gtk::Button::with_label(if matches!(state, State::Locked | State::Unlocked) {
                "Change Administrator Password…"
            } else {
                "Create Administrator Password…"
            });
            crate::explain(&admin, "Administrator Password", "Create, change or remove the Administrator Password that encrypts saved sign-ins");
            for b in [&add, &lock, &admin] {
                bar.append(b);
            }
            {
                let (ctx, again) = (ctx.clone(), again.clone());
                add.connect_clicked(move |_| form(&ctx, None, again.clone()));
            }
            {
                let (ctx, again) = (ctx.clone(), again.clone());
                lock.connect_clicked(move |_| {
                    if ctx.manager.vault().state() == State::Unlocked {
                        ctx.manager.vault().lock();
                        again();
                    } else {
                        let ctx2 = ctx.clone();
                        let again2 = again.clone();
                        unlock(&ctx, None, Rc::new(move || {
                            resume(&ctx2, "");
                            again2();
                        }), again.clone());
                    }
                });
            }
            {
                let (ctx, again) = (ctx.clone(), again.clone());
                admin.connect_clicked(move |_| {
                    if ctx.manager.vault().state() == State::NotChosen {
                        explain_setup(&ctx, again.clone(), again.clone());
                    } else {
                        create_or_modify(&ctx, again.clone(), again.clone());
                    }
                });
            }
            let bar_row = adw::PreferencesRow::builder().child(&bar).activatable(false).build();
            bar.set_margin_top(8);
            bar.set_margin_bottom(8);
            bar.set_margin_start(12);
            group.add(&bar_row);
            rows.borrow_mut().push(bar_row.upcast());
            for s in vault.list() {
                let extras: Vec<&str> = [
                    s.cookies.as_deref().is_some_and(|c| !c.is_empty()).then_some("cookies"),
                    (!s.referer.is_empty()).then_some("referer"),
                ]
                .into_iter()
                .flatten()
                .collect();
                let how = if s.password.is_none() { "Locked" } else if s.encrypted { "Encrypted" } else { "Plain text" };
                let mut subtitle = vec![s.user.clone(), how.to_string()];
                if !extras.is_empty() {
                    subtitle.push(format!("with {}", extras.join(" and ")));
                }
                let row = adw::ActionRow::builder().title(&s.site).subtitle(subtitle.into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ")).build();
                let edit = gtk::Button::builder().icon_name("document-edit-symbolic").valign(gtk::Align::Center).build();
                edit.add_css_class("flat");
                edit.set_sensitive(s.password.is_some());
                crate::explain(&edit, &format!("Edit the sign-in for {}", s.site), "Change this site's user name, password, cookies or referer");
                let delete = gtk::Button::builder().icon_name("user-trash-symbolic").valign(gtk::Align::Center).build();
                delete.add_css_class("flat");
                crate::explain(&delete, &format!("Delete the sign-in for {}", s.site), "Delete this site's sign-in from the app");
                row.add_suffix(&edit);
                row.add_suffix(&delete);
                {
                    let (ctx, again, s) = (ctx.clone(), again.clone(), s.clone());
                    edit.connect_clicked(move |_| form(&ctx, Some(s.clone()), again.clone()));
                }
                {
                    let (ctx, again, site) = (ctx.clone(), again.clone(), s.site.clone());
                    delete.connect_clicked(move |_| {
                        let d = alert(&format!("Delete the sign-in for {site}?"),
                                      "Its user name, password, cookies and referer are removed from the app. This can't be undone.");
                        d.add_response("cancel", "Cancel");
                        d.add_response("delete", "Delete");
                        d.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
                        d.set_default_response(Some("cancel"));
                        let (ctx2, again2, site2) = (ctx.clone(), again.clone(), site.clone());
                        d.connect_response(Some("delete"), move |_, _| {
                            let _ = ctx2.manager.vault().delete(&site2);
                            again2();
                        });
                        d.present(Some(&ctx.window));
                    });
                }
                group.add(&row);
                rows.borrow_mut().push(row.upcast());
            }
        })
    };
    *rebuild.borrow_mut() = Some(fill.clone());
    fill();
    group
}

/// Add or Edit: site, user name, password, cookies and referer.
fn form(ctx: &Ctx, existing: Option<hd_core::vault::Listed>, done: Then) {
    let (ctx2, done2) = (ctx.clone(), done.clone());
    ready_to_save(ctx, Rc::new(move || form_open(&ctx2, existing.clone(), None, done2.clone())), done);
}

fn form_open(ctx: &Ctx, existing: Option<hd_core::vault::Listed>, message: Option<String>, done: Then) {
    let heading = match &existing {
        None => "Add a Sign-in".to_string(),
        Some(e) => format!("Edit the Sign-in for {}", e.site),
    };
    let d = alert(&heading, message.as_deref().unwrap_or("Downloads from the site and its subdomains send these. Leave out what the site doesn't need."));
    let e = existing.clone();
    let site = entry("Site, such as example.com", e.as_ref().map(|x| x.site.as_str()).unwrap_or(""), "The site these apply to; its subdomains too");
    let user = entry("User name", e.as_ref().map(|x| x.user.as_str()).unwrap_or(""), "The user name for this site");
    let pass = password("Password", "The password for this site");
    pass.set_text(e.as_ref().and_then(|x| x.password.as_deref()).unwrap_or(""));
    let cookies = entry("Cookies, such as session=abc123", e.as_ref().and_then(|x| x.cookies.as_deref()).unwrap_or(""),
                        "The Cookie header downloads from this site send, as name=value pairs separated by semicolons");
    let referer = entry("Referer, the page links come from", e.as_ref().map(|x| x.referer.as_str()).unwrap_or(""),
                        "For sites that only allow downloads coming from one of their pages");
    d.set_extra_child(Some(&column(&[site.upcast_ref(), user.upcast_ref(), pass.upcast_ref(), cookies.upcast_ref(), referer.upcast_ref()])));
    d.add_response("cancel", "Cancel");
    d.add_response("save", "Save");
    d.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("save"));
    let ctx2 = ctx.clone();
    d.connect_response(None, move |_, response| {
        if response != "save" {
            return done();
        }
        let vault = ctx2.manager.vault();
        let new_site = hd_core::vault::site_of(&site.text());
        if let Some(old) = &existing
            && old.site != new_site
        {
            let _ = vault.delete(&old.site);
        }
        let sign = SignIn {
            site: site.text().into(),
            user: user.text().into(),
            password: pass.text().into(),
            cookies: cookies.text().into(),
            referer: referer.text().into(),
        };
        match vault.save_sign_in(sign) {
            Ok(()) => {
                resume(&ctx2, &new_site);
                done();
            }
            Err(m) => form_open(&ctx2, existing.clone(), Some(m), done.clone()),
        }
    });
    d.present(Some(&ctx.window));
}
