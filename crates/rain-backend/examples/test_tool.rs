#![allow(clippy::disallowed_methods)] // a test program: a plain sleep is the point
//! A tiny program for the tests of jobs and idle jobs. It does nothing useful.
//!
//! - `ticker <count> <millis>`: prints `tick 1`, `tick 2` ... with a pause after each, then ends
//!   with exit code 0. A frozen program prints nothing, so the lines show whether it ran.
//! - `progress <millis> <line>...`: prints each line with a pause after it, then ends.
//!   Used for the `rain-progress:` lines of idle jobs.
//! - `exit <code>`: ends at once with that code.

use std::io::Write;
use std::time::Duration;

fn say(text: &str) {
    println!("{text}");
    let _ = std::io::stdout().flush();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("ticker") => {
            let count: u32 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(10);
            let millis: u64 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(100);
            for n in 1..=count {
                say(&format!("tick {n}"));
                std::thread::sleep(Duration::from_millis(millis));
            }
        }
        Some("progress") => {
            let millis: u64 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(100);
            for line in args.iter().skip(2) {
                say(line);
                std::thread::sleep(Duration::from_millis(millis));
            }
        }
        Some("exit") => {
            let code: i32 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(0);
            std::process::exit(code);
        }
        _ => say("test_tool: nothing to do"),
    }
}
