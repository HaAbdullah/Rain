//! The command line of a run: what is started, with which arguments, input and environment.
//! Section 03 6.11.8. Pure: nothing here starts a program (`runner` does).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::entry::{CommandLine, Shell};
use super::model::{ClaudeJob, PermissionMode};

/// Everything the runner needs to start one program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
    pub program: OsString,
    pub args: Vec<OsString>,
    /// Text that is passed to the program exactly as written, without quoting (for `cmd /c`).
    pub raw_args: Option<OsString>,
    pub cwd: PathBuf,
    pub env_set: Vec<(String, String)>,
    /// Environment variables whose name starts with one of these are not passed on.
    pub env_remove_prefixes: Vec<&'static str>,
    /// Bytes for the standard input of the program (then closed). `None`: no input.
    pub stdin: Option<Vec<u8>>,
}

/// The two variables that tell a run which job it belongs to (section 03 6.11.8).
fn run_env(run_id: &str, job_id: &str, extra: &BTreeMap<String, String>) -> Vec<(String, String)> {
    let mut env = vec![
        ("RAIN_RUN_ID".to_owned(), run_id.to_owned()),
        ("RAIN_JOB_ID".to_owned(), job_id.to_owned()),
    ];
    env.extend(extra.iter().map(|(k, v)| (k.clone(), v.clone())));
    env
}

/// A command job or an idle job: the program as the file wrote it.
pub fn command_spec(
    line: &CommandLine,
    cwd: &Path,
    run_id: &str,
    job_id: &str,
    extra_env: &BTreeMap<String, String>,
) -> SpawnSpec {
    let (program, args, raw_args): (OsString, Vec<OsString>, Option<OsString>) = match line.shell {
        Shell::None => (
            line.command.clone().into(),
            line.args.iter().map(OsString::from).collect(),
            None,
        ),
        // `cmd /d /s /c "<command>"`: `/s` makes cmd keep the text inside the outer quotes as
        // it is, so the script line reaches cmd unchanged. It must not be quoted again.
        Shell::Cmd => (
            "cmd.exe".into(),
            Vec::new(),
            Some(format!("/d /s /c \"{}\"", line.command).into()),
        ),
        Shell::Powershell => (
            "powershell.exe".into(),
            ["-NoProfile", "-NonInteractive", "-Command", &line.command]
                .map(OsString::from)
                .to_vec(),
            None,
        ),
    };
    SpawnSpec {
        program,
        args,
        raw_args,
        cwd: cwd.to_owned(),
        env_set: run_env(run_id, job_id, extra_env),
        env_remove_prefixes: Vec::new(),
        stdin: None,
    }
}

/// What the headless Claude is told. The skill is asked for in words, because a headless run may
/// not type a slash command (the model's own Skill tool runs it).
pub fn claude_prompt(skill: Option<&str>, prompt: Option<&str>) -> String {
    let mut text = String::new();
    if let Some(skill) = skill {
        text.push_str(&format!(
            "Run the skill \"{skill}\" now and finish its whole workflow. Nobody is available to \
             answer questions, so make reasonable choices and say at the end what you did."
        ));
    }
    if let Some(prompt) = prompt {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(prompt);
    }
    text
}

/// A headless Claude run (`claude -p`, nobody can answer a permission prompt).
pub fn claude_spec(
    job: &ClaudeJob,
    program: PathBuf,
    job_name: &str,
    cwd: &Path,
    run_id: &str,
    job_id: &str,
    extra_env: &BTreeMap<String, String>,
) -> SpawnSpec {
    let mut args: Vec<OsString> = ["-p", "--output-format", "stream-json", "--verbose"]
        .map(OsString::from)
        .to_vec();
    let mut flag = |name: &str, value: &str| {
        args.push(name.into());
        args.push(value.into());
    };
    if let Some(model) = &job.model {
        flag("--model", model);
    }
    flag("--permission-mode", job.permission_mode.as_flag());
    if job.permission_mode == PermissionMode::BypassPermissions {
        // Only reachable with `dangerously_skip_permissions = true`: the model checks it.
        args.push("--dangerously-skip-permissions".into());
    }
    if !job.allowed_tools.is_empty() {
        args.push("--allowedTools".into());
        args.push(job.allowed_tools.join(",").into());
    }
    if !job.disallowed_tools.is_empty() {
        args.push("--disallowedTools".into());
        args.push(job.disallowed_tools.join(",").into());
    }
    args.extend(["--permission-prompts".into(), "none".into()]);
    if let Some(turns) = job.max_turns {
        args.extend(["--max-turns".into(), turns.to_string().into()]);
    }
    args.extend(["--name".into(), job_name.into()]);
    args.extend(job.extra_args.iter().map(OsString::from));

    SpawnSpec {
        program: program.into(),
        args,
        raw_args: None,
        cwd: cwd.to_owned(),
        env_set: run_env(run_id, job_id, extra_env),
        // The run is the owner's own Claude Code installation, but it must not inherit the
        // settings of a Claude session that Rain's parent might sit in.
        env_remove_prefixes: vec!["CLAUDE", "ANTHROPIC"],
        stdin: Some(claude_prompt(job.skill.as_deref(), job.prompt.as_deref()).into_bytes()),
    }
}

/// Finds the program of a Claude job. A path is used as it is. A bare name is searched in the
/// `PATH` folders (`claude` also as `claude.exe`); when that fails, the default place of the
/// installer under the user's profile is tried. `exists` stands in for the file system.
pub fn resolve_claude(
    program: &str,
    path_var: Option<&str>,
    user_profile: Option<&Path>,
    exists: impl Fn(&Path) -> bool,
) -> PathBuf {
    let given = Path::new(program);
    if given.components().count() > 1 || given.is_absolute() {
        return given.to_owned();
    }
    let file = if given.extension().is_some() {
        program.to_owned()
    } else {
        format!("{program}.exe")
    };
    for folder in std::env::split_paths(path_var.unwrap_or_default()) {
        let candidate = folder.join(&file);
        if exists(&candidate) {
            return candidate;
        }
    }
    if let Some(profile) = user_profile {
        let candidate = profile.join(".local").join("bin").join(&file);
        if exists(&candidate) {
            return candidate;
        }
    }
    given.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::jobs::entry::Shell;
    use crate::modules::jobs::model::PermissionMode;

    fn args(spec: &SpawnSpec) -> Vec<String> {
        spec.args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    fn claude(skill: Option<&str>, prompt: Option<&str>) -> ClaudeJob {
        ClaudeJob {
            skill: skill.map(str::to_owned),
            prompt: prompt.map(str::to_owned),
            model: None,
            permission_mode: PermissionMode::DontAsk,
            dangerously_skip_permissions: false,
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            max_turns: None,
            program_path: "claude".to_owned(),
            extra_args: Vec::new(),
        }
    }

    fn spec_of(job: &ClaudeJob) -> SpawnSpec {
        claude_spec(
            job,
            PathBuf::from("claude.exe"),
            "Morning run",
            Path::new("C:\\work"),
            "run-1",
            "job-1",
            &BTreeMap::new(),
        )
    }

    #[test]
    fn a_program_job_passes_its_arguments_one_by_one() {
        let line = CommandLine {
            command: "robocopy".into(),
            args: vec!["D:\\a b".into(), "E:\\c".into()],
            shell: Shell::None,
        };
        let extra = BTreeMap::from([("MODE".to_owned(), "fast".to_owned())]);
        let spec = command_spec(&line, Path::new("C:\\w"), "run-1", "job-1", &extra);
        assert_eq!(spec.program, "robocopy");
        assert_eq!(args(&spec), ["D:\\a b", "E:\\c"]);
        assert_eq!(spec.raw_args, None);
        assert_eq!(
            spec.env_set,
            [
                ("RAIN_RUN_ID".to_owned(), "run-1".to_owned()),
                ("RAIN_JOB_ID".to_owned(), "job-1".to_owned()),
                ("MODE".to_owned(), "fast".to_owned()),
            ]
        );
        assert!(spec.stdin.is_none());
    }

    #[test]
    fn the_shells_get_their_documented_command_lines() {
        let cmd = CommandLine {
            command: "echo hi & exit 3".into(),
            args: vec![],
            shell: Shell::Cmd,
        };
        let spec = command_spec(&cmd, Path::new("C:\\w"), "r", "j", &BTreeMap::new());
        assert_eq!(spec.program, "cmd.exe");
        assert_eq!(
            spec.raw_args.as_deref().and_then(|a| a.to_str()),
            Some("/d /s /c \"echo hi & exit 3\"")
        );

        let power = CommandLine {
            command: "Get-Date".into(),
            args: vec![],
            shell: Shell::Powershell,
        };
        let spec = command_spec(&power, Path::new("C:\\w"), "r", "j", &BTreeMap::new());
        assert_eq!(spec.program, "powershell.exe");
        assert_eq!(
            args(&spec),
            ["-NoProfile", "-NonInteractive", "-Command", "Get-Date"]
        );
    }

    #[test]
    fn claude_command_line() {
        let mut job = claude(Some("my-skill"), Some("Extra words."));
        job.model = Some("sonnet".into());
        job.allowed_tools = vec!["Skill".into(), "Read".into(), "Bash(git status *)".into()];
        job.disallowed_tools = vec!["WebFetch".into()];
        job.max_turns = Some(30);
        job.extra_args = vec!["--flag".into()];
        let spec = spec_of(&job);

        assert_eq!(
            args(&spec),
            [
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--model",
                "sonnet",
                "--permission-mode",
                "dontAsk",
                "--allowedTools",
                "Skill,Read,Bash(git status *)",
                "--disallowedTools",
                "WebFetch",
                "--permission-prompts",
                "none",
                "--max-turns",
                "30",
                "--name",
                "Morning run",
                "--flag"
            ]
        );
        assert_eq!(spec.env_remove_prefixes, ["CLAUDE", "ANTHROPIC"]);
        let prompt = String::from_utf8(spec.stdin.unwrap()).unwrap();
        assert!(prompt.starts_with("Run the skill \"my-skill\" now"));
        assert!(prompt.ends_with("\n\nExtra words."));
    }

    #[test]
    fn a_prompt_job_has_only_its_prompt_and_a_skill_job_only_the_instruction() {
        assert_eq!(claude_prompt(None, Some("Do it.")), "Do it.");
        assert!(claude_prompt(Some("s"), None).starts_with("Run the skill \"s\" now"));
        assert!(!claude_prompt(Some("s"), None).contains("\n\n"));
    }

    #[test]
    fn bypass_permissions_adds_the_dangerous_flag_only_through_its_mode() {
        let mut job = claude(None, Some("x"));
        job.permission_mode = PermissionMode::BypassPermissions;
        job.dangerously_skip_permissions = true;
        let list = args(&spec_of(&job));
        assert!(list.contains(&"--dangerously-skip-permissions".to_owned()));
        assert!(
            !args(&spec_of(&claude(None, Some("x"))))
                .contains(&"--dangerously-skip-permissions".to_owned())
        );
    }

    #[test]
    fn the_claude_program_is_found_on_the_path_then_in_the_default_place() {
        let on_path = |p: &Path| p == Path::new("C:\\tools\\claude.exe");
        let found = resolve_claude("claude", Some("C:\\x;C:\\tools"), None, on_path);
        assert_eq!(found, PathBuf::from("C:\\tools\\claude.exe"));

        let in_profile = |p: &Path| p == Path::new("C:\\Users\\someone\\.local\\bin\\claude.exe");
        let found = resolve_claude(
            "claude",
            Some("C:\\x"),
            Some(Path::new("C:\\Users\\someone")),
            in_profile,
        );
        assert_eq!(
            found,
            PathBuf::from("C:\\Users\\someone\\.local\\bin\\claude.exe")
        );

        // Nothing found: the bare name stays, and starting it fails with a clear message.
        assert_eq!(
            resolve_claude("claude", Some("C:\\x"), None, |_| false),
            PathBuf::from("claude")
        );
        // A path is never searched.
        assert_eq!(
            resolve_claude("D:\\fake\\claude.exe", None, None, |_| false),
            PathBuf::from("D:\\fake\\claude.exe")
        );
    }
}
