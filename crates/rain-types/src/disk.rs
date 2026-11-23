//! Disk and machine: drives, scans, the path check and the idle state.
//! Feature section 05 (`docs/backend/05-disk-machine.md`, sections 2 and 3).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The eight colour classes of the treemap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum FileCategory {
    Video,
    Image,
    Audio,
    Archive,
    Code,
    Document,
    App,
    Other,
}

/// What a node is. `Other` is the one node per folder that stands for the entries that are not
/// listed one by one ("other (1,204 items)").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum NodeKind {
    Dir,
    File,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ScanStatus {
    Scanning,
    Done,
    Failed,
    Cancelled,
}

/// One fixed local drive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Drive {
    // "drive-c": "drive-" and the lower-case letter. Stable for a letter.
    pub id: String,
    // "C" (no colon; the screen adds it).
    pub letter: String,
    // The volume label, or "Local Disk" when Windows has none (never empty).
    pub label: String,
    // Always above 0: a drive with 0 is not listed.
    pub total_bytes: u64,
    // Total minus what is available to this user.
    pub used_bytes: u64,
}

/// One folder, file or "other" entry of a scan result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct DiskNode {
    pub name: String,
    // Root `C:\`; a child is its parent's path, a backslash and its name. The `other` node's path
    // is its folder's path plus `\*` (a star cannot occur in a file name, so the path is unique).
    pub path: String,
    // Logical size of the whole subtree; 0 for a folder that Windows did not let Rain read.
    pub size_bytes: u64,
    #[ts(inline)]
    pub kind: NodeKind,
    // Files: by extension. A folder: only one that is sent without children (the category that
    // holds most bytes). Absent for an `other` node and a denied folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(inline)]
    pub category: Option<FileCategory>,
    // Absent for a file, an empty folder, a denied folder and a truncated folder. Never `[]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<DiskNode>>,
    // Only ever `true`: the folder has content that is not in this answer (getScanNode fetches it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    // Only ever `true`: Windows did not let Rain read this folder ("not accessible").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub denied: Option<bool>,
}

/// One scan. The topic `scan` carries this without `root`; `GET /scans/{id}` adds `root` when the
/// scan is done and its tree is still held.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct DiskScan {
    // "scan-20261122-093000-1"
    pub id: String,
    pub drive_id: String,
    #[ts(inline)]
    pub status: ScanStatus,
    // 0 to 1, three digits; at most 0.99 until the scan is done, then 1.
    pub progress: f64,
    // Files only, not folders.
    pub files_seen: u64,
    // Only when the status is `failed`: a short text, shown as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    // Folders that Windows did not let Rain read. Absent when 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_accessible: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<DiskNode>,
}

/// Answer of `GET /machine`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MachineState {
    // The last input is younger than 60 seconds.
    pub user_active: bool,
    // Whole minutes since the last input (0 while the user is active).
    pub idle_minutes: u32,
}

/// Body of `POST /scans`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartScanBody {
    pub drive_id: String,
}

/// Body of `POST /shell/reveal`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RevealBody {
    pub path: String,
}

/// Body of `POST /paths/check`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PathsCheckBody {
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PathKind {
    Dir,
    File,
}

/// What `POST /paths/check` says about one path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct PathCheck {
    // The text that was sent, unchanged, so that the caller can match it.
    pub path: String,
    pub exists: bool,
    // Present when `exists` is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(inline)]
    pub kind: Option<PathKind>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::JSON_INT_MAX;
    use crate::test_support::round_trip;
    use serde_json::json;

    fn file(name: &str, size: u64) -> DiskNode {
        DiskNode {
            name: name.into(),
            path: format!("C:\\{name}"),
            size_bytes: size,
            kind: NodeKind::File,
            category: Some(FileCategory::Video),
            children: None,
            truncated: None,
            denied: None,
        }
    }

    #[test]
    fn a_minimal_node_has_no_optional_keys() {
        let mut node = file("a.mp4", 5);
        node.category = None;
        assert_eq!(
            round_trip(&node),
            json!({ "name": "a.mp4", "path": "C:\\a.mp4", "sizeBytes": 5, "kind": "file" })
        );
    }

    #[test]
    fn a_full_node_with_a_recursive_child_round_trips() {
        let mut locked = file("locked", 0);
        locked.kind = NodeKind::Dir;
        locked.category = None;
        locked.denied = Some(true);
        let folder = DiskNode {
            name: "C:".into(),
            path: "C:\\".into(),
            size_bytes: 105,
            kind: NodeKind::Dir,
            category: None,
            children: Some(vec![file("a.mp4", 100), locked]),
            truncated: Some(true),
            denied: None,
        };
        let json = round_trip(&folder);
        assert_eq!(json["children"][0]["category"], "video");
        assert_eq!(json["children"][1]["denied"], true);
        assert_eq!(json["truncated"], true);
    }

    #[test]
    fn a_scan_frame_matches_the_documented_shape() {
        let scan = DiskScan {
            id: "scan-20261122-093000-1".into(),
            drive_id: "drive-c".into(),
            status: ScanStatus::Scanning,
            progress: 0.5,
            files_seen: 12,
            error: None,
            not_accessible: Some(2),
            root: None,
        };
        assert_eq!(
            round_trip(&scan),
            json!({
                "id": "scan-20261122-093000-1", "driveId": "drive-c", "status": "scanning",
                "progress": 0.5, "filesSeen": 12, "notAccessible": 2
            })
        );
    }

    #[test]
    fn drives_machine_state_and_path_checks_round_trip() {
        let drive = Drive {
            id: "drive-c".into(),
            letter: "C".into(),
            label: "Local Disk".into(),
            total_bytes: 100,
            used_bytes: 40,
        };
        assert_eq!(
            round_trip(&drive),
            json!({ "id": "drive-c", "letter": "C", "label": "Local Disk", "totalBytes": 100, "usedBytes": 40 })
        );
        let state = MachineState {
            user_active: false,
            idle_minutes: 7,
        };
        assert_eq!(
            round_trip(&state),
            json!({ "userActive": false, "idleMinutes": 7 })
        );
        let found = PathCheck {
            path: "C:\\x".into(),
            exists: true,
            kind: Some(PathKind::Dir),
        };
        assert_eq!(
            round_trip(&found),
            json!({ "path": "C:\\x", "exists": true, "kind": "dir" })
        );
        let missing = PathCheck {
            path: "C:\\y".into(),
            exists: false,
            kind: None,
        };
        assert_eq!(
            round_trip(&missing),
            json!({ "path": "C:\\y", "exists": false })
        );
    }

    #[test]
    fn bodies_refuse_unknown_fields() {
        assert!(
            serde_json::from_value::<StartScanBody>(json!({ "driveId": "d", "x": 1 })).is_err()
        );
        assert!(serde_json::from_value::<RevealBody>(json!({ "path": "C:\\", "x": 1 })).is_err());
        assert!(serde_json::from_value::<PathsCheckBody>(json!({ "paths": [], "x": 1 })).is_err());
        assert_eq!(
            serde_json::from_value::<PathsCheckBody>(json!({ "paths": ["a"] })).unwrap(),
            PathsCheckBody {
                paths: vec!["a".into()]
            }
        );
    }

    #[test]
    fn the_largest_size_a_node_can_hold_stays_exact_in_javascript() {
        let node = file("big", JSON_INT_MAX);
        let json = round_trip(&node);
        assert_eq!(json["sizeBytes"].as_u64(), Some(JSON_INT_MAX));
    }
}
