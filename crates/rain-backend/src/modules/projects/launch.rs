//! Opening a project: the command lines and the plan. Section 02 6.7.
//!
//! This file plans; it starts nothing. The launch engine of the launcher module runs the steps
//! (`PlannedStep`), so everything here is a pure function of its input and is tested with golden
//! strings. The default plan is the editor and ONE Windows Terminal tab in the project folder in
//! which Claude Code runs. The tab is a `cmd.exe` shell that runs `claude`, so when Claude Code
//! exits the tab stays open as a normal shell in the folder (decisions.md, ruling of
//! 2026-11-22).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rain_types::{EnvKind, IconKind, IconRef, LaunchMode};

use super::env::{Builtins, StepSpec, builtin_step};
use super::tools::Lookup;
use crate::core::error::ApiError;
use crate::core::launch_contract::{ErrorText, OpenError, PlannedStep, ShellFallback, StepAction};

// ---- command lines ----

/// One argument as `CommandLineToArgvW` reads it back: wrapped in quotes when it holds a space,
/// a tab or a quote (or is empty), a quote becomes `\"`, and backslashes before a quote or before
/// the closing quote are doubled.
pub fn quote_arg(arg: &str) -> String {
    let needs_quotes = arg.is_empty() || arg.contains([' ', '\t', '"']);
    if !needs_quotes {
        return arg.to_owned();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for letter in arg.chars() {
        match letter {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            other => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(other);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// Windows Terminal reads `;` as the separator of its own commands, so a `;` that belongs to a
/// title or a command is written `\;`.
fn escape_semicolons(text: &str) -> String {
    text.replace(';', "\\;")
}

/// The title of a tab: the project name without control characters, `"` and `;`.
pub fn tab_title(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_control() && *c != '"' && *c != ';')
        .collect()
}

/// What a Windows Terminal tab runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabRun {
    /// The default shell of the profile.
    Shell,
    /// `cmd.exe /d /k "<command>"`: runs the command and stays open as a shell.
    Command(String),
    /// Claude Code inside a `cmd.exe` shell: `cmd.exe /d /k ""<claude>" <args>"`.
    Claude { exe: PathBuf, args: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub title: String,
    pub folder: String,
    pub run: TabRun,
}

/// Where Windows Terminal puts the tabs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WtOptions {
    /// `-w new` (always a new window) instead of `-w 0` (the last used window).
    pub new_window: bool,
    /// A Windows Terminal profile name, empty for the default profile.
    pub profile: String,
}

/// The arguments of one `wt.exe` call (without the program) that open `tabs`, one `new-tab` part
/// for each, separated by `;`. One call for all tabs, so that they land in the same window even
/// when no Terminal window exists yet.
pub fn wt_line(options: &WtOptions, tabs: &[Tab]) -> String {
    let mut line = String::from(if options.new_window { "-w new" } else { "-w 0" });
    for (index, tab) in tabs.iter().enumerate() {
        line.push_str(if index == 0 { " new-tab" } else { " ; new-tab" });
        if !options.profile.trim().is_empty() {
            line.push_str(" -p ");
            line.push_str(&quote_arg(&escape_semicolons(options.profile.trim())));
        }
        line.push_str(" -d ");
        line.push_str(&quote_arg(tab.folder.trim_end_matches('\\')));
        if !tab.title.is_empty() {
            line.push_str(" --title ");
            line.push_str(&quote_arg(&tab.title));
        }
        match &tab.run {
            TabRun::Shell => {}
            TabRun::Command(command) => {
                line.push_str(" cmd.exe /d /k ");
                line.push_str(&quote_arg(&escape_semicolons(command)));
            }
            TabRun::Claude { exe, args } => {
                // The whole text after `/k` is wrapped in one more pair of quotes: `cmd` strips
                // the outer pair and runs what is inside, whatever it holds.
                let mut inner = format!("\"{}\"", exe.display());
                if let Some(args) = args.as_ref().filter(|a| !a.trim().is_empty()) {
                    inner.push(' ');
                    inner.push_str(&escape_semicolons(args.trim()));
                }
                line.push_str(" cmd.exe /d /k \"");
                line.push_str(&inner);
                line.push('"');
            }
        }
    }
    line
}

/// The arguments of an editor start: the folder, then the extra arguments of the step.
pub fn editor_args(folder: &str, extra: Option<&str>) -> String {
    let mut line = quote_arg(folder.trim_end_matches('\\'));
    if let Some(extra) = extra.map(str::trim).filter(|e| !e.is_empty()) {
        line.push(' ');
        line.push_str(extra);
    }
    line
}

// ---- which steps a mode runs ----

/// The steps that `mode` runs from the project's list. `Err` when there is none to run.
///
/// `terminal` and `claude` use the step of that kind in the project's list, or else the built-in
/// one, so that "Terminal here" works with the default list, which has no terminal step.
pub fn select_steps(
    mode: Option<LaunchMode>,
    specs: &[StepSpec],
    builtins: &Builtins,
) -> Result<Vec<StepSpec>, ApiError> {
    let of_kind = |kind: EnvKind, builtin: &str| {
        specs
            .iter()
            .find(|spec| spec.kind == kind)
            .cloned()
            .or_else(|| builtin_step(builtin, builtins))
            .into_iter()
            .collect::<Vec<_>>()
    };
    match mode.unwrap_or(LaunchMode::Full) {
        LaunchMode::Full if specs.is_empty() => Err(no_steps("This project has no steps")),
        LaunchMode::Full => Ok(specs.to_vec()),
        LaunchMode::EditorOnly => {
            let editors: Vec<StepSpec> = specs
                .iter()
                .filter(|spec| spec.kind == EnvKind::Editor)
                .cloned()
                .collect();
            if editors.is_empty() {
                Err(no_steps("This project has no editor step"))
            } else {
                Ok(editors)
            }
        }
        LaunchMode::Terminal => Ok(of_kind(EnvKind::Terminal, "terminal")),
        LaunchMode::Claude => Ok(of_kind(EnvKind::Claude, "claude")),
    }
}

fn no_steps(message: &str) -> ApiError {
    ApiError::invalid(message.to_owned()).with_reason("noSteps")
}

// ---- the plan ----

/// Everything the plan of one project needs. All of it is known without starting anything.
pub struct PlanInput<'a> {
    pub name: &'a str,
    /// `norm_path` of the folder.
    pub folder: &'a str,
    pub missing: bool,
    /// An editor window of this project is open now.
    pub open: bool,
    pub mode: LaunchMode,
    /// Already chosen by `select_steps`.
    pub steps: Vec<StepSpec>,
    pub wt: WtOptions,
    pub editor: Lookup,
    pub terminal: Lookup,
    /// Where Claude Code is.
    pub coding_agent: Lookup,
    pub builtins: &'a Builtins,
}

fn fixed_text(text: &'static str) -> ErrorText {
    Arc::new(move |_: &OpenError| text.to_owned())
}

fn text_of(text: String) -> ErrorText {
    Arc::new(move |_: &OpenError| text.clone())
}

fn planned(label: String, icon: IconRef, action: StepAction) -> PlannedStep {
    PlannedStep {
        label,
        icon,
        action,
        delay_ms: 0,
        preset_error: None,
        stops_run: false,
        on_spawned: None,
        shell_fallback: None,
        error_text: None,
    }
}

fn failing(label: String, icon: IconRef, message: String) -> PlannedStep {
    PlannedStep {
        preset_error: Some(message),
        ..planned(label, icon, StepAction::Nothing)
    }
}

/// A program of a `Lookup` result, or the text of the step that fails instead.
fn program_of(found: &Lookup, not_found: &str) -> Result<PathBuf, String> {
    match found {
        Lookup::At(path) => Ok(path.clone()),
        Lookup::NotFound => Err(not_found.to_owned()),
        Lookup::ConfiguredMissing(path) => {
            Err(format!("Configured program not found: {}", path.display()))
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || "the program".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Turns the chosen steps into the steps of a launch run.
///
/// A project that is open (an editor window exists) in `full` mode is not opened twice: the
/// editor step runs again (VS Code brings the window of an open folder forward by itself) and
/// the terminal and Claude steps are already satisfied. A folder that is gone is one failing
/// step that ends the run.
pub fn plan_steps(input: &PlanInput<'_>) -> Vec<PlannedStep> {
    if input.missing {
        let mut step = failing(
            "Project folder".to_owned(),
            IconRef {
                kind: IconKind::Lucide,
                value: "folder-x".to_owned(),
                tint: None,
                fallback: None,
            },
            format!("Folder not found: {}", input.folder),
        );
        step.stops_run = true;
        return vec![step];
    }
    let focus = input.mode == LaunchMode::Full && input.open;
    let mut steps = Vec::new();
    let mut index = 0;
    while index < input.steps.len() {
        let spec = &input.steps[index];
        let next = input.steps.get(index + 1);
        // A terminal step directly followed by a Claude step is one `wt.exe` call with two tabs.
        let pair = matches!(
            (spec.kind, next),
            (EnvKind::Terminal, Some(n)) if n.kind == EnvKind::Claude && n.delay_ms == 0
        );
        let step = if pair {
            index += 1;
            plan_pair(input, spec, &input.steps[index], focus)
        } else {
            plan_one(input, spec, focus)
        };
        steps.push(step);
        index += 1;
    }
    steps
}

fn plan_pair(
    input: &PlanInput<'_>,
    terminal: &StepSpec,
    claude: &StepSpec,
    focus: bool,
) -> PlannedStep {
    let label = format!("{} + {}", terminal.label, claude.label);
    let icon = claude.icon.clone();
    let make = |action: StepAction| PlannedStep {
        delay_ms: terminal.delay_ms,
        ..planned(label.clone(), icon.clone(), action)
    };
    if focus {
        return make(StepAction::Nothing);
    }
    let wt = match program_of(&input.terminal, "Windows Terminal was not found") {
        Ok(wt) => wt,
        Err(message) => {
            return PlannedStep {
                preset_error: Some(message),
                ..make(StepAction::Nothing)
            };
        }
    };
    let claude_exe = match program_of(&input.coding_agent, CLAUDE_NOT_FOUND) {
        Ok(exe) => exe,
        Err(message) => {
            return PlannedStep {
                preset_error: Some(message),
                ..make(StepAction::Nothing)
            };
        }
    };
    let tabs = [
        Tab {
            title: tab_title(input.name),
            folder: input.folder.to_owned(),
            run: terminal
                .command
                .clone()
                .map_or(TabRun::Shell, TabRun::Command),
        },
        Tab {
            title: tab_title(input.name),
            folder: input.folder.to_owned(),
            run: TabRun::Claude {
                exe: claude_exe,
                args: claude.args.clone(),
            },
        },
    ];
    wt_step(input, wt, &wt_line(&input.wt, &tabs), make)
}

const CLAUDE_NOT_FOUND: &str = "Claude Code was not found. Set its path in projects.toml";

/// The step that starts `wt.exe` with `line`, with the `ShellExecute` fallback of the same text.
fn wt_step(
    input: &PlanInput<'_>,
    wt: PathBuf,
    line: &str,
    make: impl Fn(StepAction) -> PlannedStep,
) -> PlannedStep {
    let mut step = make(StepAction::Spawn {
        exe: wt.clone(),
        args_raw: line.to_owned(),
        cwd: PathBuf::from(input.folder),
    });
    step.shell_fallback = Some(ShellFallback {
        file: wt.to_string_lossy().into_owned(),
        params: Some(line.to_owned()),
        dir: Some(input.folder.to_owned()),
    });
    step.error_text = Some(fixed_text("Could not open Windows Terminal"));
    step
}

fn plan_one(input: &PlanInput<'_>, spec: &StepSpec, focus: bool) -> PlannedStep {
    let make = |action: StepAction| PlannedStep {
        delay_ms: spec.delay_ms,
        ..planned(spec.label.clone(), spec.icon.clone(), action)
    };
    let fail = |message: String| PlannedStep {
        preset_error: Some(message),
        ..make(StepAction::Nothing)
    };
    match spec.kind {
        EnvKind::Editor | EnvKind::App => {
            let program = match (&spec.app, spec.kind) {
                (Some(app), _) => Ok(PathBuf::from(app)),
                (None, EnvKind::Editor) => {
                    let not_found = format!(
                        "{} was not found. Set its path in projects.toml",
                        input.builtins.editor_label
                    );
                    program_of(&input.editor, &not_found)
                }
                (None, _) => Err("This step has no program".to_owned()),
            };
            let exe = match program {
                Ok(exe) => exe,
                Err(message) => return fail(message),
            };
            let args_raw = if spec.kind == EnvKind::Editor {
                editor_args(input.folder, spec.args.as_deref())
            } else {
                spec.args.clone().unwrap_or_default()
            };
            let mut step = make(StepAction::Spawn {
                exe: exe.clone(),
                args_raw,
                cwd: PathBuf::from(input.folder),
            });
            step.error_text = Some(text_of(format!("Could not start {}", file_name(&exe))));
            step
        }
        EnvKind::Terminal | EnvKind::Claude => {
            if focus {
                return make(StepAction::Nothing);
            }
            let wt = match program_of(&input.terminal, "Windows Terminal was not found") {
                Ok(wt) => wt,
                Err(message) => return fail(message),
            };
            let run = if spec.kind == EnvKind::Claude {
                match program_of(&input.coding_agent, CLAUDE_NOT_FOUND) {
                    Ok(exe) => TabRun::Claude {
                        exe,
                        args: spec.args.clone(),
                    },
                    Err(message) => return fail(message),
                }
            } else {
                spec.command.clone().map_or(TabRun::Shell, TabRun::Command)
            };
            let tab = Tab {
                title: tab_title(input.name),
                folder: input.folder.to_owned(),
                run,
            };
            wt_step(input, wt, &wt_line(&input.wt, &[tab]), make)
        }
        EnvKind::Browser => {
            let mut step = make(StepAction::ShellExec {
                file: spec.url.clone().unwrap_or_default(),
                params: None,
                dir: None,
            });
            step.error_text = Some(fixed_text("Could not open the browser"));
            step
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::projects::env::{ProjectsFile, analyze};

    fn builtins() -> Builtins {
        Builtins {
            editor_label: "VS Code".to_owned(),
        }
    }

    #[test]
    fn quote_arg_follows_the_rules_of_command_line_to_argv() {
        let table = [
            ("a", "a"),
            ("a b", "\"a b\""),
            ("a\"b", "\"a\\\"b\""),
            ("", "\"\""),
            ("C:\\a b\\", "\"C:\\a b\\\\\""),
            ("C:\\ab\\", "C:\\ab\\"),
            ("a;b", "a;b"),
            ("x \\\" y", "\"x \\\\\\\" y\""),
        ];
        for (input, expected) in table {
            assert_eq!(quote_arg(input), expected, "{input}");
        }
    }

    fn tab(run: TabRun) -> Tab {
        Tab {
            title: "web-shop".into(),
            folder: "D:\\Work\\web-shop".into(),
            run,
        }
    }

    #[test]
    fn a_plain_tab() {
        assert_eq!(
            wt_line(&WtOptions::default(), &[tab(TabRun::Shell)]),
            "-w 0 new-tab -d D:\\Work\\web-shop --title web-shop"
        );
    }

    #[test]
    fn a_tab_with_a_command_stays_open_as_a_shell() {
        assert_eq!(
            wt_line(
                &WtOptions::default(),
                &[tab(TabRun::Command("npm run dev".into()))]
            ),
            "-w 0 new-tab -d D:\\Work\\web-shop --title web-shop cmd.exe /d /k \"npm run dev\""
        );
    }

    #[test]
    fn claude_runs_inside_a_cmd_shell_in_one_tab() {
        let run = TabRun::Claude {
            exe: PathBuf::from("C:\\Tools\\claude.exe"),
            args: None,
        };
        assert_eq!(
            wt_line(&WtOptions::default(), &[tab(run)]),
            "-w 0 new-tab -d D:\\Work\\web-shop --title web-shop cmd.exe /d /k \"\"C:\\Tools\\claude.exe\"\""
        );
        let with_args = TabRun::Claude {
            exe: PathBuf::from("C:\\My Tools\\claude.exe"),
            args: Some("--model sonnet".into()),
        };
        assert_eq!(
            wt_line(&WtOptions::default(), &[tab(with_args)]),
            "-w 0 new-tab -d D:\\Work\\web-shop --title web-shop cmd.exe /d /k \"\"C:\\My Tools\\claude.exe\" --model sonnet\""
        );
    }

    #[test]
    fn two_tabs_share_one_call_with_a_semicolon_between() {
        let line = wt_line(
            &WtOptions::default(),
            &[tab(TabRun::Shell), tab(TabRun::Command("echo hi".into()))],
        );
        assert_eq!(
            line,
            "-w 0 new-tab -d D:\\Work\\web-shop --title web-shop ; new-tab -d D:\\Work\\web-shop --title web-shop cmd.exe /d /k \"echo hi\""
        );
    }

    #[test]
    fn a_new_window_and_a_profile() {
        let options = WtOptions {
            new_window: true,
            profile: "Command Prompt".into(),
        };
        assert_eq!(
            wt_line(&options, &[tab(TabRun::Shell)]),
            "-w new new-tab -p \"Command Prompt\" -d D:\\Work\\web-shop --title web-shop"
        );
    }

    #[test]
    fn semicolons_in_a_command_are_escaped_and_titles_lose_them() {
        assert_eq!(tab_title("we;b \"shop\"\u{7}"), "web shop");
        let line = wt_line(
            &WtOptions::default(),
            &[tab(TabRun::Command("echo hello; echo there".into()))],
        );
        assert!(
            line.ends_with("cmd.exe /d /k \"echo hello\\; echo there\""),
            "{line}"
        );
    }

    #[test]
    fn a_folder_with_a_trailing_backslash_is_passed_without_it() {
        let t = Tab {
            folder: "D:\\Work\\my shop\\".into(),
            ..tab(TabRun::Shell)
        };
        assert!(wt_line(&WtOptions::default(), &[t]).contains("-d \"D:\\Work\\my shop\""));
        assert_eq!(
            editor_args("D:\\Work\\my shop", Some("--new-window")),
            "\"D:\\Work\\my shop\" --new-window"
        );
        assert_eq!(editor_args("D:\\Work\\a", None), "D:\\Work\\a");
    }

    fn specs(text: &str) -> Vec<StepSpec> {
        let file: ProjectsFile = toml::from_str(text).unwrap();
        analyze(&file, &builtins()).resolve("k", "i", &builtins()).0
    }

    #[test]
    fn modes_choose_their_steps() {
        let defaults = specs("");
        let full = select_steps(None, &defaults, &builtins()).unwrap();
        assert_eq!(full.len(), 2);
        let editor = select_steps(Some(LaunchMode::EditorOnly), &defaults, &builtins()).unwrap();
        assert_eq!(editor.len(), 1);
        assert_eq!(editor[0].kind, EnvKind::Editor);
        // The default list has no terminal step: the built-in one is used.
        let terminal = select_steps(Some(LaunchMode::Terminal), &defaults, &builtins()).unwrap();
        assert_eq!(terminal.len(), 1);
        assert_eq!(terminal[0].kind, EnvKind::Terminal);
        let claude = select_steps(Some(LaunchMode::Claude), &defaults, &builtins()).unwrap();
        assert_eq!(claude[0].kind, EnvKind::Claude);
    }

    #[test]
    fn a_list_without_an_editor_cannot_be_opened_editor_only_and_an_empty_list_has_no_steps() {
        let terminal_only = specs("[defaults]\nsteps = [\"terminal\"]");
        let error =
            select_steps(Some(LaunchMode::EditorOnly), &terminal_only, &builtins()).unwrap_err();
        assert_eq!(error.body.reason.as_deref(), Some("noSteps"));
        assert_eq!(error.body.message, "This project has no editor step");
        let empty = specs("[defaults]\nsteps = []");
        let error = select_steps(None, &empty, &builtins()).unwrap_err();
        assert_eq!(error.body.message, "This project has no steps");
        // The built-in steps still serve the two "here" modes.
        assert!(select_steps(Some(LaunchMode::Terminal), &empty, &builtins()).is_ok());
    }

    // ---- plans ----

    fn found(path: &str) -> Lookup {
        Lookup::At(PathBuf::from(path))
    }

    fn input<'a>(steps: Vec<StepSpec>, builtins: &'a Builtins) -> PlanInput<'a> {
        PlanInput {
            name: "web-shop",
            folder: "D:\\Work\\web-shop",
            missing: false,
            open: false,
            mode: LaunchMode::Full,
            steps,
            wt: WtOptions::default(),
            editor: found("C:\\Code\\Code.exe"),
            terminal: found("C:\\Apps\\wt.exe"),
            coding_agent: found("C:\\Tools\\claude.exe"),
            builtins,
        }
    }

    fn spawn_of(step: &PlannedStep) -> (&Path, &str, &Path) {
        match &step.action {
            StepAction::Spawn { exe, args_raw, cwd } => (exe, args_raw, cwd),
            _ => panic!("not a spawn: {}", step.label),
        }
    }

    #[test]
    fn the_default_plan_is_the_editor_and_one_terminal_call() {
        let b = builtins();
        let steps = plan_steps(&input(specs(""), &b));
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].label, "VS Code");
        let (exe, args, cwd) = spawn_of(&steps[0]);
        assert_eq!(exe, Path::new("C:\\Code\\Code.exe"));
        assert_eq!(args, "D:\\Work\\web-shop");
        assert_eq!(cwd, Path::new("D:\\Work\\web-shop"));

        assert_eq!(steps[1].label, "Claude Code");
        let (exe, args, _) = spawn_of(&steps[1]);
        assert_eq!(exe, Path::new("C:\\Apps\\wt.exe"));
        assert_eq!(args.matches("new-tab").count(), 1);
        assert!(args.contains("cmd.exe /d /k \"\"C:\\Tools\\claude.exe\"\""));
        let fallback = steps[1].shell_fallback.as_ref().unwrap();
        assert_eq!(fallback.params.as_deref(), Some(args));
        assert_eq!(fallback.dir.as_deref(), Some("D:\\Work\\web-shop"));
    }

    #[test]
    fn a_terminal_step_followed_by_claude_is_one_call_with_two_tabs() {
        let b = builtins();
        let list = specs("[defaults]\nsteps = [\"editor\", \"terminal\", \"claude\"]");
        let steps = plan_steps(&input(list, &b));
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[1].label, "Terminal + Claude Code");
        let (_, args, _) = spawn_of(&steps[1]);
        assert_eq!(args.matches("new-tab").count(), 2);
        assert_eq!(args.matches(" ; new-tab").count(), 1);
    }

    #[test]
    fn a_browser_step_waits_and_hands_the_url_to_the_shell() {
        let b = builtins();
        let list = specs(
            "[defaults]\nsteps = [\"preview\"]\n[[step]]\nname = \"preview\"\nkind = \"browser\"\nurl = \"http://localhost:5173\"\ndelay_ms = 3000\n",
        );
        let steps = plan_steps(&input(list, &b));
        assert_eq!(steps[0].delay_ms, 3000);
        match &steps[0].action {
            StepAction::ShellExec { file, .. } => assert_eq!(file, "http://localhost:5173"),
            _ => panic!("expected a shell start"),
        }
    }

    #[test]
    fn an_open_project_runs_the_editor_again_and_starts_nothing_else() {
        let b = builtins();
        let mut plan_input = input(specs(""), &b);
        plan_input.open = true;
        let steps = plan_steps(&plan_input);
        assert!(matches!(steps[0].action, StepAction::Spawn { .. }));
        assert!(matches!(steps[1].action, StepAction::Nothing));
        assert!(steps[1].preset_error.is_none());

        // The other modes always start their steps.
        plan_input.mode = LaunchMode::Terminal;
        plan_input.steps = select_steps(Some(LaunchMode::Terminal), &specs(""), &b).unwrap();
        let steps = plan_steps(&plan_input);
        assert!(matches!(steps[0].action, StepAction::Spawn { .. }));
    }

    #[test]
    fn a_missing_folder_is_one_failing_step_that_ends_the_run() {
        let b = builtins();
        let mut plan_input = input(specs(""), &b);
        plan_input.missing = true;
        let steps = plan_steps(&plan_input);
        assert_eq!(steps.len(), 1);
        assert_eq!(
            steps[0].preset_error.as_deref(),
            Some("Folder not found: D:\\Work\\web-shop")
        );
        assert!(steps[0].stops_run);
    }

    #[test]
    fn a_missing_program_fails_only_its_own_step() {
        let b = builtins();
        let mut plan_input = input(specs(""), &b);
        plan_input.editor = Lookup::NotFound;
        plan_input.coding_agent = Lookup::NotFound;
        let steps = plan_steps(&plan_input);
        assert_eq!(
            steps[0].preset_error.as_deref(),
            Some("VS Code was not found. Set its path in projects.toml")
        );
        assert_eq!(
            steps[1].preset_error.as_deref(),
            Some("Claude Code was not found. Set its path in projects.toml")
        );
        assert!(!steps[0].stops_run && !steps[1].stops_run);

        plan_input.editor = Lookup::ConfiguredMissing(PathBuf::from("D:\\gone\\Code.exe"));
        plan_input.terminal = Lookup::NotFound;
        let steps = plan_steps(&plan_input);
        assert_eq!(
            steps[0].preset_error.as_deref(),
            Some("Configured program not found: D:\\gone\\Code.exe")
        );
        assert_eq!(
            steps[1].preset_error.as_deref(),
            Some("Windows Terminal was not found")
        );
    }

    #[test]
    fn step_error_texts_ignore_the_error_code_and_say_what_failed() {
        let b = builtins();
        let steps = plan_steps(&input(specs(""), &b));
        let error = OpenError {
            ui_text: "x",
            code: Some(5),
            log_reason: String::new(),
        };
        assert_eq!(
            (steps[0].error_text.as_ref().unwrap())(&error),
            "Could not start Code.exe"
        );
        assert_eq!(
            (steps[1].error_text.as_ref().unwrap())(&error),
            "Could not open Windows Terminal"
        );
    }
}
