//! Paths and ids: the small pure functions that every other file of this module uses.
//! Section 02 6.1.
//!
//! Windows paths are not case sensitive, so two spellings of one folder must be one project.
//! `norm_path` makes one spelling that keeps its case (for showing), and `path_key` makes the
//! lower-case form that every comparison and every database key uses.

/// One spelling of a Windows path: `/` becomes `\`, a `\\?\` prefix goes, repeated `\` collapse
/// (the leading `\\` of a network path stays), the drive letter is upper case, and there is no
/// trailing `\` (except in `X:\`). The case of the rest is kept.
pub fn norm_path(raw: &str) -> String {
    let mut text = raw.trim().replace('/', "\\");
    if let Some(rest) = text.strip_prefix("\\\\?\\") {
        // `\\?\UNC\server\share` is the long form of `\\server\share`.
        text = match rest.strip_prefix("UNC\\") {
            Some(unc) => format!("\\\\{unc}"),
            None => rest.to_owned(),
        };
    }
    let network = text.starts_with("\\\\");
    let mut collapsed = String::with_capacity(text.len());
    let mut last_was_separator = false;
    for letter in text.chars() {
        if letter == '\\' {
            if last_was_separator {
                continue;
            }
            last_was_separator = true;
        } else {
            last_was_separator = false;
        }
        collapsed.push(letter);
    }
    if network {
        collapsed.insert(0, '\\');
    }
    let is_drive_root = collapsed.len() == 3 && collapsed.ends_with(":\\");
    if collapsed.len() > 1 && collapsed.ends_with('\\') && !is_drive_root {
        collapsed.pop();
    }
    let mut letters = collapsed.chars();
    match (letters.next(), letters.next()) {
        (Some(drive), Some(':')) if drive.is_ascii_alphabetic() => {
            format!("{}{}", drive.to_ascii_uppercase(), &collapsed[1..])
        }
        _ => collapsed,
    }
}

/// The key of a path: `norm_path`, lower case.
pub fn path_key(raw: &str) -> String {
    norm_path(raw).to_lowercase()
}

/// The text of a name as an id part: lower case, only `a-z0-9`, every other character (also a
/// non-ASCII one) becomes `-`, runs of `-` collapse, no `-` at either end, at most 48 characters.
/// A name with no usable letter becomes `x` and the hash of the name.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for letter in text.to_lowercase().chars() {
        let keep = letter.is_ascii_lowercase() || letter.is_ascii_digit();
        if keep {
            out.push(letter);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    let cut: String = trimmed.chars().take(48).collect();
    let cut = cut.trim_end_matches('-');
    if cut.is_empty() {
        format!("x{}", fnv8(text))
    } else {
        cut.to_owned()
    }
}

/// FNV-1a (32 bit) of the UTF-8 bytes of `text`, as 8 lower-case hex digits. A short stable
/// hash for ids; not for anything secret.
pub fn fnv8(text: &str) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in text.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{hash:08x}")
}

/// The id of a project that has none yet: `proj-<slug of the folder name>`. When that id is
/// already taken by another folder, the hash of the path key is added, so the id stays the same
/// every time the same folders are found.
pub fn project_id(name: &str, key: &str, taken: &dyn Fn(&str) -> bool) -> String {
    let plain = format!("proj-{}", slug(name));
    if taken(&plain) {
        format!("{plain}-{}", fnv8(key))
    } else {
        plain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn norm_path_gives_one_spelling() {
        let table = [
            ("c:\\dev\\alpha", "C:\\dev\\alpha"),
            ("C:/Dev/Alpha", "C:\\Dev\\Alpha"),
            ("C:\\Dev\\Alpha\\", "C:\\Dev\\Alpha"),
            ("C:\\Dev\\\\Alpha", "C:\\Dev\\Alpha"),
            ("C:\\", "C:\\"),
            ("c:/", "C:\\"),
            ("\\\\?\\C:\\Dev", "C:\\Dev"),
            ("\\\\?\\UNC\\box\\share\\p", "\\\\box\\share\\p"),
            ("\\\\box\\share\\\\p\\", "\\\\box\\share\\p"),
            ("  D:\\Work  ", "D:\\Work"),
            ("D:\\Work\\Ünï", "D:\\Work\\Ünï"),
        ];
        for (input, expected) in table {
            assert_eq!(norm_path(input), expected, "{input}");
        }
    }

    #[test]
    fn path_key_ignores_case_and_spelling() {
        assert_eq!(path_key("D:/Work/Alpha/"), path_key("d:\\work\\ALPHA"));
        assert_eq!(path_key("D:\\Work\\ÄB"), "d:\\work\\äb");
    }

    #[test]
    fn slug_keeps_letters_and_digits_only() {
        assert_eq!(slug("Web Shop 2"), "web-shop-2");
        assert_eq!(slug("--a__b--"), "a-b");
        assert_eq!(slug("rain.backend"), "rain-backend");
        assert_eq!(slug(&"a".repeat(80)).len(), 48);
    }

    #[test]
    fn a_name_without_letters_gets_the_hash() {
        let id = slug("日本語");
        assert!(id.starts_with('x') && id.len() == 9, "{id}");
        assert_eq!(id, slug("日本語"));
    }

    #[test]
    fn fnv8_matches_the_known_value() {
        // The reference value of FNV-1a 32 for "a".
        assert_eq!(fnv8("a"), "e40c292c");
        assert_eq!(fnv8(""), "811c9dc5");
    }

    #[test]
    fn a_taken_id_gets_the_hash_of_the_path() {
        let free = |_: &str| false;
        let taken = |id: &str| id == "proj-alpha";
        assert_eq!(project_id("alpha", "d:\\a\\alpha", &free), "proj-alpha");
        let id = project_id("alpha", "e:\\b\\alpha", &taken);
        assert_eq!(id, format!("proj-alpha-{}", fnv8("e:\\b\\alpha")));
    }
}
