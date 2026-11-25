//! Names, keys and ids: pure text functions with no disk access. Section 02, 6.1.
//!
//! - `norm_path` and `path_key` make two spellings of one folder compare equal.
//! - `slug` and `fnv8` build the ids (`game-steam-123`, `game-dir-some-game`).

pub use crate::modules::projects::ids::{fnv8, norm_path, path_key, slug};

/// Whether `inner` is `outer` or lies below it. Both are compared by `path_key`, and only whole
/// folder names match (`D:\Games\Foo` does not contain `D:\Games\Foobar`).
pub fn is_inside(inner: &str, outer: &str) -> bool {
    let (inner, outer) = (path_key(inner), path_key(outer));
    inner == outer
        || inner
            .strip_prefix(&outer)
            .is_some_and(|rest| outer.ends_with('\\') || rest.starts_with('\\'))
}

/// FNV-1a, 32 bit, as 8 lower-case hex digits. Small, stable and without a crate: it names cover
/// files, nothing secret.
pub fn fnv8_bytes(bytes: &[u8]) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in bytes {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{hash:08x}")
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
    fn inside_means_a_whole_folder_name() {
        assert!(is_inside("D:\\Games\\Foo\\bin\\x.exe", "d:\\games\\foo"));
        assert!(is_inside("D:\\Games\\Foo", "D:\\Games\\Foo\\"));
        assert!(!is_inside("D:\\Games\\Foobar\\x.exe", "D:\\Games\\Foo"));
        assert!(is_inside("D:\\Games\\x.exe", "D:\\"));
    }

    #[test]
    fn a_taken_id_gets_a_counter() {
        let taken = ["game-a", "game-a-2"];
        assert_eq!(unique_id("game-b", |s| taken.contains(&s)), "game-b");
        assert_eq!(unique_id("game-a", |s| taken.contains(&s)), "game-a-3");
    }
}
