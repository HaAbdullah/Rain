//! Machine: how long the user has not touched the keyboard or the mouse. Feature section 05.
//!
//! One fact, read live: no timer, no thread, no cache, no table and no data file. The idle jobs
//! (section 03) sample `Service::idle_ms()` themselves; this module has no clock of its own.
//! There is no `store.rs`: nothing is stored.

mod routes;
mod service;

pub use service::{IdleSource, Service, SystemIdle};

use std::sync::Arc;

use super::{ModuleEntry, nothing, ready};

pub const ENTRY: ModuleEntry = ModuleEntry {
    name: "machine",
    migrations: &[],
    router: routes::router,
    init: |state| {
        // A test replaces the source, so that it controls what the idle time is.
        let source = state
            .seam::<dyn IdleSource>()
            .unwrap_or_else(|| Arc::new(SystemIdle));
        state.slots.machine.set(Service::new(source));
        ready()
    },
    start: |_| ready(),
    stop: |_| nothing(),
};
