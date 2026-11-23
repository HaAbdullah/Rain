//! Machine: the PC's state and idle time. Feature section 05.
//! Built by its feature section. Until then this is a stub: a service with the interfaces that
//! other modules already call, empty routes and no tables.

mod routes;
mod service;
mod store;

pub use service::Service;

use super::{ModuleEntry, nothing, ready};

pub const ENTRY: ModuleEntry = ModuleEntry {
    name: "machine",
    // The migrations of this module's tables (`include_str!("migrations/0001_<name>.sql")`).
    migrations: &[],
    router: routes::router,
    init: |state| {
        state.slots.machine.set(Service);
        ready()
    },
    start: |_| ready(),
    stop: |_| nothing(),
};
