//! Which projects are open: an editor window exists for the project's folder. Section 02 6.4.
//!
//! The editor puts the folder name in the title of its window (`main.rs - web-shop - Visual
//! Studio Code`). A window counts for a project when its process is the editor program and its
//! title names the project's folder. This is UNPROVEN on every editor but VS Code, so it is
//! deliberately strict: a wrong "closed" only costs a second editor start, which VS Code answers
//! by bringing the folder's window forward.

use std::collections::HashSet;

use crate::platform::process::ProcessLister;
use crate::platform::windows::{WindowInfo, WindowLister};

/// A window of an editor program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorWindow {
    pub title: String,
}

/// The visible windows that belong to one of the programs named `image_names` (file names,
/// compared without regard to case).
///
/// The process lister gives full paths of the running programs but not process ids, so the
/// match is by program: when no process of an editor image runs at all, no window counts; when
/// one does, its windows are the ones whose title fits the folder name. That is enough here,
/// because the title rule is strict.
pub fn editor_windows(
    windows: &dyn WindowLister,
    processes: &dyn ProcessLister,
    image_names: &[String],
) -> Vec<EditorWindow> {
    let running = processes.images().iter().any(|path| {
        let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
        image_names
            .iter()
            .any(|image| image.eq_ignore_ascii_case(name))
    });
    if !running {
        return Vec::new();
    }
    windows
        .windows()
        .into_iter()
        .map(|WindowInfo { title, .. }| EditorWindow { title })
        .collect()
}

/// The folders (by name) that some window shows.
pub fn open_names<'a>(
    windows: &[EditorWindow],
    names: impl Iterator<Item = &'a str>,
) -> HashSet<String> {
    names
        .filter(|name| windows.iter().any(|w| title_matches(&w.title, name)))
        .map(str::to_owned)
        .collect()
}

/// The text without its `[...]` parts.
fn without_tags(text: &str) -> String {
    let mut depth = 0usize;
    let mut out = String::with_capacity(text.len());
    for letter in text.chars() {
        match letter {
            '[' => depth += 1,
            ']' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(letter),
            _ => {}
        }
    }
    out
}

/// Whether a window title names the folder `name`.
///
/// Rule: strip a leading `*` or bullet and spaces; strip a trailing ` (Workspace)` and trailing
/// `[...]` tags such as `[Administrator]`; then the title matches when it is the name, starts
/// with `name - `, contains ` - name - `, or ends with ` - name` (en and em dashes count as the
/// hyphen). Case does not matter.
pub fn title_matches(title: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let trimmed = title
        .trim()
        .trim_start_matches(['*', '\u{25CF}', '\u{2022}', ' ']);
    // The tags that an editor adds next to the folder name: `[Administrator]`, `(Workspace)`.
    let text = without_tags(trimmed)
        .to_lowercase()
        .replace("(workspace)", "");
    // One kind of dash and one kind of space make the rule shorter to state.
    let text = text
        .replace(['\u{2013}', '\u{2014}'], "-")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let name = name.to_lowercase();
    text == name
        || text.starts_with(&format!("{name} - "))
        || text.contains(&format!(" - {name} - "))
        || text.ends_with(&format!(" - {name}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn titles_that_name_the_folder_match() {
        let name = "web-shop";
        for title in [
            "main.rs - web-shop - Visual Studio Code",
            "web-shop - Visual Studio Code",
            "* main.rs - web-shop - Visual Studio Code",
            "\u{25CF} main.rs - web-shop - Visual Studio Code",
            "web-shop (Workspace) - Visual Studio Code",
            "web-shop [Administrator] - Visual Studio Code",
            "main.rs \u{2014} web-shop \u{2014} Visual Studio Code",
            "WEB-SHOP - Visual Studio Code",
            "Welcome - web-shop",
        ] {
            assert!(title_matches(title, name), "{title}");
        }
    }

    #[test]
    fn titles_of_other_folders_and_windows_do_not_match() {
        let name = "web-shop";
        for title in [
            "web-shop-old - Visual Studio Code",
            "web - Visual Studio Code",
            "notes about web-shop - Chrome",
            "Visual Studio Code",
            "",
        ] {
            assert!(!title_matches(title, name), "{title}");
        }
        assert!(!title_matches("anything", ""));
    }

    #[test]
    fn a_folder_name_with_a_dash_inside_matches_whole() {
        assert!(title_matches(
            "a.rs - my - app - Visual Studio Code",
            "my - app"
        ));
        assert!(!title_matches(
            "a.rs - my - app - Visual Studio Code",
            "app - x"
        ));
    }

    struct Windows(Vec<WindowInfo>);
    impl WindowLister for Windows {
        fn windows(&self) -> Vec<WindowInfo> {
            self.0.clone()
        }
    }
    struct Processes(Mutex<Vec<String>>);
    impl ProcessLister for Processes {
        fn images(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }

    fn window(title: &str) -> WindowInfo {
        WindowInfo {
            pid: 1,
            title: title.to_owned(),
        }
    }

    #[test]
    fn only_windows_of_a_running_editor_count() {
        let windows = Windows(vec![window("web-shop - Visual Studio Code")]);
        let names = vec!["Code.exe".to_owned()];

        let editor_running = Processes(Mutex::new(vec!["C:\\Apps\\Code\\code.EXE".into()]));
        let found = editor_windows(&windows, &editor_running, &names);
        assert_eq!(
            open_names(&found, ["web-shop", "other"].into_iter()).len(),
            1
        );

        let no_editor = Processes(Mutex::new(vec!["C:\\Apps\\chrome.exe".into()]));
        assert!(editor_windows(&windows, &no_editor, &names).is_empty());
    }
}
