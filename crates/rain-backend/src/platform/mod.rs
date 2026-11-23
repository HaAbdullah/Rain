//! All unsafe code and raw Windows calls of the backend live in this folder, and nowhere else.
//! Foundation section 11.1. Every `unsafe` block says why it is sound.
//!
//! What each file offers:
//!
//! | File | Offers |
//! |---|---|
//! | `com` | the one COM thread and `com::run` |
//! | `shell` | `shell_execute` (the one launch call), `reveal`, `list_apps`, `resolve_shortcut` |
//! | `icons` | an icon as PNG for a file, a shortcut or an app id |
//! | `known_folders` | `%APPDATA%`, the Start Menu folders and the like, from Windows |
//! | `registry` | reading values, and the autostart entry (`RunKey`) |
//! | `process` | `hidden_command`, process lists, `spawn_detached`, `ExitWatcher` |
//! | `job` | `OwnedJob`: a process tree that dies with Rain, with its output captured |
//! | `windows` | the visible top-level windows |
//! | `windows_focus` | the windows with their handles, and bringing one to the front |
//! | `idle` | milliseconds since the last keyboard or mouse input |
//! | `fs` | the fixed drives and their sizes |
//! | `time` | the local calendar date |
//! | `power` | a notice when the PC wakes up |
//! | `thread` | below-normal priority for a worker thread |
//! | `pipe_acl`, `console` | the pipe's access list; output for `--stop` in a terminal |
#![allow(unsafe_code)]

pub mod com;
pub mod console;
pub mod fs;
pub mod icons;
pub mod idle;
pub mod job;
pub mod known_folders;
pub mod pipe_acl;
pub mod power;
pub mod process;
pub mod registry;
pub mod shell;
pub mod thread;
pub mod time;
pub mod windows;
pub mod windows_focus;
