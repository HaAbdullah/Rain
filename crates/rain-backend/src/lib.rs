//! The Rain backend: the always-on program that holds every feature of Rain.
//! Design: docs/backend/00-foundation.md (sections 2, 3, 5 and 11 describe this crate).
//!
//! The backend is a library plus a thin program (`main.rs`), so that the tests can start it
//! inside the test process, on a pipe of their own, and still use the real transport.

#![deny(unsafe_code)]

pub mod config;
pub mod core;
pub mod machine_file;
pub mod modules;
pub mod platform;
pub mod server;
mod start;
mod stop;
pub mod store;

pub use start::{Running, StartError, run, start};
pub use stop::{StopOutcome, stop};

/// What `GET /ping` calls this program. It tells Rain's backend from any other program on the pipe.
pub const APP_NAME: &str = "rain-backend";

/// The version of the API (the `/api/v1` of every route). Foundation section 4.1.
pub const API_VERSION: u32 = 1;
