//! Which apps are running. Section 01 6.5.
//!
//! One snapshot of the programs that Windows lets Rain see (the full path of each program file)
//! is turned into an index, and every app is looked up in it. It is a hint for a dot on a row,
//! never used for a decision, so it is best effort:
//! - elevated and system processes cannot be read, so their apps show "not running";
//! - a launcher that starts the real program through a shim is recognised only in the Squirrel
//!   form (`Update.exe --processStart App.exe`);
//! - a Store app whose process lives outside `WindowsApps` shows "not running".

use std::collections::HashSet;

use rain_types::AppKind;

use super::apps::{AppRecord, normalise_path, package_name};

/// What a snapshot of the running programs says, in the forms that the matching needs.
#[derive(Debug, Default)]
pub struct RunningIndex {
    /// Full paths, normalised and lower case.
    paths: HashSet<String>,
    /// File names, lower case.
    files: HashSet<String>,
    /// `(package name, publisher id)` of every process that lives in a `WindowsApps` folder.
    packages: HashSet<(String, String)>,
}

impl RunningIndex {
    pub fn new(images: &[String]) -> Self {
        let mut index = Self::default();
        for image in images {
            let path = normalise_path(image).to_lowercase();
            if let Some(file) = path.rsplit('\\').next() {
                index.files.insert(file.to_owned());
            }
            if let Some(pair) = package_of_path(&path) {
                index.packages.insert(pair);
            }
            index.paths.insert(path);
        }
        index
    }

    /// Whether a process of this app is in the snapshot.
    pub fn is_running(&self, app: &AppRecord) -> bool {
        match app.kind {
            AppKind::Program => {
                if self
                    .paths
                    .contains(&normalise_path(&app.exe_path).to_lowercase())
                {
                    return true;
                }
                // Squirrel shortcuts (Discord, Slack): `Update.exe --processStart App.exe`.
                squirrel_target(&app.link_args).is_some_and(|file| self.files.contains(&file))
            }
            AppKind::Store => {
                package_of_aumid(&app.aumid).is_some_and(|pair| self.packages.contains(&pair))
            }
            AppKind::Other => false,
        }
    }
}

/// The file name after `--processStart` (or `--process-start`), lower case.
fn squirrel_target(args: &str) -> Option<String> {
    let lower = args.to_lowercase();
    let mut words = lower.split_whitespace();
    while let Some(word) = words.next() {
        if word == "--processstart" || word == "--process-start" {
            let file = words.next()?.trim_matches('"');
            return file.rsplit(['\\', '/']).next().map(str::to_owned);
        }
    }
    None
}

/// `<Name>_<PublisherId>!<App>` gives the pair, lower case.
fn package_of_aumid(aumid: &str) -> Option<(String, String)> {
    let package = aumid.split('!').next().filter(|_| aumid.contains('!'))?;
    let name = package_name(aumid)?;
    let publisher = package.rsplit('_').next()?;
    (publisher != package).then(|| (name.to_lowercase(), publisher.to_lowercase()))
}

/// `...\windowsapps\<Name>_<Version>_<Arch>__<PublisherId>\app.exe` gives the pair.
fn package_of_path(path: &str) -> Option<(String, String)> {
    let mut parts = path.split('\\');
    parts.find(|part| *part == "windowsapps")?;
    let folder = parts.next()?;
    let name = folder.split('_').next().filter(|name| !name.is_empty())?;
    let publisher = folder.rsplit('_').next().filter(|p| *p != folder)?;
    Some((name.to_owned(), publisher.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(kind: AppKind, aumid: &str, exe: &str, args: &str) -> AppRecord {
        AppRecord {
            id: "app-x".into(),
            key: "k".into(),
            aumid: aumid.into(),
            name: "X".into(),
            kind,
            exe_path: exe.into(),
            link_args: args.into(),
            keywords: Vec::new(),
            icon_version: "00000000".into(),
            icon_file: "x.png".into(),
        }
    }

    fn index() -> RunningIndex {
        let list: Vec<String> = serde_json::from_str(include_str!(
            "../../../tests/fixtures/launcher/processes.json"
        ))
        .expect("the fixture is JSON");
        RunningIndex::new(&list)
    }

    #[test]
    fn a_program_is_running_when_its_full_path_is_in_the_snapshot() {
        let index = index();
        assert!(index.is_running(&app(
            AppKind::Program,
            "a",
            r"C:\Fake\Editor\editor.exe",
            ""
        )));
        assert!(!index.is_running(&app(AppKind::Program, "a", r"C:\Fake\Editor\other.exe", "")));
    }

    #[test]
    fn paths_are_compared_without_their_spelling() {
        let index = index();
        // The snapshot has `\\?\C:\Fake\Prefixed\tool.exe`, `d:/fake/slashes/tool.exe` and an
        // upper-case drive letter.
        assert!(index.is_running(&app(
            AppKind::Program,
            "a",
            r"C:\Fake\Prefixed\tool.exe",
            ""
        )));
        assert!(index.is_running(&app(AppKind::Program, "a", r"D:\Fake\Slashes\tool.exe", "")));
        assert!(index.is_running(&app(AppKind::Program, "a", r"c:\fake\upper\TOOL.EXE", "")));
    }

    #[test]
    fn a_squirrel_shortcut_is_matched_by_the_program_it_starts() {
        let index = index();
        let chat = app(
            AppKind::Program,
            "a",
            r"C:\Fake\Chat\Update.exe",
            "--processStart Chat.exe",
        );
        assert!(index.is_running(&chat));
        let other = app(
            AppKind::Program,
            "a",
            r"C:\Fake\Chat\Update.exe",
            "--processStart Missing.exe",
        );
        assert!(!index.is_running(&other));
        assert!(!index.is_running(&app(AppKind::Program, "a", r"C:\Fake\Chat\Update.exe", "")));
    }

    #[test]
    fn a_store_app_is_matched_by_package_and_publisher() {
        let index = index();
        let calc = app(AppKind::Store, "Fake.Calculator_abc123!App", "", "");
        assert!(index.is_running(&calc));
        let other_publisher = app(AppKind::Store, "Fake.Calculator_zzz999!App", "", "");
        assert!(!index.is_running(&other_publisher));
        let other_package = app(AppKind::Store, "Fake.Notes_abc123!App", "", "");
        assert!(!index.is_running(&other_package));
    }

    #[test]
    fn an_other_item_is_never_running() {
        assert!(!index().is_running(&app(AppKind::Other, "a", "", "")));
    }

    #[test]
    fn the_package_is_read_from_the_folder_name() {
        assert_eq!(
            package_of_path(
                r"c:\program files\windowsapps\fake.calculator_1.2.3_x64__abc123\calc.exe"
            ),
            Some(("fake.calculator".into(), "abc123".into()))
        );
        assert_eq!(package_of_path(r"c:\fake\calc.exe"), None);
        assert_eq!(
            package_of_aumid("Fake.Calculator_abc123!App"),
            Some(("fake.calculator".into(), "abc123".into()))
        );
        assert_eq!(package_of_aumid("not-a-package"), None);
    }
}
