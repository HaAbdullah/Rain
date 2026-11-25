//! Starting a game. Section 02, 6.14.
//!
//! The launcher (section 01) owns the one launch engine: it asks `plan` for the steps of a game,
//! runs them, and awaits `finished` before the last frame of the run. This file says what the
//! steps are:
//!
//! | Source | The step |
//! |---|---|
//! | Steam | the address `steam://rungameid/<appid>`, handed to Windows (Steam starts the game) |
//! | Epic | the address `com.epicgames.launcher://apps/<AppName>?action=launch&silent=true`; for a game of the new launcher `<AppName>` is `<namespace>%3A<itemId>%3A<AppName>` |
//! | Riot | the Riot Client program, started with `--launch-product=<product> --launch-patchline=<patchline>`, in its own folder |
//! | Local | a direct start of the program file in its own folder, detached from Rain |
//!
//! `launch_after_game_mode` does the same without a run, for game mode, which calls it just
//! before the backend ends.

use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
use rain_types::{LaunchRun, LaunchTarget, LaunchType, RunStatus};

use super::library::Library;
use super::model::{Found, Launch};
use super::service::{LaunchedAfterGameMode, Service, icon_of, lock, play_key};
use super::store;
use crate::core::error::{ApiError, ApiResult};
use crate::core::launch_contract::{
    ErrorText, LaunchPlan, LaunchPlanner, OnSpawned, OpenRequest, PlannedStep, ShellFallback,
    StepAction,
};

/// `launch_after_game_mode` returns within this time.
const HAND_OVER_LIMIT: Duration = Duration::from_secs(10);

/// What a `ShellExecute` result of 31 means: no program is registered for the address.
const NO_HANDLER: u32 = 31;

fn unknown_game() -> ApiError {
    ApiError::not_found("Game not found")
        .with_source("games")
        .with_reason("unknownId")
}

/// The request that starts a game, or the text of why it cannot be started.
pub(super) enum Start {
    Open(OpenRequest),
    /// The step fails before it starts, with this text.
    Refuse(String),
}

pub(super) fn start_of(found: &Found) -> Start {
    match &found.launch {
        Launch::Steam { appid } => Start::Open(OpenRequest::ShellExec {
            // The id is digits only: the manifest reader checked it.
            file: format!("steam://rungameid/{appid}"),
            params: None,
            dir: None,
        }),
        Launch::Epic { app_name, catalog } => Start::Open(OpenRequest::ShellExec {
            file: super::epic::launch_url(app_name, catalog.as_ref()),
            params: None,
            dir: None,
        }),
        Launch::Riot {
            client,
            product,
            patchline,
        } => {
            if !client.is_file() {
                return Start::Refuse(format!("Riot Client not found: {}", client.display()));
            }
            // Through the shell, like Steam and Epic: the client starts the game itself, so Rain
            // has no process of the game to watch and counts no play time.
            Start::Open(OpenRequest::ShellExec {
                file: client.to_string_lossy().into_owned(),
                params: Some(super::riot::launch_args(product, patchline)),
                dir: client
                    .parent()
                    .map(|folder| folder.to_string_lossy().into_owned()),
            })
        }
        Launch::Local => {
            let Some(local) = &found.local else {
                return Start::Refuse("No program file. Choose one with Edit".to_owned());
            };
            let Some(exe) = &local.exe else {
                return Start::Refuse("No program file. Choose one with Edit".to_owned());
            };
            if !exe.is_file() {
                return Start::Refuse(format!("Program file not found: {}", exe.display()));
            }
            Start::Open(OpenRequest::Spawn {
                exe: exe.clone(),
                args_raw: local.args.clone().unwrap_or_default(),
                // The program starts in its own folder: many games read files next to them.
                cwd: exe
                    .parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or_default(),
            })
        }
    }
}

/// The text of a step that failed when it was started, from the code of the failure (never from
/// the text of the error, which can change).
fn error_text_of(found: &Found) -> ErrorText {
    let launch = found.launch.clone();
    Arc::new(move |error| match (&launch, error.code) {
        (Launch::Steam { .. }, Some(NO_HANDLER)) => "Steam is not installed".to_owned(),
        (Launch::Steam { .. }, _) => "Could not hand the game to Steam".to_owned(),
        (Launch::Epic { .. }, Some(NO_HANDLER)) => {
            "Epic Games Launcher is not installed".to_owned()
        }
        (Launch::Epic { .. }, _) => "Could not hand the game to Epic Games".to_owned(),
        (Launch::Riot { .. }, _) => "Could not start the Riot Client".to_owned(),
        (Launch::Local, _) => "Could not start the program".to_owned(),
    })
}

impl Service {
    fn find_cached(&self, id: &str) -> ApiResult<(Arc<Library>, Found)> {
        let library = self.cached_library().ok_or_else(|| {
            ApiError::unavailable("games", "Games are still loading").with_reason("loading")
        })?;
        let found = library.find(id).cloned().ok_or_else(unknown_game)?;
        Ok((library, found))
    }
}

impl LaunchPlanner for Service {
    fn plan(&self, target: &LaunchTarget) -> ApiResult<LaunchPlan> {
        if target.r#type != LaunchType::Game {
            return Err(ApiError::invalid("This is not a game").with_source("games"));
        }
        let app = self.app()?;
        if app.game_mode_active() {
            return Err(ApiError::conflict("gameMode", "Game mode is on").with_source("games"));
        }
        let (_, found) = self.find_cached(&target.id)?;
        let key = play_key(&found);

        let (action, preset_error, shell_fallback, on_spawned) =
            if lock(&self.sessions).contains_key(&key) {
                // Rain started this game and it still runs: do not start a second one, and
                // do not count the run as a launch either (see `finished`).
                lock(&self.already_running).insert(key.clone());
                (StepAction::Nothing, None, None, None)
            } else {
                lock(&self.already_running).remove(&key);
                match start_of(&found) {
                    Start::Refuse(text) => (StepAction::Nothing, Some(text), None, None),
                    Start::Open(OpenRequest::ShellExec { file, params, dir }) => (
                        StepAction::ShellExec { file, params, dir },
                        None,
                        None,
                        None,
                    ),
                    Start::Open(OpenRequest::Spawn { exe, args_raw, cwd }) => {
                        // A program that needs administrator rights cannot be started directly: the
                        // engine then starts it through the shell (a prompt appears; not watched).
                        let fallback = ShellFallback {
                            file: exe.to_string_lossy().into_owned(),
                            params: (!args_raw.is_empty()).then(|| args_raw.clone()),
                            dir: Some(cwd.to_string_lossy().into_owned()),
                        };
                        let service = app.games();
                        let folder = Some(cwd.to_string_lossy().into_owned());
                        let runtime = tokio::runtime::Handle::current();
                        let session_key = key.clone();
                        let on_spawned: OnSpawned = Arc::new(move |pid, creation_time| {
                            runtime.spawn(Arc::clone(&service).begin_session(
                                session_key.clone(),
                                pid,
                                creation_time,
                                folder.clone(),
                            ));
                        });
                        (
                            StepAction::Spawn { exe, args_raw, cwd },
                            None,
                            Some(fallback),
                            Some(on_spawned),
                        )
                    }
                    // `start_of` never asks for an activation; the step fails instead of panicking.
                    Start::Open(OpenRequest::Activate { .. }) => (
                        StepAction::Nothing,
                        Some("Could not start the program".to_owned()),
                        None,
                        None,
                    ),
                }
            };

        Ok(LaunchPlan {
            steps: vec![PlannedStep {
                // The failure notice reads "<title>: <error>".
                label: found.title.clone(),
                icon: icon_of(&found),
                action,
                delay_ms: 0,
                preset_error,
                stops_run: true,
                on_spawned,
                shell_fallback,
                error_text: Some(error_text_of(&found)),
                placement: None,
            }],
        })
    }

    /// Writes the launch into Rain's own record before the last frame of the run, so that the
    /// screen that refetches after it sees "last played" at once.
    fn finished<'a>(&'a self, run: &'a LaunchRun) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if run.target.r#type != LaunchType::Game || run.status != RunStatus::Success {
                return;
            }
            let Ok(app) = self.app() else { return };
            let Some(found) = self
                .cached_library()
                .and_then(|library| library.find(&run.target.id).cloned())
            else {
                return;
            };
            if lock(&self.already_running).remove(&play_key(&found)) {
                return; // the game was already running: nothing was launched
            }
            let now = rain_types::time::iso_from_system_time(app.clock.now());
            if store::record_launch(&app.db, play_key(&found), now)
                .await
                .is_ok()
            {
                self.reload_plays().await;
                self.invalidate();
                self.publish().await;
            }
        })
    }
}

impl Service {
    /// Starts a game for game mode, just before the backend ends. No run, no frame, and no
    /// check of game mode (only game mode calls this). It returns when the game is handed over,
    /// within 10 seconds.
    pub fn launch_after_game_mode(
        &self,
        id: &str,
    ) -> BoxFuture<'_, ApiResult<LaunchedAfterGameMode>> {
        let id = id.to_owned();
        Box::pin(async move {
            tokio::time::timeout(HAND_OVER_LIMIT, self.hand_over(&id))
                .await
                .unwrap_or_else(|_| {
                    Err(ApiError::failed("The game did not start in time").with_source("games"))
                })
        })
    }

    async fn hand_over(&self, id: &str) -> ApiResult<LaunchedAfterGameMode> {
        let app = self.app()?;
        let library = self.library(false).await?;
        let found = library.find(id).cloned().ok_or_else(unknown_game)?;
        let request = match start_of(&found) {
            Start::Open(request) => request,
            Start::Refuse(text) => {
                return Err(ApiError::failed(text)
                    .with_source("games")
                    .with_reason("cannotStart"));
            }
        };
        let is_local = matches!(request, OpenRequest::Spawn { .. });
        let folder = match &request {
            OpenRequest::Spawn { cwd, .. } => Some(cwd.to_string_lossy().into_owned()),
            _ => None,
        };
        let started = app.opener().open(request).await.map_err(|error| {
            ApiError::failed(error.ui_text)
                .with_source("games")
                .with_reason("cannotStart")
        })?;

        let now = rain_types::time::iso_from_system_time(app.clock.now());
        // The backend is about to exit, so no watcher starts: the next start picks the session up
        // from this row (section 6.13).
        if is_local && let (Some(pid), Some(creation_time)) = (started.pid, started.creation_time) {
            let _ = store::insert_session(
                &app.db,
                play_key(&found),
                pid,
                creation_time,
                folder,
                now.clone(),
            )
            .await;
        }
        if store::record_launch(&app.db, play_key(&found), now)
            .await
            .is_ok()
        {
            self.reload_plays().await;
            self.invalidate();
        }
        // The opener says whether the program broke away from a job of the backend. Only a
        // program that Rain started itself can be inside such a job: Steam, Epic and Riot start the
        // game from their own process.
        Ok(LaunchedAfterGameMode {
            in_job: is_local && started.in_job,
        })
    }
}
