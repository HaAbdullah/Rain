//! Disk: drives, the disk scan with its budgeted tree, revealing a file in Explorer, and the
//! path check that Settings uses. Feature section 05.
//!
//! What lives here:
//! - `drives`: the fixed local drives (`GET /drives`);
//! - `walk`: the parallel folder walker, and `arena`: the finished scan in memory and the small
//!   tree that the screen gets from it;
//! - `category`: the colour class of a file, by extension;
//! - `store`: the scans that exist, the one tree that is held, and the timers around them;
//! - `paths`: `POST /paths/check` and `POST /shell/reveal`;
//! - `service` and `routes`.
//!
//! This module has no tables and no data file: a scan result is too big to keep and too short
//! lived to be worth writing (section 05, sections 4 and 5).

mod arena;
mod category;
mod drives;
mod paths;
mod routes;
mod service;
mod store;
mod walk;

pub use drives::{DriveInfo, DriveSource, NO_LABEL};
pub use paths::{PathProbe, Probe, Revealer};
pub use service::Service;
pub use walk::{DirSource, RawEntry};

use super::{ModuleEntry, nothing, ready};

pub const ENTRY: ModuleEntry = ModuleEntry {
    name: "disk",
    // No tables: see the module text.
    migrations: &[],
    router: routes::router,
    init: |state| {
        state.slots.disk.set(Service::new(state));
        ready()
    },
    // Nothing runs until a scan is asked for: no timer, no thread, no watcher.
    start: |_| ready(),
    stop: |state| {
        state.disk().stop();
        nothing()
    },
};
