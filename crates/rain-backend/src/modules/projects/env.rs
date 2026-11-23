//! `projects.toml` and the steps of a project. Section 02 4.1 and 6.6.
//!
//! The file is read by the user's own hand: it is never changed by Rain after it exists. A bad
//! entry is skipped and counted (the Projects source in `/health` says "n step(s) ignored"), and
//! does not fail the file. Only a value that makes the whole file meaningless (a wrong
//! `forget_missing_days`) fails it, and then the last good copy stays.

use std::collections::HashMap;

use rain_types::{EnvKind, IconKind, IconRef};
use serde::Deserialize;

use super::ids::path_key;
use super::tools::Overrides;

/// The text that Rain writes when `projects.toml` does not exist yet. Only comments except for
/// the version, so every default applies.
pub const EXAMPLE_FILE: &str = r#"schema_version = 1

# Rain opens a project with two steps: VS Code, and a Windows Terminal tab in the folder
# that runs Claude Code. This file changes that. Nothing here is required.
# Rain only reads this file. Edit it with any editor; Rain notices the change.

# [tools]                       # only when Rain cannot find a program by itself
# editor   = "C:\\Path\\To\\Code.exe"
# terminal = "C:\\Path\\To\\wt.exe"
# claude   = "C:\\Path\\To\\claude.exe"
# git      = "C:\\Path\\To\\git.exe"

# [terminal]
# window  = "current"           # "current" = a tab in your last Terminal window, "new" = a new window
# profile = ""                  # a Windows Terminal profile name; empty = your default profile

# [defaults]
# steps = ["editor", "claude"]      # add "terminal" for a second, plain shell tab
# forget_missing_days = 30      # a project whose folder vanished stays listed (dimmed) this long

# A named step can be used by any project:
# [[step]]
# name    = "dev-server"
# kind    = "terminal"
# label   = "Dev server"
# command = "npm run dev"
#
# [[step]]
# name     = "preview"
# kind     = "browser"
# label    = "Browser preview"
# url      = "http://localhost:5173"
# delay_ms = 3000

# One project with its own steps:
# [[project]]
# path  = "C:\\Dev\\web-shop"
# steps = ["editor", "dev-server", "preview", "claude"]
"#;

/// Days a vanished project stays listed, when the file does not say.
pub const DEFAULT_FORGET_DAYS: u32 = 30;
const MAX_FORGET_DAYS: u32 = 3650;
const MAX_TEXT: usize = 500;
const MAX_DELAY_MS: u32 = 30_000;

// ---- the file as it is read ----

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ToolsDef {
    pub editor: String,
    pub terminal: String,
    #[serde(rename = "claude")]
    /// The path of Claude Code (the key is `claude`; the field avoids that name because the source
    /// rules reserve the text of the Claude folder for the sessions module).
    pub coding_agent: String,
    pub git: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TerminalDef {
    /// `current` (the default) or `new`.
    pub window: String,
    pub profile: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DefaultsDef {
    pub steps: Option<Vec<String>>,
    pub forget_missing_days: Option<u32>,
}

/// The whole file. `step` and `project` stay raw values so that one bad entry does not fail the
/// others: each is read on its own in `analyze`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ProjectsFile {
    pub tools: ToolsDef,
    pub terminal: TerminalDef,
    pub defaults: DefaultsDef,
    pub step: Vec<toml::Value>,
    pub project: Vec<toml::Value>,
}

/// The rules that the types cannot state. A file that breaks one keeps its last good copy.
pub fn validate(file: &ProjectsFile) -> Result<(), String> {
    match file.terminal.window.as_str() {
        "" | "current" | "new" => {}
        _ => return Err("[terminal] window must be \"current\" or \"new\"".to_owned()),
    }
    if file
        .defaults
        .forget_missing_days
        .is_some_and(|days| days > MAX_FORGET_DAYS)
    {
        return Err(format!(
            "[defaults] forget_missing_days must be 0 to {MAX_FORGET_DAYS}"
        ));
    }
    Ok(())
}

impl ProjectsFile {
    pub fn overrides(&self) -> Overrides {
        Overrides {
            editor: self.tools.editor.clone(),
            terminal: self.tools.terminal.clone(),
            coding_agent: self.tools.coding_agent.clone(),
            git: self.tools.git.clone(),
        }
    }

    pub fn forget_missing_days(&self) -> u32 {
        self.defaults
            .forget_missing_days
            .unwrap_or(DEFAULT_FORGET_DAYS)
    }

    /// `-w new` instead of `-w 0`.
    pub fn terminal_in_new_window(&self) -> bool {
        self.terminal.window == "new"
    }
}

// ---- steps ----

/// One step with every default filled in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepSpec {
    pub name: String,
    pub kind: EnvKind,
    pub label: String,
    pub icon: IconRef,
    /// A terminal step: what runs in the new tab. A Claude step shows `claude` here.
    pub command: Option<String>,
    pub url: Option<String>,
    pub delay_ms: u32,
    /// `app`: the program. `editor`: this editor instead of the detected one.
    pub app: Option<String>,
    /// `app` and `editor`: arguments after the folder. `claude`: arguments after `claude`.
    pub args: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct StepDef {
    name: String,
    kind: Option<EnvKind>,
    label: Option<String>,
    command: Option<String>,
    url: Option<String>,
    delay_ms: Option<u32>,
    app: Option<String>,
    args: Option<String>,
    icon: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ProjectDef {
    path: Option<String>,
    id: Option<String>,
    name: Option<String>,
    icon: Option<String>,
    steps: Option<Vec<String>>,
}

/// What the file says about one project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectEntry {
    pub path_key: Option<String>,
    pub id: Option<String>,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub steps: Option<Vec<String>>,
}

/// The file after every entry was checked.
#[derive(Debug, Clone, Default)]
pub struct Analysis {
    /// The valid `[[step]]` entries by name.
    pub steps: HashMap<String, StepSpec>,
    pub projects: Vec<ProjectEntry>,
    pub default_steps: Vec<String>,
    /// How many entries and list names were skipped.
    pub ignored: usize,
}

/// The built-in step names, and what the editor step is called.
pub struct Builtins {
    pub editor_label: String,
}

fn lucide(name: &str) -> IconRef {
    IconRef {
        kind: IconKind::Lucide,
        value: name.to_owned(),
        tint: None,
        fallback: None,
    }
}

fn is_step_name(name: &str) -> bool {
    let mut letters = name.chars();
    let first_ok = letters
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    first_ok
        && name.len() <= 40
        && letters.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// The icon a step of this kind has when the file names none.
fn default_icon(kind: EnvKind) -> IconRef {
    match kind {
        EnvKind::Editor => lucide("code-xml"),
        EnvKind::Terminal => lucide("square-terminal"),
        EnvKind::Claude => IconRef {
            kind: IconKind::Brand,
            value: "claude".to_owned(),
            tint: None,
            fallback: None,
        },
        EnvKind::Browser => lucide("globe"),
        EnvKind::App => lucide("app-window"),
    }
}

fn default_label(kind: EnvKind, builtins: &Builtins) -> String {
    match kind {
        EnvKind::Editor => builtins.editor_label.clone(),
        EnvKind::Terminal => "Terminal".to_owned(),
        EnvKind::Claude => "Claude Code".to_owned(),
        EnvKind::Browser => "Browser".to_owned(),
        EnvKind::App => "App".to_owned(),
    }
}

/// The built-in step of that name (`editor`, `terminal`, `claude`).
pub fn builtin_step(name: &str, builtins: &Builtins) -> Option<StepSpec> {
    let kind = match name {
        "editor" => EnvKind::Editor,
        "terminal" => EnvKind::Terminal,
        "claude" => EnvKind::Claude,
        _ => return None,
    };
    Some(StepSpec {
        name: name.to_owned(),
        kind,
        label: default_label(kind, builtins),
        icon: default_icon(kind),
        command: None,
        url: None,
        delay_ms: 0,
        app: None,
        args: None,
    })
}

/// Checks one `[[step]]`. `None` when it breaks a rule.
fn check_step(def: StepDef, builtins: &Builtins) -> Option<StepSpec> {
    if !is_step_name(&def.name) {
        return None;
    }
    let kind = def.kind?;
    let label = match def.label {
        Some(label) => {
            let label = label.trim().to_owned();
            if label.is_empty() || label.chars().count() > 40 {
                return None;
            }
            label
        }
        None => default_label(kind, builtins),
    };
    let too_long =
        |text: &Option<String>| text.as_ref().is_some_and(|t| t.chars().count() > MAX_TEXT);
    if too_long(&def.command) || too_long(&def.args) {
        return None;
    }
    // A double quote cannot be passed safely through Windows Terminal and `cmd`: a complicated
    // command belongs in a `.cmd` file.
    if def.command.as_ref().is_some_and(|c| c.contains('"')) {
        return None;
    }
    let url = def.url.map(|u| u.trim().to_owned());
    if kind == EnvKind::Browser
        && !url
            .as_deref()
            .is_some_and(|u| u.starts_with("http://") || u.starts_with("https://"))
    {
        return None;
    }
    let delay_ms = def.delay_ms.unwrap_or(0);
    if delay_ms > MAX_DELAY_MS {
        return None;
    }
    let app = def
        .app
        .map(|a| a.trim().to_owned())
        .filter(|a| !a.is_empty());
    if kind == EnvKind::App && app.is_none() {
        return None;
    }
    if app.as_deref().is_some_and(|a| !is_absolute(a)) {
        return None;
    }
    let icon = def
        .icon
        .as_deref()
        .map_or_else(|| default_icon(kind), lucide);
    Some(StepSpec {
        name: def.name,
        kind,
        label,
        icon,
        command: if kind == EnvKind::Terminal {
            def.command.filter(|c| !c.trim().is_empty())
        } else {
            None
        },
        url: if kind == EnvKind::Browser { url } else { None },
        delay_ms,
        app,
        args: def.args.filter(|a| !a.trim().is_empty()),
    })
}

fn is_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    (bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\')
        || path.starts_with("\\\\")
}

/// Reads every entry of the file on its own.
pub fn analyze(file: &ProjectsFile, builtins: &Builtins) -> Analysis {
    let mut analysis = Analysis::default();
    for value in &file.step {
        let spec = value
            .clone()
            .try_into::<StepDef>()
            .ok()
            .and_then(|def| check_step(def, builtins));
        match spec {
            // The first entry of a name wins.
            Some(spec) if !analysis.steps.contains_key(&spec.name) => {
                analysis.steps.insert(spec.name.clone(), spec);
            }
            _ => analysis.ignored += 1,
        }
    }
    for value in &file.project {
        match value.clone().try_into::<ProjectDef>() {
            Ok(def) if def.path.is_some() || def.id.is_some() => {
                analysis.projects.push(ProjectEntry {
                    path_key: def.path.as_deref().map(path_key),
                    id: def.id,
                    name: def.name.filter(|n| !n.trim().is_empty()),
                    icon: def.icon,
                    steps: def.steps,
                });
            }
            _ => analysis.ignored += 1,
        }
    }
    analysis.default_steps = file
        .defaults
        .steps
        .clone()
        .unwrap_or_else(|| vec!["editor".to_owned(), "claude".to_owned()]);
    analysis
}

impl Analysis {
    /// The entry of one project: by path first, else by id.
    pub fn entry_for(&self, key: &str, id: &str) -> Option<&ProjectEntry> {
        self.projects
            .iter()
            .find(|entry| entry.path_key.as_deref() == Some(key))
            .or_else(|| {
                self.projects
                    .iter()
                    .find(|entry| entry.path_key.is_none() && entry.id.as_deref() == Some(id))
            })
    }

    /// The step list of a project, in order. A name that is neither a step of the file nor a
    /// built-in is skipped and counted in `ignored_names`.
    pub fn resolve(&self, key: &str, id: &str, builtins: &Builtins) -> (Vec<StepSpec>, usize) {
        let names = self
            .entry_for(key, id)
            .and_then(|entry| entry.steps.as_ref())
            .unwrap_or(&self.default_steps);
        self.specs_of(names, builtins)
    }

    /// The step specs of a list of names (a user step replaces a built-in of the same name).
    pub fn specs_of(&self, names: &[String], builtins: &Builtins) -> (Vec<StepSpec>, usize) {
        let mut specs = Vec::new();
        let mut unknown = 0;
        for name in names {
            let spec = self
                .steps
                .get(name)
                .cloned()
                .or_else(|| builtin_step(name, builtins));
            match spec {
                Some(spec) => specs.push(spec),
                None => unknown += 1,
            }
        }
        (specs, unknown)
    }

    /// The names in the lists of the file that match no step, for the health message.
    pub fn unknown_names(&self, builtins: &Builtins) -> usize {
        let lists = std::iter::once(&self.default_steps).chain(
            self.projects
                .iter()
                .filter_map(|entry| entry.steps.as_ref()),
        );
        lists.map(|names| self.specs_of(names, builtins).1).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builtins() -> Builtins {
        Builtins {
            editor_label: "VS Code".to_owned(),
        }
    }

    fn parse(text: &str) -> ProjectsFile {
        toml::from_str(text).unwrap()
    }

    fn names(specs: &[StepSpec]) -> Vec<&str> {
        specs.iter().map(|spec| spec.name.as_str()).collect()
    }

    #[test]
    fn the_example_file_is_valid_and_changes_nothing() {
        let file = parse(EXAMPLE_FILE);
        validate(&file).unwrap();
        let analysis = analyze(&file, &builtins());
        assert_eq!(analysis.ignored, 0);
        assert!(analysis.steps.is_empty() && analysis.projects.is_empty());
        assert_eq!(file.forget_missing_days(), 30);
        assert!(!file.terminal_in_new_window());
    }

    #[test]
    fn the_default_plan_is_the_editor_and_claude() {
        let analysis = analyze(&ProjectsFile::default(), &builtins());
        let (specs, unknown) = analysis.resolve("d:\\w\\alpha", "proj-alpha", &builtins());
        assert_eq!(names(&specs), ["editor", "claude"]);
        assert_eq!(unknown, 0);
        assert_eq!(specs[0].label, "VS Code");
        assert_eq!(specs[0].icon, lucide("code-xml"));
        assert_eq!(specs[1].label, "Claude Code");
        assert_eq!(specs[1].icon.kind, IconKind::Brand);
    }

    const TYPICAL: &str = r#"
        [defaults]
        steps = ["editor", "terminal"]

        [[step]]
        name = "dev-server"
        kind = "terminal"
        label = "Dev server"
        command = "npm run dev"

        [[step]]
        name = "preview"
        kind = "browser"
        url = "http://localhost:5173"
        delay_ms = 3000

        [[project]]
        path = "D:/Work/Alpha/"
        steps = ["editor", "dev-server", "preview", "claude"]

        [[project]]
        id = "proj-beta"
        name = "Beta shop"
        steps = ["terminal"]
    "#;

    #[test]
    fn a_project_is_found_by_path_in_any_spelling_and_by_id() {
        let analysis = analyze(&parse(TYPICAL), &builtins());
        assert_eq!(analysis.ignored, 0);
        let (specs, _) = analysis.resolve("d:\\work\\alpha", "proj-alpha", &builtins());
        assert_eq!(names(&specs), ["editor", "dev-server", "preview", "claude"]);
        assert_eq!(specs[1].command.as_deref(), Some("npm run dev"));
        assert_eq!(specs[2].delay_ms, 3000);
        assert_eq!(specs[2].url.as_deref(), Some("http://localhost:5173"));

        let (by_id, _) = analysis.resolve("e:\\other\\beta", "proj-beta", &builtins());
        assert_eq!(names(&by_id), ["terminal"]);
        assert_eq!(
            analysis
                .entry_for("e:\\other\\beta", "proj-beta")
                .and_then(|e| e.name.clone()),
            Some("Beta shop".to_owned())
        );

        let (others, _) = analysis.resolve("d:\\work\\gamma", "proj-gamma", &builtins());
        assert_eq!(names(&others), ["editor", "terminal"]);
    }

    #[test]
    fn an_unknown_step_name_is_skipped_and_counted() {
        let file = parse(
            r#"
            [defaults]
            steps = ["editor", "nonsense", "claude"]
            "#,
        );
        let analysis = analyze(&file, &builtins());
        let (specs, unknown) = analysis.resolve("k", "i", &builtins());
        assert_eq!(names(&specs), ["editor", "claude"]);
        assert_eq!(unknown, 1);
        assert_eq!(analysis.unknown_names(&builtins()), 1);
    }

    #[test]
    fn a_user_step_replaces_a_builtin_of_the_same_name() {
        let file = parse(
            r#"
            [[step]]
            name = "claude"
            kind = "claude"
            label = "Claude, careful"
            args = "--model sonnet"
            "#,
        );
        let analysis = analyze(&file, &builtins());
        let (specs, _) = analysis.resolve("k", "i", &builtins());
        assert_eq!(specs[1].label, "Claude, careful");
        assert_eq!(specs[1].args.as_deref(), Some("--model sonnet"));
    }

    #[test]
    fn bad_steps_are_skipped_and_counted_without_failing_the_file() {
        let file = parse(
            r#"
            [[step]]
            name = "web"
            kind = "browser"
            url = "ftp://nope"

            [[step]]
            name = "quoted"
            kind = "terminal"
            command = "echo \"hi\""

            [[step]]
            name = "Bad Name"
            kind = "terminal"

            [[step]]
            name = "slow"
            kind = "terminal"
            delay_ms = 999999

            [[step]]
            name = "ok"
            kind = "terminal"

            [[step]]
            name = "ok"
            kind = "terminal"

            [[step]]
            name = "nokind"

            [[step]]
            name = "app1"
            kind = "app"

            [[project]]
            name = "no path and no id"
            "#,
        );
        validate(&file).unwrap();
        let analysis = analyze(&file, &builtins());
        assert_eq!(analysis.steps.len(), 1);
        assert!(analysis.steps.contains_key("ok"));
        assert_eq!(analysis.ignored, 8);
    }

    #[test]
    fn the_file_is_refused_for_a_wrong_window_or_a_huge_forget_time() {
        assert!(validate(&parse("[terminal]\nwindow = \"sideways\"")).is_err());
        assert!(validate(&parse("[defaults]\nforget_missing_days = 99999")).is_err());
        validate(&parse("[defaults]\nforget_missing_days = 0")).unwrap();
        assert!(parse("[terminal]\nwindow = \"new\"").terminal_in_new_window());
    }

    #[test]
    fn step_names_follow_the_pattern() {
        assert!(is_step_name("dev-server"));
        assert!(is_step_name("a"));
        assert!(is_step_name("2fa_check"));
        assert!(!is_step_name(""));
        assert!(!is_step_name("-x"));
        assert!(!is_step_name("X"));
        assert!(!is_step_name(&"a".repeat(41)));
    }

    #[test]
    fn an_app_step_needs_an_absolute_program() {
        let file = parse(
            r#"
            [[step]]
            name = "a"
            kind = "app"
            app = "notepad.exe"

            [[step]]
            name = "b"
            kind = "app"
            app = "C:\\Tools\\tool.exe"
            args = "--x"
            "#,
        );
        let analysis = analyze(&file, &builtins());
        assert_eq!(analysis.ignored, 1);
        assert_eq!(
            analysis.steps["b"].app.as_deref(),
            Some("C:\\Tools\\tool.exe")
        );
    }
}
