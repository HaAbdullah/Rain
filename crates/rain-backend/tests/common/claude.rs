//! Helpers for the tests of the sessions module (section 03 10.2): a throwaway Claude folder, the
//! fakes of the PC that the module reaches through traits, and builders of hook envelopes.
//!
//! Included with `#[path = "common/claude.rs"] mod claude;` by the test files that need it. Every
//! name, path and session text here is made up. Nothing touches the real Claude folder: each
//! test builds its own folder inside a temporary one and hands it to the backend through
//! `Config.claude_dir`.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures_util::future::BoxFuture;
use rain_backend::modules::sessions::{
    ExitSource, ForwarderProgram, ProcInfo, ProcessProbe, WindowApi,
};
use rain_backend::platform::windows_focus::FocusWindow;
use serde_json::{Value, json};
use tempfile::TempDir;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A throwaway Claude folder with a `sessions` folder in it.
pub struct ClaudeHome {
    pub temp: TempDir,
}

impl ClaudeHome {
    pub fn new() -> Self {
        let home = Self {
            temp: tempfile::tempdir().expect("a temporary folder"),
        };
        std::fs::create_dir_all(home.root().join("sessions")).expect("the sessions folder");
        home
    }

    /// A Claude folder that exists but has no registry folder (Claude never ran a session).
    pub fn without_registry() -> Self {
        let home = Self {
            temp: tempfile::tempdir().expect("a temporary folder"),
        };
        std::fs::create_dir_all(home.root()).expect("the Claude folder");
        home
    }

    pub fn root(&self) -> PathBuf {
        self.temp.path().join("claude-home")
    }

    pub fn settings_file(&self) -> PathBuf {
        self.root().join("settings.json")
    }

    /// Writes `<pid>.json` the way Claude Code does. `fields` is merged over the defaults.
    pub fn write_registry(&self, pid: u32, session_id: &str, fields: Value) {
        let mut file = json!({
            "pid": pid,
            "sessionId": session_id,
            "cwd": r"C:\work\demo",
            "startedAt": 1_790_000_000_000u64,
            "procStart": FakeProbe::start_of(pid).to_string(),
            "kind": "interactive",
            "entrypoint": "cli",
            "name": "Demo session",
            "nameSource": "derived",
            "status": "idle",
            "statusUpdatedAt": 1_790_000_001_000u64,
            "updatedAt": 1_790_000_002_000u64,
            "peerProtocol": 1,
            "messagingSocketPath": "CANARY-SOCKET",
        });
        if let (Some(base), Some(more)) = (file.as_object_mut(), fields.as_object()) {
            base.extend(more.clone());
        }
        std::fs::write(
            self.root().join("sessions").join(format!("{pid}.json")),
            file.to_string(),
        )
        .expect("a registry file");
    }

    /// Writes a transcript where Claude Code would, for the working folder `cwd`.
    pub fn write_transcript(&self, cwd: &str, session_id: &str, lines: &[Value]) -> PathBuf {
        let folder: String = cwd
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let dir = self.root().join("projects").join(folder);
        std::fs::create_dir_all(&dir).expect("the projects folder");
        let path = dir.join(format!("{session_id}.jsonl"));
        let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
        std::fs::write(&path, text).expect("a transcript");
        path
    }
}

// ---- the PC ----

/// A process table that the test controls.
#[derive(Default)]
pub struct FakeProbe {
    procs: Mutex<HashMap<u32, FakeProc>>,
    pub terminated: Mutex<Vec<u32>>,
}

#[derive(Clone)]
pub struct FakeProc {
    pub running: bool,
    pub image: String,
    pub parent: u32,
}

impl FakeProbe {
    /// The creation time that the fake gives the process with this pid, and that
    /// `ClaudeHome::write_registry` writes as `procStart`.
    pub fn start_of(pid: u32) -> u64 {
        134_398_601_336_233_121 + u64::from(pid) * 10_000_000
    }

    pub fn add(&self, pid: u32, image: &str, parent: u32) {
        lock(&self.procs).insert(
            pid,
            FakeProc {
                running: true,
                image: image.to_owned(),
                parent,
            },
        );
    }

    pub fn end(&self, pid: u32) {
        if let Some(process) = lock(&self.procs).get_mut(&pid) {
            process.running = false;
        }
    }
}

impl ProcessProbe for FakeProbe {
    fn creation_filetime(&self, pid: u32) -> Option<u64> {
        // 300 ns apart from what the registry says, like the real thing.
        lock(&self.procs).get(&pid).map(|_| Self::start_of(pid) + 3)
    }

    fn is_running(&self, pid: u32) -> bool {
        lock(&self.procs).get(&pid).is_some_and(|p| p.running)
    }

    fn image_name(&self, pid: u32) -> Option<String> {
        lock(&self.procs).get(&pid).map(|p| p.image.clone())
    }

    fn chain(&self, pid: u32, levels: usize) -> Vec<ProcInfo> {
        let procs = lock(&self.procs);
        let mut chain = Vec::new();
        let mut next = Some(pid);
        while let Some(current) = next {
            let Some(process) = procs.get(&current) else {
                break;
            };
            if chain.len() >= levels {
                break;
            }
            chain.push(ProcInfo {
                pid: current,
                name: process.image.clone(),
            });
            next = Some(process.parent).filter(|parent| *parent != 0);
        }
        chain
    }

    fn descendants(&self, pid: u32) -> Vec<u32> {
        let procs = lock(&self.procs);
        let mut found = vec![pid];
        let mut index = 0;
        while index < found.len() {
            let parent = found[index];
            for (child, process) in procs.iter() {
                if process.parent == parent && !found.contains(child) {
                    found.push(*child);
                }
            }
            index += 1;
        }
        found.remove(0);
        found
    }

    fn terminate(&self, pid: u32) -> std::io::Result<()> {
        lock(&self.terminated).push(pid);
        self.end(pid);
        Ok(())
    }
}

/// Exit watches that never fire by themselves: a test checks liveness through the read-time rule.
pub struct NoExits;

impl ExitSource for NoExits {
    fn watch(
        &self,
        _pid: u32,
        _start: u64,
        _on_exit: Box<dyn FnOnce() + Send>,
    ) -> Option<Box<dyn Send>> {
        None
    }
}

/// Exit watches that a test fires on demand.
#[derive(Default)]
pub struct ManualExits {
    callbacks: Mutex<HashMap<u32, Box<dyn FnOnce() + Send>>>,
}

impl ManualExits {
    pub fn fire(&self, pid: u32) -> bool {
        let callback = lock(&self.callbacks).remove(&pid);
        callback.map(|callback| callback()).is_some()
    }

    pub fn watched(&self) -> Vec<u32> {
        let mut pids: Vec<u32> = lock(&self.callbacks).keys().copied().collect();
        pids.sort_unstable();
        pids
    }
}

impl ExitSource for ManualExits {
    fn watch(
        &self,
        pid: u32,
        _start: u64,
        on_exit: Box<dyn FnOnce() + Send>,
    ) -> Option<Box<dyn Send>> {
        lock(&self.callbacks).insert(pid, on_exit);
        Some(Box::new(()))
    }
}

/// Windows that "are open", and a record of what was brought to the front.
pub struct FakeDesktop {
    pub windows: Mutex<Vec<FocusWindow>>,
    pub works: Mutex<bool>,
    pub brought: Mutex<Vec<isize>>,
    pub flashed: Mutex<Vec<isize>>,
}

impl FakeDesktop {
    pub fn new(windows: Vec<FocusWindow>) -> Self {
        Self {
            windows: Mutex::new(windows),
            works: Mutex::new(true),
            brought: Mutex::default(),
            flashed: Mutex::default(),
        }
    }
}

impl WindowApi for FakeDesktop {
    fn windows(&self) -> Vec<FocusWindow> {
        lock(&self.windows).clone()
    }

    fn bring_to_front(&self, hwnd: isize) -> bool {
        lock(&self.brought).push(hwnd);
        *lock(&self.works)
    }

    fn flash(&self, hwnd: isize) {
        lock(&self.flashed).push(hwnd);
    }
}

/// A hook program that is a file with made-up bytes, whose self test the test decides.
pub struct FakeForwarder {
    pub source: PathBuf,
    pub starts: Mutex<bool>,
}

impl FakeForwarder {
    pub fn new(folder: &Path, bytes: &[u8]) -> Arc<Self> {
        let source = folder.join("rain-hook-source.exe");
        std::fs::write(&source, bytes).expect("a fake hook program");
        Arc::new(Self {
            source,
            starts: Mutex::new(true),
        })
    }
}

impl ForwarderProgram for FakeForwarder {
    fn source(&self) -> Option<PathBuf> {
        self.source.is_file().then(|| self.source.clone())
    }

    fn selftest<'a>(&'a self, _program: &'a Path) -> BoxFuture<'a, bool> {
        let starts = *lock(&self.starts);
        Box::pin(async move { starts })
    }
}

// ---- hook envelopes ----

/// The body of `POST /internal/hook`, as `rain-hook` writes it.
pub fn envelope(mode: &str, event: Value, env: Value) -> Value {
    json!({ "v": 1, "mode": mode, "env": env, "event": event })
}

/// An event of a session with the fields that every event carries.
pub fn event(name: &str, session_id: &str, extra: Value) -> Value {
    let mut event = json!({
        "hook_event_name": name,
        "session_id": session_id,
        "cwd": r"C:\work\demo",
        "transcript_path": r"C:\nowhere\transcript.jsonl",
    });
    if let (Some(base), Some(more)) = (event.as_object_mut(), extra.as_object()) {
        base.extend(more.clone());
    }
    event
}

pub fn event_envelope(name: &str, session_id: &str, extra: Value) -> Value {
    envelope(
        "event",
        event(name, session_id, extra),
        json!({ "entrypoint": "cli" }),
    )
}

pub fn permission_envelope(session_id: &str, tool: &str, input: Value, extra: Value) -> Value {
    let mut fields = json!({ "tool_name": tool, "tool_input": input });
    if let (Some(base), Some(more)) = (fields.as_object_mut(), extra.as_object()) {
        base.extend(more.clone());
    }
    envelope(
        "permission",
        event("PermissionRequest", session_id, fields),
        json!({ "entrypoint": "cli" }),
    )
}
