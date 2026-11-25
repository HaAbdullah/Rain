# Rain backend: foundation

Status: draft 2, 2026-11-22. Draft 1 was written by the backend architect. Draft 2 replaces every open "spike pending" choice by the result the five spikes proved (appendix D links each result to its report and source file), and was checked line by line against `decisions.md`.
This document is the contract for all backend code, in the same way `docs/ui-spec.md` is the contract for the frontend.
It decides everything that two or more feature areas share. It decides nothing that belongs to one feature area.

Six feature sections are written on top of it. Each one is a separate file in this folder:

| File | Area | Modules it owns (section 11) |
|---|---|---|
| `01-launcher-apps.md` | launcher and apps | `launcher` |
| `02-projects-games.md` | projects and games | `projects`, `games` |
| `03-sessions-jobs-idle.md` | Claude sessions, scheduled jobs, idle jobs | `sessions`, `jobs`, `idle` |
| `04-life.md` | life (vault, habits, calendar) | `life` |
| `05-disk-machine.md` | disk and machine | `disk`, `machine` |
| `06-gamemode-settings-lifecycle.md` | game mode, settings, lifecycle | `gamemode`, `settings`, `lifecycle` |

Inputs that bind this document, in this order: `scratchpad/backend/decisions.md` (the settled rulings), then this file, then the research reports (background only).
Where a feature section disagrees with this file, this file wins. If a section needs a different rule, it says so in its "open points" and the architect changes this file.

Words used with one meaning only:

| Word | Meaning |
|---|---|
| shell | the Tauri program `rain.exe`: window, hotkey, bridge |
| backend | the program `rain-backend.exe`: all features |
| hook | the program `rain-hook.exe`: forwards one Claude Code event |
| pipe | the Windows named pipe the backend listens on |
| bridge | the code that carries a frontend call to the pipe (Tauri command or Vite proxy) |
| module | one folder of backend code for one feature area (section 11) |
| topic | one named kind of live update on the stream (section 5) |
| frame | one message on the stream |
| sandbox | a run that must not touch real user data (section 2.7) |
| data folder | the private folder of TOML files that the user owns (section 10) |
| local folder | the machine-local folder with the database, cache and logs (section 10) |
| stage | one step of the build plan (section 14) |

"Proven" means a spike ran it on this PC (Windows 11, rustc 1.100) and appendix D names the source file to copy from. "UNPROVEN" or "UNVERIFIED" means nobody saw it work; the code must handle it defensively and appendix C lists what depends on it. A choice that was tested keeps the exact crate names, versions and features of the spike.

---------------------------------------------------------------------------------------------------

## 0. Decisions at a glance

| # | Decision | Reason |
|---|---|---|
| D1 | One Cargo workspace at the repository root: `src-tauri` (package `rain`) plus `crates/rain-types`, `crates/rain-pipe`, `crates/rain-backend`, `crates/rain-hook`. | One lock file, one target folder, shared versions. |
| D2 | The backend is a library plus a thin binary. Tests start the library in-process on a test pipe. | Fast tests with the real HTTP path. |
| D3 | The API is HTTP/1.1 with JSON over a named pipe. Base path `/api/v1`. No TCP port. The pipe is protected by an explicit DACL for the current user SID and SYSTEM, keeps 8 pending instances, and clients use one pooled `hyper-util` client. | A browser page cannot open a pipe, so no web page can reach it. Proven (spike types-stream): a single pending instance made 10 parallel requests wait about 15 ms each; 8 instances and a pooled client give 0.04 ms median. |
| D4 | Live updates are Server-Sent Events on `GET /api/v1/stream`. Every frame is a full snapshot. A reconnect always triggers a refetch. | Simple, no replay logic, cannot drift. Proven: delivery median 0.12 ms, reconnect after graceful stop and hard kill, `resync` on lag. |
| D5 | The shell has exactly one Tauri command: `api_request` (decisions.md section 2). It relays the stream as the Tauri event `rain:stream`. Backend state that only the shell knows is answered by `api_request` itself for the reserved path `/local/backend` (section 6.1). Binary resources go through the custom scheme `rain-media`. | One generic door, nothing to keep in step with every route. |
| D6 | The shell starts the backend when the pipe is silent. The backend is a detached process and survives the window. Only the backend decides when it exits, by a stop reason it announces on the stream. | The user can close the window and keep jobs running; game mode can end Rain for good. |
| D7 | Debug builds are a sandbox by default: they use other names and folders (`Rain-dev`, `rain-dev-<user>`). Release builds use the real ones. Environment variables override both. | A developer or an agent can never touch real data by accident. |
| D8 | The frontend gets a `Bridge` object from the existing `shell` (Tauri or browser). A REST client implements `RainApi` on top of it. A router sends each method to the real client or to the mock until stage 10. | The app keeps working after every stage. |
| D9 | Types are written once in Rust (`rain-types`) and generated to TypeScript with ts-rs 12.0.1 into ONE file, `src/bindings/rain-types.ts`, by a `decl()`-based generator (`cargo test -p rain-types write_single_file`). `src/data/types.ts` re-exports it. | Rust is the source of truth. Proven: 23 contract types come out strictly identical to the hand-written ones. ts-rs per-file export loses imports of types given with `#[ts(type = ...)]`, so the single file is the proven layout. specta needs about twice the attributes and is a pre-release. |
| D10 | TOML files in the data folder are edited with `toml_edit` 0.25 (with two proven fixes: keep the value decor, keep CRLF), watched with `notify` 9 and its debouncer, and written atomically (temp file, `sync_all`, rename, retry). SQLite holds only history and rebuildable cache. | The user owns the TOML; losing the database loses nothing important. |
| D11 | Every module has the same files: `mod.rs`, `routes.rs`, `service.rs`, `store.rs`. Modules call each other only through the other module's public `Service`. | A reader who knows one module knows all of them. |
| D12 | Idle cost rule: no timers except the six listed in section 11.5. Everything else is event driven. Clippy forbids `tokio::time::interval`. The runtime is `multi_thread` with 2 workers (measured equal to `current_thread`: 0.0 ms CPU idle, under 1 MB private memory). | Idle under about 30 MB and about 0% CPU. |
| D13 | A handler aims to finish within 10 s; the hard limit enforced by the server is 15 s. Long work returns an id at once and reports on a topic. | The UI never freezes and no timeout is needed above 15 s. |
| D14 | Errors have one body, `{ code, message, source?, reason? }`, equal to the frontend `RainError` plus two new codes (`invalid`, `conflict`) and an optional machine tag `reason`. Body and query rejections of the extractors, unknown routes, wrong methods and handler panics all use this body. | The UI already shows `message` as is. Proven for every one of these paths (spike types-stream). |
| D15 | Process rules: the backend is started detached with `DETACHED_PROCESS`. It is never inside a Job Object. Every child tree that Rain owns and may have to kill (a scheduled job run, an idle job run) gets its own hand-written Job Object with `KILL_ON_JOB_CLOSE`. A program the user launches (app, game, project) is never inside such a job. | Proven (spike process): a launched exe dies with a kill-on-close job unless the job allows breakaway; `process-wrap` never sets `KILL_ON_JOB_CLOSE`, so a backend crash would orphan runs. |

---------------------------------------------------------------------------------------------------

## 1. Programs and crates

### 1.1 Layout

```
Rain/                                  repository root
  Cargo.toml                           [workspace]; shared versions, lints, profiles
  Cargo.lock                           the only lock file (moved from src-tauri)
  clippy.toml                          disallowed methods (section 11.5)
  rustfmt.toml                         max_width = 100
  .cargo/config.toml                   TS_RS_LARGE_INT, RAIN_TYPES_OUT (section 9.3)
  crates/
    rain-types/                        pure data: API types, Frame, ApiError, names and paths, the TypeScript generator
    rain-pipe/                         Windows named-pipe HTTP client (pooled) + SSE reader (tokio, hyper)
    rain-backend/                      lib `rain_backend` + bin `rain-backend`
    rain-hook/                         bin `rain-hook` (std only, zero dependencies)
  src-tauri/                           package `rain`: the shell (lib `rain_lib` + bin `rain`)
    tauri.conf.json  build.rs  capabilities/  icons/  gen/    (unchanged places)
    tauri.bundle.conf.json             release-only additions (externalBin), section 1.5
    binaries/                          release-only copies of the sidecars (git-ignored)
  src/
    bindings/rain-types.ts             GENERATED TypeScript, one file (committed, section 9.3)
    data/ shell/ ...                   frontend (section 8)
  vite/pipe-proxy.ts                   Vite plugin for browser dev mode (section 7)
  scripts/                             dev.mjs backend.mjs smoke.mjs check.mjs bundle.mjs
  docs/backend/                        this document and the six sections
```

The project targets Windows 11 only. Only `rain-types` must also compile on other systems (so type generation is portable). Everything else may use `#[cfg(windows)]` freely.

### 1.2 What each crate owns and may depend on

| Crate | Kind | Owns | Depends on |
|---|---|---|---|
| `rain-types` | lib | Every type that crosses the API (structs, enums, bodies). `Frame`, `ApiError`, `Ping`, `Health`, topic definitions. Name and path rules in module `env` (pipe name, folder names, sandbox flag). The TypeScript generator (`bindings.rs`). Uses only `std`, `serde`, `serde_json`, `serde_repr`, `ts-rs`. No tokio, no OS calls. | none of ours |
| `rain-pipe` | lib | `PipeClient` (one cloneable pooled `hyper-util` client over a pipe connector): `request`, `json`, `api_request`, `send` (response not read, for SSE). `SseReader` and `run_stream`: read frames, reconnect forever, idle limit. Retry for `ERROR_PIPE_BUSY`. Used by the shell, the integration tests and `rain-backend --stop`. | `rain-types`, tokio, hyper, hyper-util, http-body-util, tower |
| `rain-backend` | lib + bin | All features, the pipe server, storage, logging. | `rain-types`; dev-dependency `rain-pipe` |
| `rain-hook` | bin | Reads one Claude Code hook event from stdin, forwards it to the backend, exits. Built in stage 8. Proven shape: spike claude-hooks `fwd` (no dependencies, release exe 162 KB). | none (std only). Its 15-line pipe-name function is a copy of `rain_types::env::pipe_name`; a test in `rain-backend` includes the hook's file with `#[path]` and compares both on a list of user names. |
| `rain` (`src-tauri`) | lib + bin | Window, global hotkey, single instance, backend start and watch, `api_request`, stream relay, `rain-media` scheme. | `rain-types`, `rain-pipe`, tauri and its plugins |

Dependency rules (a test, `tests/crate_graph.rs` in `rain-backend`, parses `cargo metadata` and fails on a violation):

1. `rain-types` depends on no other workspace crate.
2. Nothing depends on `rain-backend` except its own tests. The shell never links the backend.
3. `rain-hook` depends on no crate at all.
4. Nobody depends on `rain` (the shell).

### 1.3 Binary names

| Package | Binary | Windows subsystem | Notes |
|---|---|---|---|
| `rain` | `rain.exe` | `windows` in release, console in debug (as today) | In an installed build the bundler may rename the main binary to `Rain.exe`; nothing in the code may assume the file name of the shell except the shell itself (`current_exe`). |
| `rain-backend` | `rain-backend.exe` | `windows` always, also in debug | Never has a console. Logs go to a file. Flag `--console` attaches to the parent console for debugging (`AttachConsole(ATTACH_PARENT_PROCESS)`) and also logs to it. |
| `rain-hook` | `rain-hook.exe` | `console` (Claude Code needs its stdio) | Measured cost of one call: about 20 ms wall (median 18.5 to 23 ms; a bare `cmd /c rem` costs 16.9 ms), of which `main` is 4.4 to 6.6 ms. The earlier target "under 10 ms" cannot be met because process creation alone is 12 to 17 ms. Budget: wall time under 50 ms and `main` under 10 ms when Rain is not running. Always exits 0 with empty stdout on every failure. |

Backend flags (all of them; no others):

| Flag | Effect |
|---|---|
| (none) | Run the server. |
| `--stop` | Connect to the pipe, `POST /api/v1/internal/shutdown` with reason `stop`, wait up to 5 s until the pipe is gone. Exit 0 if nothing was running or it stopped; exit 1 on timeout. |
| `--stop --force` | As `--stop`, then if the pipe is still there: read `pid` from `GET /api/v1/ping` and end that process with `TerminateProcess`. Never by image name. |
| `--version` | Print `version` and exit. |
| `--console` | See above. |

Environment variables (read once at start, only by `Config::from_env` in the backend and by the functions in `rain_types::env`):

| Variable | Meaning |
|---|---|
| `RAIN_PIPE` | Pipe name without the `\\.\pipe\` prefix. Allowed: `A-Z a-z 0-9 . _ -`, 1 to 64 characters. Anything else is rejected at start with a clear error. |
| `RAIN_DATA_DIR` | The data folder (section 10). Beats `machine.toml`. |
| `RAIN_LOCAL_DIR` | The local folder. Default when `RAIN_DATA_DIR` is set: the sibling folder `<RAIN_DATA_DIR>.local`. |
| `RAIN_LOG` | Log filter in `tracing` syntax, for example `info,rain_backend::disk=debug`. Default `info`. |
| `RAIN_SANDBOX` | `1` forces sandbox mode, `0` forces real mode. Default: sandbox in debug builds, real in release builds, and always sandbox when `RAIN_DATA_DIR`, `RAIN_LOCAL_DIR` or `RAIN_PIPE` is set. |

### 1.4 The workspace root file

```toml
# Cargo.toml (repository root)
[workspace]
resolver = "3"
members = ["src-tauri", "crates/*"]
default-members = ["src-tauri", "crates/rain-backend"]   # `cargo build` builds shell + backend

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.100"        # the version every spike compiled with; lowering it is untested
license = "MIT"
# repository = "..."          # set when the repository is published

[workspace.lints.rust]
unsafe_op_in_unsafe_fn = "deny"
[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
dbg_macro = "deny"
todo = "warn"
unwrap_used = "warn"          # allowed in tests and in `main` start-up code with a comment
expect_used = "allow"

[profile.release]
opt-level = "s"
lto = "thin"
codegen-units = 1
strip = true
panic = "unwind"              # NOT abort: a handler panic must become a 500 (section 11.7).
                              # The spikes measured sizes with abort; unwind adds a little. `panic` cannot be set per package.
# switch on in stage 8, when crates/rain-hook exists (Cargo warns about a profile for a package that is not in the workspace):
# [profile.release.package.rain-hook]
# opt-level = "z"             # the hook is small and short-lived; spike fwd: 162 KB with opt-level z

[workspace.dependencies]      # the only place where versions are written (list in section 1.6)
```

Every member has `[lints] workspace = true` and takes versions with `dep = { workspace = true }`. A feature section never writes a version number: it names the crate it needs, and the stage that builds it adds one line to `[workspace.dependencies]`.

### 1.5 How `src-tauri` becomes a member without breaking the Tauri CLI

What stays exactly where it is: `src-tauri/tauri.conf.json`, `build.rs`, `capabilities/`, `icons/`, `gen/`, `src/main.rs`, `src/lib.rs`. The CLI looks for `src-tauri/tauri.conf.json` from the repository root, which is where every npm script runs. Nothing about that changes.

Steps (done once, in stage 1a, in this order):

1. Copy `src-tauri/Cargo.lock` to `Cargo.lock` at the root, so the existing pins (tauri 2.12.7 and friends) are kept.
2. Create the root `Cargo.toml` (section 1.4).
3. Edit `src-tauri/Cargo.toml`: `version.workspace = true`, `edition.workspace = true`, `rust-version.workspace = true`, `[lints] workspace = true`, and add the new dependencies. Keep `[lib] name = "rain_lib"` and `crate-type`. Keep `[build-dependencies] tauri-build`.
4. Delete `src-tauri/Cargo.lock`. The folder `src-tauri/target` is now stale and may be deleted by hand. Build output moves to `target/` at the root. `.gitignore` already ignores `target`.
5. Run `cargo build -p rain` and `npm run tauri dev` once. The Tauri CLI uses `cargo metadata`, so the moved target folder is found by itself (UNVERIFIED in this repository; this is the first acceptance check of stage 1).

Fallback if step 5 fails and cannot be fixed with a setting: make `src-tauri/Cargo.toml` the workspace root (`[workspace] members = [".", "../crates/*"]`) and keep everything else in this document unchanged. Only the place of `Cargo.lock` and `target` moves.

The sidecar rule (release only): `src-tauri/tauri.bundle.conf.json` holds

```json
{ "bundle": { "externalBin": ["binaries/rain-backend", "binaries/rain-hook"] } }
```

and is merged only by the release script: `tauri build --config src-tauri/tauri.bundle.conf.json`. The base `tauri.conf.json` has no `externalBin`, because Tauri then requires `src-tauri/binaries/<name>-<target-triple>.exe` before every build and its build script deletes the destination file, which fails while a backend is running (research-transport section 7). In development the shell finds the backend next to itself: `current_exe().with_file_name("rain-backend.exe")`, in `target\debug\` (dev) and in the install folder (release, Tauri copies the sidecar next to the main binary without the triple in its name; UNVERIFIED for the NSIS layout, checked in stage 2b). The target triple on this PC is `x86_64-pc-windows-msvc`. The release script `scripts/bundle.mjs` (stage 2b) builds both sidecars in release mode, copies them to `src-tauri/binaries/<name>-x86_64-pc-windows-msvc.exe`, and then calls the Tauri CLI. `src-tauri/binaries/` is git-ignored.

### 1.6 Dependency list (every row marked "proven" compiled and ran in a spike on 2026-11-22; pin by `Cargo.lock`)

| Crate | Version | Features | Used by | Why | Proof |
|---|---|---|---|---|---|
| tokio | 1.54 (1.54.1 resolved) | default-features off; rt, rt-multi-thread, net, time, macros, sync, io-util, signal; the backend adds process, fs | backend, pipe, shell | async runtime; `net` has `named_pipe` | proven (types-stream, crates); `process` and `fs` not in a spike, standard |
| axum | 0.8.9 | default-features off; http1, json, query, tokio, macros | backend | router and extractors (`macros` for `#[derive(FromRequest)]`) | proven |
| hyper | 1.12 (1.12.0) | http1, client, server | backend, pipe | HTTP over the pipe | proven |
| hyper-util | 0.1.21 | tokio, service, server, http1, client, client-legacy | backend, pipe | `TowerToHyperService`, `TokioIo`, `TokioExecutor`, the pooled `Client` (`client-legacy` IS needed: it is the pool) | proven |
| http-body-util | 0.1.5 | | backend, pipe | read bodies, `Full<Bytes>` | proven |
| tower | 0.5 (0.5.3) | util (the backend adds timeout, limit) | backend, pipe | `Service` for the pipe connector, layers | util proven; timeout and limit standard |
| tower-http | 0.6 (0.6.11) | default-features off; catch-panic (the backend adds limit) | backend | panic to 500 JSON, body limit | catch-panic proven |
| tokio-stream | 0.1 | sync | backend | `BroadcastStream` to SSE | proven |
| futures-util | 0.3 | | backend, pipe | `stream::iter`, `StreamExt::take_until` | proven |
| tokio-util | 0.7 | rt | backend | `TaskTracker`, `CancellationToken` (a `CancellationToken` replaces the spike's `watch<bool>`: the spike's `watch::Receiver::wait_for` guard is not `Send` inside `select!` in a spawned task and needed a helper) | token not in a spike, standard |
| serde, serde_json | 1 | derive; serde_json with feature `preserve_order` and NEVER `arbitrary_precision` | all | JSON. `arbitrary_precision` would break untagged number enums through feature unification, so a test in `source_rules.rs` fails if it is enabled anywhere in the workspace (`cargo tree -e features`) | proven |
| serde_repr | 0.1 | | rain-types | number unions such as `30 \| 60 \| 300`; parsing rejects any other number, so the backend validates for free | proven |
| ts-rs | 12.0.1 | default only (serde-compat); the per-file `export` feature set is not used | rain-types | TypeScript generation, single file (section 9.3). `no-serde-warnings` is NOT used: it was never tested | proven |
| toml | 0.9 or newer, chosen at stage 1b | | backend | read typed TOML | UNPROVEN: no spike read TOML with it; `toml_edit` 0.25.20 was proven for editing. If `toml` causes trouble, `toml_edit::de` reads typed values from the same parse |
| toml_edit | 0.25 (0.25.20) | | backend | edit TOML keeping comments, with the two fixes of section 10.4 | proven |
| rusqlite | 0.43 (0.43.0, SQLite 3.54.1) | bundled | backend | SQLite; bundled adds 1.10 MB to the exe with `lto`, `strip`, `opt-level = "s"` (1.67 MB with the default release profile) | proven |
| notify, notify-debouncer-full | 9.0.0, 0.8.0 | | backend | file watching. Fallback, tested and identical on Windows: notify 8.2.2 with notify-debouncer-full 0.7.0 | proven (notify 9.0.0 is one day old on 2026-11-22: see appendix C) |
| tracing, tracing-subscriber, tracing-appender | 0.1.44, 0.3.23, 0.2.5 | subscriber: default-features off; fmt, env-filter, registry | backend, shell | logging (section 10.7) | proven |
| chrono | 0.4.45 | | backend (jobs only) | `croner` 5 takes `chrono` date-times; this stays inside the `jobs` module. The API contract still carries dates as strings (section 4.3) | proven (with croner) |
| croner | 5.0.0 | default (chrono support) | backend (jobs) | cron schedules; DST handled (section 03) | proven |
| windows-sys | 0.61 (0.61.2) | Win32_Foundation, Win32_Security, Win32_Security_Authorization, Win32_System_Threading | backend (pipe ACL) | security descriptor and user SID | proven |
| windows | 0.62 (0.62.2) | the union below | backend | COM, shell, icons, process, job objects, console | proven |
| winreg | 0.56 | | backend | autostart key; Steam path (read) | proven |
| png | 0.18 | | backend | encode icons (`Compression::Fast`) | proven |
| rayon | 1.12 | | backend (disk) | the disk walker's thread pool, built for one scan and dropped after it | proven |
| thiserror | 2 | | backend, pipe | error enums | standard, not in a spike |
| anyhow | 1 | | backend start-up only | start-up errors | standard, not in a spike |
| keyring-core, windows-native-keyring-store | 1.0.0, 1.1.0 | | backend (`core::secrets`) | Credential Manager. Used as `keyring_core::set_default_store(Store::new()?)` then `Entry::new(service, name)`. The all-in-one `keyring` 4.2.0 also works and is NOT chosen (more dependencies). Not added until a feature needs a secret; none does in this round (calendar is off, decisions.md section 4) | proven |
| tempfile | 3.27 | | dev only | temp folders in tests | proven |

The `windows` features the spikes needed, per use (add a feature in the stage that first needs it; features not listed were tested by removal and are NOT needed: `Win32_System_Com_StructuredStorage`, `Win32_System_Variant`, `Win32_UI_Shell_Common`, `Win32_UI_Shell_PropertiesSystem`):

| Use | Features |
|---|---|
| everything | `Win32_Foundation`, `Win32_Security` |
| COM thread, app list, icons, reveal, launch by AUMID | `Win32_System_Com`, `Win32_UI_Shell`, `Win32_Storage_EnhancedStorage` (the property keys `PKEY_AppUserModel_ID`, `PKEY_Link_TargetParsingPath`, `PKEY_Link_Arguments` live there), `Win32_Graphics_Gdi`, `Win32_UI_WindowsAndMessaging` |
| process list, process start and end times, thread resume | `Win32_System_Diagnostics_ToolHelp`, `Win32_System_Threading`, `Win32_System_ProcessStatus` |
| job objects | `Win32_System_JobObjects` |
| files, drives, disk size | `Win32_Storage_FileSystem`, `Win32_System_IO`, `Win32_System_SystemInformation` (tick count) |
| idle time | `Win32_UI_Input_KeyboardAndMouse` |
| console control (only in the throwaway helper, section 11.6) | `Win32_System_Console` |
| registry through the raw API (not used: `winreg` is used) | `Win32_System_Registry`; `RegCreateKeyExW` also needs `Win32_Security` |
| pipes | `Win32_System_Pipes` (only if the raw API is used; tokio's `named_pipe` needs none) |

API notes for `windows` 0.62 (they cost the spikes time): `BOOL` is `windows::core::BOOL`; `LimitFlags` takes the `JOB_OBJECT_LIMIT` newtype directly; `SHGetKnownFolderItem` is generic (`let folder: IShellItem = SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)?`); `DRIVE_FIXED` needs `Win32_System_WindowsProgramming`, so use the constant `3`; every `PWSTR` from a COM call is freed with `CoTaskMemFree`.

Not used, by decision or by spike result: `tauri-plugin-shell` (alpha, extra permissions), `tauri-plugin-autostart` (it would put the autostart switch in the shell, away from the settings the backend owns; it writes the same registry key), WebSocket crates, `chrono`/`time` in the contract (dates are strings, section 4.3), `process-wrap` (never sets `KILL_ON_JOB_CLOSE`; spike process), `sysinfo` (the hand-written process list is as fast; spike process), `jwalk` (30 times slower than a hand-written walk and hides access-denied folders; spike winapi), `ignore` (same speed as the hand-written walker, 4 times the memory), `keyvalues-parser` (11 times slower than the 90-line VDF parser), a YAML crate (a YAML library re-serialises, so it cannot promise that every other byte of a note is unchanged; spike crates), `specta` and `tauri-specta` (spike types-stream), calendar crates (decisions.md section 4).

---------------------------------------------------------------------------------------------------

## 2. Process lifecycle

### 2.1 The five states of the backend as the shell sees it

| State | Meaning | Entered when |
|---|---|---|
| `starting` | spawn requested, pipe not yet answering | `ensure_backend` spawned it |
| `up` | `GET /ping` answered and the stream is connected | stream `hello` received |
| `down` | pipe silent, no stop was announced (crash or not started) | stream ended without a `lifecycle` frame, or the first connect failed |
| `stopped` | the backend announced it was stopping for reason `gameMode` or `stop` | `lifecycle` frame with that reason |
| `restarting` | announced reason `upgrade` or `restart` | `lifecycle` frame with that reason; the shell starts a new backend as soon as the pipe is gone |

### 2.2 Who starts the backend, and when

| Trigger | Who | Rule |
|---|---|---|
| Shell starts (user, Start menu, logon) | shell, in `setup`, as a background task | `ensure_backend()` runs after the window is created. It never delays the window. |
| A frontend call finds the pipe silent | shell, inside `api_request` | `ensure_backend()` once, then the call is retried once. |
| Window is summoned while state is `down` | shell | `ensure_backend()`. |
| Development | `scripts/dev.mjs` and `scripts/backend.mjs` | Build, stop the old one, start the new one (section 2.8). |

State `stopped` with reason `gameMode`: nothing starts the backend again in this shell process. The shell exits (section 2.6).
State `stopped` with reason `stop`: nothing starts it by itself, but the next trigger in the table does.

`ensure_backend()` (shell side, one function, one lock so concurrent callers share one spawn):

1. Try `GET /api/v1/ping` on the pipe (timeout 1 s).
2. Answered: compare `version` and `exePath` with the expected backend next to the shell. Equal: done. Different and the pipe is not a sandbox override: the upgrade rule (section 2.5).
3. Not answered, error "file not found" (os error 2) or timeout: spawn the backend (section 2.3), then poll the pipe every 50 ms for at most 5 s (`BACKEND_READY_WAIT`).
4. Spawn guard: at most 3 spawns in any 60 s. Over that, return `unavailable` with reason `backendCrashLoop` and do not spawn until a user action calls `ensure_backend` again after the 60 s are over.
5. Not ready after 5 s: `unavailable`, reason `backendDown`, message "Rain backend is not running".

### 2.3 The detached start

Proven pattern (spike process, `src/bin/parent.rs` and `src/bin/backend.rs`): the child was alive 800 ms after its parent exited, had no window (`MainWindowHandle` 0, `GetConsoleWindow` null, no `conhost` child), was stopped by pid and restarted without trouble.

```rust
// shell: backend.rs
const DETACHED_PROCESS: u32 = 0x0000_0008;          // NOT combined with CREATE_NO_WINDOW (the two are exclusive)
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

let exe = std::env::current_exe()?.with_file_name("rain-backend.exe");
let spawn = |flags: u32| {
    // clippy.toml forbids `Command::new` (section 11.5); the shell is the one place besides `platform/`
    // that may use it, with #[allow(clippy::disallowed_methods)] and a comment pointing here.
    let mut cmd = std::process::Command::new(&exe);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).creation_flags(flags);
    cmd.spawn()
};
// The job of the parent may forbid breakaway: Windows then fails the spawn with os error 5.
// This happened in the spike harness, so the retry is part of the product, not a theory.
let child = spawn(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB)
    .or_else(|e| if e.raw_os_error() == Some(5) { spawn(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP) } else { Err(e) })?;
drop(child);                      // never wait; `Child` has no kill-on-drop
```

`rain-backend.exe` carries `#![windows_subsystem = "windows"]` (section 1.3), so it never has a console.

Rules:
- The backend inherits the shell's environment, so `RAIN_PIPE`, `RAIN_DATA_DIR` and the other variables reach it.
- The backend is never placed in a Job Object by the shell, and the backend never puts launched user apps in a kill-on-close Job Object (decision C10, D15). The scheduler gives each job run its own hand-written job (section 11.6); the user's apps and games never get one.
- If the shell itself runs inside a kill-on-close job (Windows Terminal, some task runners) and the job forbids breakaway, the backend started without breakaway can die with that job. This is accepted in development (a terminal that closes ends everything it started). The logon Run key and the Task Scheduler were not tested (appendix C): the backend started from a Run-key shell must still be checked by hand in stage 2.

### 2.4 Single instance and the pipe

- One backend per pipe name. The backend creates the first pipe instance with `first_pipe_instance(true)`. A second backend gets "access denied" (os error 5) from that call, logs "already running" at `info`, and exits with code 0 (the spikes exited with code 3; nothing reads the code, so quiet success is kept). No mutex and no pid file. Proven twice (spike process: `CreateNamedPipeW` with `FILE_FLAG_FIRST_PIPE_INSTANCE`, and a named mutex, both work; after the first copy was killed a new one started; spike types-stream: the same check on a pipe with the user-only DACL). The pipe is chosen over a mutex because it is also the API transport, so a running backend is by definition one that answers.
- The shell is single instance through `tauri-plugin-single-instance` (as today, must stay the first plugin). A second launch calls the callback, which summons the window; it passes `args`, so `--hidden` is honoured only on the first launch. The lock belongs to the app identifier (`app.rain.desktop`). In a sandbox (section 3) the shell adds `.dev` to the identifier before the app is built, so a development run (`npm run dev`, `npm run smoke`) and the installed Rain are two apps for Windows: each has its own lock, its own saved window state and its own WebView2 profile, and both can run at the same time. Only one of them holds the summon hotkey; the other reports `hotkeyAvailable: false`.
- The pipe name contains the Windows user name, so two users on one PC never meet (section 3.1).

### 2.5 Upgrade: the old backend still holds the pipe

Windows cannot overwrite a running program and the pipe name is the same for the old and the new version. The rule:

1. At every `ensure_backend()` that gets a `ping` answer, the shell compares `ping.version` with its own version (`env!("CARGO_PKG_VERSION")` of `rain`, equal to the workspace version) and `ping.exePath` with the path it would spawn (case-insensitive).
2. If both are equal: nothing.
3. If either differs (the user installed a new version, or a dev build): the shell sends `POST /api/v1/internal/shutdown` with `{"reason":"upgrade"}`.
   - The backend answers `409 conflict` with reason `busy` when `ping.busy` is true (a scheduled job, idle job, scan or launch is running). The shell then keeps the old backend, logs a warning and tries again at its next start. Nothing is killed.
   - Otherwise the backend announces `lifecycle { kind: "stopping", reason: "upgrade" }`, shuts down (section 2.6), and the shell waits up to 5 s (`UPGRADE_WAIT`) for the pipe to vanish, then spawns the new one.
4. The installer (stage 2, NSIS pre-install step) runs `rain-backend.exe --stop` from the old install before it copies files.
5. In sandbox runs (any of the override variables set, or debug builds) rule 3 does not apply: the dev scripts own restarts.

### 2.6 Shutdown

One route, `POST /api/v1/internal/shutdown`, body `{"reason":"gameMode"|"stop"|"upgrade"|"restart","force"?:boolean}`, answered `204` before anything stops. Only these callers: the shell, `rain-backend --stop`, the game mode module inside the backend (it calls the same internal function, not the route), and the settings module (reason `restart`, after a change of the data folder; section 06).

Order inside the backend (`core::shutdown`):

Proven in spike types-stream (`bin/server.rs`, `src/server.rs`): the stream ends with a final frame, the process exits 2 ms after the shutdown request, also when a client holds a stalled stream open and never reads. The basic path (route, `--stop`, `lifecycle` frame, closing the pipe) is built in stage 1a; the module stop budgets, the upgrade rule, `--force` and the checkpoint belong to stage 2.

1. Mark `state.shutting_down` (new work is refused with `unavailable`, reason `shuttingDown`; reads still work).
2. Store the stop reason in `state.shutdown`. Every open stream writes the frame `lifecycle { kind: "stopping", reason }` itself as its tail (section 5.7), so it cannot be lost in a race with the cancellation. Wait until the streams have written it (at most 500 ms). The game mode `done` frame of the paragraph below is published BEFORE this step through the normal bus.
3. Call `stop()` of every module in reverse start order, all in parallel, each with a 5 s budget. A module that exceeds the budget is logged at `error` and abandoned. A `stop()` publishes no topic frame (so `done` of game mode stays the last topic frame).
4. Close the stream connections (SSE streams end because they `take_until(shutdown)`), stop the accept loop, wait at most 2 s for in-flight requests (`TaskTracker`).
5. `PRAGMA wal_checkpoint(TRUNCATE)`, close the database, flush the log, exit with code 0. Total budget `SHUTDOWN_BUDGET` = 8 s; after that the process exits anyway.

Game mode (decision C5), the part that belongs to this foundation: the game mode module performs its own stops, then calls the internal shutdown with reason `gameMode`. It publishes its `done` state as the very last topic frame before step 2. The shell, on the `lifecycle` frame with reason `gameMode`, enters state `stopped`, waits until the pipe is closed, waits another fixed `GAME_MODE_EXIT_GRACE` = 3 s, and calls `app.exit(0)`. Nothing restarts Rain afterwards. The exact ordering of the stops is section 06.

### 2.7 Sandbox mode

`rain_types::env::sandbox()` is true when the build is a debug build, or `RAIN_SANDBOX=1`, or `RAIN_DATA_DIR`, `RAIN_LOCAL_DIR` or `RAIN_PIPE` is set; it is false only in a release build with none of those and `RAIN_SANDBOX` not `1`. `RAIN_SANDBOX=0` overrides to false.

| | Real (release, no overrides) | Sandbox (debug, tests, overrides) |
|---|---|---|
| Pipe name | `rain-<user>` | `rain-dev-<user>` (or `RAIN_PIPE`) |
| Roaming folder | `%APPDATA%\Rain` | `%APPDATA%\Rain-dev` |
| Local folder | `%LOCALAPPDATA%\Rain` | `%LOCALAPPDATA%\Rain-dev` (or `RAIN_LOCAL_DIR`) |
| Credential target prefix | `rain` | `rain-dev` |
| Autostart registry value | `Rain` | refused (`conflict`, reason `sandbox`) |
| Claude user settings file | writable by the hooks installer (section 06/03 rules) | refused unless the canonical target path lies inside the system temp folder, or the data folder or local folder of this run (section 03 6.9.8) |
| Claude folder (`RAIN_CLAUDE_DIR`) | `.claude` under the user profile (read only by sessions, section 03) | `RAIN_CLAUDE_DIR` if set; honoured in sandbox mode only, and only when the canonical path lies inside the system temp folder, or the data folder or local folder of this run; `Config.claude_dir` holds the result |
| Vault write (habit tick) | allowed | refused unless the vault folder is inside the data folder (a fixture) |
| `GET /ping` | `sandbox: false` | `sandbox: true` |

`<user>` = `sanitize(USERNAME)`: lower-case, every character outside `a-z0-9` replaced by `-`, cut to 32 characters. The same function exists in Rust (`rain_types::env::user_slug`) and in `vite/pipe-proxy.ts`, and a test in `scripts` compares the two on a list of names.

### 2.8 Development workflow

npm scripts (exact):

| Script | Does |
|---|---|
| `npm run dev` | `node scripts/dev.mjs`: (1) `backend --stop`, (2) `cargo build -p rain-backend -p rain-hook`, (3) start the backend detached, (4) start a watcher on `crates/**/src` and `crates/**/Cargo.toml`: on a change it debounces 1 s and runs stop, build, start again, and also `npm run types` when the change is under `crates/rain-types`; (5) run `vite`. This is also the `beforeDevCommand` of Tauri, so `npm run tauri dev` and a plain browser use the same backend. A build error is printed and the old backend stays stopped (the UI shows its error rows). |
| `npm run backend:start` / `backend:stop` / `backend:restart` | `node scripts/backend.mjs <verb>`: the same steps as `dev.mjs` without vite. |
| `npm run types` | `cargo test -p rain-types write_single_file`: writes `src/bindings/rain-types.ts` (section 9.3). |
| `npm run types:check` | `RAIN_TYPES_CHECK=1 cargo test -p rain-types write_single_file`: fails if the committed file differs from what the Rust types generate (CRLF-normalised). |
| `npm test` | `vitest run` (frontend unit tests, section 12). |
| `npm run test:rust` | `cargo test --workspace`. |
| `npm run check` | `node scripts/check.mjs`: the stage checklist of section 12.4, in order, stopping at the first failure. |
| `npm run smoke` | `node scripts/smoke.mjs`: the "app starts" check (section 12.5). |
| `npm run bundle` | `node scripts/bundle.mjs` (stage 2b): release build of the sidecars and `tauri build --config src-tauri/tauri.bundle.conf.json`. |

Why the backend is rebuilt only by scripts: `tauri dev` watches only `src-tauri/` for Rust changes (UNVERIFIED), and Windows refuses to overwrite a running `rain-backend.exe`. So the script always stops first (the stop command is the same exe, which Windows allows to run a second time).

`vite.config.ts` adds `crates/**`, `target/**` and `.rain-*` to `server.watch.ignored`.

Logs while developing: `Get-Content -Wait -Tail 50 "$env:LOCALAPPDATA\Rain-dev\logs\backend.log"`. `scripts/dev.mjs` prints that path at start.

### 2.9 Other lifecycle events

| Event | Behaviour |
|---|---|
| Window closed (Alt+F4, close button) | Unchanged: the shell hides the window (`prevent_close`). The page, the shell, the backend and its work keep running. |
| Hidden window | The shell keeps the stream open and keeps relaying frames. The frontend stops its own timers as today (`rain:hidden`). |
| "Start with Windows" on | The backend writes `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` value `Rain` = `"<shell exe>" --hidden`. `<shell exe>` is the path the shell reports with `PUT /internal/shell` (section 6.8), never a guess. The shell with `--hidden` creates the window hidden, registers the hotkey, starts the backend, and does not call `summon`. Also removes a `StartupApproved\Run` "disabled" value for `Rain` when the user switches on. Off by default. Details: section 06. |
| Logon without the switch | Nothing runs. |
| Backend crash (stream ends, no `lifecycle` frame) | State `down`; the shell calls `ensure_backend()` at once (guard: 3 per 60 s) and emits `rain:backend` `down` then `starting` then `up`. The new `hello` carries a new `startId`, so the frontend resets runs and refetches (section 5.5). |
| Shell crash or killed | The backend keeps running (it is detached). Next shell start finds it. |
| Windows sign-out or shutdown | Both processes are ended by Windows. No special code; every write is atomic (section 10.4). |
| Sleep and resume | The pipe survives. Timers that must not drift (the scheduler) recompute from the wall clock on resume (feature section 03). |

---------------------------------------------------------------------------------------------------

## 3. The named pipe

### 3.1 Name

`\\.\pipe\<pipeName>` where `pipeName` is `RAIN_PIPE` if set, otherwise `rain-<user>` (real) or `rain-dev-<user>` (sandbox). The shell, the hook, `rain-backend --stop` and the Vite proxy compute the name with the same rule (section 2.7).

Decision: the name carries the sanitised user NAME, not the user SID, although the spike proved a name with the SID (`rain-<SID>`). Reason: `rain-hook` must stay std-only and start in a few milliseconds, and the Vite proxy runs in Node; both can read `USERNAME` but cannot cheaply get the SID. The name only avoids a clash between two Windows users on one PC; the protection is the DACL (section 3.2), which uses the SID. A user name that sanitises to the same slug as another user's cannot connect anyway (access denied, `pipeDenied`). Never write a SID into a document, log or test (it is a machine identifier).

### 3.2 Security

- Every pipe instance is created with an explicit security descriptor: SDDL `D:P(A;;GA;;;<user SID>)(A;;GA;;;SY)`: a protected DACL (inherits nothing) with two allow entries, the current user's SID and LocalSystem. Draft 1 used the alias `OW` (owner); the spike used the explicit SID instead and that is the proven form, and it also avoids the case where the owner of a new object is the Administrators group of an elevated process. Proven (spike types-stream, `winsec.rs` and `bin/secproof.rs`): the DACL read back from the live handle with `GetSecurityInfo` shows exactly two ACEs, user and SYSTEM, protected (Windows expands `GA` to `FA`, mask `0x001F01FF`); a second `first_pipe_instance(true)` fails with error 5; a connect with a restricted token whose only restricting SID is Everyone is denied (error 5); a connect through `\\localhost\pipe\<name>` is denied by `reject_remote_clients(true)` while the same pipe without the flag connects.
- The user SID comes from `OpenProcessToken` then `GetTokenInformation(TokenUser)` (called twice: size, then data into a `Vec<u64>` for 8-byte alignment) then `ConvertSidToStringSidW`, freed with `LocalFree`.
- Every instance: `reject_remote_clients(true)`. Only the very first instance: `first_pipe_instance(true)`. Instances are created with `ServerOptions::create_with_security_attributes_raw(name, &sa)`.
- The descriptor is built once by `platform::pipe_acl::PipeSecurity::current_user_only()` (copy `winsec.rs` of the spike); the unsafe code lives only there; the struct owns the descriptor and frees it with `LocalFree` in `Drop`; it is `Send + Sync` and never mutated.
- There is no authentication beyond the ACL. Any process of the same Windows user can call the API. This is the accepted boundary (section 13.1). The ACL stops other users and other machines; the missing TCP port stops web pages. Not tested: a real second Windows account, an elevated administrator, narrower rights than `GA` (appendix C).
- Routes under `/api/v1/internal/` are for the shell, the hook and scripts. The shell command `api_request` and the Vite proxy both refuse them (sections 6 and 7), so the web page and any page in a browser never reach them.

### 3.3 Serving loop

axum 0.8 cannot serve a named pipe through `axum::serve`, so the backend uses hyper directly. A pipe has no listen backlog: a client takes one pipe INSTANCE, and a client that finds no free instance gets `ERROR_PIPE_BUSY`. Measured (spike types-stream): with one pending instance, 10 parallel cold requests of 300 ms took 419 ms and the worst of 200 new-connection requests took 130 ms (the busy retry sleeps round up to the 15 ms Windows timer tick); with 4 pending instances 341 ms and 23 ms; with 8 pending instances 327 ms and 7 ms. So the proven design is N acceptors, N = `PIPE_PENDING_INSTANCES` = 8. Copy `server.rs` and `winsec.rs` of the spike (`spikes/types-stream/spike/src`). The shape:

```rust
// crates/rain-backend/src/server/pipe.rs
pub struct PipeListener { name: String, sec: Arc<PipeSecurity>, pending: Vec<NamedPipeServer> }
impl PipeListener {
    /// Fails with os error 5 when the name exists: another backend runs (section 2.4).
    pub fn bind(name: &str, acceptors: usize) -> io::Result<Self> {
        let sec = Arc::new(PipeSecurity::current_user_only()?);
        let mut pending = vec![sec.create(name, true)?];              // first_pipe_instance(true)
        for _ in 1..acceptors { pending.push(sec.create(name, false)?); }
        Ok(Self { name: name.to_owned(), sec, pending })
    }
}
// One acceptor per pending instance: wait for a client, create the replacement instance at once,
// hand the connected instance to the serve loop over an mpsc channel.
async fn acceptor(mut next: NamedPipeServer, name: String, sec: Arc<PipeSecurity>,
                  tx: mpsc::Sender<NamedPipeServer>, shutdown: CancellationToken) {
    loop {
        tokio::select! {
            r = next.connect() => if r.is_err() { log; return },
            _ = shutdown.cancelled() => return,
        }
        let Ok(fresh) = sec.create(&name, false) else { log; return };
        if tx.send(std::mem::replace(&mut next, fresh)).await.is_err() { return }
    }
}
// Serve loop: for each connected instance, one task. `conns` is a JoinSet (reaped as it finishes).
//   if conns.len() >= PIPE_MAX_CONNECTIONS { drop(pipe); continue }              // connection cap (UNPROVEN: not in the spike)
//   let conn = http1::Builder::new()
//         .timer(TokioTimer::new()).header_read_timeout(HEADER_READ_TIMEOUT)    // UNPROVEN: not in the spike
//         .serve_connection(TokioIo::new(pipe), TowerToHyperService::new(router.clone()));
//   select! { _ = conn => {}, _ = shutdown.cancelled() => { conn.graceful_shutdown(); conn.await } }
// On shutdown: acceptors return and drop their instances (the pipe name disappears: new clients get "file not found"),
// then wait up to `grace` (2 s here, 3 s in the spike) for open connections, then `conns.abort_all()`.
```

Rules:
- HTTP/1.1 only. No HTTP/2, no upgrades.
- The acceptors always keep 8 idle instances pending, so a burst of up to 8 new connections never sees `ERROR_PIPE_BUSY`. Tokio's default maximum of 255 instances is left alone. The shell's pool (section 6.10) keeps at most 8 idle connections, so in practice the number of instances stays small.
- The SSE stream must end on shutdown (`take_until` on the cancellation token), because `graceful_shutdown` waits for open requests. Proven: with a stalled client that never reads, the process still exited 2 ms after the request.
- A `CancellationToken` (tokio-util) is the shutdown signal. The spike used a `tokio::sync::watch<bool>` and had to wrap `wait_for` in a helper, because the guard it returns is not `Send` inside `select!` in a spawned task; the token has no such problem.
- Client side (`rain-pipe`, copy `client.rs` of the spike): `open_pipe(name)` loops `ClientOptions::new().open(name)`; on `ERROR_PIPE_BUSY` (231) it sleeps 5 ms and retries until a 3 s deadline (`PIPE_BUSY_RETRY`, `PIPE_BUSY_DEADLINE`; draft 1 said 50 ms times 40, which the 15 ms timer tick makes worse than it looks). `ERROR_FILE_NOT_FOUND` (2) means "backend not running". `ERROR_ACCESS_DENIED` (5) means "another user or a squatter owns the name" and is reported as `unavailable`, reason `pipeDenied`. Not tried: `WaitNamedPipe` instead of the sleep.
- The shell uses ONE cloned `PipeClient` (a pooled `hyper-util` legacy `Client` with a pipe connector, `pool_max_idle_per_host(8)`, `pool_idle_timeout(8 s)`) for all calls, and the stream is one more request on it, run by one background task. Measured (release, SSE stream open meanwhile, 200 requests per run): pooled sequential median 0.030 to 0.037 ms and worst 0.069 to 0.121 ms; pooled 10 at once median 0.04 ms and worst 0.43 to 0.55 ms; connection-per-request 10 at once worst about 15 ms. 10 requests of 300 ms at once finished in 326 to 341 ms. A pool whose idle connections went stale because the backend restarted (graceful stop and hard kill) recovered: 5 concurrent requests after the restart all returned 200. The latency figures are Rust client to Rust server; Tauri IPC adds its own, not measured.
- The `Host` header is required by HTTP/1.1. Clients use the placeholder authority `http://rain{path}`, so the header is `rain`. The server ignores its value.

### 3.4 Limits

| Limit | Value | Where enforced | Over the limit |
|---|---|---|---|
| Pending pipe instances | 8 (`PIPE_PENDING_INSTANCES`) | acceptors (section 3.3) | not a limit: it is what keeps clients from seeing `ERROR_PIPE_BUSY` |
| Concurrent connections | 64 | serve loop | connection closed at once (UNPROVEN) |
| In-flight requests of the shell | 32 (`CLIENT_MAX_INFLIGHT`, a semaphore in the shell) | shell | the 33rd call waits |
| Request header read | 10 s | hyper `header_read_timeout` with `TokioTimer` (UNPROVEN) | connection closed |
| Idle keep-alive connection | 10 s (`HEADER_READ_TIMEOUT`; hyper applies it while it waits for the next request head) | hyper | connection closed; the client pool drops idle connections after 8 s, before the server does |
| Request body | 1 MiB (`BODY_LIMIT`); a route may raise it to 4 MiB and must say so in its section | `tower_http::limit` + extractor | `400 invalid`, reason `bodyTooLarge` |
| Request handling time | 15 s (`REQUEST_TIMEOUT`), SSE route exempt. Handlers aim for 10 s (D13); 15 s is the hard stop | `tower::timeout` | `503 unavailable`, reason `timeout` |
| URL length | 2048 bytes | router layer | `400 invalid` |
| Response size | no hard cap in the backend; the shell refuses more than 16 MiB (`RESPONSE_MAX`) | shell | `failed`, reason `responseTooLarge` |
| Stream frame | 256 KiB (`FRAME_MAX`) | `EventBus::publish` | the frame is dropped, an `error` is logged (this is a programming error) |
| Numbers in JSON | integers must stay within 2^53 - 1 | types | tests assert it for `u64` fields |

Rule D13 again, because it shapes every route: a route that may take longer than 10 s does not wait. It validates, starts the work, answers `201` with an id (or the initial state) and reports progress on a topic, exactly as `startScan` and `launch` already do.

---------------------------------------------------------------------------------------------------

## 4. API conventions

### 4.1 Base path, versioning, naming

- Every route starts with `/api/v1`. All paths in this document and in the feature sections omit that prefix; the bridge and the REST client add it.
- Within v1 only additive changes are allowed: new optional fields, new routes, new enum values the client can ignore. A breaking change creates `/api/v2` next to v1. The routes `/internal/hook` and `/ping` stay compatible for the life of v1, because the hook program is copied into the user's Claude settings and may be older than the backend.
- Route names: lower-case kebab-case; collections are plural nouns (`/pools`, `/idle-jobs`); one item is `/<collection>/{id}`; an action that is not CRUD is `POST /<collection>/{id}/<verb>` (`/sessions/{id}/focus`); a singleton is a singular noun (`/settings`, `/vault`, `/game-mode`, `/machine`).
- Query parameter names are camelCase. A list is comma separated (`types=app,pool`). A boolean is `true` or `false`. Text is UTF-8, percent-encoded by the client.
- Methods: `GET` reads and never changes state. `POST` creates or runs. `PUT` replaces or sets a value by key. `PATCH` changes some fields. `DELETE` removes. `HEAD` and `OPTIONS` are not supported (`405`, JSON error body).
- Path parameters are the item id, percent-encoded.

### 4.2 JSON

| Rule | Value |
|---|---|
| Field names | camelCase (`#[serde(rename_all = "camelCase")]` on every struct and enum) |
| Enum values | camelCase strings (`editorOnly`, `windingDown`) |
| Optional fields | absent, never `null` (`skip_serializing_if = "Option::is_none"`). The only `null` allowed is where the TypeScript type says `\| null` on purpose. |
| Numbers | JSON numbers; integers only unless the field is a fraction; no NaN or infinity |
| Booleans | `true` / `false`, never strings |
| Request bodies | `Content-Type: application/json`; unknown fields are rejected (`deny_unknown_fields` on every request-body type, not on TOML files) |
| Response bodies | a bare object or array exactly as the TypeScript type; no wrapper like `{ "data": ... }` |
| Charset | UTF-8 |
| Response headers | `Content-Type: application/json; charset=utf-8`, `Cache-Control: no-store` |

### 4.3 Dates, times, ids

| Thing | Format | Example |
|---|---|---|
| Instant (`ISODateTime`) | RFC 3339, UTC, always `Z`, always 3 fraction digits | `2026-11-21T09:30:00.000Z` |
| Calendar date (`ISODate`) | `YYYY-MM-DD` in the user's local zone | `2026-11-21` |
| Duration | integer milliseconds (`durationMs`) or seconds (`...Seconds`); the field name says which | |
| Id | opaque string, 1 to 128 characters of `A-Za-z0-9._~-`; the owning module creates it; the client never parses it | `app-vscode`, `run-20261121-0930-ab12` |

Dates in Rust are `String`, produced by `rain_types::time::{now_iso, iso_from_system_time}` (std only; no chrono in the contract; UTC timestamps need no time zone). Local calendar dates (`ISODate`) need the Windows time zone, so they are computed by the backend in `platform::time`: `local_date(SystemTime) -> String` and `local_date_of_unix(i64) -> String`, never by the frontend and never in `rain-types` (it makes no OS call, section 1.2).

Id stability: an id that the user can store (pins, pool members, hidden games) must survive a rescan and a restart. The module that issues it states how (feature section; for apps see decision C3).

### 4.4 Pagination and size

There is no pagination. Lists are returned whole, as today. A route that can return more than about 5000 items, or more than about 1 MiB, must have an explicit budget in its own section (`limit` parameter, a lazy sub-route, or a truncated flag), as the disk scan does (decision C11).

### 4.5 Errors

The body of every non-2xx answer, also `404` for an unknown route and `405`:

```json
{ "code": "notFound", "message": "Game not found", "source": "games", "reason": "unknownId" }
```

`ApiError` (Rust, in `rain-types`; generated to TypeScript as `ApiError`; `RainError` in `api.ts` becomes an alias of it):

| Field | Type | Meaning |
|---|---|---|
| `code` | `'unavailable' \| 'notFound' \| 'failed' \| 'invalid' \| 'conflict'` | the class; the UI may branch on it |
| `message` | string | one plain English sentence for the user; shown as is (`errorMessage()` in the UI) |
| `source` | optional string | the module or data source (`games`, `steam`, `vault`, `backend`) |
| `reason` | optional string | a stable machine tag in camelCase, for tests and for the few UI branches (`requestExpired`, `notStoppable`, `busy`, `sandbox`, `backendDown`) |

HTTP status to `code`:

| Status | `code` | Used for |
|---|---|---|
| 400 | `invalid` | malformed body, unknown field, value outside its allowed set (a number union such as `resetAfterSeconds = 45`), bad query, wrong content type (axum's 400, 415 and 422 all end here) |
| 404 | `notFound` | unknown id or route |
| 405 | `failed` | method not allowed on a known route (a bug in the client) |
| 409 | `conflict` | the request is fine but the current state forbids it (game mode phase, expired permission request, `busy`) |
| 500 | `failed` | the operation failed or a handler panicked |
| 503 | `unavailable` | a source or the backend part is off or not ready (Steam not installed, shutting down, timeout) |

The shell and the browser bridge map transport failures to the same shape: pipe missing, refused, reset, or timed out becomes `unavailable` with `source: "backend"` and the reason of section 6.6. A response that is not valid JSON becomes `failed`, reason `badResponse`.

Rules for messages: write for the user in plain words ("Could not read the Steam library"); never include file contents or note text; never include a raw OS error alone (add what Rain was doing); a configured path may appear. Rules for the server side: handlers never build an error from `format!("{e}")` of a library error without saying what failed.

Rust (module `core::error`):

```rust
pub struct ApiError { pub status: StatusCode, pub body: rain_types::ApiError }
impl ApiError {
    pub fn invalid(msg: impl Into<String>) -> Self;
    pub fn not_found(msg: impl Into<String>) -> Self;
    pub fn conflict(reason: &'static str, msg: impl Into<String>) -> Self;
    pub fn unavailable(source: &'static str, msg: impl Into<String>) -> Self;
    pub fn failed(msg: impl Into<String>) -> Self;
    pub fn with_source(self, s: &'static str) -> Self;
    pub fn with_reason(self, r: &'static str) -> Self;
}
impl IntoResponse for ApiError { ... }                // writes the JSON body
pub type ApiResult<T> = Result<T, ApiError>;
// There is NO blanket `From<io::Error>`/`From<rusqlite::Error>`: the handler must say what failed.
```

Proven (spike types-stream, `api_error.rs`; copy it, then change the three points below). Every one of these produced the one body with the right status: a handler error, a `?` on an `io::Error`, a bad JSON body, a missing field, a bad query value, an unknown route, a wrong method, a handler panic (the connection survived), and a settings value outside its literal union.

```rust
#[derive(FromRequest)]
#[from_request(via(axum::Json), rejection(ApiError))]
struct ApiJson<T>(T);                       // body extractor
#[derive(FromRequestParts)]                 // NOT FromRequest: `Query` is a parts extractor and the derive does not compile
#[from_request(via(axum::extract::Query), rejection(ApiError))]
struct ApiQuery<T>(T);
#[derive(FromRequestParts)]                 // path segments, same rejection
#[from_request(via(axum::extract::Path), rejection(ApiError))]
struct ApiPath<T>(T);                       // `ApiPath` sits beside `ApiJson` and `ApiQuery`; sections use it for `{id}` segments
// router: .fallback(fallback_404)                                   -> 404 notFound, reason noRoute
//         .method_not_allowed_fallback(fallback_405)                -> 405 failed,   reason methodNotAllowed
//         .layer(CatchPanicLayer::custom(panic_response))           -> 500 failed,   reason panic, message "Internal error"
```

Changes against the spike: (1) there is no blanket `From<io::Error>` (the spike mapped `NotFound` to 404 and the rest to 500; the foundation forbids it, see the Rust block above); (2) the spike kept the 400, 415 and 422 that axum produces; here all three become `400 invalid`, with reason `badBody`, `badQuery` or `badContentType`, because a client cannot act on the difference; (3) the rejection text of axum contains serde wording ("invalid value: 45, expected one of: 30, 60, 300" was seen verbatim), so `From<JsonRejection>` writes the message as "The request was not valid: <text>" and never echoes the body. The UI shows `message` as is.

The client side (`rain-pipe`, `api_request`): a non-2xx answer whose body parses as the error body is used as is; otherwise the code comes from the status (404 `notFound`; 502, 503 and 504 `unavailable`; the rest `failed`) with reason `badResponse`; a transport failure is `unavailable` with the reasons of section 6.6. A 304 is not an error.

### 4.6 Status codes of success and idempotency

| Case | Status | Body |
|---|---|---|
| Read | 200 | the resource |
| Create (`POST` that makes a new resource: launch, scan, run, pool) | 201 | the new resource; no `Location` needed |
| Action or set that returns the new state | 200 | the state |
| Action with nothing to return (`Promise<void>` in `RainApi`) | 204 | none |

The REST client treats 200, 201 and 204 as success; 204 resolves to `undefined`.

Idempotency: `GET`, `PUT` and `DELETE` are idempotent; `PATCH` is idempotent for equal values; `DELETE` of something already gone returns `204` (not `404`) unless the section says otherwise. `POST` is not idempotent by default. There are no idempotency keys. A `POST` that the UI may send twice by double click (`launch`, `startScan`, `startGameMode`) must itself be safe: the section for it states what a second call does (usually: returns the run that is already in progress). Every route table in a feature section has a column "Idempotent".

### 4.7 Binary resources (icons, covers)

JSON never carries image bytes. A binary resource has a reference and a route:

- Route: `GET /media/{kind}/{id}` where `kind` is registered by one module (`app-icon`, `game-cover`, `project-icon`, ...) and `id` may contain `/`? No: `id` is one path segment. If a module needs more parts it encodes them into the id.
- The answer is `200` with the bytes, `Content-Type` of the image, `Content-Length`, `X-Content-Type-Options: nosniff`, an `ETag`, and `Cache-Control: public, max-age=31536000, immutable` when the query has `v=<version>`, else `private, max-age=86400` (the value the spike measured; `immutable` was not tested, it is only a header). When the query holds `v`, the ETag is the quoted text `kind-id-v`, computed by the route without calling the provider; a matching `If-None-Match` returns `304` at once. Without `v` the provider is called and its `Media.version` is used. A request with a matching `If-None-Match` gets `304` with the same headers, and the check happens BEFORE the bytes are produced: the spike's first version encoded the PNG first and the 304 took 46 ms; after the reorder it took 0.06 to 0.10 ms. A missing image is `404` with the JSON error body, `Content-Type: application/json` (the `<img>` then shows its fallback; screens already draw a letter tile); a client must look at the status before it treats the body as an image. Measured: a 64 px icon 173 bytes in 0.8 ms including the encoding; a 208 KB cover in under 1 ms from cache; 100 cached covers (20.8 MB) in 24 ms, about 870 MB/s over the pipe. Copy `icons.rs` of the spike (`png_response`).
- Reference in JSON: a scheme-less string relative to the API base, starting with `media/`, for example `media/app-icon/app-vscode?v=3fa9`. The `v` value is a content version (a short hash or the file mtime in hex) chosen by the module, so a new icon changes the reference and the cache works.
- Where it appears: `IconRef { kind: 'url', value: 'media/...' }` and `Game.coverUrl` (and similar fields that a section adds, named `...Url`).
- The frontend REST client rewrites these strings to a loadable URL (section 8.4): in Tauri `http://rain-media.localhost/media/...`; in the browser `/api/v1/media/...`. No screen changes.
- Providers register in core, one line per kind:

```rust
pub trait MediaProvider: Send + Sync {
    /// Returns None for an unknown id. Runs on a blocking thread when it reads files.
    fn get(&self, id: &str) -> BoxFuture<'_, ApiResult<Option<Media>>>;
}
pub struct Media { pub bytes: Bytes, pub content_type: &'static str, pub version: String }
state.media.register("app-icon", Arc::new(AppIcons::new(..)));
```

Why the shell asks the backend and does not read the cache folder itself (decision C4): the backend decides what may be read; the shell never learns the cache layout.

---------------------------------------------------------------------------------------------------

## 5. The live stream

### 5.1 Route and framing

`GET /stream`. The answer is `200`, `Content-Type: text/event-stream`, `Cache-Control: no-store`, a never-ending body. Each frame:

```
event: sessions
id: 42
data: {"topic":"sessions","seq":42,"payload":[ ... ]}

```

The handler is the one proven in spike types-stream (`routes.rs`, `events`), shown in section 5.7. The `data` line is one JSON object, the envelope (type `Frame` in `rain-types`):

| Field | Type | Meaning |
|---|---|---|
| `topic` | string | the topic name (section 5.3) |
| `seq` | number | one counter per backend process, +1 for every frame published to all clients; starts at 1 |
| `payload` | any JSON | the full snapshot defined by the topic |

The `event:` line repeats `topic` and `id:` repeats `seq`, only so that a human can read the stream with a tool (axum `Event::event(..)`, `.id(..)`, `.data(..)`). The shell and the browser read only the `data` line. The reconnect delay `retry: 2000` is set on the `hello` event with `Event::retry(2 s)` (a stand-alone `retry:` preamble is not possible with axum's `Sse`; the setting only matters to the browser's `EventSource`, the Rust reader has its own backoff).

### 5.2 The first frames of every connection

1. `hello`: `{ startId, version, apiVersion: 1, seq }` where `startId` is an opaque string, unique per backend process start (`format!("{unix_ms:x}-{pid:x}")`). `seq` is the current counter, so the client sees where the process is. This frame is written only to the new client and has `seq` equal to the current counter.
2. `resync` with payload `{ "reason": "connect" }`: the client must refetch everything it shows. (On lag the payload is `{ "reason": "lagged", "missed": <count> }`, section 5.4.)

Proven: every connection began with `hello` carrying a new `startId` and then `resync`, after a graceful stop and after a hard kill (`Stop-Process`); the client reconnected with a backoff of 100 ms doubling to 2 s.

There is no replay. `Last-Event-ID` is ignored. The stream never says what was missed; it says "refetch" (decision C9).

### 5.3 Topics

A topic is defined once in `rain-types`, which ties the name to the payload type, so a publisher cannot send the wrong shape:

```rust
pub trait Topic { const NAME: &'static str; type Payload: Serialize; }
pub struct Sessions;  impl Topic for Sessions { const NAME: &'static str = "sessions"; type Payload = Vec<ClaudeSession>; }
// publish:  state.bus.publish::<Sessions>(&sessions);
```

Where the Topic structs live and how they are named (one rule for every section): ALL Topic structs are in `crates/rain-types/src/events.rs`, never in the feature file of their payload. The struct has the name of the topic in PascalCase: `Launch`, `Scan`, `Projects`, `Games`, `Jobs`, `IdleJobs`, `Habits`, `Sessions`. Where the name would clash with the payload type, the suffix `Topic` is used, and it is used for all three of these: `SettingsTopic` (payload `Settings`), `HealthTopic` (payload `Health`) and `GameModeTopic` (payload `GameModeState`). `bus.publish::<T>` and `topic_names_ts()` use these names. A section adds its struct to `events.rs` in the stage that adds its payload type (section 14.1, stage 1a item 2).

Existing topics (they all carry a complete snapshot; this is a rule for every topic):

| Topic | Payload | Owner section | Client matches on |
|---|---|---|---|
| `sessions` | `ClaudeSession[]` (whole list) | 03 | none |
| `jobs` | `ScheduledJob[]` (whole list) | 03 | none |
| `idleJobs` | `IdleJob[]` (whole list) | 03 | none |
| `launch` | `LaunchRun` (one run) | 01 (and 02 for projects and games) | `run.id` |
| `scan` | `DiskScan` (one scan) | 05 | `scan.id` |
| `gameMode` | `GameModeState` | 06 | none |
| `projects` | `Project[]` (whole list), at most 1 per second | 02 | none |
| `games` | `GameLibrary` (whole object), at most 1 per second | 02 | none |
| `habits` | `HabitSummary[]` (whole list, the same value as `GET /habits`), at most 4 per second | 04 | none |
| `settings` | `Settings` (whole object), at most 1 per second | 06 | none |

Topics added by this foundation: `health` (payload `Health`, section 6.9). Internal frames, not in `TopicPayload` and not for subscribers: `hello`, `resync`, `lifecycle` (payload `{ kind: "stopping", reason: "gameMode"|"stop"|"upgrade"|"restart" }`, the last frame before the stream closes).

Designers may add topics in their sections. A new topic needs: a name (camelCase, one word if possible), the Rust `Topic` struct, the payload type, "whole list" or "one item and its key", the rate (frames per second at most), and a row in the contract-change table (section 15). Further examples a section may need: `machine`, `apps`, `vault`. Rules:
- A payload is a snapshot of one resource, never a patch.
- A topic that can change more than 4 times a second must be throttled by its publisher (`core::throttle::Throttle`, at most 4 per second, the newest value wins).
- A payload over 256 KiB (`FRAME_MAX`) is not allowed. If a snapshot can be that big, the topic carries a small notice (id, status, counters) and the data is fetched by a route (the disk tree is fetched by `GET /scans/{id}` and its node route).
- No frames are published while no client is connected (the bus checks `receiver_count()`), but a module's own state still updates.

### 5.4 Keep-alive, lag, shutdown

- Keep-alive: axum `KeepAlive::new().interval(25 s).text("ka")`, a comment line `: ka`, per connected client (`SSE_KEEPALIVE`). Proven: the comment appears only when the stream is idle (every event resets the timer); seen at 2012, 4024 and 6034 ms with a 2 s setting. The client treats 60 s without any byte as a dead connection (`STREAM_DEAD_AFTER`) and reconnects. Proven: a client whose idle limit (1 s) was shorter than the keep-alive (2 s) reconnected every second.
- Bus: one `tokio::sync::broadcast` channel with capacity 64 (`BUS_CAPACITY`; draft 1 had 256, but with the 256 KiB frame limit 256 frames could hold 64 MiB, while 64 frames hold at most 16 MiB; the spike proved 64). Frames are serialised once and shared (`Arc<Bytes>`). A client that falls behind gets `Lagged(n)`; the stream then writes a `resync` frame with `{ "reason": "lagged", "missed": n }` instead of dropping silently, and continues with the newest frames. Proven: a client that stopped reading while 2000 events were published received `resync` with `missed: 1936` and then the 64 newest events.
- `Throttle::publish_final(value)` cancels the pending throttled value and publishes `value` at once (a run that ends must not be followed by a stale progress frame). `EventBus::has_receivers()` tells a publisher whether anyone listens; `EventBus::freeze()` stops all further topic frames (game mode uses it after `done`, so `done` stays the last topic frame).
- Shutdown: the stream writes the `lifecycle` frame and ends (section 2.6). The spike called this frame `shutdown`; its purpose is the same: it tells a planned stop from a crash.

### 5.5 What the frontend does

| Event | Client action |
|---|---|
| `hello` | remember `startId`. If it differs from the previous one (a different backend process): `restarted = true`. |
| `resync` (always follows `hello`; also on lag) | call every registered reset callback with `{ restarted }`. `src/main.ts` registers one: if `restarted` then `resetLaunches()` and `resetDisk()`; then always `refreshResources()`. |
| a topic frame | deliver `payload` to the subscribers of that topic. |
| stream lost | state `down`; reconnect with backoff 0.5 s, 1 s, 2 s, then every 5 s; no limit. The next `hello` triggers the refetch. |
| `lifecycle` | remember the reason (used only to avoid flashing an error in the one case of game mode `done`). |

The stream stays open while the window is hidden (decision C9, cheap, and `summon` already refetches). The frontend never closes it on `rain:hidden`.

### 5.6 How `subscribe()` of `RainApi` maps

`subscribe(topic, cb)` stays synchronous and returns the unsubscribe function. The stream client keeps `Map<Topic, Set<callback>>`. One global listener is installed once at start, before the first `subscribe` can run, so there is no "listen is asynchronous" race: `subscribe` only adds to the map. In Tauri the listener is `listen('rain:stream')`; in the browser it is the `EventSource`. A callback runs for every frame of its topic, in order. No filtering by id: callers filter, as with the mock.

### 5.7 The proven server handler and reader

```rust
// crates/rain-backend/src/server/stream.rs   (spike types-stream: routes.rs `events`)
async fn stream(State(s): State<AppState>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = s.bus.subscribe();                       // subscribe BEFORE building the head frames: nothing is missed
    let stop = s.shutdown.token().clone();
    let head = stream::iter([hello_event(&s), resync_event("connect")]);   // `hello` is written to this client only
    let live = BroadcastStream::new(rx).map(|item| match item {
        Ok(frame)                                  => Event::default().event(frame.topic).id(frame.seq).data(frame.json),
        Err(BroadcastStreamRecvError::Lagged(n))   => resync_event_lagged(n),
    });
    let tail = stream::once(async { lifecycle_event_for(&s) });          // the last frame: planned stop, with its reason
    Sse::new(head.chain(live.take_until(stop.cancelled_owned())).chain(tail).map(Ok))
        .keep_alive(KeepAlive::new().interval(SSE_KEEPALIVE).text("ka"))
}
```

The reader (`rain-pipe`, copy `sse.rs` of the spike): `SseReader::next()` takes a complete frame (blank-line terminated; it handles comment lines, `event:`, multi-line `data:` and CRLF) from a buffer, else awaits `body.frame()`. `run_stream(client, path, idle_limit, stop, on_notice)` reconnects forever with a backoff that resets on any frame, drops a silent connection after `idle_limit` (`STREAM_DEAD_AFTER`), and reports `Connecting`, `Connected`, `Frame` and `Disconnected { reason }` to a callback (the shell turns these into `app.emit` calls). Observed disconnect reasons: a planned stop is the final frame and then "stream closed by the server"; a hard kill is "unexpected EOF during chunk size line"; no backend is "os error 2". The backoff in this foundation is the one of section 5.5 (0.5, 1, 2, then 5 s); the spike used 0.1 to 2 s.

---------------------------------------------------------------------------------------------------

## 6. The Tauri bridge

### 6.1 Commands

One command (decisions.md section 2: "one generic command"), registered in `lib.rs` with `invoke_handler`. Draft 1 had a second command, `backend_status`; it is folded into `api_request` through a shell-local path (below), so the allow-list has one entry. UNPROVEN as a whole: the spike measured Rust client to Rust server only, not the Tauri command, the event relay or the webview IPC (appendix C); stage 1b proves them first.

```rust
// src-tauri/src/bridge.rs
#[tauri::command]
async fn api_request(
    backend: tauri::State<'_, Backend>,   // holds the one cloned `rain_pipe::PipeClient` (section 6.10)
    method: String,                  // "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
    path: String,                    // after /api/v1, starts with "/", may carry "?query"
    body: Option<serde_json::Value>, // JSON body or None
    timeout_ms: Option<u64>,         // default 20_000, maximum 60_000
) -> Result<serde_json::Value, rain_types::ApiError>;
```

Shell-local path: `GET /local/backend` is answered by the shell itself and never sent to the pipe. The answer is `{ "state": "starting"|"up"|"down"|"stopped"|"restarting", "startId"?: string }` (the state of section 2.1). `tauri-bridge.ts` calls it once, right after it has installed its `listen('rain:stream')` and `listen('rain:backend')`, and turns an `up` answer into a `resync`, so a page that loads after the relay started does not miss the state. The browser bridge has no such path and needs none (its `EventSource` receives `hello` at once). `/local/` is a reserved prefix of the shell; the backend has no route that starts with it.

JavaScript side (argument keys are camelCase, Tauri converts to the Rust snake_case names):

```ts
invoke<unknown>('api_request', { method: 'GET', path: '/settings', body: undefined, timeoutMs: undefined })
```

Serialisation:
- Success: the JSON body of the answer as a `serde_json::Value`; an empty body (204) becomes `null`. The page receives the parsed value. Raw bytes (`tauri::ipc::Response`) are a later optimisation, not needed.
- Failure: the promise rejects with a JSON object that is exactly `rain_types::ApiError`. The shell builds it: if the backend answered a non-2xx status with a valid error body, that body is passed through unchanged; otherwise the shell synthesises one (section 6.6). The frontend therefore always receives a `RainError`.
- Validation in the shell before anything is sent: `method` is in the allow-list; `path` starts with `/`, is at most 2048 bytes, contains no `..`, no `\`, no `//`, no `://`, no `#`, no control characters, and does not start with `/internal/`; a path that starts with `/local/` is answered by the shell (only `GET /local/backend` exists, anything else under it is `404 notFound`); `timeout_ms` is clamped. A violation rejects with `invalid`, reason `badRequest`. The shell never sees the pipe name in a value from the page.
- Concurrency: calls run in parallel on the pooled client of section 6.10 (the pool opens one pipe connection per request in flight and reuses idle ones); at most 32 are in flight (`CLIENT_MAX_INFLIGHT`, a semaphore).

### 6.2 The stream relay

`src-tauri/src/relay.rs` owns one task for the life of the shell: it runs `rain_pipe::run_stream` (section 5.7) on the shared client, parses frames, and calls `app.emit("rain:stream", frame_json)` for every frame (the `Frame` object as is). It also tracks `lifecycle` and `hello` for the state machine of section 2.1. Events to the page:

| Event | Payload | When |
|---|---|---|
| `rain:stream` | `Frame` | every frame from the backend, including `hello`, `resync`, `lifecycle` |
| `rain:backend` | `{ state: 'starting'\|'up'\|'down'\|'stopped'\|'restarting', startId?: string }` | every change of the state of section 2.1 |
| `rain:summoned`, `rain:hidden` | unchanged | as today |

Tauri events are untyped JSON and not designed for high throughput; the frame size rule (256 KiB) and the throttling rule keep them well inside that. Events are not governed by capabilities, but `listen` needs `core:event:default`, which `core:default` includes (to be confirmed in stage 1).

A page that loads after the relay started has missed earlier frames. That is safe because `main.ts` always starts with fresh fetches, and because the page asks `GET /local/backend` once at start (section 6.1) and treats an `up` answer like a `resync`.

Reconnect rule of the relay: when the stream ends, look at the last `lifecycle` frame: reason `gameMode` leads to `stopped` and the exit sequence (section 2.6); `stop` leads to `stopped` and an idle relay that wakes up only when `ensure_backend()` is called; `upgrade` or `restart` leads to `restarting`; no frame means crash, so `down`, then `ensure_backend()` through the spawn guard, then reconnect with backoff 0.5 s doubling to 5 s. The relay uses no periodic timer except the 60 s read timeout of the open stream.

### 6.3 The custom URI scheme for binary resources

- Scheme name: `rain-media`. Registered in `lib.rs` with `register_asynchronous_uri_scheme_protocol("rain-media", handler)`.
- Handler: accepts only `GET`; takes the request path (`/media/app-icon/app-vscode`) and query; refuses anything whose path does not start with `/media/`; asks the backend `GET /api/v1<path>?<query>` through `rain-pipe`; answers with the bytes and the content type; on any error answers `404` with an empty body (an `<img>` just shows its fallback). It copies `Content-Type`, `Cache-Control` and `ETag` as the backend sent them, forwards `If-None-Match`, and passes a `304` through (the spike's handler recipe: rebuild the response from the raw answer, look at the status before treating the body as an image). It never reads a file itself.
- URL the page uses on Windows: `http://rain-media.localhost/media/...` (Tauri 2 form for custom protocols on Windows when `useHttpsScheme` is false; read in `tauri-2.12.7/src/webview/mod.rs`). The window config key `useHttpsScheme` is left at its default `false` and must never be changed: it moves the webview's localStorage and IndexedDB, and the UI keeps selections there (`persistKey`). The URL form is built in one function, `mediaUrl(ref)` in `src/shell/tauri-bridge.ts`.
- Whether an `<img>` served from the dev page `http://localhost:1420` loads this scheme without a CSP entry is UNVERIFIED (the CSP is `null`, so probably yes); checked in stage 4 when the first icon appears. If it fails, the CSP gets `img-src 'self' data: http://rain-media.localhost`.

### 6.4 Capabilities

- Commands defined by the app and registered with `invoke_handler` are allowed for every window unless a command manifest is declared. Decision: declare it, so the allow-list is explicit. `src-tauri/build.rs`:

```rust
fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new().commands(&["api_request"]),
        ),
    ).expect("tauri build failed");
}
```

  and `capabilities/default.json` gains `"allow-api-request"`. (The API exists in tauri-build 2.7.6; the exact permission name is confirmed in stage 1b. If the manifest causes trouble, the fallback is to leave `build.rs` unchanged and rely on the default allow, with a note in the stage report.)
- No other permissions are added. No `shell:*` permission, no `fs:*` permission, no `http:*` permission.
- The custom scheme and events need no capability entry.

### 6.5 Hotkey and shell facts

The shell keeps registering the hotkey (decision C6). After every `hello` it reports its facts: `PUT /internal/shell` with

```json
{ "hotkeyRegistered": true, "hotkeyError": "optional text", "shellExe": "C:\\...\\rain.exe", "shellVersion": "0.1.0", "windowVisible": false }
```

The backend keeps this in memory (`state.shell_facts`, an `Option<ShellFacts>` that is `None` until the first report; not persisted). `GET /settings` merges `hotkeyAvailable` from it (`true` when no shell ever reported, so a plain browser shows no false warning). The shell repeats the call when a fact changes (hotkey re-registered).

### 6.6 Errors the shell creates itself

| Situation | `ApiError` |
|---|---|
| pipe missing after `ensure_backend` | `{ code: 'unavailable', message: 'Rain backend is not running', source: 'backend', reason: 'backendDown' }` |
| spawn guard tripped | same with reason `backendCrashLoop` and message "Rain backend keeps stopping. See the log." |
| backend exe missing next to the shell | reason `backendMissing`, message "Rain backend program was not found" |
| timeout of the call | `unavailable`, reason `timeout`, message "Rain backend did not answer in time" |
| connection reset or other transport error | `unavailable`, reason `transport` |
| access denied on the pipe | `unavailable`, reason `pipeDenied` |
| state `stopped` for game mode | `unavailable`, reason `stopped`, message "Rain has stopped" |
| answer not JSON | `failed`, reason `badResponse` |
| answer larger than 16 MiB | `failed`, reason `responseTooLarge` |

### 6.7 The backend is not running

`api_request` first tries the pipe. On "file not found" it calls `ensure_backend()` once (which may spawn), then retries the call once. If that fails it rejects with the error of the table above; the screens show their error row with Retry (already built). Retry calls `api_request` again, which calls `ensure_backend()` again, which respects the spawn guard. `subscribe` stays silent meanwhile.

### 6.8 Shell code layout

```
src-tauri/src/
  main.rs        unchanged
  lib.rs         builder: plugins, setup, window events, invoke_handler, uri scheme; hotkey code stays here
  backend.rs     Backend state machine, ensure_backend, spawn, upgrade rule, spawn guard
  bridge.rs      api_request (and the shell-local GET /local/backend), validation, error mapping
  relay.rs       stream reader task and `rain:*` events, lifecycle handling, exit sequence
  media.rs       rain-media handler
  facts.rs       PUT /internal/shell reporting
  logging.rs     tracing to <local>\logs\shell.log
```

### 6.9 `Ping` and `Health`

`GET /ping` (no side effects, always cheap, answered even while `shutting_down`):

```json
{ "app": "rain-backend", "version": "0.1.0", "apiVersion": 1, "startId": "19a3f1c2-1f4c",
  "pid": 12345, "exePath": "C:\\...\\rain-backend.exe", "sandbox": false, "busy": false }
```

`busy` is true while any scheduled job, idle job, scan or launch is running (each module reports through `state.activity`).

`GET /health` (and topic `health`): `{ startId, version, shell: { reported: boolean, hotkeyRegistered?: boolean }, configIssues: ConfigIssue[], sources: SourceStatus[] }` where `ConfigIssue = { file, line?, message }` (a TOML file that failed to parse; section 10.4) and `SourceStatus = { name, state: SourceState, message? }` where `SourceState` is the one named Rust enum `pub enum SourceState { Ok, Off, Failed }` in `system.rs` (serialises as `'ok'|'off'|'failed'`; section 02 imports it for `GameSourceStatus` and does not define its own) (a module reports "steam: off, not installed" with `state.health.set_source(...)`; this is the one shared place for per-source status; a section may additionally put a status into its own list answer).

### 6.10 The shell's pipe client

One `rain_pipe::PipeClient` is created at start, stored in the shell's managed state (inside `Backend`) and cloned everywhere: `api_request`, the media handler, `facts.rs`, the relay, and `ensure_backend` (its `GET /ping` uses the same client with a 1 s timeout). It is a hyper-util legacy `Client` built as `Client::builder(TokioExecutor::new()).pool_max_idle_per_host(8).pool_idle_timeout(8 s).build(PipeConnector)`. `PipeConnector` implements `tower::Service<Uri>` and returns `PipeIo`, a wrapper that implements `hyper::rt::Read`, `hyper::rt::Write` and `hyper_util::client::legacy::connect::Connection` by delegation to `TokioIo<NamedPipeClient>`. Requests use the URI `http://rain{path}`. Copy `client.rs` of the spike. Proven: idle connections that went stale because the backend restarted did not fail the next calls (spike types-stream, "stale pool" runs). UNPROVEN: a non-idempotent `POST` that lands on a connection that died in the instant before the send; the pool may report a transport error. `api_request` retries once only when the pipe could not be opened at all (the backend was not running), never after a request was written; the REST client never retries (section 8.3).

---------------------------------------------------------------------------------------------------

## 7. The browser development bridge

Purpose: `npm run dev` opens `http://localhost:1420` in a normal browser, and the page must reach the same backend. A browser cannot open a pipe and cannot call `invoke`. The Vite dev server (Node, which can connect to a Windows pipe with `socketPath`) proxies `/api` to the pipe. This exists only in `vite serve`; the plugin has `apply: 'serve'`, so `vite build` never contains it.

File: `vite/pipe-proxy.ts`, exported as `rainPipeProxy()` and added to `plugins` in `vite.config.ts`.

Behaviour for every request whose path starts with `/api/v1/`:

1. Guard (any failure returns `403` with an `ApiError` body, code `failed`, reason `forbidden`):
   - `Host` must be exactly `localhost:1420` or `127.0.0.1:1420`.
   - If `Origin` is present it must be exactly `http://localhost:1420` or `http://127.0.0.1:1420`.
   - If `Sec-Fetch-Site` is present it must be `same-origin` or `none`.
   - The path must not start with `/api/v1/internal/`.
   - A method other than `GET` must carry the header `X-Rain-Dev: 1`. A foreign page cannot add a custom header without a CORS pre-flight, and the proxy does not answer pre-flights.
2. Forward with `http.request({ socketPath: '\\\\.\\pipe\\' + pipeName, method, path, headers: { host: 'rain', 'content-type': ..., 'content-length': ... } })`. `Origin`, `Cookie` and `X-Rain-Dev` are not forwarded.
3. Stream the answer back unchanged (no buffering, so SSE works); when the browser closes the request, destroy the upstream request; when the pipe closes, end the response.
4. If the pipe does not exist (`ENOENT`) or refuses: answer `503` with `{ code: 'unavailable', message: 'Rain backend is not running. Start it with: npm run backend:start', source: 'backend', reason: 'backendDown' }`.

Pipe name in the proxy: `process.env.RAIN_PIPE`, else `rain-dev-<user>` (the dev server is always sandbox).

The browser bridge (`src/shell/browser-bridge.ts`) calls `fetch('/api/v1' + path, { method, headers, body, signal: AbortSignal.timeout(20_000) })` with `X-Rain-Dev: 1` on non-GET, maps failures to `ApiError` with the rules of section 6.6, and builds media URLs as `/api/v1/` + ref. Its stream is an `EventSource('/api/v1/stream')`; because an `EventSource` gives up for good after a non-200 answer (the proxy's `503`), the bridge recreates it after the backoff of section 5.5 when `readyState` is `CLOSED`.

How the frontend picks the bridge: `src/shell/index.ts` already picks the Tauri or the browser shell with `isTauri`. The `Shell` interface gets one more member, `readonly bridge: Bridge` (section 8.2); `tauri.ts` builds it from `tauri-bridge.ts`, `browser.ts` from `browser-bridge.ts`. No other file chooses. The rule in `ui-spec.md` 2.7 stays true: only `src/shell/tauri*.ts` imports `@tauri-apps/api`.

---------------------------------------------------------------------------------------------------

## 8. The frontend data layer after the mock

### 8.1 File layout

```
src/
  bindings/                    GENERATED by `npm run types`; committed; never edited by hand
    rain-types.ts              ONE file: every exported type, plus `TOPIC_NAMES` (section 9.3)
  shell/
    shell.ts                   + `bridge: Bridge`
    bridge.ts                  NEW: the Bridge interface and its small types (no Tauri import)
    tauri.ts                   + `bridge` from tauri-bridge
    tauri-bridge.ts            NEW: invoke('api_request'), listen('rain:stream'), rain-media URL
    browser.ts                 + `bridge` from browser-bridge
    browser-bridge.ts          NEW: fetch + EventSource
  data/
    api.ts                     RainApi interface (the contract). Edited by the feature sections.
    types.ts                   barrel: re-exports ../bindings/rain-types, the three aliases, and (until stage 10) the hand-written types that are not ported yet
    index.ts                   chooses the api (section 8.5)
    rest/
      client.ts                createRestApi(bridge): RainApi   (one function per method; stubs for the rest)
      errors.ts                toRainError(), status and code rules
      media.ts                 resolveMedia(): rewrites `media/...` strings in answers
      stream.ts                createStream(bridge): subscribe(), onStreamReset(), state
    router.ts                  TEMPORARY: sends each method to real or mock (section 8.6)
    routing.ts                 TEMPORARY: the table that says which stage routes what
    mock/                      the existing mock; deleted in stage 10
```

### 8.2 The Bridge interface

```ts
// src/shell/bridge.ts  (no import from tauri, no import from data)
export type HttpMethod = 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE'
export interface BridgeError { code: string; message: string; source?: string; reason?: string }
export interface StreamFrame { topic: string; seq: number; payload: unknown }
export type BackendState = 'starting' | 'up' | 'down' | 'stopped' | 'restarting'
export interface Bridge {
  /** Resolves with the parsed JSON (undefined for 204). Rejects with a BridgeError (a RainError). */
  request(req: { method: HttpMethod; path: string; body?: unknown; timeoutMs?: number }): Promise<unknown>
  /** One stream for the life of the page. Returns a stop function. */
  openStream(h: { onFrame(f: StreamFrame): void; onState(s: BackendState): void }): () => void
  /** `media/app-icon/x?v=1` -> a URL the page can load. */
  mediaUrl(ref: string): string
}
```

`BridgeError` has the same shape as `RainError`; the data layer uses it as one. `Shell` gets `readonly bridge: Bridge`.

### 8.3 The REST client

`createRestApi(bridge: Bridge): RainApi` is one object with one function per `RainApi` method. Each function is one line on top of a private `call<T>(method, path, body?, opts?)`:

```ts
getSettings: () => call<Settings>('GET', '/settings'),
updateSettings: (patch) => call<Settings>('PATCH', '/settings', patch),
setPinned: (type, id, pinned) => call<void>('PUT', `/pinned/${type}/${enc(id)}`, { pinned }),
```

Rules:
- Paths come from the route registry (section 14.4); the client adds nothing else.
- `enc(id)` is `encodeURIComponent`.
- Query strings are built by one helper `qs({ q, types })` that drops `undefined` and joins arrays with commas.
- Every method that is not yet real is a stub that rejects with `{ code: 'unavailable', message: 'Not connected yet', source: 'client', reason: 'notImplemented' }`. The router never calls a stub; a stage replaces the stubs of its routes.
- `call` resolves media references with `resolveMedia(value)` unless the call passes `{ media: false }` (disk scans and logs, which are large and have no media).
- `call` does not retry. Retries are the UI's (Retry button), as today.

Error mapping (`errors.ts`): an object with `code` and `message` passes through; anything else becomes `{ code: 'failed', message: <text> }`. The union `RainError['code']` is the generated `ApiError['code']`.

### 8.4 Media references

`resolveMedia` walks a parsed answer once and replaces a string that starts with `media/` and sits in a field named `coverUrl`, or in `value` of an object whose `kind` is `'url'`, with `bridge.mediaUrl(ref)`. Objects and arrays are walked iteratively; strings elsewhere are untouched. Reason for one central rewrite: no screen needs to know where images come from.

### 8.5 `src/data/index.ts`

```ts
import { shell } from '@/shell'
import { createRestApi } from './rest/client'
import { createStream } from './rest/stream'
// stage 1b..9:
import { createMockApi } from './mock'
import { createRouterApi } from './router'
import { routing } from './routing'

const stream = createStream(shell.bridge)
const real = createRestApi(shell.bridge, stream)
export const api: RainApi = import.meta.env.VITE_RAIN_MOCK === '1'
  // Retired methods are undefined here; their screens show an error row; stage 10 deletes this branch.
  ? (createMockApi() as RainApi)
  : createRouterApi({ real, mock: createMockApi(), routing })   // `mock` is typed `MockApi`
export const onStreamReset = stream.onReset
```

In stage 10 the file becomes `export const api: RainApi = createRestApi(shell.bridge, stream)`, and `router.ts`, `routing.ts`, `mock/` and the `VITE_RAIN_MOCK` branch are deleted.

### 8.6 The migration router

`createRouterApi` returns a `RainApi` in which every method forwards to `real` or `mock`, decided by `routing`:

```ts
// src/data/routing.ts  (the ONLY file a stage edits to move a method)
export type Side = 'real' | 'mock'
export const routing: { methods: Record<keyof Omit<RainApi, 'subscribe'>, Side>; topics: Record<Topic, Side> } = {
  methods: { search: 'mock', listPinned: 'mock', /* ... one line per method ... */ getSettings: 'real', updateSettings: 'real' },
  topics: { sessions: 'mock', jobs: 'mock', idleJobs: 'mock', launch: 'mock', scan: 'mock', gameMode: 'mock', health: 'real' },
}
```

The type `Record<keyof RainApi, Side>` makes a missing method a compile error. `subscribe(topic, cb)` goes to the side given by `topics[topic]`, except `launch` (table below).

How the mock keeps satisfying `RainApi` when slices are deleted (decided here; sections 01, 02, 03, 04, 05 and 06 follow it): in `src/data/mock/index.ts` the type is `type MockApi = Omit<RainApi, RetiredMockMethod>`, where `RetiredMockMethod` is a union of method names. Rule: every stage adds to `RetiredMockMethod` every method that the mock no longer implements at the end of that stage. That covers the methods of the slices the stage deletes AND the new `RainApi` methods that the mock never had. The two types are CREATED in stage 2 (section 06 9.2 step 2 item 4, with `'getHealth'`, because stage 2 adds `getHealth` to `RainApi`); stage 3 (section 05) only extends them, together with the deletion of `mock/slices/disk.ts` and `mock/seed/disk.ts` (no stub is left behind). `createMockApi()` is declared to return `MockApi` (not `RainApi`) from stage 2 on, because its object literal lacks every retired method; the all-mock branch of `src/data/index.ts` therefore casts it (`createMockApi() as RainApi`, with the comment of section 8.5), and a `MockApi` is not assignable to `RainApi` once one method is retired. The router types `mock` as `MockApi`, and `routing.ts` may not name a retired method as `'mock'` (a compile error). The all-mock run (`VITE_RAIN_MOCK=1`, `dev:ui`) is accepted to lose screens from stage 3 on; stage 10 removes the flag.

The names each stage adds (the complete list; a section may not leave one out):

| Stage | Names added to `RetiredMockMethod` | Reason |
|---|---|---|
| 2 | `getHealth` | new method, the mock never had it; the two types are created here |
| 3 | `listDrives`, `startScan`, `getScan`, `getScanNode`, `cancelScan`, `revealPath`, `checkPaths` | disk slice deleted; `getScanNode` and `checkPaths` are new |
| 5 | `listApps`, `listPools`, `savePool`, `deletePool`, `rescanApps`, `launch`, `getLaunch`, `listProjects`, `listGames`, `saveCustomGame`, `setGameHidden`, `updateGame`, `deleteCustomGame` | `launches.ts` (it implements `launch` and `getLaunch`), `library.ts` and the app and pool methods of `launcher.ts` are deleted; `rescanApps`, `updateGame` and `deleteCustomGame` are new |
| 6 | `listJobs`, `setJobEnabled`, `runJob`, `getRunLog`, `listIdleJobs`, `setIdleJobEnabled`, `runIdleJob`, `getIdleJobLog`, `getMachineState` | `jobs.ts` deleted (`getMachineState` lives in it) |
| 7 | `getVaultStatus`, `listHabits`, `listHabitDays`, `listEvents`, `openDailyNote`, `rereadVault`, `listHabitSuggestions`, `saveHabit`, `deleteHabit`, `getHabitStats`, `setHabitDone` (all eleven life methods; the last six are new) | `life.ts` deleted |
| 8 | `listSessions`, `getSessionTranscript`, `focusSession`, `stopSession`, `answerPermission` | `sessions.ts` deleted |
| 9 | `getGameMode`, `startGameMode`, `resolveGameMode`, `markGameModeRecapSeen` | `gameMode.ts` deleted; `markGameModeRecapSeen` is new |

Stage 4 adds nothing: the mock slices of apps and pools are deleted in stage 5 (section 01 9.6).

Mixed-state cases (they exist only between stage 4 and the stage that finishes them; the router has a small special-case function for each, and each is deleted with the router):

| Method | Rule while mixed |
|---|---|
| `search` | call `real.search` for the types the backend already covers, `mock.search` for the others, merge by `score` descending. The scale is that of `src/lib/score.ts` (`api.ts` says Home ranks items together with command scores computed in the frontend), so the backend scorer is a port of `score.ts`, and one shared file of test vectors (query, text, keywords, expected score) is read by a vitest test and by a Rust test: decided here, built by section 01 in stage 4 |
| `listPinned`, `listRecent` | concatenate real and mock items, real first for equal timestamps; `listRecent` cuts to `limit` |
| `setPinned` | by `type`: real types to `real`, others to `mock` |
| `launch` | by `target.type`; the router remembers the ids of runs it got from the real side (a `Set`) so that `getLaunch(id)` goes to the same side |
| `subscribe(launch)` | the router subscribes to BOTH real and mock and forwards the frames of both (during stages 4 and 5 app and pool frames come from the backend, project and game frames from the mock); the router and the `routing.ts` test keep this special case until stage 5 ends, then `topics.launch` is real only (section 01 9.2 holds the code) |

Rule for stage planning: a stage moves together every method whose ids refer to each other (apps, pools, pins and launch of apps and pools move in one stage). The mock keeps running its own copy of the settings, because other mock slices read them; real and mock settings can differ meanwhile and that is accepted.

`mockControl` (scenario switcher, "Reset mock data", paused timers) keeps affecting only the mock until stage 10 deletes it. The stream client has nothing to do with the mock.

### 8.7 The stream client

`createStream(bridge)` returns `{ subscribe, onReset, state }`:
- `subscribe(topic, cb)` as in section 5.6; payloads pass through `resolveMedia` except topic `scan`.
- `onReset(cb: (info: { restarted: boolean }) => void): () => void`.
- `state`: a Vue `ref<BackendState>` for a future banner.
- It opens the bridge stream at creation, not at the first `subscribe`, so the `hello` that decides `restarted` is never missed.
- Unit-tested with a fake bridge (section 12.3).

`src/main.ts` after stage 1b (the mock reseed block stays until stage 10):

```ts
onStreamReset(({ restarted }) => {
  if (restarted) { resetLaunches(); resetDisk() }
  refreshResources()
})
```

### 8.8 Types barrel

`src/data/types.ts` becomes:

```ts
export * from '../bindings/rain-types'      // generated: everything that has been ported
export type ID = string                     // hand-written aliases; the generator inlines `string`
export type ISODate = string
export type ISODateTime = string
// ...the hand-written types that are not ported yet stay here until their stage deletes them.
```

Why a barrel and not the generated file as `types.ts` (which the spike recommended): the spike ported all 23 types at once, but the migration ports a few types per stage, and the not-yet-ported types must stay hand-written next to the generated ones. The barrel is the proven single-file output with a different path. Structural typing makes the aliases and the generated `string` fields interchangeable, so no screen changes (the spike's `Id` newtype, which kept the alias names in the output, is not needed and is not used: plain `String` in Rust). Generated names equal the existing TypeScript names (the Rust type is named like the TypeScript type). A type must not exist in both places: when a stage ports a type it deletes the hand-written one in the same commit. TypeScript lets a local export silently shadow one that comes from `export *`, so a leftover would not be an error; `scripts/check.mjs` therefore fails when a name exported by `rain-types.ts` is also declared in `types.ts`.

---------------------------------------------------------------------------------------------------

## 9. Types

### 9.1 `rain-types`

```
crates/rain-types/src/
  lib.rs
  common.rs        Status, IconKind, IconRef, ApiError, ErrorCode (ids and dates are plain `String`). `IconKind` is the enum `Lucide, Brand, Url, Letter` (camelCase) and is used as `#[ts(inline)] kind: IconKind` in `IconRef`
  events.rs        Frame, Topic trait and one struct per topic (ALL Topic structs live here; naming rule in section 5.3), Hello, Resync, Lifecycle
  system.rs        Ping, Health, ConfigIssue, SourceStatus, SourceState, ShellFacts
  launcher.rs  projects.rs  games.rs  sessions.rs  jobs.rs  life.rs  disk.rs  gamemode.rs  settings.rs
  env.rs           names and paths: pipe_name(), user_slug(), sandbox(), folder names (std only)
  time.rs          now_iso(), iso_from_system_time()
  limits.rs        the constants of Appendix A
  bindings.rs      the TypeScript generator: the `decls!` list and the `write_single_file` test (section 9.3)
```

One file per feature area, named like the area; a section adds its types to its file (and request bodies next to the type they belong to) AND one line to the `decls!` list in `bindings.rs`. A type that is missing from the list is not generated, which shows up as a TypeScript compile error in the frontend when it is used. Bodies of requests are named `<Thing>Body` (`PinBody`), patches `<Thing>Patch` (all fields optional; an absent field means "unchanged").

Dependencies of the crate (proven): `serde` (derive), `serde_repr` 0.1, `ts-rs` 12.0.1 with its default features; dev: `serde_json`.

### 9.2 Serde and generation conventions (every type follows these)

Every rule below was exercised by the spike (`spikes/types-stream/rain-types/src/lib.rs`, 23 types, checked against the hand-written `types.ts` with a strict `tsc` type-identity test, with `exactOptionalPropertyTypes`, plus real JSON captured from a running backend assigned to the generated types).

```rust
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]                     // no `export`: the generator calls `decl()` itself (section 9.3)
pub struct AppEntry {
    pub id: String,
    pub name: String,
    #[ts(inline)]                          // a field whose type is a string enum: keep the literal union inline
    pub kind: AppKind,
    pub icon: IconRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_launched_at: Option<String>,
    pub size_bytes: u64,                   // u64 becomes `number` through TS_RS_LARGE_INT in .cargo/config.toml: no per-field attribute
}
```

| Case | Rule |
|---|---|
| Optional field | `Option<T>` with `#[serde(default, skip_serializing_if = "Option::is_none")]` on the field AND `#[ts(optional_fields)]` on the struct. BOTH are needed: ts-rs only honours `skip_serializing_if` together with `default`, and without them it generates `tint: string \| null` (a required nullable field). Never send `null`. |
| String union | a Rust enum with `#[serde(rename_all = "camelCase")]`, and `#[ts(inline)]` on every field of that type, so the TypeScript keeps `'a' \| 'b'` in the field as in the hand-written file (13 fields in the spike). The enum is also exported as an extra named type; that is harmless. |
| Number union (`30 \| 60 \| 300`, `5 \| 10 \| 20 \| 30`) | a `serde_repr` enum: `#[derive(Serialize_repr, Deserialize_repr)] #[repr(u32)]` with `#[ts(type = "30 \| 60 \| 300")]`. Parsing rejects every other number, so the body validation comes for free ("invalid value: 45, expected one of: 30, 60, 300" was seen). Plain `u32` gives `number` in TypeScript, which is wider than the contract. (Draft 1 said `u32` plus a `validate()`; replaced.) |
| `u64`, `i64` | `number`, set once for the whole workspace by `TS_RS_LARGE_INT = "number"` in `.cargo/config.toml`. Without it ts-rs emits `bigint`. Values stay under 2^53 - 1 (`JSON_INT_MAX`); a test checks the largest value a field can take. specta, in contrast, fails to export `u64` by default. |
| Map | `BTreeMap<String, V>` (stable order) with `#[ts(type = "Record<ID, V>")]` on the field (the output is otherwise `{ [key in string]: V }`). This override works in the single-file layout only. |
| Dates, ids | plain `String`, built by `rain_types::time`. The TypeScript aliases `ID`, `ISODate`, `ISODateTime` stay hand-written in `types.ts` (section 8.8). |
| Tagged unions | avoided; use a flat struct with a `kind` enum, as the contract does today. |
| Recursive type, `r#type` field | work without a fix (`DiskNode` with `children?: Array<DiskNode>`; `r#type` is emitted as `type`). |
| `#[serde(transparent)]` | not needed (serde writes a newtype struct as its inner value) and ts-rs prints a warning for it. Do not use it. |
| Request bodies | `#[serde(deny_unknown_fields)]`. TOML file structs do not deny unknown fields (users edit by hand). |
| Naming | the Rust name equals the TypeScript name already in `types.ts` (`ClaudeSession`, `DiskScan`). |
| Doc comments | not emitted by `decl()` (field docs land in odd places). Keep the explanation in Rust and the generated file's header comment. |
| Output style | double quotes, semicolons and `Array<T>` instead of the repo's style. Commit the raw output, do not format it (prettier with `--no-semi --single-quote --print-width 130` reproduces the repo style, but a formatter step would break the byte-equal drift check). |
| Sample test | every exported type has `#[cfg(test)]` JSON round-trip with a fully filled and a minimal value, and calls `assert_no_nulls` (a helper that fails on `null` unless the type is on a small allow-list). |

Not yet exercised by the spike (the same attribute kinds are expected to cover them, but nobody saw it): `PoolDraft`, `EnvStep`, `Project`, `Game*`, `CustomGameDraft`, `Permission*`, `SessionEvent`, `ClaudeSession`, `JobRun`, `LogLine`, `ScheduledJob`, `IdleJob`, `MachineState`, `VaultStatus`, `CalendarEvent`. Each feature stage runs the strict type-identity check for its own types before it deletes the hand-written ones (section 9.4).

### 9.3 Tool, command, output (proven)

Tool: ts-rs 12.0.1. Rejected: specta 2.0.0-rc.28 (pre-release; its companions `specta-serde` and `specta-typescript` pin each other exactly; it needed 55 attribute lines against 30 for ts-rs, 17 repeated `Option` type overrides, per-field `u64` overrides and hand-written marker types for number unions and `Record`; `tauri-specta` gives nothing for one generic command). To keep a later swap cheap, only these things name the tool: the `#[derive(TS)]` and `#[ts(...)]` attributes in `rain-types`, `bindings.rs`, `.cargo/config.toml`, and the content of `src/bindings/rain-types.ts`.

Why one file and not ts-rs per-file `#[ts(export)]`: with `#[ts(type = "Record<ID, V>")]` the generated per-file type file has no imports for `ID` and `V` (`type =` switches dependency tracking off). The single file needs no imports.

```rust
// crates/rain-types/src/bindings.rs   (copy of spikes/types-stream/rain-types/src/bindings.rs)
macro_rules! decls { ($cfg:expr; $($t:ty),+ $(,)?) => {{ let mut out = String::new();
    $( out.push_str("export "); out.push_str(&<$t as ts_rs::TS>::decl($cfg)); out.push_str("\n\n"); )+ out }}; }
pub fn single_file(cfg: &ts_rs::Config) -> String {
    use crate::*;
    let body = decls!(cfg; Status, IconRef, ApiError, ErrorCode, Frame, Ping, Health, Settings, /* ...one line per type... */);
    format!("// GENERATED by `cargo test -p rain-types write_single_file`. Do not edit.\n\n{body}{}", topic_names_ts())
}
#[cfg(test)] #[test] fn write_single_file() {
    // RAIN_TYPES_OUT: the target file. RAIN_TYPES_CHECK=1: do not write, fail if the file on disk differs (CRLF-normalised).
}
```

`topic_names_ts()` appends `export const TOPIC_NAMES = ["sessions", "jobs", ...] as const`, built from the `Topic` structs, so no separate script writes a barrel (draft 1 had `scripts/gen-types.mjs`; it is not needed).

| Item | Value |
|---|---|
| Generate | `npm run types`, which runs `cargo test -p rain-types write_single_file` |
| Drift check | `npm run types:check`, which runs `RAIN_TYPES_CHECK=1 cargo test -p rain-types write_single_file` (proven: it failed as expected after a line was appended to the output) |
| Output | `src/bindings/rain-types.ts`, committed to git; `npm run build` needs no Rust |
| `.cargo/config.toml` | `[env]` `TS_RS_LARGE_INT = "number"` and `RAIN_TYPES_OUT = { value = "src/bindings/rain-types.ts", relative = true }` (`relative` resolves against the folder that holds `.cargo`; proven) |
| Determinism | same input gives byte-identical output (the check compares the text) |
| When | after every change under `crates/rain-types`; `scripts/dev.mjs` does it automatically; `npm run check` verifies it |

A small file `src/data/rest/topic-check.ts` makes sure at compile time that a topic cannot exist on one side only. A topic struct exists in `rain-types` only from the stage that ports its payload type (section 5.3, 14.1), so `TOPIC_NAMES` grows stage by stage, while the hand-written `Topic` of `api.ts` already holds every topic. The file therefore holds a hand-written union `UnportedTopic` that lists the topics whose Rust struct does not exist yet. It starts as `'sessions' | 'jobs' | 'idleJobs' | 'launch' | 'scan' | 'gameMode'`. It asserts `Equal<Topic, (typeof TOPIC_NAMES)[number] | UnportedTopic>` and that `(typeof TOPIC_NAMES)[number]` and `UnportedTopic` do not overlap. Each stage that adds a `Topic` struct removes its name from `UnportedTopic` in the same commit (3 `scan`, 4 `launch`, 6 `jobs` and `idleJobs`, 8 `sessions`, 9 `gameMode`). `projects`, `games` and `habits` are born ported and never appear in it. After stage 9 `UnportedTopic` is `never`. A new topic is added to `Topic` and to `TOPIC_NAMES` (through its struct), not to `UnportedTopic`.

### 9.4 How `types.ts` and `api.ts` relate to the generated file

- Generated: every ported entity, body and patch (`src/bindings/rain-types.ts`). `src/data/types.ts` re-exports it (section 8.8). The hand-written definitions in `types.ts` are deleted for the types of each stage, in the same commit as the Rust type, and `scripts/check.mjs` fails on a name that exists in both.
- Hand-written: `RainApi` and `TopicPayload` in `api.ts` (the interface of the client, not data). `RainError` becomes `export type RainError = ApiError`.
- A ts-rs gap or surprise (a union that comes out wider, a lost alias) is fixed with a `#[ts(type = "...")]` in Rust. It is never fixed by editing a generated file. The eleven differences the spike found and their fixes are the table of section 9.2.
- Stage 1a writes `Settings` in its final shape (section 06) and compares it with the TypeScript in the test file (section 06 A1); `src/data/types.ts` is untouched until stage 1b, so the old hand-written `Settings` is not the comparison target in 1a. Stage 1a generates only these types: `ApiError`, `ErrorCode`, `Frame`, `Ping`, `Health`, `Settings` and `SettingsPatch`, `IconRef` (with `IconKind`), `Status` (plus the helper types they contain, for example `ResetAfterSeconds`, `CountdownSeconds`, `MonitorClaudeState`). Nothing of game mode is generated in 1a: `GameModeState` and the other game-mode types, with `gamemode.rs` and its `decls!` entry, come in stage 9, where the hand-written ones are deleted. The output is compared with the hand-written ones by the strict type-identity check (`spikes/types-stream/tscheck/check.ts`: `Equals<H.X, G.X>` for each type under `strict` and `exactOptionalPropertyTypes`, with a negative control); the three hard cases (a number union, a `u64` field, an optional field) are among them, so the proven attributes are applied to real types at once.

---------------------------------------------------------------------------------------------------

## 10. Storage

### 10.1 Locations

| What | Real | Sandbox | Content |
|---|---|---|---|
| Machine file | `%APPDATA%\Rain\machine.toml` | `%APPDATA%\Rain-dev\machine.toml` | where the data folder is; nothing else |
| Data folder (default) | `%APPDATA%\Rain\data` | `%APPDATA%\Rain-dev\data` | the user's TOML files, covers the user adds |
| Local folder | `%LOCALAPPDATA%\Rain` | `%LOCALAPPDATA%\Rain-dev` | `rain.db` (+ `-wal`, `-shm`), `cache\`, `logs\`, `bin\`, `backups\claude-settings\` (copies of the Claude settings file made by the hooks installer, section 03) |

Resolution of the data folder, in this order: `RAIN_DATA_DIR`; `data_dir` in `machine.toml` (read once by `Config::from_env`, so the folder is known before any module starts); the default. Resolution of the local folder: `RAIN_LOCAL_DIR`; `<RAIN_DATA_DIR>.local` when `RAIN_DATA_DIR` is set; the default.

The owner of the project keeps the data folder outside the profile (decisions.md section 4); that location is stored by the user in `machine.toml` through Settings. No such path is in code, tests, fixtures, defaults or documents of the repository.

`machine.toml` (the whole schema; written by the settings module only):

```toml
schema_version = 1
data_dir = "C:\\path\\to\\folder"   # absent = default
```

Why a separate machine file: the setting that points at the data folder cannot live inside the data folder. The file is tiny, machine-specific, and not meant to be shared between PCs.

First run: if the data folder does not exist the backend creates it (and its parent) and writes nothing else. A feature creates its file on its first write, except where its section says it writes a file that contains only `schema_version` and comments (for example the commented example of `idle-jobs.toml`). The backend never deletes a file in the data folder, except a temp file it wrote itself.

### 10.2 The data folder: one TOML file per kind

| File | Owner section | Holds |
|---|---|---|
| `settings.toml` | 06 | user settings (the user-data fields of `Settings`, including `projectRoots` and `gameFolders`; overrides of detected paths, daily-note folder and format; the Claude monitoring switch) |
| `pins.toml` | 01 | pinned apps, pools, projects, games |
| `pools.toml` | 01 | pools and their members |
| `app-ids.toml` | 01 | id map: AUMID or normalised target path to a readable stable id (decision C3) |
| `projects.toml` | 02 | per-project environment steps and overrides |
| `games.toml` | 02 | custom games, and the per-game user data of every source: hidden flag, nicer title, cover, and for a game from a game folder the corrected program file (key = the game's folder; decisions.md section 4, "Game folders") |
| `jobs.toml` | 03 | scheduled job definitions, including `enabled` (the owner's first job, a Claude job at 06:00 that is switched off, is a definition in the owner's data folder, never in code or fixtures: decisions.md section 4) |
| `idle-jobs.toml` | 03 | idle job definitions, including `enabled` |
| `habits.toml` | 04 | habit definitions |
| `gamemode.toml` | 06 | game mode: `close_other_apps` (the master switch) and `keep`, the programs the user never wants closed (06 6.11) |
| `media\` (folder) | 02 | covers the user picked for games, copied here |

A new file needs an architect decision (a row here). Exact schemas belong to the owning section. What applies to all (section 10.3).

### 10.3 Common rules for every TOML file

1. First key: `schema_version = <integer>`. A missing key means version 1.
2. A file with a higher version than the code knows is loaded read-only: it is never written, and a `ConfigIssue` says "written by a newer Rain".
3. Migration of an older file happens in memory at load (`fn migrate(doc: &mut DocumentMut, from: u32)` in the module's `store.rs`). The migrated text is written only at the next legitimate write, after a one-time copy `<file>.bak-v<from>` beside it.
4. Readers use `serde` with `#[serde(default)]` on every field and do not deny unknown fields. Missing file means all defaults. Missing key means that key's default. Nothing is required.
5. Writers use `toml_edit::DocumentMut` so comments, key order and unknown keys survive. The `toml` crate is used only to read typed values.
6. Keys are `snake_case`. Table arrays use `[[name]]`. Every user-defined item has an `id` key that is its identity (ids are written by Rain on creation, readable, stable).
7. The text of a key never holds a secret (secrets: section 10.8).
8. Commented examples are part of the written file, not a separate file.

### 10.4 Loading, validation, watching, writing

Core type (module `store::toml_file`; every module's `store.rs` uses it and never touches `std::fs` for its TOML):

```rust
pub struct TomlFile<T> { /* path, last good value, content hash, mutex */ }
impl<T: DeserializeOwned + Default + Send + Sync + 'static> TomlFile<T> {
    pub fn open(hub: &ConfigHub, file_name: &'static str, validate: fn(&T) -> Result<(), String>) -> Self;
    pub fn get(&self) -> Arc<T>;                                   // last good value, never blocks on disk
    pub async fn edit(&self, f: impl FnOnce(&mut DocumentMut) -> Result<(), ApiError> + Send + 'static)
        -> ApiResult<Arc<T>>;                                      // read fresh, apply, write atomically, reload
    pub fn subscribe(&self) -> watch::Receiver<Arc<T>>;            // fires on load, on hand edit, on own write
}
```

Load: read the file on a blocking thread, parse with `toml`, run `validate`. On success the value replaces the last good one. On a parse or validation error the last good value is kept, a `ConfigIssue { file, line?, message }` is added to `state.health` (topic `health` is published), and the file is never overwritten while the issue stands. A broken file at first load means defaults plus the issue.

Watch (proven, spike crates `src/bin/q2_notify.rs`, crate versions in section 1.6): one watcher made with `new_debouncer(300 ms, None, handler)` and `deb.watch(folder, RecursiveMode::NonRecursive)` (`ReadDirectoryChangesW`, no polling), on the data folder plus every sub-folder a module registers (a non-recursive watch reports nothing from a sub-folder: observed). Events are debounced 300 ms (`TOML_DEBOUNCE`; the spike used 300 ms and 20 rapid writes became one event). What the spike saw: create gives `Create(File)`; modify gives `Modify(Any)`; rename gives `Modify(Name(Both))` with old and new path; an editor-style temp file plus rename over the target gives `Remove` and `Create` of the target (the temp file is not reported); delete gives `Remove(Any)`. Rule: do not interpret the event kind. On ANY event for a path ending in `.toml`, re-read that path. Rain's own atomic writes arrive as `Remove` plus `Create`, so the same rule covers them. A missing file after the event means "all defaults" (a deleted file). Rain's own temp files (`.<name>.rain-tmp-<pid>`) never match `.toml` and are ignored; so are names ending in `~` and `*.bak*`. After a debounced event the file is reloaded; if its content hash equals the hash Rain wrote last, it is the echo of Rain's own write and is ignored. A reload that changes the value notifies the module (`TomlFile::subscribe`), which publishes its topic or drops its cache. Risk and fallback: `notify` 9.0.0 was one day old when it was tested and only for seconds, not for days and not on a network drive; the fallback `notify` 8.2.2 with `notify-debouncer-full` 0.7.0 behaves identically on Windows (tested) and is a change of two lines in `Cargo.toml`. In `notify` 9 `watched_paths()` returns a `Result`.

Folder trees: `ConfigHub::watch_dir(path, recursive: bool, quiet: Duration) -> subscription` is the one watch API for folders that are not TOML files (the Start Menu folders of section 01, the Claude sessions folder of section 03, the vault of section 04). Sections 01, 03 and 04 all use it; none builds its own debouncer. A recursive watch of the Start Menu is unproven (appendix C); the fallback is one non-recursive watch per sub-folder.

Until the watcher exists (stage 1b), `TomlFile::refresh()` compares the file's length and modified time (and then the hash) with the last load and reloads when they differ; the settings read route calls it. After stage 2 it stays as a cheap safety net.

Write (`edit`):
1. Take the per-file async lock.
2. Read the current text from disk (never the in-memory copy) and parse into `DocumentMut`.
3. If the hash differs from the hash at the last load, a hand edit happened: reload first, so the closure sees the user's text.
4. Run the closure on the document. If it cannot find what it must change (the key moved or vanished), return `conflict`, reason `configChanged`. If the closure changed nothing, write nothing (the spike's `update_toml(path, |doc| -> bool)` returns false for "nothing changed").
5. Render, validate by parsing as `T`, write to `.<name>.rain-tmp-<pid>` in the same folder, `sync_all`, then rename over the target (`std::fs::rename`, which is `MoveFileExW` with `REPLACE_EXISTING`). If the rename fails with os error 5, 32 or 33 (an editor, antivirus or sync client holds the file), retry up to 8 times, 25 ms apart (`TOML_WRITE_RETRIES`); remove the temp file on every failure. After that return `failed` with "Could not save <file>". Proven: with another program holding the file with the default share mode (read, write and delete) the rename works; with a share mode that lacks delete it fails with "Access is denied (os error 5)" after about 0.9 s, the target unchanged and the temp file removed; if the lock is released inside the retry window it succeeds. Draft 1's 5 retries of 20 to 200 ms is replaced by what was tested.
6. On the first write of a process keep a copy `<name>.toml.bak` of the previous content.
7. Remember the written hash, reload, return the new value.

Fixed reasons for a refused or failed write (every section uses exactly these; none invents another): `configBroken` (409 `conflict`: the file does not parse (a syntax error), or fails as a whole (`settings.toml` has no per-entry skipping: a value of the wrong type fails the file), so the last good value is shown and the file is never overwritten. Entry-level `ConfigIssue`s, a skipped bad entry, do not block writes; a write keeps the skipped entry's text unchanged), `configNewer` (409 `conflict`: the file has a newer `schema_version`), `configChanged` (409 `conflict`: the closure could not find what it must change), `saveFailed` (500 `failed`: the write itself failed). A section may add reasons for other situations (for example a note write in section 04), not for these four.

Two `toml_edit` pitfalls were found and fixed in the spike (`src/toml_store.rs`; copy it, it holds both fixes and the tests): (1) `table[key] = value(x)` DROPS the trailing comment of the old value; copy the old value's decor to the new one (`set_bool_keep_decor`); (2) `toml_edit` normalises CRLF line ends to LF; remember whether the file used CRLF and render with the same line end (`render(doc, crlf)`). Also proven: comments, blank lines, trailing comments and arrays of tables survive a change, an add and a remove. One more rule for the feature sections: removing an array element (an `[[item]]`) also removes the comment lines directly above it, so a comment block above an item belongs to that item, and a section that wants a comment to stay puts it in a file header, not above an item it may delete.

All of this runs in `spawn_blocking`, with one in-process mutex per file. The data folder may be on a synced drive; the code never assumes it is local.

### 10.5 SQLite

| Item | Rule |
|---|---|
| File | `<local>\rain.db` (WAL files beside it). Never in the data folder, never in the repository. |
| Holds | machine history and rebuildable cache only (launch history, run records, git cache, habit-day cache, the game mode record; scan results are in memory only, section 05). User-owned data stays in TOML. Losing the file loses history, never the user's definitions. |
| Crate | `rusqlite` 0.43.0 with `bundled` (SQLite 3.54.1). Proven (spike crates `src/bin/q4_sqlite.rs`): the bundled library adds 1.10 MB to the exe with `lto`, `strip`, `opt-level = "s"` and `codegen-units = 1` (1.67 MB in the default release profile). |
| Connection model | One dedicated OS thread owns one `Connection` and receives closures over a `std::sync::mpsc` channel; it blocks in `recv` (zero CPU idle) and answers over a reply channel. The async API is `db.call(|conn| -> Result<T> { ... }).await`. All writes use it; so do reads, because the volume is tiny. No `Connection` is ever used on an async task. Proven, and available if a read ever needs to be concurrent with a long write: extra read-only connections opened with `OPEN_READ_ONLY \| OPEN_NO_MUTEX` (they refuse writes); measured 2000 acknowledged single-row inserts in 99.5 ms while 3 readers did about 240 000 reads each, worst read 1.3 to 17.6 ms, snapshots never went backwards. |
| Pragmas at open | `journal_mode=WAL` (set with `pragma_update_and_check`, because the statement returns a row; a plain `execute` fails), `synchronous=NORMAL`, `foreign_keys=ON`, `busy_timeout=5000`, `cache_size=-2000` (2 MB), `mmap_size=0`. |
| Shutdown | `PRAGMA wal_checkpoint(TRUNCATE)` so game mode leaves one clean file. |
| Corruption | If open fails or `PRAGMA quick_check` is not `ok`: rename the file to `rain.db.corrupt-<unix_ms>`, start empty, log at `error`, add a `ConfigIssue` for `rain.db`. |
| Migrations | Per module (below). Append only. |

Migrations: each module exports `pub const MIGRATIONS: &[Migration]` where

```rust
pub struct Migration { pub id: u32, pub name: &'static str, pub sql: &'static str }   // sql via include_str!("migrations/0001_create_history.sql")
```

ids start at 1 per module and are consecutive. A table `core_migrations(module TEXT, id INTEGER, name TEXT, applied_at TEXT, PRIMARY KEY(module, id))` records what ran. (The spike proved the same mechanism with `PRAGMA user_version` and one transaction per step, where a second open does nothing; `user_version` is one number per database, so the per-module table replaces it here and the transaction-per-step pattern stays.) At start, for every module in `MODULES` order, every migration with an id above the recorded maximum runs in its own transaction. A shipped migration is never edited; a change is a new migration. A database with a recorded id the code does not know (an older Rain started on a newer database) refuses to start that module and reports a `ConfigIssue` (the module's routes answer `unavailable`, reason `dbTooNew`).

Table naming rule: `<module>_<thing>`, snake_case, singular for a row of one kind of event (`launcher_launch`, `projects_git_cache`, `jobs_run`, `life_note`, `gamemode_record`). Indexes: `idx_<table>_<columns>`. A module touches only tables with its own prefix. Timestamps are stored as RFC 3339 `TEXT` (same format as the API); durations as integer milliseconds. Every table a section creates is listed in its "tables" heading with columns, retention (how long rows live) and whether it is a rebuildable cache.

### 10.6 Cache folder

`<local>\cache\<module>\...` for files that can be rebuilt (icons, covers, the last scan). A module owns only its own sub-folder and states a size cap. Sub-folders in use include `launcher` (icons, `cache\launcher\icons`), `jobs` (the default working folder of a job, `cache\jobs\<id>`, owned by `jobs`) and `idle` (the default working folder of an idle job, `cache\idle\<id>`, owned by `idle`); a module never writes into another module's sub-folder. A module recreates its default folder when it needs it (for example at the start of a run). The whole `cache` folder may be deleted by the user while the backend is stopped; the backend recreates what it needs.

### 10.7 Log folder

`<local>\logs\`: `backend.log`, `shell.log`, `hook.log` (file names `backend.<date>.log` and so on). Proven setup (spike crates `src/bin/q6_tracing.rs`, `src/log_budget.rs`):

```rust
let appender = tracing_appender::rolling::Builder::new()
    .rotation(Rotation::DAILY).filename_prefix("rain-backend").filename_suffix("log")
    .max_log_files(7).build(&log_dir)?;
let (writer, guard) = tracing_appender::non_blocking::NonBlockingBuilder::default()
    .buffered_lines_limit(8192).lossy(true).finish(appender);      // keep `guard` alive in main
// tracing_subscriber: registry + fmt layer (with_ansi(false)) + EnvFilter from RAIN_LOG; plus a panic hook that logs the panic.
```

- Rolling by day with 7 files kept: `max_log_files(7)` pruned 10 old files to 7 at start. There is no size cap in the library, so at start-up `log_budget::enforce_size_budget(dir, prefix, 20 MB)` deletes the oldest files first and never the newest (copy the spike's 25 lines).
- The non-blocking writer is used (draft 1 said no): a file write must not block an async task, and 100 000 `info!` calls returned in 31 ms. It is lossy on purpose: a burst beyond the 8192-line queue drops lines (26 746 of 100 000 reached the file in the stress test). Normal logging is far below that.
- Timestamps and the file date are UTC (tracing-appender rotates at UTC midnight). Draft 1 wanted the local offset; that was not tested and is not required. Log one line per event, no colour.
- A panic hook logs the panic message and location. What is never logged: section 13.4.

`hook.log` is written by `rain-hook` itself, append only, capped at 256 KiB (truncated at the start when over), and only for failures.

### 10.8 Secrets

Windows Credential Manager through `keyring-core` 1.0.0 with `windows-native-keyring-store` 1.1.0 (proven, spike crates `src/bin/q1_keyring.rs` and `q1b_minimal`: `keyring_core::set_default_store(Store::new()?)`, then `Entry::new(service, name)`, `set_password`, `get_password` (equal), `delete_credential`, then `Error::NoEntry`; `cmdkey` showed nothing afterwards). The backend uses it only through `core::secrets`, so a swap changes one file. This round no feature stores a secret (the calendar is off, decisions.md section 4), so the implementation and its dependency lines are added by the first stage that needs one; the trait and the in-memory fake may exist earlier.

```rust
pub trait Secrets: Send + Sync {
    fn get(&self, name: &str) -> ApiResult<Option<String>>;      // None = not set, never a crash
    fn set(&self, name: &str, value: &str) -> ApiResult<()>;
    fn delete(&self, name: &str) -> ApiResult<()>;
}
```

- Service name `rain` (real) or `rain-dev` (sandbox). User (entry) name = the secret's name, lower-case kebab. No secret name is reserved this round; a new secret name is a row in the section that needs it.
- Not tested and therefore handled defensively: large values, access from two processes, and the persistence (roaming or local) of the stored credential. Keep a blob under 2.5 KB. Never access one entry from two threads at once (the `Secrets` implementation holds a mutex).
- A TOML file may hold a reference such as `secret = "<name>"`, never the value. Only the backend touches Credential Manager (never the shell, never the hook).
- Tests use an in-memory `Secrets`.

---------------------------------------------------------------------------------------------------

## 11. The backend's internal structure

### 11.1 Crate layout

```
crates/rain-backend/
  Cargo.toml
  src/
    main.rs               flags, Config::from_env, runtime, run(); `#![windows_subsystem = "windows"]`
    lib.rs                pub fn run(config) -> Result; pub mod config, core, store, server, platform, modules
    config.rs             Config { pipe_name, data_dir, local_dir, sandbox, ... }; from_env(); testing constructor
    core/
      state.rs            AppState, AppStateBuilder, Slot<T>
      bus.rs              EventBus (section 11.3)
      error.rs            ApiError, ApiResult, extractors (section 4.5)
      health.rs           Health registry (config issues, sources; `set_file_issues(file, issues)` replaces the issues of one file)
      clock.rs            Clock trait and SystemClock
      blocking.rs         run(), spawn_hidden helpers (section 11.6)
      media.rs            MediaRegistry and MediaProvider (section 4.7)
      secrets.rs          Secrets trait and the Windows implementation
      shutdown.rs         ShutdownHandle, stop reasons, activity gauge
      throttle.rs         Throttle (`publish`, `publish_final`)
      winddown.rs         WindDownPort trait and its result types (section 06); jobs and idle implement it
      launch_contract.rs  the plain Rust types that launcher, projects and games share (section 01 6.9): ItemSource, LaunchPlanner, LaunchPlan, PlannedStep, StepAction, ShellFallback, OpenError, OpenRequest and the trait `Opener` (the one launch trait of the whole backend, section 11.6). Section 01 implements the engine against them; section 02 implements the traits
      paths.rs            PathProblem and `normalize(raw, allow_unc)`: the one pure path normaliser (section 05 6.9); disk and settings both call it
      logging.rs          tracing setup
    store/
      toml_file.rs        TomlFile<T>
      hub.rs              ConfigHub: folder, watcher, hash bookkeeping
      db.rs               Db thread and call()
      migrations.rs       runner
    server/
      pipe.rs             pipe options, accept loop (section 3.3)
      router.rs           assembles module routers; layers; fallback
      builtin.rs          GET /ping, GET /health, GET /stream, GET /media/..., PUT /internal/shell, POST /internal/shutdown
      stream.rs           SSE writer
    platform/             ALL unsafe code and raw Windows APIs
      pipe_acl.rs  com.rs  process.rs  job.rs  registry.rs  shell.rs  fs.rs  time.rs  idle.rs  ...
    modules/
      mod.rs              the MODULES table (section 11.2)
      launcher/  projects/  games/  sessions/  jobs/  idle/  life/  disk/  machine/  gamemode/  settings/  lifecycle/
  tests/
    common/mod.rs         the harness (section 12.2)
    crate_graph.rs  source_rules.rs  <module>.rs ...
    fixtures/<name>/data/ ...
```

`#![deny(unsafe_code)]` is set at the crate root; `platform/` carries `#![allow(unsafe_code)]`. No `unsafe` outside `platform/`.

### 11.2 Module shape

Every feature area is one folder under `modules/` with the same files:

| File | Contains | Must not contain |
|---|---|---|
| `mod.rs` | `pub const ENTRY: ModuleEntry`, the `Service` type re-export, the module's `MIGRATIONS` | logic |
| `routes.rs` | axum handlers and `pub fn router() -> Router<AppState>`: parse the request, call the `Service`, map errors, return. | business logic, file or database access |
| `service.rs` | `pub struct Service`: the logic and the in-memory state; publishes topics; the only public API other modules may call | HTTP types, SQL, TOML paths |
| `store.rs` | the `TomlFile`s, SQL statements, cache files; plain functions | HTTP types, bus access |
| more files | allowed (`scan.rs`, `steam.rs`); put OS-facing code behind a trait (section 12.2) | |

```rust
pub struct ModuleEntry {
    pub name: &'static str,                                   // "launcher", "disk" ...
    pub migrations: &'static [Migration],
    pub router: fn() -> Router<AppState>,                     // mounted under /api/v1
    pub init: fn(&AppStateBuilder) -> BoxFuture<'static, anyhow::Result<()>>,   // build the Service, store it in its Slot
    pub start: fn(AppState) -> BoxFuture<'static, anyhow::Result<()>>,          // spawn background tasks, subscribe to TomlFile
    pub stop: fn(AppState) -> BoxFuture<'static, ()>,
}
// modules/mod.rs
pub const MODULES: &[ModuleEntry] = &[ settings::ENTRY, lifecycle::ENTRY, machine::ENTRY, disk::ENTRY,
    launcher::ENTRY, projects::ENTRY, games::ENTRY, sessions::ENTRY, jobs::ENTRY, idle::ENTRY, life::ENTRY, gamemode::ENTRY ];
```

Start order is the table order; stop order is the reverse. Two-phase start: all `init` calls run first (so every `Slot` is filled), then all `start` calls. So a module may call another module's `Service` inside `start`, and cycles of calls at run time are fine; a cycle in `init` is a bug.

Ownership of modules (sections own the folder, nobody else edits it):

| Module | Section | Owns routes (section 14.4 has the full table) |
|---|---|---|
| `settings` | 06 | `/settings`, machine file; reads `state.shell_facts` for `hotkeyAvailable`; `Service::on_shell_facts_changed()` (stage 1b) publishes topic `settings` when the computed `hotkeyAvailable` changed |
| `lifecycle` | 06 | autostart, `on_shell_facts` (stage 2: ONLY the autostart self-heal and the `hotkey` health source, called by core after it stored the facts in `PUT /internal/shell`), stop-reason policy (the mechanism is core) |
| `gamemode` | 06 | `/game-mode*`, `/game-mode/recap/seen` |
| `launcher` | 01 | `/search`, `/pinned*`, `/recent`, `/apps`, `/apps/scan`, `/pools*`, `/launches*` and the launch engine for apps and pools (it also provides the content of the `app-icon` media kind through core's `GET /media/{kind}/{id}`) |
| `projects` | 02 | `/projects`, `POST /projects/refresh` and launching a project |
| `games` | 02 | `/games*`, `POST /games/refresh` and launching a game |
| `sessions` | 03 | `/sessions*`, `/internal/hook`, hook installer |
| `jobs` | 03 | `/jobs*`, `/runs/{id}/log` |
| `idle` | 03 | `/idle-jobs*` |
| `life` | 04 | `/vault`, `/habits*`, `/habit-days`, `/calendar/events`, `/daily-notes/{date}/open`, the single vault write function |
| `disk` | 05 | `/drives`, `/scans*` (with `GET /scans/{id}/node`), `/shell/reveal`, `/paths/check` |
| `machine` | 05 | `/machine` and the shared idle and load readings |

Core (foundation, stages 1a to 2) owns `GET /ping`, `GET /health`, `GET /stream`, `GET /media/{kind}/{id}`, `PUT /internal/shell` (stage 1b: the handler stores `ShellFacts` in `state.shell_facts`, publishes `health`, then calls `settings::Service::on_shell_facts_changed()`, which recomposes `Settings` and publishes `settings` only when the computed `hotkeyAvailable` changed; from stage 2 the handler also calls `lifecycle::Service::on_shell_facts`, which only does the autostart self-heal and sets the `hotkey` health source; core itself never builds a `Settings` payload), `POST /internal/shutdown`.

Calls between modules: only through the other module's public `Service` methods (`state.sessions().list()`), never its `store`, tables or files, and never types that are not in `rain-types`, with two exceptions that are part of the rule: (1) the plain Rust types of `core/launch_contract.rs` (section 11.1), which exist because launcher, projects and games must agree on one launch interface; (2) plain Rust types that appear in a public `Service` signature (`Resolved`, `LaunchedAfterGameMode`, `HooksStatus`) are part of that Service's API and may be named by the caller; they are re-exported from the owner's `mod.rs`. Path expansion is the Service method `settings::Service::expand_env(&self, raw: &str) -> String`. The pure function `core::paths::normalize(raw, allow_unc) -> Result<PathBuf, PathProblem>` is shared. Settings calls it with `allow_unc = true`, and disk calls it for `/paths/check` (`true`) and `/shell/reveal` (`false`). No other path normaliser exists for user-typed paths. A module reads another module's data through the owner's `Service` so that cache, hidden items and enabled flags stay consistent (the gaps report asks for a property test: every id returned by search, pins or recent exists in its list; section 01 owns that test).

### 11.3 Shared state and the event bus

```rust
#[derive(Clone)] pub struct AppState(Arc<Inner>);
struct Inner {
    env: Env,                          // names, folders, sandbox flag (copy of Config)
    start_id: String,
    bus: EventBus,
    db: Db,
    config: ConfigHub,
    health: Health,
    media: MediaRegistry,
    secrets: Arc<dyn Secrets>,
    clock: Arc<dyn Clock>,
    opener: Arc<dyn Opener>,           // the ONE launch seam (section 11.6): `state.opener()`; the `WindowsOpener` of section 01 in production, the one `FakeOpener` in tests
    shell_facts: RwLock<Option<ShellFacts>>,   // from PUT /internal/shell; None until a shell reported (feeds `Health.shell.reported`)
    shutdown: ShutdownHandle,          // token, reason, `shutting_down()`
    activity: Activity,                // `busy()` for /ping; `Activity::guard()` returns a guard that is dropped at the end of the work (scans, launch runs, job runs, idle runs, game-mode records)
    game_mode_active: AtomicBool,      // true from game mode start; schedulers and launchers refuse new work
    // one Slot per module service, filled in `init`:
    settings: Slot<settings::Service>, launcher: Slot<launcher::Service>, /* ... all twelve ... */
}
```

`Slot<T>` is a `OnceLock<Arc<T>>` whose `get()` panics with the module name if the module was not initialised (a programming error, caught by the start-up test). All twelve slots exist from stage 1 with stub services, so a section only fills in its own stub.

Event bus:

```rust
impl EventBus {
    pub fn publish<T: Topic>(&self, payload: &T::Payload);       // serialise once, add seq, broadcast; no-op without receivers
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<FrameBytes>>;   // used by the SSE writer only
    pub fn seq(&self) -> u64;
}
```

Modules publish and never subscribe to the bus. A module that must react to another module's change subscribes to that module's `Service` (a `watch` or callback it exposes), not to the bus.

### 11.4 Time and clock

`Clock` has `now() -> SystemTime`. Modules ask the clock, never `SystemTime::now()` directly, so tests control time. Timers use `tokio::time` (tests use `tokio::time::pause`).

### 11.5 Background work rules (idle cost)

Targets: backend idle under about 30 MB resident and about 0% CPU with no client; a connected shell adds one wake-up every 25 s. Measured in stage 2 (section 14).

Runtime (decision, with the measurement): `tokio` multi-thread with `worker_threads(2)`, `max_blocking_threads(8)`, `thread_keep_alive(5 s)`, `enable_all()`. The spike (crates, `q7_idle`) measured an idle axum server in a release build: `current_thread` 0.0 ms CPU over 20 s and after 200 requests, 0.8 MB private memory, 4 threads, 467 KB exe; `multi_thread(2)` 0.0 ms CPU, 0.9 to 1.0 MB private memory, 6 threads, 478 KB exe. The spike recommended `current_thread`; this document keeps `multi_thread(2)` because the cost is 0.1 MB and two threads, and because with one thread any accidental blocking call in an async task would freeze the pipe, `/ping` and every open stream together. The transport alone, with 8 pending instances and a connected stream, measured 6.7 to 7.0 MB working set and 1.5 to 1.8 MB private (types-stream; that spike ran a 1 s demo ticker that the product must not have). Blocking work (SQLite writer, scans, file reads) never runs on the two workers.

A background task is allowed to use a timer only in these six cases; each is a named exception in the code (`#[allow(clippy::disallowed_methods)]` with a comment pointing here):

| # | Timer | Rule |
|---|---|---|
| T1 | SSE keep-alive, 25 s per connected client | none without a client |
| T2 | Scheduler: one `sleep_until(next_due)` for scheduled jobs | recomputed when the job list changes, on wake from sleep, and on a clock jump; never a tick. If the power-resume notification cannot be registered, the sleep is capped at 5 minutes and a `warn` is logged |
| T3 | Idle detection sampling | 30 s; 5 s only while an idle job is running (to freeze it quickly); only while an idle job is enabled and game mode is off |
| T4 | Short debounces and timeouts (TOML watch 300 ms, transcript folder watch 2 s quiet period and 2 s pause after a read, only after a change, request 15 s, stop budgets, the 5 s pipe-busy retry, the 19 s permission wait `HOOK_SERVER_WAIT`, the 180 s `GAME_MODE_WAIT_LIMIT`, `REVIEW_ABANDON` 10 min, `RESTART_DELAY` 500 ms) | created for one operation, dropped after |
| T5 | In-flight operation progress (scan progress, launch step delay, the game mode countdown tick at the next whole second of `ends_at`, wind-down step delays of 400 ms, the job log flush of 200 ms) | exists only while the operation runs |
| T6 | Expiry of a finished disk scan's in-memory tree: one `sleep` of 10 minutes (`SCAN_ARENA_TTL`) | started when a scan finishes, restarted whenever its tree is read, dropped when it fires or when a new scan starts. Reason: a full tree of a 1 000 000 file drive costs 60 to 200 MB (spike winapi), far above the idle budget, so it must not stay forever; the budgeted tree and the lazy-subtree route are served from it |

Everything else is event driven:

| Need | Source |
|---|---|
| File and folder changes (TOML, vault, git HEAD, Steam files, the Claude registry folder, the transcript folders of live sessions) | `notify` (`ReadDirectoryChangesW`) |
| Process ended | `tokio::process::Child::wait` or `WaitForSingleObject` on the handle |
| Config changes | `TomlFile::subscribe` |
| Facts (apps, git, Steam, sessions) | computed on request, cached in SQLite or memory, invalidated by a watch or by a refresh route; a TTL is checked at read time, never by a timer |
| New work for the UI | `bus.publish` from the code that learned about the change |

Lint that enforces it: `clippy.toml`

```toml
disallowed-methods = [
  { path = "tokio::time::interval", reason = "no polling timers: see docs/backend/00-foundation.md 11.5" },
  { path = "tokio::time::interval_at", reason = "same" },
  { path = "std::thread::sleep", reason = "blocks a thread; use tokio::time::sleep or an event" },
  { path = "std::process::Command::new", reason = "use platform::process::hidden_command (no console window, no handle leaks)" },
]
```

A review checklist item for every stage: "list every `tokio::time` and `thread::spawn` this change adds, and the table row that allows it".

While game mode is active (`game_mode_active`), modules start no new scheduled job, idle job, scan, git refresh, Steam refresh or vault scan; `Service` methods that would start work return `conflict`, reason `gameMode`. Reads of cached data still work.

### 11.6 Blocking work, processes, COM

| Work | Rule |
|---|---|
| Anything that can take more than about 5 ms or calls a blocking OS API: file walks, `read_dir` loops, registry enumeration, SQLite, hashing, reading a big file, image decoding | `core::blocking::run(|| ...).await` (a thin `spawn_blocking` that maps a join error to `failed`). |
| Long-running CPU or disk loops (the disk scan walkers) | the proven walker (spike winapi, `src/walk.rs`): `std::fs::read_dir`, `entry.metadata()` (free on Windows, it comes from the find data), `rayon` `par_iter` per directory on a pool of 8 threads (`ThreadPoolBuilder::new().num_threads(8)`, built for the scan and dropped after it, so no idle threads remain; 1, 2, 4, 8, 16 and 32 threads took 2.11, 1.19, 0.68, 0.41, 0.56 and 0.56 s, so 8), an `AtomicBool` cancel flag checked at the top of each directory (cancel took 3.1 ms and left a valid partial tree). Skip reparse points (attributes `& 0x400`; they are not followed). A `PermissionDenied` folder is counted and kept as an empty "denied" node. Below-normal thread priority was not tried (UNPROVEN, set it through the pool's start handler and measure). Not the blocking pool. |
| External programs (git, `claude`, user commands) | `platform::process::hidden_command(program)` returns a `tokio::process::Command` with `CREATE_NO_WINDOW`, null stdin, and no inherited handles. At most 4 `git` processes at once (a semaphore in the `projects` module). Output is read with a size cap. Environment `GIT_TERMINAL_PROMPT=0`, `GIT_PAGER=cat`, `LC_ALL=C` for git. `CLAUDE*` and `ANTHROPIC*` variables are removed from the environment of a headless Claude child, and `RAIN_RUN_ID` and `RAIN_JOB_ID` are set on it (proven: both reached every hook, spike claude-hooks). |
| A child tree Rain owns and may have to kill (a scheduled job run, an idle job run) | `platform::job::OwnedJob`, hand-written (about 60 lines; copy `spikes/process/src/bin/q4_hand.rs` and the helpers `resume_process_threads` and `job_pids` in `src/lib.rs`): `CreateJobObjectW`; `SetInformationJobObject(JobObjectExtendedLimitInformation)` with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; spawn the child with `CREATE_SUSPENDED \| CREATE_NO_WINDOW` and piped output; `AssignProcessToJobObject(job, child handle)`; resume the child's threads through a Toolhelp thread snapshot and `ResumeThread` (std does not expose the thread handle); stop with `TerminateJobObject` or by closing the job handle; list the members with `QueryInformationJobObject(JobObjectBasicProcessIdList)`. Proven: terminate, close and an owner `abort()` each left zero survivors of a 4-process tree (cmd, cmd, ping, conhost) and the tree was gone in about 1 ms. Do not use `process-wrap` 10.0.1: it never sets `KILL_ON_JOB_CLOSE` (a dropped child and an aborted owner leave the tree alive) and hides the job handle (no member list, no breakaway flags). The claude-hooks spike used `process-wrap` for its headless job and the kill worked there, but only because the owner did not crash; the crash safety is the point of the hand-written job. The job is Rain's own, so it must NOT have `SILENT_BREAKAWAY_OK` (that frees the tree Rain wants to kill). Exit code of a killed run is 1 (not a signal); Rain marks the run "cancelled" from its own state. |
| Stopping a child | There is no graceful stop for a headless Claude run: closing its stdin mid-tool does not stop it (alive 8 s later), so the stop is `TerminateJobObject`. For other children the proven order is: `WM_CLOSE` to the child's top-level windows (proven for a GUI child with a window, not for a packaged app, whose window belongs to `ApplicationFrameHost`) or a cooperative named event; a console control event (`CTRL_C_EVENT`, or `CTRL_BREAK_EVENT` for a child started with `CREATE_NEW_PROCESS_GROUP`) sent by a THROWAWAY helper process that does `FreeConsole`, `AttachConsole(pid)`, `SetConsoleCtrlHandler(None, true)`, `GenerateConsoleCtrlEvent` (calling it directly from a process without that console silently does nothing, and `AttachConsole` inside the test process killed that process); a grace period; then `TerminateJobObject`. `ping -t` answers `CTRL_BREAK` with statistics and keeps running, so use `CTRL_C` for such programs. This round builds the kill and `WM_CLOSE` only; the console helper is not built (no feature section needs it). |
| Starting user programs | `ShellExecuteW(None, "open", target, None, None, SW_SHOWNORMAL)` for shortcuts, URIs and folders; it returns a value above 32 on success (folder 162 ms, `.lnk` 609 ms, `shell:AppsFolder\<AUMID>` 217 ms, URL about 290 ms). A packaged app (known AUMID) may instead be started with `IApplicationActivationManager::ActivateApplication(aumid, null, AO_NONE)` on the COM thread, which returns the pid in 160 to 260 ms. A direct process start (`Command` with `DETACHED_PROCESS \| CREATE_NEW_PROCESS_GROUP \| CREATE_BREAKAWAY_FROM_JOB`, retried without breakaway on os error 5, as in section 2.3) only where exact argument quoting is needed or the working folder must be set (games, decision C10). NEVER inside a kill-on-close Job Object, and never put the backend itself in one. Why (spike process, `q3_outer`, `q3_inner`): `ShellExecuteExW` on an exe does NOT escape a kill-on-close job (the child inherits the job and cannot pass `CREATE_BREAKAWAY_FROM_JOB`, and was killed with it); a `Command` with `CREATE_BREAKAWAY_FROM_JOB` fails with os error 5 unless the job allows breakaway; `explorer.exe <path>` escapes but leaves a long-lived `explorer.exe /factory` process per call and cannot pass arguments, so it is NOT used; packaged apps are started by the system broker and never inside the caller's job. Tests never call the real `ShellExecute` with a URL: the spike's URL test opened a tab in the owner's real browser four times and they could not be closed automatically. Every launch goes through ONE trait (`trait Opener`, defined once in `core/launch_contract.rs` with `OpenRequest`; the instance is `state.opener()`), with one `FakeOpener` in tests. The launcher engine (section 01), `games::Service::launch_after_game_mode` (section 02) and `openDailyNote` (section 04) all use it; no section defines another opener trait. |
| One shell call for every ShellExecute | `platform::shell::shell_execute(verb, file, params, dir, flags) -> Result<(), u32>` (`ShellExecuteExW`, `SEE_MASK_FLAG_NO_UI` set, run through `platform::com::run`). Sections 01, 02 and 04 all call it; there is no `ShellExec`, `shell_execute_open` or second variant. The `ShellExecuteW` text above describes the same call. |
| Other new `platform` functions (owner section) | `list_windows`, `spawn_detached` (returns `Detached::Free` or `Detached::InJob`: when breakaway fails with os error 5 it asks `IsProcessInJob`, logs `warn` "launched inside a job" and reports `InJob`, because the program then dies with the backend), `open_for_wait(pid, creation_time) -> Option<ExitWatcher>` (02 and 03 share it: `ExitWatcher` of section 03 6.3.6, built on `RegisterWaitForSingleObject`; it parks no thread, so the 8-thread blocking pool is never used for a process wait); `power::on_resume` (03); `shell::reveal` (05); `thread::set_below_normal` (05). The two local-date functions live in `platform::time` (section 4.3): `local_date(SystemTime) -> String` and `local_date_of_unix(i64) -> String`; `rain_types::time` has only `now_iso` and `iso_from_system_time`. |
| Revealing a file | `SHParseDisplayName` then `SHOpenFolderAndSelectItems(pidl, None, 0)` on the COM thread (218 ms, selection verified). Not `explorer.exe /select` (4 ms, but it leaves a `/factory` process). |
| Windows shell and COM calls (icons, shortcut resolution, the app list, `IApplicationActivationManager`, `SHOpenFolderAndSelectItems`, known folders) | One dedicated COM thread, `platform::com::run(|| ...)` (below). |
| Process list (which programs run) | hand-written Toolhelp snapshot plus `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` plus `QueryFullProcessImageNameW` (spike process, `src/bin/q6.rs` `list_images`): 262 processes in a median of 4.71 ms, 147 image paths resolved and 114 not readable (protected or system processes, no administrator rights). Names only, from the snapshot, cost 2.40 ms. No `sysinfo`. Compare paths, never only names. |
| Registry, Credential Manager, `GetLastInputInfo`, power state, drive facts | blocking helper; plain functions in `platform/` that return plain data. `GetLastInputInfo` works from a windowless process: idle milliseconds = `(GetTickCount64() as u32).wrapping_sub(dwTime)` (the 32-bit wrap is by design; observed 0 to 562 ms while the user was active). Drives: `GetLogicalDrives`, `GetDriveTypeW == 3`, `GetVolumeInformationW`, `GetDiskFreeSpaceExW` (use "available" as free); about 35 microseconds per call; the label can be empty, so the backend writes a fallback name. |

COM threading rule (all Windows shell calls):
- There is exactly one COM thread per process, created on first use, initialised with `CoInitializeEx(COINIT_APARTMENTTHREADED)` (STA), and uninitialised at shutdown. It blocks on a channel when idle (zero CPU).
- API: `platform::com::run<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> impl Future<Output = Result<T>>`. The closure runs on the COM thread. It returns plain data (`String`, `Vec<u8>`, numbers). No COM interface pointer, `HSTRING` buffer or handle may leave the closure, and none is stored between calls.
- A caller wraps `run` in `tokio::time::timeout(10 s)`. A call that times out is abandoned (the thread cannot be killed) and logged at `warn`; if it never returns, later COM calls fail with `unavailable`, reason `comBusy`, until the process restarts. Shortcuts on disconnected network drives are the usual cause; the modules avoid resolving them eagerly.
- No other thread may call COM. Code in `platform/` asserts this in debug builds.
- Proven on such a thread (spike winapi, `src/bin/apps.rs`, `icons.rs`, `launch.rs`): enumerating `shell:AppsFolder` (125 items in 190 to 200 ms, every item with an AUMID); rendering a 64 px PNG icon of an exe, a `.lnk` and an AUMID through `IShellItemImageFactory::GetImage` with `SIIGBF_ICONONLY \| SIIGBF_BIGGERSIZEOK` (first call 40 to 65 ms, warm 1 to 11 ms; all 125 icons in 2.7 s); `ActivateApplication` by AUMID. Details for the feature sections are in the review of this document and in each section.

### 11.7 Errors and logging

- Library errors use `thiserror` enums inside a module. A route converts them to `ApiError` with an explicit sentence. `anyhow` is only for start-up and `ModuleEntry` functions.
- A failed module `start` does not stop the backend: the module's routes answer `unavailable`, reason `moduleFailed`, the error is logged at `error`, and `Health.sources` gets `failed`. The backend starts without it.
- A panic in a handler becomes `500`; a panic in a background task is caught by its `JoinHandle` wrapper (`core::task::spawn_named`), logged at `error` with the task name, and the task is not restarted automatically unless its section says so.
- `tracing` levels: `error` = something that needs the user or developer; `warn` = a source degraded or a fallback was used; `info` = lifecycle (start, stop, module start, config reload, a job started or ended); `debug` = one line per request (method, path, status, ms) and per cache refresh; `trace` = rarely used. Default filter `info`.
- Log fields: use structured fields for ids, counts and durations. Never log the items of section 13.4.

---------------------------------------------------------------------------------------------------

## 12. Testing

### 12.1 Unit tests

Every module has unit tests next to the code (`#[cfg(test)] mod tests`), for pure logic: parsers, scoring, schedules, state machines, TOML migrations, mappers from OS data to API types. Rules: no real network, no real Windows shell, no real clock, no files outside a temp directory, no real registry or Credential Manager (use fakes of the traits below). Each test names what it proves.

### 12.2 Integration harness

Location: `crates/rain-backend/tests/common/mod.rs`. Integration tests are files `tests/<module>.rs` plus `tests/process_smoke.rs`.

```rust
pub struct TestBackend { pub client: PipeClient, pub data_dir: PathBuf, pub local_dir: PathBuf, /* handle */ }
impl TestBackend {
    pub async fn start() -> Self;                       // empty fixture
    pub async fn with_fixture(name: &str) -> Self;      // copies tests/fixtures/<name>/data into a temp dir
    pub fn builder() -> TestBackendBuilder;             // .fixture(..) .clock(..) .with_apps(FakeApps) ... .secrets(MemorySecrets)
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> T;
    pub async fn send(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value);
    pub async fn stream(&self) -> StreamReader;         // yields frames
    pub async fn stop(self);                            // POST /internal/shutdown, wait for exit
}
```

- It runs `rain_backend::run` in-process on the pipe `rain-test-<pid>-<counter>` with a temp data dir and temp local dir, `sandbox = true`, `MemorySecrets`, and a controllable `Clock`. Configuration is passed as a `Config` value, not through environment variables, so tests can run in parallel.
- The test client is `rain_pipe::PipeClient`, the same pooled client the shell uses, so every test also exercises the real transport (the spike's `bin/client.rs` has the `get`, `watch`, `lag`, `stall`, `stale` and `bench` helpers to copy from).
- `tests/process_smoke.rs` is the only test that starts the real `rain-backend.exe` (found next to the test binary) with the environment variables `RAIN_PIPE`, `RAIN_DATA_DIR` and `RAIN_LOCAL_DIR` set, calls `/ping`, reads the first two stream frames (`hello`, `resync`), and stops it with `--stop`. This proves the environment overrides and the binary.
- Fixtures: `tests/fixtures/empty/data/` (an empty folder), `typical/data/` (every TOML file with a few made-up items: fake names, no real paths of anyone, no personal data), `broken/data/` (a `settings.toml` with a syntax error). A section adds files to the fixtures for its own TOML. Fixtures never contain a path under a real user profile; paths are placeholders resolved by the harness (`{TEMP}`, `{DATA}`).
- Machine facts come through traits owned by the module, with production code in the module or in `platform/` and a fake in `tests/`: for example `trait AppSource`, `trait GitRunner`, `trait SteamLibrary`, `trait ProcessLister`, `trait VaultFs`, `trait Secrets`, `trait Clock`. The builder lets a test replace any of them: `builder().with::<dyn AppSource>(Arc::new(FakeApps::new(...)))`. Each section lists its traits and fakes under "tests".
- Every route has at least one test for: success, an `invalid` body, a `notFound` id, and (for mutations) idempotency as stated in the route table. Every topic has a test that the frame arrives with the expected payload and that a `resync` arrives on connect.

### 12.3 Frontend tests

The frontend has no test runner today. Stage 1b adds `vitest` (dev dependency) and `npm test`. What is tested once the mock is gone:

| What | How |
|---|---|
| REST client: path, method and body of every implemented method; media rewrite; error mapping | a fake `Bridge` records calls and returns canned JSON typed by the generated types |
| Stream client: `hello` then `resync`, restart detection, `subscribe` fan-out, reconnect callback | a fake `Bridge` that pushes frames |
| Router: each method goes to the side `routing` says | two fake apis |
| Screens and composables | not unit tested (as today); verified by running the app against a backend |

States that the mock scenarios (Typical, Empty, Busy, Failing) produced are now produced like this:

| State | How |
|---|---|
| Typical, Empty | start the backend on a fixture data folder: `node scripts/backend.mjs start --fixture typical` (copies `tests/fixtures/typical/data` to `.rain-dev/data` and sets `RAIN_DATA_DIR`) or `--fixture empty` |
| Failing | stop the backend (`npm run backend:stop`), or use the `broken` fixture (a corrupt `settings.toml`) |
| Busy | start the `typical` fixture and run the job "Demo wait" (about 90 s) from the Jobs screen; other sections may still add a fault route under `/internal/test/...` (sandbox only) |

`--fixture <name>` is built in `scripts/backend.mjs` and `scripts/dev.mjs` in stage 1b, together with the npm script `dev:fixture` (stage 10 does not build them; it only removes `dev:ui` and `VITE_RAIN_MOCK`).

### 12.4 Checks every stage must pass

`npm run check` runs these in order and stops at the first failure. A stage is not done until it passes on a clean checkout of the working tree.

| # | Command | Meaning |
|---|---|---|
| 1 | `cargo fmt --all --check` | formatting |
| 2 | `cargo build --workspace` | everything compiles |
| 3 | `cargo clippy --workspace --all-targets -- -D warnings` | lints, including the disallowed methods |
| 4 | `npm run types:check` | generated types are current (it runs BEFORE the tests, because the test run rewrites the generated file and would hide a drift) |
| 5 | `cargo test --workspace` | unit and integration tests |
| 6 | `npm test` | frontend tests (skipped until stage 1b adds vitest and the `test` script; `scripts/check.mjs` turns the step on by itself when the script exists) |
| 7 | `npm run build` | `vue-tsc -b` and `vite build` |
| 8 | `npm run smoke` | the app starts (section 12.5) |

Each stage adds to the checks its own acceptance tests (section 14), as ordinary tests in the above.

### 12.5 The smoke check ("the app starts")

`scripts/smoke.mjs`, using unique temp folders and a unique pipe name. In stage 1a it runs steps 1, 2 and the backend half of 4 and 5 (there is no shell change yet); from stage 1b it runs all five:

1. Build `rain-backend` and the shell in debug (`cargo build -p rain-backend -p rain`; the shell's debug build loads `devUrl`, so it needs no frontend build to start).
2. Start `rain-backend.exe` with `RAIN_PIPE`, `RAIN_DATA_DIR`, `RAIN_LOCAL_DIR` set. Wait for `/ping`.
3. Start `rain.exe --hidden` with the same variables. Wait until `GET /health` shows `shell.reported = true`. This proves: the shell started, found the pipe, relayed `hello`, and reported its facts.
4. Call `GET /settings` through the pipe; compare with the defaults.
5. End the shell process, call `rain-backend --stop`, check the pipe is gone and no process of ours is left.

The smoke check does not open a visible window (`--hidden`) and does not register the real hotkey conflict-free: if the hotkey is taken, `hotkeyRegistered` is false and the check still passes.

---------------------------------------------------------------------------------------------------

## 13. Security and privacy rules

### 13.1 The pipe boundary

- Boundary: the Windows user. Anyone who is that user (any program of theirs) can call every route. This is the accepted trust model for a personal launcher; it is stated here so nobody believes the pipe is a lock against the user's own malware.
- Other users, other machines and web pages cannot reach the pipe (ACL, `reject_remote_clients`, no TCP port).
- `/internal/*` is for Rain's own programs. The shell and the Vite proxy never forward it.
- Browser dev mode has an origin guard (section 7) and exists only in `vite serve`.
- Pipe-name squatting by another user is not defended beyond the user name in the name and `first_pipe_instance`. Not worth more on a one-user PC (documented, not implemented).

### 13.2 Web requests and personal data

- Rain's backend and shell make no network requests in this round, except what a section explicitly allows (none today: covers are local only; the calendar is not connected).
- If a section later adds one, it must say what leaves the PC, and it must never contain the user's name, profile paths, note text or file content.
- Research and development web requests never include personal details (hard rule 6 in `decisions.md`).

### 13.3 The two write rules

These two rules are checked by `crates/rain-backend/tests/source_rules.rs`, which scans the source text and fails the build on a violation (a plain text scan is crude on purpose: it is cheap, and a false alarm is fixed by moving the code).

1. Vault write rule. Rain never writes inside the Obsidian vault except through one function, `life::vault_write::set_habit_property(...)`, called only from the habit tick route and only as the direct result of the user's tick, changing exactly one property of exactly one daily note (or creating a bare note with only habit properties). The function is the proven line-based editor of spike crates (`src/frontmatter.rs`: `set_bool` and `tick_habit_in_file`; copy it with its tests, 38 exact-output cases, 12 refusal cases and a 20 000-note randomised invariant test that uses `yaml-rust2` only as a test oracle): it replaces only the `true` or `false` token (spacing, trailing comment, key quoting and line ends kept; `True` stays `True`; an already-equal note is returned byte-identical and is not written), inserts one line before the closing fence when the key is absent, prepends `---`, the line and `---` after any BOM when there is no front matter, works on top-level keys only, reads strict UTF-8, writes through a temp file and a rename after a length-and-mtime stale check (up to 3 retries), creates a missing note with `create_new`, never creates a folder, and REFUSES (`InvalidKey`, `DuplicateKey`, `NotABoolean` for a string, `yes`, an empty value, a list or a block scalar, `UnreadableFrontmatter`) instead of guessing. The refusals become `invalid` or `conflict` errors with a plain message. The scan fails if any file of `modules/life/` other than `vault_write.rs`, or any file of another module, contains a write call (`fs::write`, `File::create`, `OpenOptions` with write or append, `fs::rename`, `fs::remove_file`, `fs::create_dir`) on a path derived from the vault path; and fails if `set_habit_property` is referenced outside `life/service.rs`, where it is called only from the function `tick`, which only the route handler of `PUT /habits/{id}/days/{date}` calls (no `plan_tick` and `finish_tick` split). In sandbox mode the function refuses unless the vault folder is inside the data folder.
2. Claude settings rule. Rain never touches the user-level Claude settings file (`.claude\settings.json` under the user profile) during development or tests. The only code that may name it is `sessions/hooks_install.rs`, gated by the Settings switch "Monitor Claude sessions", and in sandbox mode it refuses any target unless the canonical target path lies inside the system temp folder, or the data folder or local folder of this run. The string `.claude` is also allowed in `sessions/claude_paths.rs`, which only reads (sessions, projects, transcripts); the string `settings.json` stays allowed only in `sessions/hooks_install.rs`. The scan fails if the strings `.claude` or `settings.json` appear in any other file of the crate (test files may use them only with a temp path built by the harness, in files listed in an allow-list inside the test). "Always allow" in Rain lasts for the session only; nothing is written to any Claude settings file for it (decision of 2026-11-22).

### 13.4 What is never logged

Never in any log, in any level: note text or vault file content; Claude prompts, transcript text and the `detail` of a permission request (a command can contain a secret; only the tool name and a request id may be logged); hook payloads as a whole; credential values and the iCal address; the content of user files; the environment of a launched program; full command lines of the user's launched programs (log the app id). Paths of the data folder and the local folder may be logged. The source scan also fails on a `tracing` macro call that passes a field named `text`, `content`, `detail`, `prompt`, `body` or `secret`.

### 13.5 Other rules

- No elevation. The backend never asks for administrator rights; the disk scan skips what it cannot read and counts it (decision 2026-11-22).
- Launched programs and games are never children of a kill-on-close Job Object of Rain.
- The user's open Claude sessions (terminal or desktop app) are only listed by game mode, never stopped (decision 2026-11-22). Only processes Rain itself started may be stopped by game mode.
- No code outside `platform/` calls `unsafe`.
- Commit nothing, push nothing: only the lead commits when the user asks.

---------------------------------------------------------------------------------------------------

## 14. The stage plan

Stage 0 (the five spikes) has run and is not part of this plan; its results are in the sections above and in appendix D.

Common to every stage: the checks of section 12.4 pass; the app builds and starts and every screen still works (a screen whose data has moved to the backend shows real data, an error row with Retry, or a true empty state; every other screen runs on the mock); from stage 1b on the router (section 8.6) sends exactly the methods of finished stages to the backend; every stage report lists the files changed and the contract changes (section 15).

### 14.1 Stage 1: plumbing and the first real call, in two parts

Review result: draft 1 put about 35 files and five new technologies in one stage 1, which is neither small nor provable in one step. Stage 1 is now two parts. Each part leaves the app working; the numbers 2 to 10 do not change (the feature sections refer to them).

#### Stage 1a: the workspace and the empty backend (no frontend change, no shell change)

Goal: prove that the backend exists, is reachable through the pipe by a Rust client, speaks the error shape and the stream, and that the type pipeline works. The app itself is untouched, so it keeps running on the mock exactly as before.

Build:
1. The workspace (section 1.5), crates `rain-types`, `rain-pipe`, `rain-backend` with all twelve module stubs and empty `Slot`s (no module has a service yet), `.cargo/config.toml`, `clippy.toml`, `rustfmt.toml`.
2. `rain-types`: `ApiError`, `ErrorCode`, `Frame`, the `Topic` trait and the topic struct `HealthTopic` (every other topic struct is added to `events.rs` in the stage that adds its payload type: stage 3 `Scan`, 4 `Launch`, 5 `Projects` and `Games`, 6 `Jobs` and `IdleJobs`, 7 `Habits`, 8 `Sessions`, 9 `GameModeTopic`, 1b `SettingsTopic`; section 5.3 has the naming rule), `Ping`, `Health`, `ShellFacts`, `IconRef` with its enum `IconKind`, `Status`, `Settings` and `SettingsPatch`, `env.rs`, `time.rs`, `limits.rs`, and the generator with the drift check (sections 9.3, 9.4).
3. Backend: `config.rs`; `core/` (state, bus, error with the extractors, health, clock, shutdown, logging); `server/` (pipe with the DACL and 8 acceptors, router with layers and fallbacks, `GET /ping`, `GET /health`, `GET /stream` with `hello`, `resync`, keep-alive, lag, the `lifecycle` tail; `POST /internal/shutdown` and `rain-backend --stop` in their basic form, section 2.6); `platform/pipe_acl.rs`. No SQLite, no `notify` watcher, no media registry, no `TomlFile`.
4. `rain-pipe`: the pooled `PipeClient` and `run_stream` (copy the spike).
5. The integration harness (section 12.2) and the first tests: ping, error bodies for each failure path, stream `hello` then `resync`, lag, shutdown with a stalled stream, a second backend exits, the user-only DACL read back, `crate_graph`, `source_rules` (the scan exists but has nothing to scan yet).
6. Scripts: `check.mjs`, `smoke.mjs` (backend half), the cargo npm scripts of section 2.8 (`dev.mjs` waits for 1b).

Routes that go real: none (no `RainApi` method changes). Nothing is deleted.

Acceptance:
1. `npm run check` passes (the smoke check starts the backend binary on a unique pipe, reads `hello` and `resync`, stops it with `--stop`).
2. `npm run tauri dev` still works with the workspace layout, and the app behaves exactly as before (Cargo.lock is at the root, `src-tauri/Cargo.lock` is gone). If it does not, the fallback of section 1.5 is used before anything else is built.
3. A second `rain-backend.exe` on the same pipe exits quietly; `--stop` ends the first within 5 s; `GET /ping` over the pipe shows `sandbox: true` in a debug build.
4. The strict type-identity check passes for the stage's types, and `npm run types:check` fails after a hand edit of the generated file.
5. Idle measurement, quick version: `rain-backend.exe` idle for 60 s with no client stays under 30 MB working set and under 0.1 s CPU time gained.

#### Stage 1b: the bridge and the first real call (settings)

Goal: prove the whole chain: frontend, bridge, pipe, backend, TOML file, stream, restart. One real resource, no feature logic beyond settings storage; nothing that needs COM, SQLite, `notify` or the registry.

Build:
1. Backend: `core/paths.rs` (`PathProblem`, `normalize(raw, allow_unc)` and the table test of section 05 (its tests section); the settings validation of section 06 table V needs it) and `core/throttle.rs` (`Throttle` with `publish` and `publish_final`; the topic `settings` needs it); `store/toml_file.rs` and `hub.rs` without the watcher (`TomlFile::refresh()` of section 10.4 stands in for it), `settings` module: `GET /settings` and `PATCH /settings` over `settings.toml` for all user-data fields; `hotkeyAvailable` from `shell_facts`; `startWithWindows` returns false and a patch of it answers `503 notBuilt` until stage 2; `dataDir` (stage 2) and `monitorClaude` (stage 8) also answer `503 notBuilt`, `monitorClaudeState` is `off`, and `dataDir` returns the effective folder from `Config`; `steamPath` and `epicPath` return the override or empty (section 06 A11); `calendarConnected` is false (no registry, no detection yet); validation of the two number unions (the `serde_repr` types reject other numbers); an unknown key or a read-only key in a patch is `invalid`; topic `settings` (own writes) and topic `health`. Core (`server/builtin.rs`, not the settings module) adds `PUT /internal/shell`, which stores `ShellFacts` in `state.shell_facts`, publishes `health`, then calls `settings::Service::on_shell_facts_changed()` (the settings module publishes `settings` when `hotkeyAvailable` changed; section 11.2).
2. Shell: `backend.rs` (ensure, spawn with the proven flags, state machine, guard, no upgrade rule yet), `bridge.rs` (`api_request` and `GET /local/backend`), `relay.rs`, `facts.rs`, `logging.rs`, the `--hidden` flag (needed by the smoke check; the logon autostart that uses it stays in stage 2), `build.rs` manifest and capability. This part is where the Tauri-specific unknowns are settled (appendix C): the command manifest names, `core:default` and `listen`, the Tauri CLI with the moved target folder.
3. Frontend: `Bridge` and both bridge files, `rest/`, `router.ts`, `routing.ts`, `index.ts`, `main.ts` wiring, `vite/pipe-proxy.ts`, `types.ts` becomes the barrel for the stage-1 types, `vitest` with the first tests. `src/data/rest/topic-check.ts` is created with the `UnportedTopic` union of section 9.3.
4. Scripts: `dev.mjs`, `backend.mjs` (with the option `--fixture typical|empty|broken`, section 12.3), `smoke.mjs` (all five steps), the npm script `dev:fixture` (`dev.mjs` started on a fixture; the acceptance checks of stages 1b to 9 depend on it), and `check.mjs` gets the duplicate-name check of section 8.8.

Routes that go real: `getSettings`, `updateSettings`; topics `health` and `settings` (own writes). Deleted from the mock: nothing (the mock's settings slice stays: other slices read it).

Not in stage 1: SQLite, the `notify` watcher, media, autostart, the upgrade rule, the installer.

Acceptance:
1. `npm run check` passes (all five smoke steps included).
2. `npm run tauri dev`: Settings opens with values from the backend. Change "Return to Home"; the value stays after `npm run backend:restart`.
3. The same Settings page works in a plain browser at `http://localhost:1420` through the proxy; a request with a foreign `Origin` to `/api/v1/settings` gets `403`.
4. Stop the backend (`npm run backend:stop`): Settings shows its error row; "Retry" in the window starts the backend again (shell) and the page recovers; in the browser Retry recovers after `npm run backend:start`.
5. Kill the backend process by pid in Task Manager: the shell restarts it within about 5 s, `hello` has a new `startId`, and a resource on screen refetches.
6. Edit `settings.toml` by hand while the backend runs: the next Retry or refresh shows the edit (through `refresh()`; the live reload without a click arrives in stage 2).
7. The measured round trip of `api_request` for `GET /ping` from the page is written into the stage report (the spike measured only Rust to Rust: 0.03 ms).

### 14.2 Stage 2: lifecycle, storage engine, packaging

Goal: the always-on process behaves: shutdown with module budgets, upgrade, autostart, logs, database, file watching; idle cost measured.

Build: `--stop --force` and the full shutdown order of section 2.6 (module stop budgets, checkpoint), the upgrade rule, spawn guard polish, autostart (registry value through `winreg`, `StartupApproved`), the `hotkeyAvailable` report on every `hello`, SQLite (`Db`, migration runner, `core_migrations`, corruption policy, checkpoint on stop), the `notify` watcher and `TomlFile` live reload and `ConfigIssue`s, the media registry route and the `rain-media` handler (with a test provider only in tests), log rotation with the size budget, an idle measurement script, the `lifecycle` module (`on_shell_facts`: autostart self-heal and the `hotkey` health source), settings detection (vault, Steam, Epic, daily-note folder and format), the data folder change with `machine.toml`, `getHealth` (the config-issue line in Settings) and the `settings` live reload (section 06 1.4).

**Status: stage 2b is NOT built; a small install script stands in for it.** `npm run install:local` (`scripts/install.ps1`) builds the three release programs, copies them into `%LOCALAPPDATA%\Programs\Rain` and adds a Start menu shortcut. It needs no administrator rights, stops a running Rain before it replaces the files, and never touches the data folder or the settings in `%APPDATA%\Rain`. After it, the `target` folder can be deleted (`cargo clean`): the installed programs do not need the repository. `npm run install:local -- -Uninstall` removes the folder and the shortcut, and refuses while "Start with Windows" is on. Without the script, Rain runs from the repository (`npm run dev`, `npm run backend:start`, or `cargo build` and starting the programs from `target`). A plain `cargo build` builds the shell, the backend and `rain-hook` (all three are default members), and the three programs must stay in one folder: the shell finds `rain-backend.exe` next to itself and the session monitor finds `rain-hook.exe` next to the backend. An installer built by `tauri build` today would contain neither program. Before removing Rain from a PC, switch "Monitor Claude sessions" off in Settings first: this removes the hooks from the Claude settings file and the copied hook program; deleting the folders by hand leaves hook entries that point at a missing program.

Stage 2b (packaging; not on the critical path, may be done any time before a release, and no stage from 3 to 10 depends on it): `tauri.bundle.conf.json`, `scripts/bundle.mjs`, the NSIS pre-install stop of the old backend, the check that the sidecars land next to the main binary.

Routes real: `getHealth` (`GET /health`; it joins `RainApi`, the mock never had it, so `RetiredMockMethod` and `MockApi` are created here with `'getHealth'`, section 8.6), `GET /media/{kind}/{id}` (no kinds yet in production), `startWithWindows` becomes effective in `PATCH /settings` (real mode only).

Deleted from the mock: nothing.

Acceptance: a second backend exits quietly (already true since 1a); version mismatch triggers upgrade, `busy` blocks it; a hand edit of `settings.toml` shows in the UI within 1 s without a click; a broken `settings.toml` keeps the last good value and shows in `GET /health`; `rain-backend.exe` idle for 60 s: under 30 MB working set and under 0.1 s CPU time gained (`Get-Process`); the backend started by the logon Run key (checked by hand, appendix C) is alive after the Run-key shell exits; installer build (2b) starts the shell and backend from the install folder (manual check).

### 14.3 Stage 3 to 10

| Stage | Goal | Routes that go real | Mock deleted when done | Acceptance (besides the checks of section 12.4) |
|---|---|---|---|---|
| 3 Read-only sources | Disk and machine facts without personal data; first use of the COM thread and large payloads | `listDrives`, `startScan`, `getScan`, `cancelScan` (+ node route), `revealPath`, `getMachineState` (route `/machine`), `checkPaths` (`POST /paths/check`); topic `scan`. There is no `machine` topic (decided here: the page asks `GET /machine` when it needs it, and the idle detection of stage 6 reads the machine facts inside the backend); section 05 may ask for one only through the architect | `mock/slices/disk.ts`, `mock/seed/disk.ts` (no stub; `MockApi` and `RetiredMockMethod` of section 8.6 were created in stage 2 and are extended here), `mockControl.pathExists` and its use in `SettingRow.vue` | scan of a test folder in the fixture matches a known tree; cancel works; a frame over 256 KiB cannot occur; reveal opens Explorer on a temp file; the path glyphs in Settings call `checkPaths` |
| 4 Apps, pools, pins, launch | Find and start apps; first media (icons) | `listApps`, `listPools`, `savePool`, `deletePool`, `rescanApps`, `launch` and `getLaunch` for targets `app` and `pool`, `listPinned`, `setPinned` for `app` and `pool`, `listRecent`, `search` for `app` and `pool`; topic `launch`; media kind `app-icon` | `seed/apps.ts` emptied; the rest per section 01 9.6 (stage 5: `launches.ts`, app and pool methods of `launcher.ts`, and their method names join `RetiredMockMethod`; stage 8: the router special cases for `search`, `listPinned`, `listRecent`; stage 10: the remainder). The router special cases for `search`, `listPinned`, `listRecent`, `setPinned`, `launch` start here | pins and pool members survive a rescan and a restart; a launched classic program (a small test exe; NOT Notepad, a packaged app that always survives, spike process) survives a backend stop and is not a member of any job; an icon loads in `tauri dev` and in the browser; the property check "every id from search, pinned, recent exists in its list" |
| 5 Projects and games | Projects from `projectRoots`, git facts, Steam and custom games | `listProjects`, `listGames`, `saveCustomGame`, `setGameHidden`, `updateGame`, `deleteCustomGame`, `POST /projects/refresh`, `POST /games/refresh`, `launch` for targets `project` and `game`, `setPinned` for those types, `search` for those types; media kinds `game-cover`, `project-icon` if needed; topics `projects` and `games` | `mock/slices/library.ts`, `seed/projects.ts`, `seed/games.ts`, `mock/covers.ts` (at the end of stage 5, section 02); also `mock/slices/launches.ts`, `seed/apps.ts` and the app and pool methods of `mock/slices/launcher.ts` (section 01 9.6; `markGamePlayed` moves into `slices/gameMode.ts` first, section 02 9.6); the router special cases for `launch`, `getLaunch`, `subscribe('launch')` and `setPinned` are deleted; the stage 5 names of the table in section 8.6 (including `launch` and `getLaunch`) join `RetiredMockMethod` | a fixture folder with three repositories lists them with branch, dirty count and 14-day commits; Steam installed games appear with local covers; Epic missing is `sources` "off", not an error; opening a project starts the editor and ONE Windows Terminal tab in the folder in which Claude Code runs (the tab stays open as a shell after Claude exits) |
| 6 Jobs and idle jobs | Scheduler and idle jobs | `listJobs`, `setJobEnabled`, `runJob`, `getRunLog`, `listIdleJobs`, `setIdleJobEnabled`, `runIdleJob`, `getIdleJobLog`; `search` for the types `job` and `idleJob` (`launcher/search.rs` maps `jobs().list()` and `idle().list()`, section 01 6.3; `REAL_SEARCH` of the router gains both); topics `jobs`, `idleJobs` | `mock/slices/jobs.ts`, `seed/workflows.ts` (job and idle parts), the jobs part of `mock/logs.ts` | a job due in 5 s (fixture, fake clock in tests) runs once; disabling writes `enabled = false` into the TOML keeping comments; a job missed while the backend was off runs once at next start; idle timer exists only while an idle job is enabled |
| 7 Life | Habits from the vault, daily notes | `getVaultStatus`, `listHabits`, `rereadVault`, `listHabitSuggestions`, `saveHabit`, `deleteHabit`, `getHabitStats`, `setHabitDone` (section 04), `listHabitDays`, `listEvents` (empty, route kept), `openDailyNote` | `mock/slices/life.ts`, `seed/life.ts` | a tick changes one property and no other byte of the fixture note (byte-compare test); a missing note is created with only the habit properties; `source_rules` passes; the vault is never written in sandbox outside the fixture |
| 8 Claude sessions | Sessions, transcript, then hooks | `listSessions`, `getSessionTranscript`, `focusSession`, `stopSession`; then `answerPermission` and `/internal/hook`; `search` for the type `session` (`REAL_SEARCH` gains it, and the router special cases for `search`, `listPinned` and `listRecent` are deleted); topic `sessions`; `rain-hook` crate | `mock/slices/sessions.ts`, the session part of `seed/workflows.ts` and `mock/logs.ts` | registry and transcript read-only first; the hook crate and installer come after the stage-8 manual check below (the spike proved headless behaviour only; the interactive terminal and Desktop behaviour is unproven, appendix C); the hook installer is built and tested against a throwaway folder, the owner turns the Settings switch on the real PC only after a manual interactive check and a first run with the switch on a copy of the settings; tests use throwaway folders and never the real Claude settings file; a hook with Rain stopped costs under 50 ms wall and exits 0 with empty stdout (measured about 20 ms for the spike forwarder; process creation alone is 12 to 17 ms); the install merge and un-merge give byte-identical files on a copy (the spike merge crate has 11 tests: copy them) |
| 9 Game mode | Real wind-down, game launch, shell exit | `getGameMode`, `startGameMode`, `resolveGameMode`, `markGameModeRecapSeen`; topic `gameMode`; the stop reason `gameMode` end to end | `mock/slices/gameMode.ts`, `seed/gameMode.ts`, the "Restart prototype" action, the `AppShell` pause | stops run in the order of section 06; `done` is the last frame; the shell exits after the pipe closes plus 3 s; nothing of Rain is running afterwards; the next start shows what was stopped (section 06); the reset-to-Home hazard of the contract report is fixed |
| 10 Delete the mock | End state: no mock code | none new | `src/data/mock/` entirely, `router.ts`, `routing.ts`, the `VITE_RAIN_MOCK` branch, the six mock commands, the Settings "Prototype" group, the `main.ts` reseed block, `mockControl` imports, `localStorage 'rain.prototype'` note, the `dev:ui` script and the mock note of `VITE_RAIN_MOCK`, and every hand-written type left in `src/data/types.ts` (it becomes the barrel only, and `topic-check.ts` stays); `docs/ui-spec.md`: each section's replacement text (01 9.7, 02 9.7, 03 9.4, 04 9.10, 05 9.10, 06 9.6) is applied in the stage that builds its screen change (the heading of each block names the stage; earlier stages change only the table rows they add), and sections 7.8 and 7.9 carry the final text of section 06 9.6 at stage 10 (7.9 then describes the final row list, including the rows 'Daily notes folder', 'Game folders' and 'Monitor Claude sessions', which do not exist before stages 5, 7 and 8), section 8 is the new Data layer text, and the old section 9 tree is marked historical | `grep -r "data/mock\|mockControl" src` finds nothing; all checks pass; every screen shows real data or a true empty state |

Shared core files and the stage that builds them (they are listed in section 11.1; a stage that needs one earlier builds it then): `core/paths.rs` and `core/throttle.rs` in stage 1b; `core/launch_contract.rs` in stage 4 (the traits and types of section 01 6.9; the `Service` stubs of projects and games implement the traits there with empty answers, so stage 4 compiles before stage 5); `core/winddown.rs` in stage 6 (the types of section 06 6.7.1; `jobs` and `idle` implement `WindDownPort` there, and `gamemode` uses it in stage 9).

Order reason: stages 1 and 2 retire the transport and lifecycle risks that every later stage needs. Stages 3 to 5 use data that is not personal or is read-only. Stage 7 and 8 come late because they touch the vault and the Claude configuration and need the user's decisions to be built and tested first. Stage 9 needs every other stage.

### 14.4 Route registry (all methods of `RainApi`, final routes)

This table is the contract between the REST client and the modules. Comments in `api.ts` are replaced by it. A section may add routes in its own area; it may not rename a row without a contract change. Paths are after `/api/v1`. "New" marks a route not in the current `api.ts` comments.

| `RainApi` method | HTTP | Module | Section | Notes |
|---|---|---|---|---|
| `search` | `GET /search?q=&types=` | launcher | 01 | `types` comma separated |
| `listPinned` | `GET /pinned` | launcher | 01 | |
| `listRecent` | `GET /recent?limit=` | launcher | 01 | |
| `setPinned` | `PUT /pinned/{type}/{id}` | launcher | 01 | body `{ "pinned": boolean }`, 204 |
| `listApps` | `GET /apps` | launcher | 01 | |
| `listPools` | `GET /pools` | launcher | 01 | |
| `savePool` | `POST /pools` (create, 201) or `PUT /pools/{id}` | launcher | 01 | |
| `deletePool` | `DELETE /pools/{id}` | launcher | 01 | 204 |
| `launch` | `POST /launches` | launcher (projects and games implement `LaunchPlanner` and `ItemSource` for their targets) | 01, 02 | 201 new run, 200 the running run is returned |
| `rescanApps` (new) | `POST /apps/scan` | launcher | 01 | |
| `getLaunch` | `GET /launches/{id}` | launcher | 01 | |
| `listProjects` | `GET /projects` | projects | 02 | |
| `listGames` | `GET /games` | games | 02 | |
| `saveCustomGame` | `POST /games` | games | 02 | 201 |
| `setGameHidden` | `PATCH /games/{id}` | games | 02 | body `{ "hidden": boolean }` |
| `updateGame` (new) | `PATCH /games/{id}` | games | 02 | same route as `setGameHidden`, wider body (section 02) |
| `deleteCustomGame` (new) | `DELETE /games/{id}` | games | 02 | 204 |
| (refresh) | `POST /projects/refresh`, `POST /games/refresh` | projects, games | 02 | |
| `listSessions` | `GET /sessions` | sessions | 03 | |
| `answerPermission` | `POST /sessions/{id}/permission` | sessions | 03 | body `{ "requestId", "answer" }` (contract change) |
| `focusSession` | `POST /sessions/{id}/focus` | sessions | 03 | returns `{ "focused": "window"\|"app"\|"none" }` (contract change) |
| `stopSession` | `POST /sessions/{id}/stop` | sessions | 03 | 204 |
| `getSessionTranscript` | `GET /sessions/{id}/transcript` | sessions | 03 | |
| `listJobs` | `GET /jobs` | jobs | 03 | |
| `setJobEnabled` | `PATCH /jobs/{id}` | jobs | 03 | body `{ "enabled": boolean }` |
| `runJob` | `POST /jobs/{id}/runs` | jobs | 03 | 201 |
| `getRunLog` | `GET /runs/{id}/log` | jobs | 03 | |
| `listIdleJobs` | `GET /idle-jobs` | idle | 03 | |
| `setIdleJobEnabled` | `PATCH /idle-jobs/{id}` | idle | 03 | |
| `runIdleJob` | `POST /idle-jobs/{id}/run` | idle | 03 | |
| `getIdleJobLog` | `GET /idle-jobs/{id}/log` | idle | 03 | |
| `getMachineState` | `GET /machine` | machine | 05 | |
| `listHardware` | `GET /hardware` | hardware | 05 | read at the call, no cache (section 05, 13) |
| `openHardware` | `POST /hardware/{id}/open` | hardware | 05 | starts what a tile of the last read opens |
| `getScanNode` (new) | `GET /scans/{id}/node` | disk | 05 | lazy subtree route |
| `getVaultStatus` | `GET /vault` | life | 04 | |
| `listHabits` | `GET /habits` | life | 04 | |
| `saveHabit` (new) | `POST /habits` (create) or `PUT /habits/{id}` | life | 04 | |
| `deleteHabit` (new) | `DELETE /habits/{id}` | life | 04 | |
| `getHabitStats` (new) | `GET /habits/{id}/stats` | life | 04 | |
| `setHabitDone` (new) | `PUT /habits/{id}/days/{date}` (the tick) | life | 04 | |
| `rereadVault` (new) | `POST /vault/reread` | life | 04 | |
| `listHabitSuggestions` (new) | `GET /vault/properties` | life | 04 | |
| `listHabitDays` | `GET /habit-days?from=&to=&habit=` (`habit` optional) | life | 04the query `habit` is new |
| `listEvents` | `GET /calendar/events?from=&to=` | life | 04 | route renamed (the comment said `/events`, which is now free of confusion with the stream); returns `[]` this round |
| `openDailyNote` | `POST /daily-notes/{date}/open` | life | 04 | |
| `listDrives` | `GET /drives` | disk | 05 | |
| `startScan` | `POST /scans` | disk | 05 | 201 |
| `getScan` | `GET /scans/{id}` | disk | 05 | |
| `cancelScan` | `DELETE /scans/{id}` | disk | 05 | 204 |
| `revealPath` | `POST /shell/reveal` | disk | 05 | body `{ "path" }`, 204 |
| `getGameMode` | `GET /game-mode` | gamemode | 06 | |
| `startGameMode` | `POST /game-mode` | gamemode | 06 | body `{ "thenLaunchGameId"? }` |
| `resolveGameMode` | `POST /game-mode/choice` | gamemode | 06 | body `{ "choice" }` |
| `getSettings` | `GET /settings` | settings | 06 | |
| `getHealth` (new) | `GET /health` | core | 00 (06 uses it) | stage 2; the Settings screen shows one info line when `configIssues` is not empty (section 06) |
| `markGameModeRecapSeen` (new) | `POST /game-mode/recap/seen` | gamemode | 06 | |
| `updateSettings` | `PATCH /settings` | settings | 06 | |
| `checkPaths` (new) | `POST /paths/check` | disk | 05 | body `{ "paths": string[] }` (at most 32, 1024 characters each), stage 3; replaces the synchronous `mockControl.pathExists`; section 06 builds on it and does not define it |
| `subscribe` | `GET /stream` | core | 00 | |
| (core) | `GET /ping`, `GET /health`, `GET /media/{kind}/{id}` | core | 00 | |
| (internal) | `PUT /internal/shell`, `POST /internal/shutdown` | core (`server/builtin.rs`, stage 1b for the first) | 00 | never from the page. `PUT /internal/shell` stores `ShellFacts` in `state.shell_facts`, publishes `health`, then calls `settings::Service::on_shell_facts_changed()` (publishes `settings` only when `hotkeyAvailable` changed; stage 1b) and, from stage 2, `lifecycle::Service::on_shell_facts` (autostart self-heal and the `hotkey` health source only) (11.2) |
| (internal) | `POST /internal/hook` | sessions | 03 | from `rain-hook` only; body limit 4 MiB |
| (internal, sandbox only) | `PUT /internal/test/apps-source` | launcher | 01 | test fixture for the app list; absent in a real build |
| (shell-local) | `GET /local/backend` | shell | 00 | answered by the shell itself (section 6.1); does not exist in the backend or in the browser bridge |

Not in the registry on purpose: `GET /claude-monitor` and `POST /claude-monitor/repair` (the Monitor Claude switch is the setting `monitorClaude`, section 06; section 03 offers `install_hooks`, `remove_hooks` and `hooks_state` to it).

---------------------------------------------------------------------------------------------------

## 15. Changes to the TypeScript contract required by the foundation

The contract (`src/data/api.ts`, `src/data/types.ts`) may change where a real backend needs it (decision). Every change is listed with the frontend files it touches.

| # | Item | From | To | Reason | Frontend files touched | Stage |
|---|---|---|---|---|---|---|
| F1 | `RainError` | `{ code: 'unavailable'\|'notFound'\|'failed'; message; source? }` | `ApiError` generated: code adds `'invalid'` and `'conflict'`; adds `reason?: string`; `RainError` is an alias | the backend needs 400 and 409 classes and a machine tag | `src/data/api.ts`, `src/data/types.ts`; no screen switches on `code` today (checked by search for `.code`), so none | 1 |
| F2 | `Shell` | no bridge | adds `readonly bridge: Bridge` | the single place that picks Tauri or browser | `src/shell/shell.ts`, `tauri.ts`, `browser.ts`, new `bridge.ts`, `tauri-bridge.ts`, `browser-bridge.ts` | 1 |
| F3 | `src/data/types.ts` | hand-written entities | barrel: `export * from '../bindings/rain-types'` (one generated file), plus the hand-written `ID`, `ISODate`, `ISODateTime` and the not-yet-ported types | Rust is the source of truth | `src/data/types.ts` only (names unchanged); new generated `src/bindings/rain-types.ts`; types leave `types.ts` stage by stage | 1a to 10 |
| F4 | `api.ts` route comments | per-method comments | replaced by the route registry (section 14.4); `listEvents` route is `/calendar/events` | one place for routes | `src/data/api.ts` (comments only) | 1 |
| F5 | `data/index.ts` | `createMockApi()` | router, then `createRestApi` | section 8 | `src/data/index.ts`, new `src/data/rest/*`, `router.ts`, `routing.ts` | 1, 10 |
| F6 | `subscribe` plus resets | `subscribe` only | `subscribe` unchanged; adds exported `onStreamReset(cb)` | repair after reconnect and restart | `src/data/index.ts`, `src/main.ts` | 1 |
| F7 | `TopicPayload` / `Topic` | six topics | adds `health: Health` | config issues and sources | `src/data/api.ts`; `src/data/mock/engine.ts` (`listeners` entry `health: new Set(),` with the comment "The mock never publishes these; the router sends them to the backend."); consumers none yet | 1 |
| F8 | `IconRef` kind `url` and `Game.coverUrl` | image URL or data URI | relative `media/...` reference, resolved by the REST client to a loadable URL | binary resources (section 4.7) | none in screens (rewrite in `rest/media.ts`); `RIcon.vue`, `GameTile.vue`, `GameDetail.vue`, `GameModeScreen.vue` keep reading the same fields | 4 |
| F9 | `Settings` | hand-written | generated `Settings` and `SettingsPatch`; `updateSettings(patch: SettingsPatch)`; read-only fields are rejected by the backend (details: section 06) | one source of truth | `src/data/api.ts`, `src/features/settings/*` only if section 06 changes the shape | 1 |
| F11 | topics `projects`, `games` (02), `habits` (04), `settings` (06) | six topics and `health` | adds `projects: Project[]`, `games: GameLibrary`, `habits: HabitSummary[]`, `settings: Settings` | whole-list snapshots, section 5.3 | `src/data/api.ts` (`TopicPayload`), `src/data/routing.ts`; consumers per section | 5, 7, 1b |
| F10 | `package.json`, `vite.config.ts`, `tauri.conf.json`, `capabilities/default.json`, `.gitignore` | as today | scripts (section 2.8), proxy plugin, `beforeDevCommand` stays `npm run dev`, one capability entry (`allow-api-request`), ignore `.rain-dev/`, `src-tauri/binaries/` | tooling | those files | 1a, 1b, 2 |

Everything else (`checkPaths`, `focusSession` result, `answerPermission` request id, scan truncation and node route, `Project.lastOpenedAt` optional, calendar methods, game mode `launchError`) belongs to the feature sections. They use this table:

### 15.1 Template for the contract changes of a feature section

| # | Item (type or method) | From | To | Reason | Frontend files touched (full paths) | Breaking for the mock? | Stage |
|---|---|---|---|---|---|---|---|
| S.1 | | | | | | yes/no (if yes, how the router or the mock adapts) | |

Rules: one row per change; "files touched" lists every file that must change, found by search, not by memory; a change that makes the mock fail to type-check must say how the mock is adjusted in the same stage (the mock stays compilable until stage 10).

---------------------------------------------------------------------------------------------------

## 16. Template for the feature sections

Each section file starts with a header block (title, area, modules, author, date, status) and then has exactly these headings, in this order. An empty heading says "none" and why. Use tables and code blocks for exact shapes.

1. Scope. What the area does, what it does not. The methods of `RainApi` and the topics it covers. The modules it owns. The decisions of `decisions.md` it implements.
2. Routes. A table with the columns: Method, Path, Request (query or body type), Response (type), Success status, Errors (code and reason), Idempotent, Topic effects. Then "Live topics": for each topic its name, payload type, "whole list" or "one item and key", maximum rate, who publishes and when. Every route of the area is here and in the registry of section 14.4.
3. Types. The Rust types to add to `rain-types` (names equal to TypeScript names), with every `#[ts(type = ...)]` override and every validation rule. Number unions and optional fields follow section 9.2.
4. Data files. For each TOML file: name (a row of section 10.2), full schema with every key, type, default and validation, a commented example, `schema_version` and migration notes, who writes it and when, and what a hand edit may do.
5. Tables. For each SQLite table: name (rule of section 10.5), columns, indexes, retention, rebuildable or not, the module's migration list.
6. Algorithms. The precise rules: how data is found, ranked, ordered, merged, id issued and kept stable, how a state machine moves. Pseudocode where it helps. Numbers (limits, timeouts) in a table.
7. Background work. A table: task, trigger (event or timer), which timer row of section 11.5 allows it, what it costs, what it does in game mode, how it stops. No row may use a timer that section 11.5 does not allow.
8. Errors. A table of every error the area can produce: situation, status, `code`, `reason`, `message` text, what the UI shows. Degraded sources (a source that is off or failed) and how they appear in `GET /health` `sources`.
9. Frontend changes. Each file to change with what changes, the contract-change table of section 15.1, the mock adaptation, and the stage in which each step happens.
10. Tests. Unit tests (what is pure), integration tests (routes, topics, fixtures to add), the traits and fakes needed (section 12.2), property tests, and what cannot be tested automatically and how it is checked by hand.
11. Acceptance checks. The checks of the stages that build this area, written so a person or a script can run them: command or action, expected result.
12. Open points. Every question left, each with: the question, who decides (architect, another section, the user), what the code does until it is decided, and which choice it blocks. A section may not leave open anything a later section depends on.

A section must not redefine anything of this document. If it needs a new shared rule, a new data file, a new topic, a new secret name, a new dependency, or a new timer, it asks for it in "Open points" and in the relevant table row; the architect updates this file.

---------------------------------------------------------------------------------------------------

## Appendix A. Constants

All constants live in `rain_types::limits` (one file) so the shell, the backend and the tests agree.

| Name | Value | Used in |
|---|---|---|
| `API_PREFIX` | `/api/v1` | everywhere |
| `PIPE_PENDING_INSTANCES` | 8 | 3.3 |
| `PIPE_MAX_CONNECTIONS` | 64 | 3.3 |
| `PIPE_BUSY_RETRY`, `PIPE_BUSY_DEADLINE` | 5 ms, 3 s | 3.3 |
| `CLIENT_POOL_IDLE_PER_HOST`, `CLIENT_POOL_IDLE_TIMEOUT` | 8, 8 s | 6.10 |
| `CLIENT_MAX_INFLIGHT` | 32 | 6.1 |
| `HEADER_READ_TIMEOUT` | 10 s | 3.3 |
| `BODY_LIMIT` | 1 MiB (raise to 4 MiB per route) | 3.4 |
| `REQUEST_TIMEOUT` | 15 s | 3.4 |
| `BRIDGE_TIMEOUT` | 20 s default, 60 s maximum | 6.1 |
| `RESPONSE_MAX` | 16 MiB | 3.4, 6.6 |
| `URL_MAX` | 2048 bytes | 3.4, 6.1 |
| `SSE_KEEPALIVE` | 25 s | 5.4 |
| `STREAM_DEAD_AFTER` | 60 s | 5.4 |
| `STREAM_BACKOFF` | 0.5 s, 1 s, 2 s, then 5 s | 5.5 |
| `BUS_CAPACITY` | 64 frames | 5.4 |
| `FRAME_MAX` | 256 KiB | 5.3 |
| `TOPIC_RATE_MAX` | 4 per second | 5.3 |
| `BACKEND_READY_WAIT` | 5 s (poll 50 ms) | 2.2 |
| `SPAWN_GUARD` | 3 spawns per 60 s | 2.2 |
| `UPGRADE_WAIT` | 5 s | 2.5 |
| `MODULE_STOP_BUDGET` | 5 s | 2.6 |
| `SHUTDOWN_BUDGET` | 8 s | 2.6 |
| `GAME_MODE_EXIT_GRACE` | 3 s | 2.6 |
| `TOML_DEBOUNCE` | 300 ms | 10.4 |
| `TOML_WRITE_RETRIES` | 8 attempts, 25 ms apart | 10.4 |
| `SQLITE_CACHE_KIB` | 2048 | 10.5 |
| `LOG_FILES_KEPT` | 7 | 10.7 |
| `LOG_SIZE_BUDGET` | 20 MB | 10.7 |
| `LOG_QUEUE_LINES` | 8192 (lossy) | 10.7 |
| `HOOK_WAIT` (forwarder waits for Rain's answer), `HOOK_HANDLER_TIMEOUT` (in the installed JSON) | 20 s, 35 s | decision C2, section 03 |
| `SCAN_ARENA_TTL` | 10 min | 11.5 |
| `SCAN_TREE_BUDGET`, `SCAN_PER_FOLDER`, `SCAN_OPEN_FOLDER_CHILDREN` | 2000 nodes, 12 children for a deeper folder, 200 for the open folder (about 350 KB of JSON) | section 05 |
| `SCAN_KEEP_FILES_PER_DIR`, `SCAN_MAX_DEPTH` | 64, 256 | section 05 |
| `PATH_PROBE_TIMEOUT`, `PATH_PROBE_PARALLEL`, `PATHS_CHECK_MAX`, `PATH_MAX_CHARS` | 2 s, 4, 32, 1024 characters | section 05 |
| `HOOK_SERVER_WAIT` | 19 s | section 03 |
| `GAME_MODE_WAIT_LIMIT` | 180 s | section 06 |
| `REVIEW_ABANDON` | 10 min | section 06 |
| `GAME_MODE_ITEM_BUDGET`, `GAME_MODE_LAUNCH_BUDGET`, `GAME_MODE_ITEM_MIN_VISIBLE` | 5 s, 10 s, 400 ms | section 06 |
| `RESTART_DELAY` | 500 ms | section 06 |
| `DETECT_TTL` | 30 s | section 06 |
| `GAMEMODE_RECORDS_KEPT` | 20 | section 06 |
| `USER_ACTIVE_BELOW_MS` | 60000 | section 05 |
| `SCAN_WALK_THREADS` | 8 | 11.6 |
| `COM_CALL_TIMEOUT` | 10 s | 11.6 |
| `GIT_MAX_PARALLEL` | 4 | 11.6 |
| `RUNTIME_WORKERS`, `BLOCKING_MAX`, `BLOCKING_KEEPALIVE` | 2, 8, 5 s | 11.5 |
| `JSON_INT_MAX` | 2^53 - 1 | 3.4, 9.2 |

## Appendix B. Names and places at a glance

| Thing | Real | Sandbox |
|---|---|---|
| Shell binary | `rain.exe` (installed name may be `Rain.exe`) | `target\debug\rain.exe` |
| Backend binary | `rain-backend.exe` next to the shell | `target\debug\rain-backend.exe` |
| Hook binary | `rain-hook.exe`; copied to `<local>\bin\` when hooks are installed (section 03) | `target\debug\rain-hook.exe` |
| Pipe | `\\.\pipe\rain-<user>` | `\\.\pipe\rain-dev-<user>` or `RAIN_PIPE` |
| Custom scheme | `rain-media` -> `http://rain-media.localhost/media/...` | same; browser: `/api/v1/media/...` |
| Tauri events | `rain:summoned`, `rain:hidden`, `rain:stream`, `rain:backend` | same |
| Tauri commands | `api_request` | same |
| Roaming folder | `%APPDATA%\Rain` | `%APPDATA%\Rain-dev` |
| Local folder | `%LOCALAPPDATA%\Rain` | `%LOCALAPPDATA%\Rain-dev` |
| Logs | `<local>\logs\{backend,shell,hook}.log` | same under `Rain-dev` |
| Database | `<local>\rain.db` | same under `Rain-dev` |
| Autostart value | `HKCU\...\Run\Rain` | refused |
| Credential service | `rain` | `rain-dev` |

## Appendix C. What is still unproven (and what depends on it)

The spikes closed the questions of draft 1 (see appendix D). These claims are still not proven. Code that depends on one handles it defensively, as the last column says.

| Claim (not seen working) | Depends on it | Defensive rule until proven | Where it is checked |
|---|---|---|---|
| The Tauri parts: the `api_request` command, `app.emit` relay, the `rain-media` scheme handler, `core:default` covering `listen`, the command manifest permission name, webview IPC latency (the spikes measured Rust to Rust only) | sections 6 and 8 | fallbacks are written in 6.4; the REST client never assumes a latency | stage 1b |
| The Tauri CLI accepts the workspace root at the repository root and finds the moved target folder | section 1.5 | fallback in 1.5 (the workspace root stays in `src-tauri`) | stage 1a, acceptance 2 |
| An `<img>` from `http://localhost:1420` loads `http://rain-media.localhost/...` without a CSP entry; the exact URL form on Windows with `useHttpsScheme = false` (read from the Tauri source, never run) | section 6.3 | add `img-src 'self' data: http://rain-media.localhost` to the CSP | stage 4, first icon |
| The Vite proxy to the pipe, its origin guard and SSE through it | section 7 | none (development only) | stage 1b |
| The hyper `header_read_timeout` with `TokioTimer`, the 64-connection cap, the `tower::timeout` and `tower_http::limit` layers (not in any spike) | section 3.4 | a test for each in stage 1a; if one fails the limit is dropped and written here | stage 1a |
| A real second Windows account, an elevated administrator, rights narrower than `GA`, name squatting by another user | sections 3.1 and 3.2 | the restricted-token and network-path tests stand in; documented as accepted in 13.1 | not planned |
| The backend started from the logon Run key, from Task Scheduler, or from an elevated shell; whether `CREATE_BREAKAWAY_FROM_JOB` is allowed there | sections 2.3, 2.9 | the retry without breakaway is in the product; a backend that dies with its parent job is restarted by the shell (`down` then `ensure_backend`) | stage 2, by hand |
| A non-idempotent `POST` on a pooled connection that just died | section 6.10 | `api_request` retries only a failed pipe open, never a written request | stage 1b test |
| Ctrl+C, Ctrl+Break, logoff and system shutdown of the backend; a client frozen mid-frame; the grace-timeout abort path | section 2.6 | every write is atomic (10.4), so a hard end loses at most the last change | not planned |
| A soak of hours: stream memory growth, antivirus or EDR interference; idle CPU of the real design (the spike ran a 1 s ticker) | section 11.5 | the stage 2 measurement, repeated after every stage that adds a task | stage 2 and later |
| `notify` 9.0.0 over days and on a network drive (it was one day old); `toml` crate for typed reads | section 10.4, 1.6 | fallbacks in 1.6 (notify 8.2.2 and debouncer 0.7.0; `toml_edit::de`) | stage 1b and 2 |
| `keyring-core` with large values, from two processes, and whether the entry roams | section 10.8 | keep blobs under 2.5 KB; one process only; nothing uses a secret this round | first feature that needs one |
| SQLite after a power loss, and in a OneDrive-synced folder | section 10.5 | the database stays in `%LOCALAPPDATA%`; the corruption policy renames and starts empty | not planned |
| Log timestamps in local time and rotation at local midnight | section 10.7 | UTC is used | not needed |
| Idle time after a long real idle (the 32-bit tick wraps after 49.7 days of uptime, by design) | idle jobs | none needed | stage 6 by hand |
| A scan of a real 1 000 000 file drive with a cold cache (the spike used C:\Windows, 195 236 files, and a synthetic million); hard links and allocation size; below-normal thread priority | section 05 | a scan may take longer than 10 s, so it is already asynchronous; the tree is capped (64 files per folder) and dropped after `SCAN_ARENA_TTL` | stage 3 |
| Steam with several accounts, with Steam closed (`ActiveUser` is 0; fallback `AutoLoginUser` and `loginusers.vdf` was not exercised), or a game with several processes | section 02 | when the signed-in user cannot be found, list the installed games without play time and report `sources` as `ok` with a message | stage 5 |
| `ShellExecute` on a classic AppsFolder item or a `steam://` link; a packaged app with several processes (`ActivateApplication` returns the main pid only); `wt.exe` started directly from a windowless backend; which browser really handles `https` (the registry says Edge, Chrome opened) | sections 01 and 02 | try `shell:AppsFolder\<AUMID>`, fall back to the item's target and arguments; use `ShellExecute` with parameters for `wt.exe` if a direct start fails | stage 4 and 5 |
| `NtResumeProcess` or `PROC_THREAD_ATTRIBUTE_JOB_LIST` instead of the suspended start plus Toolhelp resume | section 11.6 | the proven way is used | not planned |
| Claude Code, INTERACTIVE sessions (terminal and Desktop): that user-level hooks run, hook behaviour in an untrusted folder, the terminal dialog while a `PermissionRequest` hook runs, cancel when the user answers first, the "hook error" notice, Esc and registry `status` idle, live pickup of a changed settings file by running sessions, async `SessionEnd` at exit, killing or focusing Desktop-hosted sessions, a forwarder path with spaces, managed-settings policy | decisions C1, C2; section 03; `rain-hook` | the forwarder always exits 0 with empty stdout; it waits 20 s (`HOOK_WAIT`) with a 35 s handler timeout in the installed JSON; a late answer returns `conflict`, reason `requestExpired`; discovery uses the registry; every payload field is optional | stage 8: manual check by the owner before the switch is turned on |
| The merged settings file itself accepted by Claude Code (the spike merged and un-merged byte-exactly but never ran Claude against the merged file); an unknown JSON field in a hook entry (the marker is an extra `args` entry instead) | section 03 installer | the installer reads the file back after writing; `status` shows missing forwarders | stage 8, throwaway folder |
| Events that did not fire in headless runs: `Notification`, `StopFailure`, `PostToolUseFailure`, `PermissionDenied`, `CwdChanged`, `PostModelSwitch`, subagent payloads | section 03 | tolerate them, keep all their fields optional, never depend on one | stage 8 |
| The named-pipe write and reply from `rain-hook` (the spike forwarder logged to a file) | section 03, `rain-hook` | open the pipe with `std::fs::OpenOptions` read and write, retry `ERROR_PIPE_BUSY` briefly, wait for the answer on a helper thread with a 20 s limit, then exit 0 | stage 8 |
| Real Obsidian with the note open and unsaved edits; sync tools seeing the short-lived dot-prefixed temp file in the daily notes folder; Obsidian's own rules for an unterminated `---` or `True`; lone CR line ends | section 04, vault write | the editor refuses instead of guessing; the stale check retries 3 times and then returns `conflict`; a note that is open and changed in Obsidian after the check is a risk the owner accepts (decisions.md: Rain reads and writes) | stage 7, by hand with a copy of the vault |
| `ShellExecuteExW` with `SEE_MASK_FLAG_NO_UI` on a missing file (no dialog, error code returned); launching a classic AppsFolder item; the stability of `Microsoft.AutoGenerated` AUMIDs between scans | section 01 | the launch fails with a plain message; ids come from the id map, not from the AUMID alone | stage 4 |
| Recursive watch of the Start Menu folders with `ConfigHub::watch_dir` | sections 01, 10.4 | one non-recursive watch per sub-folder | stage 4 |
| Interactive and Desktop hooks, `SuspendThread` of a whole process tree, `RegisterWaitForSingleObject` exit watch | sections 02, 03 | see the rows above; the process wait falls back to a short-lived blocking wait on the handle | stage 5, 8 |
| Reveal, `obsidian://` and `SetForegroundWindow` from a windowless backend (the shell may refuse to bring a window forward; `AllowSetForegroundWindow` by the shell is the fallback); `wt.exe` and VS Code window detection | sections 02, 04, 05 | the action still runs; the window may stay behind | stage 3, 5, 7 |
| Epic manifests of the real launcher; OneDrive placeholder files in the disk scan | sections 02, 05 | Epic off when absent; placeholder files counted by their listed size | stage 3, 5 |
| An idle job's behaviour under real use; a Claude job run when the claude program path is not on `PATH` | section 03 | the run fails with a clear message and the run record | stage 6 |

## Appendix D. Spike results: each decision, the report, the source that proves it

Spike folders (all below `scratchpad/backend/spikes/`; reports are `<folder>/REPORT.md`; the agents could not write them, so the lead recovered them from their returned results). Paths below are relative to the folder named in the first column.

| Folder | Report |
|---|---|
| process | `spikes/process/REPORT.md` (spike a) |
| crates | `spikes/crates/REPORT.md` (spike b) |
| winapi | `spikes/winapi/REPORT.md` |
| types-stream | `spikes/types-stream/REPORT.md` (spike c) |
| claude-hooks | `spikes/claude-hooks/REPORT.md` (spike d) |

| Decision in this document | Section | Result | Report section | Source to copy from |
|---|---|---|---|---|
| Detached backend start: `DETACHED_PROCESS \| CREATE_NEW_PROCESS_GROUP \| CREATE_BREAKAWAY_FROM_JOB`, retry without breakaway on os error 5 | 2.3 | GO. Breakaway failed with os error 5 in the harness, so the retry is required | process Q1 | process: `src/bin/parent.rs`, `src/bin/backend.rs` |
| Single instance by the first pipe instance (os error 5), exit quietly | 2.4 | GO (mutex also works; pipe preferred) | process Q2; types-stream 3 | process: `src/lib.rs` (the pipe and mutex checks), `src/bin/backend.rs`; types-stream: `spike/src/winsec.rs`, `spike/src/bin/secproof.rs` |
| Launched apps never inside a kill-on-close job; backend not in a job; breakaway rules | 2.3, 11.6, D15 | GO with rules | process Q3 | process: `src/bin/q3_outer.rs`, `src/bin/q3_inner.rs`, `run_q3.ps1`, `q3_sleeper.out`, `q3_notepad.out` |
| Hand-written job object for owned trees; `process-wrap` rejected | 11.6, D15 | GO for hand-written; `process-wrap` 10.0.1 never sets `KILL_ON_JOB_CLOSE` | process Q4 | process: `src/bin/q4_hand.rs`, `src/lib.rs` (`resume_process_threads`, `job_pids`); comparison `src/bin/q4_wrap.rs`; outputs `out_q4_*.txt` |
| Graceful stop order; no graceful stop for headless Claude | 11.6 | GO, partial | process Q5; claude-hooks Q6 | process: `src/bin/q5.rs`, `ctrlchild.rs`, `ctrlsend.rs`, `guichild.rs`; claude-hooks: `jobrun/src/main.rs` |
| Process list by Toolhelp + `QueryFullProcessImageNameW` | 11.6 | GO, 4.71 ms median | process Q6 | process: `src/bin/q6.rs` |
| `ShellExecuteW` for folders, links, URIs, AUMID; reveal by `SHOpenFolderAndSelectItems`; no real URL in tests | 11.6 | GO | process Q7 | process: `src/bin/q7.rs`, `run_q7.ps1` |
| Credential Manager: `keyring-core` 1.0.0 + `windows-native-keyring-store` 1.1.0 | 10.8, 1.6 | GO | crates Q1 | crates: `src/bin/q1_keyring.rs`, `q1b_minimal/` |
| `notify` 9.0.0 + `notify-debouncer-full` 0.8.0, non-recursive, re-read on any event | 10.4 | GO (fallback 8.2.2 + 0.7.0 identical) | crates Q2 | crates: `src/bin/q2_notify.rs`, `q2b_notify8/` |
| `toml_edit` 0.25.20 round trip; two fixes; atomic write and retry | 10.4 | GO with the fixes | crates Q3 | crates: `src/toml_store.rs` |
| `rusqlite` 0.43.0 bundled, WAL, writer thread, read-only readers, size | 10.5 | GO | crates Q4 | crates: `src/bin/q4_sqlite.rs`, `q4_baseline.rs`, `q4_size_probe.rs` |
| `croner` 5.0.0, DST rules, hand-written cron label | 1.6, section 03 | GO | crates Q5 | crates: `src/cron_label.rs`, `src/bin/q5_croner.rs` |
| `tracing` daily rolling file, 7 files, lossy non-blocking writer, size budget, UTC | 10.7 | GO | crates Q6 | crates: `src/bin/q6_tracing.rs`, `src/log_budget.rs` |
| Idle axum server: runtime choice and idle cost | 11.5, D12 | GO for both runtimes; `multi_thread(2)` kept | crates Q7 | crates: `q7_idle/` |
| Line-based front-matter editor, byte-exact, no YAML crate in product | 13.3, section 04 | GO | crates Q8 | crates: `src/frontmatter.rs` |
| `shell:AppsFolder` as the single app source; filters; dedupe on AUMID | section 01 | GO, 125 items in 190 to 200 ms | winapi Q1 | winapi: `src/bin/apps.rs` |
| 64 px PNG icons for exe, lnk and AUMID | 4.7, 11.6 | GO | winapi Q2 | winapi: `src/bin/icons.rs`, `src/lib.rs`; `icon_exe.png`, `icon_lnk.png`, `icon_aumid.png` |
| Packaged app launch by AUMID with pid; `WM_CLOSE` fails for packaged apps | 11.6 | GO | winapi Q3 | winapi: `src/bin/launch.rs` |
| Idle time and drive facts | 11.6 | GO | winapi Q4, Q5 | winapi: `src/bin/sysinfo.rs` |
| Parallel disk walk, cancel, denied folders, budgeted tree, lazy subtree, logical size | 11.5, 11.6 | GO; 8 threads; `jwalk` rejected | winapi Q6 | winapi: `src/walk.rs`, `src/bin/walk.rs` |
| Registry: Steam facts, Run key write, read, delete with `winreg` 0.56 | 2.9, section 06 | GO | winapi Q7 | winapi: `src/bin/registry.rs` |
| Steam: manifests, play time, cover layouts, hand-written VDF parser | section 02 | GO | winapi Q8 | winapi: `src/vdf.rs`, `src/bin/steam.rs` |
| SSE over the pipe: delivery, reconnect, lag resync, shutdown, keep-alive | 5 | GO | types-stream 1 | types-stream: `spike/src/routes.rs` (`events`), `spike/src/sse.rs`, `spike/src/bin/server.rs`, `spike/src/bin/client.rs`; `logs/bench_final.txt` |
| Pooled client and 8 pending instances | 3.3, 6.10 | GO | types-stream 2 | types-stream: `spike/src/server.rs`, `spike/src/client.rs`; `logs/` |
| Pipe DACL for the current user SID and SYSTEM; remote rejected | 3.2 | GO | types-stream 3 | types-stream: `spike/src/winsec.rs`, `spike/src/bin/secproof.rs` |
| Binary responses with ETag and 304 before the bytes are produced | 4.7 | GO | types-stream 4 | types-stream: `spike/src/icons.rs` |
| ts-rs 12.0.1, single file, command, `TS_RS_LARGE_INT` | 9 | GO for both tools; ts-rs chosen | types-stream 5 | types-stream: `rain-types/src/lib.rs`, `rain-types/src/bindings.rs`, `.cargo/config.toml`, `tscheck/check.ts`, `tscheck/wire.ts`; for comparison `types-specta/`, `naive/` |
| One error body for every failure path | 4.5 | GO | types-stream 6 | types-stream: `spike/src/api_error.rs`, `rain-types/src/error.rs` |
| Hook events, order, payload keys; project hooks run in an untrusted folder in `-p` | section 03 | GO | claude-hooks Q1 | claude-hooks: `logs/`, `out/keys-all.json`, `control/keys.js` |
| `PermissionRequest`: allow, deny, nothing, exit codes, bad JSON, timeout, session-only rule | decisions C2, section 03 | GO: every failure branch equals no hook (headless) | claude-hooks Q2 | claude-hooks: `control/perm-queue.txt`, `control/run.js`, `out/run3.jsonl` |
| Forwarder cost about 20 ms; `async: true` works for observation events | 1.3, 14.3 stage 8 | GO | claude-hooks Q3 | claude-hooks: `fwd/src/main.rs`, `control/bench.js` |
| Session identity, registry file, death rule with tolerance | section 03 | GO | claude-hooks Q4 | claude-hooks: `fwd/src/main.rs`, `out/` |
| Transcript line types and token count | section 03 | GO | claude-hooks Q5 | claude-hooks: `control/transcript.js` |
| Headless job in a job object, stop, resume | 11.6, section 03 | GO | claude-hooks Q6 | claude-hooks: `jobrun/src/main.rs`, `out/run4.jsonl`, `out/run5.jsonl`, `out/run6.jsonl` |
| Install JSON and merge or un-merge routine | section 03, 13.3 | GO | claude-hooks Q7 | claude-hooks: `merge/src/main.rs`, `out/rain-hooks-fragment.json`, `out/installed-example.json` |
