//! Which program file starts a game folder. Section 02, 6.11.
//!
//! A game folder holds many programs: the game, its launcher, a crash reporter, redistributable
//! installers, mod tools, an art book viewer. "The largest .exe" is often wrong, so each program
//! gets a score and the best one wins:
//!
//! 1. programs that are never the game (uninstallers, crash reporters, installers) are dropped;
//! 2. the rest is scored: a name that resembles the folder name counts most, a program nearer to
//!    the top of the folder counts a little more, a 64-bit program a little more, size is only a
//!    tie-breaker, and names such as `launcher`, `editor` or `server`, or a folder called `tools`,
//!    take points away;
//! 3. ties are decided by depth, size and path, so the answer never changes between runs.
//!
//! The function is pure: it gets the list of files and returns the winner. Walking the disk is
//! `folders::walk`.

use std::collections::HashSet;

/// One `.exe` found in a game folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Path below the game folder, `\` as the separator, for example `Game\Quest.exe`.
    pub rel: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Guess {
    pub rel: String,
    pub score: f64,
}

/// Words in a program name that mean "not the game", with the points they take away. The worst
/// single match counts, not the sum.
const NAME_PENALTIES: &[(&str, f64)] = &[
    ("launcher", 50.0),
    ("updater", 60.0),
    ("patcher", 60.0),
    ("autoupdate", 60.0),
    ("config", 40.0),
    ("configurator", 40.0),
    ("settings", 40.0),
    ("options", 40.0),
    ("editor", 50.0),
    ("modtool", 50.0),
    ("modkit", 50.0),
    ("mod", 50.0),
    ("sdk", 50.0),
    ("tool", 50.0),
    ("tools", 50.0),
    ("server", 50.0),
    ("dedicated", 50.0),
    ("benchmark", 30.0),
    ("viewer", 50.0),
    ("artbook", 50.0),
    ("soundtrack", 50.0),
    ("manual", 50.0),
    ("credits", 50.0),
    ("trailer", 50.0),
    ("helper", 60.0),
    ("service", 60.0),
    ("daemon", 60.0),
    ("subprocess", 60.0),
    ("webhelper", 60.0),
    ("cef", 60.0),
    ("eac", 80.0),
    ("easyanticheat", 80.0),
    ("battleye", 80.0),
    ("beservice", 80.0),
    ("anticheat", 80.0),
    ("register", 60.0),
    ("activation", 60.0),
    ("uplay", 60.0),
    ("origin", 60.0),
];

/// Folders on the path of a program that make it less likely to be the game.
const PENALISED_FOLDERS: &[&str] = &[
    "tools",
    "tool",
    "support",
    "installers",
    "installer",
    "setup",
    "prerequisites",
    "prereq",
    "artbook",
    "soundtrack",
    "ost",
    "manual",
    "docs",
    "documentation",
    "extras",
    "bonus",
    "mods",
    "modding",
    "modkit",
    "sdk",
    "launcher",
    "crashreporter",
    "thirdparty",
    "plugins",
    "server",
    "servers",
];

/// Parts of a compact name that make a program never the game.
const HARD_DROP_PARTS: &[&str] = &[
    "uninstall",
    "uninst",
    "vcredist",
    "vcruntime",
    "dxsetup",
    "dxwebsetup",
    "oalinst",
    "crashreport",
    "crashhandler",
    "crashpad",
    "crashsender",
    "crashdump",
    "bugreport",
    "bugsplat",
    "errorreport",
    "werfault",
    "breakpad",
    "installer",
    "dotnetfx",
    "windowsdesktopruntime",
    "redist",
];
const HARD_DROP_TOKENS: &[&str] = &["setup", "install", "unins", "instmsi"];

/// Words that say nothing about which game a name belongs to.
const FILLER_TOKENS: &[&str] = &[
    "the",
    "of",
    "and",
    "edition",
    "goty",
    "deluxe",
    "definitive",
    "complete",
    "remastered",
    "game",
    "hd",
];
/// Tokens that describe the build, not the game: removed before names are compared.
const BUILD_TOKENS: &[&str] = &[
    "x64", "x86", "win64", "win32", "64", "32", "shipping", "release", "final", "retail",
];

/// The winner among `files`, or `None` when nothing is left after the hard drops. A program with
/// a bad score is still chosen when it is the only one: a wrong guess can be corrected, a
/// missing one cannot be used.
pub fn guess(folder_name: &str, files: &[FileEntry]) -> Option<Guess> {
    let folder_compact = compact(folder_name);
    let folder_tokens = word_set(&tokens(folder_name));
    let mut best: Option<(Guess, usize, u64)> = None;
    for file in files {
        let Some(candidate) = score(file, &folder_compact, &folder_tokens) else {
            continue;
        };
        let depth = file.rel.matches('\\').count();
        let wins = match &best {
            None => true,
            Some((current, current_depth, current_size)) => {
                if (candidate - current.score).abs() > 1e-9 {
                    candidate > current.score
                } else if depth != *current_depth {
                    depth < *current_depth
                } else if file.size != *current_size {
                    file.size > *current_size
                } else {
                    file.rel.to_lowercase() < current.rel.to_lowercase()
                }
            }
        };
        if wins {
            best = Some((
                Guess {
                    rel: file.rel.clone(),
                    score: candidate,
                },
                depth,
                file.size,
            ));
        }
    }
    best.map(|(guess, _, _)| guess)
}

/// The score of one program, or `None` when it is dropped.
fn score(file: &FileEntry, folder_compact: &str, folder_tokens: &HashSet<String>) -> Option<f64> {
    let mut parts: Vec<&str> = file.rel.split('\\').collect();
    let name = parts.pop().unwrap_or_default();
    let stem = name
        .rsplit_once('.')
        .map_or(name, |(stem, _)| stem)
        .to_owned();
    let stem_compact = compact(&stem);
    let stem_tokens = tokens(&stem);

    if HARD_DROP_PARTS
        .iter()
        .any(|part| stem_compact.contains(part))
        || stem_tokens
            .iter()
            .any(|token| HARD_DROP_TOKENS.contains(&token.as_str()))
    {
        return None;
    }

    let depth = parts.len();
    let build_free: Vec<String> = stem_tokens
        .iter()
        .filter(|token| !BUILD_TOKENS.contains(&token.as_str()))
        .cloned()
        .collect();
    let core = build_free.concat();

    let resemblance = resemblance(&core, &word_set(&build_free), folder_compact, folder_tokens);
    let size_mb = file.size as f64 / (1024.0 * 1024.0);
    let size_bonus = (2.0 * (size_mb + 1.0).log10()).min(6.0);

    // 64-bit and 32-bit hints come from the name and from the folders on the path.
    let hints: Vec<String> = stem_tokens
        .iter()
        .cloned()
        .chain(parts.iter().map(|folder| folder.to_lowercase()))
        .collect();
    let has = |names: &[&str]| hints.iter().any(|hint| names.contains(&hint.as_str()));
    let arch = match (
        has(&["x64", "win64", "64", "bin64"]),
        has(&["x86", "win32", "32"]),
    ) {
        (true, false) => 3.0,
        (false, true) => -3.0,
        _ => 0.0,
    };

    let name_penalty = NAME_PENALTIES
        .iter()
        .filter(|(word, _)| {
            stem_tokens.iter().any(|token| token == word)
                || stem_compact == *word
                // A longer word also counts inside a joined name (`FixtureQuestArtBook`).
                || (word.len() >= 5 && stem_compact.contains(word))
        })
        .map(|(_, points)| *points)
        .fold(0.0, f64::max);

    let penalised: HashSet<String> = parts
        .iter()
        .map(|folder| folder.to_lowercase())
        .filter(|folder| PENALISED_FOLDERS.contains(&folder.as_str()))
        .collect();
    let folder_penalty = (30.0 * penalised.len() as f64).min(60.0);

    Some(
        100.0 * resemblance - 6.0 * depth as f64 + size_bonus + arch
            - name_penalty
            - folder_penalty,
    )
}

/// 0 to 1: how much the program's name looks like the folder's name.
fn resemblance(
    core: &str,
    core_words: &HashSet<String>,
    folder_compact: &str,
    folder_words: &HashSet<String>,
) -> f64 {
    if core.is_empty() || folder_compact.is_empty() {
        return 0.0;
    }
    if core == folder_compact {
        return 1.0;
    }
    let (shorter, longer) = if core.len() <= folder_compact.len() {
        (core, folder_compact)
    } else {
        (folder_compact, core)
    };
    if shorter.len() >= 4 && longer.contains(shorter) {
        return 0.85;
    }
    let union = core_words.union(folder_words).count();
    let jaccard = if union == 0 {
        0.0
    } else {
        core_words.intersection(folder_words).count() as f64 / union as f64
    };
    let prefix = core
        .chars()
        .zip(folder_compact.chars())
        .take_while(|(a, b)| a == b)
        .count();
    let by_prefix = prefix as f64 / longer.chars().count() as f64;
    (0.8 * jaccard).max(0.7 * by_prefix)
}

/// The words of a name without the filler words.
fn word_set(tokens: &[String]) -> HashSet<String> {
    tokens
        .iter()
        .filter(|token| !FILLER_TOKENS.contains(&token.as_str()))
        .cloned()
        .collect()
}

/// Lower-case letters and digits only.
pub fn compact(text: &str) -> String {
    text.chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Splits a name into lower-case words: at every character that is not a letter or digit, at
/// every lower-to-upper change (`CrashReportClient` gives crash, report, client) and at every
/// letter-digit change (`unins000` gives unins, 000). `x64`, `x86`, `win32` and `win64` are put
/// back together afterwards, because they are one word for the build.
pub fn tokens(text: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut previous: Option<char> = None;
    for ch in text.chars() {
        if !ch.is_alphanumeric() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            previous = None;
            continue;
        }
        let split = previous.is_some_and(|before| {
            (before.is_lowercase() && ch.is_uppercase())
                || (before.is_alphabetic() && ch.is_ascii_digit())
                || (before.is_ascii_digit() && ch.is_alphabetic())
        });
        if split && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        current.extend(ch.to_lowercase());
        previous = Some(ch);
    }
    if !current.is_empty() {
        words.push(current);
    }
    merge_arch(words)
}

fn merge_arch(words: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    for word in words {
        let joins = matches!(word.as_str(), "64" | "86" | "32")
            && out.last().is_some_and(|last| {
                (last == "x" && (word == "64" || word == "86"))
                    || (last == "win" && (word == "64" || word == "32"))
            });
        if joins {
            if let Some(last) = out.last_mut() {
                last.push_str(&word);
            }
        } else {
            out.push(word);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(list: &[(&str, u64)]) -> Vec<FileEntry> {
        list.iter()
            .map(|(rel, mb)| FileEntry {
                rel: (*rel).to_owned(),
                size: mb * 1024 * 1024,
            })
            .collect()
    }

    fn winner(folder: &str, list: &[(&str, u64)]) -> Option<String> {
        guess(folder, &files(list)).map(|g| g.rel)
    }

    #[test]
    fn tokens_split_at_camel_case_digits_and_symbols() {
        assert_eq!(tokens("CrashReportClient"), ["crash", "report", "client"]);
        assert_eq!(tokens("unins000"), ["unins", "000"]);
        assert_eq!(
            tokens("Realm-Win64-Shipping"),
            ["realm", "win64", "shipping"]
        );
        assert_eq!(tokens("Game_x64"), ["game", "x64"]);
        assert_eq!(tokens("dxsetup"), ["dxsetup"]);
        assert_eq!(compact("Fixture-Quest 2!"), "fixturequest2");
    }

    #[test]
    fn the_nested_game_beats_every_kind_of_decoy() {
        let list = [
            ("Game\\FixtureQuest.exe", 180),
            ("ArtBook\\FixtureQuestArtBook.exe", 350),
            ("tools\\FixtureQuestEditor.exe", 300),
            ("tools\\FixtureQuest.exe", 320),
            ("Game\\Launcher.exe", 90),
            ("Game\\CrashReportClient.exe", 40),
            ("Game\\unins000.exe", 3),
            ("_CommonRedist\\vcredist_x64.exe", 25),
            ("_CommonRedist\\DirectX\\DXSETUP.exe", 2),
        ];
        assert_eq!(
            winner("Fixture Quest", &list).as_deref(),
            Some("Game\\FixtureQuest.exe")
        );
    }

    #[test]
    fn the_x64_program_at_depth_three_wins() {
        let list = [
            ("FA_Main\\bin\\x64\\FixtureArena.exe", 220),
            ("FA_Main\\bin\\x86\\FixtureArena.exe", 100),
            ("FA_Main\\bin\\x64\\FixtureArena_dedicated_server.exe", 400),
            ("Tools\\MapEditor.exe", 50),
            ("FA_Main\\bin\\x64\\FixtureArenaCrashHandler.exe", 5),
        ];
        assert_eq!(
            winner("Fixture Arena", &list).as_deref(),
            Some("FA_Main\\bin\\x64\\FixtureArena.exe")
        );
    }

    #[test]
    fn a_small_root_program_beats_the_big_deep_one() {
        let list = [
            ("Realm.exe", 1),
            ("Realm\\Binaries\\Win64\\Realm-Win64-Shipping.exe", 150),
        ];
        assert_eq!(winner("Fixture Realm", &list).as_deref(), Some("Realm.exe"));
    }

    #[test]
    fn nothing_resembles_the_folder_so_the_plain_program_wins() {
        let list = [
            ("bin\\run.exe", 30),
            ("bin\\tools\\unpack.exe", 5),
            ("bin\\config.exe", 2),
        ];
        assert_eq!(
            winner("Ancient Thing", &list).as_deref(),
            Some("bin\\run.exe")
        );
    }

    #[test]
    fn a_folder_with_only_a_launcher_gives_the_launcher() {
        assert_eq!(
            winner("Old Game", &[("launcher.exe", 10)]).as_deref(),
            Some("launcher.exe")
        );
    }

    #[test]
    fn the_word_crash_alone_does_not_drop_a_game() {
        let list = [("CrashBandicoot.exe", 100), ("CrashHandler.exe", 1)];
        assert_eq!(
            winner("Crash Bandicoot", &list).as_deref(),
            Some("CrashBandicoot.exe")
        );
    }

    #[test]
    fn only_a_setup_gives_none() {
        assert_eq!(
            winner("Some Game", &[("setup.exe", 10), ("unins000.exe", 1)]),
            None
        );
        assert_eq!(winner("Some Game", &[]), None);
    }

    #[test]
    fn a_64_bit_name_beats_the_same_name_without_it() {
        let list = [("Game.exe", 10), ("Game64.exe", 10)];
        assert_eq!(winner("Game", &list).as_deref(), Some("Game64.exe"));
    }

    #[test]
    fn a_release_folder_name_with_dots_still_finds_its_program() {
        let list = [("gamename.exe", 10), ("support\\redist_setup.exe", 5)];
        assert_eq!(
            winner("GameName.v1.2.3-GOG", &list).as_deref(),
            Some("gamename.exe")
        );
    }

    #[test]
    fn a_unicode_folder_name_is_compared_by_letters() {
        let list = [("Ärger.exe", 10), ("Helper.exe", 10)];
        assert_eq!(winner("Ärger", &list).as_deref(), Some("Ärger.exe"));
    }

    #[test]
    fn ties_are_decided_by_depth_then_size_then_path() {
        let a = [("b\\Same.exe", 5), ("a\\Same.exe", 5)];
        assert_eq!(winner("Same", &a).as_deref(), Some("a\\Same.exe"));
        let shallow = [("x\\y\\Same.exe", 5), ("Same.exe", 5)];
        assert_eq!(winner("Same", &shallow).as_deref(), Some("Same.exe"));
    }

    #[test]
    fn ten_thousand_files_are_scored_quickly() {
        let many: Vec<FileEntry> = (0..10_000)
            .map(|n| FileEntry {
                rel: format!("Data\\Part{n}\\Helper{n}.exe"),
                size: 1024,
            })
            .chain(std::iter::once(FileEntry {
                rel: "Quest.exe".into(),
                size: 1024,
            }))
            .collect();
        let started = std::time::Instant::now();
        assert_eq!(guess("Quest", &many).unwrap().rel, "Quest.exe");
        // The design aims at 50 ms in a release build; a debug build is several times slower.
        assert!(
            started.elapsed().as_millis() < 1500,
            "{:?}",
            started.elapsed()
        );
    }
}
