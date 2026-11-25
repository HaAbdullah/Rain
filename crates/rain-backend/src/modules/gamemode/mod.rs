//! Gamemode: game mode. Feature section 06 (sections 3.2, 5 and 6.7 to 6.10).
//!
//! Game mode closes the other programs of the PC (politely first, never a program that shows a
//! window), ends what Rain itself started (its scheduled jobs and idle jobs), then the background
//! service and the window. It never ends a Claude session: those are listed as "left running".
//! The pieces:
//! - `machine`: the pure state machine (phases, choices, timers as data);
//! - `items`: the six rows of the checklist;
//! - `others`: the other programs: which ones, the keep file `gamemode.toml`, the closing order;
//! - `neighbours`: what game mode asks of the other modules, behind one trait (so a test can
//!   put fakes in its place);
//! - `service`: the driver task, the wind-down task and the effects of the machine;
//! - `store` and `recap`: the record in SQLite and the recap that is built from it;
//! - `routes`: `GET` and `POST /game-mode`, `POST /game-mode/choice`, `POST
//!   /game-mode/recap/seen`, `PATCH /game-mode/other-apps`.

mod items;
mod machine;
mod neighbours;
mod others;
mod recap;
mod routes;
mod service;
mod store;

use std::sync::Arc;

pub use machine::Timings;
pub use neighbours::{GameInfo, Neighbours, Production};
pub use others::{CloseTimings, NoPc, Pc, Scan, SystemPc};
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
        Migration {
            id: 3,
            name: "other_apps",
            sql: include_str!("migrations/0003_other_apps.sql"),
        },
    ],
    router: routes::router,
    init: |state| {
        let state = state.clone();
        Box::pin(async move {
            // A test replaces the neighbours and the timings, so that no real process is touched
            // and no test waits three minutes.
            let had_neighbours = state.seam::<dyn Neighbours>().is_some();
            let neighbours = state.seam::<dyn Neighbours>().unwrap_or_else(|| {
                Arc::new(Production::new(
                    state.sessions(),
                    state.jobs(),
                    state.idle(),
                    state.disk(),
                    state.games(),
                    state.launcher(),
                ))
            });
            let timings = state
                .seam::<Timings>()
                .map(|timings| (*timings).clone())
                .unwrap_or_default();
            // The real PC, unless a test replaced it. A sandbox run and a test that replaced the
            // neighbours but not this part get a PC with nothing on it: they can never close a
            // real program.
            let pc = state.seam::<dyn Pc>().unwrap_or_else(|| {
                if had_neighbours || state.config.sandbox {
                    Arc::new(NoPc)
                } else {
                    Arc::new(SystemPc)
                }
            });
            let icons = Arc::new(others::ProcIcons::default());
            state.media.register("proc-icon", icons.clone());
            let others = others::Others::open(&state.hub, pc, icons, timings.closing.clone())
                .await
                .map_err(|e| anyhow::anyhow!("{}", e.body.message))?;
            state
                .slots
                .gamemode
                .set(Service::new(&state, neighbours, timings, others));
            Ok(())
        })
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
