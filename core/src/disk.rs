//! Will a download fit where it's going (enhancements.md: free space check)? The free space and file system of the
//! drive a folder is on, read from the system: statvfs/statfs on Linux and macOS (plus the space macOS frees on demand,
//! as Finder counts it), GetDiskFreeSpaceExW/GetVolumeInformationW on Windows.

use std::path::Path;

/// Room left beyond the file itself, for the system and other programs.
/// Space always left free on a disk, so the system and other apps aren't starved (as JDownloader keeps 128 MB).
pub const MARGIN: u64 = 128 * 1024 * 1024;
/// FAT32's largest file: 4 GiB − 1.
pub const FAT32_LIMIT: u64 = (4 << 30) - 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Volume {
    /// Bytes a new file may take.
    pub available: u64,
    /// As the system names it ("apfs", "ext4", "NTFS", "msdos", "vfat", "FAT32", "exFAT"…), or empty.
    pub file_system: String,
}

impl Volume {
    pub fn is_fat32(&self) -> bool {
        matches!(self.file_system.to_ascii_lowercase().as_str(), "fat32" | "vfat" | "msdos" | "fat")
    }
}

/// The drive `path` is on, or will be: a missing folder is looked up through the nearest folder above it that exists.
/// `None` when nothing of the path exists (a drive not plugged in) or the system can't tell.
pub fn volume_of(path: &Path) -> Option<Volume> {
    let mut p = path;
    while !p.exists() {
        p = p.parent()?;
    }
    let mut v = probe(p)?;
    let pretend = FREE_FOR_TESTS.load(std::sync::atomic::Ordering::SeqCst);
    if pretend != u64::MAX {
        v.available = pretend;
    }
    Some(v)
}

static FREE_FOR_TESTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);

/// Tests: pretend every disk has this much free space (None: the real amount).
#[doc(hidden)]
pub fn pretend_free_space(bytes: Option<u64>) {
    FREE_FOR_TESTS.store(bytes.unwrap_or(u64::MAX), std::sync::atomic::Ordering::SeqCst);
}

/// Why a file of `size` bytes can't go into `folder`, or `None` when it fits (or the system can't tell).
pub fn space_problem(folder: &Path, size: u64) -> Option<String> {
    let v = volume_of(folder)?;
    let place = folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| folder.display().to_string());
    if v.is_fat32() && size > FAT32_LIMIT {
        return Some(format!(
            "{place} is on a FAT32 drive, which can't hold files of 4 GB or more ({} here): choose a folder on another drive",
            crate::format::bytes(size)
        ));
    }
    if size.saturating_add(MARGIN) > v.available {
        return Some(format!(
            "Not enough space in {place}: the file needs {} and {} is free",
            crate::format::bytes(size),
            crate::format::bytes(v.available)
        ));
    }
    None
}

/// While downloading: why to stop when the folder's disk is nearly full (less than MARGIN free), so a download never
/// fills it. What's downloaded is kept, and Resume carries on once there's room.
pub fn nearly_full(folder: &Path) -> Option<String> {
    let v = volume_of(folder)?;
    (v.available < MARGIN).then(|| {
        format!("The disk is nearly full ({} free): free some space, then Resume", crate::format::bytes(v.available))
    })
}

#[cfg(unix)]
fn probe(path: &Path) -> Option<Volume> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut vfs: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut vfs) } != 0 {
        return None;
    }
    #[allow(clippy::unnecessary_cast)]
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut available = vfs.f_bavail as u64 * vfs.f_frsize as u64;
    let mut fs: libc::statfs = unsafe { std::mem::zeroed() };
    let file_system = if unsafe { libc::statfs(c.as_ptr(), &mut fs) } == 0 { fs_name(&fs) } else { String::new() };
    #[cfg(target_os = "macos")]
    if let Some(important) = macos_important_capacity(path) {
        available = available.max(important);
    }
    Some(Volume { available, file_system })
}

#[cfg(target_os = "macos")]
fn fs_name(fs: &libc::statfs) -> String {
    let bytes: Vec<u8> = fs.f_fstypename.iter().take_while(|&&c| c != 0).map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn fs_name(fs: &libc::statfs) -> String {
    #[allow(clippy::unnecessary_cast)]
    match fs.f_type as i64 {
        0x4d44 => "vfat".into(),
        0x2011_BAB0 => "exfat".into(),
        0xEF53 => "ext4".into(),
        0x9123_683E => "btrfs".into(),
        0x5846_5342 => "xfs".into(),
        0x6969 => "nfs".into(),
        0x0102_1994 => "tmpfs".into(),
        _ => String::new(),
    }
}

/// The volume's capacity "for important usage": the free space plus what macOS frees on demand (caches, local
/// snapshots), as Finder shows it. statvfs leaves that out and would refuse a download that fits.
#[cfg(target_os = "macos")]
fn macos_important_capacity(path: &Path) -> Option<u64> {
    use objc2_foundation::{NSArray, NSNumber, NSString, NSURL, NSURLVolumeAvailableCapacityForImportantUsageKey};
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let key = unsafe { NSURLVolumeAvailableCapacityForImportantUsageKey };
    let values = url.resourceValuesForKeys_error(&NSArray::from_slice(&[key])).ok()?;
    let number = values.objectForKey(key)?.downcast::<NSNumber>().ok()?;
    u64::try_from(number.longLongValue()).ok()
}

#[cfg(windows)]
fn probe(path: &Path) -> Option<Volume> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetVolumeInformationW, GetVolumePathNameW};
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut available = 0u64;
    if unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, std::ptr::null_mut(), std::ptr::null_mut()) } == 0 {
        return None;
    }
    let mut root = [0u16; 260];
    let mut name = [0u16; 64];
    let file_system = if unsafe { GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), root.len() as u32) } != 0
        && unsafe {
            GetVolumeInformationW(root.as_ptr(), std::ptr::null_mut(), 0, std::ptr::null_mut(), std::ptr::null_mut(),
                                  std::ptr::null_mut(), name.as_mut_ptr(), name.len() as u32)
        } != 0
    {
        String::from_utf16_lossy(&name[..name.iter().position(|&c| c == 0).unwrap_or(0)])
    } else {
        String::new()
    };
    Some(Volume { available, file_system })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_temp_folder_has_a_volume_and_a_huge_file_does_not_fit() {
        let v = volume_of(&std::env::temp_dir().join("no/such/folder")).expect("the temp folder's drive");
        assert!(v.available > 0);
        assert!(space_problem(&std::env::temp_dir(), u64::MAX / 2).unwrap().starts_with("Not enough space"));
        assert_eq!(space_problem(&std::env::temp_dir(), 1), None);
    }

    #[test]
    fn fat32_refuses_files_of_4_gb() {
        assert!(Volume { available: u64::MAX, file_system: "msdos".into() }.is_fat32());
        assert!(!Volume { available: 0, file_system: "exFAT".into() }.is_fat32());
    }
}
