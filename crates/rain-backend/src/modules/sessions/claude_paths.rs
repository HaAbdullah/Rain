//! Where the Claude folder is, and the paths inside it that Rain READS. Section 03 6.1.
//!
//! This is the one file (besides `hooks_install.rs`) that may name the Claude folder: the source
//! scan in `tests/source_rules.rs` fails on `.claude` anywhere else. Everything here only builds
//! paths and never opens a file for writing. The settings file is not named here at all: only
//! `hooks_install.rs` names it, because it is the only code that may change it.

use std::path::{Path, PathBuf};

use crate::config::Config;

/// The folder that Claude Code keeps its own files in.
#[derive(Debug, Clone)]
pub struct ClaudePaths {
    root: PathBuf,
}

impl ClaudePaths {
    /// The folder of this run, in this order:
    ///
    /// 1. `Config.claude_dir`: a throwaway folder that a test or `RAIN_CLAUDE_DIR` chose;
    /// 2. the environment variable `CLAUDE_CONFIG_DIR`, which Claude Code itself honours;
    /// 3. `.claude` in the user's profile folder.
    pub fn resolve(config: &Config) -> Self {
        let root = config
            .claude_dir
            .clone()
            .or_else(|| std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from))
            .or_else(|| {
                std::env::var_os("USERPROFILE").map(|home| Path::new(&home).join(".claude"))
            })
            .unwrap_or_default();
        Self { root }
    }

    #[cfg(test)]
    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// True when the folder exists, which means Claude Code has run on this PC.
    pub fn exists(&self) -> bool {
        self.root.is_dir()
    }

    /// The registry: one small file per running session, `<pid>.json`.
    pub fn registry_dir(&self) -> PathBuf {
        self.root.join("sessions")
    }

    /// Where Claude Code writes the transcripts: one folder per working folder.
    pub fn projects_dir(&self) -> PathBuf {
        self.root.join("projects")
    }

    /// The transcript of a session when the hook did not tell the path: the working folder with
    /// every character that is not a letter or a digit replaced by `-`, then `<id>.jsonl`.
    pub fn guess_transcript(&self, cwd: &str, session_id: &str) -> PathBuf {
        let folder: String = cwd
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        self.projects_dir()
            .join(folder)
            .join(format!("{session_id}.jsonl"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_in_the_config_wins() {
        let config = Config::for_test("p", "d".into(), "l".into());
        let paths = ClaudePaths::resolve(&config);
        assert_eq!(paths.root(), Path::new("l").join("claude"));
        assert_eq!(paths.registry_dir(), Path::new("l/claude/sessions"));
    }

    #[test]
    fn the_transcript_guess_replaces_every_other_character() {
        let paths = ClaudePaths::with_root(PathBuf::from("root"));
        let guessed = paths.guess_transcript(r"C:\work\my project", "abc");
        assert_eq!(
            guessed,
            Path::new("root")
                .join("projects")
                .join("C--work-my-project")
                .join("abc.jsonl")
        );
    }

    #[test]
    fn a_missing_folder_does_not_exist() {
        assert!(!ClaudePaths::with_root(PathBuf::from("no-such-folder-here")).exists());
    }
}
