//! Lifecycle: start with Windows, what happens when the shell reports its facts, and the policy
//! for stopping the backend. Feature section 06 (6.4, 6.5 and 6.9).
//!
//! The mechanism of a stop (the order of steps, the module budgets, the final checkpoint) is core
//! (`start.rs`, `core/shutdown.rs`). This module owns the rules: which stop reasons are refused
//! when, and the autostart entry of Windows. It has no tables and no routes.

mod autostart;
mod policy;
mod routes;
mod service;
mod store;

use std::sync::Arc;

pub use policy::check_stop;
pub use service::Service;

use super::{ModuleEntry, nothing, ready};
use crate::platform::registry::{RunKey, WindowsRunKey};

pub const ENTRY: ModuleEntry = ModuleEntry {
    name: "lifecycle",
    migrations: &[],
    router: routes::router,
    init: |state| {
        // A test replaces the registry entry with a fake, so that it never touches the real one.
        let run_key = state
            .seam::<dyn RunKey>()
            .unwrap_or_else(|| Arc::new(WindowsRunKey::real()));
        state.slots.lifecycle.set(Service::new(
            run_key,
            state.config.sandbox,
            state.shell_facts.clone(),
            Arc::clone(&state.health),
        ));
        ready()
    },
    start: |_| ready(),
    stop: |_| nothing(),
};
