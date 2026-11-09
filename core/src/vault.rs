//! Sign-ins for links that need a login (enhancements.md): a user name and password per site, and the cookies and
//! referer its links need, kept in `sign-ins.json` in the app's own folder, never the system's keychain.
//!
//! As in Herb's Terminal: with an Administrator Password, passwords and cookies are encrypted with AES-256-GCM under a
//! key derived from it with PBKDF2-HMAC-SHA256 (600,000 rounds, a random 16-byte salt). Only the salt, the round
//! count and an encrypted check value are stored, never the password itself. The store is locked when the app starts
//! and unlocked once per session. Without one, they're stored as plain text, only after an explicit, warned choice.
//! Sites and user names are stored as plain text either way, so the list shows them while locked.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::Engine;
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};

const ROUNDS: u32 = 600_000;
const CHECK: &[u8] = b"Herb's Download Manager sign-ins";
const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// One site's sign-in, readable.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignIn {
    /// A host name ("example.com"); it also covers the host's subdomains ("files.example.com").
    pub site: String,
    pub user: String,
    pub password: String,
    /// A `Cookie` header's value ("session=abc; theme=dark").
    pub cookies: String,
    /// The page a link must come from, for servers that check.
    pub referer: String,
}

impl SignIn {
    pub fn covers(&self, host: &str) -> bool {
        covers(&self.site, host)
    }
}

/// Whether a sign-in for `site` applies to links on `host`.
pub fn covers(site: &str, host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == site || host.ends_with(&format!(".{site}"))
}

/// The site a link or a typed site belongs to: its host, lowercase, without "www.".
pub fn site_of(text: &str) -> String {
    let t = text.trim();
    let host = reqwest::Url::parse(t)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| t.split(['/', ':']).next().unwrap_or("").to_string());
    let host = host.to_ascii_lowercase();
    host.strip_prefix("www.").map(str::to_string).unwrap_or(host)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Nothing saved yet, and no choice made: the first save asks how to protect sign-ins.
    NotChosen,
    /// Stored as plain text (chosen, and confirmed).
    Plain,
    /// Encrypted; the Administrator Password hasn't been entered this session.
    Locked,
    Unlocked,
}

/// A sign-in as the list shows it: the secrets are `None` while locked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub site: String,
    pub user: String,
    pub referer: String,
    pub password: Option<String>,
    pub cookies: Option<String>,
    pub encrypted: bool,
}

/// What a download finds for a host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    Found(SignIn),
    /// There is one, but the store is locked.
    Locked,
    None,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
enum Protection {
    #[default]
    NotChosen,
    Plain,
    Encrypted {
        salt: String,
        rounds: u32,
        check: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Secret {
    Plain { password: String, cookies: String },
    /// Base64 of the nonce and the sealed JSON of `Secrets`.
    Sealed(String),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Secrets {
    password: String,
    cookies: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Saved {
    site: String,
    user: String,
    #[serde(default)]
    referer: String,
    secret: Secret,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Store {
    protection: Protection,
    sites: Vec<Saved>,
}

struct Inner {
    store: Store,
    key: Option<[u8; 32]>,
    /// Sign-ins typed for this session only ("Remember" left off); never saved.
    session: HashMap<String, SignIn>,
}

pub struct Vault {
    file: Option<PathBuf>,
    rounds: u32,
    inner: Mutex<Inner>,
}

impl std::fmt::Debug for Vault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vault").field("file", &self.file).finish_non_exhaustive()
    }
}

impl Default for Vault {
    fn default() -> Self {
        Vault::open(None)
    }
}

fn derive(password: &str, salt: &[u8], rounds: u32) -> [u8; 32] {
    let mut key = [0u8; 32];
    ring::pbkdf2::derive(
        ring::pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(rounds.max(1)).unwrap(),
        salt,
        password.as_bytes(),
        &mut key,
    );
    key
}

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    SystemRandom::new().fill(&mut b).expect("the system's random numbers");
    b
}

fn seal(key: &[u8; 32], plain: &[u8]) -> String {
    let sealing = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).unwrap());
    let nonce = random::<NONCE_LEN>();
    let mut data = plain.to_vec();
    sealing.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut data).unwrap();
    let mut out = nonce.to_vec();
    out.extend(data);
    B64.encode(out)
}

fn open_sealed(key: &[u8; 32], sealed: &str) -> Option<Vec<u8>> {
    let bytes = B64.decode(sealed).ok()?;
    if bytes.len() < NONCE_LEN {
        return None;
    }
    let (nonce, data) = bytes.split_at(NONCE_LEN);
    let opening = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).ok()?);
    let mut data = data.to_vec();
    let plain = opening.open_in_place(Nonce::try_assume_unique_for_key(nonce).ok()?, Aad::empty(), &mut data).ok()?;
    Some(plain.to_vec())
}

impl Inner {
    fn secrets(&self, s: &Saved) -> Option<Secrets> {
        match &s.secret {
            Secret::Plain { password, cookies } => Some(Secrets { password: password.clone(), cookies: cookies.clone() }),
            Secret::Sealed(sealed) => serde_json::from_slice(&open_sealed(self.key.as_ref()?, sealed)?).ok(),
        }
    }

    fn encode(&self, secrets: &Secrets) -> Secret {
        match &self.key {
            Some(key) => Secret::Sealed(seal(key, &serde_json::to_vec(secrets).unwrap())),
            None => Secret::Plain { password: secrets.password.clone(), cookies: secrets.cookies.clone() },
        }
    }

    /// Re-encodes every sign-in for the current key (none: plain text). Needs every secret readable.
    fn reencode(&mut self, old: &Inner) {
        let sites = std::mem::take(&mut self.store.sites);
        self.store.sites = sites
            .into_iter()
            .map(|s| {
                let secrets = old.secrets(&s).unwrap_or_default();
                Saved { secret: self.encode(&secrets), ..s }
            })
            .collect();
    }
}

impl Vault {
    /// The sign-ins kept in `dir` (`None`: kept in memory only, for tests).
    pub fn open(dir: Option<&Path>) -> Vault {
        Self::open_with(dir, ROUNDS)
    }

    /// Fewer key rounds, for tests.
    pub fn open_with(dir: Option<&Path>, rounds: u32) -> Vault {
        let file = dir.map(|d| d.join("sign-ins.json"));
        let store = file
            .as_deref()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|b| serde_json::from_slice::<Store>(&b).ok())
            .unwrap_or_default();
        Vault { file, rounds, inner: Mutex::new(Inner { store, key: None, session: HashMap::new() }) }
    }

    fn save(&self, inner: &Inner) -> Result<(), String> {
        let Some(file) = &self.file else { return Ok(()) };
        let tmp = file.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(&inner.store).map_err(|e| e.to_string())?;
        let write = || -> std::io::Result<()> {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            // Only the user's account can read it.
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            std::io::Write::write_all(&mut options.open(&tmp)?, &bytes)?;
            std::fs::rename(&tmp, file)
        };
        write().map_err(|e| format!("Can't save the sign-ins: {e}"))
    }

    pub fn state(&self) -> State {
        let inner = self.inner.lock().unwrap();
        match inner.store.protection {
            Protection::NotChosen => State::NotChosen,
            Protection::Plain => State::Plain,
            Protection::Encrypted { .. } if inner.key.is_some() => State::Unlocked,
            Protection::Encrypted { .. } => State::Locked,
        }
    }

    /// Creates the Administrator Password (first use, or after plain text): every sign-in is encrypted with it.
    pub fn create_password(&self, password: &str) -> Result<(), String> {
        if password.chars().count() < 8 {
            return Err("The Administrator Password needs at least 8 characters".into());
        }
        let mut inner = self.inner.lock().unwrap();
        if matches!(inner.store.protection, Protection::Encrypted { .. }) {
            return Err("There already is an Administrator Password".into());
        }
        let old = Inner { store: inner.store.clone(), key: None, session: HashMap::new() };
        self.set_key(&mut inner, &old, Some(password));
        self.save(&inner)
    }

    /// Changes the Administrator Password (unlocked only); an empty one removes it, storing sign-ins as plain text.
    pub fn change_password(&self, password: &str) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        if inner.key.is_none() {
            return Err("Unlock the sign-ins first".into());
        }
        if !password.is_empty() && password.chars().count() < 8 {
            return Err("The Administrator Password needs at least 8 characters".into());
        }
        let old = Inner { store: inner.store.clone(), key: inner.key, session: HashMap::new() };
        self.set_key(&mut inner, &old, (!password.is_empty()).then_some(password));
        self.save(&inner)
    }

    fn set_key(&self, inner: &mut Inner, old: &Inner, password: Option<&str>) {
        match password {
            Some(p) => {
                let salt = random::<16>();
                let key = derive(p, &salt, self.rounds);
                inner.key = Some(key);
                inner.store.protection =
                    Protection::Encrypted { salt: B64.encode(salt), rounds: self.rounds, check: seal(&key, CHECK) };
            }
            None => {
                inner.key = None;
                inner.store.protection = Protection::Plain;
            }
        }
        inner.reencode(old);
    }

    /// Stores sign-ins as plain text: the explicit choice the app asks for, with a second confirmation.
    pub fn use_plain_text(&self) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        match inner.store.protection {
            Protection::NotChosen => {
                inner.store.protection = Protection::Plain;
                self.save(&inner)
            }
            Protection::Plain => Ok(()),
            Protection::Encrypted { .. } => Err("Remove the Administrator Password instead".into()),
        }
    }

    /// Unlocks with the Administrator Password; false when it's wrong.
    pub fn unlock(&self, password: &str) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let Protection::Encrypted { salt, rounds, check } = &inner.store.protection else { return false };
        let Ok(salt) = B64.decode(salt) else { return false };
        let key = derive(password, &salt, *rounds);
        if open_sealed(&key, check).as_deref() != Some(CHECK) {
            return false;
        }
        inner.key = Some(key);
        true
    }

    pub fn lock(&self) {
        self.inner.lock().unwrap().key = None;
    }

    pub fn list(&self) -> Vec<Listed> {
        let inner = self.inner.lock().unwrap();
        let mut out: Vec<Listed> = inner
            .store
            .sites
            .iter()
            .map(|s| {
                let secrets = inner.secrets(s);
                Listed {
                    site: s.site.clone(),
                    user: s.user.clone(),
                    referer: s.referer.clone(),
                    password: secrets.as_ref().map(|x| x.password.clone()),
                    cookies: secrets.map(|x| x.cookies),
                    encrypted: matches!(s.secret, Secret::Sealed(_)),
                }
            })
            .collect();
        out.sort_by(|a, b| a.site.cmp(&b.site));
        out
    }

    /// Saves a site's sign-in (replacing one for the same site). Needs a protection chosen, and unlocked if encrypted.
    pub fn save_sign_in(&self, sign_in: SignIn) -> Result<(), String> {
        let site = site_of(&sign_in.site);
        if site.is_empty() {
            return Err("Type the site, such as example.com".into());
        }
        let mut inner = self.inner.lock().unwrap();
        match inner.store.protection {
            Protection::NotChosen => return Err("Choose how to protect sign-ins first".into()),
            Protection::Encrypted { .. } if inner.key.is_none() => return Err("Unlock the sign-ins first".into()),
            _ => {}
        }
        let secret = inner.encode(&Secrets { password: sign_in.password.clone(), cookies: sign_in.cookies.clone() });
        inner.store.sites.retain(|s| s.site != site);
        inner.session.remove(&site);
        inner.store.sites.push(Saved { site, user: sign_in.user, referer: sign_in.referer, secret });
        self.save(&inner)
    }

    pub fn delete(&self, site: &str) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        inner.store.sites.retain(|s| s.site != site);
        inner.session.remove(site);
        self.save(&inner)
    }

    /// Uses a sign-in until the app quits, without saving it.
    pub fn remember_for_session(&self, mut sign_in: SignIn) {
        sign_in.site = site_of(&sign_in.site);
        self.inner.lock().unwrap().session.insert(sign_in.site.clone(), sign_in);
    }

    /// The sign-in for a link's host: this session's first, then the saved one for the closest site.
    pub fn for_host(&self, host: &str) -> Lookup {
        let inner = self.inner.lock().unwrap();
        let closest = |sites: &mut dyn Iterator<Item = &str>| -> Option<String> {
            sites.filter(|s| covers(s, host)).max_by_key(|s| s.len()).map(str::to_string)
        };
        if let Some(site) = closest(&mut inner.session.keys().map(String::as_str)) {
            return Lookup::Found(inner.session[&site].clone());
        }
        let Some(site) = closest(&mut inner.store.sites.iter().map(|s| s.site.as_str())) else { return Lookup::None };
        let saved = inner.store.sites.iter().find(|s| s.site == site).unwrap();
        match inner.secrets(saved) {
            Some(x) => Lookup::Found(SignIn {
                site,
                user: saved.user.clone(),
                password: x.password,
                cookies: x.cookies,
                referer: saved.referer.clone(),
            }),
            None => Lookup::Locked,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign_in(site: &str, password: &str) -> SignIn {
        SignIn { site: site.into(), user: "herb".into(), password: password.into(), cookies: "s=1".into(), referer: String::new() }
    }

    #[test]
    fn encrypted_sign_ins_need_the_administrator_password() {
        let dir = std::env::temp_dir().join(format!("hd-vault-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let v = Vault::open_with(Some(&dir), 1000);
        assert_eq!(v.state(), State::NotChosen);
        assert!(v.save_sign_in(sign_in("example.com", "pw")).is_err(), "nothing is saved before a choice");
        v.create_password("test-password").unwrap();
        v.save_sign_in(sign_in("https://www.Example.com/files/a.zip", "secret-pw")).unwrap();
        let text = std::fs::read_to_string(dir.join("sign-ins.json")).unwrap();
        assert!(!text.contains("secret-pw") && !text.contains("s=1") && !text.contains("test-password"), "{text}");
        assert!(text.contains("example.com"));

        let again = Vault::open_with(Some(&dir), 1000);
        assert_eq!(again.state(), State::Locked);
        assert_eq!(again.for_host("files.example.com"), Lookup::Locked);
        assert_eq!(again.list()[0].password, None);
        assert!(!again.unlock("wrong"));
        assert!(again.unlock("test-password"));
        let Lookup::Found(found) = again.for_host("files.example.com") else { panic!() };
        assert_eq!((found.password.as_str(), found.cookies.as_str()), ("secret-pw", "s=1"));
        assert_eq!(again.for_host("example.org"), Lookup::None);

        // Removing the Administrator Password stores them as plain text.
        again.change_password("").unwrap();
        assert_eq!(again.state(), State::Plain);
        assert!(std::fs::read_to_string(dir.join("sign-ins.json")).unwrap().contains("secret-pw"));
        again.create_password("new password").unwrap();
        let third = Vault::open_with(Some(&dir), 1000);
        assert!(third.unlock("new password") && !Vault::open_with(Some(&dir), 1000).unlock("test-password"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn session_sign_ins_are_not_saved_and_win() {
        let v = Vault::open_with(None, 1000);
        v.use_plain_text().unwrap();
        v.save_sign_in(sign_in("example.com", "saved")).unwrap();
        v.remember_for_session(sign_in("example.com", "typed"));
        let Lookup::Found(found) = v.for_host("example.com") else { panic!() };
        assert_eq!(found.password, "typed");
        assert_eq!(v.list().len(), 1);
    }
}
