//! What Rain finds out about this PC: the Obsidian vaults, the daily-note settings of the chosen
//! vault, the Steam folder and the Epic Games Launcher folder. Section 06 6.1.2 to 6.1.5.
//!
//! First run needs no setup: a value that the user has not typed is detected here. Nothing
//! personal is in the code: every path comes from Windows, the registry or a file that Obsidian
//! wrote. Every failure means "not found", never an error, and nothing is logged above `debug`
//! (the paths are private).
//!
//! The detectors sit behind `trait Detect` so that tests use a fake. `Detector` keeps the answer
//! for `DETECT_TTL`: the age is checked when the answer is asked for, never by a timer.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use rain_types::limits::DETECT_TTL;
use serde::Deserialize;
use tracing::debug;

use super::rules;
use crate::core::clock::Clock;
use crate::core::paths::normalize;
use crate::platform::known_folders::{KnownFolder, known_folder};
use crate::platform::registry::{Hive, read_string};

/// One vault that Obsidian knows: its key in `obsidian.json` and its folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultEntry {
    pub id: String,
    pub path: PathBuf,
}

/// What a vault's `.obsidian/daily-notes.json` says. A missing file or key is `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteSettings {
    pub format: Option<String>,
    /// Relative to the vault, with `/` as the separator, without a `/` at either end.
    pub folder: Option<String>,
}

/// The detectors. Every method is blocking (small file reads and registry reads) and never fails:
/// what cannot be read is not there.
pub trait Detect: Send + Sync {
    /// The vaults of Obsidian's own list whose folder exists.
    fn vaults(&self) -> Vec<VaultEntry>;
    /// The daily-note settings of this vault.
    fn daily_notes(&self, vault: &Path) -> NoteSettings;
    /// The Steam folder, when Steam is installed.
    fn steam(&self) -> Option<PathBuf>;
    /// The data folder of the Epic Games Launcher, when it is installed.
    fn epic(&self) -> Option<PathBuf>;
}

/// The answers of all detectors at one moment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Detected {
    pub vaults: Vec<VaultEntry>,
    pub steam: Option<PathBuf>,
    pub epic: Option<PathBuf>,
    /// The daily-note settings of the chosen vault (empty without one).
    pub notes: NoteSettings,
}

impl Detected {
    /// The vault that Obsidian offers when the user has chosen none: the only one it knows. With
    /// several there is no default (Rain must not read or write a vault the user did not pick).
    pub fn single_vault(&self) -> Option<&VaultEntry> {
        match self.vaults.as_slice() {
            [only] => Some(only),
            _ => None,
        }
    }
}

// ---- the cache ----

struct Snapshot {
    at: SystemTime,
    /// The vault that was chosen when the notes were read.
    chosen: Option<PathBuf>,
    detected: Arc<Detected>,
}

/// The detectors and their cache.
pub struct Detector {
    source: Arc<dyn Detect>,
    clock: Arc<dyn Clock>,
    cache: Mutex<Option<Snapshot>>,
}

impl Detector {
    pub fn new(source: Arc<dyn Detect>, clock: Arc<dyn Clock>) -> Self {
        Self {
            source,
            clock,
            cache: Mutex::new(None),
        }
    }

    /// The detected values, from the cache while it is younger than `DETECT_TTL` and the chosen
    /// vault has not changed. `vault_override` is the vault that the user typed, normalised, or
    /// `None`: the notes settings are read from it, else from the only detected vault.
    /// Blocking when it has to look.
    pub fn detected(&self, vault_override: Option<&Path>) -> Arc<Detected> {
        let now = self.clock.now();
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(snapshot) = cache.as_ref()
            && is_fresh(snapshot.at, now)
            && snapshot.chosen.as_deref() == vault_override
        {
            return Arc::clone(&snapshot.detected);
        }
        let detected = Arc::new(self.look(vault_override));
        *cache = Some(Snapshot {
            at: now,
            chosen: vault_override.map(Path::to_path_buf),
            detected: Arc::clone(&detected),
        });
        detected
    }

    /// Forgets the answers: the next call looks again. A change of `vault_path`, `steam_path` or
    /// `epic_path` calls this.
    pub fn invalidate(&self) {
        *self.cache.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// Whether the cached answer is too old (or absent), without looking. The service uses it to
    /// decide whether a read has to refresh.
    pub fn is_stale(&self) -> bool {
        let now = self.clock.now();
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_none_or(|snapshot| !is_fresh(snapshot.at, now))
    }

    fn look(&self, vault_override: Option<&Path>) -> Detected {
        let vaults = self.source.vaults();
        let steam = self.source.steam();
        let epic = self.source.epic();
        let mut detected = Detected {
            vaults,
            steam,
            epic,
            notes: NoteSettings::default(),
        };
        let chosen = vault_override
            .map(Path::to_path_buf)
            .or_else(|| detected.single_vault().map(|vault| vault.path.clone()));
        if let Some(vault) = chosen {
            detected.notes = self.source.daily_notes(&vault);
        }
        debug!(vaults = detected.vaults.len(), "detection looked at the PC");
        detected
    }
}

fn is_fresh(at: SystemTime, now: SystemTime) -> bool {
    // A clock that went back counts as fresh: the next real tick makes it stale.
    now.duration_since(at).is_ok_and(|age| age < DETECT_TTL) || now < at
}

// ---- the real detectors ----

/// Reads the real PC. No state.
pub struct SystemDetect;

impl Detect for SystemDetect {
    fn vaults(&self) -> Vec<VaultEntry> {
        let Ok(roaming) = known_folder(KnownFolder::RoamingAppData) else {
            return Vec::new();
        };
        let list = roaming.join("obsidian").join("obsidian.json");
        let Ok(text) = std::fs::read_to_string(list) else {
            return Vec::new();
        };
        parse_obsidian_json(&text)
            .into_iter()
            .filter(|vault| vault.path.is_dir())
            .collect()
    }

    fn daily_notes(&self, vault: &Path) -> NoteSettings {
        let file = vault.join(".obsidian").join("daily-notes.json");
        std::fs::read_to_string(file)
            .map(|text| parse_daily_notes_json(&text))
            .unwrap_or_default()
    }

    fn steam(&self) -> Option<PathBuf> {
        let current_user = read_string(Hive::CurrentUser, r"Software\Valve\Steam", "SteamPath");
        let local_machine = || {
            read_string(
                Hive::LocalMachine,
                r"SOFTWARE\WOW6432Node\Valve\Steam",
                "InstallPath",
            )
        };
        let text = current_user
            .ok()
            .flatten()
            .or_else(|| local_machine().ok().flatten())?;
        existing_folder(&text)
    }

    fn epic(&self) -> Option<PathBuf> {
        epic_folder(&known_folder(KnownFolder::ProgramData).ok()?)
    }
}

/// A folder that exists, from a path as a program wrote it (the Steam value has forward slashes
/// and a lower-case drive letter).
fn existing_folder(text: &str) -> Option<PathBuf> {
    normalize(text, true).ok().filter(|path| path.is_dir())
}

/// The Epic Games Launcher folder under `ProgramData`, when it holds `Data\Manifests`. This only
/// defines what `epicPath` points at; section 02 owns the reading of the manifests and may
/// change the definition. Not seen working: not tested against a real Epic launcher.
pub fn epic_folder(program_data: &Path) -> Option<PathBuf> {
    let launcher = program_data.join("Epic").join("EpicGamesLauncher");
    launcher
        .join("Data")
        .join("Manifests")
        .is_dir()
        .then_some(launcher)
}

// ---- the files that Obsidian writes ----

#[derive(Deserialize)]
struct ObsidianList {
    #[serde(default)]
    vaults: std::collections::BTreeMap<String, ObsidianVault>,
}

#[derive(Deserialize)]
struct ObsidianVault {
    path: Option<String>,
}

/// The vaults of `obsidian.json`: the key of each entry and its `path`. Entries without a path
/// are skipped; so are paths that are not usable absolute paths. Whether the folder exists is
/// the caller's question. Bad JSON gives no vault.
pub fn parse_obsidian_json(text: &str) -> Vec<VaultEntry> {
    let Ok(list) = serde_json::from_str::<ObsidianList>(text) else {
        return Vec::new();
    };
    list.vaults
        .into_iter()
        .filter_map(|(id, vault)| {
            let path = normalize(vault.path.as_deref()?, true).ok()?;
            Some(VaultEntry { id, path })
        })
        .collect()
}

#[derive(Deserialize, Default)]
struct DailyNotesFile {
    format: Option<String>,
    folder: Option<String>,
}

/// `format` and `folder` of `daily-notes.json`; other keys are ignored. A folder that is absolute
/// or goes up with `..` is dropped. Bad JSON gives nothing.
pub fn parse_daily_notes_json(text: &str) -> NoteSettings {
    let file: DailyNotesFile = serde_json::from_str(text).unwrap_or_default();
    NoteSettings {
        format: file
            .format
            .map(|format| format.trim().to_owned())
            .filter(|format| !format.is_empty() && rules::note_format(format).is_ok()),
        folder: file
            .folder
            .and_then(|folder| rules::note_folder(&folder).ok())
            .filter(|folder| !folder.is_empty()),
    }
}

/// Whether two vault paths are the same folder: case does not matter, `/` equals `\`, and a
/// closing slash is ignored.
pub fn same_folder(a: &Path, b: &Path) -> bool {
    let clean = |path: &Path| {
        path.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    };
    clean(a) == clean(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::clock::SystemClock;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// A detector that counts how often it was asked.
    struct Counting {
        calls: AtomicUsize,
        vaults: Vec<VaultEntry>,
    }

    impl Detect for Counting {
        fn vaults(&self) -> Vec<VaultEntry> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.vaults.clone()
        }

        fn daily_notes(&self, _vault: &Path) -> NoteSettings {
            NoteSettings {
                format: Some("YYYY-MM-DD".into()),
                folder: Some("Daily".into()),
            }
        }

        fn steam(&self) -> Option<PathBuf> {
            None
        }

        fn epic(&self) -> Option<PathBuf> {
            None
        }
    }

    struct TestClock(Mutex<SystemTime>);

    impl Clock for TestClock {
        fn now(&self) -> SystemTime {
            *self.0.lock().unwrap()
        }
    }

    fn entry(id: &str, path: &str) -> VaultEntry {
        VaultEntry {
            id: id.into(),
            path: PathBuf::from(path),
        }
    }

    fn detector(vaults: Vec<VaultEntry>) -> (Detector, Arc<Counting>, Arc<TestClock>) {
        let source = Arc::new(Counting {
            calls: AtomicUsize::new(0),
            vaults,
        });
        let clock = Arc::new(TestClock(Mutex::new(
            SystemTime::UNIX_EPOCH + Duration::from_secs(1000),
        )));
        let detector = Detector::new(source.clone(), clock.clone());
        (detector, source, clock)
    }

    #[test]
    fn the_answer_is_kept_for_30_seconds_and_then_looked_up_again() {
        let (detector, source, clock) = detector(vec![entry("a", r"C:\Notes")]);
        detector.detected(None);
        detector.detected(None);
        assert_eq!(
            source.calls.load(Ordering::SeqCst),
            1,
            "the second call used the cache"
        );

        *clock.0.lock().unwrap() += Duration::from_secs(29);
        detector.detected(None);
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);

        *clock.0.lock().unwrap() += Duration::from_secs(2);
        assert!(detector.is_stale());
        detector.detected(None);
        assert_eq!(source.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn invalidate_and_a_changed_vault_override_force_a_new_look() {
        let (detector, source, _clock) = detector(vec![entry("a", r"C:\Notes")]);
        detector.detected(None);
        detector.invalidate();
        assert!(detector.is_stale());
        detector.detected(None);
        assert_eq!(source.calls.load(Ordering::SeqCst), 2);

        detector.detected(Some(Path::new(r"D:\Other")));
        assert_eq!(
            source.calls.load(Ordering::SeqCst),
            3,
            "another vault, another notes file"
        );
    }

    #[test]
    fn the_notes_come_from_the_override_or_else_from_the_only_vault() {
        let (with_one, _, _) = detector(vec![entry("a", r"C:\Notes")]);
        assert_eq!(
            with_one.detected(None).notes.folder.as_deref(),
            Some("Daily")
        );

        let (with_two, _, _) = detector(vec![entry("a", r"C:\One"), entry("b", r"C:\Two")]);
        assert_eq!(
            with_two.detected(None).notes,
            NoteSettings::default(),
            "several vaults and none chosen: nothing is read"
        );
        assert_eq!(
            with_two
                .detected(Some(Path::new(r"C:\Two")))
                .notes
                .folder
                .as_deref(),
            Some("Daily")
        );
    }

    #[test]
    fn only_a_single_vault_is_the_default() {
        let mut detected = Detected::default();
        assert!(detected.single_vault().is_none());
        detected.vaults = vec![entry("a", r"C:\One")];
        assert_eq!(detected.single_vault().unwrap().id, "a");
        detected.vaults.push(entry("b", r"C:\Two"));
        assert!(detected.single_vault().is_none());
    }

    #[test]
    fn a_clock_that_went_back_does_not_make_the_cache_stale() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        assert!(is_fresh(now + Duration::from_secs(5), now));
    }

    #[test]
    fn obsidian_json_gives_the_id_and_a_normalised_path_of_every_vault() {
        let text = r#"{"vaults":{"abc123":{"path":"C:/Notes/Main/","ts":1,"open":true},
            "def456":{"path":"D:\\Work Notes"},"nopath":{"ts":2},"bad":{"path":"relative"}}}"#;
        let vaults = parse_obsidian_json(text);
        assert_eq!(
            vaults,
            [
                entry("abc123", r"C:\Notes\Main"),
                entry("def456", r"D:\Work Notes")
            ]
        );
    }

    #[test]
    fn obsidian_json_that_is_broken_or_empty_gives_no_vault() {
        assert!(parse_obsidian_json("not json").is_empty());
        assert!(parse_obsidian_json("{}").is_empty());
        assert!(parse_obsidian_json(r#"{"vaults":[]}"#).is_empty());
    }

    #[test]
    fn daily_notes_json_gives_format_and_folder_and_ignores_other_keys() {
        let both =
            parse_daily_notes_json(r#"{"folder":"Dailys","format":"YYYY-MM-DD","template":"T"}"#);
        assert_eq!(both.folder.as_deref(), Some("Dailys"));
        assert_eq!(both.format.as_deref(), Some("YYYY-MM-DD"));

        let folder_only = parse_daily_notes_json(r#"{"folder":"Dailys"}"#);
        assert_eq!(folder_only.format, None);
        assert_eq!(folder_only.folder.as_deref(), Some("Dailys"));
    }

    #[test]
    fn daily_notes_json_drops_a_folder_that_is_absolute_or_goes_up_and_a_bad_format() {
        assert_eq!(
            parse_daily_notes_json(r#"{"folder":"C:\\Notes"}"#).folder,
            None
        );
        assert_eq!(parse_daily_notes_json(r#"{"folder":"../x"}"#).folder, None);
        assert_eq!(parse_daily_notes_json(r#"{"folder":""}"#).folder, None);
        assert_eq!(parse_daily_notes_json(r#"{"format":"a:b"}"#).format, None);
        assert_eq!(parse_daily_notes_json("garbage"), NoteSettings::default());
    }

    #[test]
    fn two_spellings_of_a_vault_folder_are_the_same_folder() {
        assert!(same_folder(
            Path::new("C:/Notes/Main/"),
            Path::new(r"c:\notes\main")
        ));
        assert!(!same_folder(
            Path::new(r"C:\Notes"),
            Path::new(r"C:\Notes2")
        ));
    }

    #[test]
    fn the_real_detector_does_not_fail_on_a_pc_without_these_programs() {
        // Whatever this PC has, the calls return values and never panic.
        let real = SystemDetect;
        let _ = real.vaults();
        let _ = real.steam();
        let _ = real.epic();
        let _ = real.daily_notes(Path::new(r"C:\no-such-folder-rain-test"));
        let _ = Detector::new(Arc::new(SystemDetect), Arc::new(SystemClock)).detected(None);
    }
}
