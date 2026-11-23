//! The fixed drives of the PC and their sizes. Foundation section 11.6; used by the disk module.
//!
//! About 35 microseconds a drive (spike winapi).

use std::path::PathBuf;

use windows::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
};
use windows::core::PCWSTR;

/// `DRIVE_FIXED`. The named constant is behind another feature of the `windows` crate, so the
/// value is written here.
const DRIVE_FIXED: u32 = 3;

/// One fixed drive. `label` is empty when the volume has none: the caller writes a fallback name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveInfo {
    /// `C`, `D`, ...
    pub letter: char,
    /// `C:\`
    pub root: PathBuf,
    pub label: String,
    pub file_system: String,
    pub total_bytes: u64,
    /// Space available to this user (quota aware).
    pub free_bytes: u64,
}

/// The fixed (not removable, not network) drives that answer.
pub fn fixed_drives() -> Vec<DriveInfo> {
    // SAFETY: `GetLogicalDrives` takes no arguments.
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|bit| mask & (1 << bit) != 0)
        .filter_map(|bit| drive_info(char::from(b'A' + bit)))
        .collect()
}

fn drive_info(letter: char) -> Option<DriveInfo> {
    let root = format!("{letter}:\\");
    let wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
    let root_ptr = PCWSTR(wide.as_ptr());
    // SAFETY: `wide` is NUL-terminated and outlives every call below.
    if unsafe { GetDriveTypeW(root_ptr) } != DRIVE_FIXED {
        return None;
    }
    let mut label = [0u16; 261];
    let mut file_system = [0u16; 261];
    // A failure here (a drive that is locked, for example) only leaves the names empty.
    // SAFETY: both buffers are valid for the lengths that the slices carry.
    let named = unsafe {
        GetVolumeInformationW(
            root_ptr,
            Some(&mut label),
            None,
            None,
            None,
            Some(&mut file_system),
        )
    }
    .is_ok();
    let (mut available, mut total) = (0u64, 0u64);
    // SAFETY: the two out-parameters are valid for writes; the third is optional.
    unsafe { GetDiskFreeSpaceExW(root_ptr, Some(&mut available), Some(&mut total), None) }.ok()?;
    Some(DriveInfo {
        letter,
        root: PathBuf::from(root),
        label: if named {
            text_of(&label)
        } else {
            String::new()
        },
        file_system: if named {
            text_of(&file_system)
        } else {
            String::new()
        },
        total_bytes: total,
        free_bytes: available,
    })
}

/// The text of a NUL-terminated UTF-16 buffer.
fn text_of(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_drive_is_a_fixed_drive_with_sane_sizes() {
        let system = std::env::var("SystemDrive").unwrap();
        let drives = fixed_drives();
        let drive = drives
            .iter()
            .find(|drive| format!("{}:", drive.letter).eq_ignore_ascii_case(&system))
            .expect("the system drive is listed");
        assert!(drive.total_bytes > 0);
        assert!(drive.free_bytes <= drive.total_bytes);
        assert_eq!(
            drive.root,
            PathBuf::from(format!("{}\\", system.to_uppercase()))
        );
        assert!(
            !drive.file_system.is_empty(),
            "a fixed drive has a file system"
        );
    }

    #[test]
    fn the_text_of_a_buffer_ends_at_the_first_nul() {
        let buffer: Vec<u16> = "Data\0junk".encode_utf16().collect();
        assert_eq!(text_of(&buffer), "Data");
        assert_eq!(text_of(&[]), "");
    }
}
