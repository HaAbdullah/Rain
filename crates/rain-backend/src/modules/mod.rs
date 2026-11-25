//! The feature modules and the table that starts and stops them. Foundation section 11.2.
//!
//! Every module is one folder with the same files (`mod.rs`, `routes.rs`, `service.rs`,
//! `store.rs`). Modules call each other only through the other module's public `Service`.
//! All thirteen modules are built. The table below holds the entry of each one (name, migrations,
//! router, init, start, stop); a feature section only changes the files of its own folder.

use std::time::Duration;

use axum::Router;
use axum::extract::{Request, State};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use futures_util::future::{BoxFuture, join_all};
use rain_types::{ConfigIssue, SourceState};
use tracing::{error, info, warn};

use crate::core::error::ApiError;
use crate::core::state::AppState;
use crate::store::migrations::{self, Migration, MigrationError};

pub mod disk;
pub mod gamemode;
pub mod games;
pub mod hardware;
pub mod idle;
pub mod jobs;
pub mod launcher;
pub mod life;
pub mod lifecycle;
pub mod machine;
pub mod projects;
pub mod sessions;
pub mod settings;

/// What the backend needs to know about one module.
pub struct ModuleEntry {
    /// Names the module in logs and in the health report: `launcher`, `disk` ... It is also the
    /// prefix of the module's tables (`launcher_launch`): the migration runner enforces that.
    pub name: &'static str,
    /// The schema of the module's tables, oldest first. Empty when the module has no tables.
    pub migrations: &'static [Migration],
    /// The routes of the module, mounted under `/api/v1`.
    pub router: fn() -> Router<AppState>,
    /// Builds the module's `Service` and puts it into its slot. Runs for all modules before any
    /// `start`, so a module may call another one's `Service` inside `start`.
    pub init: fn(&AppState) -> BoxFuture<'static, anyhow::Result<()>>,
    /// Starts background tasks.
    pub start: fn(AppState) -> BoxFuture<'static, anyhow::Result<()>>,
    /// Ends background tasks. Publishes no topic frame.
    pub stop: fn(AppState) -> BoxFuture<'static, ()>,
}

/// Start order is the order of this table; stop order is the reverse.
pub const MODULES: &[ModuleEntry] = &[
    settings::ENTRY,
    lifecycle::ENTRY,
    machine::ENTRY,
    disk::ENTRY,
    hardware::ENTRY,
    launcher::ENTRY,
    projects::ENTRY,
    games::ENTRY,
    sessions::ENTRY,
    jobs::ENTRY,
    idle::ENTRY,
    life::ENTRY,
    gamemode::ENTRY,
];

/// An `init` or `start` that has nothing to do yet.
pub(crate) fn ready() -> BoxFuture<'static, anyhow::Result<()>> {
    Box::pin(std::future::ready(Ok(())))
}

/// A `stop` that has nothing to do yet.
pub(crate) fn nothing() -> BoxFuture<'static, ()> {
    Box::pin(std::future::ready(()))
}

// ---- routes ----

/// The routes of all modules, joined into one router. The routes of a module that is out of
/// service (it failed to start, or its tables are from a newer Rain) answer `unavailable`.
pub fn routers(state: &AppState, modules: &[ModuleEntry]) -> Router<AppState> {
    modules.iter().fold(Router::new(), |all, module| {
        let gate = Gate {
            state: state.clone(),
            module: module.name,
        };
        let routes = (module.router)();
        // `route_layer` on a router without routes is an error in axum, and a module may have none.
        let routes = if routes.has_routes() {
            routes.route_layer(from_fn_with_state(gate, refuse_if_failed))
        } else {
            routes
        };
        all.merge(routes)
    })
}

/// What the gate in front of a module's routes needs.
#[derive(Clone)]
struct Gate {
    state: AppState,
    module: &'static str,
}

async fn refuse_if_failed(State(gate): State<Gate>, request: Request, next: Next) -> Response {
    match gate.state.module_failure(gate.module) {
        Some(reason) => ApiError::unavailable(
            gate.module,
            format!("The {} part of Rain is not available", gate.module),
        )
        .with_reason(reason)
        .into_response(),
        None => next.run(request).await,
    }
}

// ---- start and stop ----

/// The database schema of every module (foundation 10.5). A module whose migrations fail is out
/// of service: its routes answer `unavailable` (`dbTooNew` when the database is from a newer
/// Rain) and it is not started. The other modules are not affected.
pub async fn migrate_all(state: &AppState, modules: &[ModuleEntry]) {
    let applied_at = rain_types::time::iso_from_system_time(state.clock.now());
    let list = modules
        .iter()
        .filter(|module| !module.migrations.is_empty())
        .map(|module| (module.name, module.migrations))
        .collect();
    match migrations::run_all(&state.db, list, applied_at).await {
        Ok(failed) => {
            for (module, cause) in failed {
                report_migration_failure(state, module, &cause);
            }
        }
        Err(e) => {
            // The database thread is gone, which is a bug: no module can use its tables.
            error!(error = %e, "the database migrations could not run");
            for module in modules.iter().filter(|m| !m.migrations.is_empty()) {
                state.fail_module(module.name, "moduleFailed");
            }
        }
    }
}

fn report_migration_failure(state: &AppState, module: &'static str, cause: &MigrationError) {
    error!(module, error = %cause, "the migrations of a module failed");
    state.fail_module(module, cause.reason());
    state.health.set_source(
        module,
        SourceState::Failed,
        Some(format!(
            "The {module} part of Rain could not prepare its data"
        )),
    );
    if matches!(cause, MigrationError::TooNew { .. }) {
        state.health.set_file_issues(
            "rain.db",
            vec![ConfigIssue {
                file: "rain.db".to_owned(),
                line: None,
                message: "Written by a newer Rain".to_owned(),
            }],
        );
    }
}

/// Phase one: every module builds its service. A failure here is a bug (for example a cycle of
/// calls between two modules), so it stops the start.
pub async fn init_all(state: &AppState, modules: &[ModuleEntry]) -> anyhow::Result<()> {
    for module in modules {
        (module.init)(state)
            .await
            .map_err(|e| e.context(format!("module {} failed to initialise", module.name)))?;
    }
    Ok(())
}

/// Phase two: every module starts its background work. A module that fails to start does not stop
/// the backend: the failure is logged and shown in the health report, its routes answer
/// `unavailable` (`moduleFailed`), and the rest carry on. A module that is already out of service
/// (its migrations failed) is not started.
pub async fn start_all(state: &AppState, modules: &[ModuleEntry]) {
    for module in modules {
        if state.module_failure(module.name).is_some() {
            continue;
        }
        match (module.start)(state.clone()).await {
            Ok(()) => info!(module = module.name, "module started"),
            Err(e) => {
                error!(module = module.name, error = %e, "module failed to start");
                state.fail_module(module.name, "moduleFailed");
                state.health.set_source(
                    module.name,
                    SourceState::Failed,
                    Some(format!("The {} part of Rain could not start", module.name)),
                );
            }
        }
    }
}

/// Stops every module in the reverse order of the table, all in parallel. A module that is not
/// finished after `budget` is abandoned and logged.
pub async fn stop_all(state: &AppState, modules: &[ModuleEntry], budget: Duration) {
    let stops = modules.iter().rev().map(|module| async move {
        let stopped = tokio::time::timeout(budget, (module.stop)(state.clone())).await;
        if stopped.is_err() {
            warn!(
                module = module.name,
                "module did not stop in time and was abandoned"
            );
        }
    });
    join_all(stops).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    fn state() -> AppState {
        let config = Config::for_test("rain-unit", "data".into(), "local".into());
        AppState::builder(config).build()
    }

    fn failing_start() -> ModuleEntry {
        ModuleEntry {
            name: "broken",
            migrations: &[],
            router: Router::new,
            init: |_| ready(),
            start: |_| Box::pin(std::future::ready(Err(anyhow::anyhow!("no luck")))),
            stop: |_| nothing(),
        }
    }

    #[test]
    fn module_names_are_unique() {
        let mut names: Vec<_> = MODULES.iter().map(|m| m.name).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count);
        assert_eq!(count, 13);
    }

    #[tokio::test]
    async fn init_fills_the_slot_of_every_module() {
        // The real modules write their example files at `init` (projects.toml, jobs.toml, ...), so
        // the data folder is a temporary one and nothing lands in the crate folder.
        let data = tempfile::tempdir().unwrap();
        let config = Config::for_test("rain-unit", data.path().to_path_buf(), "local".into());
        let state = AppState::builder(config).build();
        init_all(&state, MODULES).await.unwrap();
        // The accessors panic when a slot is empty.
        state.settings();
        state.lifecycle();
        state.machine();
        state.disk();
        state.hardware();
        state.launcher();
        state.projects();
        state.games();
        state.sessions();
        state.jobs();
        state.idle();
        state.life();
        state.gamemode();
    }

    #[tokio::test]
    async fn a_module_that_fails_to_start_is_reported_and_the_others_still_start() {
        let state = state();
        start_all(&state, &[failing_start()]).await;

        let sources = state.health.snapshot().sources;
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].name, "broken");
        assert_eq!(sources[0].state, SourceState::Failed);
        assert_eq!(state.module_failure("broken"), Some("moduleFailed"));
    }

    #[tokio::test]
    async fn a_module_that_is_out_of_service_is_not_started() {
        static STARTED: AtomicUsize = AtomicUsize::new(0);
        let state = state();
        state.fail_module("counted", "dbTooNew");
        let counted = ModuleEntry {
            name: "counted",
            migrations: &[],
            router: Router::new,
            init: |_| ready(),
            start: |_| {
                Box::pin(async {
                    STARTED.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
            },
            stop: |_| nothing(),
        };
        start_all(&state, &[counted]).await;
        assert_eq!(STARTED.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn stopping_gives_up_on_a_module_that_never_finishes() {
        static STOPPED: AtomicUsize = AtomicUsize::new(0);
        let hanging = ModuleEntry {
            name: "hanging",
            migrations: &[],
            router: Router::new,
            init: |_| ready(),
            start: |_| ready(),
            stop: |_| Box::pin(std::future::pending()),
        };
        let quick = ModuleEntry {
            name: "quick",
            migrations: &[],
            router: Router::new,
            init: |_| ready(),
            start: |_| ready(),
            stop: |_| {
                Box::pin(async {
                    STOPPED.fetch_add(1, Ordering::SeqCst);
                })
            },
        };
        let started = std::time::Instant::now();
        stop_all(&state(), &[hanging, quick], Duration::from_millis(80)).await;

        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(
            STOPPED.load(Ordering::SeqCst),
            1,
            "the other module still stopped"
        );
    }

    fn answering_module() -> Router<AppState> {
        Router::new().route("/ok", axum::routing::get(|| async { "fine" }))
    }

    async fn status_of(state: &AppState, module: &'static str) -> (StatusCode, String) {
        let entry = ModuleEntry {
            name: module,
            migrations: &[],
            router: answering_module,
            init: |_| ready(),
            start: |_| ready(),
            stop: |_| nothing(),
        };
        let app = routers(state, &[entry]).with_state(state.clone());
        let response = app
            .oneshot(Request::builder().uri("/ok").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn the_routes_of_a_failed_module_answer_unavailable_with_the_reason() {
        let state = state();
        assert_eq!(
            status_of(&state, "demo").await,
            (StatusCode::OK, "fine".to_owned())
        );

        state.fail_module("demo", "dbTooNew");
        let (status, body) = status_of(&state, "demo").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(body.contains("\"reason\":\"dbTooNew\""), "{body}");
        assert!(body.contains("\"source\":\"demo\""), "{body}");
    }

    const FIRST: Migration = Migration {
        id: 1,
        name: "create_thing",
        sql: "CREATE TABLE demo_thing (id INTEGER PRIMARY KEY)",
    };
    const SECOND: Migration = Migration {
        id: 2,
        name: "add_label",
        sql: "ALTER TABLE demo_thing ADD COLUMN label TEXT",
    };

    #[tokio::test]
    async fn a_module_whose_tables_are_from_a_newer_rain_is_out_of_service_and_reported() {
        let temp = tempfile::tempdir().unwrap();
        let config = Config::for_test(
            "rain-unit",
            temp.path().join("data"),
            temp.path().join("local"),
        );
        let state = AppState::builder(config).build();
        state.db.open(1).await.unwrap();
        let module = |migrations| ModuleEntry {
            name: "demo",
            migrations,
            router: Router::new,
            init: |_| ready(),
            start: |_| ready(),
            stop: |_| nothing(),
        };

        migrate_all(&state, &[module(&[FIRST, SECOND])]).await;
        assert_eq!(state.module_failure("demo"), None);

        // An older Rain (it knows migration 1 only) on the same database.
        migrate_all(&state, &[module(&[FIRST])]).await;
        assert_eq!(state.module_failure("demo"), Some("dbTooNew"));
        let health = state.health.snapshot();
        assert_eq!(health.config_issues[0].file, "rain.db");
        assert_eq!(health.sources[0].state, SourceState::Failed);
    }
}
