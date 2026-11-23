//! Steam: the installed games of the signed-in account, with local covers only.
//! Section 02, 6.9. Nothing here touches the network.
//!
//! Where the facts come from:
//! - `steamapps\libraryfolders.vdf` lists the library folders (the Steam folder itself, and any
//!   other drive the user installs games to);
//! - `steamapps\appmanifest_<appid>.acf` in each library is one installed game;
//! - `userdata\<account>\config\localconfig.vdf` has Steam's own play time and last played;
//! - `appcache\librarycache` has the cover pictures Steam already downloaded.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rain_types::GameSource;
use tracing::warn;

use super::model::{Found, Launch, SourceResult};
use super::names::path_key;
use super::vdf::{self, Vdf};
use crate::platform::registry::{self, Hive};

/// The part of Steam that lives in the registry. A trait so that a test says who is signed in.
pub trait SteamRegistry: Send + Sync {
    /// `HKCU\Software\Valve\Steam\ActiveProcess`, `ActiveUser`: the account id of the running
    /// Steam client, 0 or `None` when Steam is signed out or not running.
    fn active_user(&self) -> Option<u32>;
    /// `HKCU\Software\Valve\Steam`, `AutoLoginUser`: the account name that signs in by itself.
    fn auto_login_user(&self) -> Option<String>;
    /// `HKCU\Software\Valve\Steam`, `RunningAppID`: the game that runs now, 0 for none.
    fn running_app_id(&self) -> Option<u32>;
}

/// The real registry of this PC.
pub struct SystemSteamRegistry;

const STEAM_KEY: &str = r"Software\Valve\Steam";

impl SteamRegistry for SystemSteamRegistry {
    fn active_user(&self) -> Option<u32> {
        registry::read_u32(
            Hive::CurrentUser,
            r"Software\Valve\Steam\ActiveProcess",
            "ActiveUser",
        )
        .ok()
        .flatten()
    }

    fn auto_login_user(&self) -> Option<String> {
        registry::read_string(Hive::CurrentUser, STEAM_KEY, "AutoLoginUser")
            .ok()
            .flatten()
            .filter(|name| !name.is_empty())
    }

    fn running_app_id(&self) -> Option<u32> {
        registry::read_u32(Hive::CurrentUser, STEAM_KEY, "RunningAppID")
            .ok()
            .flatten()
    }
}

/// A manifest file larger than this is not a manifest.
const MANIFEST_MAX: u64 = 256 * 1024;
/// `localconfig.vdf` can be large when the account owns many games.
const LOCALCONFIG_MAX: u64 = 4 * 1024 * 1024;
/// `steamid64 - ACCOUNT_BASE` is the account id used in the `userdata` folder names.
const ACCOUNT_BASE: u64 = 76_561_197_960_265_728;

/// Tools that Steam lists as installed apps. The name rule below is the safety net for the ones
/// not listed here.
const TOOL_APPIDS: &[&str] = &["228980", "1007", "1070560", "1391110", "1628350"];
const TOOL_NAME_PREFIXES: &[&str] = &[
    "Steamworks Common",
    "Steam Linux Runtime",
    "Proton ",
    "Steam Client",
];

/// Reads the Steam library. Blocking: call it through `core::blocking::run`.
///
/// `enabled` and `path` come from the settings: `path` is `None` when no override is set and
/// Steam was not detected.
pub fn scan(enabled: bool, path: Option<&Path>, registry: &dyn SteamRegistry) -> SourceResult {
    if !enabled {
        return SourceResult::off("Turned off in Settings");
    }
    let Some(steam) = path else {
        return SourceResult::off("Steam is not installed");
    };
    if !steam.is_dir() {
        return SourceResult::failed(format!("Steam folder not found: {}", steam.display()));
    }

    let (libraries, missing_libraries, list_read) = libraries(steam);
    let account = account(steam, registry);
    let played = account.and_then(|id| read_local_config(steam, id));

    let mut items = Vec::new();
    let (mut manifests, mut failed_manifests) = (0usize, 0usize);
    for library in &libraries {
        let Ok(entries) = fs::read_dir(library.join("steamapps")) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| is_manifest_name(path))
            .collect();
        files.sort();
        for file in files {
            manifests += 1;
            match read_manifest(&file) {
                Ok(Some(manifest)) => {
                    items.push(found(steam, library, manifest, played.as_ref()));
                }
                Ok(None) => {}
                Err(()) => {
                    failed_manifests += 1;
                    // Only the file name: a game name is not private, but a path may be.
                    warn!(file = ?file.file_name(), "a Steam manifest could not be read");
                }
            }
        }
    }
    if (!list_read && manifests == 0) || (manifests > 0 && failed_manifests == manifests) {
        return SourceResult::failed("Could not read the Steam library");
    }

    let mut notes = Vec::new();
    if missing_libraries > 0 {
        notes.push(format!(
            "{missing_libraries} library folder(s) not available"
        ));
    }
    if account.is_none() {
        notes.push("Play time is not available (Steam is signed out)".to_owned());
    }
    let message = (!notes.is_empty()).then(|| notes.join(". "));
    SourceResult::ok(items, message)
}

// ---- libraries ----

/// The library folders: Steam itself plus every `path` of `libraryfolders.vdf`, without repeats.
/// Also how many listed folders do not exist (a drive that is not plugged in) and whether the
/// list file could be read.
fn libraries(steam: &Path) -> (Vec<PathBuf>, usize, bool) {
    let mut found = vec![steam.to_path_buf()];
    let mut missing = 0;
    let text = read_text(
        &steam.join("steamapps").join("libraryfolders.vdf"),
        MANIFEST_MAX,
    );
    let parsed = text.as_deref().and_then(|text| vdf::parse(text).ok());
    let list_read = parsed.is_some();
    if let Some(root) = parsed {
        for (name, entry) in root
            .get("libraryfolders")
            .map(Vdf::children)
            .unwrap_or_default()
        {
            if name.is_empty() || !name.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            // Newer Steam writes a block with `path`, older Steam wrote the path as the value.
            let path = entry.text_of("path").or_else(|| entry.text());
            let Some(path) = path else { continue };
            let path = PathBuf::from(super::names::norm_path(path));
            if found.iter().any(|known| {
                path_key(&known.to_string_lossy()) == path_key(&path.to_string_lossy())
            }) {
                continue;
            }
            if path.join("steamapps").is_dir() {
                found.push(path);
            } else {
                missing += 1;
            }
        }
    }
    (found, missing, list_read)
}

// ---- manifests ----

fn is_manifest_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            let lower = name.to_ascii_lowercase();
            lower.starts_with("appmanifest_") && lower.ends_with(".acf")
        })
}

/// What one manifest says about one installed game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub appid: String,
    pub name: String,
    pub install_dir: String,
    pub size_bytes: Option<u64>,
    pub last_played: Option<i64>,
}

fn read_manifest(file: &Path) -> Result<Option<Manifest>, ()> {
    // Steam rewrites manifests while it updates a game, so a failed read is tried once more.
    let text = read_text(file, MANIFEST_MAX)
        .or_else(|| {
            std::thread::park_timeout(Duration::from_millis(50));
            read_text(file, MANIFEST_MAX)
        })
        .ok_or(())?;
    let root = vdf::parse(&text).map_err(|_| ())?;
    Ok(parse_manifest(&root))
}

/// The game of a parsed manifest, or `None` for a file that is not an installed game: a tool, a
/// download that is not finished, or a file without an id or a name. Pure.
pub fn parse_manifest(root: &Vdf) -> Option<Manifest> {
    let state = root.get("AppState")?;
    let appid = state.text_of("appid")?.trim();
    if appid.is_empty() || !appid.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let name = state.text_of("name")?.trim();
    if name.is_empty() || TOOL_APPIDS.contains(&appid) {
        return None;
    }
    if TOOL_NAME_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
    {
        return None;
    }
    // Bit 4 = fully installed. Bits 2 (update needed), 256, 512 and 1024 may be set at the same
    // time and the game still starts. A first download (1026) has no bit 4: skipped.
    let flags: u64 = state.text_of("StateFlags")?.trim().parse().ok()?;
    if flags & 4 == 0 {
        return None;
    }
    Some(Manifest {
        appid: appid.to_owned(),
        name: name.to_owned(),
        install_dir: state
            .text_of("installdir")
            .unwrap_or_default()
            .trim()
            .to_owned(),
        size_bytes: state
            .text_of("SizeOnDisk")
            .and_then(|text| text.trim().parse().ok()),
        last_played: unix_seconds(state.text_of("LastPlayed")),
    })
}

/// Unix seconds from Steam's text; `"0"` and a missing value mean "never".
fn unix_seconds(text: Option<&str>) -> Option<i64> {
    text.and_then(|text| text.trim().parse::<i64>().ok())
        .filter(|seconds| *seconds > 0)
}

fn found(steam: &Path, library: &Path, manifest: Manifest, played: Option<&LocalConfig>) -> Found {
    let from_config =
        played.and_then(|config| config.apps.iter().find(|app| app.0 == manifest.appid));
    let install = (!manifest.install_dir.is_empty()).then(|| {
        library
            .join("steamapps")
            .join("common")
            .join(&manifest.install_dir)
    });
    let last_played = [manifest.last_played, from_config.and_then(|app| app.2)]
        .into_iter()
        .flatten()
        .max();
    Found {
        key: format!("steam:{}", manifest.appid),
        id: format!("game-steam-{}", manifest.appid),
        title: manifest.name,
        source: GameSource::Steam,
        launch: Launch::Steam {
            appid: manifest.appid.clone(),
        },
        install,
        size_bytes: manifest.size_bytes,
        cover: cover(steam, &manifest.appid),
        playtime_minutes: from_config.map_or(0, |app| app.1),
        last_played_unix: last_played,
        hidden: false,
        cover_version: None,
        local: None,
    }
}

// ---- play time ----

/// The apps of `localconfig.vdf`: (appid, playtime in minutes, last played in Unix seconds).
#[derive(Debug, Default)]
struct LocalConfig {
    apps: Vec<(String, u32, Option<i64>)>,
}

fn read_local_config(steam: &Path, account: u32) -> Option<LocalConfig> {
    let file = local_config_path(steam, account);
    let text = read_text(&file, LOCALCONFIG_MAX)?;
    let root = vdf::parse(&text).ok()?;
    Some(parse_local_config(&root))
}

fn local_config_path(steam: &Path, account: u32) -> PathBuf {
    steam
        .join("userdata")
        .join(account.to_string())
        .join("config")
        .join("localconfig.vdf")
}

fn parse_local_config(root: &Vdf) -> LocalConfig {
    let apps = root
        .path(&["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"])
        .map(Vdf::children)
        .unwrap_or_default();
    LocalConfig {
        apps: apps
            .iter()
            .map(|(appid, app)| {
                let minutes = app
                    .text_of("Playtime")
                    .and_then(|text| text.trim().parse::<u32>().ok())
                    .unwrap_or(0);
                (
                    appid.clone(),
                    minutes,
                    unix_seconds(app.text_of("LastPlayed")),
                )
            })
            .collect(),
    }
}

// ---- which account ----

/// The account whose play time is shown. The account id is never logged or stored.
fn account(steam: &Path, registry: &dyn SteamRegistry) -> Option<u32> {
    let login_users = read_text(&steam.join("config").join("loginusers.vdf"), MANIFEST_MAX)
        .and_then(|text| vdf::parse(&text).ok());
    choose_account(
        registry.active_user(),
        registry.auto_login_user().as_deref(),
        login_users.as_ref(),
        || newest_userdata_folder(steam),
    )
}

/// The four rules of section 6.9, in order: the running client's user; the auto-login name
/// looked up in `loginusers.vdf`; the user marked `MostRecent`; the `userdata` folder whose
/// `localconfig.vdf` is newest. Pure apart from the last rule, which is a closure.
fn choose_account(
    active: Option<u32>,
    auto_login: Option<&str>,
    login_users: Option<&Vdf>,
    newest_folder: impl FnOnce() -> Option<u32>,
) -> Option<u32> {
    if let Some(id) = active.filter(|id| *id != 0) {
        return Some(id);
    }
    let users = login_users
        .and_then(|root| root.get("users"))
        .map(Vdf::children)
        .unwrap_or_default();
    let account_of = |steam_id: &str| {
        steam_id
            .trim()
            .parse::<u64>()
            .ok()
            .and_then(|id| id.checked_sub(ACCOUNT_BASE))
            .and_then(|id| u32::try_from(id).ok())
            .filter(|id| *id != 0)
    };
    if let Some(name) = auto_login {
        let by_name = users
            .iter()
            .find(|(_, user)| {
                user.text_of("AccountName")
                    .is_some_and(|account| account.eq_ignore_ascii_case(name))
            })
            .and_then(|(steam_id, _)| account_of(steam_id));
        if by_name.is_some() {
            return by_name;
        }
    }
    let most_recent = users
        .iter()
        .find(|(_, user)| user.text_of("MostRecent") == Some("1"))
        .and_then(|(steam_id, _)| account_of(steam_id));
    most_recent.or_else(newest_folder)
}

fn newest_userdata_folder(steam: &Path) -> Option<u32> {
    fs::read_dir(steam.join("userdata"))
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let id: u32 = entry.file_name().to_str()?.parse().ok()?;
            let modified = fs::metadata(local_config_path(steam, id))
                .ok()?
                .modified()
                .ok()?;
            Some((modified, id))
        })
        .max()
        .map(|(_, id)| id)
        .filter(|id| *id != 0)
}

// ---- covers ----

/// The portrait cover that Steam cached, in the order of section 6.9. `header.jpg` is landscape
/// and crops badly in a portrait tile, so it is never used.
fn cover(steam: &Path, appid: &str) -> Option<PathBuf> {
    let folder = steam.join("appcache").join("librarycache").join(appid);
    let portrait = "library_600x900.jpg";
    let capsule = "library_capsule.jpg";
    let flat = folder.join(portrait);
    if flat.is_file() {
        return Some(flat);
    }
    // The newer layout puts each picture into a folder named by a 40-digit hash.
    let mut subfolders: Vec<PathBuf> = fs::read_dir(&folder)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    subfolders.sort();
    for name in [portrait, capsule] {
        if let Some(found) = subfolders
            .iter()
            .map(|sub| sub.join(name))
            .find(|path| path.is_file())
        {
            return Some(found);
        }
    }
    let flat_capsule = folder.join(capsule);
    flat_capsule.is_file().then_some(flat_capsule)
}

// ---- reading files ----

/// A text file of at most `limit` bytes, or `None` when it is missing, unreadable or too large.
fn read_text(path: &Path, limit: u64) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut text = String::new();
    file.take(limit + 1).read_to_string(&mut text).ok()?;
    (text.len() as u64 <= limit).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct FakeRegistry {
        active: Option<u32>,
        auto: Option<&'static str>,
        running: Option<u32>,
    }

    impl SteamRegistry for FakeRegistry {
        fn active_user(&self) -> Option<u32> {
            self.active
        }
        fn auto_login_user(&self) -> Option<String> {
            self.auto.map(str::to_owned)
        }
        fn running_app_id(&self) -> Option<u32> {
            self.running
        }
    }

    fn manifest(flags: &str, appid: &str, name: &str) -> String {
        format!(
            "\"AppState\"\n{{\n\"appid\" \"{appid}\"\n\"name\" \"{name}\"\n\"installdir\" \"Fixture Quest\"\n\"StateFlags\" \"{flags}\"\n\"SizeOnDisk\" \"2048\"\n\"LastPlayed\" \"0\"\n}}"
        )
    }

    fn parsed(text: &str) -> Option<Manifest> {
        parse_manifest(&vdf::parse(text).unwrap())
    }

    #[test]
    fn only_fully_installed_games_are_manifests() {
        assert!(parsed(&manifest("4", "1000001", "Fixture's Quest")).is_some());
        assert!(
            parsed(&manifest("6", "1000001", "A")).is_some(),
            "update pending still starts"
        );
        assert!(
            parsed(&manifest("1026", "1000001", "A")).is_none(),
            "first download"
        );
        assert!(parsed(&manifest("0", "1000001", "A")).is_none());
    }

    #[test]
    fn tools_and_bad_ids_are_dropped() {
        assert!(
            parsed(&manifest(
                "4",
                "228980",
                "Steamworks Common Redistributables"
            ))
            .is_none()
        );
        assert!(parsed(&manifest("4", "5", "Proton 8.0")).is_none());
        assert!(parsed(&manifest("4", "5", "Steam Linux Runtime 3.0")).is_none());
        assert!(parsed(&manifest("4", "12ab", "A")).is_none());
        assert!(parsed(&manifest("4", "5", "")).is_none());
    }

    #[test]
    fn a_manifest_gives_its_size_and_never_played() {
        let game = parsed(&manifest("4", "1000001", "Fixture's Quest")).unwrap();
        assert_eq!(game.size_bytes, Some(2048));
        assert_eq!(game.last_played, None);
        assert_eq!(game.install_dir, "Fixture Quest");
    }

    fn users() -> Vdf {
        vdf::parse(
            "\"users\" {
               \"76561197960266728\" { \"AccountName\" \"alpha\" \"MostRecent\" \"0\" }
               \"76561197960267728\" { \"AccountName\" \"beta\" \"MostRecent\" \"1\" }
             }",
        )
        .unwrap()
    }

    #[test]
    fn the_account_follows_the_four_rules_in_order() {
        let users = users();
        let none = || None;
        // 1: the running client wins.
        assert_eq!(
            choose_account(Some(42), Some("alpha"), Some(&users), none),
            Some(42)
        );
        // 2: the auto-login name, converted from the 64-bit id (…266728 - base = 1000).
        assert_eq!(
            choose_account(Some(0), Some("ALPHA"), Some(&users), none),
            Some(1000)
        );
        // 3: MostRecent when the name is unknown.
        assert_eq!(
            choose_account(None, Some("nobody"), Some(&users), none),
            Some(2000)
        );
        assert_eq!(choose_account(None, None, Some(&users), none), Some(2000));
        // 4: the newest userdata folder.
        assert_eq!(choose_account(None, None, None, || Some(7)), Some(7));
        // none
        assert_eq!(choose_account(None, None, None, none), None);
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// A made-up Steam folder: two libraries, a tool, a first download, play time for one game.
    fn steam_tree(root: &Path) -> (PathBuf, PathBuf) {
        let steam = root.join("Steam");
        let second = root.join("Library2");
        let list = format!(
            "\"libraryfolders\" {{ \"0\" {{ \"path\" \"{}\" }} \"1\" {{ \"path\" \"{}\" }} \"2\" {{ \"path\" \"{}\" }} }}",
            steam.display().to_string().replace('\\', "\\\\"),
            second.display().to_string().replace('\\', "\\\\"),
            root.join("Unplugged")
                .display()
                .to_string()
                .replace('\\', "\\\\"),
        );
        write(&steam.join("steamapps").join("libraryfolders.vdf"), &list);
        write(
            &steam.join("steamapps").join("appmanifest_1000001.acf"),
            &manifest("4", "1000001", "Fixture's Quest"),
        );
        write(
            &steam.join("steamapps").join("appmanifest_228980.acf"),
            &manifest("4", "228980", "Steamworks Common Redistributables"),
        );
        write(
            &steam.join("steamapps").join("appmanifest_1000002.acf"),
            &manifest("1026", "1000002", "Still Downloading"),
        );
        write(
            &second.join("steamapps").join("appmanifest_1000003.acf"),
            &manifest("4", "1000003", "Second Library Game"),
        );
        write(
            &steam
                .join("userdata")
                .join("1111")
                .join("config")
                .join("localconfig.vdf"),
            "\"UserLocalConfigStore\" { \"Software\" { \"Valve\" { \"Steam\" { \"apps\" {
               \"1000001\" { \"Playtime\" \"125\" \"LastPlayed\" \"1700000000\" }
               \"1000003\" { \"Playtime\" \"5\" }
             } } } } }",
        );
        write(
            &steam
                .join("appcache")
                .join("librarycache")
                .join("1000001")
                .join("library_600x900.jpg"),
            "jpg",
        );
        write(
            &steam
                .join("appcache")
                .join("librarycache")
                .join("1000003")
                .join("0123456789abcdef0123456789abcdef01234567")
                .join("library_capsule.jpg"),
            "jpg",
        );
        (steam, second)
    }

    #[test]
    fn a_steam_folder_gives_its_installed_games_from_every_library() {
        let temp = tempfile::tempdir().unwrap();
        let (steam, second) = steam_tree(temp.path());
        let registry = FakeRegistry {
            active: Some(1111),
            auto: None,
            running: None,
        };
        let result = scan(true, Some(&steam), &registry);

        assert_eq!(result.state, rain_types::SourceState::Ok);
        let by_key: HashMap<_, _> = result.items.iter().map(|g| (g.key.as_str(), g)).collect();
        assert_eq!(by_key.len(), 2, "{:?}", result.items);
        let quest = by_key["steam:1000001"];
        assert_eq!(quest.title, "Fixture's Quest");
        assert_eq!(quest.id, "game-steam-1000001");
        assert_eq!(quest.playtime_minutes, 125);
        assert_eq!(quest.last_played_unix, Some(1_700_000_000));
        assert_eq!(quest.size_bytes, Some(2048));
        assert!(
            quest
                .cover
                .as_ref()
                .unwrap()
                .ends_with("library_600x900.jpg")
        );
        assert_eq!(
            quest.install.as_deref(),
            Some(steam.join("steamapps/common/Fixture Quest").as_path())
        );
        let other = by_key["steam:1000003"];
        assert!(other.install.as_ref().unwrap().starts_with(&second));
        assert!(
            other
                .cover
                .as_ref()
                .unwrap()
                .ends_with("library_capsule.jpg")
        );
        assert!(
            result
                .message
                .as_deref()
                .unwrap()
                .contains("1 library folder(s) not available"),
            "{:?}",
            result.message
        );
    }

    #[test]
    fn a_signed_out_steam_lists_games_with_no_play_time() {
        let temp = tempfile::tempdir().unwrap();
        let (steam, _) = steam_tree(temp.path());
        // No registry answer, no loginusers.vdf: the newest userdata folder is used (rule 4), so
        // remove it to get the signed-out case.
        fs::remove_dir_all(steam.join("userdata")).unwrap();
        let registry = FakeRegistry {
            active: None,
            auto: None,
            running: None,
        };
        let result = scan(true, Some(&steam), &registry);
        assert_eq!(result.state, rain_types::SourceState::Ok);
        assert!(result.items.iter().all(|g| g.playtime_minutes == 0));
        assert!(result.message.unwrap().contains("signed out"));
    }

    #[test]
    fn the_off_and_failed_states_follow_the_settings() {
        let registry = FakeRegistry {
            active: None,
            auto: None,
            running: None,
        };
        let off = scan(false, Some(Path::new("D:\\x")), &registry);
        assert_eq!(off.state, rain_types::SourceState::Off);
        assert_eq!(off.message.as_deref(), Some("Turned off in Settings"));
        let absent = scan(true, None, &registry);
        assert_eq!(absent.message.as_deref(), Some("Steam is not installed"));
        let temp = tempfile::tempdir().unwrap();
        let wrong = temp.path().join("nothing-here");
        let failed = scan(true, Some(&wrong), &registry);
        assert_eq!(failed.state, rain_types::SourceState::Failed);
        assert!(
            failed
                .message
                .unwrap()
                .starts_with("Steam folder not found: ")
        );
    }

    #[test]
    fn a_library_that_cannot_be_read_at_all_is_a_failure() {
        let temp = tempfile::tempdir().unwrap();
        let steam = temp.path().join("Steam");
        write(&steam.join("steamapps").join("libraryfolders.vdf"), "");
        let registry = FakeRegistry {
            active: None,
            auto: None,
            running: None,
        };
        // An empty list file parses (no libraries) and there are no manifests: an empty library.
        assert_eq!(
            scan(true, Some(&steam), &registry).state,
            rain_types::SourceState::Ok
        );
        // A manifest cut in half, the only one: every manifest failed.
        write(
            &steam.join("steamapps").join("appmanifest_5.acf"),
            "\"AppState\" {\n\"appid\"",
        );
        let broken = scan(true, Some(&steam), &registry);
        assert_eq!(broken.state, rain_types::SourceState::Failed);
        assert_eq!(
            broken.message.as_deref(),
            Some("Could not read the Steam library")
        );
    }

    #[test]
    fn only_a_portrait_or_capsule_is_a_cover() {
        let temp = tempfile::tempdir().unwrap();
        let folder = temp.path().join("appcache").join("librarycache").join("9");
        write(&folder.join("header.jpg"), "x");
        assert_eq!(cover(temp.path(), "9"), None, "header.jpg is landscape");
        write(&folder.join("library_capsule.jpg"), "x");
        assert!(
            cover(temp.path(), "9")
                .unwrap()
                .ends_with("library_capsule.jpg")
        );
        write(&folder.join("abc").join("library_600x900.jpg"), "x");
        assert!(
            cover(temp.path(), "9")
                .unwrap()
                .ends_with("library_600x900.jpg")
        );
    }
}
