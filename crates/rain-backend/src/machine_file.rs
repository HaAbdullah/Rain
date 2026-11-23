//! `machine.toml`: where the data folder is, and nothing else. Foundation section 10.1 and
//! feature section 06 4.2.
//!
//! The setting that points at the data folder cannot live inside the data folder, so it has a
//! small file of its own in the roaming folder. `Config::from_env` reads it once at start,
//! before any module exists. Only the settings module writes it, after a change of the data
//! folder; a hand edit takes effect at the next start. It is not watched.

use std::io;
use std::path::{Path, PathBuf};

use rain_types::ConfigIssue;
use rain_types::limits::PATH_MAX_CHARS;
use serde::Deserialize;
use toml_edit::DocumentMut;

use crate::store::toml_file::{render, set_value, string_value, write_atomic};

pub const FILE_NAME: &str = "machine.toml";

/// The newest `schema_version` this Rain understands.
const KNOWN_SCHEMA_VERSION: u32 = 1;

const HEADER: &str = "\
# Rain machine file: where your data folder is. Change it in Rain (Settings, Data folder).
";

/// What the file says. `data_dir` is `None` when the key is absent or unusable.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MachineData {
    pub data_dir: Option<PathBuf>,
    pub issues: Vec<ConfigIssue>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Raw {
    schema_version: Option<u32>,
    data_dir: Option<String>,
}

/// Reads the file. A missing file is not a problem. A file that cannot be used gives an issue
/// and no `data_dir`: Rain then uses the default folder (section 06 4.2).
pub fn read(path: &Path) -> MachineData {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => MachineData::default(),
        Err(e) => issue_only(format!("Rain could not read this file ({e})")),
    }
}

/// The rules of the file, on its text. Pure.
pub fn parse(text: &str) -> MachineData {
    let raw: Raw = match toml::from_str(text) {
        Ok(raw) => raw,
        Err(e) => return issue_only(e.message().to_owned()),
    };
    if raw.schema_version.is_some_and(|v| v > KNOWN_SCHEMA_VERSION) {
        return issue_only("Written by a newer Rain".to_owned());
    }
    let Some(text) = raw.data_dir else {
        return MachineData::default();
    };
    match absolute_folder(&text) {
        Some(folder) => MachineData {
            data_dir: Some(folder),
            issues: Vec::new(),
        },
        None => issue_only("data_dir must be a full path such as D:\\RainData".to_owned()),
    }
}

fn issue_only(message: String) -> MachineData {
    MachineData {
        data_dir: None,
        issues: vec![ConfigIssue {
            file: FILE_NAME.to_owned(),
            line: None,
            message,
        }],
    }
}

/// An absolute path: a drive letter or a network share, at most 1024 characters, no control
/// characters.
fn absolute_folder(text: &str) -> Option<PathBuf> {
    let text = text.trim();
    let bytes = text.as_bytes();
    let has_drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    let ok = (has_drive || text.starts_with(r"\\"))
        && text.chars().count() <= PATH_MAX_CHARS
        && !text.chars().any(char::is_control);
    ok.then(|| PathBuf::from(text))
}

/// Writes the file with `data_dir` set, or with the key removed (`None`: the default folder).
/// A new file gets its header and `schema_version`; an existing one keeps its comments. The write
/// is atomic like every write of a TOML file (foundation 10.4). Blocking.
pub fn write_data_dir(path: &Path, data_dir: Option<&Path>) -> io::Result<()> {
    let existing = match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let crlf = existing
        .as_deref()
        .is_some_and(|text| text.contains("\r\n"));
    let mut document: DocumentMut = existing
        .as_deref()
        .unwrap_or_default()
        .parse()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{e}")))?;

    match data_dir {
        Some(folder) => {
            if existing.is_none() {
                new_file_header(&mut document);
            }
            set_value(
                document.as_table_mut(),
                "data_dir",
                string_value(&folder.to_string_lossy()),
            );
        }
        None => {
            document.remove("data_dir");
        }
    }
    write_atomic(path, &render(&document, crlf))
}

fn new_file_header(document: &mut DocumentMut) {
    document.insert("schema_version", toml_edit::value(1));
    if let Some(mut key) = document.key_mut("schema_version") {
        key.leaf_decor_mut().set_prefix(HEADER);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data_dir(text: &str) -> Option<String> {
        parse(text)
            .data_dir
            .map(|p| p.to_string_lossy().into_owned())
    }

    #[test]
    fn a_missing_file_means_no_data_dir_and_no_issue() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(read(&temp.path().join(FILE_NAME)), MachineData::default());
    }

    #[test]
    fn a_full_path_is_read_as_typed() {
        assert_eq!(
            data_dir("schema_version = 1\ndata_dir = 'D:\\RainData'\n").as_deref(),
            Some("D:\\RainData")
        );
        assert_eq!(
            data_dir("data_dir = '\\\\server\\share\\rain'\n").as_deref(),
            Some("\\\\server\\share\\rain")
        );
    }

    #[test]
    fn a_path_that_is_not_absolute_is_an_issue_and_is_not_used() {
        for text in [
            "data_dir = 'RainData'\n",
            "data_dir = 'C:foo'\n",
            "data_dir = ''\n",
            // A basic TOML string: `\u0001` becomes a control character.
            "data_dir = \"D:\\\\a\\u0001b\"\n",
        ] {
            let read = parse(text);
            assert_eq!(read.data_dir, None, "{text}");
            assert_eq!(read.issues.len(), 1, "{text}");
        }
    }

    #[test]
    fn a_path_over_1024_characters_is_refused() {
        let text = format!("data_dir = 'D:\\{}'\n", "x".repeat(1100));
        assert_eq!(parse(&text).data_dir, None);
    }

    #[test]
    fn a_newer_schema_is_ignored_with_an_issue() {
        let read = parse("schema_version = 2\ndata_dir = 'D:\\RainData'\n");
        assert_eq!(read.data_dir, None);
        assert_eq!(read.issues[0].message, "Written by a newer Rain");
        assert_eq!(read.issues[0].file, "machine.toml");
    }

    #[test]
    fn a_file_with_a_syntax_error_is_an_issue() {
        let read = parse("data_dir = = 1\n");
        assert_eq!(read.data_dir, None);
        assert_eq!(read.issues.len(), 1);
    }

    #[test]
    fn writing_makes_a_file_that_reads_back_with_its_header() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(FILE_NAME);
        write_data_dir(&path, Some(Path::new(r"D:\Rain Data"))).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# Rain machine file"), "{text}");
        assert!(text.contains("schema_version = 1"));
        assert!(text.contains(r"data_dir = 'D:\Rain Data'"), "{text}");
        assert_eq!(read(&path).data_dir, Some(PathBuf::from(r"D:\Rain Data")));
    }

    #[test]
    fn writing_the_default_removes_only_the_key() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(FILE_NAME);
        std::fs::write(
            &path,
            "# my note\nschema_version = 1\ndata_dir = 'D:\\Old'  # old place\nother = 1\n",
        )
        .unwrap();
        write_data_dir(&path, None).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, "# my note\nschema_version = 1\nother = 1\n");
    }

    #[test]
    fn changing_the_folder_keeps_the_comment_after_the_value() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(FILE_NAME);
        std::fs::write(
            &path,
            "schema_version = 1\ndata_dir = 'D:\\Old'  # old place\n",
        )
        .unwrap();
        write_data_dir(&path, Some(Path::new(r"E:\New"))).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("data_dir = 'E:\\New'  # old place"), "{text}");
    }

    #[test]
    fn a_file_with_crlf_keeps_its_line_ends() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(FILE_NAME);
        std::fs::write(&path, "schema_version = 1\r\n").unwrap();
        write_data_dir(&path, Some(Path::new(r"E:\New"))).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("\r\n") && !text.replace("\r\n", "").contains('\n'),
            "{text:?}"
        );
    }
}
