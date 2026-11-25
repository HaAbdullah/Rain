//! Which programs game mode may close. Section 06 6.11.
//!
//! Pure: a scan and the rules go in, a list of programs comes out. Nothing here touches the PC,
//! so every rule is a plain table test.
//!
//! A process is never touched when any of these is true:
//! - it is not the signed-in user's in this session, or Windows would not let Rain open it
//!   (services, other users, elevated programs);
//! - its program file is under `%WINDIR%`, or its name is a shell program (`keep::SHELL`);
//! - it is Rain: this process, the programs it was started from, anything in Rain's own folder
//!   and everything those started (the WebView);
//! - it is Claude: the Claude app, and the terminal (with everything in it) of each running
//!   Claude Code session;
//! - it is the game that game mode starts, or anything in its folder, with everything it started;
//! - it is on the built-in keep list (`keep::GROUPS`), with everything it started. That keeps a
//!   game that a launcher started.
//!
//! What is left is grouped by program file name. The user's own keep list (`gamemode.toml`) does
//! not hide a program: it stays in the list, unticked.

use std::collections::{HashMap, HashSet};

use super::keep;
use super::pc::Scan;
use crate::platform::process::{Owner, ProcessFacts};

/// The programs of Rain, wherever they are installed.
const RAIN_NAMES: [&str; 3] = ["rain", "rain-backend", "rain-hook"];

/// Why a process is never touched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    Windows,
    Shell,
    Rain,
    Game,
    Claude,
    Group(&'static str),
}

/// What Rain knows about the game that game mode starts, to keep it and its folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GameKeep {
    /// The program file of a local game.
    pub exe: Option<String>,
    /// The folder of the game.
    pub install: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Rules {
    /// `%WINDIR%`.
    pub windir: String,
    /// The process id of the backend.
    pub self_pid: u32,
    /// The folder of the backend's program file; Rain's other programs live next to it.
    pub rain_dir: Option<String>,
    /// The process ids of the running Claude Code sessions.
    pub session_pids: Vec<u32>,
    pub game: Option<GameKeep>,
    /// The names in `keep` of `gamemode.toml`, as written.
    pub keep: Vec<String>,
}

/// A process of a program, as the closing needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcId {
    pub pid: u32,
    pub parent_pid: u32,
    pub created: u64,
}

/// One program of the user, all its processes together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The lower-case file name: `discord.exe`.
    pub id: String,
    /// The program file of the biggest process, for the icon and the name.
    pub image: Option<String>,
    pub procs: Vec<ProcId>,
    pub memory_bytes: u64,
    /// Some process of it shows a window now.
    pub has_window: bool,
    /// The user's own keep list holds it.
    pub kept_by_user: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// Biggest memory first.
    pub candidates: Vec<Candidate>,
    /// Programs that are never touched, not counting Windows' own and the shell.
    pub always_kept: u32,
}

/// Lower case, `/` as `\`, no `\\?\` prefix, no trailing `\`.
pub fn norm(path: &str) -> String {
    let lower = path.trim().to_lowercase().replace('/', "\\");
    let plain = lower.strip_prefix(r"\\?\").unwrap_or(&lower);
    plain.trim_end_matches('\\').to_owned()
}

/// Whether `path` is inside `folder` (both normalised).
fn under(path: &str, folder: &str) -> bool {
    !folder.is_empty()
        && path.len() > folder.len()
        && path.starts_with(folder)
        && path.as_bytes()[folder.len()] == b'\\'
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('\\').map_or("", |(dir, _)| dir)
}

struct Tree<'a> {
    procs: HashMap<u32, &'a ProcessFacts>,
    children: HashMap<u32, Vec<u32>>,
}

impl<'a> Tree<'a> {
    fn new(scan: &'a Scan) -> Self {
        let procs: HashMap<u32, &ProcessFacts> = scan.procs.iter().map(|p| (p.pid, p)).collect();
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        for process in &scan.procs {
            if Self::parent_of(&procs, process).is_some() {
                children
                    .entry(process.parent_pid)
                    .or_default()
                    .push(process.pid);
            }
        }
        Self { procs, children }
    }

    /// The parent of a process, unless the pid was reused: a real parent started first.
    fn parent_of(
        procs: &HashMap<u32, &'a ProcessFacts>,
        process: &ProcessFacts,
    ) -> Option<&'a ProcessFacts> {
        let parent = procs.get(&process.parent_pid)?;
        let reused =
            parent.created != 0 && process.created != 0 && process.created < parent.created;
        (parent.pid != process.pid && !reused).then_some(*parent)
    }

    fn parent(&self, pid: u32) -> Option<&'a ProcessFacts> {
        Self::parent_of(&self.procs, self.procs.get(&pid)?)
    }

    /// `root` and everything it started.
    fn tree(&self, root: u32) -> Vec<u32> {
        let mut found = vec![root];
        let mut index = 0;
        while index < found.len() {
            if let Some(kids) = self.children.get(&found[index]) {
                for kid in kids {
                    if !found.contains(kid) {
                        found.push(*kid);
                    }
                }
            }
            index += 1;
        }
        found
    }
}

/// The processes that are never touched, with the reason.
fn protected(scan: &Scan, tree: &Tree<'_>, rules: &Rules) -> HashMap<u32, Why> {
    let mut kept: HashMap<u32, Why> = HashMap::new();
    let add_tree = |kept: &mut HashMap<u32, Why>, root: u32, why: Why| {
        for pid in tree.tree(root) {
            kept.entry(pid).or_insert(why);
        }
    };

    // Rain: its own folder (the shell, the backend, the hook) with what they started, and this
    // process with the chain of programs that started it (a terminal in a development run).
    // A Rain by the same names in another folder (an installed one next to a development run)
    // is Rain too.
    let rain_dir = rules.rain_dir.as_deref().map(norm).unwrap_or_default();
    for process in &scan.procs {
        let image = process.image.as_deref().map(norm).unwrap_or_default();
        if under(&image, &rain_dir) || RAIN_NAMES.contains(&keep::stem(&process.name).as_str()) {
            add_tree(&mut kept, process.pid, Why::Rain);
        }
    }
    add_tree(&mut kept, rules.self_pid, Why::Rain);
    let mut current = rules.self_pid;
    while let Some(parent) = tree.parent(current) {
        if keep::is_shell(&parent.name) {
            break;
        }
        kept.entry(parent.pid).or_insert(Why::Rain);
        current = parent.pid;
    }

    // Claude: the app, and the terminal of every running session with all that is in it.
    for process in &scan.procs {
        if keep::stem(&process.name).starts_with("claude") {
            add_tree(&mut kept, process.pid, Why::Claude);
        }
    }
    for session in &rules.session_pids {
        if !tree.procs.contains_key(session) {
            continue;
        }
        let mut top = *session;
        while let Some(parent) = tree.parent(top) {
            // Stop at the shell, and at Rain: they are not the session's terminal.
            if keep::is_shell(&parent.name) || kept.get(&parent.pid) == Some(&Why::Rain) {
                break;
            }
            top = parent.pid;
        }
        add_tree(&mut kept, top, Why::Claude);
    }

    // The game that starts, and the folder it lives in.
    if let Some(game) = &rules.game {
        let exe = game.exe.as_deref().map(norm);
        let install = game.install.as_deref().map(norm);
        for process in &scan.procs {
            let image = process.image.as_deref().map(norm).unwrap_or_default();
            let is_exe = exe
                .as_deref()
                .is_some_and(|exe| !exe.is_empty() && image == exe);
            let in_folder = install.as_deref().is_some_and(|dir| under(&image, dir));
            if is_exe || in_folder {
                add_tree(&mut kept, process.pid, Why::Game);
            }
        }
    }

    // The built-in list, with what each started: a game that a launcher started stays.
    for process in &scan.procs {
        if let Some(group) = keep::group_of(&process.name) {
            add_tree(&mut kept, process.pid, Why::Group(group));
        }
    }
    kept
}

/// The programs that may be closed, and how many are always kept.
pub fn select(scan: &Scan, rules: &Rules) -> Selection {
    let tree = Tree::new(scan);
    let kept = protected(scan, &tree, rules);
    let windir = norm(&rules.windir);
    let visible: HashSet<u32> = scan
        .wins
        .iter()
        .filter(|window| window.visible)
        .map(|window| window.pid)
        .collect();
    let user_keep: HashSet<String> = rules.keep.iter().map(|name| keep::stem(name)).collect();

    let mut always: HashSet<String> = HashSet::new();
    let mut groups: HashMap<String, Candidate> = HashMap::new();
    let mut biggest: HashMap<String, u64> = HashMap::new();
    for process in &scan.procs {
        // A process without a start time cannot be told from a later one with the same pid, so
        // it is never offered.
        if process.owner != Owner::Mine || process.pid == 0 || process.created == 0 {
            continue;
        }
        let name = process.name.to_lowercase();
        // A program whose file Rain cannot read may be a Windows program: it is kept.
        let Some(image) = process.image.as_deref().map(norm) else {
            always.insert(name);
            continue;
        };
        let image = Some(image);
        let why = kept.get(&process.pid).copied().or_else(|| {
            if image.as_deref().is_some_and(|image| under(image, &windir)) {
                Some(Why::Windows)
            } else if keep::is_shell(&process.name) {
                Some(Why::Shell)
            } else {
                None
            }
        });
        if let Some(why) = why {
            if !matches!(why, Why::Windows | Why::Shell) {
                always.insert(name);
            }
            continue;
        }
        let entry = groups.entry(name.clone()).or_insert_with(|| Candidate {
            id: name.clone(),
            image: None,
            procs: Vec::new(),
            memory_bytes: 0,
            has_window: false,
            kept_by_user: user_keep.contains(&keep::stem(&name)),
        });
        entry.procs.push(ProcId {
            pid: process.pid,
            parent_pid: process.parent_pid,
            created: process.created,
        });
        entry.memory_bytes += process.memory_bytes;
        entry.has_window |= visible.contains(&process.pid);
        let size = biggest.entry(name).or_insert(0);
        if process.image.is_some() && (entry.image.is_none() || process.memory_bytes > *size) {
            entry.image.clone_from(&process.image);
            *size = process.memory_bytes;
        }
    }

    let mut candidates: Vec<Candidate> = groups.into_values().collect();
    candidates.sort_by(|a, b| {
        b.memory_bytes
            .cmp(&a.memory_bytes)
            .then_with(|| a.id.cmp(&b.id))
    });
    // A name whose other processes are protected still counts as kept only when none is left.
    let left: HashSet<&str> = candidates.iter().map(|c| c.id.as_str()).collect();
    let always_kept = always
        .iter()
        .filter(|name| !left.contains(name.as_str()))
        .count();
    Selection {
        candidates,
        always_kept: u32::try_from(always_kept).unwrap_or(u32::MAX),
    }
}

/// The folder of the program that runs: Rain's own programs live in it.
pub fn dir_of_image(image: &str) -> String {
    dir_of(&norm(image)).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::windows::TopWindow;

    const WINDIR: &str = r"C:\Windows";
    const ME: u32 = 100;

    fn proc(pid: u32, parent: u32, name: &str, image: &str) -> ProcessFacts {
        ProcessFacts {
            pid,
            parent_pid: parent,
            name: name.to_owned(),
            image: (!image.is_empty()).then(|| image.to_owned()),
            memory_bytes: 10_000_000,
            created: u64::from(pid) * 10,
            owner: Owner::Mine,
        }
    }

    fn rules() -> Rules {
        Rules {
            windir: WINDIR.to_owned(),
            self_pid: ME,
            rain_dir: Some(r"C:\Program Files\Rain".to_owned()),
            ..Rules::default()
        }
    }

    /// Explorer (1), Rain (100, 101 WebView), and what the test adds.
    fn pc(extra: Vec<ProcessFacts>) -> Scan {
        let mut procs = vec![
            proc(1, 0, "explorer.exe", r"C:\Windows\explorer.exe"),
            proc(
                ME,
                1,
                "rain-backend.exe",
                r"C:\Program Files\Rain\rain-backend.exe",
            ),
            proc(
                101,
                ME,
                "msedgewebview2.exe",
                r"C:\Program Files (x86)\Microsoft\EdgeWebView\msedgewebview2.exe",
            ),
        ];
        procs.extend(extra);
        Scan {
            procs,
            wins: Vec::new(),
        }
    }

    fn ids(selection: &Selection) -> Vec<&str> {
        selection.candidates.iter().map(|c| c.id.as_str()).collect()
    }

    #[test]
    fn an_ordinary_program_is_closable_and_rain_and_windows_are_not() {
        let scan = pc(vec![proc(
            200,
            1,
            "Discord.exe",
            r"C:\Users\x\AppData\Local\Discord\Discord.exe",
        )]);
        let selection = select(&scan, &rules());
        assert_eq!(ids(&selection), ["discord.exe"]);
    }

    #[test]
    fn every_group_of_the_keep_list_is_kept_and_counted() {
        let names = [
            "vgc.exe",
            "EasyAntiCheat_EOS.exe",
            "steam.exe",
            "RiotClientServices.exe",
            "RadeonSoftware.exe",
            "nvcontainer.exe",
            "lghub.exe",
            "obs64.exe",
            "1Password.exe",
        ];
        let extra = names
            .iter()
            .enumerate()
            .map(|(n, name)| proc(300 + n as u32, 1, name, &format!(r"C:\Apps\{name}")))
            .collect();
        let selection = select(&pc(extra), &rules());
        assert!(selection.candidates.is_empty(), "{:?}", ids(&selection));
        // The nine, and Rain's backend and WebView (also not Windows).
        assert!(selection.always_kept >= 9);
    }

    #[test]
    fn a_program_under_the_windows_folder_is_kept_whatever_its_name() {
        let scan = pc(vec![
            proc(200, 1, "mystery.exe", r"C:\Windows\System32\mystery.exe"),
            proc(201, 1, "mystery2.exe", r"c:\windows\SysWOW64\mystery2.exe"),
            proc(202, 1, "windowsish.exe", r"C:\WindowsApps\windowsish.exe"),
        ]);
        let selection = select(&scan, &rules());
        assert_eq!(
            ids(&selection),
            ["windowsish.exe"],
            "only a sibling folder is closable"
        );
    }

    #[test]
    fn the_shell_is_kept_by_name_even_outside_the_windows_folder() {
        let scan = pc(vec![
            proc(
                200,
                1,
                "SearchHost.exe",
                r"C:\Program Files\WindowsApps\Search\SearchHost.exe",
            ),
            proc(201, 1, "TextInputHost.exe", ""),
        ]);
        assert!(select(&scan, &rules()).candidates.is_empty());
    }

    #[test]
    fn another_user_and_a_process_that_cannot_be_opened_are_not_listed() {
        let mut other = proc(200, 1, "other.exe", r"C:\Apps\other.exe");
        other.owner = Owner::Other;
        let mut locked = proc(201, 1, "elevated.exe", "");
        locked.owner = Owner::Unknown;
        let selection = select(&pc(vec![other, locked]), &rules());
        assert!(selection.candidates.is_empty());
        assert_eq!(
            selection.always_kept, 2,
            "only Rain's backend and its WebView count"
        );
    }

    #[test]
    fn a_process_whose_program_file_cannot_be_read_is_kept() {
        let scan = pc(vec![proc(250, 1, "mystery.exe", "")]);
        let selection = select(&scan, &rules());
        assert!(selection.candidates.is_empty(), "{:?}", ids(&selection));
    }

    #[test]
    fn a_process_whose_start_time_is_unknown_is_never_offered() {
        let mut unknown = proc(200, 1, "Odd.exe", r"C:\Apps\Odd.exe");
        unknown.created = 0;
        assert!(select(&pc(vec![unknown]), &rules()).candidates.is_empty());
    }

    #[test]
    fn rain_its_webview_and_the_programs_it_was_started_from_are_kept() {
        let scan = Scan {
            procs: vec![
                proc(1, 0, "explorer.exe", r"C:\Windows\explorer.exe"),
                proc(50, 1, "node.exe", r"C:\Tools\node.exe"),
                proc(
                    ME,
                    50,
                    "rain-backend.exe",
                    r"C:\Dev\target\rain-backend.exe",
                ),
                proc(101, ME, "msedgewebview2.exe", r"C:\Edge\msedgewebview2.exe"),
                proc(60, 1, "node.exe", r"C:\Tools\node.exe"),
            ],
            wins: Vec::new(),
        };
        let mut rules = rules();
        rules.rain_dir = None;
        let selection = select(&scan, &rules);
        let left: Vec<u32> = selection.candidates[0]
            .procs
            .iter()
            .map(|p| p.pid)
            .collect();
        assert_eq!(
            left,
            [60],
            "the node that started Rain stays, the other one is closable"
        );
    }

    #[test]
    fn a_rain_in_another_folder_is_kept_by_its_names_with_its_webview() {
        let scan = pc(vec![
            proc(130, 1, "Rain.exe", r"D:\Installed\Rain.exe"),
            proc(
                131,
                130,
                "msedgewebview2.exe",
                r"C:\Edge\msedgewebview2.exe",
            ),
            proc(
                132,
                130,
                "rain-backend.exe",
                r"D:\Installed\rain-backend.exe",
            ),
        ]);
        assert!(select(&scan, &rules()).candidates.is_empty());
    }

    #[test]
    fn programs_in_rains_own_folder_and_what_they_started_are_kept() {
        let scan = pc(vec![
            proc(120, 1, "Rain.exe", r"C:\Program Files\Rain\Rain.exe"),
            proc(
                121,
                120,
                "msedgewebview2.exe",
                r"C:\Edge\msedgewebview2.exe",
            ),
        ]);
        assert!(select(&scan, &rules()).candidates.is_empty());
    }

    #[test]
    fn the_claude_app_and_every_tree_with_a_running_session_are_kept() {
        let scan = pc(vec![
            proc(
                200,
                1,
                "claude.exe",
                r"C:\Users\x\AppData\Local\AnthropicClaude\claude.exe",
            ),
            proc(201, 200, "node.exe", r"C:\Tools\node.exe"),
            // A terminal with a session: terminal, shell, node, and a tool node starts.
            proc(
                300,
                1,
                "WindowsTerminal.exe",
                r"C:\Apps\WindowsTerminal.exe",
            ),
            proc(301, 300, "pwsh.exe", r"C:\Apps\pwsh.exe"),
            proc(302, 301, "node.exe", r"C:\Tools\node.exe"),
            proc(303, 302, "rg.exe", r"C:\Tools\rg.exe"),
            // Another terminal without one.
            proc(
                400,
                1,
                "WindowsTerminal.exe",
                r"C:\Apps\WindowsTerminal.exe",
            ),
            proc(401, 400, "pwsh.exe", r"C:\Apps\pwsh.exe"),
        ]);
        let mut with_session = rules();
        with_session.session_pids = vec![302];
        let selection = select(&scan, &with_session);
        let mut closable: Vec<(String, Vec<u32>)> = selection
            .candidates
            .iter()
            .map(|c| (c.id.clone(), c.procs.iter().map(|p| p.pid).collect()))
            .collect();
        closable.sort();
        assert_eq!(
            closable,
            [
                ("pwsh.exe".to_owned(), vec![401]),
                ("windowsterminal.exe".to_owned(), vec![400])
            ]
        );

        // Without the session id (the monitor is off), the Claude app is still kept by name.
        let selection = select(&scan, &rules());
        assert!(!ids(&selection).contains(&"claude.exe"));
        assert!(
            selection
                .candidates
                .iter()
                .all(|c| c.procs.iter().all(|p| p.pid != 201)),
            "what the Claude app started is kept"
        );
    }

    #[test]
    fn the_game_its_folder_and_its_tree_are_kept() {
        let scan = pc(vec![
            proc(200, 1, "Game.exe", r"D:\Games\Quest\Game.exe"),
            proc(201, 200, "helper.exe", r"C:\Temp\helper.exe"),
            proc(202, 1, "tool.exe", r"D:\Games\Quest\bin\tool.exe"),
            proc(203, 1, "Game2.exe", r"D:\Games\Quest2\Game2.exe"),
        ]);
        let mut rules = rules();
        rules.game = Some(GameKeep {
            exe: Some(r"d:/games/quest/game.exe".to_owned()),
            install: Some(r"D:\Games\Quest\".to_owned()),
        });
        let selection = select(&scan, &rules);
        assert_eq!(
            ids(&selection),
            ["game2.exe"],
            "a sibling folder is not the game"
        );
    }

    #[test]
    fn what_a_launcher_started_is_kept_but_what_explorer_started_is_not() {
        let scan = pc(vec![
            proc(200, 1, "steam.exe", r"C:\Steam\steam.exe"),
            proc(201, 200, "Game.exe", r"C:\Steam\Game.exe"),
            proc(202, 1, "Notes.exe", r"C:\Apps\Notes.exe"),
        ]);
        assert_eq!(ids(&select(&scan, &rules())), ["notes.exe"]);
    }

    #[test]
    fn a_child_that_started_before_its_parent_is_a_reused_pid_and_is_not_kept_with_it() {
        let mut stale = proc(201, 200, "Old.exe", r"C:\Apps\Old.exe");
        stale.created = 5; // before the launcher started (2000)
        let scan = pc(vec![
            proc(200, 1, "steam.exe", r"C:\Steam\steam.exe"),
            stale,
        ]);
        assert_eq!(ids(&select(&scan, &rules())), ["old.exe"]);
    }

    #[test]
    fn the_users_keep_file_marks_a_program_but_still_lists_it() {
        let scan = pc(vec![
            proc(200, 1, "Discord.exe", r"C:\Apps\Discord.exe"),
            proc(201, 1, "Slack.exe", r"C:\Apps\Slack.exe"),
        ]);
        let mut rules = rules();
        rules.keep = vec!["DISCORD.EXE".to_owned(), "other".to_owned()];
        let selection = select(&scan, &rules);
        let discord = selection
            .candidates
            .iter()
            .find(|c| c.id == "discord.exe")
            .unwrap();
        let slack = selection
            .candidates
            .iter()
            .find(|c| c.id == "slack.exe")
            .unwrap();
        assert!(discord.kept_by_user);
        assert!(!slack.kept_by_user);
    }

    #[test]
    fn a_program_is_one_row_with_its_processes_memory_and_window() {
        let mut a = proc(200, 1, "chrome.exe", r"C:\Chrome\chrome.exe");
        a.memory_bytes = 300;
        let mut b = proc(201, 200, "chrome.exe", r"C:\Chrome\chrome.exe");
        b.memory_bytes = 500;
        let mut small = proc(202, 1, "Small.exe", r"C:\Apps\Small.exe");
        small.memory_bytes = 1;
        let mut scan = pc(vec![a, b, small]);
        scan.wins = vec![
            TopWindow {
                hwnd: 7,
                pid: 201,
                visible: true,
            },
            TopWindow {
                hwnd: 8,
                pid: 202,
                visible: false,
            },
        ];
        let selection = select(&scan, &rules());
        assert_eq!(
            ids(&selection),
            ["chrome.exe", "small.exe"],
            "biggest first"
        );
        let chrome = &selection.candidates[0];
        assert_eq!(chrome.procs.len(), 2);
        assert_eq!(chrome.memory_bytes, 800);
        assert!(chrome.has_window);
        assert!(
            !selection.candidates[1].has_window,
            "a hidden window is no window"
        );
    }

    #[test]
    fn paths_are_compared_by_folder_not_by_text_prefix() {
        assert!(under(r"c:\windows\x.exe", r"c:\windows"));
        assert!(!under(r"c:\windowsapps\x.exe", r"c:\windows"));
        assert!(!under(r"c:\windows", r"c:\windows"));
        assert!(!under(r"c:\a", ""));
        assert_eq!(norm(r"\\?\C:/Games/Quest\"), r"c:\games\quest");
        assert_eq!(
            dir_of_image(r"C:\Program Files\Rain\rain-backend.exe"),
            r"c:\program files\rain"
        );
    }
}
