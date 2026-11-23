//! Names that the shell, the backend, the hook and the tests must agree on: the pipe name, the
//! folder names, and whether this run is a sandbox. Foundation sections 1.3, 2.7 and 3.1.
//!
//! Each rule is a pure function that takes its inputs as arguments, so a test can try every case.
//! The few functions at the bottom read the real environment and call them.

use std::env;
use std::fmt;

/// Environment variables of Rain. They are read only by this module and by the backend's config.
pub const ENV_PIPE: &str = "RAIN_PIPE";
pub const ENV_DATA_DIR: &str = "RAIN_DATA_DIR";
pub const ENV_LOCAL_DIR: &str = "RAIN_LOCAL_DIR";
pub const ENV_LOG: &str = "RAIN_LOG";
pub const ENV_SANDBOX: &str = "RAIN_SANDBOX";
/// The Claude folder in a sandbox run (section 03). Only a throwaway folder is accepted.
pub const ENV_CLAUDE_DIR: &str = "RAIN_CLAUDE_DIR";

/// Longest allowed value of `RAIN_PIPE`, and longest user slug in a pipe name.
const PIPE_NAME_MAX: usize = 64;
const USER_SLUG_MAX: usize = 32;

/// Used in a pipe name when Windows gives no usable user name.
const FALLBACK_USER: &str = "user";

/// The text before the pipe name in a Windows pipe path.
const PIPE_PATH_PREFIX: &str = r"\\.\pipe\";

/// `RAIN_PIPE` holds something that cannot be a pipe name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidPipeName(String);

impl fmt::Display for InvalidPipeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{ENV_PIPE} must be 1 to {PIPE_NAME_MAX} characters from A-Z a-z 0-9 . _ - (got {:?})",
            self.0
        )
    }
}

impl std::error::Error for InvalidPipeName {}

/// Lower-cases a Windows user name and keeps only `a-z0-9`: every other character (one Unicode
/// character at a time) becomes `-`. The result is cut to 32 characters.
///
/// The same rule exists in `vite/pipe-proxy.ts` (it must loop over characters, not UTF-16 units).
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

/// Decides whether a run is a sandbox (foundation section 2.7).
///
/// `RAIN_SANDBOX=0` forces real mode and `1` forces sandbox mode. Otherwise a run is a sandbox
/// when it is a debug build, or when `RAIN_DATA_DIR`, `RAIN_LOCAL_DIR` or `RAIN_PIPE` is set
/// (the caller passes the first two as one flag, `data_dir_set`).
pub fn sandbox_for(
    debug_build: bool,
    sandbox_var: Option<&str>,
    data_dir_set: bool,
    pipe_set: bool,
) -> bool {
    match sandbox_var {
        Some("0") => false,
        Some("1") => true,
        _ => debug_build || data_dir_set || pipe_set,
    }
}

/// The pipe name, without the `\\.\pipe\` prefix.
///
/// An override (`RAIN_PIPE`) wins when it is valid. Otherwise the name is `rain-<user>` in a real
/// run and `rain-dev-<user>` in a sandbox, with `<user>` from [`user_slug`].
pub fn pipe_name_for(
    pipe_override: Option<&str>,
    user_name: &str,
    sandbox: bool,
) -> Result<String, InvalidPipeName> {
    if let Some(name) = pipe_override {
        return validate_pipe_name(name);
    }
    let mut user = user_slug(user_name);
    if user.is_empty() {
        user = FALLBACK_USER.to_owned();
    }
    Ok(if sandbox {
        format!("rain-dev-{user}")
    } else {
        format!("rain-{user}")
    })
}

fn validate_pipe_name(name: &str) -> Result<String, InvalidPipeName> {
    let allowed_char = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
    if (1..=PIPE_NAME_MAX).contains(&name.len()) && name.chars().all(allowed_char) {
        Ok(name.to_owned())
    } else {
        Err(InvalidPipeName(name.to_owned()))
    }
}

/// The full Windows path of a pipe: `\\.\pipe\<name>`.
pub fn pipe_path(pipe_name: &str) -> String {
    format!("{PIPE_PATH_PREFIX}{pipe_name}")
}

/// Name of the Rain folder under `%APPDATA%` and under `%LOCALAPPDATA%`.
pub fn app_folder_name(sandbox: bool) -> &'static str {
    if sandbox { "Rain-dev" } else { "Rain" }
}

/// Service name Rain uses in Windows Credential Manager.
pub fn credential_service(sandbox: bool) -> &'static str {
    if sandbox { "rain-dev" } else { "rain" }
}

/// True when this run must not touch real user data. Reads the environment.
pub fn sandbox() -> bool {
    sandbox_for(
        cfg!(debug_assertions),
        env::var(ENV_SANDBOX).ok().as_deref(),
        env::var_os(ENV_DATA_DIR).is_some() || env::var_os(ENV_LOCAL_DIR).is_some(),
        env::var_os(ENV_PIPE).is_some(),
    )
}

/// The pipe name of this run, without the `\\.\pipe\` prefix. Reads the environment.
pub fn pipe_name() -> Result<String, InvalidPipeName> {
    let pipe_override = env::var(ENV_PIPE).ok();
    let user_name = env::var("USERNAME").unwrap_or_default();
    pipe_name_for(pipe_override.as_deref(), &user_name, sandbox())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_slug_lowers_case_and_replaces_other_characters() {
        assert_eq!(user_slug("Ada Lovelace"), "ada-lovelace");
        assert_eq!(user_slug("a.b_c"), "a-b-c");
        assert_eq!(user_slug("user42"), "user42");
    }

    #[test]
    fn user_slug_replaces_each_unicode_character_with_one_dash() {
        assert_eq!(user_slug("Zoë"), "zo-");
        assert_eq!(user_slug("名前"), "--");
    }

    /// The same list is read by the test of `userSlug` in `vite/pipe-proxy.test.ts`, so the two
    /// implementations cannot drift apart (foundation 2.7).
    #[test]
    fn user_slug_gives_the_slugs_of_the_list_shared_with_typescript() {
        let list: serde_json::Value =
            serde_json::from_str(include_str!("../user-slug-cases.json")).unwrap();
        let cases = list["cases"].as_array().expect("the list has `cases`");
        assert!(!cases.is_empty());
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let slug = case["slug"].as_str().unwrap();
            assert_eq!(user_slug(name), slug, "slug of {name:?}");
        }
    }

    #[test]
    fn user_slug_is_cut_to_32_characters() {
        let slug = user_slug(&"x".repeat(100));
        assert_eq!(slug.len(), 32);
    }

    #[test]
    fn sandbox_rules_of_section_2_7() {
        // (debug build, RAIN_SANDBOX, RAIN_DATA_DIR set, RAIN_PIPE set) -> sandbox
        let cases = [
            (true, None, false, false, true),
            (false, None, false, false, false),
            (false, None, true, false, true),
            (false, None, false, true, true),
            (false, Some("1"), false, false, true),
            (true, Some("0"), false, false, false),
            (false, Some("0"), true, true, false),
            (false, Some("yes"), false, false, false),
            (true, Some("yes"), false, false, true),
        ];
        for (debug, var, data, pipe, expected) in cases {
            assert_eq!(
                sandbox_for(debug, var, data, pipe),
                expected,
                "{debug} {var:?} {data} {pipe}"
            );
        }
    }

    #[test]
    fn pipe_name_follows_the_sandbox_flag() {
        assert_eq!(pipe_name_for(None, "Ada", false).unwrap(), "rain-ada");
        assert_eq!(pipe_name_for(None, "Ada", true).unwrap(), "rain-dev-ada");
    }

    #[test]
    fn pipe_name_falls_back_when_the_user_name_is_empty() {
        assert_eq!(pipe_name_for(None, "", false).unwrap(), "rain-user");
    }

    #[test]
    fn a_valid_override_wins() {
        assert_eq!(
            pipe_name_for(Some("rain-test-1.a_B"), "Ada", false).unwrap(),
            "rain-test-1.a_B"
        );
    }

    #[test]
    fn an_invalid_override_is_an_error_with_a_clear_message() {
        for bad in [
            "",
            "has space",
            "a\\b",
            "a/b",
            "name:colon",
            "é",
            &"x".repeat(65),
        ] {
            let error = pipe_name_for(Some(bad), "Ada", false).unwrap_err();
            assert!(error.to_string().contains("RAIN_PIPE"), "{bad:?}");
        }
        assert!(pipe_name_for(Some(&"x".repeat(64)), "Ada", false).is_ok());
    }

    #[test]
    fn pipe_path_adds_the_windows_prefix() {
        assert_eq!(pipe_path("rain-ada"), r"\\.\pipe\rain-ada");
    }

    #[test]
    fn folder_and_credential_names_differ_between_real_and_sandbox() {
        assert_eq!(app_folder_name(false), "Rain");
        assert_eq!(app_folder_name(true), "Rain-dev");
        assert_eq!(credential_service(false), "rain");
        assert_eq!(credential_service(true), "rain-dev");
    }
}
