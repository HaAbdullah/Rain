//! Where the daily note of a date is, and the link that opens it in Obsidian. Section 04 6.2
//! and 6.9. Pure path and text work: nothing here reads a file.
//!
//! The settings module already decided which vault, folder and format are in effect (override,
//! else Obsidian's own `daily-notes.json`, else `YYYY-MM-DD`). This file only turns that and a
//! date into a path. It is date driven: format the date, join the folder, look at that one path.
//! It never scans a folder and never parses a file name back into a date.

use std::path::{Path, PathBuf};

use super::dates::CivilDate;
use super::moment;

/// What is wrong with the notes setup, in terms of the reasons of the API (section 8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// No vault is chosen (`noVault`).
    NoVault,
    /// The format cannot be rendered, or renders to a name that cannot be a path (`badDailyFormat`).
    /// The text is the sentence for the user.
    BadFormat(String),
    /// The folder setting is not a folder inside the vault (`badDailyFolder`).
    BadFolder,
}

/// The notes setup in effect: the vault, the folder inside it (normalised, `/` separated, may be
/// empty) and the Moment format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyConfig {
    pub vault: PathBuf,
    pub folder: String,
    pub format: String,
}

impl DailyConfig {
    /// Checks the three values. The vault folder itself may still be missing on disk: that is
    /// `vaultMissing`, found by the caller with one `metadata` call.
    pub fn new(vault: Option<&Path>, folder: &str, format: &str) -> Result<Self, ConfigError> {
        let vault = vault.ok_or(ConfigError::NoVault)?.to_path_buf();
        let folder = normalise_folder(folder).ok_or(ConfigError::BadFolder)?;
        moment::validate(format).map_err(ConfigError::BadFormat)?;
        Ok(Self {
            vault,
            folder,
            format: format.to_owned(),
        })
    }

    /// The folder of the daily notes, as a path.
    pub fn folder_path(&self) -> PathBuf {
        let mut path = self.vault.clone();
        path.extend(self.folder.split('/').filter(|part| !part.is_empty()));
        path
    }

    /// The note's name relative to the vault, `/` separated, without `.md`:
    /// `Dailys/2026-11-22`. The format may hold `/` (a nested folder per month, for example).
    pub fn relative_name(&self, date: CivilDate) -> Result<String, ConfigError> {
        let name = moment::render(&self.format, date)
            .map_err(|e| ConfigError::BadFormat(e.to_string()))?;
        let mut parts: Vec<&str> = Vec::new();
        for part in name.split('/') {
            if !usable_part(part) {
                return Err(ConfigError::BadFormat(
                    "The daily note format gives a name that Rain cannot use as a file name. \
                     Change it in Settings."
                        .to_owned(),
                ));
            }
            parts.push(part);
        }
        let folder = self.folder.as_str();
        Ok(if folder.is_empty() {
            parts.join("/")
        } else {
            format!("{folder}/{}", parts.join("/"))
        })
    }

    /// The full path of the note of `date`.
    pub fn file_for(&self, date: CivilDate) -> Result<PathBuf, ConfigError> {
        let relative = self.relative_name(date)?;
        let mut path = self.vault.clone();
        path.extend(relative.split('/'));
        // `set_extension` would cut a name such as `2026.11.22`; the `.md` is appended.
        let mut name = path.into_os_string();
        name.push(".md");
        Ok(PathBuf::from(name))
    }

    /// Identifies vault, folder and format together, so that a changed setting never mixes old
    /// cached rows with new ones. Not a security feature: any stable hash will do.
    pub fn scope(&self) -> String {
        let text = format!(
            "{}|{}|{}",
            self.vault.to_string_lossy().to_lowercase(),
            self.folder,
            self.format
        );
        format!("{:016x}", fnv1a64(text.as_bytes()))
    }

    /// The `obsidian://open` link of the note of `date`. With the id of the vault in Obsidian's
    /// own list the link is `?vault=<id>&file=<relative name>`. Without an id Rain falls back to
    /// `?path=<absolute path>` (not proven for Windows paths; appendix C of the design).
    pub fn open_link(
        &self,
        date: CivilDate,
        vault_id: Option<&str>,
    ) -> Result<String, ConfigError> {
        Ok(match vault_id {
            Some(id) => format!(
                "obsidian://open?vault={}&file={}",
                percent_encode(id),
                percent_encode(&self.relative_name(date)?)
            ),
            None => format!(
                "obsidian://open?path={}",
                percent_encode(&self.file_for(date)?.to_string_lossy().replace('\\', "/"))
            ),
        })
    }
}

/// A part of a file name that may be used: not empty, not `.` or `..`, no drive colon, no
/// control character and no backslash (a separator is always `/` after the render).
fn usable_part(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && !part.contains(':')
        && !part.contains('\\')
        && !part.chars().any(char::is_control)
}

/// `\` becomes `/`, empty parts and `.` are dropped. A part that is `..`, holds a colon (a
/// drive) or a control character makes the folder invalid (`None`): the folder must lie inside
/// the vault. `""` is the vault root.
pub fn normalise_folder(raw: &str) -> Option<String> {
    let mut parts = Vec::new();
    let unified = raw.replace('\\', "/");
    for part in unified.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            _ if part.contains(':') || part.chars().any(char::is_control) => return None,
            _ => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

/// Every byte except `A-Z a-z 0-9 - . _ ~` becomes `%XX` (UTF-8 bytes for other characters).
/// A `/` is encoded too: in the `file` value it is `%2F`.
pub fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// FNV-1a, 64 bit.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> CivilDate {
        CivilDate::parse(text).expect("a real date")
    }

    fn config(folder: &str, format: &str) -> DailyConfig {
        DailyConfig::new(Some(Path::new(r"C:\Notes\Vault")), folder, format).expect("a config")
    }

    #[test]
    fn the_folder_is_normalised_and_must_stay_inside_the_vault() {
        assert_eq!(normalise_folder(""), Some(String::new()));
        assert_eq!(normalise_folder("Dailys"), Some("Dailys".into()));
        assert_eq!(
            normalise_folder(r"\Dailys\\2026\"),
            Some("Dailys/2026".into())
        );
        assert_eq!(normalise_folder("./a/./b/"), Some("a/b".into()));
        assert_eq!(normalise_folder("/"), Some(String::new()));
        for bad in [
            "..",
            "a/../b",
            r"..\a",
            "C:",
            r"C:\Windows",
            "a\u{7}b",
            "a:b",
        ] {
            assert_eq!(normalise_folder(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_missing_vault_a_bad_folder_and_a_bad_format_are_told_apart() {
        assert_eq!(
            DailyConfig::new(None, "Dailys", "YYYY-MM-DD"),
            Err(ConfigError::NoVault)
        );
        assert_eq!(
            DailyConfig::new(Some(Path::new("C:/V")), "..", "YYYY-MM-DD"),
            Err(ConfigError::BadFolder)
        );
        let bad = DailyConfig::new(Some(Path::new("C:/V")), "", "YYYY-MM-DD HH:mm");
        assert!(matches!(bad, Err(ConfigError::BadFormat(text)) if text.contains("HH")));
    }

    #[test]
    fn the_path_is_vault_folder_name_and_md() {
        let cfg = config("Dailys", "YYYY-MM-DD");
        assert_eq!(
            cfg.file_for(date("2026-11-22")).unwrap(),
            PathBuf::from(r"C:\Notes\Vault\Dailys\2026-11-22.md")
        );
        assert_eq!(
            cfg.relative_name(date("2026-11-22")).unwrap(),
            "Dailys/2026-11-22"
        );
        let root = config("", "YYYY-MM-DD");
        assert_eq!(
            root.file_for(date("2026-01-05")).unwrap(),
            PathBuf::from(r"C:\Notes\Vault\2026-01-05.md")
        );
        assert_eq!(cfg.folder_path(), PathBuf::from(r"C:\Notes\Vault\Dailys"));
        assert_eq!(root.folder_path(), PathBuf::from(r"C:\Notes\Vault"));
    }

    #[test]
    fn a_format_with_slashes_gives_nested_folders() {
        let cfg = config("Daily", "YYYY/MM/YYYY-MM-DD");
        assert_eq!(
            cfg.file_for(date("2026-11-03")).unwrap(),
            PathBuf::from(r"C:\Notes\Vault\Daily\2026\11\2026-11-03.md")
        );
        assert_eq!(
            cfg.relative_name(date("2026-11-03")).unwrap(),
            "Daily/2026/11/2026-11-03"
        );
    }

    #[test]
    fn a_name_with_dots_keeps_them() {
        let cfg = config("", "YYYY.MM.DD");
        assert_eq!(
            cfg.file_for(date("2026-11-22")).unwrap(),
            PathBuf::from(r"C:\Notes\Vault\2026.11.22.md")
        );
    }

    #[test]
    fn a_format_that_renders_a_bad_path_part_is_refused() {
        for format in ["YYYY//DD", "[..]/DD", "[a:b]", "YYYY/", "[a\\b]"] {
            let cfg = config("", format);
            assert!(
                matches!(
                    cfg.file_for(date("2026-11-22")),
                    Err(ConfigError::BadFormat(_))
                ),
                "{format}"
            );
        }
    }

    #[test]
    fn the_link_uses_the_vault_id_and_encodes_every_byte_but_the_safe_ones() {
        let cfg = config("Dailys", "YYYY-MM-DD");
        assert_eq!(
            cfg.open_link(date("2026-11-22"), Some("a1b2c3")).unwrap(),
            "obsidian://open?vault=a1b2c3&file=Dailys%2F2026-11-22"
        );
        let nested = config("Daily", "YYYY/MM/YYYY-MM-DD");
        assert_eq!(
            nested.open_link(date("2026-11-03"), Some("id")).unwrap(),
            "obsidian://open?vault=id&file=Daily%2F2026%2F11%2F2026-11-03"
        );
    }

    #[test]
    fn without_a_vault_id_the_link_falls_back_to_the_path() {
        let cfg = config("Dailys", "YYYY-MM-DD");
        assert_eq!(
            cfg.open_link(date("2026-11-22"), None).unwrap(),
            "obsidian://open?path=C%3A%2FNotes%2FVault%2FDailys%2F2026-11-22.md"
        );
    }

    #[test]
    fn percent_encoding_handles_spaces_symbols_and_non_ascii() {
        assert_eq!(percent_encode("a b&c#d"), "a%20b%26c%23d");
        assert_eq!(percent_encode("A-z0_9.~"), "A-z0_9.~");
        assert_eq!(percent_encode("a/b"), "a%2Fb");
        assert_eq!(percent_encode("é☕"), "%C3%A9%E2%98%95");
        assert_eq!(percent_encode(""), "");
    }

    #[test]
    fn the_scope_changes_with_every_part_and_ignores_the_case_of_the_vault() {
        let base = config("Dailys", "YYYY-MM-DD").scope();
        assert_eq!(base.len(), 16);
        assert_ne!(base, config("Other", "YYYY-MM-DD").scope());
        assert_ne!(base, config("Dailys", "YYYY.MM.DD").scope());
        let other_vault =
            DailyConfig::new(Some(Path::new(r"D:\Elsewhere")), "Dailys", "YYYY-MM-DD").unwrap();
        assert_ne!(base, other_vault.scope());
        let upper =
            DailyConfig::new(Some(Path::new(r"C:\NOTES\VAULT")), "Dailys", "YYYY-MM-DD").unwrap();
        assert_eq!(base, upper.scope());
    }
}
