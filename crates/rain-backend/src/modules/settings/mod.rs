//! Settings: `settings.toml` and the `Settings` resource that the Settings screen shows.
//! Feature section 06 (sections 3, 4.1, 6.1 to 6.3).
//!
//! What lives here:
//! - `store`: the schema of `settings.toml`, and how a change is written into it;
//! - `rules` and `patch`: the validation of table V and the plan of a `PATCH`;
//! - `detect`: what Rain finds out about this PC (Obsidian vaults, Steam, Epic);
//! - `resolved`: the values as the other modules read them (`Service::resolved()`);
//! - `datadir`: changing the data folder (`machine.toml`);
//! - `service` and `routes`: `GET` and `PATCH /settings`, and the interface for the other
//!   modules (`resolved`, `watch`, `expand_env`, `vault_id`).
//!
//! The effects of a few settings belong to other modules: start with Windows is `lifecycle`, the
//! Claude switch is `sessions`. This module calls them.

mod datadir;
mod detect;
mod patch;
mod resolved;
mod routes;
mod rules;
mod service;
mod store;

use std::sync::Arc;

use anyhow::anyhow;
pub use detect::{
    Detect, Detected, EPIC_JOURNAL_FILE, NoteSettings, SystemDetect, VaultEntry, epic_folder,
    epic_v2_folder, parse_daily_notes_json, parse_obsidian_json, riot_folder,
};
pub use resolved::Resolved;
pub use service::Service;

use super::ModuleEntry;

/// Names this module on the errors it raises.
const SOURCE: &str = "settings";

pub const ENTRY: ModuleEntry = ModuleEntry {
    name: SOURCE,
    migrations: &[],
    router: routes::router,
    init: |state| {
        let state = state.clone();
        Box::pin(async move {
            let file = store::open(&state.hub)
                .await
                .map_err(|e| anyhow!("{}", e.body.message))?;
            // A test replaces the detectors with a fake, so that it never reads the real PC.
            let detect = state
                .seam::<dyn Detect>()
                .unwrap_or_else(|| Arc::new(SystemDetect));
            state
                .slots
                .settings
                .set(Service::start(file, &state, detect).await);
            Ok(())
        })
    },
    start: |state| {
        // The task ends with the shutdown token, so `stop` has nothing to do.
        let service = state.settings();
        crate::core::task::spawn_named(
            "settings-publish",
            service.publish_changes(state.shutdown.token().clone()),
        );
        super::ready()
    },
    stop: |_| super::nothing(),
};
