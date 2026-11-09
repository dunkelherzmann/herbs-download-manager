//! Herb's Download Manager engine. It has no UI: each platform's app draws its own window over it (SPECIFICATION.md §1).

pub mod format;
pub mod http;
pub mod limit;
pub mod links;
pub mod manager;
pub mod segments;
pub mod settings;
pub mod transfer;

/// The core's version, shown in the app's About window.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
