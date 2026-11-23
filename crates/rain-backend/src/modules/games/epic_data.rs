//! The new Epic Games Launcher ("eglv2"): its data folder, read-only. Section 02, 6.10.1.
//!
//! The new launcher keeps no `.item` manifests. Everything lives in one data folder per user:
//! - `installations.jsonl`: a journal of the installed games, one JSON object per line. The same
//!   installation appears many times (the file is appended to while a game installs or updates);
//!   the LAST line of an `installationId` is its current state. The first line is a header.
//! - `cache\prod\account\<account>\library-cache.json`: the titles.
//! - `cache\prod\catalog-cache.json` and `images\<md5>.jpg`: the cover pictures the launcher
//!   already downloaded. Rain never downloads one.
//! - `cache\disk-size-cache.json`, `cache\prod\account\<account>\playtime-cache.json` and
//!   `last-played.json`: size, play time and last played.
//!
//! Every file is read as bytes with a size limit, parsed as plain JSON values (not typed structs:
//! the launcher may add keys at any time) and read leniently. A cache that is missing or does not
//! fit only means "no title / cover / size", never a failed source. Only the journal itself can
//! make the source `failed`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use rain_types::GameSource;
use serde_json::Value;
use tracing::{debug, warn};

use super::covers::COVER_MAX;
use super::epic::{ReadProblem, read_capped};
use super::model::{EpicCatalog, Found, Launch};
use super::names::slug;
use crate::core::paths::normalize;

/// The journal file, which also tells the new launcher's folder from the old one.
pub const JOURNAL_FILE: &str = crate::modules::settings::EPIC_JOURNAL_FILE;
/// The journal is a few KB on a real PC; this is far above any real one.
const JOURNAL_MAX: u64 = 4 * 1024 * 1024;
/// Lines read from the journal (the header included).
const RECORDS_MAX: usize = 20_000;
/// Every cache file (the catalog cache is about 230 KB on a real PC).
const CACHE_MAX: u64 = 8 * 1024 * 1024;
/// Signed-in accounts that are looked at (a PC has one or two).
const ACCOUNTS_MAX: usize = 16;

/// One installed game of the journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub namespace: String,
    pub item_id: String,
    /// The app name: the one name both launcher formats share.
    pub artifact_id: String,
    pub dir: PathBuf,
}

/// What the journal says.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Journal {
    pub games: Vec<Installed>,
    /// Lines that were not usable JSON records. They are skipped.
    pub bad_lines: usize,
    /// The journal had more lines than `RECORDS_MAX`; the rest was not read.
    pub truncated: bool,
}

/// Why the journal text could not be used at all (a file that cannot be read or is too large
/// never gets this far: `read` says so with its own sentence).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalError {
    /// There were lines, and not one of them was JSON.
    Garbage,
}

/// A name that goes into a launch address and into `games.toml`: plain characters only.
pub fn is_plain_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 128
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

/// The installed games of the journal text. Pure: `dir_exists` is the one look at the disk.
///
/// The header (first line) is skipped. A line that is not a JSON object, or has no
/// `installationId`, is skipped and counted. The last record of each `installationId` decides.
/// A game is listed when `complete` is `true`, the platform is Windows or absent, the ids are
/// plain and the folder `dir` exists; a record whose folder is gone is "not installed", the same
/// way a stale manifest of the old launcher is.
pub fn parse_journal(
    text: &str,
    dir_exists: &dyn Fn(&Path) -> bool,
) -> Result<Journal, JournalError> {
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let (mut bad_lines, mut records) = (0usize, 0usize);

    // installationId -> its last record. A later line replaces an earlier one completely.
    let mut last: BTreeMap<String, Value> = BTreeMap::new();
    for (number, line) in lines.by_ref().take(RECORDS_MAX).enumerate() {
        let parsed = serde_json::from_str::<Value>(line).ok();
        let Some(record) = parsed.filter(Value::is_object) else {
            bad_lines += 1;
            continue;
        };
        match record
            .get("installationId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 128)
        {
            Some(id) => {
                records += 1;
                last.insert(id.to_owned(), record);
            }
            // The first line is the header (`version`, `writtenAt`, `entries`): a JSON object
            // without an `installationId`. What it says is not used. A first line that is a
            // record is taken as one, so a missing header loses no game.
            None if number == 0 => {}
            None => bad_lines += 1,
        }
    }
    let truncated = lines.next().is_some();
    // Lines that are not records and not one record: the format has changed or the file is junk.
    if bad_lines > 0 && records == 0 {
        return Err(JournalError::Garbage);
    }

    // One game per app name, whatever the installations: the one with the larger installation id replaces the other (the order is only
    // fixed, nothing in the journal says which is newer).
    let mut games: BTreeMap<String, Installed> = BTreeMap::new();
    for record in last.values() {
        if let Some(game) = installed_of(record, dir_exists) {
            games.insert(game.artifact_id.clone(), game);
        }
    }
    Ok(Journal {
        games: games.into_values().collect(),
        bad_lines,
        truncated,
    })
}

fn installed_of(record: &Value, dir_exists: &dyn Fn(&Path) -> bool) -> Option<Installed> {
    if record.get("complete").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    // `platform` is "Windows" here; when it is absent the record is taken (the launcher of
    // Windows wrote the file), when it is something else (a Mac entry) it is not.
    match record.get("platform") {
        None | Some(Value::Null) => {}
        Some(Value::String(platform)) if platform.eq_ignore_ascii_case("windows") => {}
        Some(_) => return None,
    }
    let text = |key: &str| {
        record
            .get(key)
            .and_then(Value::as_str)
            .filter(|t| is_plain_id(t))
    };
    // The folder comes from a file, so it must be a plain local absolute path: `normalize`
    // refuses a relative path, a network share (`\\server\share`, which `is_dir` would try to
    // reach), a device path and odd characters.
    let dir = normalize(record.get("dir").and_then(Value::as_str)?, false).ok()?;
    if !dir_exists(&dir) {
        return None;
    }
    Some(Installed {
        namespace: text("namespace")?.to_owned(),
        item_id: text("itemId")?.to_owned(),
        artifact_id: text("artifactId")?.to_owned(),
        dir,
    })
}

// ---- the caches ----

/// The parsed cache files of one data folder, read once per scan.
#[derive(Default)]
pub struct Caches {
    /// `library-cache.json` of every account, in the order of the account folders.
    libraries: Vec<Value>,
    playtimes: Vec<Value>,
    catalog: Option<Value>,
    sizes: Option<Value>,
    last_played: Option<Value>,
    images: PathBuf,
}

impl Caches {
    /// Reads what exists. A missing, too large or broken file is left out (`debug` only).
    pub fn load(data: &Path) -> Self {
        let cache = data.join("cache");
        let accounts = account_folders(&cache.join("prod").join("account"));
        let per_account = |name: &str| -> Vec<Value> {
            accounts
                .iter()
                .filter_map(|account| read_json(&account.join(name)))
                .collect()
        };
        Self {
            libraries: per_account("library-cache.json"),
            playtimes: per_account("playtime-cache.json"),
            catalog: read_json(&cache.join("prod").join("catalog-cache.json")),
            sizes: read_json(&cache.join("disk-size-cache.json")),
            last_played: read_json(&data.join("last-played.json")),
            images: data.join("images"),
        }
    }

    /// The title of the first account that lists the game.
    pub fn title(&self, game: &Installed) -> Option<String> {
        self.libraries
            .iter()
            .find_map(|library| title_in(library, game))
    }

    /// A local cover file, or `None` (the tile then shows the title).
    pub fn cover(&self, game: &Installed) -> Option<PathBuf> {
        let images = self
            .catalog
            .as_ref()?
            .get("items")?
            .get(&game.item_id)?
            .get("item")?
            .get("keyImages")?
            .as_array()?;
        pick_cover(images, &|md5| local_image(&self.images, md5))
    }

    pub fn size_bytes(&self, game: &Installed) -> Option<u64> {
        let entries = self.sizes.as_ref()?.get("entries")?.as_array()?;
        entries.iter().find_map(|entry| {
            let id = entry.get("app_id")?;
            let same =
                |key: &str, wanted: &str| id.get(key).and_then(Value::as_str) == Some(wanted);
            if !(same("namespaceId", &game.namespace)
                && same("itemId", &game.item_id)
                && same("appName", &game.artifact_id))
            {
                return None;
            }
            // The numbers are written as floats (`37170389839.0`).
            non_negative(entry.get("bytes")?.as_f64()?)
        })
    }

    /// Epic's own play time, in whole minutes. With several accounts the largest counts: two
    /// accounts on one PC would otherwise add the same hours twice.
    pub fn playtime_minutes(&self, game: &Installed) -> u32 {
        let seconds = self
            .playtimes
            .iter()
            .filter_map(|file| {
                let item = file.get("items")?.as_array()?.iter().find(|item| {
                    item.get("artifactId").and_then(Value::as_str) == Some(&game.artifact_id)
                })?;
                non_negative(item.get("totalTime")?.as_f64()?)
            })
            .max()
            .unwrap_or(0);
        u32::try_from(seconds / 60).unwrap_or(u32::MAX)
    }

    /// The latest "last played" of any account, as Unix seconds.
    pub fn last_played_unix(&self, game: &Installed) -> Option<i64> {
        let key = format!("{}:{}:{}", game.namespace, game.item_id, game.artifact_id);
        self.last_played
            .as_ref()?
            .as_object()?
            .values()
            .filter_map(|account| {
                let text = account.get(&key)?.as_str()?;
                chrono::DateTime::parse_from_rfc3339(text)
                    .ok()
                    .map(|time| time.timestamp())
            })
            .max()
    }
}

fn non_negative(number: f64) -> Option<u64> {
    (number.is_finite() && number >= 0.0).then_some(number as u64)
}

/// The account folders, by name, at most `ACCOUNTS_MAX`.
fn account_folders(folder: &Path) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect()
        })
        .unwrap_or_default();
    folders.sort();
    folders.truncate(ACCOUNTS_MAX);
    folders
}

fn read_json(file: &Path) -> Option<Value> {
    let bytes = match read_capped(file, CACHE_MAX) {
        Ok(bytes) => bytes,
        Err(problem) => {
            debug!(file = ?file.file_name(), ?problem, "an Epic cache file was not read");
            return None;
        }
    };
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    serde_json::from_slice(bytes).ok()
}

/// The title of `game` in one `library-cache.json`: matched by namespace, item id and app name.
fn title_in(library: &Value, game: &Installed) -> Option<String> {
    let relationship = format!("{}:{}:{}", game.namespace, game.item_id, game.artifact_id);
    library
        .get("items")?
        .as_array()?
        .iter()
        .find(|item| {
            let app = item.get("appId");
            let same = |key: &str, wanted: &str| {
                app.and_then(|app| app.get(key)).and_then(Value::as_str) == Some(wanted)
            };
            (same("namespaceId", &game.namespace)
                && same("itemId", &game.item_id)
                && same("appName", &game.artifact_id))
                || item.get("relationshipId").and_then(Value::as_str) == Some(relationship.as_str())
        })
        .and_then(|item| item.get("title")?.as_str())
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_owned)
}

/// The local file of a key image: `<images>\<md5>.<ext>`, at most `COVER_MAX`. The name is a
/// hash from a file, so it is checked to be hex before it becomes part of a path.
fn local_image(images: &Path, md5: &str) -> Option<PathBuf> {
    if !matches!(md5.len(), 32 | 40) || !md5.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    ["jpg", "jpeg", "png", "webp"].iter().find_map(|extension| {
        let file = images.join(format!("{md5}.{extension}"));
        fs::metadata(&file)
            .ok()
            .filter(|meta| meta.is_file() && meta.len() <= COVER_MAX)
            .map(|_| file)
    })
}

/// The cover among a game's key images: only one whose file exists locally (`local` answers that
/// from the image's `md5`); a portrait one (taller than wide) before a landscape one, then the
/// larger. Ties go to the smaller hash, so the answer does not depend on the file's order.
pub fn pick_cover(
    key_images: &[Value],
    local: &dyn Fn(&str) -> Option<PathBuf>,
) -> Option<PathBuf> {
    key_images
        .iter()
        .filter_map(|image| {
            let md5 = image.get("md5")?.as_str()?;
            let file = local(md5)?;
            let side = |key: &str| image.get(key).and_then(Value::as_f64).unwrap_or(0.0);
            let (width, height) = (side("width"), side("height"));
            Some(((height > width, (width * height) as u64), md5, file))
        })
        .max_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(a.1)))
        .map(|(_, _, file)| file)
}

// ---- the whole folder ----

/// The games of one data folder. `Err` is a sentence for the user that names the journal.
/// Blocking.
pub fn read(data: &Path) -> Result<Vec<Found>, String> {
    let journal_file = data.join(JOURNAL_FILE);
    let bytes = read_capped(&journal_file, JOURNAL_MAX).map_err(|problem| match problem {
        ReadProblem::TooLarge => format!("{JOURNAL_FILE} is too large to read"),
        ReadProblem::Unreadable => format!("Could not read {JOURNAL_FILE}"),
    })?;
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    let text = String::from_utf8_lossy(bytes);
    let journal = parse_journal(&text, &|dir| dir.is_dir())
        .map_err(|_| format!("Could not read {JOURNAL_FILE}"))?;
    if journal.bad_lines > 0 || journal.truncated {
        warn!(
            skipped = journal.bad_lines,
            truncated = journal.truncated,
            "{JOURNAL_FILE} had lines that were not used"
        );
    }
    if journal.games.is_empty() {
        return Ok(Vec::new());
    }

    let caches = Caches::load(data);
    Ok(journal
        .games
        .iter()
        .map(|game| found_of(game, &caches))
        .collect())
}

fn found_of(game: &Installed, caches: &Caches) -> Found {
    Found {
        key: format!("epic:{}", game.artifact_id),
        id: format!("game-epic-{}", slug(&game.artifact_id)),
        title: caches
            .title(game)
            .unwrap_or_else(|| game.artifact_id.clone()),
        source: GameSource::Epic,
        launch: Launch::Epic {
            app_name: game.artifact_id.clone(),
            catalog: Some(EpicCatalog {
                namespace: game.namespace.clone(),
                item_id: game.item_id.clone(),
            }),
        },
        install: Some(game.dir.clone()),
        size_bytes: caches.size_bytes(game),
        cover: caches.cover(game),
        playtime_minutes: caches.playtime_minutes(game),
        last_played_unix: caches.last_played_unix(game),
        hidden: false,
        cover_version: None,
        local: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn exists(_: &Path) -> bool {
        true
    }

    fn record(id: &str, art: &str, complete: bool) -> String {
        json!({
            "installationId": id, "complete": complete, "dir": format!("D:\\Epic\\{art}"),
            "platform": "Windows", "namespace": "fixturens", "itemId": "00112233445566778899aabbccddeeff",
            "artifactId": art, "version": "1.0", "tags": ["a", "b"], "sidecar": { "x": 1 },
            "somethingNew": [1, 2, 3],
        })
        .to_string()
    }

    const HEADER: &str = r#"{"version":1,"writtenAt":"2026-01-01T00:00:00Z","entries":2}"#;

    fn journal(lines: &[String]) -> String {
        format!("{HEADER}\n{}\n", lines.join("\n"))
    }

    #[test]
    fn a_journal_with_only_a_header_lists_nothing() {
        let result = parse_journal(&format!("{HEADER}\n"), &exists).unwrap();
        assert_eq!(result, Journal::default());
        assert_eq!(parse_journal("", &exists).unwrap(), Journal::default());
    }

    #[test]
    fn the_last_record_of_an_installation_wins() {
        let text = journal(&[
            record("a1", "FixtureQuest", false),
            record("a1", "FixtureQuest", true),
            record("b2", "FixtureArena", true),
            record("b2", "FixtureArena", false),
        ]);
        let result = parse_journal(&text, &exists).unwrap();
        let names: Vec<&str> = result
            .games
            .iter()
            .map(|g| g.artifact_id.as_str())
            .collect();
        assert_eq!(names, ["FixtureQuest"], "b2 ended as incomplete");
        assert_eq!(result.games[0].namespace, "fixturens");
        assert_eq!(result.games[0].dir, Path::new("D:\\Epic\\FixtureQuest"));
        assert_eq!(result.bad_lines, 0);
    }

    #[test]
    fn an_incomplete_install_and_another_platform_are_not_listed() {
        let mut mac: Value = serde_json::from_str(&record("m", "MacGame", true)).unwrap();
        mac["platform"] = json!("Mac");
        let mut no_platform: Value =
            serde_json::from_str(&record("n", "NoPlatform", true)).unwrap();
        no_platform.as_object_mut().unwrap().remove("platform");
        let mut no_complete: Value =
            serde_json::from_str(&record("c", "NoComplete", true)).unwrap();
        no_complete.as_object_mut().unwrap().remove("complete");
        let text = journal(&[
            record("i", "Pending", false),
            mac.to_string(),
            no_platform.to_string(),
            no_complete.to_string(),
        ]);
        let result = parse_journal(&text, &exists).unwrap();
        let names: Vec<&str> = result
            .games
            .iter()
            .map(|g| g.artifact_id.as_str())
            .collect();
        assert_eq!(names, ["NoPlatform"]);
    }

    #[test]
    fn a_bad_line_is_skipped_and_counted() {
        let text = journal(&[
            record("a", "FixtureQuest", true),
            "{ broken".to_owned(),
            "[1,2]".to_owned(),
            json!({"complete": true}).to_string(),
            record("b", "FixtureArena", true),
        ]);
        let result = parse_journal(&text, &exists).unwrap();
        assert_eq!(result.games.len(), 2);
        assert_eq!(
            result.bad_lines, 3,
            "broken, not an object, no installationId"
        );
    }

    #[test]
    fn a_missing_header_loses_no_game_and_a_header_with_only_junk_is_an_error() {
        let no_header = format!(
            "{}
",
            record("a", "FixtureQuest", true)
        );
        let result = parse_journal(&no_header, &exists).unwrap();
        assert_eq!(result.games.len(), 1);
        assert_eq!(result.bad_lines, 0);
        assert_eq!(
            parse_journal(
                &format!(
                    "{HEADER}
not json
"
                ),
                &exists
            ),
            Err(JournalError::Garbage)
        );
    }

    #[test]
    fn a_folder_that_is_not_a_plain_local_path_is_not_listed() {
        for dir in [
            r"relative\Game",
            r"\\server\share\Game",
            r"\\?\C:\Game",
            r"D:\Epic\..\Game",
            "",
        ] {
            let mut odd: Value = serde_json::from_str(&record("o", "Odd", true)).unwrap();
            odd["dir"] = json!(dir);
            let result = parse_journal(&journal(&[odd.to_string()]), &exists).unwrap();
            assert!(result.games.is_empty(), "{dir:?} must not be listed");
        }
    }

    #[test]
    fn a_journal_of_garbage_is_an_error() {
        assert_eq!(
            parse_journal("not json\nmore\n", &exists),
            Err(JournalError::Garbage)
        );
    }

    #[test]
    fn a_record_whose_folder_is_gone_is_not_installed() {
        let text = journal(&[
            record("a", "FixtureQuest", true),
            record("b", "FixtureArena", true),
        ]);
        let only_arena = |dir: &Path| dir.ends_with("FixtureArena");
        let result = parse_journal(&text, &only_arena).unwrap();
        assert_eq!(result.games.len(), 1);
        assert_eq!(result.games[0].artifact_id, "FixtureArena");
    }

    #[test]
    fn ids_that_do_not_fit_a_launch_address_are_not_listed() {
        let mut odd: Value = serde_json::from_str(&record("o", "Odd", true)).unwrap();
        odd["artifactId"] = json!("Odd Game?x=1");
        let result = parse_journal(&journal(&[odd.to_string()]), &exists).unwrap();
        assert!(result.games.is_empty());
    }

    #[test]
    fn lines_beyond_the_limit_are_not_read() {
        let mut lines = vec![HEADER.to_owned()];
        lines.extend((0..RECORDS_MAX + 5).map(|n| record(&format!("i{n}"), "FixtureQuest", true)));
        let result = parse_journal(&lines.join("\n"), &exists).unwrap();
        assert!(result.truncated);
        assert_eq!(result.games.len(), 1);
    }

    #[test]
    fn a_huge_journal_is_refused_and_a_missing_one_is_unreadable() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join(JOURNAL_FILE);
        let huge = fs::File::create(&file).unwrap();
        huge.set_len(JOURNAL_MAX + 1).unwrap();
        assert_eq!(
            read(temp.path()).unwrap_err(),
            "installations.jsonl is too large to read"
        );
        fs::remove_file(&file).unwrap();
        assert_eq!(
            read(temp.path()).unwrap_err(),
            "Could not read installations.jsonl"
        );
    }

    fn installed() -> Installed {
        Installed {
            namespace: "fixturens".into(),
            item_id: "00112233445566778899aabbccddeeff".into(),
            artifact_id: "FixtureQuest".into(),
            dir: PathBuf::from("D:\\Epic\\FixtureQuest"),
        }
    }

    #[test]
    fn the_title_is_matched_by_all_three_ids() {
        let library = json!({"items": [
            {"appId": {"namespaceId": "fixturens", "itemId": "ffeeddccbbaa99887766554433221100", "appName": "FixtureQuest"}, "title": "Wrong Item"},
            {"appId": {"namespaceId": "fixturens", "itemId": "00112233445566778899aabbccddeeff", "appName": "FixtureQuest"}, "title": " Fixture Quest "},
            {"appId": {"namespaceId": "fixturens", "itemId": "00112233445566778899aabbccddeeff", "appName": "FixtureQuest"}, "title": "Second"},
        ]});
        assert_eq!(
            title_in(&library, &installed()).as_deref(),
            Some("Fixture Quest")
        );
        assert_eq!(title_in(&json!({"items": []}), &installed()), None);
        assert_eq!(title_in(&json!("nonsense"), &installed()), None);
        // Only the relationship id is there.
        let relationship = json!({"items": [
            {"relationshipId": "fixturens:00112233445566778899aabbccddeeff:FixtureQuest", "title": "By Relationship"}
        ]});
        assert_eq!(
            title_in(&relationship, &installed()).as_deref(),
            Some("By Relationship")
        );
    }

    #[test]
    fn a_portrait_cover_that_exists_locally_wins() {
        let images = [
            json!({"md5": "aa", "width": 2000, "height": 1000}),
            json!({"md5": "bb", "width": 600, "height": 800}),
            json!({"md5": "cc", "width": 1200, "height": 1600}),
            json!({"md5": "dd", "width": 3000, "height": 4000}),
            json!({"width": 1, "height": 2}),
        ];
        let local = |md5: &str| (md5 != "dd").then(|| PathBuf::from(format!("{md5}.jpg")));
        assert_eq!(
            pick_cover(&images, &local),
            Some(PathBuf::from("cc.jpg")),
            "dd is not on disk"
        );
        // Landscape only: used when it is all there is. None on disk: no cover.
        assert_eq!(
            pick_cover(&images[..1], &local),
            Some(PathBuf::from("aa.jpg"))
        );
        assert_eq!(pick_cover(&images, &|_| None), None);
        assert_eq!(pick_cover(&[], &local), None);
    }

    #[test]
    fn an_image_name_must_be_a_hash_and_the_file_must_exist() {
        let temp = tempfile::tempdir().unwrap();
        let md5 = "0123456789abcdef0123456789abcdef";
        assert_eq!(local_image(temp.path(), md5), None);
        fs::write(temp.path().join(format!("{md5}.jpg")), [0xFF, 0xD8, 0xFF]).unwrap();
        assert_eq!(
            local_image(temp.path(), md5),
            Some(temp.path().join(format!("{md5}.jpg")))
        );
        assert_eq!(local_image(temp.path(), "..\\..\\secret"), None);
        assert_eq!(local_image(temp.path(), "abc"), None);
    }

    #[test]
    fn size_play_time_and_last_played_come_from_their_caches() {
        let game = installed();
        let caches = Caches {
            sizes: Some(json!({"entries": [
                {"app_id": {"namespaceId": "other", "itemId": "x", "appName": "FixtureQuest"}, "bytes": 1.0},
                {"app_id": {"namespaceId": "fixturens", "itemId": "00112233445566778899aabbccddeeff", "appName": "FixtureQuest"}, "bytes": 37170389839.0},
            ]})),
            playtimes: vec![
                json!({"items": [{"artifactId": "FixtureQuest", "totalTime": 181245}]}),
                json!({"items": [{"artifactId": "FixtureQuest", "totalTime": 600}, {"artifactId": "Other", "totalTime": 9}]}),
            ],
            last_played: Some(json!({
                "acct1": {"fixturens:00112233445566778899aabbccddeeff:FixtureQuest": "2026-11-22T05:57:51.736380500+00:00"},
                "acct2": {"fixturens:00112233445566778899aabbccddeeff:FixtureQuest": "2026-01-01T00:00:00+00:00"},
            })),
            ..Caches::default()
        };
        assert_eq!(caches.size_bytes(&game), Some(37_170_389_839));
        assert_eq!(caches.playtime_minutes(&game), 3020);
        assert_eq!(caches.last_played_unix(&game), Some(1_795_327_071));

        let nothing = Caches::default();
        assert_eq!(nothing.size_bytes(&game), None);
        assert_eq!(nothing.playtime_minutes(&game), 0);
        assert_eq!(nothing.last_played_unix(&game), None);
        assert_eq!(nothing.title(&game), None);
        assert_eq!(nothing.cover(&game), None);
    }
}
