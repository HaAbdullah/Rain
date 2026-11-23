//! Gamemode: game mode. Feature section 06 (sections 3.2, 5 and 6.7 to 6.10).
//!
//! Game mode ends what Rain itself started (its scheduled jobs and idle jobs), then the
//! background service and the window. It never ends a Claude session: those are listed as "left
//! running". The pieces:
//! - `machine`: the pure state machine (phases, choices, timers as data);
//! - `items`: the five rows of the checklist;
//! - `neighbours`: what game mode asks of the other modules, behind one trait (so a test can
//!   put fakes in its place);
//! - `service`: the driver task, the wind-down task and the effects of the machine;
//! - `store` and `recap`: the record in SQLite and the recap that is built from it;
//! - `routes`: `GET` and `POST /game-mode`, `POST /game-mode/choice`, `POST
//!   /game-mode/recap/seen`.

mod items;
mod machine;
mod neighbours;
mod recap;
mod routes;
mod service;
mod store;

use std::sync::Arc;

pub use machine::Timings;
pub use neighbours::{GameInfo, Neighbours, Production};
pub use service::Service;

use super::ModuleEntry;
use crate::store::migrations::Migration;

pub const ENTRY: ModuleEntry = ModuleEntry {
    name: "gamemode",
    migrations: &[
        Migration {
            id: 1,
            name: "create_record",
            sql: include_str!("migrations/0001_create_record.sql"),
        },
        Migration {
            id: 2,
            name: "launch_in_job",
            sql: include_str!("migrations/0002_launch_in_job.sql"),
        },
    ],
    router: routes::router,
    init: |state| {
        // A test replaces the neighbours and the timings, so that no real process is touched and
        // no test waits three minutes.
        let neighbours = state.seam::<dyn Neighbours>().unwrap_or_else(|| {
            Arc::new(Production::new(
                state.sessions(),
                state.jobs(),
                state.idle(),
                state.disk(),
                state.games(),
            ))
        });
        let timings = state
            .seam::<Timings>()
            .map(|timings| (*timings).clone())
            .unwrap_or_default();
        state
            .slots
            .gamemode
            .set(Service::new(state, neighbours, timings));
        super::ready()
    },
    start: |state| {
        Box::pin(async move {
            state.gamemode().load_last(&state).await;
            Ok(())
        })
    },
    // The driver ends with the shutdown token; the wind-down ends with the process.
    stop: |_| super::nothing(),
};
