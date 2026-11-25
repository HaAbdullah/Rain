//! Changing the data folder. Section 06 6.3.
//!
//! Re-point, never move. When the new folder already holds Rain files, Rain reads them as they
//! are. When it holds none and the old folder has some, Rain copies them (never overwriting,
//! never deleting the old ones). The reasons: the folder may be kept in a private git
//! repository, so a silent move would damage it; and someone who sets a new folder early would
//! otherwise lose the few settings he already changed.
//!
//! The change takes effect through a backend restart (the stop reason `restart`): every module
//! already reads its files at start, so there is one mechanism and not twelve re-point hooks.
//! This file does the checks, the copy and `machine.toml`; the service does the restart.

use std::fs;
use std::path::{Path, PathBuf};

use tracing::{error, info};

use super::SOURCE;
use super::resolved::expand_env;
use crate::core::error::{ApiError, ApiResult};
use crate::core::paths::{PathProblem, normalize};
use crate::machine_file;

/// The files and the folder that Rain keeps in a data folder (foundation 10.2). A folder holds
/// "Rain files" when it has any of them.
const KNOWN_FILES: [&str; 11] = [
    "settings.toml",
    "pins.toml",
    "pools.toml",
    "app-ids.toml",
    "projects.toml",
    "games.toml",
    "jobs.toml",
    "idle-jobs.toml",
    "habits.toml",
    "hardware.toml",
    "gamemode.toml",
];
/// The folder of the covers that the user picked for games.
const MEDIA_FOLDER: &str = "media";

/// What the change needs to know about the folders of this run.
pub struct Folders<'a> {
    /// The data folder in use now.
    pub current: &'a Path,
    /// The local folder (database, cache, logs): the data folder must stay out of it.
    pub local: &'a Path,
    /// What an empty text means.
    pub default: &'a Path,
    /// `machine.toml`.
    pub machine_file: &'a Path,
}

/// Checks the new folder, prepares it, and writes `machine.toml`. Returns the new folder.
/// Blocking. `request` is the text the user typed; empty means the default folder.
pub fn change(request: &str, folders: &Folders<'_>) -> ApiResult<PathBuf> {
    let request = request.trim();
    let (target, machine_value) = if request.is_empty() {
        (folders.default.to_path_buf(), None)
    } else {
        let target = checked_target(request)?;
        (target.clone(), Some(target))
    };
    refuse_overlap(&target, folders)?;
    prepare_folder(&target)?;
    if is_empty_of_rain_files(&target) {
        copy_rain_files(folders.current, &target)?;
    }
    machine_file::write_data_dir(folders.machine_file, machine_value.as_deref()).map_err(|e| {
        error!(error = %e, "machine.toml could not be written");
        ApiError::failed("Could not save the data folder setting.")
            .with_source(SOURCE)
            .with_reason("saveFailed")
    })?;
    info!("the data folder was changed; the backend restarts");
    Ok(target)
}

/// The typed text as an absolute folder path that is not a file.
fn checked_target(request: &str) -> ApiResult<PathBuf> {
    let target = normalize(&expand_env(request), true).map_err(|problem| match problem {
        PathProblem::TooLong => refusal("tooLong", "That text is too long."),
        PathProblem::BadChar => refusal("badText", "That text has characters Rain cannot use."),
        _ => refusal("badPath", "Enter a full folder path, like D:\\Rain"),
    })?;
    if target.is_file() {
        return Err(refusal("notAFolder", "That path is a file, not a folder."));
    }
    Ok(target)
}

/// The new folder may not be the current one, contain it, lie inside it, or lie inside the local
/// folder.
fn refuse_overlap(target: &Path, folders: &Folders<'_>) -> ApiResult<()> {
    let overlaps = is_inside(target, folders.current)
        || is_inside(folders.current, target)
        || is_inside(target, folders.local);
    if overlaps {
        return Err(refusal(
            "nested",
            "Choose a folder outside the current one.",
        ));
    }
    Ok(())
}

/// Whether `inner` is `outer` or lies below it. Case and a closing slash do not matter.
fn is_inside(inner: &Path, outer: &Path) -> bool {
    let key = |path: &Path| {
        let mut text = path.to_string_lossy().replace('/', "\\").to_lowercase();
        while text.ends_with('\\') && text.len() > 3 {
            text.pop();
        }
        text
    };
    let (inner, outer) = (key(inner), key(outer));
    inner == outer || inner.starts_with(&format!("{}\\", outer.trim_end_matches('\\')))
}

/// Creates the folder if it is missing and proves that Rain can write into it.
fn prepare_folder(target: &Path) -> ApiResult<()> {
    let not_writable = || refusal("notWritable", "Rain cannot write to that folder.");
    fs::create_dir_all(target).map_err(|_| not_writable())?;
    let probe = target.join(format!(".rain-write-test-{}", std::process::id()));
    fs::write(&probe, b"").map_err(|_| not_writable())?;
    // A probe that cannot be removed is a folder that Rain cannot use properly either.
    fs::remove_file(&probe).map_err(|_| not_writable())
}

fn is_empty_of_rain_files(folder: &Path) -> bool {
    !KNOWN_FILES.iter().any(|name| folder.join(name).is_file())
        && !folder.join(MEDIA_FOLDER).exists()
}

/// Copies the known files and the `media` folder from `from` into `to`. A file that already
/// exists in `to` is left alone; nothing in `from` is changed. Only plain files are copied, no
/// links. A failure leaves the files copied so far (harmless) and changes nothing else.
fn copy_rain_files(from: &Path, to: &Path) -> ApiResult<()> {
    let failed = |e: std::io::Error| {
        error!(error = %e, "copying the data files failed");
        ApiError::failed("Could not copy your data files to that folder.")
            .with_source(SOURCE)
            .with_reason("copyFailed")
    };
    for name in KNOWN_FILES {
        copy_file(&from.join(name), &to.join(name)).map_err(failed)?;
    }
    copy_folder(&from.join(MEDIA_FOLDER), &to.join(MEDIA_FOLDER)).map_err(failed)
}

fn copy_file(from: &Path, to: &Path) -> std::io::Result<()> {
    // `symlink_metadata` does not follow a link, so a link is not copied as a file.
    let is_plain_file = fs::symlink_metadata(from).is_ok_and(|meta| meta.is_file());
    if !is_plain_file || to.exists() {
        return Ok(());
    }
    fs::copy(from, to).map(|_| ())
}

fn copy_folder(from: &Path, to: &Path) -> std::io::Result<()> {
    let Ok(entries) = fs::read_dir(from) else {
        return Ok(()); // no media folder: nothing to copy
    };
    fs::create_dir_all(to)?;
    for entry in entries {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_folder(&entry.path(), &target)?;
        } else if kind.is_file() {
            copy_file(&entry.path(), &target)?;
        }
    }
    Ok(())
}

fn refusal(reason: &'static str, message: &str) -> ApiError {
    ApiError::invalid(message)
        .with_source(SOURCE)
        .with_reason(reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three folders of one run in a temporary place, and the machine file next to them.
    struct Run {
        _temp: tempfile::TempDir,
        current: PathBuf,
        local: PathBuf,
        default: PathBuf,
        machine: PathBuf,
        root: PathBuf,
    }

    impl Run {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().to_path_buf();
            let current = root.join("current");
            fs::create_dir_all(&current).unwrap();
            Self {
                current,
                local: root.join("local"),
                default: root.join("default"),
                machine: root.join("machine.toml"),
                root,
                _temp: temp,
            }
        }

        fn folders(&self) -> Folders<'_> {
            Folders {
                current: &self.current,
                local: &self.local,
                default: &self.default,
                machine_file: &self.machine,
            }
        }

        fn change(&self, request: &str) -> ApiResult<PathBuf> {
            change(request, &self.folders())
        }
    }

    fn reason(result: ApiResult<PathBuf>) -> String {
        result.unwrap_err().body.reason.expect("a reason")
    }

    #[test]
    fn a_new_empty_folder_becomes_the_data_folder_and_machine_toml_names_it() {
        let run = Run::new();
        let target = run.root.join("new data");
        let chosen = run.change(&target.to_string_lossy()).unwrap();

        assert_eq!(chosen, target);
        assert!(target.is_dir());
        let machine = machine_file::read(&run.machine);
        assert_eq!(machine.data_dir, Some(target));
        assert!(
            fs::read_dir(&chosen).unwrap().next().is_none(),
            "the probe file is gone and nothing was copied from an empty folder"
        );
    }

    #[test]
    fn an_empty_text_means_the_default_folder_and_removes_the_key() {
        let run = Run::new();
        fs::write(&run.machine, "schema_version = 1\ndata_dir = 'D:\\Old'\n").unwrap();
        let chosen = run.change("  ").unwrap();

        assert_eq!(chosen, run.default);
        assert_eq!(
            fs::read_to_string(&run.machine).unwrap(),
            "schema_version = 1\n"
        );
    }

    #[test]
    fn the_rain_files_are_copied_into_an_empty_target_and_nothing_is_deleted() {
        let run = Run::new();
        fs::write(run.current.join("settings.toml"), "hide_on_blur = false\n").unwrap();
        fs::write(run.current.join("pins.toml"), "x = 1\n").unwrap();
        fs::write(run.current.join("notes.txt"), "not a Rain file").unwrap();
        fs::create_dir_all(run.current.join("media").join("sub")).unwrap();
        fs::write(
            run.current.join("media").join("sub").join("cover.png"),
            "png",
        )
        .unwrap();
        let target = run.root.join("target");

        run.change(&target.to_string_lossy()).unwrap();

        assert_eq!(
            fs::read_to_string(target.join("settings.toml")).unwrap(),
            "hide_on_blur = false\n"
        );
        assert!(target.join("pins.toml").is_file());
        assert_eq!(
            fs::read(target.join("media").join("sub").join("cover.png")).unwrap(),
            b"png"
        );
        assert!(
            !target.join("notes.txt").exists(),
            "only known files are copied"
        );
        assert!(
            run.current.join("settings.toml").is_file(),
            "the old files stay"
        );
    }

    #[test]
    fn a_target_that_has_a_rain_file_is_re_pointed_without_copying() {
        let run = Run::new();
        fs::write(run.current.join("settings.toml"), "hide_on_blur = false\n").unwrap();
        fs::write(run.current.join("pins.toml"), "x = 1\n").unwrap();
        let target = run.root.join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("jobs.toml"), "y = 2\n").unwrap();

        run.change(&target.to_string_lossy()).unwrap();

        assert!(!target.join("settings.toml").exists());
        assert!(!target.join("pins.toml").exists());
        assert_eq!(
            fs::read_to_string(target.join("jobs.toml")).unwrap(),
            "y = 2\n"
        );
    }

    #[test]
    fn a_target_with_a_media_folder_counts_as_a_data_folder() {
        let run = Run::new();
        fs::write(run.current.join("settings.toml"), "x = 1\n").unwrap();
        let target = run.root.join("target");
        fs::create_dir_all(target.join("media")).unwrap();
        run.change(&target.to_string_lossy()).unwrap();
        assert!(!target.join("settings.toml").exists());
    }

    #[test]
    fn a_file_in_the_target_is_never_overwritten() {
        let run = Run::new();
        fs::write(run.current.join("settings.toml"), "from the old folder\n").unwrap();
        // `settings.toml` is missing in the target, but `pools.toml` is not, so it counts as
        // holding Rain files and nothing is copied. Copy-when-empty only runs for an empty one.
        let target = run.root.join("target");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("pools.toml"), "mine\n").unwrap();
        run.change(&target.to_string_lossy()).unwrap();
        assert_eq!(
            fs::read_to_string(target.join("pools.toml")).unwrap(),
            "mine\n"
        );
        assert!(!target.join("settings.toml").exists());
    }

    #[test]
    fn a_relative_or_unusable_text_is_not_a_full_path() {
        let run = Run::new();
        for bad in ["RainData", "C:foo", r"..\x", r"\\?\C:\x"] {
            assert_eq!(reason(run.change(bad)), "badPath", "{bad}");
        }
        assert_eq!(reason(run.change("C:\\a\tb")), "badText");
        assert_eq!(
            reason(run.change(&format!(r"C:\{}", "x".repeat(1100)))),
            "tooLong"
        );
    }

    #[test]
    fn a_file_is_not_a_folder() {
        let run = Run::new();
        let file = run.root.join("a file");
        fs::write(&file, "x").unwrap();
        assert_eq!(reason(run.change(&file.to_string_lossy())), "notAFolder");
    }

    #[test]
    fn the_new_folder_may_not_overlap_the_current_or_the_local_folder() {
        let run = Run::new();
        let inside_current = run.current.join("inner");
        let above_current = run.root.clone();
        let inside_local = run.local.join("cache");
        for (name, path) in [
            ("the same folder", run.current.clone()),
            ("inside the current folder", inside_current),
            ("around the current folder", above_current),
            ("the local folder", run.local.clone()),
            ("inside the local folder", inside_local),
        ] {
            assert_eq!(
                reason(run.change(&path.to_string_lossy())),
                "nested",
                "{name}"
            );
        }
        assert!(
            !machine_file::read(&run.machine).data_dir.is_some(),
            "nothing was written"
        );
    }

    #[test]
    fn a_folder_that_cannot_be_written_is_refused() {
        let run = Run::new();
        // A path below a file cannot be created.
        let blocker = run.root.join("blocker");
        fs::write(&blocker, "x").unwrap();
        let below = blocker.join("data");
        assert_eq!(reason(run.change(&below.to_string_lossy())), "notWritable");
    }

    #[test]
    fn a_name_that_only_starts_like_the_current_folder_is_not_inside_it() {
        assert!(!is_inside(Path::new(r"D:\Rain2"), Path::new(r"D:\Rain")));
        assert!(is_inside(Path::new(r"D:\Rain\x"), Path::new(r"d:/rain/")));
        assert!(is_inside(Path::new(r"D:\Rain"), Path::new(r"D:\Rain")));
        assert!(is_inside(Path::new(r"D:\x"), Path::new(r"D:\")));
    }
}
