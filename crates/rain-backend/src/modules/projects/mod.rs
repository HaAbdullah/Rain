//! Projects: the git repositories directly inside the project folders. Feature section 02.
//!
//! What lives here:
//! - `discovery`: finding the repositories of the folders in `projectRoots`;
//! - `git`, `facts`, `dates`: the branch, the changed files and the commits of 14 days, cached
//!   in SQLite and refreshed when a client asks, never on a timer;
//! - `env` and `tools`: `projects.toml` (what opening a project does) and finding the programs
//!   (editor, Windows Terminal, Claude Code, git);
//! - `launch`: the command lines and the plan that the launcher engine runs;
//! - `open`: which projects have an editor window open;
//! - `store`: the tables;
//! - `service` and `routes`: the list, the topic, and the interface for the other modules.

mod dates;
mod discovery;
mod env;
mod facts;
mod git;
pub(crate) mod ids;
mod launch;
mod open;
mod routes;
mod service;
mod store;
mod tools;

use anyhow::anyhow;
pub use git::{GitCall, GitError, GitOutput, GitRunner};
pub use service::{Refresh, Service};
pub use tools::ToolEnv;

use super::ModuleEntry;
use crate::store::migrations::Migration;

/// The tables of this module (section 5.1).
pub const MIGRATIONS: &[Migration] = &[Migration {
    id: 1,
    name: "create_projects",
    sql: include_str!("migrations/0001_projects.sql"),
}];

pub const ENTRY: ModuleEntry = ModuleEntry {
    name: "projects",
    migrations: MIGRATIONS,
    router: routes::router,
    init: |state| {
        let state = state.clone();
        Box::pin(async move {
            let service = Service::open(&state).await?;
            state.slots.projects.set(service);
            Ok(())
        })
    },
    start: |state| {
        Box::pin(async move {
            state
                .projects()
                .start(state.clone())
                .await
                .map_err(|e| anyhow!("{e}"))
        })
    },
    stop: |state| {
        Box::pin(async move {
            state.projects().stop();
        })
    },
};
