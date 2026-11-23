//! Jobs: scheduled jobs. Feature section 03.
//!
//! The definitions are the user's `jobs.toml`; the code ships no job. What lives here:
//! - `model` and `entry`: reading the file entry by entry (a bad entry is skipped and named);
//! - `schedule`: cron, the next occurrence and the label (the only file that knows `chrono`);
//! - `scheduler`: the one task and the pure rules of when a job is due and what is made up;
//! - `command`, `claude_stream`, `runner`, `log`: one run, from the command line to the log;
//! - `file_edit`: writing `enabled` back with the comments kept;
//! - `store`: the SQL; `service` and `routes`: the interface.
//!
//! `idle` uses `entry`, `runner`, `log` and `file_edit` for its own runs.

mod claude_stream;
pub(crate) mod command;
pub(crate) mod entry;
pub(crate) mod file_edit;
pub(crate) mod log;
mod model;
mod routes;
pub(crate) mod runner;
mod schedule;
mod scheduler;
mod service;
mod store;
mod tuning;

use anyhow::anyhow;

pub use log::LogStore;
pub use schedule::parse_iso;
pub use service::{Service, format_duration, game_mode_refusal, run_id};
pub use tuning::{ResumeSignal, Tuning};

use super::ModuleEntry;
use crate::store::migrations::Migration;

/// The tables of this module (section 03 5.1).
pub const MIGRATIONS: &[Migration] = &[Migration {
    id: 1,
    name: "create_jobs",
    sql: include_str!("migrations/0001_jobs.sql"),
}];

pub const ENTRY: ModuleEntry = ModuleEntry {
    name: "jobs",
    migrations: MIGRATIONS,
    router: routes::router,
    init: |state| {
        let state = state.clone();
        Box::pin(async move {
            let service = Service::open(&state)
                .await
                .map_err(|e| anyhow!("{}", e.body.message))?;
            state.slots.jobs.set(service);
            Ok(())
        })
    },
    start: |state| {
        Box::pin(async move {
            let jobs = state.jobs();
            jobs.start(&state).await
        })
    },
    stop: |state| {
        Box::pin(async move {
            state.jobs().stop().await;
        })
    },
};
