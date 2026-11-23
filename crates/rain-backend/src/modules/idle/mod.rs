//! Idle jobs: jobs that run while the user is away. Feature section 03.
//!
//! The definitions are the user's `idle-jobs.toml`; the code ships none. What lives here:
//! - `model`: reading the file entry by entry;
//! - `decide`: the rules as pure functions (when to start, freeze, resume, queue again);
//! - `service`: the sampler task (it exists only while an idle job is enabled), the runs and the
//!   interface for the other modules;
//! - `store` and `routes`.
//!
//! The child process, the job object and the log are the shared pieces of the `jobs` module.

mod decide;
mod model;
mod routes;
mod service;
mod store;

use anyhow::anyhow;

pub use service::{GamesState, MachineIdle, Service, parse_progress_line};

use super::ModuleEntry;
use crate::store::migrations::Migration;

/// The tables of this module (section 03 5.2).
pub const MIGRATIONS: &[Migration] = &[Migration {
    id: 1,
    name: "create_idle",
    sql: include_str!("migrations/0001_idle.sql"),
}];

pub const ENTRY: ModuleEntry = ModuleEntry {
    name: "idle",
    migrations: MIGRATIONS,
    router: routes::router,
    init: |state| {
        let state = state.clone();
        Box::pin(async move {
            let service = Service::open(&state)
                .await
                .map_err(|e| anyhow!("{}", e.body.message))?;
            state.slots.idle.set(service);
            Ok(())
        })
    },
    start: |state| {
        Box::pin(async move {
            let idle = state.idle();
            idle.start().await
        })
    },
    stop: |state| {
        Box::pin(async move {
            state.idle().stop().await;
        })
    },
};
