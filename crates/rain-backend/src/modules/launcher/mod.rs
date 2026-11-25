//! Launcher: apps, pools, pins, recent items, search and the launch engine. Feature section 01.
//!
//! What lives here:
//! - `apps`: the app list from `shell:AppsFolder`, its filter, ids, kinds and keywords;
//! - `score` and `search`: the scorer (a port of `src/lib/score.ts`) and the ranking;
//! - `store`: the three TOML files and the two SQLite tables;
//! - `pools`: the rules for a pool draft;
//! - `launch`: the launch engine (how a step starts a program, the registry of runs);
//! - `opener`: the production `Opener`, the one place that starts programs;
//! - `icons` and `running`: app icons as media, and the "is it running" hint;
//! - `service` and `routes`: the interface for the other modules and the HTTP routes.
//!
//! Projects and games are built by another section. They reach the launcher through the two
//! traits of `core::launch_contract` (`ItemSource`, `LaunchPlanner`), so search, pins, recent and
//! launch already work for them as soon as their services answer.

mod apps;
mod icons;
mod launch;
mod limits;
mod opener;
mod other_items;
mod pools;
mod routes;
mod running;
mod score;
mod search;
mod service;
mod store;

pub use apps::{AppRecord, AppSource, SourceError};
pub use icons::IconRenderer;
pub use launch::PathProbe;
pub use opener::WindowsOpener;
pub use service::{AppFacts, FaultMode, ScanError, Service, pin_kind};

use anyhow::anyhow;

use super::ModuleEntry;

pub const ENTRY: ModuleEntry = ModuleEntry {
    name: "launcher",
    migrations: store::MIGRATIONS,
    router: routes::router,
    init: |state| {
        let state = state.clone();
        Box::pin(async move {
            let service = Service::new(&state).await.map_err(|e| anyhow!("{e}"))?;
            Service::register(&state);
            state.slots.launcher.set(service);
            Ok(())
        })
    },
    start: |state| {
        state.launcher().start(&state);
        super::ready()
    },
    stop: |state| {
        Box::pin(async move {
            state.launcher().stop().await;
        })
    },
};
