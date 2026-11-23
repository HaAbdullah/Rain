//! The name of the pipe of the backend. A copy of `rain_types::env::pipe_name_for` (the hook has
//! no dependencies, so it cannot use that crate). A test in `rain-backend`
//! (`tests/hook_sources.rs`) includes this file and compares it with the original on a list of
//! user names, so the two cannot drift apart. Keep this file free of `crate::` paths for that
//! reason.

const PIPE_NAME_MAX: usize = 64;
const USER_SLUG_MAX: usize = 32;
const FALLBACK_USER: &str = "user";

/// Lower-cases a Windows user name and keeps only `a-z0-9`: every other character (one Unicode
/// character at a time) becomes `-`. The result is cut to 32 characters.
pub fn user_slug(user_name: &str) -> String {
    user_name
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                c
            } else {
                '-'
            }
        })
        .take(USER_SLUG_MAX)
        .collect()
}

/// The pipe name without the `\\.\pipe\` prefix: the override when it is valid, else
/// `rain-<user>` (`rain-dev-<user>` in a sandbox). `None` for an override that cannot be a pipe
/// name: the hook then does nothing.
pub fn pipe_name_for(
    pipe_override: Option<&str>,
    user_name: &str,
    sandbox: bool,
) -> Option<String> {
    if let Some(name) = pipe_override {
        let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
        let valid = (1..=PIPE_NAME_MAX).contains(&name.len()) && name.chars().all(allowed);
        return valid.then(|| name.to_owned());
    }
    let mut user = user_slug(user_name);
    if user.is_empty() {
        user = FALLBACK_USER.to_owned();
    }
    Some(if sandbox {
        format!("rain-dev-{user}")
    } else {
        format!("rain-{user}")
    })
}
