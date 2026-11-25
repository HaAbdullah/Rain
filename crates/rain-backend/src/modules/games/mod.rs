//! Games: Steam, Epic, game folders and hand-added games. Feature section 02.
//!
//! What lives here:
//! - `steam`, `epic`, `epic_data`, `riot`, `folders`: the kinds of source (installed games of the
//!   signed-in Steam account, the old Epic launcher's manifests and the new one's journal, the Riot
//!   Client's product folders, and the folders of the setting `gameFolders`);
//! - `guess`: which program file starts a game folder (pure scoring);
//! - `config`: `games.toml`, the user's corrections and hand-added games;
//! - `library`: puts the sources together; `edit`: what the user changes;
//! - `launch` and `running`: starting a game, play time, which games run;
//! - `covers` and `ico`: cover pictures (and the biggest picture of an icon file) and the media
//!   provider `game-cover`;
//! - `store`: the SQLite tables (play time, running sessions, caches).
//!
//! The launcher (section 01) uses `Service` through the launch contract (`ItemSource`,
//! `LaunchPlanner`); game mode (section 06) uses `Service::get` and `launch_after_game_mode`.

mod config;
mod covers;
mod edit;
mod epic;
mod epic_data;
mod folders;
mod guess;
mod ico;
mod launch;
mod library;
mod model;
mod names;
mod riot;
mod routes;
mod running;
mod service;
mod steam;
mod store;
mod vdf;

use std::sync::Arc;

use anyhow::anyhow;

pub use running::ProcessWaiter;
pub use service::{LaunchedAfterGameMode, Service};
pub use steam::SteamRegistry;

use super::ModuleEntry;
use crate::store::toml_file::TomlFile;

pub const ENTRY: ModuleEntry = ModuleEntry {
    name: "games",
    migrations: store::MIGRATIONS,
    router: routes::router,
    init: |state| {
        let state = state.clone();
        Box::pin(async move {
            let file =
                TomlFile::<config::GamesDoc>::open(&state.hub, config::FILE_NAME, |_| Ok(()))
                    .await
                    .map_err(|e| anyhow!("{}", e.body.message))?;
            state.slots.games.set(Service::new(&state, file));
            state.media.register(
                "game-cover",
                Arc::new(covers::GameCovers {
                    state: state.downgrade(),
                }),
            );
            Ok(())
        })
    },
    start: |state| {
        Box::pin(async move {
            let service = state.games();
            service.start(state).await;
            Ok(())
        })
    },
    stop: |state| {
        Box::pin(async move {
            state.games().stop().await;
        })
    },
};
