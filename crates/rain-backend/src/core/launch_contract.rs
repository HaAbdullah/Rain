//! The plain Rust types that the launcher, the projects and the games share: how a program is
//! started, how a module offers items to search, and how it plans a launch.
//! Foundation 11.1 and 11.6; feature section 01, 6.8.1 and 6.9.
//!
//! Why this file is in `core`: modules call each other only through a `Service`, and the launcher
//! must call into projects and games while they call back into the launcher's engine. One file
//! of plain types that all three use breaks the cycle. It may name only `rain-types`, `std` and
//! its own types.
//!
//! - `Opener` is THE one way a program is started: the launcher engine, game mode and the daily
//!   note all use `state.opener()`, and tests replace it with one fake that records the requests.
//! - `ItemSource` is implemented by the `Service` of projects and games. Sessions, jobs and idle
//!   jobs are mapped to search items by the launcher itself.
//! - `LaunchPlanner` is implemented by the same two services. It turns a launch target into
//!   steps; the launcher engine runs them.

use std::path::PathBuf;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use rain_types::{IconRef, LaunchRun, LaunchTarget, LayoutZone, SearchItem};

use super::error::ApiResult;

// ---- starting a program ----

/// One request to start something. The kinds are the three ways Rain starts programs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenRequest {
    /// A packaged app by its AUMID, through `IApplicationActivationManager` (160 to 260 ms,
    /// returns the process id).
    Activate { aumid: String, args: Option<String> },
    /// `ShellExecuteExW`: a shortcut, a URI, a folder, `shell:AppsFolder\<AUMID>`, or a file with
    /// parameters and a working folder.
    ShellExec {
        file: String,
        params: Option<String>,
        dir: Option<String>,
    },
    /// A direct process start where the exact argument text or the working folder matters (a
    /// game). Detached, in its own process group, never inside a job object.
    Spawn {
        exe: PathBuf,
        args_raw: String,
        cwd: PathBuf,
    },
}

/// What a successful start knows about the new process (`Spawn` and `Activate` only).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Started {
    pub pid: Option<u32>,
    /// The creation time of the process as a Windows FILETIME. With the pid it identifies the
    /// process even after the pid is reused.
    pub creation_time: Option<u64>,
    /// `Spawn` only: the backend runs inside a job object that forbids breakaway, so the new
    /// program is inside that job too and may end together with the backend. False for a program
    /// that is free of the backend (and for every other kind of request).
    pub in_job: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenError {
    /// A short plain text for the user.
    pub ui_text: &'static str,
    /// The `ShellExecute` result (32 or less), or the Win32 or HRESULT low word of the start
    /// call. `None` for a timeout or a check made before the call. `31` means that no program is
    /// registered for the file type. Callers match on this, never on the texts.
    pub code: Option<u32>,
    /// For the log only. Never holds a command line.
    pub log_reason: String,
}

pub trait Opener: Send + Sync {
    fn open(&self, request: OpenRequest) -> BoxFuture<'_, Result<Started, OpenError>>;
}

// ---- search ----

/// Implemented ONLY by the `Service` of projects and games (section 02).
pub trait ItemSource: Send + Sync {
    /// Items that search may find, each with its keywords. The owner has left out hidden games
    /// and disabled sources. A project whose folder is missing is still returned. `score` is 0.
    fn search_entries(&self) -> Vec<(SearchItem, Vec<String>)>;

    /// The item for pins and recent items; `None` when the id is gone, a game is hidden or its
    /// source is off. A missing project is returned.
    fn item_for(&self, id: &str) -> Option<SearchItem>;

    /// Whether the id exists and may be pinned (a hidden game may not).
    fn knows(&self, id: &str) -> bool {
        self.item_for(id).is_some()
    }

    /// The owner's own history, newest first: the time (RFC 3339) and the item.
    fn recent(&self) -> Vec<(String, SearchItem)>;
}

// ---- planning a launch ----

/// Implemented by projects and games for their launch targets.
pub trait LaunchPlanner: Send + Sync {
    /// Reads caches only. It starts no process and walks no directory (a few `symlink_metadata`
    /// calls at most).
    fn plan(&self, target: &LaunchTarget) -> ApiResult<LaunchPlan>;

    /// Awaited by the launcher engine before the final frame of a run. The owner writes its
    /// history here.
    fn finished<'a>(&'a self, _run: &'a LaunchRun) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

/// Called after a `Spawn` step started a process: the process id and its creation time.
pub type OnSpawned = Arc<dyn Fn(u32, u64) + Send + Sync>;

/// Makes the text of a failed step from the error of the start call.
pub type ErrorText = Arc<dyn Fn(&OpenError) -> String + Send + Sync>;

pub struct LaunchPlan {
    pub steps: Vec<PlannedStep>,
}

pub struct PlannedStep {
    pub label: String,
    pub icon: IconRef,
    pub action: StepAction,
    pub delay_ms: u32,
    /// A step that is known to fail before it starts (the project folder is missing).
    pub preset_error: Option<String>,
    /// A failure here makes the rest of the run pointless.
    pub stops_run: bool,
    /// Called by the engine after a `Spawn` step started a process, with the process id and its
    /// creation time (a Windows FILETIME).
    pub on_spawned: Option<OnSpawned>,
    /// Used when the main start (a `Spawn`) fails with os error 5, 1920 or 740: the same
    /// program is then started through `ShellExecuteExW` with these values.
    pub shell_fallback: Option<ShellFallback>,
    /// Replaces the text the engine would take from its own table for a failed step, for texts
    /// that only the planner knows ("Program file not found: <path>").
    pub error_text: Option<ErrorText>,
    /// Where the window of this step goes once the step has started. `None` leaves it where
    /// Windows puts it.
    pub placement: Option<Placement>,
}

/// Where the window of a step goes, and how to tell which window the step made. Plain data: the
/// rules that use it are in `core::placement`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub zone: LayoutZone,
    /// 1 is the monitor at the far left. `None`: the monitor that Rain's window is on.
    pub monitor: Option<u32>,
    pub find: WindowQuery,
}

/// How the window of a step is told from the other windows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WindowQuery {
    /// Image names (lower case, with `.exe`) of the programs that may own the window. The
    /// started process itself always counts, whatever its name. Without names and with a title
    /// rule, any program may own the window.
    pub images: Vec<String>,
    /// The window title must hold this text (lower case). Needed for a window of a program that
    /// owns many windows.
    pub title_has: Option<String>,
    /// A window that is open already may be the one (VS Code with the folder open: the project
    /// is being opened, so its own window is placed). Needs `title_has`.
    pub allow_existing: bool,
}

pub struct ShellFallback {
    pub file: String,
    pub params: Option<String>,
    pub dir: Option<String>,
}

pub enum StepAction {
    App {
        app_id: String,
        args: Option<String>,
    },
    ShellExec {
        file: String,
        params: Option<String>,
        dir: Option<String>,
    },
    Spawn {
        exe: PathBuf,
        args_raw: String,
        cwd: PathBuf,
    },
    /// A step that is already satisfied (a project that is already open): success at once.
    Nothing,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rain_types::{IconKind, SearchType};

    struct OneItem;

    impl ItemSource for OneItem {
        fn search_entries(&self) -> Vec<(SearchItem, Vec<String>)> {
            Vec::new()
        }

        fn item_for(&self, id: &str) -> Option<SearchItem> {
            (id == "known").then(|| SearchItem {
                r#type: SearchType::Project,
                id: id.to_owned(),
                title: "Known".into(),
                subtitle: None,
                icon: IconRef {
                    kind: IconKind::Letter,
                    value: "K".into(),
                    tint: None,
                    fallback: None,
                },
                status: None,
                score: 0,
            })
        }

        fn recent(&self) -> Vec<(String, SearchItem)> {
            Vec::new()
        }
    }

    #[test]
    fn knows_follows_item_for_unless_the_owner_says_otherwise() {
        assert!(OneItem.knows("known"));
        assert!(!OneItem.knows("gone"));
    }
}
