//! The launch engine: how a launch target becomes steps, how one step starts a program, and the
//! registry of runs. Section 01 6.8.
//!
//! This file has no state of its own except the registry, and it never publishes a frame or
//! touches SQLite: `service.rs` owns the run task and the order of the end of a run. What is
//! here can be tested with a fake `Opener` and a clock that a test sets.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use rain_types::{
    AppKind, IconRef, LaunchRun, LaunchStep, LaunchTarget, LaunchType, RunStatus, Status,
};
use tracing::{debug, warn};

use super::apps::AppRecord;
use super::limits::{EXISTS_TIMEOUT, RUN_KEEP, RUN_MAX, STEP_TIMEOUT};
use crate::core::blocking;
use crate::core::launch_contract::{
    ErrorText, OnSpawned, OpenError, OpenRequest, Opener, Placement, PlannedStep, ShellFallback,
    Started, StepAction,
};

// ---- what a step does ----

/// What a step starts, with the app already looked up.
pub enum Source {
    /// An app of the list, with the extra arguments of this launch.
    App {
        app: Box<AppRecord>,
        args: Option<String>,
    },
    /// A request that a planner (projects, games) made.
    Open(OpenRequest),
    /// Already satisfied: success at once.
    Nothing,
}

/// One step of a run, ready to execute.
pub struct EngineStep {
    pub label: String,
    pub icon: IconRef,
    pub source: Source,
    pub delay_ms: u32,
    /// The step is known to fail before it starts; this is its text.
    pub preset_error: Option<String>,
    pub stops_run: bool,
    pub on_spawned: Option<OnSpawned>,
    pub shell_fallback: Option<ShellFallback>,
    pub error_text: Option<ErrorText>,
    pub placement: Option<Placement>,
}

impl EngineStep {
    /// The step that starts one app.
    pub fn for_app(app: &AppRecord, args: Option<String>, delay_ms: u32) -> Self {
        Self {
            label: app.name.clone(),
            icon: app.icon(),
            source: Source::App {
                app: Box::new(app.clone()),
                args,
            },
            delay_ms,
            preset_error: None,
            stops_run: false,
            on_spawned: None,
            shell_fallback: None,
            error_text: None,
            placement: None,
        }
    }

    /// A pool member whose app is gone or hidden: it fails at once and the pool goes on.
    pub fn unknown_app() -> Self {
        Self {
            label: "Unknown app".to_owned(),
            icon: IconRef::lucide("app-window"),
            source: Source::Nothing,
            delay_ms: 0,
            preset_error: Some("App not found".to_owned()),
            stops_run: false,
            on_spawned: None,
            shell_fallback: None,
            error_text: None,
            placement: None,
        }
    }

    /// The step of a plan that a project or a game made. `find` looks up an app by id.
    pub fn from_planned(planned: PlannedStep, find: impl Fn(&str) -> Option<AppRecord>) -> Self {
        let PlannedStep {
            label,
            icon,
            action,
            delay_ms,
            mut preset_error,
            stops_run,
            on_spawned,
            shell_fallback,
            error_text,
            placement,
        } = planned;
        let source = match action {
            StepAction::App { app_id, args } => match find(&app_id) {
                Some(app) => Source::App {
                    app: Box::new(app),
                    args,
                },
                None => {
                    preset_error.get_or_insert_with(|| "App not found".to_owned());
                    Source::Nothing
                }
            },
            StepAction::ShellExec { file, params, dir } => {
                Source::Open(OpenRequest::ShellExec { file, params, dir })
            }
            StepAction::Spawn { exe, args_raw, cwd } => {
                Source::Open(OpenRequest::Spawn { exe, args_raw, cwd })
            }
            StepAction::Nothing => Source::Nothing,
        };
        Self {
            label,
            icon,
            source,
            delay_ms,
            preset_error,
            stops_run,
            on_spawned,
            shell_fallback,
            error_text,
            placement,
        }
    }
}

// ---- how an app is started: the decision table of 6.8.1 ----

/// When the second request is tried after the first one failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    /// Whatever the error was (a packaged app that activation could not start).
    Any,
    /// ShellExecute refused with a `SE_ERR_*` value (32 or less).
    ShellRefused,
    /// The start was refused for rights: access denied, or the program needs administrator
    /// rights (the codes 5, 1920 and 740 of section 01 6.8).
    Rights,
}

impl When {
    pub fn applies(self, error: &OpenError) -> bool {
        match (self, error.code) {
            (Self::Any, _) => true,
            (Self::ShellRefused, Some(code)) => code <= 32,
            (Self::Rights, Some(code)) => matches!(code, 5 | 1920 | 740),
            _ => false,
        }
    }
}

/// The request for an app, and the one to try when it fails.
#[derive(Debug, PartialEq, Eq)]
pub struct AppRequests {
    pub primary: OpenRequest,
    pub fallback: Option<(When, OpenRequest)>,
}

fn apps_folder_item(aumid: &str) -> OpenRequest {
    OpenRequest::ShellExec {
        file: format!(r"shell:AppsFolder\{aumid}"),
        params: None,
        dir: None,
    }
}

/// The table of section 01 6.8.1:
///
/// | kind | arguments | call | fallback |
/// |---|---|---|---|
/// | store | any | activate by AUMID | the same item through `shell:AppsFolder` |
/// | program | none | the `shell:AppsFolder` item (keeps the shortcut's own arguments, folder and run-as) | the target, with the link's arguments |
/// | program | some | the target with the link's arguments and these | none |
/// | other | any | the `shell:AppsFolder` item (arguments are ignored: nothing can take them) | none |
pub fn requests_for(app: &AppRecord, args: Option<&str>) -> AppRequests {
    let args = args.map(str::trim).filter(|args| !args.is_empty());
    match app.kind {
        AppKind::Store => AppRequests {
            primary: OpenRequest::Activate {
                aumid: app.aumid.clone(),
                args: args.map(str::to_owned),
            },
            fallback: Some((When::Any, apps_folder_item(&app.aumid))),
        },
        AppKind::Other => AppRequests {
            primary: apps_folder_item(&app.aumid),
            fallback: None,
        },
        AppKind::Program => {
            let link_args = (!app.link_args.trim().is_empty()).then(|| app.link_args.clone());
            let dir = Path::new(&app.exe_path)
                .parent()
                .map(|dir| dir.to_string_lossy().into_owned())
                .filter(|dir| !dir.is_empty());
            let direct = |params: Option<String>| OpenRequest::ShellExec {
                file: app.exe_path.clone(),
                params,
                dir: dir.clone(),
            };
            match args {
                Some(args) => {
                    let params = match &link_args {
                        Some(link) => format!("{link} {args}"),
                        None => args.to_owned(),
                    };
                    AppRequests {
                        primary: direct(Some(params)),
                        fallback: None,
                    }
                }
                None if app.aumid.is_empty() => AppRequests {
                    primary: direct(link_args),
                    fallback: None,
                },
                None => AppRequests {
                    primary: apps_folder_item(&app.aumid),
                    fallback: Some((When::ShellRefused, direct(link_args))),
                },
            }
        }
    }
}

// ---- the existence check ----

/// Whether a program file is there. A trait, so that a test says which files "exist" (the apps of
/// a test have made-up paths).
pub trait PathProbe: Send + Sync {
    /// `Some(false)` only when the file is definitely not there; `None` when it cannot be told.
    fn exists(&self, path: &Path) -> Option<bool>;
}

pub struct DiskProbe;

impl PathProbe for DiskProbe {
    fn exists(&self, path: &Path) -> Option<bool> {
        match std::fs::metadata(path) {
            Ok(_) => Some(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(false),
            Err(_) => None,
        }
    }
}

// ---- starting one step ----

/// What a step needs from outside while it runs.
pub struct StepEnv {
    pub opener: Arc<dyn Opener>,
    pub probe: Arc<dyn PathProbe>,
}

const NO_ANSWER: &str = "Did not respond";

/// One start call with its time limit. A call that does not return is reported as an error with
/// no code; later COM calls may then answer `comBusy` (foundation 11.6).
async fn open_in_time(opener: &dyn Opener, request: OpenRequest) -> Result<Started, OpenError> {
    match tokio::time::timeout(STEP_TIMEOUT, opener.open(request)).await {
        Ok(result) => result,
        Err(_) => {
            warn!("a start call did not return in time");
            Err(OpenError {
                ui_text: NO_ANSWER,
                code: None,
                log_reason: "the start call did not return in time".to_owned(),
            })
        }
    }
}

/// Runs one step: the pre-set error, the delay, the existence check, the start with its
/// fallback. `Err` is the text for `LaunchStep.error`.
pub async fn run_step(env: &StepEnv, step: &EngineStep) -> Result<Started, String> {
    if let Some(error) = &step.preset_error {
        return Err(error.clone());
    }
    if step.delay_ms > 0 {
        tokio::time::sleep(Duration::from_millis(u64::from(step.delay_ms))).await;
    }
    let (primary, mut fallback) = match &step.source {
        Source::Nothing => return Ok(Started::default()),
        Source::Open(request) => (request.clone(), None),
        Source::App { app, args } => {
            if app.kind == AppKind::Program && program_is_missing(env, &app.exe_path).await {
                return Err("Executable not found".to_owned());
            }
            let requests = requests_for(app, args.as_deref());
            (requests.primary, requests.fallback)
        }
    };
    // The planner's own fallback (a refused direct start becomes a ShellExecuteEx).
    if fallback.is_none()
        && let Some(shell) = &step.shell_fallback
    {
        fallback = Some((
            When::Rights,
            OpenRequest::ShellExec {
                file: shell.file.clone(),
                params: shell.params.clone(),
                dir: shell.dir.clone(),
            },
        ));
    }
    let was_spawn = matches!(primary, OpenRequest::Spawn { .. });
    let mut result = open_in_time(env.opener.as_ref(), primary).await;
    if let (Err(first), Some((when, second))) = (&result, fallback)
        && when.applies(first)
    {
        // The first error stays in the log; the second call decides the step.
        debug!(reason = %first.log_reason, "the first start failed; trying the fallback");
        result = open_in_time(env.opener.as_ref(), second).await;
    }
    match result {
        Ok(started) => {
            if was_spawn
                && let (Some(callback), Some(pid), Some(created)) =
                    (&step.on_spawned, started.pid, started.creation_time)
            {
                callback(pid, created);
            }
            Ok(started)
        }
        Err(error) => {
            warn!(reason = %error.log_reason, "a launch step failed");
            Err(match &step.error_text {
                Some(make) => make(&error),
                None => error.ui_text.to_owned(),
            })
        }
    }
}

/// True only when the file is definitely not there. A slow or failing check does not block the
/// start (a network drive may be slow, and the shell knows better).
async fn program_is_missing(env: &StepEnv, exe_path: &str) -> bool {
    if exe_path.is_empty() {
        return false;
    }
    let (probe, path) = (Arc::clone(&env.probe), exe_path.to_owned());
    let check = blocking::run(move || probe.exists(Path::new(&path)));
    matches!(
        tokio::time::timeout(EXISTS_TIMEOUT, check).await,
        Ok(Ok(Some(false)))
    )
}

// ---- the registry of runs ----

struct Entry {
    run: LaunchRun,
    /// When the run ended; `None` while it runs.
    ended: Option<SystemTime>,
}

/// The runs that `GET /launches/{id}` can still answer: running ones, and finished ones for
/// `RUN_KEEP`, at most `RUN_MAX`. Eviction happens at insert and at read time; no timer.
#[derive(Default)]
pub struct Registry {
    runs: HashMap<String, Entry>,
}

impl Registry {
    /// Adds a run. Finished runs that are too old go first; when the registry is still full the
    /// oldest finished run goes. A running run is never evicted.
    pub fn insert(&mut self, run: LaunchRun, now: SystemTime) {
        self.evict_old(now);
        while self.runs.len() >= RUN_MAX {
            let oldest = self
                .runs
                .iter()
                .filter_map(|(id, entry)| entry.ended.map(|ended| (ended, id.clone())))
                .min();
            match oldest {
                Some((_, id)) => {
                    self.runs.remove(&id);
                }
                None => break, // everything is still running
            }
        }
        self.runs.insert(run.id.clone(), Entry { run, ended: None });
    }

    fn evict_old(&mut self, now: SystemTime) {
        self.runs.retain(|_, entry| {
            entry
                .ended
                .is_none_or(|ended| now.duration_since(ended).unwrap_or_default() < RUN_KEEP)
        });
    }

    pub fn get(&mut self, id: &str, now: SystemTime) -> Option<LaunchRun> {
        self.evict_old(now);
        self.runs.get(id).map(|entry| entry.run.clone())
    }

    /// The run of this target that is still running.
    pub fn running(&self, target: &LaunchTarget) -> Option<LaunchRun> {
        self.runs
            .values()
            .find(|entry| {
                entry.run.status == RunStatus::Running
                    && entry.run.target.r#type == target.r#type
                    && entry.run.target.id == target.id
            })
            .map(|entry| entry.run.clone())
    }

    /// Changes a run and returns the new snapshot.
    pub fn update(&mut self, id: &str, change: impl FnOnce(&mut LaunchRun)) -> Option<LaunchRun> {
        let entry = self.runs.get_mut(id)?;
        change(&mut entry.run);
        Some(entry.run.clone())
    }

    /// Marks the run as ended with its last snapshot.
    pub fn finish(&mut self, run: &LaunchRun, now: SystemTime) {
        if let Some(entry) = self.runs.get_mut(&run.id) {
            entry.run = run.clone();
            entry.ended = Some(now);
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.runs.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }
}

/// The run as the `201` answer shows it: step 0 running, the others idle.
pub fn new_run(id: String, target: &LaunchTarget, steps: &[EngineStep]) -> LaunchRun {
    LaunchRun {
        id,
        target: LaunchTarget {
            r#type: target.r#type,
            id: target.id.clone(),
            mode: target.mode,
            args: target.args.clone(),
        },
        status: RunStatus::Running,
        steps: steps
            .iter()
            .enumerate()
            .map(|(i, step)| LaunchStep {
                label: step.label.clone(),
                icon: step.icon.clone(),
                status: if i == 0 {
                    Status::Running
                } else {
                    Status::Idle
                },
                error: None,
                note: None,
            })
            .collect(),
    }
}

/// The kind that history and "last launched" are kept under; projects and games keep their own.
pub fn is_launcher_history(kind: LaunchType) -> bool {
    matches!(kind, LaunchType::App | LaunchType::Pool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::time::UNIX_EPOCH;

    use futures_util::future::BoxFuture;

    fn app(kind: AppKind, aumid: &str, exe: &str, link_args: &str) -> AppRecord {
        AppRecord {
            id: "app-x".into(),
            key: "k".into(),
            aumid: aumid.into(),
            name: "X".into(),
            kind,
            exe_path: exe.into(),
            link_args: link_args.into(),
            keywords: Vec::new(),
            icon_version: "00000000".into(),
            icon_file: "x.png".into(),
        }
    }

    // ---- the decision table ----

    #[test]
    fn a_store_app_is_activated_and_falls_back_to_the_apps_folder_item() {
        let requests = requests_for(
            &app(AppKind::Store, "Fake.App_abc!App", "", ""),
            Some(" --x "),
        );
        assert_eq!(
            requests.primary,
            OpenRequest::Activate {
                aumid: "Fake.App_abc!App".into(),
                args: Some("--x".into())
            }
        );
        let (when, second) = requests.fallback.expect("a fallback");
        assert_eq!(when, When::Any);
        assert_eq!(
            second,
            OpenRequest::ShellExec {
                file: r"shell:AppsFolder\Fake.App_abc!App".into(),
                params: None,
                dir: None
            }
        );
    }

    #[test]
    fn a_program_without_arguments_starts_the_shortcut_item_and_falls_back_to_the_target() {
        let requests = requests_for(
            &app(
                AppKind::Program,
                "{GUID}\\Fake\\tool.exe",
                r"C:\Fake\tool.exe",
                "--link",
            ),
            None,
        );
        assert_eq!(
            requests.primary,
            OpenRequest::ShellExec {
                file: r"shell:AppsFolder\{GUID}\Fake\tool.exe".into(),
                params: None,
                dir: None
            }
        );
        let (when, second) = requests.fallback.expect("a fallback");
        assert_eq!(when, When::ShellRefused);
        assert_eq!(
            second,
            OpenRequest::ShellExec {
                file: r"C:\Fake\tool.exe".into(),
                params: Some("--link".into()),
                dir: Some(r"C:\Fake".into())
            }
        );
    }

    #[test]
    fn a_program_with_arguments_starts_the_target_with_the_exact_text() {
        let with_link = requests_for(
            &app(AppKind::Program, "a", r"C:\Fake\tool.exe", "--link"),
            Some("--new-window http://localhost:5173"),
        );
        assert_eq!(
            with_link.primary,
            OpenRequest::ShellExec {
                file: r"C:\Fake\tool.exe".into(),
                params: Some("--link --new-window http://localhost:5173".into()),
                dir: Some(r"C:\Fake".into())
            }
        );
        assert!(with_link.fallback.is_none());
        let plain = requests_for(
            &app(AppKind::Program, "a", r"C:\Fake\tool.exe", ""),
            Some("-x"),
        );
        assert!(matches!(
            plain.primary,
            OpenRequest::ShellExec { params: Some(ref p), .. } if p == "-x"
        ));
    }

    #[test]
    fn a_program_without_an_aumid_starts_its_target_directly() {
        let requests = requests_for(&app(AppKind::Program, "", r"C:\Fake\tool.exe", ""), None);
        assert_eq!(
            requests.primary,
            OpenRequest::ShellExec {
                file: r"C:\Fake\tool.exe".into(),
                params: None,
                dir: Some(r"C:\Fake".into())
            }
        );
    }

    #[test]
    fn an_other_item_ignores_arguments() {
        let with = requests_for(&app(AppKind::Other, "{GUID}", "", ""), Some("-x"));
        let without = requests_for(&app(AppKind::Other, "{GUID}", "", ""), None);
        assert_eq!(with, without);
        assert!(with.fallback.is_none());
    }

    #[test]
    fn the_fallback_conditions() {
        let error = |code| OpenError {
            ui_text: "x",
            code,
            log_reason: String::new(),
        };
        assert!(When::Any.applies(&error(None)));
        assert!(When::ShellRefused.applies(&error(Some(31))));
        assert!(!When::ShellRefused.applies(&error(Some(740))));
        assert!(!When::ShellRefused.applies(&error(None)));
        for code in [5, 1920, 740] {
            assert!(When::Rights.applies(&error(Some(code))));
        }
        assert!(!When::Rights.applies(&error(Some(2))));
    }

    // ---- running a step ----

    struct Script {
        requests: Mutex<Vec<OpenRequest>>,
        answers: Mutex<VecDeque<Result<Started, OpenError>>>,
    }

    impl Script {
        fn new(answers: Vec<Result<Started, OpenError>>) -> Arc<Self> {
            Arc::new(Self {
                requests: Mutex::default(),
                answers: Mutex::new(answers.into()),
            })
        }

        fn seen(&self) -> Vec<OpenRequest> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl Opener for Script {
        fn open(&self, request: OpenRequest) -> BoxFuture<'_, Result<Started, OpenError>> {
            self.requests.lock().unwrap().push(request);
            let answer = self
                .answers
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok(Started::default()));
            Box::pin(async move { answer })
        }
    }

    struct Probe(Option<bool>);

    impl PathProbe for Probe {
        fn exists(&self, _path: &Path) -> Option<bool> {
            self.0
        }
    }

    fn env(script: &Arc<Script>, exists: Option<bool>) -> StepEnv {
        StepEnv {
            opener: script.clone(),
            probe: Arc::new(Probe(exists)),
        }
    }

    fn refused(code: u32, text: &'static str) -> Result<Started, OpenError> {
        Err(OpenError {
            ui_text: text,
            code: Some(code),
            log_reason: "test".into(),
        })
    }

    fn program_step() -> EngineStep {
        EngineStep::for_app(
            &app(AppKind::Program, "x", r"C:\Fake\tool.exe", ""),
            None,
            0,
        )
    }

    #[tokio::test]
    async fn a_missing_program_file_fails_the_step_without_any_call() {
        let script = Script::new(vec![]);
        let result = run_step(&env(&script, Some(false)), &program_step()).await;
        assert_eq!(result.unwrap_err(), "Executable not found");
        assert!(script.seen().is_empty(), "no shell call, so no dialog");
    }

    #[tokio::test]
    async fn an_unknown_answer_of_the_existence_check_goes_on_to_the_start() {
        let script = Script::new(vec![]);
        assert!(run_step(&env(&script, None), &program_step()).await.is_ok());
        assert_eq!(script.seen().len(), 1);
    }

    #[tokio::test]
    async fn a_refused_shortcut_start_tries_the_target_and_the_second_call_decides() {
        let script = Script::new(vec![
            refused(31, "No program for this file"),
            Ok(Started::default()),
        ]);
        assert!(
            run_step(&env(&script, Some(true)), &program_step())
                .await
                .is_ok()
        );
        assert_eq!(script.seen().len(), 2);

        let script = Script::new(vec![
            refused(31, "No program for this file"),
            refused(5, "Access denied"),
        ]);
        let error = run_step(&env(&script, Some(true)), &program_step())
            .await
            .unwrap_err();
        assert_eq!(error, "Access denied");
    }

    #[tokio::test]
    async fn an_error_that_the_fallback_does_not_cover_ends_the_step_at_once() {
        let script = Script::new(vec![refused(1223, "Cancelled")]);
        let error = run_step(&env(&script, Some(true)), &program_step())
            .await
            .unwrap_err();
        assert_eq!(error, "Cancelled");
        assert_eq!(script.seen().len(), 1);
    }

    #[tokio::test]
    async fn a_preset_error_fails_before_anything_else() {
        let script = Script::new(vec![]);
        let step = EngineStep::unknown_app();
        assert_eq!(
            run_step(&env(&script, Some(true)), &step)
                .await
                .unwrap_err(),
            "App not found"
        );
        assert!(script.seen().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn the_delay_is_waited_before_the_start() {
        let script = Script::new(vec![]);
        let mut step = EngineStep::for_app(&app(AppKind::Other, "{G}", "", ""), None, 2000);
        step.delay_ms = 2000;
        let started = tokio::time::Instant::now();
        run_step(&env(&script, Some(true)), &step).await.unwrap();
        assert!(started.elapsed() >= Duration::from_millis(2000));
    }

    #[tokio::test(start_paused = true)]
    async fn a_start_that_does_not_return_in_time_says_so() {
        struct Never;
        impl Opener for Never {
            fn open(&self, _r: OpenRequest) -> BoxFuture<'_, Result<Started, OpenError>> {
                Box::pin(std::future::pending())
            }
        }
        let env = StepEnv {
            opener: Arc::new(Never),
            probe: Arc::new(Probe(Some(true))),
        };
        let step = EngineStep::for_app(&app(AppKind::Other, "{G}", "", ""), None, 0);
        assert_eq!(run_step(&env, &step).await.unwrap_err(), "Did not respond");
    }

    #[tokio::test]
    async fn a_planned_spawn_calls_back_with_the_process_and_uses_its_shell_fallback() {
        let script = Script::new(vec![
            refused(740, "Needs administrator rights"),
            Ok(Started::default()),
        ]);
        let spawn = OpenRequest::Spawn {
            exe: r"C:\Fake\game.exe".into(),
            args_raw: String::new(),
            cwd: r"C:\Fake".into(),
        };
        let mut step = EngineStep::from_planned(
            PlannedStep {
                label: "Game".into(),
                icon: IconRef::letter("G"),
                action: StepAction::Spawn {
                    exe: r"C:\Fake\game.exe".into(),
                    args_raw: String::new(),
                    cwd: r"C:\Fake".into(),
                },
                delay_ms: 0,
                preset_error: None,
                stops_run: false,
                on_spawned: None,
                shell_fallback: Some(ShellFallback {
                    file: r"C:\Fake\game.exe".into(),
                    params: None,
                    dir: Some(r"C:\Fake".into()),
                }),
                error_text: None,
                placement: None,
            },
            |_| None,
        );
        assert!(run_step(&env(&script, None), &step).await.is_ok());
        let seen = script.seen();
        assert_eq!(seen[0], spawn);
        assert!(matches!(seen[1], OpenRequest::ShellExec { .. }));

        // The callback runs after a successful spawn that knows the process.
        let called = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&called);
        step.on_spawned = Some(Arc::new(move |pid, time| {
            sink.lock().unwrap().push((pid, time))
        }));
        step.shell_fallback = None;
        let script = Script::new(vec![Ok(Started {
            pid: Some(7),
            creation_time: Some(99),
            in_job: false,
        })]);
        run_step(&env(&script, None), &step).await.unwrap();
        assert_eq!(*called.lock().unwrap(), [(7, 99)]);
    }

    #[tokio::test]
    async fn the_planner_may_replace_the_text_of_a_failed_step() {
        let script = Script::new(vec![refused(2, "Executable not found")]);
        let mut step = EngineStep::from_planned(
            PlannedStep {
                label: "Editor".into(),
                icon: IconRef::letter("E"),
                action: StepAction::ShellExec {
                    file: "x".into(),
                    params: None,
                    dir: None,
                },
                delay_ms: 0,
                preset_error: None,
                stops_run: true,
                on_spawned: None,
                shell_fallback: None,
                error_text: None,
                placement: None,
            },
            |_| None,
        );
        step.error_text = Some(Arc::new(|e: &OpenError| {
            format!("Planner says: {}", e.ui_text)
        }));
        assert_eq!(
            run_step(&env(&script, None), &step).await.unwrap_err(),
            "Planner says: Executable not found"
        );
    }

    #[test]
    fn a_planned_app_step_that_names_an_unknown_app_fails_with_its_text() {
        let step = EngineStep::from_planned(
            PlannedStep {
                label: "Terminal".into(),
                icon: IconRef::letter("T"),
                action: StepAction::App {
                    app_id: "app-gone".into(),
                    args: None,
                },
                delay_ms: 0,
                preset_error: None,
                stops_run: false,
                on_spawned: None,
                shell_fallback: None,
                error_text: None,
                placement: None,
            },
            |_| None,
        );
        assert_eq!(step.preset_error.as_deref(), Some("App not found"));
    }

    // ---- the registry ----

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn run(id: &str, target: &str) -> LaunchRun {
        LaunchRun {
            id: id.into(),
            target: LaunchTarget {
                r#type: LaunchType::App,
                id: target.into(),
                mode: None,
                args: None,
            },
            status: RunStatus::Running,
            steps: Vec::new(),
        }
    }

    fn finished(registry: &mut Registry, id: &str, now: SystemTime) {
        let mut done = registry.get(id, now).expect("the run is there");
        done.status = RunStatus::Success;
        registry.finish(&done, now);
    }

    #[test]
    fn a_running_run_is_found_by_its_target_and_a_finished_one_is_not() {
        let mut registry = Registry::default();
        registry.insert(run("r1", "app-a"), at(0));
        let target = LaunchTarget {
            r#type: LaunchType::App,
            id: "app-a".into(),
            mode: None,
            args: None,
        };
        assert_eq!(registry.running(&target).map(|r| r.id), Some("r1".into()));
        finished(&mut registry, "r1", at(1));
        assert!(registry.running(&target).is_none());
    }

    #[test]
    fn a_finished_run_goes_after_ten_minutes_and_a_running_one_never() {
        let mut registry = Registry::default();
        registry.insert(run("done", "a"), at(0));
        registry.insert(run("busy", "b"), at(0));
        finished(&mut registry, "done", at(0));
        assert!(registry.get("done", at(599)).is_some());
        assert!(registry.get("done", at(601)).is_none());
        assert!(registry.get("busy", at(100_000)).is_some());
    }

    #[test]
    fn the_sixty_fifth_run_evicts_the_oldest_finished_one() {
        let mut registry = Registry::default();
        for n in 0..RUN_MAX {
            let id = format!("r{n}");
            registry.insert(run(&id, &id), at(n as u64));
            finished(&mut registry, &id, at(n as u64));
        }
        assert_eq!(registry.len(), RUN_MAX);
        registry.insert(run("new", "new"), at(100));
        assert_eq!(registry.len(), RUN_MAX);
        assert!(registry.get("r0", at(100)).is_none());
        assert!(registry.get("r1", at(100)).is_some());
        assert!(registry.get("new", at(100)).is_some());
    }

    #[test]
    fn when_everything_still_runs_nothing_is_evicted() {
        let mut registry = Registry::default();
        for n in 0..=RUN_MAX {
            registry.insert(run(&format!("r{n}"), "x"), at(0));
        }
        assert_eq!(registry.len(), RUN_MAX + 1);
        assert!(!registry.is_empty());
    }

    #[test]
    fn a_new_run_has_step_zero_running_and_the_others_idle() {
        let steps = [
            EngineStep::unknown_app(),
            EngineStep::unknown_app(),
            EngineStep::unknown_app(),
        ];
        let target = LaunchTarget {
            r#type: LaunchType::Pool,
            id: "pool-a".into(),
            mode: None,
            args: None,
        };
        let run = new_run("run-1".into(), &target, &steps);
        let statuses: Vec<Status> = run.steps.iter().map(|s| s.status).collect();
        assert_eq!(statuses, [Status::Running, Status::Idle, Status::Idle]);
        assert_eq!(run.status, RunStatus::Running);
        assert!(is_launcher_history(LaunchType::Pool) && !is_launcher_history(LaunchType::Game));
    }
}
