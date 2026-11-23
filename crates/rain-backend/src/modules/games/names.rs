//! Names, keys and ids: pure text functions with no disk access. Section 02, 6.1.
//!
//! - `norm_path` and `path_key` make two spellings of one folder compare equal.
//! - `slug` and `fnv8` build the ids (`game-steam-123`, `game-dir-some-game`).

/// Writes a path the one way this module compares and stores it: `\` as the separator, no `\\?\`
/// prefix, no repeated or trailing separator, an upper-case drive letter. The case of the rest is
/// kept. A UNC path (`\\server\share`) keeps its two leading backslashes.
pub fn norm_path(text: &str) -> String {
    let mut s = text.trim().replace('/', "\\");
    if let Some(rest) = s.strip_prefix("\\\\?\\") {
        // `\\?\UNC\server\share` is the long spelling of `\\server\share`.
        s = match rest.strip_prefix("UNC\\") {
            Some(unc) => format!("\\\\{unc}"),
            None => rest.to_owned(),
        };
    }
    let unc = s.starts_with("\\\\");
    let mut out = String::with_capacity(s.len());
    let mut last_was_separator = false;
    for ch in s.chars() {
        if ch == '\\' {
            if last_was_separator {
                continue;
            }
            last_was_separator = true;
        } else {
            last_was_separator = false;
        }
        out.push(ch);
    }
    if unc {
        out.insert(0, '\\');
    }
    // A trailing separator goes, except the one of `X:\`.
    while out.len() > 3 && out.ends_with('\\') {
        out.pop();
    }
    let bytes = out.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        let drive = out[..1].to_ascii_uppercase();
        out.replace_range(..1, &drive);
    }
    out
}

/// The text two paths are compared by: `norm_path`, lower-cased.
pub fn path_key(text: &str) -> String {
    norm_path(text).to_lowercase()
}

/// Whether `inner` is `outer` or lies below it. Both are compared by `path_key`, and only whole
/// folder names match (`D:\Games\Foo` does not contain `D:\Games\Foobar`).
pub fn is_inside(inner: &str, outer: &str) -> bool {
    let (inner, outer) = (path_key(inner), path_key(outer));
    inner == outer
        || inner
            .strip_prefix(&outer)
            .is_some_and(|rest| outer.ends_with('\\') || rest.starts_with('\\'))
}

/// FNV-1a, 32 bit, of the UTF-8 bytes, as 8 lower-case hex digits. Small, stable and without a
/// crate: it names cover files and breaks id collisions, nothing secret.
pub fn fnv8(text: &str) -> String {
    fnv8_bytes(text.as_bytes())
}

pub fn fnv8_bytes(bytes: &[u8]) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in bytes {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{hash:08x}")
}

/// Lower-case `a-z0-9` words joined by `-`: every other character, also a non-ASCII letter,
/// becomes a separator. At most 48 characters. A name with no usable character becomes
/// `x<fnv8>`, so the id is never empty.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for ch in text.to_lowercase().chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            out.push(ch);
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

/// `base`, or `base-2`, `base-3` ... until `taken` says the name is free.
pub fn unique_id(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|candidate| !taken(candidate))
        .expect("an unbounded counter finds a free name")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_written_one_way() {
        let cases = [
            ("d:\\games\\foo", "D:\\games\\foo"),
            ("D:/Games/Foo", "D:\\Games\\Foo"),
            ("D:\\Games\\Foo\\", "D:\\Games\\Foo"),
            ("D:\\\\Games\\\\Foo", "D:\\Games\\Foo"),
            ("D:\\", "D:\\"),
            ("D:/", "D:\\"),
            ("\\\\?\\D:\\Games", "D:\\Games"),
            ("\\\\server\\share\\Games\\", "\\\\server\\share\\Games"),
            ("\\\\?\\UNC\\server\\share", "\\\\server\\share"),
            ("  D:\\Games  ", "D:\\Games"),
            ("relative\\path", "relative\\path"),
        ];
        for (input, expected) in cases {
            assert_eq!(norm_path(input), expected, "{input}");
        }
    }

    #[test]
    fn the_key_ignores_case_including_non_ascii_letters() {
        assert_eq!(path_key("D:\\Games\\ÄRGER"), path_key("d:/games/ärger/"));
        assert_ne!(path_key("D:\\a"), path_key("D:\\b"));
    }

    #[test]
    fn inside_means_a_whole_folder_name() {
        assert!(is_inside("D:\\Games\\Foo\\bin\\x.exe", "d:\\games\\foo"));
        assert!(is_inside("D:\\Games\\Foo", "D:\\Games\\Foo\\"));
        assert!(!is_inside("D:\\Games\\Foobar\\x.exe", "D:\\Games\\Foo"));
        assert!(is_inside("D:\\Games\\x.exe", "D:\\"));
    }

    #[test]
    fn fnv8_is_stable() {
        assert_eq!(fnv8(""), "811c9dc5");
        assert_eq!(fnv8("a"), "e40c292c");
        assert_eq!(fnv8("a").len(), 8);
    }

    #[test]
    fn slugs_keep_ascii_words_and_replace_the_rest() {
        assert_eq!(slug("Fixture Quest"), "fixture-quest");
        assert_eq!(slug("  --Some  Game: Remastered!"), "some-game-remastered");
        assert_eq!(slug("Game.v1.2.3-GOG"), "game-v1-2-3-gog");
        assert_eq!(slug("日本語"), format!("x{}", fnv8("日本語")));
        assert_eq!(slug(&"a".repeat(100)).len(), 48);
        assert!(!slug("ab-----").ends_with('-'));
    }

    #[test]
    fn a_taken_id_gets_a_counter() {
        let taken = ["game-a", "game-a-2"];
        assert_eq!(unique_id("game-b", |s| taken.contains(&s)), "game-b");
        assert_eq!(unique_id("game-a", |s| taken.contains(&s)), "game-a-3");
    }
}
