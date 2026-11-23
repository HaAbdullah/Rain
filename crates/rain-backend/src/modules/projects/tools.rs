//! Finding the programs that a project open needs: the editor, Windows Terminal, Claude Code and
//! git. Section 02 6.5.
//!
//! Order, first hit wins: the path the user set in `projects.toml`, then every folder of `PATH`,
//! then the places where the program is usually installed. A path that the user set and that does
//! not exist is an error and not a reason to look further: a wrong setting must be seen.
//!
//! The answer is never cached. It costs a handful of `stat` calls, and the user may install a
//! program and click again a second later.

use std::path::{Path, PathBuf};

/// The four programs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Editor,
    WindowsTerminal,
    Claude,
    Git,
}

/// What the lookup found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    At(PathBuf),
    NotFound,
    /// The user set this path in `projects.toml` and there is no file there.
    ConfiguredMissing(PathBuf),
}

/// What the lookup needs from the PC, so that a test says what exists.
pub trait ToolEnv: Send + Sync {
    /// The folders of `PATH`, in order.
    fn path_dirs(&self) -> Vec<PathBuf>;
    /// An environment variable, if it is set.
    fn var(&self, name: &str) -> Option<String>;
    /// Whether a file exists. `symlink_metadata`, not `metadata`: `wt.exe` is an app execution
    /// alias (a reparse point) that `metadata` can refuse with os error 1920.
    fn exists(&self, path: &Path) -> bool;
}

/// The real PC.
pub struct SystemToolEnv;

impl ToolEnv for SystemToolEnv {
    fn path_dirs(&self) -> Vec<PathBuf> {
        std::env::var_os("PATH")
            .map(|paths| std::env::split_paths(&paths).collect())
            .unwrap_or_default()
    }

    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|value| !value.is_empty())
    }

    fn exists(&self, path: &Path) -> bool {
        std::fs::symlink_metadata(path).is_ok()
    }
}

/// What `[tools]` of `projects.toml` says. An empty text means "detect".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overrides {
    pub editor: String,
    pub terminal: String,
    /// Claude Code.
    pub coding_agent: String,
    pub git: String,
}

impl Overrides {
    fn for_tool(&self, tool: Tool) -> &str {
        match tool {
            Tool::Editor => &self.editor,
            Tool::WindowsTerminal => &self.terminal,
            Tool::Claude => &self.coding_agent,
            Tool::Git => &self.git,
        }
    }
}

/// The file names to look for in every folder of `PATH`.
fn path_names(tool: Tool) -> &'static [&'static str] {
    match tool {
        // `code.cmd` is found only to locate the program one folder up; it is never started.
        Tool::Editor => &["Code.exe", "code.cmd"],
        Tool::WindowsTerminal => &["wt.exe"],
        Tool::Claude => &["claude.exe", "claude.cmd"],
        Tool::Git => &["git.exe"],
    }
}

/// The usual install places, with `%VARIABLE%` parts. An entry whose variable is not set is
/// skipped.
fn install_places(tool: Tool) -> &'static [&'static str] {
    match tool {
        Tool::Editor => &[
            "%LOCALAPPDATA%\\Programs\\Microsoft VS Code\\Code.exe",
            "%ProgramFiles%\\Microsoft VS Code\\Code.exe",
            "%ProgramFiles(x86)%\\Microsoft VS Code\\Code.exe",
        ],
        Tool::WindowsTerminal => &["%LOCALAPPDATA%\\Microsoft\\Windows\\WindowsApps\\wt.exe"],
        Tool::Claude => &[
            "%USERPROFILE%\\.local\\bin\\claude.exe",
            "%APPDATA%\\npm\\claude.cmd",
        ],
        Tool::Git => &[
            "%ProgramFiles%\\Git\\cmd\\git.exe",
            "%ProgramFiles(x86)%\\Git\\cmd\\git.exe",
            "%LOCALAPPDATA%\\Programs\\Git\\cmd\\git.exe",
        ],
    }
}

/// Replaces every `%NAME%` by the variable. `None` when a variable is not set.
fn expand(template: &str, env: &dyn ToolEnv) -> Option<PathBuf> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find('%')?;
        out.push_str(&env.var(&after[..end])?);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Some(PathBuf::from(out))
}

/// `...\bin\code.cmd` stands next to the folder that holds `Code.exe`.
fn editor_from_cmd(cmd: &Path) -> Option<PathBuf> {
    Some(cmd.parent()?.parent()?.join("Code.exe"))
}

/// Looks for one program.
pub fn find(tool: Tool, overrides: &Overrides, env: &dyn ToolEnv) -> Lookup {
    let configured = overrides.for_tool(tool).trim();
    if !configured.is_empty() {
        let path = PathBuf::from(configured);
        return if env.exists(&path) {
            Lookup::At(path)
        } else {
            Lookup::ConfiguredMissing(path)
        };
    }
    for dir in env.path_dirs() {
        for name in path_names(tool) {
            let candidate = dir.join(name);
            if !env.exists(&candidate) {
                continue;
            }
            if tool == Tool::Editor && name.ends_with(".cmd") {
                match editor_from_cmd(&candidate).filter(|exe| env.exists(exe)) {
                    Some(exe) => return Lookup::At(exe),
                    None => continue,
                }
            }
            return Lookup::At(candidate);
        }
    }
    for place in install_places(tool) {
        if let Some(path) = expand(place, env)
            && env.exists(&path)
        {
            return Lookup::At(path);
        }
    }
    Lookup::NotFound
}

/// The image file names that count as "the editor is running" for the open detection: the
/// detected editor, else the two names of VS Code.
pub fn editor_image_names(found: &Lookup) -> Vec<String> {
    match found {
        Lookup::At(path) | Lookup::ConfiguredMissing(path) => path
            .file_name()
            .map(|name| vec![name.to_string_lossy().into_owned()])
            .unwrap_or_default(),
        Lookup::NotFound => vec!["Code.exe".to_owned(), "Code - Insiders.exe".to_owned()],
    }
}

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::collections::{HashMap, HashSet};

    /// A PC that is a table.
    #[derive(Default)]
    pub struct FakeEnv {
        pub path: Vec<PathBuf>,
        pub vars: HashMap<String, String>,
        pub files: HashSet<PathBuf>,
    }

    impl FakeEnv {
        pub fn with_file(mut self, path: &str) -> Self {
            self.files.insert(PathBuf::from(path));
            self
        }
    }

    impl ToolEnv for FakeEnv {
        fn path_dirs(&self) -> Vec<PathBuf> {
            self.path.clone()
        }

        fn var(&self, name: &str) -> Option<String> {
            self.vars.get(name).cloned()
        }

        fn exists(&self, path: &Path) -> bool {
            self.files.contains(path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeEnv;
    use super::*;

    fn env_with_path(dirs: &[&str]) -> FakeEnv {
        FakeEnv {
            path: dirs.iter().map(PathBuf::from).collect(),
            ..FakeEnv::default()
        }
    }

    #[test]
    fn a_configured_path_that_exists_wins() {
        let env = env_with_path(&["C:\\bin"])
            .with_file("C:\\bin\\git.exe")
            .with_file("D:\\g\\git.exe");
        let overrides = Overrides {
            git: "D:\\g\\git.exe".into(),
            ..Overrides::default()
        };
        assert_eq!(
            find(Tool::Git, &overrides, &env),
            Lookup::At(PathBuf::from("D:\\g\\git.exe"))
        );
    }

    #[test]
    fn a_configured_path_that_is_missing_is_an_error_without_a_fall_through() {
        let env = env_with_path(&["C:\\bin"]).with_file("C:\\bin\\git.exe");
        let overrides = Overrides {
            git: "D:\\nowhere\\git.exe".into(),
            ..Overrides::default()
        };
        assert_eq!(
            find(Tool::Git, &overrides, &env),
            Lookup::ConfiguredMissing(PathBuf::from("D:\\nowhere\\git.exe"))
        );
    }

    #[test]
    fn path_is_searched_in_order() {
        let env = env_with_path(&["C:\\a", "C:\\b"])
            .with_file("C:\\b\\wt.exe")
            .with_file("C:\\a\\wt.exe");
        assert_eq!(
            find(Tool::WindowsTerminal, &Overrides::default(), &env),
            Lookup::At(PathBuf::from("C:\\a\\wt.exe"))
        );
    }

    #[test]
    fn code_cmd_is_mapped_to_the_program_one_folder_up() {
        let env = env_with_path(&["C:\\Apps\\Code\\bin"])
            .with_file("C:\\Apps\\Code\\bin\\code.cmd")
            .with_file("C:\\Apps\\Code\\Code.exe");
        assert_eq!(
            find(Tool::Editor, &Overrides::default(), &env),
            Lookup::At(PathBuf::from("C:\\Apps\\Code\\Code.exe"))
        );
    }

    #[test]
    fn code_cmd_without_the_program_is_not_used() {
        let env =
            env_with_path(&["C:\\Apps\\Code\\bin"]).with_file("C:\\Apps\\Code\\bin\\code.cmd");
        assert_eq!(
            find(Tool::Editor, &Overrides::default(), &env),
            Lookup::NotFound
        );
    }

    #[test]
    fn a_known_place_is_used_when_the_variable_is_set() {
        let mut env = FakeEnv::default()
            .with_file("C:\\Users\\x\\AppData\\Local\\Programs\\Microsoft VS Code\\Code.exe");
        assert_eq!(
            find(Tool::Editor, &Overrides::default(), &env),
            Lookup::NotFound
        );
        env.vars
            .insert("LOCALAPPDATA".into(), "C:\\Users\\x\\AppData\\Local".into());
        assert_eq!(
            find(Tool::Editor, &Overrides::default(), &env),
            Lookup::At(PathBuf::from(
                "C:\\Users\\x\\AppData\\Local\\Programs\\Microsoft VS Code\\Code.exe"
            ))
        );
    }

    #[test]
    fn the_alias_of_windows_terminal_is_found_by_the_known_place() {
        // The alias is a file that `exists` sees (symlink_metadata); a trimmed PATH still works.
        let mut env =
            FakeEnv::default().with_file("C:\\L\\Microsoft\\Windows\\WindowsApps\\wt.exe");
        env.vars.insert("LOCALAPPDATA".into(), "C:\\L".into());
        assert_eq!(
            find(Tool::WindowsTerminal, &Overrides::default(), &env),
            Lookup::At(PathBuf::from(
                "C:\\L\\Microsoft\\Windows\\WindowsApps\\wt.exe"
            ))
        );
    }

    #[test]
    fn claude_is_found_in_the_user_bin_folder() {
        let mut env = FakeEnv::default().with_file("C:\\H\\.local\\bin\\claude.exe");
        env.vars.insert("USERPROFILE".into(), "C:\\H".into());
        assert_eq!(
            find(Tool::Claude, &Overrides::default(), &env),
            Lookup::At(PathBuf::from("C:\\H\\.local\\bin\\claude.exe"))
        );
    }

    #[test]
    fn nothing_found_is_not_found() {
        assert_eq!(
            find(Tool::Git, &Overrides::default(), &FakeEnv::default()),
            Lookup::NotFound
        );
    }

    #[test]
    fn the_image_names_follow_the_detected_editor() {
        assert_eq!(
            editor_image_names(&Lookup::At(PathBuf::from("C:\\E\\Cursor.exe"))),
            vec!["Cursor.exe".to_owned()]
        );
        assert_eq!(editor_image_names(&Lookup::NotFound).len(), 2);
    }
}
