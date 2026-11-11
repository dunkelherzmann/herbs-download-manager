//! The app's version for `hd_core::version()`: written by each platform's build script into `version.txt` at the
//! repository's root ("0.1.113", from scripts/version.sh), or given as HDM_VERSION; the crate's own version when
//! neither is there (a plain `cargo test`).
fn main() {
    println!("cargo:rerun-if-env-changed=HDM_VERSION");
    println!("cargo:rerun-if-changed=../version.txt");
    let version = std::env::var("HDM_VERSION")
        .ok()
        .or_else(|| std::fs::read_to_string("../version.txt").ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || c == '.'))
        .unwrap_or_else(|| std::env::var("CARGO_PKG_VERSION").unwrap_or_default());
    println!("cargo:rustc-env=HDM_APP_VERSION={version}");
}
