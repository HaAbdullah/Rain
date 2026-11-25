//! Dropping the variables of a Claude Code session that started Rain.
//!
//! When Rain is started from inside a Claude Code session, the backend inherits that session's
//! variables, and so would every terminal, app and game that Rain starts. A Claude Code opened
//! in such a terminal believes it is a child of the first session: no transcript, no colours.
//! The rule for which variables these are is `rain_types::env::claude_session_vars`.

use std::env;

use super::registry::{self, Hive};

/// Where Windows keeps the variables the user set (for the user and for the whole PC).
const USER_ENV_KEY: &str = "Environment";
const MACHINE_ENV_KEY: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";

/// Removes the session's variables from this process and returns their names. A variable that
/// the user also set in Windows gets the user's value back. Call it first thing in `main`.
pub fn drop_claude_session_vars() -> Vec<String> {
    let names: Vec<String> = env::vars_os()
        .filter_map(|(name, _)| name.into_string().ok())
        .collect();
    let dropped = rain_types::env::claude_session_vars(names.iter().map(String::as_str));
    for name in &dropped {
        // SAFETY: changing the environment is only unsound where another thread may read it
        // through C `getenv` at the same moment. Windows guards its environment block with a
        // lock (the standard library documents these calls as always safe on Windows), and
        // this runs at the top of `main`, before the backend starts a thread.
        unsafe {
            match users_own_value(name) {
                Some(value) => env::set_var(name, value),
                None => env::remove_var(name),
            }
        }
    }
    dropped
}

fn users_own_value(name: &str) -> Option<String> {
    [
        (Hive::CurrentUser, USER_ENV_KEY),
        (Hive::LocalMachine, MACHINE_ENV_KEY),
    ]
    .into_iter()
    .find_map(|(hive, key)| registry::read_string(hive, key, name).ok().flatten())
}
