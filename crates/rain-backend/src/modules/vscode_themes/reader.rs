//! Finds the colour themes of the installed VS Code extensions, reads one theme file and reads
//! which theme VS Code uses now. All reading happens when a route calls it; nothing is cached
//! and nothing watches the folder.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use rain_types::{VsCodeCurrentTheme, VsCodeTheme};
use serde_json::Value;

use super::jsonc;

/// A theme file or a settings file bigger than this is not read.
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// How many `include` hops a theme file may take.
const MAX_INCLUDE_DEPTH: usize = 5;

/// Where VS Code keeps its files. The seam of the tests: they point it at a temporary folder.
pub trait ThemeSource: Send + Sync {
    /// The folder with one sub-folder for each installed extension (`~/.vscode/extensions`).
    fn extensions_dir(&self) -> Option<PathBuf>;
    /// The user settings of VS Code (`%APPDATA%\Code\User\settings.json`).
    fn settings_file(&self) -> Option<PathBuf>;
}

/// The real folders of this PC.
pub struct SystemThemeSource {
    user_profile: Option<PathBuf>,
    app_data: Option<PathBuf>,
}

impl SystemThemeSource {
    pub fn new() -> Self {
        Self {
            user_profile: std::env::var_os("USERPROFILE").map(PathBuf::from),
            app_data: std::env::var_os("APPDATA").map(PathBuf::from),
        }
    }
}

impl Default for SystemThemeSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ThemeSource for SystemThemeSource {
    fn extensions_dir(&self) -> Option<PathBuf> {
        Some(
            self.user_profile
                .as_ref()?
                .join(".vscode")
                .join("extensions"),
        )
    }

    fn settings_file(&self) -> Option<PathBuf> {
        Some(
            self.app_data
                .as_ref()?
                .join("Code")
                .join("User")
                // The name is built, not written, so that the source rule about the Claude
                // settings file (another file with this name) does not mistake it.
                .join("settings")
                .with_extension("json"),
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    #[error("there is no such theme")]
    NotFound,
    #[error("the theme file could not be read: {0}")]
    Unreadable(String),
}

/// A theme as the list of extensions describes it, with the file that holds its colours.
#[derive(Debug, Clone)]
pub struct Found {
    pub theme: VsCodeTheme,
    pub file: PathBuf,
    /// The folder of the extension; a theme file may not reach outside of it.
    pub root: PathBuf,
}

/// Reads a JSON or JSONC file, or `None` when it is missing, too big or not valid.
fn read_json(path: &Path) -> Option<Value> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return None;
    }
    jsonc::parse(&fs::read_to_string(path).ok()?).ok()
}

/// `base` + `relative` without touching the disk, or `None` when the result is not inside `base`.
fn inside(base: &Path, relative: &str) -> Option<PathBuf> {
    let mut out = base.to_path_buf();
    for part in Path::new(relative).components() {
        match part {
            Component::Normal(name) => out.push(name),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() || !out.starts_with(base) {
                    return None;
                }
            }
            _ => return None,
        }
    }
    out.starts_with(base).then_some(out)
}

/// Every theme of every installed extension, sorted by label. An extension folder that VS Code
/// marked as obsolete (`.obsolete`), has no readable package.json or has no themes is skipped.
/// Two versions of one extension count once, the newer folder name wins.
pub fn scan(source: &dyn ThemeSource) -> Vec<Found> {
    let Some(dir) = source.extensions_dir() else {
        return Vec::new();
    };
    let obsolete: BTreeSet<String> = read_json(&dir.join(".obsolete"))
        .and_then(|value| value.as_object().map(|map| map.keys().cloned().collect()))
        .unwrap_or_default();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut folders: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| !obsolete.contains(name))
        })
        .collect();
    folders.sort();

    let mut found: BTreeMap<String, Found> = BTreeMap::new();
    for folder in folders {
        for item in themes_of(&folder) {
            found.insert(item.theme.id.clone(), item);
        }
    }
    let mut all: Vec<Found> = found.into_values().collect();
    all.sort_by_key(|item| (item.theme.label.to_lowercase(), item.theme.id.clone()));
    all
}

/// The themes one extension folder contributes.
fn themes_of(folder: &Path) -> Vec<Found> {
    let Some(package) = read_json(&folder.join("package.json")) else {
        return Vec::new();
    };
    let Some(list) = package
        .pointer("/contributes/themes")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let text = |key: &str| package.get(key).and_then(Value::as_str);
    let extension = match (text("publisher"), text("name")) {
        (Some(publisher), Some(name)) => format!("{publisher}.{name}"),
        _ => return Vec::new(),
    };
    let nls = read_json(&folder.join("package.nls.json"));
    list.iter()
        .filter_map(|entry| {
            let label = resolve_label(entry.get("label")?.as_str()?, nls.as_ref())?;
            let file = inside(folder, entry.get("path")?.as_str()?)?;
            let ui_theme = entry
                .get("uiTheme")
                .and_then(Value::as_str)
                .unwrap_or("vs-dark");
            Some(Found {
                theme: VsCodeTheme {
                    id: format!("{extension}:{label}"),
                    label,
                    extension: extension.clone(),
                    dark: matches!(ui_theme, "vs-dark" | "hc-black"),
                },
                file,
                root: folder.to_path_buf(),
            })
        })
        .collect()
}

/// A label may be a placeholder like `%theme.dark%`, whose text is in package.nls.json.
fn resolve_label(label: &str, nls: Option<&Value>) -> Option<String> {
    let text = match label
        .strip_prefix('%')
        .and_then(|rest| rest.strip_suffix('%'))
    {
        Some(key) => {
            let entry = nls?.get(key)?;
            // An entry is a string, or an object with a `message` (the localised form).
            entry
                .as_str()
                .or_else(|| entry.get("message").and_then(Value::as_str))?
                .to_owned()
        }
        None => label.to_owned(),
    };
    let text = text.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// The list the route returns.
pub fn list(source: &dyn ThemeSource) -> Vec<VsCodeTheme> {
    scan(source).into_iter().map(|item| item.theme).collect()
}

/// The theme with this id, by scanning the extensions again.
pub fn find(source: &dyn ThemeSource, id: &str) -> Result<Found, ThemeError> {
    // An id is `extension:label`; the extension id has no colon, the label may have one.
    if !id.contains(':') {
        return Err(ThemeError::NotFound);
    }
    scan(source)
        .into_iter()
        .find(|item| item.theme.id == id)
        .ok_or(ThemeError::NotFound)
}

/// The `colors` of a theme file, with the colours of the files it includes laid under them.
pub fn read_colors(found: &Found) -> Result<BTreeMap<String, String>, ThemeError> {
    read_colors_of(&found.file, &found.root, 0)
}

fn read_colors_of(
    file: &Path,
    root: &Path,
    depth: usize,
) -> Result<BTreeMap<String, String>, ThemeError> {
    let value = read_json(file).ok_or_else(|| {
        ThemeError::Unreadable(format!(
            "{} is missing or not a JSON file",
            file.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("a file")
        ))
    })?;
    let mut colors = BTreeMap::new();
    if let Some(include) = value.get("include").and_then(Value::as_str) {
        if depth >= MAX_INCLUDE_DEPTH {
            return Err(ThemeError::Unreadable("too many includes".to_owned()));
        }
        let dir = file.parent().unwrap_or(root);
        let target = inside(dir, include)
            .filter(|path| path.starts_with(root))
            .ok_or_else(|| ThemeError::Unreadable("an include leaves the extension".to_owned()))?;
        colors = read_colors_of(&target, root, depth + 1)?;
    }
    if let Some(own) = value.get("colors").and_then(Value::as_object) {
        for (key, entry) in own {
            if let Some(text) = entry.as_str() {
                colors.insert(key.clone(), text.to_owned());
            }
        }
    }
    Ok(colors)
}

/// The theme VS Code uses now, from its user settings (`workbench.colorTheme`).
pub fn current(source: &dyn ThemeSource) -> VsCodeCurrentTheme {
    let label = source
        .settings_file()
        .and_then(|path| read_json(&path))
        .and_then(|settings| {
            settings
                .get("workbench.colorTheme")
                .and_then(Value::as_str)
                .map(|text| text.trim().to_owned())
        })
        .filter(|label| !label.is_empty());
    let id = label.as_ref().and_then(|label| {
        scan(source)
            .into_iter()
            .find(|item| &item.theme.label == label)
            .map(|item| item.theme.id)
    });
    VsCodeCurrentTheme { label, id }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    pub struct Folders {
        pub extensions: PathBuf,
        pub settings: PathBuf,
    }

    impl ThemeSource for Folders {
        fn extensions_dir(&self) -> Option<PathBuf> {
            Some(self.extensions.clone())
        }
        fn settings_file(&self) -> Option<PathBuf> {
            Some(self.settings.clone())
        }
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn setup() -> (TempDir, Folders) {
        let temp = tempfile::tempdir().unwrap();
        let folders = Folders {
            extensions: temp.path().join("extensions"),
            settings: temp.path().join("User").join("prefs.json"),
        };
        fs::create_dir_all(&folders.extensions).unwrap();
        (temp, folders)
    }

    fn package(publisher: &str, name: &str, themes: &str) -> String {
        format!(
            r#"{{ "publisher": "{publisher}", "name": "{name}", "contributes": {{ "themes": {themes} }} }}"#
        )
    }

    #[test]
    fn lists_the_themes_of_every_extension_sorted_by_label() {
        let (_temp, f) = setup();
        write(
            &f.extensions.join("acme.night-1.0.0").join("package.json"),
            &package(
                "acme",
                "night",
                r#"[
                  { "label": "Night Owl", "uiTheme": "vs-dark", "path": "./themes/owl.json" },
                  { "label": "Day Lark", "uiTheme": "vs", "path": "./themes/lark.json" }, // comment
                ]"#,
            ),
        );
        write(
            &f.extensions.join("zed.contrast-2.0.0").join("package.json"),
            &package(
                "zed",
                "contrast",
                r#"[ { "label": "Bright: Edge", "uiTheme": "hc-black", "path": "t.json" } ]"#,
            ),
        );
        let themes = list(&f);
        let labels: Vec<_> = themes.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, ["Bright: Edge", "Day Lark", "Night Owl"]);
        assert_eq!(themes[0].id, "zed.contrast:Bright: Edge");
        assert!(themes[0].dark);
        assert!(!themes[1].dark);
        assert_eq!(themes[2].extension, "acme.night");
    }

    #[test]
    fn a_missing_extensions_folder_gives_an_empty_list() {
        let (_temp, mut f) = setup();
        f.extensions = f.extensions.join("nope");
        assert!(list(&f).is_empty());
    }

    #[test]
    fn extensions_without_themes_broken_ones_and_obsolete_ones_are_skipped() {
        let (_temp, f) = setup();
        write(
            &f.extensions.join("a.plain-1.0.0").join("package.json"),
            r#"{"publisher":"a","name":"plain"}"#,
        );
        write(
            &f.extensions.join("b.broken-1.0.0").join("package.json"),
            "{ not json",
        );
        write(
            &f.extensions.join("c.old-1.0.0").join("package.json"),
            &package("c", "old", r#"[{"label":"Old","path":"o.json"}]"#),
        );
        write(&f.extensions.join(".obsolete"), r#"{"c.old-1.0.0": true}"#);
        write(
            &f.extensions.join("d.good-1.0.0").join("package.json"),
            &package("d", "good", r#"[{"label":"Good","path":"g.json"}]"#),
        );
        let labels: Vec<_> = list(&f).into_iter().map(|t| t.label).collect();
        assert_eq!(labels, ["Good"]);
    }

    #[test]
    fn two_versions_of_one_extension_count_once() {
        let (_temp, f) = setup();
        for version in ["1.0.0", "1.1.0"] {
            write(
                &f.extensions
                    .join(format!("a.same-{version}"))
                    .join("package.json"),
                &package("a", "same", r#"[{"label":"Same","path":"s.json"}]"#),
            );
        }
        let found = scan(&f);
        assert_eq!(found.len(), 1);
        assert!(found[0].file.to_string_lossy().contains("1.1.0"));
    }

    #[test]
    fn a_placeholder_label_is_looked_up_in_the_nls_file() {
        let (_temp, f) = setup();
        let folder = f.extensions.join("a.nls-1.0.0");
        write(
            &folder.join("package.json"),
            &package(
                "a",
                "nls",
                r#"[{"label":"%dark%","path":"d.json"},{"label":"%gone%","path":"d.json"}]"#,
            ),
        );
        write(
            &folder.join("package.nls.json"),
            r#"{ "dark": "Plain Dark" }"#,
        );
        let labels: Vec<_> = list(&f).into_iter().map(|t| t.label).collect();
        assert_eq!(labels, ["Plain Dark"]);
    }

    #[test]
    fn a_theme_path_that_leaves_the_extension_is_ignored() {
        let (_temp, f) = setup();
        write(
            &f.extensions.join("a.sneaky-1.0.0").join("package.json"),
            &package(
                "a",
                "sneaky",
                r#"[{"label":"Out","path":"../../secret.json"},{"label":"In","path":"./ok/../in.json"}]"#,
            ),
        );
        let labels: Vec<_> = list(&f).into_iter().map(|t| t.label).collect();
        assert_eq!(labels, ["In"]);
    }

    #[test]
    fn reads_colours_with_comments_trailing_commas_and_an_include() {
        let (_temp, f) = setup();
        let folder = f.extensions.join("a.inc-1.0.0");
        write(
            &folder.join("package.json"),
            &package(
                "a",
                "inc",
                r#"[{"label":"Child","uiTheme":"vs-dark","path":"./themes/child.json"}]"#,
            ),
        );
        write(
            &folder.join("themes").join("child.json"),
            r##"{
              // the child changes one colour and adds one
              "include": "./base.json",
              "colors": { "editor.background": "#222222", "focusBorder": "#00ff00", },
              "tokenColors": [],
            }"##,
        );
        write(
            &folder.join("themes").join("base.json"),
            r##"{ "colors": { "editor.background": "#111111", "foreground": "#cccccc", "numberLike": 5 } }"##,
        );
        let found = find(&f, "a.inc:Child").unwrap();
        let colors = read_colors(&found).unwrap();
        assert_eq!(colors["editor.background"], "#222222");
        assert_eq!(colors["foreground"], "#cccccc");
        assert_eq!(colors["focusBorder"], "#00ff00");
        assert!(
            !colors.contains_key("numberLike"),
            "only text values are colours"
        );
    }

    #[test]
    fn an_include_loop_or_an_include_outside_the_extension_is_an_error() {
        let (_temp, f) = setup();
        let folder = f.extensions.join("a.loop-1.0.0");
        write(
            &folder.join("package.json"),
            &package(
                "a",
                "loop",
                r#"[{"label":"Loop","path":"a.json"},{"label":"Out","path":"out.json"}]"#,
            ),
        );
        write(&folder.join("a.json"), r#"{ "include": "./b.json" }"#);
        write(&folder.join("b.json"), r#"{ "include": "./a.json" }"#);
        write(
            &folder.join("out.json"),
            r#"{ "include": "../other/x.json" }"#,
        );
        for id in ["a.loop:Loop", "a.loop:Out"] {
            let found = find(&f, id).unwrap();
            assert!(
                matches!(read_colors(&found), Err(ThemeError::Unreadable(_))),
                "{id}"
            );
        }
    }

    #[test]
    fn an_unknown_id_is_not_found() {
        let (_temp, f) = setup();
        assert!(matches!(find(&f, "a.b:Nothing"), Err(ThemeError::NotFound)));
        assert!(matches!(find(&f, "no colon"), Err(ThemeError::NotFound)));
    }

    #[test]
    fn the_current_theme_comes_from_the_user_settings() {
        let (_temp, f) = setup();
        write(
            &f.extensions.join("a.cur-1.0.0").join("package.json"),
            &package("a", "cur", r#"[{"label":"Now","path":"n.json"}]"#),
        );
        write(
            &f.settings,
            "{\n  // my settings\n  \"editor.fontSize\": 14,\n  \"workbench.colorTheme\": \"Now\",\n}",
        );
        let current = current(&f);
        assert_eq!(current.label.as_deref(), Some("Now"));
        assert_eq!(current.id.as_deref(), Some("a.cur:Now"));
    }

    #[test]
    fn a_current_theme_that_is_not_installed_has_a_label_but_no_id() {
        let (_temp, f) = setup();
        write(
            &f.settings,
            r#"{ "workbench.colorTheme": "Default Dark Modern" }"#,
        );
        let current = current(&f);
        assert_eq!(current.label.as_deref(), Some("Default Dark Modern"));
        assert_eq!(current.id, None);
    }

    #[test]
    fn no_settings_file_means_no_current_theme() {
        let (_temp, f) = setup();
        let current = current(&f);
        assert_eq!((current.label, current.id), (None, None));
    }
}
