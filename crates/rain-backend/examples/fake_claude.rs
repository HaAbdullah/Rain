//! A stand-in for the `claude` program, for the tests of Claude jobs. It never talks to a
//! network and never starts anything: a test must never run the real program.
//!
//! It reads its input to the end, then:
//! - writes what it was given to the file named by `FAKE_CLAUDE_DUMP` (as JSON: the arguments,
//!   the input, and the names of the environment variables that start with CLAUDE, ANTHROPIC or
//!   RAIN);
//! - prints the lines of the file named by `FAKE_CLAUDE_STREAM`, as `claude -p` would print its
//!   stream;
//! - ends with the exit code in `FAKE_CLAUDE_EXIT` (0 when it is not set).

use std::io::Read;

fn main() {
    let mut input = String::new();
    // The input is the prompt. A closed or empty input is fine.
    let _ = std::io::stdin().read_to_string(&mut input);

    if let Some(path) = std::env::var_os("FAKE_CLAUDE_DUMP") {
        let names: Vec<String> = std::env::vars()
            .map(|(name, _)| name)
            .filter(|name| {
                let upper = name.to_uppercase();
                ["CLAUDE", "ANTHROPIC", "RAIN"]
                    .iter()
                    .any(|p| upper.starts_with(p))
            })
            .collect();
        let dump = serde_json::json!({
            "args": std::env::args().skip(1).collect::<Vec<_>>(),
            "stdin": input,
            "envNames": names,
            "cwd": std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default(),
        });
        let _ = std::fs::write(path, dump.to_string());
    }
    if let Some(path) = std::env::var_os("FAKE_CLAUDE_STREAM")
        && let Ok(text) = std::fs::read_to_string(path)
    {
        for line in text.lines() {
            println!("{line}");
        }
    }
    let code = std::env::var("FAKE_CLAUDE_EXIT")
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or(0);
    std::process::exit(code);
}
