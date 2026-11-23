//! What the backend needs to know before it starts: its pipe, its folders and its limits.
//! Foundation sections 1.3 (variables), 2.7 (sandbox) and 10.1 (folders).
//!
//! `Config` is a plain value. Only [`Config::from_env`] reads the environment, once, at start; the
//! tests build a `Config` by hand, so they can run in parallel.

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::machine_file;

use rain_types::ConfigIssue;
use rain_types::env::{
    ENV_CLAUDE_DIR, ENV_DATA_DIR, ENV_LOCAL_DIR, ENV_LOG, ENV_PIPE, ENV_SANDBOX, InvalidPipeName,
    app_folder_name, pipe_name_for, sandbox_for,
};
use rain_types::limits::{
    BODY_LIMIT, HEADER_READ_TIMEOUT, PIPE_MAX_CONNECTIONS, REQUEST_TIMEOUT, SSE_KEEPALIVE,
};

/// The log filter when `RAIN_LOG` is not set.
const DEFAULT_LOG_FILTER: &str = "info";

/// How long open connections get to finish when the backend stops (foundation section 2.6, step 4).
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// The limits of the server (foundation section 3.4). They are fields, and not only constants,
/// so that a test can make a limit small and reach it quickly.
#[derive(Debug, Clone)]
pub struct Limits {
    /// A client has this long to send the head of a request. hyper also applies it to a
    /// connection that is idle between two requests, so an idle connection ends after this.
    pub header_read_timeout: Duration,
    /// A handler that takes longer than this is answered with `unavailable`, reason `timeout`.
    pub request_timeout: Duration,
    /// A request body larger than this is refused with `invalid`, reason `bodyTooLarge`.
    pub body_limit: usize,
    /// More simultaneous connections than this are closed at once.
    pub max_connections: usize,
    /// Open connections get this long to finish when the backend stops.
    pub shutdown_grace: Duration,
    /// A comment line (`: ka`) is sent on a stream that was idle for this long.
    pub sse_keepalive: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            header_read_timeout: HEADER_READ_TIMEOUT,
            request_timeout: REQUEST_TIMEOUT,
            body_limit: BODY_LIMIT,
            max_connections: PIPE_MAX_CONNECTIONS,
            shutdown_grace: SHUTDOWN_GRACE,
            sse_keepalive: SSE_KEEPALIVE,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    /// The pipe name without the `\\.\pipe\` prefix.
    pub pipe_name: String,
    /// The folder of the user's TOML files. The backend creates it at start when it is missing.
    pub data_dir: PathBuf,
    /// The machine-local folder: logs now, later the database and the cache.
    pub local_dir: PathBuf,
    /// What an empty `dataDir` means: the data folder of a normal run.
    pub default_data_dir: PathBuf,
    /// True when `RAIN_DATA_DIR` chose the data folder. Then `machine.toml` is not read and the
    /// folder cannot be changed in Settings.
    pub data_dir_from_env: bool,
    /// `machine.toml`: where the data folder is remembered (foundation 10.1).
    pub machine_file: PathBuf,
    /// Problems found while the configuration was made (a `machine.toml` that cannot be used).
    /// The backend puts them into `GET /health` when it starts.
    pub startup_issues: Vec<ConfigIssue>,
    /// The Claude folder of a sandbox run (`RAIN_CLAUDE_DIR`), only when it lies inside a
    /// throwaway place. `None` in a real run and when the variable is not set or not accepted;
    /// the sessions module then uses the user's own folder.
    pub claude_dir: Option<PathBuf>,
    /// True when this run must not touch real user data.
    pub sandbox: bool,
    /// A log filter in `tracing` syntax, for example `info,rain_backend::disk=debug`.
    pub log_filter: String,
    /// Also write the log to the console (flag `--console`).
    pub log_to_console: bool,
    pub limits: Limits,
}

/// The values `Config` is made from. Reading them is separate from the rules, so a test can try
/// every combination without touching the real environment.
#[derive(Debug, Clone, Default)]
pub struct EnvVars {
    pub pipe: Option<String>,
    pub data_dir: Option<PathBuf>,
    pub local_dir: Option<PathBuf>,
    pub log: Option<String>,
    pub sandbox: Option<String>,
    pub requested_claude_dir: Option<PathBuf>,
    pub user_name: String,
    pub app_data: Option<PathBuf>,
    pub local_app_data: Option<PathBuf>,
    /// The system temporary folder: the one place besides the run's own folders where a sandbox
    /// run accepts a Claude folder.
    pub temp_dir: PathBuf,
}

impl EnvVars {
    fn read() -> Self {
        let text = |name: &str| env::var(name).ok();
        let path = |name: &str| env::var_os(name).map(PathBuf::from);
        Self {
            pipe: text(ENV_PIPE),
            data_dir: path(ENV_DATA_DIR),
            local_dir: path(ENV_LOCAL_DIR),
            log: text(ENV_LOG),
            sandbox: text(ENV_SANDBOX),
            requested_claude_dir: path(ENV_CLAUDE_DIR),
            user_name: text("USERNAME").unwrap_or_default(),
            app_data: path("APPDATA"),
            local_app_data: path("LOCALAPPDATA"),
            temp_dir: env::temp_dir(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(transparent)]
    InvalidPipeName(#[from] InvalidPipeName),
    #[error("the environment variable {0} is not set, so Rain does not know where its folder is")]
    MissingFolder(&'static str),
}

impl Config {
    /// The configuration of this process. Reads the environment, `machine.toml` and, for the
    /// Claude folder of a sandbox run, the disk.
    pub fn from_env() -> Result<Self, ConfigError> {
        let vars = EnvVars::read();
        let mut config = Self::from_vars(&vars, cfg!(debug_assertions))?;
        config.apply_machine_file();
        let accepted = config.accepted_claude_dir(&vars);
        Ok(Self {
            claude_dir: accepted,
            ..config
        })
    }

    /// The data folder that the user chose in Settings (`machine.toml`) replaces the default,
    /// unless `RAIN_DATA_DIR` decided. A folder that cannot be used falls back to the default for
    /// this run, with an issue, and the file is not changed (section 06 4.2).
    pub fn apply_machine_file(&mut self) {
        if self.data_dir_from_env {
            return;
        }
        let machine = machine_file::read(&self.machine_file);
        self.startup_issues.extend(machine.issues);
        let Some(chosen) = machine.data_dir else {
            return;
        };
        match std::fs::create_dir_all(&chosen) {
            Ok(()) => self.data_dir = chosen,
            Err(_) => self.startup_issues.push(ConfigIssue {
                file: machine_file::FILE_NAME.to_owned(),
                line: None,
                message: format!(
                    "The data folder {} could not be used. Rain uses the default folder.",
                    chosen.display()
                ),
            }),
        }
    }

    /// `RAIN_CLAUDE_DIR` counts in a sandbox run only, and only for a folder inside the system
    /// temporary folder or this run's own folders, so that a test can never point Rain at the
    /// user's real Claude settings (foundation 2.7).
    fn accepted_claude_dir(&mut self, vars: &EnvVars) -> Option<PathBuf> {
        let requested = vars
            .requested_claude_dir
            .as_deref()
            .filter(|_| self.sandbox)?;
        let allowed = [&vars.temp_dir, &self.data_dir, &self.local_dir];
        if is_inside_any(requested, allowed) {
            Some(requested.to_path_buf())
        } else {
            self.startup_issues.push(ConfigIssue {
                file: ENV_CLAUDE_DIR.to_owned(),
                line: None,
                message: format!(
                    "{ENV_CLAUDE_DIR} is ignored: the folder must be inside the temporary folder or the data or local folder of this run."
                ),
            });
            None
        }
    }

    /// The rules behind [`Config::from_env`].
    pub fn from_vars(vars: &EnvVars, debug_build: bool) -> Result<Self, ConfigError> {
        let sandbox = sandbox_for(
            debug_build,
            vars.sandbox.as_deref(),
            vars.data_dir.is_some(),
            vars.pipe.is_some(),
        );
        let pipe_name = pipe_name_for(vars.pipe.as_deref(), &vars.user_name, sandbox)?;
        let folder = app_folder_name(sandbox);

        let default_data_dir = match &vars.data_dir {
            Some(dir) => dir.clone(),
            None => roaming(vars)?.join(folder).join("data"),
        };
        let data_dir = default_data_dir.clone();
        let machine_file = match &vars.app_data {
            Some(roaming) => roaming.join(folder).join(machine_file::FILE_NAME),
            None => sibling_file(&data_dir, machine_file::FILE_NAME),
        };
        let local_dir = match (&vars.local_dir, &vars.data_dir) {
            (Some(dir), _) => dir.clone(),
            (None, Some(data_dir)) => sibling_local_dir(data_dir),
            (None, None) => local(vars)?.join(folder),
        };

        Ok(Self {
            pipe_name,
            data_dir,
            local_dir,
            default_data_dir,
            data_dir_from_env: vars.data_dir.is_some(),
            machine_file,
            startup_issues: Vec::new(),
            claude_dir: None,
            sandbox,
            log_filter: vars
                .log
                .clone()
                .unwrap_or_else(|| DEFAULT_LOG_FILTER.to_owned()),
            log_to_console: false,
            limits: Limits::default(),
        })
    }

    /// A configuration for a test: the given pipe and folders, sandbox on, default limits.
    pub fn for_test(pipe_name: &str, data_dir: PathBuf, local_dir: PathBuf) -> Self {
        Self {
            pipe_name: pipe_name.to_owned(),
            machine_file: sibling_file(&data_dir, machine_file::FILE_NAME),
            default_data_dir: data_dir.clone(),
            data_dir_from_env: false,
            startup_issues: Vec::new(),
            claude_dir: None,
            data_dir,
            local_dir,
            sandbox: true,
            log_filter: DEFAULT_LOG_FILTER.to_owned(),
            log_to_console: false,
            limits: Limits::default(),
        }
    }
}

/// A file next to the data folder (not inside it): where a run without a roaming folder keeps
/// `machine.toml`.
fn sibling_file(data_dir: &Path, name: &str) -> PathBuf {
    data_dir.with_file_name(name)
}

/// Whether `path` lies inside one of `bases`, after resolving links and `..`. A path or base
/// that does not exist counts as outside.
fn is_inside_any<'a>(path: &Path, bases: impl IntoIterator<Item = &'a PathBuf>) -> bool {
    let Ok(path) = std::fs::canonicalize(path) else {
        return false;
    };
    bases
        .into_iter()
        .filter_map(|base| std::fs::canonicalize(base).ok())
        .any(|base| path.starts_with(base))
}

fn roaming(vars: &EnvVars) -> Result<&PathBuf, ConfigError> {
    vars.app_data
        .as_ref()
        .ok_or(ConfigError::MissingFolder("APPDATA"))
}

fn local(vars: &EnvVars) -> Result<&PathBuf, ConfigError> {
    vars.local_app_data
        .as_ref()
        .ok_or(ConfigError::MissingFolder("LOCALAPPDATA"))
}

/// The local folder that goes with a data folder from `RAIN_DATA_DIR`: the data folder's own path
/// with `.local` added, so that a test with a temporary data folder gets a temporary local folder.
fn sibling_local_dir(data_dir: &Path) -> PathBuf {
    let mut path: OsString = data_dir.as_os_str().to_owned();
    path.push(".local");
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> EnvVars {
        EnvVars {
            user_name: "Ada Lovelace".into(),
            app_data: Some(PathBuf::from(r"C:\Users\x\AppData\Roaming")),
            local_app_data: Some(PathBuf::from(r"C:\Users\x\AppData\Local")),
            ..EnvVars::default()
        }
    }

    #[test]
    fn a_real_run_uses_the_real_names_and_folders() {
        let config = Config::from_vars(&vars(), false).unwrap();
        assert!(!config.sandbox);
        assert_eq!(config.pipe_name, "rain-ada-lovelace");
        assert_eq!(
            config.data_dir,
            PathBuf::from(r"C:\Users\x\AppData\Roaming\Rain\data")
        );
        assert_eq!(
            config.local_dir,
            PathBuf::from(r"C:\Users\x\AppData\Local\Rain")
        );
    }

    #[test]
    fn a_debug_build_is_a_sandbox_with_its_own_names_and_folders() {
        let config = Config::from_vars(&vars(), true).unwrap();
        assert!(config.sandbox);
        assert_eq!(config.pipe_name, "rain-dev-ada-lovelace");
        assert!(
            config
                .data_dir
                .starts_with(r"C:\Users\x\AppData\Roaming\Rain-dev")
        );
        assert!(config.local_dir.ends_with("Rain-dev"));
    }

    #[test]
    fn the_overrides_make_a_run_a_sandbox_and_win() {
        let vars = EnvVars {
            pipe: Some("rain-test-1".into()),
            data_dir: Some(PathBuf::from(r"D:\tmp\data")),
            ..vars()
        };
        let config = Config::from_vars(&vars, false).unwrap();
        assert!(config.sandbox);
        assert_eq!(config.pipe_name, "rain-test-1");
        assert_eq!(config.data_dir, PathBuf::from(r"D:\tmp\data"));
        assert_eq!(config.local_dir, PathBuf::from(r"D:\tmp\data.local"));
    }

    #[test]
    fn an_explicit_local_folder_beats_the_sibling_folder() {
        let vars = EnvVars {
            data_dir: Some(PathBuf::from(r"D:\tmp\data")),
            local_dir: Some(PathBuf::from(r"D:\tmp\local")),
            ..vars()
        };
        let config = Config::from_vars(&vars, false).unwrap();
        assert_eq!(config.local_dir, PathBuf::from(r"D:\tmp\local"));
    }

    #[test]
    fn a_pipe_name_that_is_not_allowed_stops_the_start() {
        let vars = EnvVars {
            pipe: Some(r"bad\name".into()),
            ..vars()
        };
        assert!(matches!(
            Config::from_vars(&vars, false),
            Err(ConfigError::InvalidPipeName(_))
        ));
    }

    #[test]
    fn a_missing_app_data_folder_is_an_error_only_when_it_is_needed() {
        let without_folders = EnvVars {
            app_data: None,
            local_app_data: None,
            ..vars()
        };
        assert!(matches!(
            Config::from_vars(&without_folders, false),
            Err(ConfigError::MissingFolder("APPDATA"))
        ));
        let with_overrides = EnvVars {
            data_dir: Some(PathBuf::from(r"D:\d")),
            ..without_folders
        };
        assert!(Config::from_vars(&with_overrides, false).is_ok());
    }

    #[test]
    fn the_default_limits_are_the_ones_of_the_foundation() {
        let limits = Limits::default();
        assert_eq!(limits.body_limit, 1024 * 1024);
        assert_eq!(limits.request_timeout, Duration::from_secs(15));
        assert_eq!(limits.max_connections, 64);
    }

    #[test]
    fn the_log_filter_comes_from_rain_log() {
        let vars = EnvVars {
            log: Some("debug".into()),
            ..vars()
        };
        assert_eq!(Config::from_vars(&vars, false).unwrap().log_filter, "debug");
        assert_eq!(
            Config::from_vars(&self::vars(), false).unwrap().log_filter,
            "info"
        );
    }

    // ---- machine.toml and the Claude folder ----

    /// A roaming folder and a local folder in a temporary place, and the variables for them.
    fn temp_vars(temp: &std::path::Path) -> EnvVars {
        EnvVars {
            user_name: "Ada".into(),
            app_data: Some(temp.join("roaming")),
            local_app_data: Some(temp.join("local")),
            temp_dir: temp.join("system-temp"),
            ..EnvVars::default()
        }
    }

    #[test]
    fn the_machine_file_names_the_data_folder_and_the_default_stays_known() {
        let temp = tempfile::tempdir().unwrap();
        let vars = temp_vars(temp.path());
        let mut config = Config::from_vars(&vars, true).unwrap();
        let default = config.data_dir.clone();
        let chosen = temp.path().join("my data");
        std::fs::create_dir_all(config.machine_file.parent().unwrap()).unwrap();
        machine_file::write_data_dir(&config.machine_file, Some(&chosen)).unwrap();

        config.apply_machine_file();

        assert_eq!(config.data_dir, chosen);
        assert_eq!(config.default_data_dir, default);
        assert!(config.startup_issues.is_empty());
        assert!(chosen.is_dir(), "the folder was made");
        assert!(config.machine_file.ends_with(r"Rain-dev\machine.toml"));
    }

    #[test]
    fn rain_data_dir_beats_the_machine_file_and_the_file_is_not_read() {
        let temp = tempfile::tempdir().unwrap();
        let vars = EnvVars {
            data_dir: Some(temp.path().join("from env")),
            ..temp_vars(temp.path())
        };
        let mut config = Config::from_vars(&vars, false).unwrap();
        std::fs::create_dir_all(config.machine_file.parent().unwrap()).unwrap();
        machine_file::write_data_dir(&config.machine_file, Some(&temp.path().join("other")))
            .unwrap();

        config.apply_machine_file();

        assert!(config.data_dir_from_env);
        assert_eq!(config.data_dir, temp.path().join("from env"));
    }

    #[test]
    fn a_data_folder_that_cannot_be_made_falls_back_to_the_default_with_an_issue() {
        let temp = tempfile::tempdir().unwrap();
        let vars = temp_vars(temp.path());
        let mut config = Config::from_vars(&vars, true).unwrap();
        let default = config.data_dir.clone();
        // A path below a file cannot be created.
        let blocker = temp.path().join("blocker");
        std::fs::write(&blocker, "x").unwrap();
        std::fs::create_dir_all(config.machine_file.parent().unwrap()).unwrap();
        machine_file::write_data_dir(&config.machine_file, Some(&blocker.join("data"))).unwrap();

        config.apply_machine_file();

        assert_eq!(config.data_dir, default);
        assert_eq!(config.startup_issues.len(), 1);
        assert_eq!(config.startup_issues[0].file, "machine.toml");
        assert!(
            config.startup_issues[0]
                .message
                .contains("Rain uses the default folder")
        );
        assert!(
            std::fs::read_to_string(&config.machine_file)
                .unwrap()
                .contains("blocker"),
            "the file is not changed"
        );
    }

    #[test]
    fn a_machine_file_from_a_newer_rain_is_ignored_with_an_issue() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = Config::from_vars(&temp_vars(temp.path()), true).unwrap();
        std::fs::create_dir_all(config.machine_file.parent().unwrap()).unwrap();
        std::fs::write(
            &config.machine_file,
            "schema_version = 2\ndata_dir = 'D:\\X'\n",
        )
        .unwrap();

        config.apply_machine_file();

        assert_eq!(config.data_dir, config.default_data_dir);
        assert_eq!(config.startup_issues[0].message, "Written by a newer Rain");
    }

    #[test]
    fn a_claude_folder_counts_in_a_sandbox_run_only_inside_a_throwaway_place() {
        let temp = tempfile::tempdir().unwrap();
        let system_temp = temp.path().join("system-temp");
        let throwaway = system_temp.join("claude-test");
        std::fs::create_dir_all(&throwaway).unwrap();
        let elsewhere = temp.path().join("somewhere-else");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let ask = |requested: &std::path::Path, debug: bool| {
            let vars = EnvVars {
                requested_claude_dir: Some(requested.to_path_buf()),
                ..temp_vars(temp.path())
            };
            let mut config = Config::from_vars(&vars, debug).unwrap();
            let accepted = config.accepted_claude_dir(&vars);
            (accepted, config.startup_issues.len())
        };

        assert_eq!(ask(&throwaway, true), (Some(throwaway.clone()), 0));
        assert_eq!(
            ask(&elsewhere, true).0,
            None,
            "outside the throwaway places"
        );
        assert_eq!(ask(&elsewhere, true).1, 1, "and it is reported");
        assert_eq!(
            ask(&throwaway, false),
            (None, 0),
            "a real run never takes it"
        );
        assert_eq!(
            ask(&temp.path().join("missing"), true).0,
            None,
            "a folder that is not there"
        );
    }
}
