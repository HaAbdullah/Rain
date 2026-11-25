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
    /// Folders that may hold the data of the Claude desktop app (see `app_session_folders`).
    app_parents: Vec<PathBuf>,
}

impl ClaudePaths {
    /// The folder of this run, in this order:
    ///
    /// 1. `Config.claude_dir`: a throwaway folder that a test or `RAIN_CLAUDE_DIR` chose;
    /// 2. the environment variable `CLAUDE_CONFIG_DIR`, which Claude Code itself honours;
    /// 3. `.claude` in the user's profile folder.
    pub fn resolve(config: &Config) -> Self {
        let overridden = config.claude_dir.is_some();
        let root = config
            .claude_dir
            .clone()
            .or_else(|| std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from))
            .or_else(|| {
                std::env::var_os("USERPROFILE").map(|home| Path::new(&home).join(".claude"))
            })
            .unwrap_or_default();
        // A throwaway Claude folder carries its own copy of the app's data, so that a test or a
        // sandbox run never reads the real one.
        let app_parents = if overridden {
            vec![root.join("desktop-app")]
        } else {
            let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
            let roaming = std::env::var_os("APPDATA").map(PathBuf::from);
            [
                local.map(|dir| dir.join("Claude-Data")),
                roaming.map(|dir| dir.join("Claude")),
            ]
            .into_iter()
            .flatten()
            .collect()
        };
        Self { root, app_parents }
    }

    #[cfg(test)]
    pub fn with_root(root: PathBuf) -> Self {
        Self {
            root,
            app_parents: Vec::new(),
        }
    }

    /// The folders `claude-code-sessions` of the Claude desktop app: one for each profile folder
    /// below `Claude-Data` (named after the app package), and one directly in the older
    /// `%APPDATA%\Claude`. Only folders that exist are returned. The app keeps its list of
    /// scheduled tasks and one small file for each of its sessions there.
    pub fn app_session_folders(&self) -> Vec<PathBuf> {
        const NAME: &str = "claude-code-sessions";
        let mut found = Vec::new();
        for parent in &self.app_parents {
            let direct = parent.join(NAME);
            if direct.is_dir() {
                found.push(direct);
            }
            let Ok(entries) = std::fs::read_dir(parent) else {
                continue;
            };
            for entry in entries.flatten() {
                let nested = entry.path().join(NAME);
                if nested.is_dir() {
                    found.push(nested);
                }
            }
        }
        found
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
        self.projects_dir()
            .join(project_folder_name(cwd))
            .join(format!("{session_id}.jsonl"))
    }

    /// The file where Claude Code keeps its own state (the MCP servers of the user among it).
    /// It lies next to the Claude folder, or inside a Claude folder that was chosen by hand.
    /// Only the file name is built here: the library reads the names of the MCP servers from it.
    pub fn state_file(&self) -> Option<PathBuf> {
        let inside = self.root.join(".claude.json");
        if inside.is_file() {
            return Some(inside);
        }
        let named_dot_claude = self.root.file_name().is_some_and(|name| name == ".claude");
        let beside = self.root.parent()?.join(".claude.json");
        (named_dot_claude && beside.is_file()).then_some(beside)
    }
}

/// The folder of Claude Code for a working folder: every character that is not a letter or a
/// digit is replaced by `-`.
pub fn project_folder_name(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The Claude folder inside a project folder (skills, agents, commands, instructions).
pub fn project_claude_dir(project: &Path) -> PathBuf {
    project.join(".claude")
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
