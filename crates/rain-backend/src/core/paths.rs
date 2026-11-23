//! The one function that turns a path that somebody typed into a clean absolute path.
//! Foundation section 11.2; the rules are written in section 05 6.9.
//!
//! The settings module builds `Resolved` with it and the disk module answers `POST /paths/check`
//! and `POST /shell/reveal` with it, so that the glyph in Settings and the value that the
//! projects and games modules use always agree. No second normaliser exists.
//!
//! It is pure: it never looks at the disk and never expands `%NAME%` (the settings module does
//! that first, with `Service::expand_env`).

use std::path::PathBuf;

use rain_types::limits::PATH_MAX_CHARS;

/// Why a text is not a usable absolute path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathProblem {
    Empty,
    TooLong,
    /// A control character, one of `< > " | * ?`, a colon that is not the one after the drive
    /// letter (`C::stream` is an NTFS alternate data stream), or a segment that ends in `.` or
    /// a space (Windows silently drops those, so `a.` would name `a`).
    BadChar,
    /// Not `C:\...`: a relative path, `C:foo`, `C:` or a name without a drive.
    NotAbsolute,
    /// A `..` segment. It is not resolved: a typed `..` is a mistake or an attack.
    Dotdot,
    /// `\\?\` and `\\.\` paths, and a network path where `allow_unc` is false.
    NotLocal,
}

/// Characters that Windows does not allow in a path (the colon is checked by `has_stray_colon`).
const FORBIDDEN: [char; 6] = ['<', '>', '"', '|', '*', '?'];

/// Rules 1 to 8 of section 05 6.9, in this order. A path that starts with `\\` (a network share)
/// is kept as `\\server\share\...` when `allow_unc` is true.
pub fn normalize(raw: &str, allow_unc: bool) -> Result<PathBuf, PathProblem> {
    let text = strip_quotes(raw.trim());
    if text.is_empty() {
        return Err(PathProblem::Empty);
    }
    if text.chars().count() > PATH_MAX_CHARS {
        return Err(PathProblem::TooLong);
    }
    let text = text.replace('/', "\\");
    // `\\?\` contains a `?`, which rule 3 would call a bad character. "Not a local path" is the
    // answer that tells the user more, so the device prefixes are looked at first.
    if text.starts_with(r"\\?\") || text.starts_with(r"\\.\") {
        return Err(PathProblem::NotLocal);
    }
    if text
        .chars()
        .any(|c| c.is_control() || FORBIDDEN.contains(&c))
        || has_stray_colon(&text)
    {
        return Err(PathProblem::BadChar);
    }
    if let Some(share) = text.strip_prefix(r"\\") {
        return if allow_unc {
            join_segments(r"\\", share)
        } else {
            Err(PathProblem::NotLocal)
        };
    }
    let bytes = text.as_bytes();
    let has_drive =
        bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\';
    if !has_drive {
        return Err(PathProblem::NotAbsolute);
    }
    let drive = format!("{}:\\", text[..1].to_ascii_uppercase());
    join_segments(&drive, &text[3..])
}

/// A colon anywhere except at index 1 (right after a drive letter).
fn has_stray_colon(text: &str) -> bool {
    text.char_indices().any(|(i, c)| c == ':' && i != 1)
}

/// Removes one pair of surrounding double quotes, as a path copied from Explorer has them.
fn strip_quotes(text: &str) -> &str {
    text.strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .map_or(text, str::trim)
}

/// Rules 7 and 8: drops empty and `.` segments, refuses `..`, and joins without a trailing `\`
/// (a drive root `C:\` keeps its backslash).
fn join_segments(prefix: &str, rest: &str) -> Result<PathBuf, PathProblem> {
    let mut kept = Vec::new();
    for segment in rest.split('\\') {
        match segment {
            "" | "." => {}
            ".." => return Err(PathProblem::Dotdot),
            other if other.ends_with(['.', ' ']) => return Err(PathProblem::BadChar),
            other => kept.push(other),
        }
    }
    Ok(PathBuf::from(format!("{prefix}{}", kept.join("\\"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(raw: &str) -> String {
        normalize(raw, true).unwrap().to_string_lossy().into_owned()
    }

    #[test]
    fn a_clean_path_is_kept_and_its_drive_letter_is_upper_case() {
        assert_eq!(ok(r"c:\Games\Fake"), r"C:\Games\Fake");
        assert_eq!(ok(r"D:\Code"), r"D:\Code");
    }

    #[test]
    fn slashes_become_backslashes_and_the_trailing_one_goes() {
        assert_eq!(ok("D:/Code/Rain/"), r"D:\Code\Rain");
        assert_eq!(ok(r"D:\Code\\Rain\\"), r"D:\Code\Rain");
    }

    #[test]
    fn a_drive_root_keeps_its_backslash() {
        assert_eq!(ok("D:\\"), "D:\\");
        assert_eq!(ok("d:/"), "D:\\");
    }

    #[test]
    fn surrounding_quotes_and_spaces_are_removed() {
        assert_eq!(ok(r#"  "C:\Fake\Library"  "#), r"C:\Fake\Library");
        assert_eq!(ok(r#""  C:\Fake  ""#), r"C:\Fake");
    }

    #[test]
    fn dot_segments_are_dropped_and_dotdot_is_refused() {
        assert_eq!(ok(r"C:\a\.\b"), r"C:\a\b");
        assert_eq!(normalize(r"C:\a\..\b", true), Err(PathProblem::Dotdot));
        assert_eq!(normalize(r"C:\a\..", true), Err(PathProblem::Dotdot));
    }

    #[test]
    fn each_problem_has_its_own_answer() {
        let cases = [
            ("", PathProblem::Empty),
            ("   ", PathProblem::Empty),
            ("\"\"", PathProblem::Empty),
            (r"C:\bad|name", PathProblem::BadChar),
            (r"C:\wild*card", PathProblem::BadChar),
            (r"C:\what?", PathProblem::BadChar),
            ("C:\\tab\there", PathProblem::BadChar),
            (r#"C:\inner"quote"#, PathProblem::BadChar),
            (r"C:\Games:stream", PathProblem::BadChar),
            (r"C::b", PathProblem::BadChar),
            (r"\server\sh:are", PathProblem::BadChar),
            (r"C:.", PathProblem::BadChar),
            (r"C: ", PathProblem::BadChar),
            (r"C:...", PathProblem::BadChar),
            (r"\\?\C:\x", PathProblem::NotLocal),
            (r"\\.\pipe\x", PathProblem::NotLocal),
            ("C:foo", PathProblem::NotAbsolute),
            ("C:", PathProblem::NotAbsolute),
            (r"foo\bar", PathProblem::NotAbsolute),
            (r"\rooted", PathProblem::NotAbsolute),
            ("~/Games", PathProblem::NotAbsolute),
        ];
        for (raw, problem) in cases {
            assert_eq!(normalize(raw, true), Err(problem), "{raw:?}");
        }
    }

    #[test]
    fn a_path_of_1024_characters_is_allowed_and_one_more_is_too_long() {
        let name = |n: usize| format!(r"C:\{}", "x".repeat(n - 3));
        assert!(normalize(&name(1024), true).is_ok());
        assert_eq!(normalize(&name(1025), true), Err(PathProblem::TooLong));
    }

    #[test]
    fn a_network_path_depends_on_allow_unc() {
        assert_eq!(ok(r"\\server\share\Games\"), r"\\server\share\Games");
        assert_eq!(
            normalize(r"\\server\share", false),
            Err(PathProblem::NotLocal)
        );
        assert_eq!(ok("//server/share"), r"\\server\share");
    }

    #[test]
    fn the_same_folder_in_two_spellings_normalises_alike() {
        assert_eq!(
            ok("d:/Games/").to_lowercase(),
            ok(r"D:\games").to_lowercase()
        );
    }

    #[test]
    fn an_environment_variable_is_not_expanded_here() {
        assert_eq!(
            normalize(r"%USERPROFILE%\Games", true),
            Err(PathProblem::NotAbsolute)
        );
    }
}
