//! Registering hdm with the browsers (SPECIFICATION.md §13), in a made-up home folder. Its own test binary, since it
//! changes HOME for the whole process. Windows registers in the registry, which a test leaves alone.

#![cfg(unix)]

mod common;

use hd_core::browser::{self, CHROME_ID, FIREFOX_ID, HOST};

#[test]
fn hdm_is_registered_with_the_browsers_found_and_only_those() {
    let home = common::Scratch::new("browser-home");
    let (chrome, firefox, missing) = if cfg!(target_os = "macos") {
        ("Library/Application Support/Microsoft Edge", "Library/Application Support/Firefox", "Library/Application Support/Vivaldi")
    } else {
        (".config/microsoft-edge", ".mozilla", ".config/vivaldi")
    };
    std::fs::create_dir_all(home.join(chrome)).unwrap();
    std::fs::create_dir_all(home.join(firefox)).unwrap();
    // SAFETY: this test binary has only this test, so nothing else reads the environment meanwhile.
    unsafe { std::env::set_var("HOME", home.as_path()) };
    let hdm = home.join("app/hdm");
    assert!(browser::register(&hdm, &home.join("data")).is_empty());

    let read = |path: std::path::PathBuf| -> serde_json::Value { serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap() };
    let edge = read(home.join(chrome).join("NativeMessagingHosts").join(format!("{HOST}.json")));
    assert_eq!(edge["allowed_origins"][0], format!("chrome-extension://{CHROME_ID}/"));
    let firefox_hosts = if cfg!(target_os = "macos") { "Library/Application Support/Mozilla/NativeMessagingHosts" } else { ".mozilla/native-messaging-hosts" };
    let fox = read(home.join(firefox_hosts).join(format!("{HOST}.json")));
    assert_eq!(fox["allowed_extensions"][0], FIREFOX_ID);
    // Outside the Flatpak (as here) the browsers start hdm itself.
    assert_eq!(edge["path"], hdm.display().to_string());
    assert!(!home.join(missing).exists(), "a browser that isn't there gets no folder");
}
