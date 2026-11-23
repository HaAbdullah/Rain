//! The rules for the text values of `settings.toml`. The same checks run for a `PATCH` and for a
//! file that the user edited by hand (section 06, table V and section 4.1).
//!
//! Pure functions: no disk, no state.

/// Longest text of a path setting, and of one entry of a folder list.
const MAX_TEXT_CHARS: usize = 1024;
/// Longest daily-note date format.
const MAX_FORMAT_CHARS: usize = 64;
/// Longest daily-note folder (it is a folder inside the vault).
const MAX_NOTE_FOLDER_CHARS: usize = 128;
/// Most entries of a folder list.
pub const MAX_FOLDERS: usize = 32;
/// Characters that Windows does not allow in a file name. A date format becomes a file name;
/// `/` is allowed because it makes sub-folders, as in Obsidian.
const FORBIDDEN_IN_FORMAT: [char; 7] = ['<', '>', ':', '"', '|', '?', '*'];

/// What is wrong with a value. Each case has its reason tag and its sentence (section 06 8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reject {
    BadText,
    TooLong,
    BadFormat,
    BadFolder,
    TooMany,
}

impl Reject {
    pub fn reason(self) -> &'static str {
        match self {
            Self::BadText => "badText",
            Self::TooLong => "tooLong",
            Self::BadFormat => "badFormat",
            Self::BadFolder => "badFolder",
            Self::TooMany => "tooMany",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::BadText => "That text has characters Rain cannot use.",
            Self::TooLong => "That text is too long.",
            Self::BadFormat => "Not a valid date format",
            Self::BadFolder => "Use a folder inside the vault, like Daily",
            Self::TooMany => "Too many folders (at most 32).",
        }
    }
}

fn has_control_character(text: &str) -> bool {
    text.chars().any(char::is_control)
}

/// A path or a folder name: no control characters, at most 1024 characters. Whether the folder
/// exists is not checked: the user may type one that does not exist yet.
pub fn path_text(text: &str) -> Result<(), Reject> {
    if has_control_character(text) {
        return Err(Reject::BadText);
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(Reject::TooLong);
    }
    Ok(())
}

/// A date format for the names of daily notes.
pub fn note_format(text: &str) -> Result<(), Reject> {
    if has_control_character(text) {
        return Err(Reject::BadText);
    }
    let allowed = text.chars().count() <= MAX_FORMAT_CHARS
        && !text.contains('\\')
        && !text.contains(FORBIDDEN_IN_FORMAT);
    if allowed {
        Ok(())
    } else {
        Err(Reject::BadFormat)
    }
}

/// A folder inside the vault: relative (no drive, not rooted, no `..`), at most 128 characters.
/// Returns it as it is stored: `/` as the separator and without a `/` at either end. An empty
/// text is allowed and means "no override".
pub fn note_folder(text: &str) -> Result<String, Reject> {
    if has_control_character(text) {
        return Err(Reject::BadText);
    }
    let text = text.trim().replace('\\', "/");
    let stored = text.trim_matches('/');
    let has_drive = stored.as_bytes().get(1) == Some(&b':');
    let is_network_path = text.starts_with("//");
    let goes_up = stored.split('/').any(|segment| segment == "..");
    let ok = stored.chars().count() <= MAX_NOTE_FOLDER_CHARS
        && !has_drive
        && !is_network_path
        && !goes_up;
    if ok {
        Ok(stored.to_owned())
    } else {
        Err(Reject::BadFolder)
    }
}

/// A list of folders: not too long, and every entry a valid path text.
pub fn folder_list(entries: &[String]) -> Result<(), Reject> {
    if entries.len() > MAX_FOLDERS {
        return Err(Reject::TooMany);
    }
    entries.iter().try_for_each(|entry| path_text(entry))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_may_not_hold_control_characters_or_be_too_long() {
        assert_eq!(path_text(r"D:\My Games\Steam"), Ok(()));
        assert_eq!(path_text(""), Ok(()));
        assert_eq!(path_text("a\nb"), Err(Reject::BadText));
        assert_eq!(path_text("a\u{7f}b"), Err(Reject::BadText));
        assert_eq!(path_text(&"x".repeat(1024)), Ok(()));
        assert_eq!(path_text(&"x".repeat(1025)), Err(Reject::TooLong));
    }

    #[test]
    fn the_length_of_a_path_counts_characters_not_bytes() {
        assert_eq!(path_text(&"é".repeat(1024)), Ok(()));
    }

    #[test]
    fn a_date_format_may_have_slashes_but_not_the_characters_windows_refuses() {
        assert_eq!(note_format("YYYY-MM-DD"), Ok(()));
        assert_eq!(note_format("YYYY/MM/YYYY-MM-DD"), Ok(()));
        for bad in ["a<b", "a>b", "a:b", "a\"b", "a|b", "a?b", "a*b", r"a\b"] {
            assert_eq!(note_format(bad), Err(Reject::BadFormat), "{bad}");
        }
        assert_eq!(note_format("a\tb"), Err(Reject::BadText));
        assert_eq!(note_format(&"Y".repeat(64)), Ok(()));
        assert_eq!(note_format(&"Y".repeat(65)), Err(Reject::BadFormat));
    }

    #[test]
    fn a_note_folder_is_relative_and_stored_with_forward_slashes() {
        assert_eq!(note_folder("Daily").as_deref(), Ok("Daily"));
        assert_eq!(note_folder("").as_deref(), Ok(""));
        assert_eq!(
            note_folder(" /Journal/2026/ ").as_deref(),
            Ok("Journal/2026")
        );
        assert_eq!(
            note_folder("Journal\\Daily").as_deref(),
            Ok("Journal/Daily")
        );
        assert_eq!(note_folder(&"x".repeat(128)).map(|f| f.len()), Ok(128));
    }

    #[test]
    fn a_note_folder_may_not_leave_the_vault_or_be_absolute() {
        for bad in [
            "C:\\Notes",
            "C:/Notes",
            "..",
            "a/../b",
            "//server/share",
            &"x".repeat(129),
        ] {
            assert_eq!(note_folder(bad), Err(Reject::BadFolder), "{bad}");
        }
        assert_eq!(note_folder("a\nb"), Err(Reject::BadText));
        // `..` inside a name is fine: only a whole segment goes up.
        assert!(note_folder("my..notes").is_ok());
    }

    #[test]
    fn a_folder_list_has_at_most_32_valid_entries() {
        let entries = |n: usize| vec![r"D:\Code".to_owned(); n];
        assert_eq!(folder_list(&entries(32)), Ok(()));
        assert_eq!(folder_list(&entries(33)), Err(Reject::TooMany));
        assert_eq!(
            folder_list(&["ok".into(), "a\nb".into()]),
            Err(Reject::BadText)
        );
    }

    #[test]
    fn every_rejection_has_a_reason_and_a_sentence() {
        for reject in [
            Reject::BadText,
            Reject::TooLong,
            Reject::BadFormat,
            Reject::BadFolder,
            Reject::TooMany,
        ] {
            assert!(!reject.reason().is_empty() && !reject.message().is_empty());
        }
    }
}
