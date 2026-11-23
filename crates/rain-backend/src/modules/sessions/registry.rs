//! Reads the registry of Claude Code: `<claude folder>\sessions\<pid>.json`, one small file for
//! each running session. Section 03 6.1.
//!
//! The format is NOT documented by Claude Code and may change. So every field is optional, an
//! unknown value means "unknown" and never an error, and a file that does not parse is skipped
//! (it may be half written; the next change event reads it again). The registry is used for
//! discovery (a session that Rain never saw start), liveness (the pid and its start time), the
//! name of a session and its host. It does NOT decide "waiting" (decision C1: that comes only
//! from an open permission request).
//!
//! The `.key` files next to the registry files hold keys and are never opened: only names that
//! are exactly `<digits>.json` are read.

use std::io::Read;
use std::path::Path;

use serde_json::Value;

/// A file larger than this is not a registry file.
const FILE_MAX: u64 = 64 * 1024;

/// What the registry says about one session. Anything else in the file is ignored and never kept.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RegistryInfo {
    pub pid: u32,
    pub session_id: String,
    pub cwd: Option<String>,
    /// Epoch milliseconds.
    pub started_at_ms: Option<u64>,
    /// The creation time of the process as a Windows FILETIME. With `pid` it names the process
    /// for good: the pid may be reused by another program later.
    pub proc_start: Option<u64>,
    pub entrypoint: Option<String>,
    pub name: Option<String>,
    /// `user` when the user chose the name, anything else when Claude derived it.
    pub name_source: Option<String>,
    /// `busy`, `idle`, `waiting` or something newer. Only `busy` is used.
    pub status: Option<String>,
    pub status_updated_at_ms: Option<u64>,
    pub updated_at_ms: Option<u64>,
}

/// True for `<digits>.json` and nothing else.
pub fn is_registry_name(name: &str) -> bool {
    name.strip_suffix(".json")
        .is_some_and(|stem| !stem.is_empty() && stem.bytes().all(|b| b.is_ascii_digit()))
}

/// Reads one file. `None` when it is not a registry file, cannot be read, or does not parse.
pub fn read_file(path: &Path) -> Option<RegistryInfo> {
    let name = path.file_name()?.to_str()?;
    if !is_registry_name(name) {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(FILE_MAX + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > FILE_MAX {
        return None;
    }
    parse(&bytes)
}

/// What reading the registry folder found.
#[derive(Debug, Default)]
pub struct Scan {
    pub infos: Vec<RegistryInfo>,
    /// How many files had the name of a registry file. When there are files and not one could
    /// be understood, the format has probably changed.
    pub files: usize,
}

/// Reads every registry file of a folder, in file name order. A folder that cannot be read gives
/// an empty scan.
pub fn scan_dir(dir: &Path) -> Scan {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Scan::default();
    };
    let mut paths: Vec<_> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(is_registry_name)
        })
        .collect();
    paths.sort();
    Scan {
        files: paths.len(),
        infos: paths.iter().filter_map(|path| read_file(path)).collect(),
    }
}

/// The understood files of a folder.
#[cfg(test)]
pub fn read_dir(dir: &Path) -> Vec<RegistryInfo> {
    scan_dir(dir).infos
}

/// The shape of one file. `None` when it is not a JSON object with a pid and a session id: those
/// two are the only required fields.
pub fn parse(bytes: &[u8]) -> Option<RegistryInfo> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let object = value.as_object()?;
    let text = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let number = |key: &str| object.get(key).and_then(Value::as_u64);
    Some(RegistryInfo {
        pid: u32::try_from(number("pid")?).ok()?,
        session_id: text("sessionId")?,
        cwd: text("cwd"),
        started_at_ms: number("startedAt"),
        // `procStart` is written as decimal text, because a FILETIME is larger than what a
        // JavaScript number holds exactly.
        proc_start: text("procStart").and_then(|digits| digits.parse().ok()),
        entrypoint: text("entrypoint"),
        name: text("name"),
        name_source: text("nameSource"),
        status: text("status"),
        status_updated_at_ms: number("statusUpdatedAt"),
        updated_at_ms: number("updatedAt"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"{
        "pid": 4242, "sessionId": "aaaa-bbbb", "cwd": "C:\\work\\demo",
        "startedAt": 1790000000000, "procStart": "134398601336233121",
        "entrypoint": "cli", "name": "Demo", "nameSource": "user",
        "status": "busy", "statusUpdatedAt": 1790000005000, "updatedAt": 1790000006000,
        "peerFeatures": {"a": 1}, "messagingSocketPath": "ignored", "future": [1, 2]
    }"#;

    #[test]
    fn a_full_file_gives_the_fields_that_are_used_and_nothing_else() {
        let info = parse(FULL.as_bytes()).unwrap();
        assert_eq!(info.pid, 4242);
        assert_eq!(info.session_id, "aaaa-bbbb");
        assert_eq!(info.cwd.as_deref(), Some(r"C:\work\demo"));
        assert_eq!(info.proc_start, Some(134_398_601_336_233_121));
        assert_eq!(info.entrypoint.as_deref(), Some("cli"));
        assert_eq!(info.name_source.as_deref(), Some("user"));
        assert_eq!(info.status.as_deref(), Some("busy"));
        assert_eq!(info.updated_at_ms, Some(1_790_000_006_000));
    }

    #[test]
    fn missing_fields_are_none_and_only_pid_and_session_id_are_required() {
        let info = parse(br#"{"pid": 7, "sessionId": "s"}"#).unwrap();
        assert_eq!(info.cwd, None);
        assert_eq!(info.proc_start, None);
        assert_eq!(parse(br#"{"pid": 7}"#), None);
        assert_eq!(parse(br#"{"sessionId": "s"}"#), None);
    }

    #[test]
    fn a_wrong_type_is_the_same_as_a_missing_field() {
        let info = parse(
            br#"{"pid": 7, "sessionId": "s", "startedAt": "soon", "procStart": 12, "status": 5}"#,
        )
        .unwrap();
        assert_eq!(info.started_at_ms, None);
        assert_eq!(info.proc_start, None);
        assert_eq!(info.status, None);
    }

    #[test]
    fn half_written_and_foreign_content_is_skipped() {
        for bad in [
            &b"{\"pid\": 7, \"sess"[..],
            b"",
            b"[1,2]",
            b"\"text\"",
            b"null",
        ] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn only_pid_dot_json_names_are_registry_files() {
        assert!(is_registry_name("4242.json"));
        for name in [
            "4242.abcdef0123.key",
            "4242.json.tmp",
            "abc.json",
            ".json",
            "42 .json",
            "other-file.json",
        ] {
            assert!(!is_registry_name(name), "{name}");
        }
    }

    #[test]
    fn a_folder_is_read_without_opening_key_files() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("11.json"), FULL).unwrap();
        std::fs::write(temp.path().join("12.json"), "{ broken").unwrap();
        // A directory with a registry-like name must not be opened as a file either.
        std::fs::create_dir(temp.path().join("13.json")).unwrap();
        std::fs::write(temp.path().join("11.abcdef.key"), "SECRET-CANARY").unwrap();

        let found = read_dir(temp.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].session_id, "aaaa-bbbb");
        assert!(read_dir(&temp.path().join("missing")).is_empty());
    }
}
