//! File names that work on every system (SPECIFICATION.md §5.6): what a server, a page or a person names a file is
//! fitted before the file is made, so no download fails for its name.

use std::path::Path;

/// Bytes a name may take: 255 is every system's limit for one name, less room for " (12)" and ".hdpart(s)".
const NAME_BYTES: usize = 230;
/// Characters a whole path may take on Windows, where many programs still stop at 260.
#[cfg(windows)]
const PATH_CHARS: usize = 250;

/// Names Windows keeps for devices, whatever their extension.
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3",
    "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// `name` made safe and short enough to be saved in `folder` on this system: characters a system refuses become
/// `_`, leading dots (hidden files) and trailing dots and spaces go, a device name gets `_` after it, and a name too
/// long is shortened before its extension, which is kept.
pub fn fit(folder: &Path, name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { '_' } else { c }).collect();
    let mut name = cleaned.trim().trim_start_matches('.').trim_end_matches(['.', ' ']).trim().to_string();
    if name.is_empty() {
        name = "download".into();
    }
    let stem = name.split('.').next().unwrap_or("").trim().to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        name = match name.split_once('.') {
            Some((a, b)) => format!("{a}_.{b}"),
            None => format!("{name}_"),
        };
    }
    let limit = room(folder);
    if name.len() <= limit {
        return name;
    }
    // Keep a short extension ("…mp4"); a long "extension" is part of the name.
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && e.len() <= 10 => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    let mut keep = limit.saturating_sub(ext.len()).max(1);
    while keep > 0 && !stem.is_char_boundary(keep.min(stem.len())) {
        keep -= 1;
    }
    format!("{}{ext}", stem[..keep.min(stem.len())].trim_end_matches(['.', ' ']))
}

/// Bytes the name may take in `folder`.
fn room(folder: &Path) -> usize {
    #[cfg(windows)]
    {
        let used = folder.to_string_lossy().encode_utf16().count() + 1 + ".hdparts".len();
        // At least a short name, even in a deep folder.
        return NAME_BYTES.min(PATH_CHARS.saturating_sub(used).max(24));
    }
    #[cfg(not(windows))]
    {
        let _ = folder;
        NAME_BYTES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_fitted() {
        let here = Path::new("/tmp");
        assert_eq!(fit(here, "a/b:c*d?.mp4"), "a_b_c_d_.mp4");
        assert_eq!(fit(here, "  ..hidden.txt. "), "hidden.txt");
        assert_eq!(fit(here, "con.txt"), "con_.txt");
        assert_eq!(fit(here, "NUL"), "NUL_");
        assert_eq!(fit(here, "console.txt"), "console.txt", "only a device name exactly");
        assert_eq!(fit(here, "..."), "download");
        let long = format!("{}.mp4", "a".repeat(400));
        let fitted = fit(here, &long);
        assert!(fitted.len() <= NAME_BYTES && fitted.ends_with(".mp4"), "{}", fitted.len());
        // Cut on a character, never inside one.
        let wide = format!("{}.mkv", "é".repeat(300));
        let fitted = fit(here, &wide);
        assert!(fitted.len() <= NAME_BYTES && fitted.ends_with(".mkv") && fitted.starts_with('é'));
    }

    #[cfg(windows)]
    #[test]
    fn a_deep_folder_leaves_less_room_on_windows() {
        let deep = Path::new("C:\\").join("x".repeat(180));
        let fitted = fit(&deep, &format!("{}.zip", "b".repeat(200)));
        assert!(deep.join(&fitted).to_string_lossy().encode_utf16().count() + ".hdparts".len() <= PATH_CHARS);
        assert!(fitted.ends_with(".zip"));
    }
}
