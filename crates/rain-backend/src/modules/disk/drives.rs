//! The list of drives (`GET /drives`). Section 05 6.1.

use std::path::PathBuf;

use rain_types::Drive;

use crate::platform::fs::{DriveInfo as RawDrive, fixed_drives};

/// The label that a volume without a name gets. The screen writes "C: Local Disk".
pub const NO_LABEL: &str = "Local Disk";

/// A drive for the screen, and where its files are.
#[derive(Debug, Clone)]
pub struct DriveInfo {
    pub drive: Drive,
    /// Where the walker starts. `C:\` on a real drive.
    pub root: PathBuf,
    /// The name of the root node and the start of every path of the scan: `C:`.
    pub root_name: String,
}

/// Where the drives come from. The seam of the tests: a fake lists folders in a temp directory.
pub trait DriveSource: Send + Sync {
    /// Reads the drives now. Blocking: a disk that sleeps may take seconds to answer.
    fn list(&self) -> Vec<DriveInfo>;
}

/// The fixed local drives of this PC.
pub struct SystemDrives;

impl DriveSource for SystemDrives {
    fn list(&self) -> Vec<DriveInfo> {
        let mut drives: Vec<DriveInfo> = fixed_drives().into_iter().filter_map(from_raw).collect();
        drives.sort_by(|a, b| a.drive.letter.cmp(&b.drive.letter));
        drives
    }
}

/// `drive-c` for the letter `C`.
pub fn drive_id(letter: char) -> String {
    format!("drive-{}", letter.to_ascii_lowercase())
}

/// Turns what Windows reported into a drive of the API. A drive with no size (an empty card
/// reader, an offline disk) is not listed: that also keeps the screen from dividing by zero.
fn from_raw(raw: RawDrive) -> Option<DriveInfo> {
    if raw.total_bytes == 0 {
        return None;
    }
    let letter = raw.letter.to_ascii_uppercase();
    let label = raw.label.trim();
    Some(DriveInfo {
        drive: Drive {
            id: drive_id(letter),
            letter: letter.to_string(),
            label: if label.is_empty() {
                NO_LABEL.to_owned()
            } else {
                label.to_owned()
            },
            total_bytes: raw.total_bytes,
            used_bytes: raw.total_bytes.saturating_sub(raw.free_bytes),
        },
        root: raw.root,
        root_name: format!("{letter}:"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(letter: char, label: &str, total: u64, free: u64) -> RawDrive {
        RawDrive {
            letter,
            root: PathBuf::from(format!("{letter}:\\")),
            label: label.to_owned(),
            file_system: "NTFS".to_owned(),
            total_bytes: total,
            free_bytes: free,
        }
    }

    #[test]
    fn a_drive_gets_an_id_a_used_size_and_a_root_name() {
        let info = from_raw(raw('d', "Data", 1000, 400)).unwrap();
        assert_eq!(info.drive.id, "drive-d");
        assert_eq!(info.drive.letter, "D");
        assert_eq!(info.drive.label, "Data");
        assert_eq!(info.drive.total_bytes, 1000);
        assert_eq!(info.drive.used_bytes, 600);
        assert_eq!(info.root_name, "D:");
    }

    #[test]
    fn an_empty_label_becomes_local_disk() {
        assert_eq!(
            from_raw(raw('C', "", 10, 5)).unwrap().drive.label,
            "Local Disk"
        );
        assert_eq!(
            from_raw(raw('C', "  ", 10, 5)).unwrap().drive.label,
            "Local Disk"
        );
    }

    #[test]
    fn a_drive_without_a_size_is_not_listed() {
        assert!(from_raw(raw('E', "Card", 0, 0)).is_none());
    }

    #[test]
    fn more_free_space_than_total_does_not_underflow() {
        assert_eq!(from_raw(raw('C', "", 10, 50)).unwrap().drive.used_bytes, 0);
    }

    #[test]
    fn ids_follow_the_letter() {
        assert_eq!(drive_id('C'), "drive-c");
        assert_eq!(drive_id('z'), "drive-z");
    }

    #[test]
    fn the_real_drives_are_sorted_and_have_sizes() {
        let drives = SystemDrives.list();
        assert!(!drives.is_empty(), "a PC has a fixed drive");
        assert!(
            drives
                .windows(2)
                .all(|w| w[0].drive.letter < w[1].drive.letter)
        );
        assert!(
            drives
                .iter()
                .all(|d| d.drive.total_bytes > 0 && !d.drive.label.is_empty())
        );
    }
}
