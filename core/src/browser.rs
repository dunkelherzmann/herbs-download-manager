//! The browser extension's side in the app (SPECIFICATION.md §13). Browsers start `hdm` as the extension's native
//! messaging host; each app registers it at launch with the browsers it finds, and keeps a copy of the extension in
//! its data folder for "Load unpacked" (Chrome, Edge, Brave, Vivaldi) and "Load Temporary Add-on" (Firefox).

use std::path::{Path, PathBuf};

/// The native messaging host's name, as the extension asks for it.
pub const HOST: &str = "org.herbsdownloadmanager.hdm";
/// The extension's id in Firefox (`browser_specific_settings` in its manifest).
pub const FIREFOX_ID: &str = "hdm@herbsdownloadmanager.org";
/// The extension's id in Chrome and its kin, fixed by the public key in its manifest.
pub const CHROME_ID: &str = "clhikpnhhffblbcgfcpebhiholmchidg";

const FILES: [(&str, &[u8]); 16] = [
    ("manifest.json", include_bytes!("../../extension/manifest.json")),
    ("background.js", include_bytes!("../../extension/background.js")),
    ("popup.html", include_bytes!("../../extension/popup.html")),
    ("popup.js", include_bytes!("../../extension/popup.js")),
    ("popup.css", include_bytes!("../../extension/popup.css")),
    ("naming.js", include_bytes!("../../extension/naming.js")),
    ("options.html", include_bytes!("../../extension/options.html")),
    ("options.js", include_bytes!("../../extension/options.js")),
    ("options.css", include_bytes!("../../extension/options.css")),
    ("icons/icon-16.png", include_bytes!("../../extension/icons/icon-16.png")),
    ("icons/icon-32.png", include_bytes!("../../extension/icons/icon-32.png")),
    ("icons/icon-48.png", include_bytes!("../../extension/icons/icon-48.png")),
    ("icons/icon-128.png", include_bytes!("../../extension/icons/icon-128.png")),
    ("icons/toolbar-16.png", include_bytes!("../../extension/icons/toolbar-16.png")),
    ("icons/toolbar-24.png", include_bytes!("../../extension/icons/toolbar-24.png")),
    ("icons/toolbar-32.png", include_bytes!("../../extension/icons/toolbar-32.png")),
];

/// Writes `path` only when its content differs, so a launch doesn't touch files that are already right.
fn write_if_changed(path: &Path, content: &[u8]) -> std::io::Result<()> {
    if std::fs::read(path).is_ok_and(|old| old == content) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)
}

/// The extension, unpacked in `data_dir/Browser Extension` (rewritten when the app brings a newer one).
pub fn extension_folder(data_dir: &Path) -> std::io::Result<PathBuf> {
    let folder = data_dir.join("Browser Extension");
    for (name, content) in FILES {
        write_if_changed(&folder.join(name), content)?;
    }
    Ok(folder)
}

fn manifest(host: &Path, firefox: bool) -> Vec<u8> {
    let mut m = serde_json::json!({
        "name": HOST,
        "description": "Herb's Download Manager: takes the browser's downloads",
        "path": host.display().to_string(),
        "type": "stdio",
    });
    if firefox {
        m["allowed_extensions"] = serde_json::json!([FIREFOX_ID]);
    } else {
        m["allowed_origins"] = serde_json::json!([format!("chrome-extension://{CHROME_ID}/")]);
    }
    serde_json::to_vec_pretty(&m).unwrap_or_default()
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// Registers `hdm` (at `hdm`) as the native messaging host with every browser found for this user. A browser
/// installed later is found at the app's next launch. Returns the problems, if any (the app goes on without them).
pub fn register(hdm: &Path, data_dir: &Path) -> Vec<String> {
    let Some(home) = home() else { return vec!["No home folder".into()] };
    let mut problems = Vec::new();
    let mut put = |path: PathBuf, content: Vec<u8>| {
        if let Err(e) = write_if_changed(&path, &content) {
            problems.push(format!("{}: {e}", path.display()));
        }
    };
    #[cfg(unix)]
    let file = format!("{HOST}.json");
    #[cfg(target_os = "macos")]
    {
        let _ = data_dir;
        let support = home.join("Library/Application Support");
        for browser in ["Google/Chrome", "Google/Chrome Beta", "Chromium", "Microsoft Edge", "BraveSoftware/Brave-Browser", "Vivaldi"] {
            if support.join(browser).is_dir() {
                put(support.join(browser).join("NativeMessagingHosts").join(&file), manifest(hdm, false));
            }
        }
        if support.join("Firefox").is_dir() {
            put(support.join("Mozilla/NativeMessagingHosts").join(&file), manifest(hdm, true));
        }
    }
    #[cfg(target_os = "linux")]
    {
        // Inside the Flatpak, hdm is at a path only the sandbox sees: browsers start it through `flatpak run`.
        let wrapper;
        let hdm = if Path::new("/.flatpak-info").exists() {
            wrapper = home.join(".local/share/herbs-download-manager/native-host");
            put(wrapper.clone(), b"#!/bin/sh\nexec flatpak run --command=hdm org.herbsdownloadmanager.app \"$@\"\n".to_vec());
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755));
            wrapper.as_path()
        } else {
            hdm
        };
        let _ = data_dir;
        // XDG_CONFIG_HOME points into the sandbox inside the Flatpak: the browsers' folders are under ~/.config.
        let config = home.join(".config");
        for browser in ["google-chrome", "google-chrome-beta", "chromium", "microsoft-edge", "BraveSoftware/Brave-Browser", "vivaldi"] {
            if config.join(browser).is_dir() {
                put(config.join(browser).join("NativeMessagingHosts").join(&file), manifest(hdm, false));
            }
        }
        if home.join(".mozilla").is_dir() {
            put(home.join(".mozilla/native-messaging-hosts").join(&file), manifest(hdm, true));
        }
    }
    #[cfg(windows)]
    {
        let _ = home;
        // Windows finds a host through the registry, which points at its manifest.
        let chromium = data_dir.join("native-host-chromium.json");
        let firefox = data_dir.join("native-host-firefox.json");
        put(chromium.clone(), manifest(hdm, false));
        put(firefox.clone(), manifest(hdm, true));
        for (key, path) in [
            (r"Software\Google\Chrome\NativeMessagingHosts", &chromium),
            (r"Software\Chromium\NativeMessagingHosts", &chromium),
            (r"Software\Microsoft\Edge\NativeMessagingHosts", &chromium),
            (r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts", &chromium),
            (r"Software\Mozilla\NativeMessagingHosts", &firefox),
        ] {
            use std::os::windows::process::CommandExt;
            let status = std::process::Command::new("reg")
                .args(["add", &format!(r"HKCU\{key}\{HOST}"), "/ve", "/t", "REG_SZ", "/d", &path.display().to_string(), "/f"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
                .status();
            if !status.is_ok_and(|s| s.success()) {
                problems.push(format!(r"HKCU\{key}: couldn't be written"));
            }
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_extension_and_its_host_agree() {
        let manifest: serde_json::Value = serde_json::from_slice(FILES[0].1).unwrap();
        assert_eq!(manifest["browser_specific_settings"]["gecko"]["id"], FIREFOX_ID);
        // Chrome's id is the first 128 bits of the key's SHA-256, written with the letters a to p.
        let key = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, manifest["key"].as_str().unwrap()).unwrap();
        let digest = ring::digest::digest(&ring::digest::SHA256, &key);
        let id: String = digest.as_ref()[..16].iter().flat_map(|b| [b >> 4, b & 15]).map(|n| (b'a' + n) as char).collect();
        assert_eq!(id, CHROME_ID);
        let script = std::str::from_utf8(FILES[1].1).unwrap();
        assert!(script.contains(&format!("\"{HOST}\"")));
        // Every file the manifest names is in the copy the app writes.
        let names: Vec<&str> = FILES.iter().map(|(n, _)| *n).collect();
        for named in [manifest["action"]["default_popup"].as_str().unwrap(), "popup.js", "popup.css"] {
            assert!(names.contains(&named), "{named} missing from FILES");
        }
        // And every file of the extension's folder: one left out would be missing from the copy the app writes.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../extension");
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let name = path.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
                    assert!(names.contains(&name.as_str()), "extension/{name} isn't in FILES");
                }
            }
        }
    }

    #[test]
    fn the_extension_is_written_once() {
        let dir = std::env::temp_dir().join(format!("hd-browser-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let folder = extension_folder(&dir).unwrap();
        assert!(folder.join("icons/icon-128.png").is_file());
        let before = std::fs::metadata(folder.join("manifest.json")).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        extension_folder(&dir).unwrap();
        assert_eq!(std::fs::metadata(folder.join("manifest.json")).unwrap().modified().unwrap(), before);
        let firefox: serde_json::Value = serde_json::from_slice(&manifest(Path::new("/x/hdm"), true)).unwrap();
        assert_eq!((firefox["path"].as_str(), firefox["allowed_extensions"][0].as_str()), (Some("/x/hdm"), Some(FIREFOX_ID)));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
