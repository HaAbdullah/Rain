//! The setup of Claude: plugins, MCP servers, hooks and output styles, as names and short facts.
//!
//! What is read: the user's settings file (`hooks`, `enabledPlugins`, `mcpServers`), the state file
//! of Claude Code (`mcpServers` only), the list of installed plugins, and the `.mcp.json` and
//! hook files of plugins and projects. Each file is parsed into a small private shape that holds
//! the wanted keys and nothing else, so an `env` or `headers` block, a token or a command line of
//! an MCP server never gets into memory beyond that parse, and never reaches the answer. The
//! credentials file of Claude Code is never opened. Blocking code: run it through `blocking::run`.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use rain_types::{LibraryHook, LibraryMcp, LibraryOutputStyle, LibraryPlugin, LibrarySetup};
use serde::Deserialize;
use serde_json::Value;

use super::frontmatter::{self, shorten};
use super::scan::{KnownProject, MAX_PLUGINS, PluginRoot, scope_plugin, scope_project, scope_you};
use crate::modules::sessions::claude_paths::{ClaudePaths, project_claude_dir};
use crate::modules::sessions::hooks_install::settings_file;

/// A settings or state file bigger than this is not read.
const FILE_MAX_BYTES: u64 = 8 * 1024 * 1024;
const HOOKS_MAX: usize = 300;
const MCP_MAX: usize = 200;
const STYLES_MAX: usize = 100;
const COMMAND_MAX_CHARS: usize = 200;

/// The keys of the settings file that the library wants. Every other key is dropped by the parse.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SettingsShape {
    hooks: Value,
    #[serde(rename = "enabledPlugins")]
    enabled_plugins: BTreeMap<String, Value>,
    #[serde(rename = "mcpServers")]
    mcp_servers: BTreeMap<String, Value>,
}

/// The only key of the state file of Claude Code that the library wants.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct StateShape {
    #[serde(rename = "mcpServers")]
    mcp_servers: BTreeMap<String, Value>,
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    let Ok(meta) = fs::metadata(path) else {
        return T::default();
    };
    if !meta.is_file() || meta.len() > FILE_MAX_BYTES {
        return T::default();
    }
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

// ---- plugins ----

/// The installed plugins: the list file of Claude Code when there is one, else the folders of
/// `plugins/cache/<marketplace>/<plugin>/<version>`. A path from the list file counts only when
/// it lies inside the plugins folder.
pub fn plugin_roots(paths: &ClaudePaths) -> Vec<PluginRoot> {
    let plugins_dir = paths.root().join("plugins");
    let listed: Value = read_json(&plugins_dir.join("installed_plugins.json"));
    let mut found: Vec<PluginRoot> = Vec::new();

    if let Some(map) = listed
        .get("plugins")
        .and_then(Value::as_object)
        .or_else(|| listed.as_object())
    {
        for (key, entry) in map {
            // Version 2 holds a list of installs for each plugin, version 1 one object.
            let installs: Vec<&Value> = match entry {
                Value::Array(list) => list.iter().collect(),
                Value::Object(_) => vec![entry],
                _ => continue,
            };
            let Some(install) = installs.first() else {
                continue;
            };
            let dir = install
                .get("installPath")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .filter(|dir| dir.starts_with(&plugins_dir) && dir.is_dir());
            if let Some(dir) = dir {
                found.push(PluginRoot {
                    name: plugin_name(key),
                    version: install
                        .get("version")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    dir,
                });
            }
        }
    }

    if found.is_empty() {
        found = cached_plugins(&plugins_dir.join("cache"));
    }
    found.truncate(MAX_PLUGINS);
    found
}

/// `name@marketplace` is `name`.
fn plugin_name(key: &str) -> String {
    key.split('@').next().unwrap_or(key).to_owned()
}

fn child_dirs(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut list: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                entry.path(),
            )
        })
        .collect();
    list.sort();
    list
}

/// The layout `cache/<marketplace>/<plugin>/<version>`; of several versions the last one counts.
fn cached_plugins(cache: &Path) -> Vec<PluginRoot> {
    let mut out = Vec::new();
    for (_, market) in child_dirs(cache) {
        for (name, plugin) in child_dirs(&market) {
            if let Some((version, dir)) = child_dirs(&plugin).pop() {
                out.push(PluginRoot {
                    name,
                    version: Some(version),
                    dir,
                });
            }
        }
    }
    out
}

// ---- the whole setup ----

pub fn read_setup(
    paths: &ClaudePaths,
    plugins: &[PluginRoot],
    projects: &[KnownProject],
) -> LibrarySetup {
    let settings: SettingsShape = read_json(&settings_file(paths));
    let state: StateShape = paths
        .state_file()
        .map(|file| read_json(&file))
        .unwrap_or_default();

    LibrarySetup {
        plugins: plugin_facts(plugins, &settings),
        mcp_servers: mcp_facts(&settings, &state, plugins, projects),
        hooks: hook_facts(&settings, plugins),
        output_styles: style_facts(paths, plugins, projects),
    }
}

fn plugin_facts(plugins: &[PluginRoot], settings: &SettingsShape) -> Vec<LibraryPlugin> {
    // `enabledPlugins` is keyed `name@marketplace`.
    let enabled: HashMap<String, bool> = settings
        .enabled_plugins
        .iter()
        .map(|(key, value)| (plugin_name(key), value.as_bool().unwrap_or(false)))
        .collect();
    let mut out: Vec<LibraryPlugin> = plugins
        .iter()
        .map(|plugin| LibraryPlugin {
            name: plugin.name.clone(),
            version: plugin.version.clone(),
            enabled: enabled.get(&plugin.name).copied(),
        })
        .collect();
    // A plugin that the settings enable but the list does not show is still part of the setup.
    for (name, on) in &enabled {
        if !out.iter().any(|plugin| &plugin.name == name) {
            out.push(LibraryPlugin {
                name: name.clone(),
                version: None,
                enabled: Some(*on),
            });
        }
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

// ---- MCP servers ----

/// The name of a server and its kind. Nothing else of the entry is looked at.
fn transport_of(entry: &Value) -> Option<String> {
    let declared = entry.get("type").and_then(Value::as_str);
    match declared {
        Some(kind @ ("stdio" | "http" | "sse")) => Some(kind.to_owned()),
        _ if entry.get("command").is_some() => Some("stdio".to_owned()),
        _ if entry.get("url").is_some() => Some("http".to_owned()),
        _ => None,
    }
}

fn mcp_from_map(map: &BTreeMap<String, Value>, scope: &str, out: &mut Vec<LibraryMcp>) {
    for (name, entry) in map {
        out.push(LibraryMcp {
            name: shorten(name, 80),
            scope: scope.to_owned(),
            transport: transport_of(entry),
        });
    }
}

/// A `.mcp.json` has `mcpServers`, and a plugin's may be the plain map of servers.
fn mcp_from_file(path: &Path, scope: &str, out: &mut Vec<LibraryMcp>) {
    let file: Value = read_json(path);
    let servers = file
        .get("mcpServers")
        .and_then(Value::as_object)
        .or_else(|| file.as_object().filter(|map| looks_like_servers(map)));
    if let Some(map) = servers {
        let wanted: BTreeMap<String, Value> =
            map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        mcp_from_map(&wanted, scope, out);
    }
}

/// A plain map of servers: every value is an object that has a command or an address.
fn looks_like_servers(map: &serde_json::Map<String, Value>) -> bool {
    !map.is_empty()
        && map
            .values()
            .all(|value| value.get("command").is_some() || value.get("url").is_some())
}

fn mcp_facts(
    settings: &SettingsShape,
    state: &StateShape,
    plugins: &[PluginRoot],
    projects: &[KnownProject],
) -> Vec<LibraryMcp> {
    let mut out = Vec::new();
    let you = scope_you();
    mcp_from_map(&state.mcp_servers, &you, &mut out);
    mcp_from_map(&settings.mcp_servers, &you, &mut out);
    for project in projects {
        mcp_from_file(
            &project.path.join(".mcp.json"),
            &scope_project(project),
            &mut out,
        );
    }
    for plugin in plugins {
        mcp_from_file(
            &plugin.dir.join(".mcp.json"),
            &scope_plugin(plugin),
            &mut out,
        );
    }
    out.truncate(MCP_MAX);
    out
}

// ---- hooks ----

fn hook_facts(settings: &SettingsShape, plugins: &[PluginRoot]) -> Vec<LibraryHook> {
    let mut out = Vec::new();
    hooks_from_value(&settings.hooks, &scope_you(), &mut out);
    for plugin in plugins {
        let file: Value = read_json(&plugin.dir.join("hooks").join("hooks.json"));
        // The file has the same shape as the settings, under a key `hooks`.
        let hooks = file.get("hooks").unwrap_or(&file);
        hooks_from_value(hooks, &scope_plugin(plugin), &mut out);
    }
    out.truncate(HOOKS_MAX);
    out
}

/// `{ "<Event>": [ { "matcher": "..", "hooks": [ { "type": "command", "command": ".." } ] } ] }`
fn hooks_from_value(hooks: &Value, scope: &str, out: &mut Vec<LibraryHook>) {
    let Some(events) = hooks.as_object() else {
        return;
    };
    for (event, groups) in events {
        for group in groups.as_array().into_iter().flatten() {
            let matcher = group
                .get("matcher")
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(|text| shorten(text, 80));
            for handler in group
                .get("hooks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                // A handler of another type (a prompt, a URL) has no command: its kind is shown.
                let command = handler
                    .get("command")
                    .and_then(Value::as_str)
                    .map(|text| shorten(&redact(text), COMMAND_MAX_CHARS))
                    .or_else(|| {
                        handler
                            .get("type")
                            .and_then(Value::as_str)
                            .map(|kind| format!("({})", shorten(kind, 20)))
                    });
                if let Some(command) = command {
                    out.push(LibraryHook {
                        event: shorten(event, 40),
                        matcher: matcher.clone(),
                        command,
                        scope: scope.to_owned(),
                    });
                }
            }
        }
    }
}

/// Replaces what looks like a secret by `***`: a word that starts like a known token, a long run
/// of letters and digits, the value of `NAME_KEY=value`, and the word after `Bearer`.
pub fn redact(command: &str) -> String {
    const PREFIXES: [&str; 8] = [
        "sk-",
        "ghp_",
        "gho_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
        "AKIA",
        "eyJ",
    ];
    const SECRET_NAMES: [&str; 5] = ["KEY", "TOKEN", "SECRET", "PASSWORD", "PASSWD"];
    let mut after_bearer = false;
    let mut words = Vec::new();
    for word in command.split(' ') {
        let (head, value) = match word.split_once('=') {
            Some((head, value)) => (head, value),
            None => ("", word),
        };
        let named_secret = !head.is_empty()
            && SECRET_NAMES
                .iter()
                .any(|name| head.to_uppercase().contains(name))
            && !value.is_empty();
        let trimmed = value.trim_matches(|c| c == '"' || c == '\'');
        let looks_secret = PREFIXES.iter().any(|prefix| trimmed.starts_with(prefix))
            || (trimmed.len() >= 32
                && trimmed
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
        if after_bearer || named_secret || looks_secret {
            words.push(if head.is_empty() {
                "***".to_owned()
            } else {
                format!("{head}=***")
            });
        } else {
            words.push(word.to_owned());
        }
        after_bearer = word.eq_ignore_ascii_case("bearer");
    }
    words.join(" ")
}

// ---- output styles ----

fn style_facts(
    paths: &ClaudePaths,
    plugins: &[PluginRoot],
    projects: &[KnownProject],
) -> Vec<LibraryOutputStyle> {
    let mut out = Vec::new();
    styles_in(&paths.root().join("output-styles"), &scope_you(), &mut out);
    for project in projects {
        let dir = project_claude_dir(&project.path).join("output-styles");
        styles_in(&dir, &scope_project(project), &mut out);
    }
    for plugin in plugins {
        styles_in(
            &plugin.dir.join("output-styles"),
            &scope_plugin(plugin),
            &mut out,
        );
    }
    out.truncate(STYLES_MAX);
    out
}

fn styles_in(dir: &Path, scope: &str, out: &mut Vec<LibraryOutputStyle>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        })
        .collect();
    files.sort();
    for file in files.into_iter().take(STYLES_MAX) {
        // Only the head of the file: the name is in the front matter.
        let head = read_head(&file, 4096);
        let parsed = frontmatter::parse(&head);
        let stem = file
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        out.push(LibraryOutputStyle {
            name: shorten(parsed.fact("name").unwrap_or(&stem), 80),
            scope: scope.to_owned(),
        });
    }
}

/// The first `max` bytes of a file as text. Empty when it cannot be read.
pub fn read_head(path: &Path, max: usize) -> String {
    use std::io::Read;
    let Ok(file) = fs::File::open(path) else {
        return String::new();
    };
    let mut bytes = Vec::new();
    if file.take(max as u64).read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn secret_like_words_are_replaced() {
        assert_eq!(
            redact("node run.js --token sk-abcdefghijklmnop API_KEY=hunter2 ok"),
            "node run.js --token *** API_KEY=*** ok"
        );
        assert_eq!(
            redact("curl -H Authorization: Bearer abc123 url"),
            "curl -H Authorization: Bearer *** url"
        );
        let long = "a".repeat(40);
        assert_eq!(redact(&format!("run {long}")), "run ***");
        assert_eq!(redact("echo plain words"), "echo plain words");
    }

    #[test]
    fn hooks_come_out_as_event_matcher_and_command() {
        let hooks = json!({
            "PreToolUse": [
                { "matcher": "Bash", "hooks": [ { "type": "command", "command": "check.sh" } ] }
            ],
            "Stop": [ { "hooks": [ { "type": "prompt", "prompt": "secret text" } ] } ]
        });
        let mut out = Vec::new();
        hooks_from_value(&hooks, "You", &mut out);
        assert_eq!(out.len(), 2);
        let pre = out.iter().find(|hook| hook.event == "PreToolUse").unwrap();
        assert_eq!(pre.matcher.as_deref(), Some("Bash"));
        assert_eq!(pre.command, "check.sh");
        let stop = out.iter().find(|hook| hook.event == "Stop").unwrap();
        assert_eq!(
            stop.command, "(prompt)",
            "the text of a prompt hook is not shown"
        );
    }

    #[test]
    fn mcp_servers_give_a_name_and_a_kind_and_nothing_else() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("claude");
        let paths = ClaudePaths::with_root(root);
        write(
            &settings_file(&paths),
            r#"{ "mcpServers": { "files": { "command": "npx", "args": ["x"], "env": { "TOKEN": "SECRET-ENV-VALUE" } } } }"#,
        );
        let plugin_dir = temp.path().join("plug");
        write(
            &plugin_dir.join(".mcp.json"),
            r#"{ "remote": { "type": "http", "url": "https://x.test", "headers": { "Authorization": "SECRET-HEADER" } } }"#,
        );
        let plugin = PluginRoot {
            name: "tools".into(),
            version: Some("1.2.0".into()),
            dir: plugin_dir,
        };
        let setup = read_setup(&paths, &[plugin], &[]);
        let wire = serde_json::to_string(&setup).unwrap();
        assert!(!wire.contains("SECRET"), "{wire}");
        let names: Vec<_> = setup
            .mcp_servers
            .iter()
            .map(|m| (m.name.as_str(), m.scope.as_str(), m.transport.as_deref()))
            .collect();
        assert!(names.contains(&("files", "You", Some("stdio"))));
        assert!(names.contains(&("remote", "Plugin tools", Some("http"))));
    }

    #[test]
    fn plugins_come_from_the_list_file_and_stay_inside_the_plugins_folder() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("claude");
        let inside = root.join("plugins/cache/m/good/1.0.0");
        write(&inside.join("skills/s/SKILL.md"), "x");
        let outside = temp.path().join("elsewhere");
        fs::create_dir_all(&outside).unwrap();
        write(
            &root.join("plugins/installed_plugins.json"),
            &json!({ "version": 2, "plugins": {
                "good@m": [ { "scope": "user", "version": "1.0.0", "installPath": inside } ],
                "bad@m": [ { "version": "9", "installPath": outside } ]
            }})
            .to_string(),
        );
        let found = plugin_roots(&ClaudePaths::with_root(root));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "good");
        assert_eq!(found[0].version.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn without_a_list_file_the_cache_folders_are_the_plugins() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("claude");
        fs::create_dir_all(root.join("plugins/cache/m/alpha/0.1.0")).unwrap();
        fs::create_dir_all(root.join("plugins/cache/m/alpha/0.2.0")).unwrap();
        let found = plugin_roots(&ClaudePaths::with_root(root));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].version.as_deref(), Some("0.2.0"));
    }

    #[test]
    fn enabled_plugins_of_the_settings_are_listed_with_their_switch() {
        let settings = SettingsShape {
            enabled_plugins: BTreeMap::from([("solo@m".to_owned(), json!(false))]),
            ..SettingsShape::default()
        };
        let list = plugin_facts(&[], &settings);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].enabled, Some(false));
    }
}
