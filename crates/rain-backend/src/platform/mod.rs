//! All unsafe code and raw Windows calls of the backend live in this folder, and nowhere else.
//! Foundation section 11.1. Every `unsafe` block says why it is sound.
//!
//! What each file offers:
//!
//! | File | Offers |
//! |---|---|
//! | `com` | the one COM thread and `com::run` |
//! | `shell` | `shell_execute` (the one launch call), `reveal`, `list_apps` |
//! | `icons` | an icon as PNG for a file, a shortcut or an app id |
//! | `known_folders` | `%APPDATA%`, the Start Menu folders and the like, from Windows |
//! | `registry` | reading values, and the autostart entry (`RunKey`) |
//! | `session_env` | dropping the variables of a Claude Code session that started Rain |
//! | `process` | `hidden_command`, process lists, `list_facts` (owner, memory, program file), `terminate_if_same`, `spawn_detached`, `ExitWatcher` |
//! | `job` | `OwnedJob`: a process tree that dies with Rain, with its output captured |
//! | `windows` | the visible top-level windows; every top-level window and `post_close` (game mode) |
//! | `windows_focus` | the windows with their handles, and bringing one to the front |
//! | `idle` | milliseconds since the last keyboard or mouse input |
//! | `fs` | the fixed drives and their sizes |
//! | `hid` | the HID devices that are plugged in, and a channel to one |
//! | `time` | the local calendar date |
//! | `power` | a notice when the PC wakes up |
//! | `thread` | below-normal priority for a worker thread |
//! | `pipe_acl`, `console` | the pipe's access list; output for `--stop` in a terminal |
#![allow(unsafe_code)]

pub mod com;
pub mod console;
pub mod fs;
pub mod hid;
pub mod icons;
pub mod idle;
pub mod job;
pub mod known_folders;
pub mod pipe_acl;
pub mod power;
pub mod process;
pub mod registry;
pub mod session_env;
pub mod shell;
pub mod thread;
pub mod time;
pub mod windows;
pub mod windows_focus;
