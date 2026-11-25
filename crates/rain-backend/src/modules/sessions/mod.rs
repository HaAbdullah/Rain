//! Sessions: the Claude sessions that run on this PC, and the hooks that watch them. Feature
//! section 03 (the sessions half; scheduled jobs and idle jobs are the modules `jobs` and `idle`).
//!
//! | File | Does |
//! |---|---|
//! | `claude_paths` | where the Claude folder is (the one file besides `hooks_install` that may name it) |
//! | `registry` | reads the registry files of Claude Code, read only, treated as a format that may change |
//! | `transcript` | reads the end of a transcript, and maps it to a readable log |
//! | `book` | the session book: how events and the registry become a status |
//! | `summary` | what a tool call looks like, and the answers Rain gives to Claude Code |
//! | `hook_wire` | the body of `POST /internal/hook` |
//! | `focus` | brings the window of a session to the front |
//! | `seams` | the parts that touch the PC, behind traits so that tests can replace them |
//! | `hooks_install` | the installer of the hooks (the ONLY file that may name the Claude settings file) |
//! | `store` | the table of sessions that ended |
//! | `service`, `routes` | the module's logic and its HTTP routes |

mod book;
pub(crate) mod claude_paths;
mod focus;
mod hook_wire;
mod hooks_install;
mod registry;
mod routes;
mod seams;
mod service;
mod store;
mod summary;
mod transcript;

pub use hook_wire::{HookEnv, HookEnvelope, HookMode};
pub use seams::{ExitSource, ForwarderProgram, ProcInfo, ProcessProbe, WindowApi};
pub use service::{HookHold, HookReply, Service, TranscriptTiming};

use super::{ModuleEntry, ready};

pub const ENTRY: ModuleEntry = ModuleEntry {
    name: "sessions",
    migrations: store::MIGRATIONS,
    router: routes::router,
    init: |state| {
        state.slots.sessions.set(Service::new(state));
        ready()
    },
    start: |state| {
        Box::pin(async move {
            state.sessions().start().await;
            Ok(())
        })
    },
    stop: |state| {
        Box::pin(async move {
            state.sessions().stop().await;
        })
    },
};
