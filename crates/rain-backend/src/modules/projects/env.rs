//! `projects.toml` and the steps of a project. Section 02 4.1 and 6.6.
//!
//! The file is written by the user's own hand. Rain changes it in one place only: the `layout`
//! of `[defaults]` or of a `[[project]]`, when the user saves a layout in the Projects screen
//! (`layout_file.rs`; comments and order stay). A bad
//! entry is skipped and counted (the Projects source in `/health` says "n step(s) ignored"), and
//! does not fail the file. Only a value that makes the whole file meaningless (a wrong
//! `forget_missing_days`) fails it, and then the last good copy stays.

use std::collections::HashMap;

use rain_types::{
    EnvKind, IconKind, IconRef, LAYOUT_ITEMS_MAX, LAYOUT_MONITOR_MAX, LayoutItem, LayoutScope,
    LayoutWhat, LayoutZone,
};
use serde::Deserialize;

use super::ids::path_key;
use super::tools::Overrides;
use crate::modules::launcher::AppFacts;

/// The text that Rain writes when `projects.toml` does not exist yet. Only comments except for
/// the version, so every default applies.
pub const EXAMPLE_FILE: &str = r#"schema_version = 1

# Rain opens a project with two steps: VS Code, and a Windows Terminal tab in the folder
# that runs Claude Code. This file changes that. Nothing here is required.
# Edit this file with any editor; Rain notices the change. Rain itself writes only the `layout`
# lists below, when you save a layout in the Projects screen. Your comments and order stay.

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

# A layout says where each window goes. Each item names WHAT opens (one of step, app, folder)
# and WHERE (zone, and monitor). Without a layout every window opens where Windows puts it.
#   step    = "editor", "terminal", "claude", or the name of a [[step]] above
#   app     = the id of an app of Rain's app list, for example "app-notepad"
#   folder  = true: the project folder in Explorer
#   zone    = "free" (the default: not moved), "full", "left-half", "right-half", "top-half",
#             "bottom-half", "top-left", "top-right", "bottom-left", "bottom-right",
#             "left-third", "middle-third", "right-third", "left-two-thirds", "right-two-thirds"
#   monitor = 1, 2, ... counted from the left. Without it: the monitor Rain's window is on.
# Terminal items in the same zone become tabs of one Windows Terminal window.
# A layout of a project beats its steps. A layout under [defaults] serves every project that has
# neither a layout nor steps of its own.
# [defaults]
# layout = [
#   { step = "editor",   zone = "right-half" },
#   { step = "terminal", zone = "top-left" },
#   { step = "claude",   zone = "bottom-left" },
# ]
#
# [[project]]
# path   = "C:\\Dev\\web-shop"
# layout = [
#   { step = "editor", zone = "left-two-thirds", monitor = 2 },
#   { app = "app-notepad", zone = "right-third", monitor = 2 },
# ]
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
    /// Raw items, so that one bad item does not fail the file (see `read_layout`).
    pub layout: Option<Vec<toml::Value>>,
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
    /// A step that a layout names: the app of Rain's app list that opens (kind `App`).
    pub app_id: Option<String>,
    /// Where the window goes. `Free` for a step of a plain list.
    pub zone: LayoutZone,
    pub monitor: Option<u32>,
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
    layout: Option<Vec<toml::Value>>,
}

/// What the file says about one project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectEntry {
    pub path_key: Option<String>,
    pub id: Option<String>,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub steps: Option<Vec<String>>,
    /// Empty: the project has no layout of its own.
    pub layout: Vec<LayoutItem>,
}

/// The file after every entry was checked.
#[derive(Debug, Clone, Default)]
pub struct Analysis {
    /// The valid `[[step]]` entries by name.
    pub steps: HashMap<String, StepSpec>,
    pub projects: Vec<ProjectEntry>,
    pub default_steps: Vec<String>,
    /// The layout under `[defaults]`; empty when there is none.
    pub default_layout: Vec<LayoutItem>,
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
        EnvKind::Folder => lucide("folder-open"),
    }
}

fn default_label(kind: EnvKind, builtins: &Builtins) -> String {
    match kind {
        EnvKind::Editor => builtins.editor_label.clone(),
        EnvKind::Terminal => "Terminal".to_owned(),
        EnvKind::Claude => "Claude Code".to_owned(),
        EnvKind::Browser => "Browser".to_owned(),
        EnvKind::App => "App".to_owned(),
        EnvKind::Folder => "Folder".to_owned(),
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
        app_id: None,
        zone: LayoutZone::Free,
        monitor: None,
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
        app_id: None,
        zone: LayoutZone::Free,
        monitor: None,
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
                let (layout, skipped) = read_layout(def.layout.as_deref().unwrap_or_default());
                analysis.ignored += skipped;
                analysis.projects.push(ProjectEntry {
                    path_key: def.path.as_deref().map(path_key),
                    id: def.id,
                    name: def.name.filter(|n| !n.trim().is_empty()),
                    icon: def.icon,
                    steps: def.steps,
                    layout,
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
    let (layout, skipped) = read_layout(file.defaults.layout.as_deref().unwrap_or_default());
    analysis.default_layout = layout;
    analysis.ignored += skipped;
    analysis
}

/// One item of a `layout` list as written: exactly one of `step`, `app` and `folder`.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct LayoutItemDef {
    step: Option<String>,
    app: Option<String>,
    folder: Option<bool>,
    zone: Option<String>,
    monitor: Option<u32>,
}

/// Checks one layout item on its own. `None` when it breaks a rule.
fn check_layout_item(value: &toml::Value) -> Option<LayoutItem> {
    let def: LayoutItemDef = value.clone().try_into().ok()?;
    let what = match (def.step, def.app, def.folder) {
        (Some(name), None, None) if is_step_name(&name) => LayoutWhat::Step { name },
        (None, Some(id), None) if !id.trim().is_empty() && id.len() <= 80 => LayoutWhat::App { id },
        (None, None, Some(true)) => LayoutWhat::Folder,
        _ => return None,
    };
    let zone = match def.zone.as_deref() {
        None => LayoutZone::Free,
        Some(name) => LayoutZone::from_name(name)?,
    };
    if def
        .monitor
        .is_some_and(|number| number == 0 || number > LAYOUT_MONITOR_MAX)
    {
        return None;
    }
    Some(LayoutItem {
        what,
        zone,
        monitor: def.monitor,
    })
}

/// The items of a `layout` list that are good, and how many were skipped. Items beyond
/// `LAYOUT_ITEMS_MAX` are skipped too. An empty list is no layout.
pub fn read_layout(values: &[toml::Value]) -> (Vec<LayoutItem>, usize) {
    let mut items = Vec::new();
    let mut skipped = 0;
    for value in values {
        match check_layout_item(value) {
            Some(item) if items.len() < LAYOUT_ITEMS_MAX => items.push(item),
            _ => skipped += 1,
        }
    }
    (items, skipped)
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

    /// The layout that counts for a project and where it comes from. The order: the project's own
    /// layout, then its own step list (no layout), then the layout of `[defaults]`.
    pub fn layout_of(&self, key: &str, id: &str) -> (LayoutScope, &[LayoutItem]) {
        let entry = self.entry_for(key, id);
        if let Some(entry) = entry.filter(|entry| !entry.layout.is_empty()) {
            return (LayoutScope::Project, &entry.layout);
        }
        if entry.is_some_and(|entry| entry.steps.is_some()) || self.default_layout.is_empty() {
            return (LayoutScope::None, &[]);
        }
        (LayoutScope::Default, &self.default_layout)
    }

    /// The step list of a project, in order: from its layout when it has one that counts, else
    /// from its list of names. A name that is neither a step of the file nor a built-in is
    /// skipped and counted in `ignored_names`.
    pub fn resolve(
        &self,
        key: &str,
        id: &str,
        builtins: &Builtins,
        apps: Apps<'_>,
    ) -> (Vec<StepSpec>, usize) {
        let (scope, items) = self.layout_of(key, id);
        if scope != LayoutScope::None {
            return self.specs_of_layout(items, builtins, apps);
        }
        let names = self
            .entry_for(key, id)
            .and_then(|entry| entry.steps.as_ref())
            .unwrap_or(&self.default_steps);
        self.specs_of(names, builtins)
    }

    /// The step specs of the items of a layout, each with its zone and monitor.
    pub fn specs_of_layout(
        &self,
        items: &[LayoutItem],
        builtins: &Builtins,
        apps: Apps<'_>,
    ) -> (Vec<StepSpec>, usize) {
        let mut specs = Vec::new();
        let mut unknown = 0;
        for item in items {
            let spec = match &item.what {
                LayoutWhat::Step { name } => self
                    .steps
                    .get(name)
                    .cloned()
                    .or_else(|| builtin_step(name, builtins)),
                LayoutWhat::App { id } => {
                    let facts = apps(id);
                    Some(StepSpec {
                        name: format!("app:{id}"),
                        kind: EnvKind::App,
                        label: facts
                            .as_ref()
                            .map_or_else(|| "Unknown app".to_owned(), |f| f.name.clone()),
                        icon: facts.map_or_else(|| default_icon(EnvKind::App), |f| f.icon.clone()),
                        command: None,
                        url: None,
                        delay_ms: 0,
                        app: None,
                        args: None,
                        app_id: Some(id.clone()),
                        zone: LayoutZone::Free,
                        monitor: None,
                    })
                }
                LayoutWhat::Folder => Some(StepSpec {
                    name: "folder".to_owned(),
                    kind: EnvKind::Folder,
                    label: default_label(EnvKind::Folder, builtins),
                    icon: default_icon(EnvKind::Folder),
                    command: None,
                    url: None,
                    delay_ms: 0,
                    app: None,
                    args: None,
                    app_id: None,
                    zone: LayoutZone::Free,
                    monitor: None,
                }),
            };
            match spec {
                Some(spec) => specs.push(StepSpec {
                    zone: item.zone,
                    monitor: item.monitor,
                    ..spec
                }),
                None => unknown += 1,
            }
        }
        (specs, unknown)
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
        let in_lists: usize = lists.map(|names| self.specs_of(names, builtins).1).sum();
        let layouts = std::iter::once(&self.default_layout)
            .chain(self.projects.iter().map(|entry| &entry.layout));
        let in_layouts: usize = layouts
            .map(|items| self.specs_of_layout(items, builtins, &|_| None).1)
            .sum();
        in_lists + in_layouts
    }
}

/// Looks up an app of the launcher's list by its id.
pub type Apps<'a> = &'a dyn Fn(&str) -> Option<AppFacts>;

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
        let (specs, unknown) =
            analysis.resolve("d:\\w\\alpha", "proj-alpha", &builtins(), &|_| None);
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
        let (specs, _) = analysis.resolve("d:\\work\\alpha", "proj-alpha", &builtins(), &|_| None);
        assert_eq!(names(&specs), ["editor", "dev-server", "preview", "claude"]);
        assert_eq!(specs[1].command.as_deref(), Some("npm run dev"));
        assert_eq!(specs[2].delay_ms, 3000);
        assert_eq!(specs[2].url.as_deref(), Some("http://localhost:5173"));

        let (by_id, _) = analysis.resolve("e:\\other\\beta", "proj-beta", &builtins(), &|_| None);
        assert_eq!(names(&by_id), ["terminal"]);
        assert_eq!(
            analysis
                .entry_for("e:\\other\\beta", "proj-beta")
                .and_then(|e| e.name.clone()),
            Some("Beta shop".to_owned())
        );

        let (others, _) = analysis.resolve("d:\\work\\gamma", "proj-gamma", &builtins(), &|_| None);
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
        let (specs, unknown) = analysis.resolve("k", "i", &builtins(), &|_| None);
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
        let (specs, _) = analysis.resolve("k", "i", &builtins(), &|_| None);
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

    // ---- layouts ----

    fn step_item(name: &str, zone: LayoutZone, monitor: Option<u32>) -> LayoutItem {
        LayoutItem {
            what: LayoutWhat::Step { name: name.into() },
            zone,
            monitor,
        }
    }

    #[test]
    fn a_layout_is_read_item_by_item_and_a_bad_item_is_skipped_and_counted() {
        let file = parse(
            r#"
            [defaults]
            layout = [
              { step = "editor", zone = "right-half" },
              { step = "terminal", zone = "left-two-thirds", monitor = 2 },
              { app = "app-notepad", zone = "bottom-right" },
              { folder = true },
              { step = "editor", zone = "sideways" },
              { step = "editor", monitor = 0 },
              { step = "editor", monitor = 99 },
              { step = "Bad Name" },
              { step = "editor", app = "app-x" },
              { folder = false },
              { zone = "full" },
              { step = "editor", colour = "red" },
            ]
            "#,
        );
        let analysis = analyze(&file, &builtins());
        assert_eq!(analysis.ignored, 8);
        assert_eq!(
            analysis.default_layout,
            [
                step_item("editor", LayoutZone::RightHalf, None),
                step_item("terminal", LayoutZone::LeftTwoThirds, Some(2)),
                LayoutItem {
                    what: LayoutWhat::App {
                        id: "app-notepad".into()
                    },
                    zone: LayoutZone::BottomRight,
                    monitor: None,
                },
                LayoutItem {
                    what: LayoutWhat::Folder,
                    zone: LayoutZone::Free,
                    monitor: None,
                },
            ]
        );
    }

    #[test]
    fn a_layout_holds_at_most_twelve_items() {
        let items = vec!["{ step = \"editor\" }"; 14].join(", ");
        let file = parse(&format!("[defaults]\nlayout = [{items}]"));
        let analysis = analyze(&file, &builtins());
        assert_eq!(analysis.default_layout.len(), LAYOUT_ITEMS_MAX);
        assert_eq!(analysis.ignored, 2);
    }

    const PRECEDENCE: &str = r#"
        [defaults]
        steps = ["editor", "terminal"]
        layout = [{ step = "claude", zone = "full" }]

        [[project]]
        path = "D:/Work/Own"
        layout = [{ step = "editor", zone = "left-half" }, { step = "terminal", zone = "right-half" }]

        [[project]]
        path = "D:/Work/Steps"
        steps = ["terminal"]

        [[project]]
        path = "D:/Work/Empty"
        layout = []
    "#;

    #[test]
    fn a_project_layout_beats_its_steps_and_its_steps_beat_the_default_layout() {
        let analysis = analyze(&parse(PRECEDENCE), &builtins());
        let apps = |_: &str| None;
        // Its own layout.
        let (scope, items) = analysis.layout_of("d:\\work\\own", "proj-own");
        assert_eq!((scope, items.len()), (LayoutScope::Project, 2));
        let (specs, _) = analysis.resolve("d:\\work\\own", "proj-own", &builtins(), &apps);
        assert_eq!(names(&specs), ["editor", "terminal"]);
        assert_eq!(specs[0].zone, LayoutZone::LeftHalf);
        assert_eq!(specs[1].zone, LayoutZone::RightHalf);
        // Its own steps, no layout: the default layout does not apply.
        let (scope, _) = analysis.layout_of("d:\\work\\steps", "proj-steps");
        assert_eq!(scope, LayoutScope::None);
        let (specs, _) = analysis.resolve("d:\\work\\steps", "proj-steps", &builtins(), &apps);
        assert_eq!(names(&specs), ["terminal"]);
        assert_eq!(specs[0].zone, LayoutZone::Free);
        // An empty list is no layout: the default layout serves the project.
        let (scope, _) = analysis.layout_of("d:\\work\\empty", "proj-empty");
        assert_eq!(scope, LayoutScope::Default);
        // A project the file does not mention gets the default layout.
        let (specs, _) = analysis.resolve("d:\\work\\other", "proj-other", &builtins(), &apps);
        assert_eq!(names(&specs), ["claude"]);
        assert_eq!(specs[0].zone, LayoutZone::Full);
    }

    #[test]
    fn without_any_layout_the_steps_are_the_same_as_before() {
        let analysis = analyze(&ProjectsFile::default(), &builtins());
        let (scope, items) = analysis.layout_of("k", "i");
        assert_eq!((scope, items.len()), (LayoutScope::None, 0));
        let (specs, _) = analysis.resolve("k", "i", &builtins(), &|_| None);
        assert!(specs.iter().all(|spec| spec.zone == LayoutZone::Free));
        assert_eq!(names(&specs), ["editor", "claude"]);
    }

    #[test]
    fn apps_and_folders_become_steps_and_an_unknown_step_is_counted() {
        let file = parse(
            r#"
            [defaults]
            layout = [
              { app = "app-notepad", zone = "top-left", monitor = 2 },
              { app = "app-gone" },
              { folder = true, zone = "top-right" },
              { step = "nonsense" },
            ]
            "#,
        );
        let analysis = analyze(&file, &builtins());
        let lookup = |id: &str| {
            (id == "app-notepad").then(|| AppFacts {
                name: "Notepad".into(),
                icon: lucide("app-window"),
                kind: rain_types::AppKind::Program,
                image: Some("notepad.exe".into()),
            })
        };
        let (specs, unknown) = analysis.resolve("k", "i", &builtins(), &lookup);
        assert_eq!(unknown, 1);
        assert_eq!(specs.len(), 3);
        assert_eq!(specs[0].kind, EnvKind::App);
        assert_eq!(specs[0].label, "Notepad");
        assert_eq!(specs[0].app_id.as_deref(), Some("app-notepad"));
        assert_eq!(
            (specs[0].zone, specs[0].monitor),
            (LayoutZone::TopLeft, Some(2))
        );
        assert_eq!(specs[1].label, "Unknown app");
        assert_eq!(specs[2].kind, EnvKind::Folder);
        assert_eq!(specs[2].zone, LayoutZone::TopRight);
        assert_eq!(analysis.unknown_names(&builtins()), 1);
    }

    #[test]
    fn the_layout_example_in_the_header_is_a_working_layout_once_uncommented() {
        let at = EXAMPLE_FILE.find("# layout = [").unwrap();
        let start = EXAMPLE_FILE[..at].rfind("# [defaults]").unwrap();
        let uncommented: String = EXAMPLE_FILE[start..]
            .lines()
            .map(|line| {
                line.strip_prefix("# ")
                    .or_else(|| line.strip_prefix('#'))
                    .unwrap_or(line)
            })
            .collect::<Vec<_>>()
            .join(
                "
",
            );
        let analysis = analyze(&parse(&uncommented), &builtins());
        assert_eq!(analysis.ignored, 0);
        assert_eq!(analysis.default_layout.len(), 3);
        assert_eq!(analysis.projects[0].layout.len(), 2);
        // As it stands, the file has no layout at all.
        assert!(
            analyze(&parse(EXAMPLE_FILE), &builtins())
                .default_layout
                .is_empty()
        );
    }
}
