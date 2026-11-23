//! What every part of the backend shares: state, the event bus, errors, health, time, shutdown
//! and logging. Foundation section 11.1.

pub mod blocking;
pub mod bus;
pub mod clock;
pub mod error;
pub mod health;
pub mod launch_contract;
pub mod logging;
pub mod media;
pub mod paths;
pub mod secrets;
pub mod shell_facts;
pub mod shutdown;
pub mod state;
pub mod task;
pub mod throttle;
pub mod winddown;
