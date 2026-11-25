//! The logic and the in-memory state of the projects module. Other modules call it, and only it.
//! Section 02 6.2 to 6.7 and 6.15.
//!
//! How a list is made (`list`):
//! 1. Discovery finds the repositories and reads their branches from the files (cheap).
//! 2. The database remembers what was seen (ids, missing projects) and holds the git facts.
//! 3. A fact that is not cached yet is computed now, but the answer waits at most a few seconds;
//!    a fact that is only stale is refreshed in the background. A refresh that changed something
//!    publishes the `projects` topic, so an open screen follows without a click.
//! 4. Open detection looks at the editor windows (cached for 2 seconds).
//!
//! Nothing runs on a timer: every step starts with a request or with a change of a setting or of
//! `projects.toml`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use futures_util::future::{BoxFuture, join_all};
use rain_types::{
    EnvKind, EnvStep, IconKind, IconRef, LAYOUT_ITEMS_MAX, LAYOUT_MONITOR_MAX, LaunchRun,
    LaunchTarget, LaunchType, LayoutSave, LayoutStepOption, LayoutWhat, PinnableType, Project,
    ProjectLayout, Projects, SearchItem, SearchType, SourceState, Status,
};
use tokio::sync::{Semaphore, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::dates;
use super::discovery::{Discovery, discover};
use super::env::{self, Analysis, Builtins, ProjectsFile, StepSpec};
use super::facts::{self, Moment, Needed, Target};
use super::git::{self, GitRunner, SystemGit};
use super::ids::{path_key, project_id};
use super::launch::{self, PlanInput, WtOptions};
use super::layout_file::{self, Scope};
use super::open::{self, EditorWindow};
use super::store::{self, Found, GitRow, Snapshot};
use super::tools::{self, Lookup as ToolFound, SystemToolEnv, Tool, ToolEnv};
use crate::core::blocking;
use crate::core::error::{ApiError, ApiResult};
use crate::core::launch_contract::{ItemSource, LaunchPlan, LaunchPlanner};
use crate::core::state::{AppState, WeakAppState};
use crate::core::task::spawn_named;
use crate::core::throttle::Throttle;
use crate::modules::launcher::AppFacts;
use crate::platform::process::{ProcessLister, SystemProcessLister};
use crate::platform::windows::{SystemWindowLister, WindowLister};
use crate::store::toml_file::TomlFile;

/// The name of this module on its errors and in `/health`.
const SOURCE: &str = "projects";
/// The file in the data folder.
const FILE_NAME: &str = "projects.toml";

/// A list that waits for facts that are not cached yet waits at most this long (section 6.3).
const FIRST_FACTS_BUDGET: Duration = Duration::from_secs(6);
/// `POST /projects/refresh` waits at most this long.
const REFRESH_WAIT: Duration = Duration::from_secs(8);
/// A discovery that takes longer (a network drive) is abandoned; the last one is used.
const DISCOVERY_BUDGET: Duration = Duration::from_secs(8);
/// How long the list of editor windows is reused.
const OPEN_TTL: Duration = Duration::from_secs(2);
/// At most this many git processes at once.
const GIT_MAX_PARALLEL: usize = 4;
/// The `projects` topic is published at most this often per second.
const TOPIC_RATE_PER_SECOND: u32 = 1;

/// What a list does about git facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refresh {
    /// Compute what is missing, refresh what is stale in the background.
    Normal,
    /// Compute everything again and wait (`POST /projects/refresh`).
    Force,
    /// Use the cache only (game mode, and the publish after a refresh).
    CacheOnly,
}

pub struct Service {
    state: WeakAppState,
    file: TomlFile<ProjectsFile>,
    git: Arc<dyn GitRunner>,
    tools: Arc<dyn ToolEnv>,
    windows: Arc<dyn WindowLister>,
    processes: Arc<dyn ProcessLister>,
    throttle: Throttle<Projects>,
    git_slots: Semaphore,
    /// Ends the background work at `stop`.
    cancel: CancellationToken,
    memory: Mutex<Memory>,
    /// The projects (by `path_key`) whose facts are being computed now.
    in_flight: Mutex<HashSet<String>>,
}

#[derive(Default)]
struct Memory {
    /// The last answer.
    list: Option<Arc<Vec<Project>>>,
    /// The ids that were given, by `path_key`. Kept so that the parts that cannot wait for the
    /// database (search, a launch plan) answer with the same ids.
    ids: HashMap<String, String>,
    /// The editor windows and when they were listed.
    windows: Option<(Instant, Arc<Vec<EditorWindow>>)>,
    /// The last discovery that finished, for the case that a new one takes too long.
    discovery: Option<Arc<Vec<Found>>>,
}

/// What one list works from.
struct Gathered {
    snapshot: Snapshot,
    analysis: Analysis,
    builtins: Builtins,
    editor: ToolFound,
    git: ToolFound,
    stamp: Stamp,
    pins: HashSet<String>,
}

/// The moment of one list, in the forms that the facts need.
#[derive(Clone)]
struct Stamp {
    iso: String,
    /// Local date, `YYYY-MM-DD`.
    date: String,
    /// Local day number.
    day: i64,
    ms: i64,
}

impl Stamp {
    fn of(now: std::time::SystemTime) -> Self {
        let date = crate::platform::time::local_date(now);
        Self {
            iso: rain_types::time::iso_from_system_time(now),
            day: dates::parse_date(&date).unwrap_or(0),
            date,
            ms: git::unix_seconds(now) * 1000,
        }
    }
}

/// One repository whose facts are computed.
struct Job {
    found: Found,
    id: String,
    old: Option<GitRow>,
    needed: Needed,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn letter(name: &str) -> IconRef {
    IconRef {
        kind: IconKind::Letter,
        value: name.to_owned(),
        tint: None,
        fallback: None,
    }
}

/// "VS Code" for VS Code (and its Insiders build), else the program's name with a capital.
fn editor_label(found: &ToolFound) -> String {
    let stem = match found {
        ToolFound::At(path) | ToolFound::ConfiguredMissing(path) => {
            path.file_stem().map(|s| s.to_string_lossy().into_owned())
        }
        ToolFound::NotFound => None,
    };
    match stem.as_deref().map(str::to_lowercase).as_deref() {
        None | Some("code" | "code - insiders") => "VS Code".to_owned(),
        Some(_) => {
            let stem = stem.unwrap_or_default();
            let mut letters = stem.chars();
            letters
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + letters.as_str())
                .unwrap_or_default()
        }
    }
}

/// The environment of a project as the detail pane lists it.
fn env_steps(specs: &[StepSpec]) -> Vec<EnvStep> {
    specs
        .iter()
        .map(|spec| EnvStep {
            kind: spec.kind,
            label: spec.label.clone(),
            icon: spec.icon.clone(),
            command: match spec.kind {
                EnvKind::Terminal => spec.command.clone(),
                EnvKind::Claude => Some("claude".to_owned()),
                _ => None,
            },
            url: spec.url.clone(),
        })
        .collect()
}

/// The order of the answer: activity, then last open (never opened last), then name, then path.
fn sort_projects(list: &mut [Project]) {
    list.sort_by(|a, b| {
        b.activity_score
            .cmp(&a.activity_score)
            .then_with(|| match (&a.last_opened_at, &b.last_opened_at) {
                (Some(x), Some(y)) => y.cmp(x),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| path_key(&a.path).cmp(&path_key(&b.path)))
    });
}

fn search_item(project: &Project) -> SearchItem {
    SearchItem {
        r#type: SearchType::Project,
        id: project.id.clone(),
        title: project.name.clone(),
        subtitle: Some(project.path.clone()),
        icon: project.icon.clone(),
        status: None,
        score: 0,
    }
}

impl Service {
    /// Opens `projects.toml` (and writes the commented example when there is none) and takes
    /// the parts that a test may have replaced.
    pub async fn open(state: &AppState) -> anyhow::Result<Self> {
        let file = TomlFile::<ProjectsFile>::open(&state.hub, FILE_NAME, env::validate)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e.body.message))?;
        if !state.hub.file_path(FILE_NAME).exists() {
            // The only write of this file by Rain: the example, so the user can find the options.
            let written = file
                .edit(|document| {
                    *document = env::EXAMPLE_FILE
                        .parse()
                        .map_err(|_| ApiError::failed("The example of projects.toml is broken"))?;
                    Ok(())
                })
                .await;
            if let Err(e) = written {
                warn!(reason = ?e.body.reason, "could not write the example projects.toml");
            }
        }
        Ok(Self {
            state: state.downgrade(),
            file,
            git: state
                .seam::<dyn GitRunner>()
                .unwrap_or_else(|| Arc::new(SystemGit)),
            tools: state
                .seam::<dyn ToolEnv>()
                .unwrap_or_else(|| Arc::new(SystemToolEnv)),
            windows: state
                .seam::<dyn WindowLister>()
                .unwrap_or_else(|| Arc::new(SystemWindowLister)),
            processes: state
                .seam::<dyn ProcessLister>()
                .unwrap_or_else(|| Arc::new(SystemProcessLister)),
            throttle: Throttle::new(state.bus.clone(), TOPIC_RATE_PER_SECOND),
            git_slots: Semaphore::new(GIT_MAX_PARALLEL),
            cancel: state.shutdown.token().child_token(),
            memory: Mutex::new(Memory::default()),
            in_flight: Mutex::new(HashSet::new()),
        })
    }

    fn state(&self) -> ApiResult<AppState> {
        self.state.upgrade().ok_or_else(|| {
            ApiError::unavailable(SOURCE, "Rain is stopping").with_reason("shuttingDown")
        })
    }

    /// Ends the background work. Running git calls are killed with their tasks.
    pub fn stop(&self) {
        self.cancel.cancel();
    }

    /// Reads the ids that were given before, and starts watching the settings and the file.
    pub async fn start(self: Arc<Self>, state: AppState) -> anyhow::Result<()> {
        let ids = state
            .db
            .call(|conn| store::load_known_ids(conn))
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        lock(&self.memory).ids = ids;
        spawn_named("projects-watch", Arc::clone(&self).watch_changes(state));
        Ok(())
    }

    /// A change of the project folders or of `projects.toml` makes a new list, which is published.
    async fn watch_changes(self: Arc<Self>, state: AppState) {
        let mut settings = state.settings().watch();
        let mut file = self.file.subscribe();
        let mut roots = state.settings().resolved().project_roots.clone();
        loop {
            tokio::select! {
                () = self.cancel.cancelled() => return,
                changed = settings.changed() => {
                    if changed.is_err() { return; }
                    let now = settings.borrow_and_update().project_roots.clone();
                    if now == roots { continue; }
                    roots = now;
                }
                changed = file.changed() => {
                    if changed.is_err() { return; }
                    file.borrow_and_update();
                    info!("projects.toml changed");
                }
            }
            self.republish().await;
        }
    }

    // ---- the list ----

    /// The list of projects. See the top of this file for the steps.
    pub async fn list(self: &Arc<Self>, requested: Refresh) -> ApiResult<Arc<Vec<Project>>> {
        let state = self.state()?;
        // In game mode nothing is computed: the cache answers.
        let mode = if state.game_mode_active() {
            Refresh::CacheOnly
        } else {
            requested
        };
        let gathered = self.gather(&state, mode == Refresh::Force).await?;
        let mut rows = gathered.snapshot.git.clone();

        if let (ToolFound::At(git_exe), true) = (&gathered.git, mode != Refresh::CacheOnly) {
            let force = mode == Refresh::Force;
            let (mut must, mut stale) = (Vec::new(), Vec::new());
            for (found, id) in &gathered.snapshot.found {
                let row = rows.get(&found.key);
                let stamp = &gathered.stamp;
                let needed = facts::needs(row, &found.sig, stamp.ms, stamp.day, force);
                if needed.nothing() {
                    continue;
                }
                let job = Job {
                    found: found.clone(),
                    id: id.clone(),
                    old: row.cloned(),
                    needed,
                };
                // A project with no facts at all is waited for; one that is only stale is
                // refreshed in the background and shown with its old values.
                if row.is_none() || force {
                    must.push(job);
                } else {
                    stale.push(job);
                }
            }
            let waiting = self.start_refresh(must, git_exe.clone(), &gathered.stamp);
            self.start_refresh(stale, git_exe.clone(), &gathered.stamp);
            if let Some(done) = waiting {
                let budget = if force {
                    REFRESH_WAIT
                } else {
                    FIRST_FACTS_BUDGET
                };
                if let Ok(Ok(computed)) = tokio::time::timeout(budget, done).await {
                    for row in computed {
                        rows.insert(row.key.clone(), row);
                    }
                }
            }
        }

        let open = self.open_names(&gathered, mode != Refresh::CacheOnly).await;
        let list = Arc::new(self.assemble(&gathered, &rows, &open));
        lock(&self.memory).list = Some(Arc::clone(&list));
        Ok(list)
    }

    /// Reads the roots and the database. Everything a list needs except git facts and windows.
    async fn gather(&self, state: &AppState, clear_facts: bool) -> ApiResult<Gathered> {
        let roots = state.settings().resolved().project_roots.clone();
        let file = self.file.get();
        let overrides = file.overrides();
        let editor = tools::find(Tool::Editor, &overrides, self.tools.as_ref());
        let git_tool = tools::find(Tool::Git, &overrides, self.tools.as_ref());
        let builtins = Builtins {
            editor_label: editor_label(&editor),
        };
        let analysis = env::analyze(&file, &builtins);

        let discovery = self.discover(roots.clone()).await?;
        self.report_health(state, &roots, &discovery, &analysis, &builtins, &git_tool);

        let stamp = Stamp::of(state.clock.now());
        let root_keys: HashSet<String> = roots
            .iter()
            .map(|root| path_key(&root.to_string_lossy()))
            .collect();
        let forget = file.forget_missing_days();
        let found = discovery.found;
        let now_iso = stamp.iso.clone();
        let snapshot = state
            .db
            .call(move |conn| {
                if clear_facts {
                    store::clear_stale(conn)?;
                }
                store::sync(conn, &found, &root_keys, &now_iso, forget)
            })
            .await
            .map_err(|e| {
                e.into_api("Could not read the project list")
                    .with_source(SOURCE)
            })?;

        {
            let mut memory = lock(&self.memory);
            memory.ids = snapshot
                .found
                .iter()
                .map(|(f, id)| (f.key.clone(), id.clone()))
                .chain(
                    snapshot
                        .missing
                        .iter()
                        .map(|m| (m.key.clone(), m.id.clone())),
                )
                .collect();
        }
        Ok(Gathered {
            snapshot,
            analysis,
            builtins,
            editor,
            git: git_tool,
            stamp,
            pins: self.pinned_ids(),
        })
    }

    /// Discovery on the blocking pool. One that takes too long is abandoned and the last good
    /// one is used.
    async fn discover(&self, roots: Vec<PathBuf>) -> ApiResult<Discovery> {
        let work = blocking::run(move || discover(&roots));
        match tokio::time::timeout(DISCOVERY_BUDGET, work).await {
            Ok(done) => {
                let discovery = done?;
                lock(&self.memory).discovery = Some(Arc::new(discovery.found.clone()));
                Ok(discovery)
            }
            Err(_) => {
                warn!("looking for projects took too long; the last result is used");
                let found = lock(&self.memory)
                    .discovery
                    .as_ref()
                    .map(|found| found.as_ref().clone())
                    .unwrap_or_default();
                Ok(Discovery {
                    found,
                    unreadable: Vec::new(),
                })
            }
        }
    }

    fn report_health(
        &self,
        state: &AppState,
        roots: &[PathBuf],
        discovery: &Discovery,
        analysis: &Analysis,
        builtins: &Builtins,
        git_tool: &ToolFound,
    ) {
        let ignored = analysis.ignored + analysis.unknown_names(builtins);
        let (source_state, message) = if roots.is_empty() {
            (
                SourceState::Off,
                Some("No project folders are set".to_owned()),
            )
        } else if let Some(first) = discovery.unreadable.first() {
            let more = discovery.unreadable.len() - 1;
            let text = if more == 0 {
                format!("Project folder not found: {first}")
            } else {
                format!("Project folder not found: {first} and {more} more")
            };
            (SourceState::Failed, Some(text))
        } else if ignored > 0 {
            (
                SourceState::Ok,
                Some(format!("projects.toml: {ignored} step(s) ignored")),
            )
        } else {
            (SourceState::Ok, None)
        };
        state.health.set_source(SOURCE, source_state, message);

        match git_tool {
            ToolFound::At(_) => state.health.set_source("git", SourceState::Ok, None),
            ToolFound::NotFound => state.health.set_source(
                "git",
                SourceState::Off,
                Some("git was not found. Showing the branch only".to_owned()),
            ),
            ToolFound::ConfiguredMissing(path) => state.health.set_source(
                "git",
                SourceState::Failed,
                Some(format!("Configured program not found: {}", path.display())),
            ),
        }
    }

    /// The projects as the API shows them.
    fn assemble(
        &self,
        gathered: &Gathered,
        rows: &HashMap<String, GitRow>,
        open: &HashSet<String>,
    ) -> Vec<Project> {
        let git_known = matches!(gathered.git, ToolFound::At(_));
        let snapshot = &gathered.snapshot;
        let mut list = Vec::with_capacity(snapshot.found.len() + snapshot.missing.len());

        let apps = self.app_lookup();
        let environment = |key: &str, id: &str| {
            let (specs, _) = gathered
                .analysis
                .resolve(key, id, &gathered.builtins, &apps);
            env_steps(&specs)
        };
        let display = |key: &str, id: &str, folder: &str| {
            let entry = gathered.analysis.entry_for(key, id);
            let name = entry
                .and_then(|e| e.name.clone())
                .unwrap_or_else(|| folder.to_owned());
            let icon = entry.and_then(|e| e.icon.as_deref()).map_or_else(
                || letter(&name),
                |lucide| IconRef {
                    kind: IconKind::Lucide,
                    value: lucide.to_owned(),
                    tint: None,
                    fallback: None,
                },
            );
            (name, icon)
        };

        for (found, id) in &snapshot.found {
            let row = rows.get(&found.key).filter(|_| git_known);
            let commits = row
                .and_then(|r| r.commits.clone())
                .unwrap_or_else(|| vec![0; dates::STRIP_DAYS]);
            let (name, icon) = display(&found.key, id, &found.name);
            list.push(Project {
                id: id.clone(),
                name,
                path: found.path.clone(),
                icon,
                branch: found.branch.clone(),
                dirty_files: row.and_then(|r| r.dirty),
                activity_score: dates::activity_score(&commits),
                commits_14d: commits,
                last_opened_at: snapshot.opened.get(&found.key).cloned(),
                environment: environment(&found.key, id),
                open: open.contains(&found.name),
                pinned: gathered.pins.contains(id),
                missing: false,
            });
        }
        for gone in &snapshot.missing {
            let (name, icon) = display(&gone.key, &gone.id, &gone.name);
            list.push(Project {
                id: gone.id.clone(),
                name,
                path: gone.path.clone(),
                icon,
                branch: String::new(),
                dirty_files: None,
                commits_14d: vec![0; dates::STRIP_DAYS],
                last_opened_at: snapshot.opened.get(&gone.key).cloned(),
                activity_score: 0,
                environment: environment(&gone.key, &gone.id),
                open: false,
                pinned: gathered.pins.contains(&gone.id),
                missing: true,
            });
        }
        sort_projects(&mut list);
        list
    }

    /// The ids of the pinned projects. The pins belong to the launcher (section 01).
    fn pinned_ids(&self) -> HashSet<String> {
        self.state
            .upgrade()
            .map(|app| app.launcher().pinned_ids(PinnableType::Project))
            .unwrap_or_default()
    }

    /// Looks up an app of the launcher's list. An id without an app (or a launcher that is not
    /// there yet) answers `None`.
    fn app_lookup(&self) -> impl Fn(&str) -> Option<AppFacts> + use<> {
        let state = self.state.clone();
        move |id: &str| state.upgrade().and_then(|app| app.launcher().app_facts(id))
    }

    // ---- layouts ----

    /// The project of an id from the last list: its folder and its key.
    fn project_for_layout(&self, id: &str) -> ApiResult<Project> {
        self.list_cached()
            .iter()
            .find(|project| project.id == id)
            .cloned()
            .ok_or_else(|| {
                ApiError::not_found("Project not found")
                    .with_source(SOURCE)
                    .with_reason("unknownId")
            })
    }

    /// The steps that an item of a layout can name: the built-in ones, then those of the file.
    fn step_options(analysis: &Analysis, builtins: &Builtins) -> Vec<LayoutStepOption> {
        let mut names: Vec<String> = ["editor", "terminal", "claude"]
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        let mut named: Vec<&String> = analysis
            .steps
            .keys()
            .filter(|name| !names.contains(name))
            .collect();
        named.sort();
        names.extend(named.into_iter().cloned());
        names
            .iter()
            .filter_map(|name| {
                analysis
                    .steps
                    .get(name)
                    .cloned()
                    .or_else(|| env::builtin_step(name, builtins))
            })
            .map(|spec| LayoutStepOption {
                name: spec.name,
                kind: spec.kind,
                label: spec.label,
                icon: spec.icon,
            })
            .collect()
    }

    /// `GET /projects/{id}/layout`.
    pub async fn layout(&self, id: &str) -> ApiResult<ProjectLayout> {
        let project = self.project_for_layout(id)?;
        let file = self.file.get();
        let editor = tools::find(Tool::Editor, &file.overrides(), self.tools.as_ref());
        let builtins = Builtins {
            editor_label: editor_label(&editor),
        };
        let analysis = env::analyze(&file, &builtins);
        let (scope, items) = analysis.layout_of(&path_key(&project.path), &project.id);
        let monitors = match self.state() {
            Ok(state) => state.launcher().monitor_count().await,
            Err(_) => 0,
        };
        Ok(ProjectLayout {
            items: items.to_vec(),
            scope,
            steps: Self::step_options(&analysis, &builtins),
            monitors,
        })
    }

    /// `PUT /projects/{id}/layout`: writes the layout into `projects.toml` and answers the layout
    /// that counts for the project afterwards.
    pub async fn save_layout(&self, id: &str, save: LayoutSave) -> ApiResult<ProjectLayout> {
        let project = self.project_for_layout(id)?;
        let invalid = |reason: &'static str, message: String| {
            ApiError::invalid(message)
                .with_source(SOURCE)
                .with_reason(reason)
        };
        if save.items.len() > LAYOUT_ITEMS_MAX {
            return Err(invalid(
                "tooManyItems",
                format!("A layout holds at most {LAYOUT_ITEMS_MAX} items"),
            ));
        }
        let file = self.file.get();
        let editor = tools::find(Tool::Editor, &file.overrides(), self.tools.as_ref());
        let builtins = Builtins {
            editor_label: editor_label(&editor),
        };
        let analysis = env::analyze(&file, &builtins);
        let apps = self.app_lookup();
        for item in &save.items {
            if item
                .monitor
                .is_some_and(|number| number == 0 || number > LAYOUT_MONITOR_MAX)
            {
                return Err(invalid(
                    "badMonitor",
                    format!("A monitor number is 1 to {LAYOUT_MONITOR_MAX}"),
                ));
            }
            match &item.what {
                LayoutWhat::Step { name }
                    if !analysis.steps.contains_key(name)
                        && env::builtin_step(name, &builtins).is_none() =>
                {
                    return Err(invalid("unknownStep", format!("No step is named {name}")));
                }
                LayoutWhat::App { id } if apps(id).is_none() => {
                    return Err(invalid(
                        "unknownApp",
                        "An app of the layout was not found".to_owned(),
                    ));
                }
                _ => {}
            }
        }
        // A file that would refuse the write says so before anything else happens.
        self.file
            .writable()
            .map_err(|error| error.with_source(SOURCE))?;
        let key = path_key(&project.path);
        let (all, items) = (save.all, save.items);
        let path = project.path.clone();
        let project_id = project.id.clone();
        self.file
            .edit(move |document| {
                let scope = if all {
                    Scope::Defaults
                } else {
                    Scope::Project {
                        key: &key,
                        id: &project_id,
                        path: &path,
                    }
                };
                layout_file::set_layout(document, &scope, &items)
            })
            .await
            .map_err(|error| {
                if error.status == axum::http::StatusCode::INTERNAL_SERVER_ERROR {
                    ApiError::failed("Could not save projects.toml")
                        .with_source(SOURCE)
                        .with_reason("saveFailed")
                } else {
                    error.with_source(SOURCE)
                }
            })?;
        // The file watcher publishes the new list; the answer is what counts now.
        self.layout(id).await
    }

    // ---- open detection ----

    /// The folder names that an editor window shows. Cached for `OPEN_TTL`.
    async fn open_names(&self, gathered: &Gathered, enabled: bool) -> HashSet<String> {
        if !enabled {
            return HashSet::new();
        }
        let cached = lock(&self.memory)
            .windows
            .as_ref()
            .filter(|(at, _)| at.elapsed() < OPEN_TTL)
            .map(|(_, windows)| Arc::clone(windows));
        let windows = match cached {
            Some(windows) => windows,
            None => {
                let (lister, processes) = (Arc::clone(&self.windows), Arc::clone(&self.processes));
                let images = tools::editor_image_names(&gathered.editor);
                let listed = blocking::run(move || {
                    open::editor_windows(lister.as_ref(), processes.as_ref(), &images)
                })
                .await
                .unwrap_or_default();
                let listed = Arc::new(listed);
                lock(&self.memory).windows = Some((Instant::now(), Arc::clone(&listed)));
                listed
            }
        };
        open::open_names(
            &windows,
            gathered.snapshot.found.iter().map(|(f, _)| f.name.as_str()),
        )
    }

    /// Whether an editor window of the folder `name` is open now. The answer of the last list is
    /// reused only while it is younger than `OPEN_TTL`; an older one is looked up again, so that
    /// Enter on a project whose editor was closed minutes ago starts it again. Used by the plan,
    /// which is not async: the lookup is one window list and one process snapshot (2 to 8 ms).
    fn open_now(&self, name: &str, editor: &ToolFound) -> bool {
        let cached = lock(&self.memory)
            .windows
            .as_ref()
            .filter(|(at, _)| at.elapsed() < OPEN_TTL)
            .map(|(_, windows)| Arc::clone(windows));
        let windows = cached.unwrap_or_else(|| {
            let images = tools::editor_image_names(editor);
            let listed = Arc::new(open::editor_windows(
                self.windows.as_ref(),
                self.processes.as_ref(),
                &images,
            ));
            lock(&self.memory).windows = Some((Instant::now(), Arc::clone(&listed)));
            listed
        });
        !open::open_names(&windows, std::iter::once(name)).is_empty()
    }

    // ---- git facts in the background ----

    /// Starts the computation of the facts of `jobs`, except those that are already being
    /// computed. The receiver gets the new rows; a caller that does not wait drops it.
    fn start_refresh(
        self: &Arc<Self>,
        jobs: Vec<Job>,
        git_exe: PathBuf,
        stamp: &Stamp,
    ) -> Option<oneshot::Receiver<Vec<GitRow>>> {
        let jobs: Vec<Job> = {
            let mut in_flight = lock(&self.in_flight);
            jobs.into_iter()
                .filter(|job| in_flight.insert(job.found.key.clone()))
                .collect()
        };
        if jobs.is_empty() {
            return None;
        }
        let (sender, receiver) = oneshot::channel();
        let service = Arc::clone(self);
        let stamp = stamp.clone();
        spawn_named("projects-git", async move {
            service.refresh_task(jobs, git_exe, stamp, sender).await;
        });
        Some(receiver)
    }

    async fn refresh_task(
        self: Arc<Self>,
        jobs: Vec<Job>,
        git_exe: PathBuf,
        stamp: Stamp,
        sender: oneshot::Sender<Vec<GitRow>>,
    ) {
        let keys: Vec<String> = jobs.iter().map(|job| job.found.key.clone()).collect();
        let work = join_all(
            jobs.into_iter()
                .map(|job| self.compute_one(job, &git_exe, &stamp)),
        );
        let finished = tokio::select! {
            () = self.cancel.cancelled() => None,
            done = work => Some(done),
        };
        {
            let mut in_flight = lock(&self.in_flight);
            for key in &keys {
                in_flight.remove(key);
            }
        }
        let Some(finished) = finished else { return };

        let changed = finished.iter().any(|(_, changed)| *changed);
        let rows: Vec<GitRow> = finished.into_iter().map(|(row, _)| row).collect();
        if let Ok(state) = self.state() {
            let to_save = rows.clone();
            if let Err(e) = state
                .db
                .call(move |conn| store::save_git_rows(conn, &to_save))
                .await
            {
                warn!(error = %e, "could not save the git facts");
            }
        }
        // The request that waited gets the rows directly; nobody may be waiting any more.
        let _ = sender.send(rows);
        if changed {
            self.republish().await;
        }
    }

    /// Computes the facts of one repository. Returns the new row and whether a shown value
    /// changed.
    async fn compute_one(
        &self,
        job: Job,
        git_exe: &std::path::Path,
        stamp: &Stamp,
    ) -> (GitRow, bool) {
        // At most `GIT_MAX_PARALLEL` git processes at once, however many projects there are.
        let _slot = self.git_slots.acquire().await;
        let offset = |time: i64| crate::platform::time::utc_offset_seconds(time);
        let moment = Moment {
            now_iso: &stamp.iso,
            today_date: &stamp.date,
            today: stamp.day,
            offset_at: &offset,
        };
        let target = Target {
            repo: std::path::Path::new(&job.found.path),
            key: &job.found.key,
            old: job.old.as_ref(),
            sig: &job.found.sig,
            branch: &job.found.branch,
        };
        let (row, failed) =
            facts::compute(self.git.as_ref(), git_exe, &target, job.needed, &moment).await;
        if failed {
            // The id only: never the path, never git's own text.
            warn!(project = %job.id, "git did not answer for a project; its facts stay as they were");
        }
        let changed = job.old.as_ref().is_none_or(|old| {
            old.dirty != row.dirty || old.commits != row.commits || old.branch != row.branch
        });
        (row, changed)
    }

    /// Makes a new list from what is known now (no git call) and publishes it.
    async fn republish(self: &Arc<Self>) {
        let Ok(state) = self.state() else { return };
        let Ok(gathered) = self.gather(&state, false).await else {
            return;
        };
        let rows = gathered.snapshot.git.clone();
        let open = self.open_names(&gathered, !state.game_mode_active()).await;
        let list = Arc::new(self.assemble(&gathered, &rows, &open));
        lock(&self.memory).list = Some(Arc::clone(&list));
        self.throttle.publish(&list);
    }

    /// Sends a list to the clients (at most once a second).
    pub fn publish(&self, list: &Arc<Vec<Project>>) {
        self.throttle.publish(list);
    }

    // ---- what other modules ask ----

    /// The last list. Without one, a single discovery with branches and no git facts: it never
    /// waits for git.
    pub fn list_cached(&self) -> Arc<Vec<Project>> {
        if let Some(list) = lock(&self.memory).list.clone() {
            return list;
        }
        let Ok(state) = self.state() else {
            return Arc::new(Vec::new());
        };
        let roots = state.settings().resolved().project_roots.clone();
        let mut found = discover(&roots).found;
        found.sort_by(|a, b| a.key.cmp(&b.key));
        let file = self.file.get();
        let overrides = file.overrides();
        let editor = tools::find(Tool::Editor, &overrides, self.tools.as_ref());
        let builtins = Builtins {
            editor_label: editor_label(&editor),
        };
        let found_with_ids: Vec<(Found, String)> = {
            let mut memory = lock(&self.memory);
            found
                .into_iter()
                .map(|item| {
                    let id = match memory.ids.get(&item.key) {
                        Some(id) => id.clone(),
                        None => {
                            let taken: HashSet<&String> = memory.ids.values().collect();
                            let id = project_id(&item.name, &item.key, &|c| {
                                taken.iter().any(|t| t.as_str() == c)
                            });
                            memory.ids.insert(item.key.clone(), id.clone());
                            id
                        }
                    };
                    (item, id)
                })
                .collect()
        };
        let gathered = Gathered {
            snapshot: Snapshot {
                found: found_with_ids,
                ..Snapshot::default()
            },
            analysis: env::analyze(&file, &builtins),
            builtins,
            editor,
            git: ToolFound::NotFound,
            stamp: Stamp::of(state.clock.now()),
            pins: self.pinned_ids(),
        };
        let list = Arc::new(self.assemble(&gathered, &HashMap::new(), &HashSet::new()));
        lock(&self.memory).list = Some(Arc::clone(&list));
        list
    }

    /// The project that holds `cwd`: its folder is `cwd` or the longest folder that `cwd` is
    /// inside of. Missing projects count (section 03 shows the sessions of a vanished folder).
    pub fn project_id_for_path(&self, cwd: &str) -> Option<String> {
        let key = path_key(cwd);
        self.list_cached()
            .iter()
            .filter(|project| {
                let own = path_key(&project.path);
                key == own || key.starts_with(&format!("{own}\\"))
            })
            .max_by_key(|project| project.path.len())
            .map(|project| project.id.clone())
    }
}

impl ItemSource for Service {
    fn search_entries(&self) -> Vec<(SearchItem, Vec<String>)> {
        self.list_cached()
            .iter()
            .filter(|project| !project.missing)
            .map(|project| {
                let keywords = if project.branch.is_empty() {
                    Vec::new()
                } else {
                    vec![project.branch.clone()]
                };
                (search_item(project), keywords)
            })
            .collect()
    }

    /// A missing project is returned too: a pin the user made stays, and its row says "Locate".
    fn item_for(&self, id: &str) -> Option<SearchItem> {
        self.list_cached()
            .iter()
            .find(|project| project.id == id)
            .map(search_item)
    }

    fn recent(&self) -> Vec<(String, SearchItem)> {
        let mut recent: Vec<(String, SearchItem)> = self
            .list_cached()
            .iter()
            .filter(|project| !project.missing)
            .filter_map(|project| {
                project
                    .last_opened_at
                    .clone()
                    .map(|at| (at, search_item(project)))
            })
            .collect();
        // ISO times in UTC sort as text.
        recent.sort_by(|a, b| b.0.cmp(&a.0));
        recent
    }
}

impl LaunchPlanner for Service {
    /// Reads caches only and starts nothing (the launch contract). The project must be in the
    /// last list.
    fn plan(&self, target: &LaunchTarget) -> ApiResult<LaunchPlan> {
        let state = self.state()?;
        if state.game_mode_active() {
            return Err(ApiError::conflict("gameMode", "Game mode is on").with_source(SOURCE));
        }
        let list = self.list_cached();
        let project = list
            .iter()
            .find(|project| project.id == target.id)
            .ok_or_else(|| {
                ApiError::not_found("Project not found")
                    .with_source(SOURCE)
                    .with_reason("unknownId")
            })?;

        let file = self.file.get();
        let overrides = file.overrides();
        let editor = tools::find(Tool::Editor, &overrides, self.tools.as_ref());
        let builtins = Builtins {
            editor_label: editor_label(&editor),
        };
        let analysis = env::analyze(&file, &builtins);
        let key = path_key(&project.path);
        let apps = self.app_lookup();
        let (specs, _) = analysis.resolve(&key, &project.id, &builtins, &apps);
        let steps = launch::select_steps(target.mode, &specs, &builtins)
            .map_err(|e| e.with_source(SOURCE))?;
        let mode = target.mode.unwrap_or(rain_types::LaunchMode::Full);
        let steps = launch::plan_steps(&PlanInput {
            name: &project.name,
            folder: &project.path,
            missing: project.missing,
            // Not the `open` of the cached list, which may be minutes old (section 6.7: at most
            // `OPEN_TTL`). The editor shows the folder's name, which is not always the display name.
            open: !project.missing
                && self.open_now(
                    project.path.rsplit('\\').next().unwrap_or(&project.name),
                    &editor,
                ),
            mode,
            steps,
            wt: WtOptions {
                new_window: file.terminal_in_new_window(),
                profile: file.terminal.profile.clone(),
            },
            editor,
            terminal: tools::find(Tool::WindowsTerminal, &overrides, self.tools.as_ref()),
            coding_agent: tools::find(Tool::Claude, &overrides, self.tools.as_ref()),
            builtins: &builtins,
            apps: &apps,
        });
        Ok(LaunchPlan { steps })
    }

    /// The project was opened when at least one step worked: write the time before the final
    /// frame, because the screen asks again right after it.
    fn finished<'a>(&'a self, run: &'a LaunchRun) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if run.target.r#type != LaunchType::Project
                || !run.steps.iter().any(|step| step.status == Status::Success)
            {
                return;
            }
            let Ok(state) = self.state() else { return };
            let Some(path) = self
                .list_cached()
                .iter()
                .find(|project| project.id == run.target.id)
                .map(|project| project.path.clone())
            else {
                return;
            };
            let key = path_key(&path);
            let now = rain_types::time::iso_from_system_time(state.clock.now());
            let (key_for_db, now_for_db) = (key.clone(), now.clone());
            if let Err(e) = state
                .db
                .call(move |conn| store::record_opened(conn, &key_for_db, &now_for_db))
                .await
            {
                warn!(error = %e, "could not record that a project was opened");
                return;
            }
            // Update the last list in place and publish it: no discovery, no git.
            let current = lock(&self.memory).list.clone();
            let updated = current.map(|list| {
                let mut changed = list.as_ref().clone();
                if let Some(project) = changed.iter_mut().find(|p| p.id == run.target.id) {
                    project.last_opened_at = Some(now.clone());
                }
                sort_projects(&mut changed);
                let changed = Arc::new(changed);
                lock(&self.memory).list = Some(Arc::clone(&changed));
                changed
            });
            if let Some(list) = updated {
                self.throttle.publish(&list);
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn project(name: &str, score: u32, opened: Option<&str>) -> Project {
        Project {
            id: format!("proj-{name}"),
            name: name.to_owned(),
            path: format!("D:\\Work\\{name}"),
            icon: letter(name),
            branch: "main".into(),
            dirty_files: None,
            commits_14d: vec![0; 14],
            last_opened_at: opened.map(str::to_owned),
            activity_score: score,
            environment: Vec::new(),
            open: false,
            pinned: false,
            missing: false,
        }
    }

    #[test]
    fn the_answer_is_ordered_by_activity_then_last_open_then_name() {
        let mut list = vec![
            project("zeta", 0, None),
            project("beta", 0, Some("2026-11-01T00:00:00.000Z")),
            project("alpha", 0, None),
            project("busy", 9, None),
            project("recent", 0, Some("2026-11-05T00:00:00.000Z")),
        ];
        sort_projects(&mut list);
        let names: Vec<&str> = list.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["busy", "recent", "beta", "alpha", "zeta"]);
    }

    #[test]
    fn the_editor_is_called_by_its_program() {
        assert_eq!(editor_label(&ToolFound::NotFound), "VS Code");
        assert_eq!(
            editor_label(&ToolFound::At(PathBuf::from("C:\\x\\Code.exe"))),
            "VS Code"
        );
        assert_eq!(
            editor_label(&ToolFound::At(PathBuf::from("C:\\x\\cursor.exe"))),
            "Cursor"
        );
    }

    #[test]
    fn the_environment_shows_the_command_of_a_step() {
        let builtins = Builtins {
            editor_label: "VS Code".into(),
        };
        let steps = ["editor", "terminal", "claude"]
            .iter()
            .filter_map(|name| env::builtin_step(name, &builtins))
            .collect::<Vec<_>>();
        let shown = env_steps(&steps);
        assert_eq!(shown[0].command, None);
        assert_eq!(shown[2].command.as_deref(), Some("claude"));
    }
}
