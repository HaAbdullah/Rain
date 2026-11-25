//! Hardware: what the Hardware screen shows. Feature section 05.
//!
//! The screen is opened rarely, so this module costs nothing in between: no thread, no timer, no
//! cache and no work at start. `GET /hardware` reads the PC at the moment it is called; each
//! reading has its own time limit, so one slow device never blocks the others.
//!
//! What lives here:
//! - `inputs`: sorting the HID devices into keyboards, mice and Logitech channels (pure);
//! - `hidpp`: the read-only HID++ 2.0 conversation that gives a Logitech device its name,
//!   kind and battery (pure, over a `Port`);
//! - `vendor`: the table of vendors and their programs;
//! - `model`: `hardware.toml`, the owner's rules for the tiles;
//! - `source`: the seam of the tests, and the real PC behind it;
//! - `service` and `routes`.
//!
//! The only file of the owner is `hardware.toml`; the module has no tables.

mod hidpp;
mod inputs;
mod model;
mod routes;
mod service;
mod source;
mod vendor;

pub use hidpp::{Battery, DeviceKind, Port, Reading};
pub use inputs::{Endpoint, InputDevice, InputKind, Inputs};
pub use service::{DEVICE_LIMIT, DeviceLimit, Service};
pub use source::{Source, SystemSource};

use std::sync::Arc;

use super::{ModuleEntry, nothing, ready};

pub const ENTRY: ModuleEntry = ModuleEntry {
    name: "hardware",
    migrations: &[],
    router: routes::router,
    init: |state| {
        let state = state.clone();
        Box::pin(async move {
            // A test replaces the source, so that no test touches a real device.
            let source = state
                .seam::<dyn Source>()
                .unwrap_or_else(|| Arc::new(SystemSource));
            let service = Service::open(&state, source).await?;
            state.slots.hardware.set(service);
            Ok(())
        })
    },
    // Nothing runs until the screen asks: no timer, no thread, no watcher of devices.
    start: |_| ready(),
    stop: |_| nothing(),
};
