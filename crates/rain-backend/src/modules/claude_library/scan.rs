//! Finding the files of the Claude library. Walks a few known folders, reads no file content.
//!
//! Every limit is here: how many files, how deep, how many projects. Nothing is cached: the
//! caller scans when a route is called. All code in this file is blocking and must run through
//! `blocking::run`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rain_types::LibraryKind;

use crate::modules::sessions::claude_paths::{
    ClaudePaths, project_claude_dir, project_folder_name,
};

/// The most files one scan lists. The rest is dropped and the answer says so.
pub const MAX_FILES: usize = 2500;
/// The most projects whose own Claude folder is looked at.
pub const MAX_PROJECTS: usize = 200;
/// The most plugins whose folder is looked at.
pub const MAX_PLUGINS: usize = 150;
/// How many folder levels `commands` may have below it (`commands/team/deploy.md` is 1).
const COMMAND_DEPTH: usize = 4;
/// The most entries read from one folder.
const FOLDER_ENTRIES: usize = 1000;

/// A project that Rain knows: a name to show and its folder.
#[derive(Debug, Clone)]
pub struct KnownProject {
    pub name: String,
    pub path: PathBuf,
}

/// A plugin that is installed: its name, version and the folder that holds its files.
#[derive(Debug, Clone)]
pub struct PluginRoot {
    pub name: String,
    pub version: Option<String>,
    pub dir: PathBuf,
}

/// One file of the library, before its content was looked at.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub kind: LibraryKind,
    pub scope: String,
    pub path: PathBuf,
    /// The name to show when the file does not say one.
    pub fallback_name: String,
}

#[derive(Debug, Default)]
pub struct Collected {
    pub candidates: Vec<Candidate>,
    /// The scan stopped at `MAX_FILES`.
    pub truncated: bool,
}

struct Collector {
    out: Collected,
    seen: HashSet<PathBuf>,
}

impl Collector {
    fn full(&self) -> bool {
        self.out.candidates.len() >= MAX_FILES
    }

    fn add(&mut self, kind: LibraryKind, scope: &str, path: PathBuf, fallback_name: String) {
        if self.full() {
            self.out.truncated = true;
            return;
        }
        // The same file reached twice (a project that is the home folder) is listed once.
        if !self.seen.insert(path.clone()) {
            return;
        }
        self.out.candidates.push(Candidate {
            kind,
            scope: scope.to_owned(),
            path,
            fallback_name,
        });
    }
}

pub fn scope_you() -> String {
    "You".to_owned()
}

pub fn scope_plugin(plugin: &PluginRoot) -> String {
    format!("Plugin {}", plugin.name)
}

pub fn scope_project(project: &KnownProject) -> String {
    format!("Project {}", project.name)
}

/// Lists all files. `dot_claude` is the folder of the user; the plugins and projects bring their
/// own folders.
pub fn collect(
    paths: &ClaudePaths,
    plugins: &[PluginRoot],
    projects: &[KnownProject],
    home_folder_name: &str,
) -> Collected {
    let mut collector = Collector {
        out: Collected::default(),
        seen: HashSet::new(),
    };
    let you = scope_you();

    // The user: the instructions, then skills, agents and commands.
    let root = paths.root();
    if root.is_dir() {
        add_instruction(&mut collector, &you, root.join("CLAUDE.md"));
        add_config_dir(&mut collector, &you, root);
        // The skills the Claude desktop app keeps in step with the account:
        // `skills/synced/<account>/<skill>/SKILL.md`.
        for (_, account) in sorted_entries(&root.join("skills").join("synced")) {
            add_skill_folders(&mut collector, SCOPE_CLAUDE_APP, &account);
        }
        // A scheduled task of the Claude app is a skill file too: `scheduled-tasks/<task>/SKILL.md`.
        add_skill_folders(
            &mut collector,
            SCOPE_SCHEDULED_TASK,
            &root.join("scheduled-tasks"),
        );
    }

    for project in projects.iter().take(MAX_PROJECTS) {
        let scope = scope_project(project);
        let own = project_claude_dir(&project.path);
        add_instruction(&mut collector, &scope, project.path.join("CLAUDE.md"));
        add_instruction(&mut collector, &scope, own.join("CLAUDE.md"));
        add_config_dir(&mut collector, &scope, &own);
    }

    for plugin in plugins.iter().take(MAX_PLUGINS) {
        add_config_dir(&mut collector, &scope_plugin(plugin), &plugin.dir);
    }

    add_memory(&mut collector, paths, projects, home_folder_name);
    collector.out
}

fn add_instruction(collector: &mut Collector, scope: &str, path: PathBuf) {
    if path.is_file() {
        collector.add(
            LibraryKind::Instruction,
            scope,
            path,
            "CLAUDE.md".to_owned(),
        );
    }
}

/// Scope of the skills that the Claude desktop app syncs into `skills/synced`.
const SCOPE_CLAUDE_APP: &str = "Claude app";
/// Scope of the skill files below `scheduled-tasks`.
const SCOPE_SCHEDULED_TASK: &str = "Scheduled task";

/// Every `<dir>/*/SKILL.md`.
fn add_skill_folders(collector: &mut Collector, scope: &str, dir: &Path) {
    for (name, skill) in sorted_entries(dir) {
        // A skill folder may be a link to the real one, so the file itself is checked.
        let file = skill.join("SKILL.md");
        if file.is_file() {
            collector.add(LibraryKind::Skill, scope, file, name);
        }
    }
}

/// `skills/*/SKILL.md`, `agents/*.md` and `commands/**/*.md` below one folder.
fn add_config_dir(collector: &mut Collector, scope: &str, dir: &Path) {
    add_skill_folders(collector, scope, &dir.join("skills"));
    for (name, file) in sorted_entries(&dir.join("agents")) {
        if is_markdown(&file) && file.is_file() {
            collector.add(LibraryKind::Agent, scope, file, strip_md(&name));
        }
    }
    let commands = dir.join("commands");
    add_commands(collector, scope, &commands, &commands, 0);
}

fn add_commands(collector: &mut Collector, scope: &str, root: &Path, dir: &Path, depth: usize) {
    for (_, path) in sorted_entries(dir) {
        let Ok(kind) = std::fs::symlink_metadata(&path).map(|meta| meta.file_type()) else {
            continue;
        };
        if kind.is_dir() {
            // A link to a folder is not followed: it may point back up and loop.
            if depth < COMMAND_DEPTH {
                add_commands(collector, scope, root, &path, depth + 1);
            }
        } else if is_markdown(&path) && path.is_file() {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            let name = relative
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(":");
            collector.add(LibraryKind::Command, scope, path.clone(), strip_md(&name));
        }
    }
}

/// `projects/*/memory/*.md`: every file, with the folder name turned into a project name.
fn add_memory(
    collector: &mut Collector,
    paths: &ClaudePaths,
    projects: &[KnownProject],
    home_folder_name: &str,
) {
    for (folder, dir) in sorted_entries(&paths.projects_dir()) {
        let memory = dir.join("memory");
        if !memory.is_dir() {
            continue;
        }
        let scope = format!(
            "Project {}",
            readable_project_name(&folder, projects, home_folder_name)
        );
        for (name, file) in sorted_entries(&memory) {
            if is_markdown(&file) && file.is_file() {
                collector.add(LibraryKind::Memory, &scope, file, strip_md(&name));
            }
        }
    }
}

/// The entries of a folder as (name, path), sorted by name, at most `FOLDER_ENTRIES`. A folder
/// that does not exist or cannot be read gives nothing.
fn sorted_entries(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut list: Vec<(String, PathBuf)> = entries
        .flatten()
        .take(FOLDER_ENTRIES)
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                entry.path(),
            )
        })
        .collect();
    list.sort_by_key(|a| a.0.to_lowercase());
    list
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
}

fn strip_md(name: &str) -> String {
    name.strip_suffix(".md")
        .or_else(|| name.strip_suffix(".MD"))
        .unwrap_or(name)
        .to_owned()
}

// ---- project names ----

/// The name of the project that a folder of `projects/` stands for. A known project gives its
/// own name. Otherwise the folder name is shortened: the drive and the profile folder go, and
/// the ending that Claude Code adds to a folder with the same name (`.08b2...`) goes.
pub fn readable_project_name(
    folder: &str,
    projects: &[KnownProject],
    home_folder_name: &str,
) -> String {
    let base = folder.split('.').next().unwrap_or(folder);
    if let Some(project) = projects
        .iter()
        .find(|project| project_folder_name(&project.path.to_string_lossy()) == base)
    {
        return project.name.clone();
    }
    let mut rest = base;
    // `C--` is the drive and the separator after it.
    if rest.len() > 3 && rest.as_bytes()[1] == b'-' && rest.as_bytes()[2] == b'-' {
        rest = &rest[3..];
    }
    let profile = format!("Users-{}", project_folder_name(home_folder_name));
    if let Some(after) = rest.strip_prefix(&profile) {
        rest = after.trim_start_matches('-');
        if rest.is_empty() {
            return "Home folder".to_owned();
        }
    }
    if rest.is_empty() {
        return folder.to_owned();
    }
    let shown: String = rest.chars().take(48).collect();
    if shown.len() < rest.len() {
        format!("{shown}...")
    } else {
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn the_names_of_project_folders_become_readable() {
        let known = vec![KnownProject {
            name: "Rain".into(),
            path: PathBuf::from(r"C:\Users\abdul\All\Rain"),
        }];
        let name = |folder: &str| readable_project_name(folder, &known, "abdul");
        assert_eq!(name("C--Users-abdul-All-Rain"), "Rain");
        assert_eq!(name("C--Users-abdul-All-Gantry"), "All-Gantry");
        assert_eq!(
            name("C--Users-abdul-All-Gantry.08b22724c147ebf4"),
            "All-Gantry"
        );
        assert_eq!(name("C--Users-abdul"), "Home folder");
        assert_eq!(name("D--Work-shop"), "Work-shop");
        assert!(name(&format!("C--Users-abdul-{}", "x".repeat(100))).ends_with("..."));
    }

    #[test]
    fn all_five_kinds_are_found_with_their_scope() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("claude");
        let project = temp.path().join("shop");
        let plugin_dir = temp.path().join("plug");
        write(&root.join("CLAUDE.md"), "user rules");
        write(
            &root.join("skills/alpha/SKILL.md"),
            "---\nname: alpha\n---\n",
        );
        write(&root.join("agents/helper.md"), "agent");
        write(&root.join("commands/team/deploy.md"), "cmd");
        write(
            &root.join("projects/C--work-shop/memory/MEMORY.md"),
            "index",
        );
        write(&root.join("projects/C--work-shop/memory/one.md"), "mem");
        write(
            &root.join("projects/C--work-shop/notes.txt"),
            "not markdown",
        );
        write(&project.join("CLAUDE.md"), "project rules");
        write(
            &project_claude_dir(&project).join("skills/beta/SKILL.md"),
            "skill",
        );
        write(&plugin_dir.join("skills/gamma/SKILL.md"), "skill");

        let paths = ClaudePaths::with_root(root);
        let projects = vec![KnownProject {
            name: "Shop".into(),
            path: project,
        }];
        let plugins = vec![PluginRoot {
            name: "tools".into(),
            version: None,
            dir: plugin_dir,
        }];
        let found = collect(&paths, &plugins, &projects, "me");
        assert!(!found.truncated);
        let listed: Vec<(LibraryKind, String, String)> = found
            .candidates
            .iter()
            .map(|c| (c.kind, c.scope.clone(), c.fallback_name.clone()))
            .collect();
        let has = |kind, scope: &str, name: &str| {
            listed
                .iter()
                .any(|(k, s, n)| *k == kind && s == scope && n == name)
        };
        assert!(has(LibraryKind::Instruction, "You", "CLAUDE.md"));
        assert!(has(LibraryKind::Skill, "You", "alpha"));
        assert!(has(LibraryKind::Agent, "You", "helper"));
        assert!(has(LibraryKind::Command, "You", "team:deploy"));
        assert!(has(LibraryKind::Memory, "Project work-shop", "MEMORY"));
        assert!(has(LibraryKind::Memory, "Project work-shop", "one"));
        assert!(has(LibraryKind::Instruction, "Project Shop", "CLAUDE.md"));
        assert!(has(LibraryKind::Skill, "Project Shop", "beta"));
        assert!(has(LibraryKind::Skill, "Plugin tools", "gamma"));
        assert_eq!(found.candidates.len(), 9, "notes.txt is not listed");
    }

    #[test]
    fn a_missing_claude_folder_gives_an_empty_list() {
        let paths = ClaudePaths::with_root(PathBuf::from("no-such-folder-for-rain-tests"));
        let found = collect(&paths, &[], &[], "me");
        assert!(found.candidates.is_empty());
    }
}
