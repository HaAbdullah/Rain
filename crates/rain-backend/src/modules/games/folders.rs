//! Game folders: finding the games in them, listing their programs, measuring their size.
//! Section 02, 6.11 and 6.13. Everything here reads the disk, so call it through
//! `core::blocking::run`. The scoring of programs is in `guess.rs` (pure).

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use super::guess::FileEntry;
use super::names::{fnv8, is_inside, norm_path, path_key};

/// Programs are looked for at most this many folders deep.
pub const GUESS_MAX_DEPTH: usize = 5;
/// At most this many folders are entered per game; files are not counted, so a folder with
/// thousands of loose files cannot use the budget up.
pub const GUESS_MAX_DIRS: usize = 1500;
/// From this depth on, only folders that look like program folders are entered.
const PROGRAM_FOLDER_DEPTH: usize = 3;
/// A shell of folders with no file in it is not a game: look this deep for one file.
const EMPTY_CHECK_DEPTH: usize = 3;
/// Size measuring stops after this many entries.
pub const SIZE_MAX_ENTRIES: usize = 2_000_000;

/// Windows marks symbolic links, junctions and other reparse points with this attribute.
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

const PROGRAM_FOLDERS: &[&str] = &[
    "bin", "binaries", "win64", "x64", "x86", "game", "release", "shipping",
];

/// Folders that hold data, runtimes or redistributables and never the game's program.
const SKIPPED_FOLDERS: &[&str] = &[
    "redist",
    "redistributable",
    "redistributables",
    "directx",
    "vcredist",
    "dotnet",
    "dotnetfx",
    "engine",
    "content",
    "paks",
    "saves",
    "savegames",
    "logs",
    "crashes",
    "crashdumps",
    "shadercache",
    "movies",
    "videos",
    "sound",
    "sounds",
    "music",
    "audio",
    "textures",
    "localization",
    "locales",
    "streamingassets",
    "monobleedingedge",
    "node_modules",
    "__installer",
];

/// One game folder that was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderGame {
    /// The folder name, which is the title.
    pub name: String,
    /// The folder, normalised.
    pub path: PathBuf,
}

#[derive(Debug, Default)]
pub struct Discovery {
    pub games: Vec<FolderGame>,
    /// The game folders of the settings that do not exist or cannot be read.
    pub missing: Vec<PathBuf>,
}

/// The games in the game folders: every direct subfolder is one game. `exclude` holds folders
/// that other sources list themselves (the `steamapps` folders of Steam and the install folders
/// of Epic): a subfolder that is or lies inside one of them is left out.
pub fn discover(game_folders: &[PathBuf], exclude: &[PathBuf]) -> Discovery {
    let mut found = Discovery::default();
    let mut seen = HashSet::new();
    for folder in game_folders {
        let Ok(entries) = fs::read_dir(folder) else {
            found.missing.push(folder.clone());
            continue;
        };
        let mut names: Vec<(String, PathBuf)> = entries
            .flatten()
            .filter_map(|entry| Some((entry.file_name().into_string().ok()?, entry.path())))
            .collect();
        names.sort_by_key(|(name, _)| name.to_lowercase());
        for (name, path) in names {
            // `metadata` follows links, so a link to a folder counts as a folder.
            if !fs::metadata(&path).is_ok_and(|meta| meta.is_dir()) {
                continue; // a loose file
            }
            let lower = name.to_lowercase();
            if name.starts_with(['_', '.', '$']) || lower == "system volume information" {
                continue;
            }
            let path = PathBuf::from(norm_path(&path.to_string_lossy()));
            if exclude
                .iter()
                .any(|other| is_inside(&path.to_string_lossy(), &other.to_string_lossy()))
            {
                continue;
            }
            if !has_file_within(&path, EMPTY_CHECK_DEPTH) {
                continue; // an empty shell
            }
            if seen.insert(path_key(&path.to_string_lossy())) {
                found.games.push(FolderGame { name, path });
            }
        }
    }
    found
}

/// Whether the folder holds at least one file within `depth` levels of folders.
fn has_file_within(folder: &Path, depth: usize) -> bool {
    let Ok(entries) = fs::read_dir(folder) else {
        return false;
    };
    let mut subfolders = Vec::new();
    for entry in entries.flatten() {
        match entry.file_type() {
            Ok(kind) if kind.is_file() => return true,
            Ok(kind) if kind.is_dir() => subfolders.push(entry.path()),
            _ => {}
        }
    }
    depth > 0 && subfolders.iter().any(|sub| has_file_within(sub, depth - 1))
}

// ---- the programs of a game folder ----

/// Every `.exe` of a game folder with its size, found breadth first within the limits above.
pub fn walk(folder: &Path) -> Vec<FileEntry> {
    let mut files = Vec::new();
    // (folder, relative path, depth). Program folders go to the front of the queue.
    let mut queue: VecDeque<(PathBuf, String, usize)> = VecDeque::new();
    queue.push_back((folder.to_path_buf(), String::new(), 0));
    let mut entered = 0usize;
    while let Some((dir, rel, depth)) = queue.pop_front() {
        if entered >= GUESS_MAX_DIRS {
            break;
        }
        entered += 1;
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut sorted: Vec<_> = entries.flatten().collect();
        sorted.sort_by_key(|entry| entry.file_name().to_string_lossy().to_lowercase());
        for entry in sorted {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                continue;
            }
            let child_rel = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}\\{name}")
            };
            if meta.is_file() {
                if Path::new(&name)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
                {
                    files.push(FileEntry {
                        rel: child_rel,
                        size: meta.len(),
                    });
                }
            } else if meta.is_dir() && depth < GUESS_MAX_DEPTH {
                let lower = name.to_lowercase();
                let program_folder = PROGRAM_FOLDERS.contains(&lower.as_str());
                let skipped = lower.starts_with(['_', '.'])
                    || SKIPPED_FOLDERS.contains(&lower.as_str())
                    || lower.ends_with("_data")
                    || lower.ends_with("_backupthisfolder_butdontshipit");
                if skipped || (depth + 1 >= PROGRAM_FOLDER_DEPTH && !program_folder) {
                    continue;
                }
                let next = (entry.path(), child_rel, depth + 1);
                if program_folder {
                    queue.push_front(next);
                } else {
                    queue.push_back(next);
                }
            }
        }
    }
    files
}

/// Changes when the folder changes at its top level: the guess is kept while this stays equal.
/// It uses the folder's own modification time, the number of direct entries and the names and
/// times of up to 50 direct subfolders.
pub fn signature(folder: &Path) -> String {
    let seconds = |path: &Path| {
        fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_secs())
    };
    let mut count = 0usize;
    let mut subfolders: Vec<(String, u64)> = Vec::new();
    if let Ok(entries) = fs::read_dir(folder) {
        for entry in entries.flatten() {
            count += 1;
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                subfolders.push((
                    entry.file_name().to_string_lossy().into_owned(),
                    seconds(&entry.path()),
                ));
            }
        }
    }
    subfolders.sort();
    subfolders.truncate(50);
    let mut text = format!("{}|{count}", seconds(folder));
    for (name, time) in subfolders {
        text.push_str(&format!("|{name}:{time}"));
    }
    fnv8(&text)
}

// ---- size ----

/// The logical size of a folder: the sum of its file lengths. Reparse points are not followed and
/// unreadable subfolders are skipped. `None` when `cancel` was raised. Stops adding after
/// `SIZE_MAX_ENTRIES` entries (a size that is a lower bound is better than a hang).
pub fn measure(folder: &Path, cancel: &AtomicBool) -> Option<u64> {
    let mut total = 0u64;
    let mut seen = 0usize;
    let mut pending = vec![folder.to_path_buf()];
    while let Some(dir) = pending.pop() {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > SIZE_MAX_ENTRIES {
                return Some(total);
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                continue;
            }
            if meta.is_dir() {
                pending.push(entry.path());
            } else {
                total += meta.len();
            }
        }
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path, size: u64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = fs::File::create(path).unwrap();
        file.set_len(size).unwrap();
    }

    fn names(found: &Discovery) -> Vec<&str> {
        found.games.iter().map(|g| g.name.as_str()).collect()
    }

    #[test]
    fn every_direct_subfolder_with_a_file_is_a_game() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Games");
        touch(&root.join("Fixture Quest").join("Game").join("q.exe"), 10);
        touch(&root.join("Fixture Arena").join("a.exe"), 10);
        touch(&root.join("_CommonRedist").join("vcredist.exe"), 10);
        touch(&root.join(".hidden").join("x.exe"), 10);
        touch(&root.join("readme.txt"), 1);
        fs::create_dir_all(root.join("Empty Folder").join("also").join("empty")).unwrap();
        touch(
            &root
                .join("Deep Shell")
                .join("a")
                .join("b")
                .join("c")
                .join("d")
                .join("x.exe"),
            1,
        );
        touch(&root.join("Épopée").join("e.exe"), 1);

        let found = discover(&[root.clone(), temp.path().join("Nowhere")], &[]);
        assert_eq!(names(&found), ["Fixture Arena", "Fixture Quest", "Épopée"]);
        assert_eq!(found.missing, [temp.path().join("Nowhere")]);
    }

    #[test]
    fn a_folder_that_other_sources_list_is_left_out_and_repeats_are_dropped() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Games");
        touch(&root.join("Listed By Epic").join("x.exe"), 1);
        touch(&root.join("Mine").join("x.exe"), 1);
        let epic = root.join("Listed By Epic");
        let found = discover(&[root.clone(), root.clone()], &[epic]);
        assert_eq!(names(&found), ["Mine"]);

        let library = temp.path().join("Games2");
        touch(
            &library
                .join("steamapps")
                .join("common")
                .join("X")
                .join("x.exe"),
            1,
        );
        let found = discover(std::slice::from_ref(&library), &[library.join("steamapps")]);
        assert!(found.games.is_empty(), "steamapps is Steam's own folder");
    }

    #[test]
    fn the_walk_sees_exe_files_only_within_the_depth_limit() {
        let temp = tempfile::tempdir().unwrap();
        let game = temp.path().join("G");
        touch(&game.join("a.exe"), 5);
        touch(&game.join("notes.txt"), 5);
        touch(&game.join("l1").join("l2").join("b.exe"), 5);
        // depth 5 is the last one seen; program folders are the only ones entered from depth 3
        touch(
            &game
                .join("l1")
                .join("l2")
                .join("bin")
                .join("x64")
                .join("win64")
                .join("deep5.exe"),
            5,
        );
        touch(
            &game
                .join("l1")
                .join("l2")
                .join("bin")
                .join("x64")
                .join("win64")
                .join("release")
                .join("deep6.exe"),
            5,
        );
        let found: Vec<String> = walk(&game).into_iter().map(|f| f.rel).collect();
        assert!(found.contains(&"a.exe".to_owned()));
        assert!(found.contains(&"l1\\l2\\b.exe".to_owned()));
        assert!(
            found.contains(&"l1\\l2\\bin\\x64\\win64\\deep5.exe".to_owned()),
            "{found:?}"
        );
        assert!(!found.iter().any(|f| f.ends_with("deep6.exe")), "{found:?}");
        assert!(!found.iter().any(|f| f.ends_with("notes.txt")));
    }

    #[test]
    fn skipped_folders_are_not_entered_and_other_deep_folders_are_not_either() {
        let temp = tempfile::tempdir().unwrap();
        let game = temp.path().join("G");
        touch(&game.join("Engine").join("e.exe"), 1);
        touch(&game.join("Redist").join("r.exe"), 1);
        touch(&game.join("Game_Data").join("u.exe"), 1);
        touch(&game.join("_hidden").join("h.exe"), 1);
        touch(
            &game.join("a").join("b").join("randomname").join("c.exe"),
            1,
        );
        touch(&game.join("a").join("b").join("bin").join("ok.exe"), 1);
        let found: Vec<String> = walk(&game).into_iter().map(|f| f.rel).collect();
        assert_eq!(found, ["a\\b\\bin\\ok.exe"]);
    }

    #[test]
    fn thousands_of_loose_files_do_not_hide_the_program_at_depth_three() {
        let temp = tempfile::tempdir().unwrap();
        let game = temp.path().join("Fixture Arena");
        for n in 0..3000 {
            touch(&game.join("Data").join(format!("file{n}.dat")), 1);
        }
        touch(
            &game
                .join("FA_Main")
                .join("bin")
                .join("x64")
                .join("FixtureArena.exe"),
            100,
        );
        let found = walk(&game);
        assert!(found.iter().any(|f| f.rel.ends_with("FixtureArena.exe")));
    }

    #[test]
    fn the_budget_of_folders_stops_the_walk() {
        let temp = tempfile::tempdir().unwrap();
        let game = temp.path().join("G");
        for n in 0..(GUESS_MAX_DIRS + 200) {
            fs::create_dir_all(game.join(format!("d{n:05}"))).unwrap();
        }
        touch(&game.join("zz-late").join("late.exe"), 1);
        touch(&game.join("first.exe"), 1);
        let found: Vec<String> = walk(&game).into_iter().map(|f| f.rel).collect();
        assert!(found.contains(&"first.exe".to_owned()));
        assert!(!found.iter().any(|f| f.ends_with("late.exe")));
    }

    #[test]
    fn the_signature_changes_with_the_top_level_and_the_size_adds_up() {
        let temp = tempfile::tempdir().unwrap();
        let game = temp.path().join("G");
        touch(&game.join("a.bin"), 1000);
        touch(&game.join("sub").join("b.bin"), 234);
        let before = signature(&game);
        assert_eq!(before, signature(&game));
        touch(&game.join("c.bin"), 1);
        assert_ne!(before, signature(&game));

        let cancel = AtomicBool::new(false);
        assert_eq!(measure(&game, &cancel), Some(1235));
        cancel.store(true, Ordering::Relaxed);
        assert_eq!(measure(&game, &cancel), None);
    }
}
