# Rain backend, section 06: settings, lifecycle, game mode, and the end of the mock

| | |
|---|---|
| Area | settings (every field), lifecycle pieces the foundation leaves open, game mode end to end, deletion of the mock (stage 10) |
| Modules | `settings`, `lifecycle`, `gamemode` (backend); the shell parts named in section 6.8 and 9.4; the frontend files of section 9 |
| Author | backend designer for section 06 |
| Date | 2026-11-22 |
| Status | draft 1, written on top of `00-foundation.md` draft 2 and `decisions.md` |

Reading rule: where this section disagrees with `00-foundation.md`, the foundation wins and the point is in section 12. Every path of the owner of this PC (data folder, project folder, game folder, vault) is deliberately absent from this document. They are set in Settings, never in code, tests, fixtures or documents (foundation 10.1).

Words with one meaning in this section:

| Word | Meaning |
|---|---|
| override | a value the user wrote into `settings.toml`; an empty override means "not set" |
| effective value | what Rain really uses: the override, else the detected value, else the built-in default |
| detected value | a value Rain found on the PC without being told (Obsidian's vault list, the Steam registry key, ...) |
| external effect | a setting whose change does something outside `settings.toml`: `startWithWindows`, `dataDir`, `monitorClaude` |
| run | one game mode session from the user's start to `done` or cancel |
| wind-down | the phase `windingDown`: the stops are carried out |

---------------------------------------------------------------------------------------------------

## 1. Scope

### 1.1 What this section covers

| Part | RainApi methods and topics | Screens | Built in stage |
|---|---|---|---|
| Settings resource | `getSettings`, `updateSettings`, new `getHealth`; new topic `settings` | Settings; the readers `summon.ts`, `TodayPanel.vue`, `LifeScreen.vue` | storage and screen 1b; detection, autostart, data folder, live reload 2; `getHealth` 2 (`checkPaths` is section 05, stage 3) |
| Lifecycle | `on_shell_facts` (autostart self-heal and the `hotkey` health source only; called by the core handler of `PUT /internal/shell`: foundation 11.2), the stop-reason policy of `POST /internal/shutdown`, autostart, data folder restart, the shell exit sequence | none (the shell) | 2 (core stores the shell facts already in 1b) |
| Game mode | `getGameMode`, `startGameMode`, `resolveGameMode`, new `markGameModeRecapSeen`; topic `gameMode` | Game mode; the recap notice in the app frame | 9 |
| End of the mock | none new | Settings "Prototype" group, six commands, `main.ts`, `AppShell.vue`, `data/mock`, `router.ts`, `routing.ts`, `types.ts` | 10 |

Fields of `Settings` whose meaning belongs to another section are only stored here: `projectRoots`, `gameFolders`, `steamEnabled`, `steamPath`, `epicEnabled`, `epicPath` (section 02), `vaultPath`, `dailyNoteFormat`, `dailyNoteFolder` (section 04), `monitorClaude` and its state (section 03). This section owns the resource, the file, the screen, the validation and the effective-value rules. The other section owns what the value does. Section 6.1 lists both.

### 1.2 What this section does not do

- The hotkey is not editable this round. The shell registers `Ctrl+Backquote`; the backend only reports whether that worked (6.5).
- The calendar has no connect flow (decisions.md section 4). `calendarConnected` is a read-only `false`.
- There is no first-run wizard. A new user sees defaults, detected values and empty states that point at Settings (6.1.6).
- There is no "Quit Rain" action. Only game mode, `rain-backend --stop` and Task Manager end Rain (open point U1).
- The user's open Claude sessions are never stopped by game mode (decisions.md section 4). They are listed as "left running".

### 1.3 Decisions of decisions.md that this section implements

| Decision | Where |
|---|---|
| Data folder location in `machine.toml`, changeable in Settings; `RAIN_DATA_DIR` and `RAIN_PIPE` for tests | 4.2, 6.3 |
| Game folders are a Settings list, set in Settings | 3.1, 4.1, 9.2 |
| Daily notes: folder and format from `.obsidian/daily-notes.json`, Settings may override | 6.1.2 |
| Claude hooks behind the switch "Monitor Claude sessions"; never touch the real Claude settings in tests | 6.1, 6.2 (rule R5) |
| Game mode stops Rain and its jobs only; the user's sessions are listed | 6.7 |
| C5: backend stops, publishes `done` last, exits; shell exits after the pipe closes plus a fixed grace; nothing restarts Rain | 6.7.5, 6.8 |
| C6: the shell registers the hotkey and reports `hotkeyAvailable` | 6.5 |
| Calendar off, `listEvents` returns nothing, Settings shows "not connected" | 3.1, 9.2 |
| "Start with Windows" off by default, the shell starts hidden | 6.4 |
| A job missed in game mode runs once at the next start | 6.7.6 |
| Stage 10 leaves no mock code | 9.7 |

### 1.4 Stage plan of this section

| Stage | Built here |
|---|---|
| 1a | `rain-types`: `settings.rs` only (`Settings`, `SettingsPatch`, `ResetAfterSeconds`, `CountdownSeconds`, `MonitorClaudeState`) in its final shape; the `Settings` identity check against the final TypeScript of 3.3 (section 12 point A1). `gamemode.rs` and its entry in `decls!` are NOT built in 1a: they are added in stage 9, where the hand-written game-mode types are deleted (foundation 9.4; otherwise the duplicate-name check of foundation 8.8 fails) |
| 1b | `settings` module: `GET` and `PATCH /settings` over `settings.toml` (all file-backed fields, validation, the "one external effect per request" rule), topic `settings` on own writes, `hotkeyAvailable` read from `state.shell_facts` (core stores them and publishes `health`, then calls `settings::Service::on_shell_facts_changed()`: `PUT /internal/shell` is core in `server/builtin.rs`, foundation 11.2); frontend 9.2 step 1b |
| 2 | `lifecycle` module: autostart, shell-facts self-heal; data folder change (`machine.toml`, copy, restart); detection of vault, Steam, Epic and the daily-note settings; live reload topic; the stop-reason policy and `--stop --force`; frontend 9.2 step 2 |
| 3 | the `placeholder` of Settings rows on top of section 05's glyph change (frontend 9.2 step 3); `POST /paths/check` is section 05's |
| 5, 7, 8 | the rows "Game folders" (5), "Daily notes folder" (7) and "Monitor Claude sessions" (8) appear in Settings when the owner of the value builds it |
| 9 | `gamemode` module, the Game mode screen changes, the recap notice, the reset-to-Home fix, the shell exit sequence |
| 10 | deletions (9.7) and the final check (9.8) |

Until the owning stage exists, a field that has an external effect refuses a change with `503 unavailable`, reason `notBuilt` (8.1). Reason: a switch that stores a value and does nothing would lie. The refusal is removed in the stage that builds the effect, and its test changes in the same commit.

---------------------------------------------------------------------------------------------------

## 2. Routes

Paths are after `/api/v1`. "Idempotent" follows foundation 4.6.

### 2.1 Route table

| Method | Path | Request | Response | Success | Errors (code, reason) | Idempotent | Topic effects |
|---|---|---|---|---|---|---|---|
| GET | `/settings` | none | `Settings` | 200 | `unavailable` `moduleFailed` | yes | none |
| PATCH | `/settings` | `SettingsPatch` body | `Settings` (the new state) | 200 | see 8.1: `invalid` `badBody`, `onePerRequest`, `tooMany`, `tooLong`, `badText`, `badFormat`, `badFolder`, `badPath`, `notAFolder`, `nested`, `notWritable`; `conflict` `configBroken`, `configNewer`, `configChanged`, `sandbox`, `noShell`, `environment`, `busy`, `gameMode`; `failed` `saveFailed`, `registry`, `copyFailed`; `unavailable` `notBuilt` | yes for equal values (a `monitorClaude: true` while it is true repairs the entry and is safe to repeat) | topic `settings` (own write) |
| GET | `/game-mode` | none | `GameModeState` | 200 | `unavailable` `moduleFailed` | yes | none |
| POST | `/game-mode` | `StartGameModeBody` | `GameModeState` | 200 | `notFound` `unknownGame` (source `games`); `conflict` `notInstalled`, `stopping`; `unavailable` `shuttingDown` | yes: when a run is under way it returns that run unchanged and ignores the body | topic `gameMode` |
| POST | `/game-mode/choice` | `GameModeChoiceBody` | `GameModeState` | 200 | `conflict` `notRunning`, `windingDown`; `invalid` `badBody` | yes (table 6.7.2: a choice already satisfied or overtaken returns the current state) | topic `gameMode` |
| POST | `/game-mode/recap/seen` | none | none | 204 | none | yes | topic `gameMode` (the recap loses its unseen mark) |
| PUT | `/internal/shell` | `ShellFacts` (foundation 6.5) | none | 204 | `invalid` `badBody` | yes | core route; the core handler stores the facts, publishes `health`, then calls `settings::Service::on_shell_facts_changed()`, which publishes `settings` only when the computed `hotkeyAvailable` changed (foundation 11.2) |
| POST | `/internal/shutdown` | `{ reason, force? }` (foundation 2.6) | none | 204 | `conflict` `busy` for `upgrade` and `restart` (6.9) | no | `lifecycle` frame as the stream tail |

`PUT /internal/shell` and `POST /internal/shutdown` are internal: the shell command and the Vite proxy refuse them (foundation 3.2). The handler of the first is core (`server/builtin.rs`, stage 1b; it stores `ShellFacts` in `state.shell_facts`, publishes `health` and calls `settings::Service::on_shell_facts_changed()`, stage 1b) and, from stage 2, `lifecycle::Service::on_shell_facts`; the handler of the second is core too (foundation 11.2), the reason policy is mine (6.9).

`POST /game-mode` answers 200 and not 201: it does not create a resource the client addresses later; it returns the new state (foundation 4.6, "action that returns the new state").

Request body limit: the foundation default (1 MiB).

### 2.2 Bodies, in short

| Body | Shape |
|---|---|
| `SettingsPatch` | any subset of the writable fields of 3.1; unknown fields are rejected (`deny_unknown_fields`), so a patch that names `hotkey`, `hotkeyAvailable`, `calendarConnected` or `monitorClaudeState` is `400 invalid` |
| `StartGameModeBody` | `{}` or `{ "thenLaunchGameId": "game-id" }` |
| `GameModeChoiceBody` | `{ "choice": "wait" \| "stopAnyway" \| "closeNow" \| "cancel" }` |

### 2.3 Live topics

| Topic | Payload | Kind | Max rate | Publisher and moment |
|---|---|---|---|---|
| `settings` (new) | `Settings` | whole object | 1 per second (a hand edit burst is already one event after the 300 ms debounce) | `settings` module: after every own `PATCH`, after a reload of `settings.toml` that changed the value, and when `hotkeyAvailable` changes (`settings::Service::on_shell_facts_changed()`, called by the core handler of `PUT /internal/shell`; the `lifecycle` module does not publish it) |
| `gameMode` | `GameModeState` | whole object | 4 per second at most (`Throttle`); in practice 1 per second in `countdown` plus one per step | `gamemode` module: on start, on every choice, every countdown second, when the counts of running work change, on every wind-down step, and last of all `done` |

`done` is the last topic frame the backend ever publishes. It is published with `Throttle::publish_final(state)` (foundation 5.4: it cancels any pending throttled value and publishes at once, so no stale countdown frame can follow it and `done` cannot be lost), and only then `EventBus::freeze()`. The same rule holds for the final state of a cancelled run. After it `EventBus::freeze()` (section 12 point A3) drops every later publish, so a module that is still stopping cannot slip a frame in behind `done`. Then core writes the `lifecycle` tail frame and closes the streams (foundation 2.6).

---------------------------------------------------------------------------------------------------

## 3. Types

### 3.1 Settings (`rain-types/src/settings.rs`)

Changes against `src/data/types.ts` and why:

| Change | Why |
|---|---|
| adds `dataDir: string` | the data folder is a setting (decisions.md section 4); the value is the effective folder |
| adds `dailyNoteFolder: string` | the vault's daily notes are in a folder, not in the vault root (for example a folder named `Daily`); decisions.md section 4 |
| adds `gameFolders: string[]` | decisions.md "Game folders" |
| adds `monitorClaude: boolean` and `monitorClaudeState` | the switch "Monitor Claude sessions" and its four states |
| `calendarConnected` stays, read-only, always `false` | no calendar this round |
| `hotkey`, `hotkeyAvailable`, `calendarConnected`, `monitorClaudeState` are read-only | they are facts; a patch that names them is rejected by `deny_unknown_fields` because `SettingsPatch` has no such fields |
| `updateSettings` takes `SettingsPatch` instead of `Partial<Settings>` | the type says exactly what may be sent (F9 of the foundation) |
| for `vaultPath`, `dailyNoteFormat`, `dailyNoteFolder`, `steamPath`, `epicPath` the value in `Settings` is the effective value; sending an empty string clears the override | one field per setting, no second "detected" field; the UI shows what Rain really uses |

```rust
// crates/rain-types/src/settings.rs
use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};
use ts_rs::TS;

/// The chord the shell registers. Not personal, not editable this round.
pub const SUMMON_HOTKEY: &str = "Ctrl+Backquote";

// Number unions: serde_repr rejects every other number, so `resetAfterSeconds = 45` is a 400 for free (foundation 9.2).
#[derive(Serialize_repr, Deserialize_repr, TS, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
#[ts(type = "30 | 60 | 300")]
pub enum ResetAfterSeconds { Half = 30, Minute = 60, FiveMinutes = 300 }

#[derive(Serialize_repr, Deserialize_repr, TS, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
#[ts(type = "5 | 10 | 20 | 30")]
pub enum CountdownSeconds { Five = 5, Ten = 10, Twenty = 20, Thirty = 30 }

/// State of the switch "Monitor Claude sessions". Computed by the sessions module (section 03).
#[derive(Serialize, Deserialize, TS, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MonitorClaudeState {
    Off,          // switch off and nothing of Rain in the Claude settings
    On,           // switch on, entries and forwarder present, at least one event arrived in this backend run
    OnInactive,   // switch on, entries and forwarder present, no event yet (no session has started since)
    NeedsRepair,  // switch on, but an entry or the forwarder is missing or changed, or the file cannot be read
}

#[derive(Serialize, Deserialize, TS, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub hotkey: String,                       // read-only, SUMMON_HOTKEY
    pub hotkey_available: bool,               // read-only, from the shell (6.5)
    pub hide_on_blur: bool,
    pub reset_after_seconds: ResetAfterSeconds,
    pub start_with_windows: bool,             // effective state of the autostart entry (6.4)
    pub data_dir: String,                     // effective data folder
    pub vault_path: String,                   // effective; "" = no vault
    pub daily_note_format: String,            // effective, never empty
    pub daily_note_folder: String,            // effective, relative to the vault, "/" separated, may be empty
    pub project_roots: Vec<String>,           // as stored
    pub game_folders: Vec<String>,            // as stored
    pub steam_enabled: bool,
    pub steam_path: String,                   // effective; "" = not found
    pub epic_enabled: bool,
    pub epic_path: String,                    // effective; "" = not found
    pub calendar_connected: bool,             // read-only, always false
    pub monitor_claude: bool,
    #[ts(inline)]
    pub monitor_claude_state: MonitorClaudeState,   // read-only
    pub game_mode_countdown_seconds: CountdownSeconds,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(optional_fields)]
pub struct SettingsPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")] pub hide_on_blur: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub reset_after_seconds: Option<ResetAfterSeconds>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub start_with_windows: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub data_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub vault_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub daily_note_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub daily_note_folder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub project_roots: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub game_folders: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub steam_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub steam_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub epic_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub epic_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub monitor_claude: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub game_mode_countdown_seconds: Option<CountdownSeconds>,
}

/// The topic struct lives in `rain-types/src/events.rs` (foundation 5.3), not in this file, and is added in stage 1b.
/// Named `SettingsTopic` because `Settings` is the payload type.
pub struct SettingsTopic;
impl crate::Topic for SettingsTopic { const NAME: &'static str = "settings"; type Payload = Settings; }
```

Validation rules that are not expressed by the types are in 6.2 (rule table V). Tests: JSON round trip with a full and a minimal value, `assert_no_nulls`, and `resetAfterSeconds: 45` and `gameModeCountdownSeconds: 15` rejected.

Resulting TypeScript (the generator writes `export type X = {...}` with double quotes; this is the same shape written for reading; `ResetAfterSeconds` and `CountdownSeconds` come out as named aliases because of the container-level `ts(type)`, which is the proven spike pattern):

```ts
export type MonitorClaudeState = 'off' | 'on' | 'onInactive' | 'needsRepair'
export interface Settings {
  hotkey: string; hotkeyAvailable: boolean; hideOnBlur: boolean; resetAfterSeconds: 30 | 60 | 300;
  startWithWindows: boolean; dataDir: string; vaultPath: string; dailyNoteFormat: string; dailyNoteFolder: string;
  projectRoots: string[]; gameFolders: string[]; steamEnabled: boolean; steamPath: string;
  epicEnabled: boolean; epicPath: string; calendarConnected: boolean;
  monitorClaude: boolean; monitorClaudeState: MonitorClaudeState; gameModeCountdownSeconds: 5 | 10 | 20 | 30 }
export interface SettingsPatch {
  hideOnBlur?: boolean; resetAfterSeconds?: 30 | 60 | 300; startWithWindows?: boolean; dataDir?: string;
  vaultPath?: string; dailyNoteFormat?: string; dailyNoteFolder?: string; projectRoots?: string[]; gameFolders?: string[];
  steamEnabled?: boolean; steamPath?: string; epicEnabled?: boolean; epicPath?: string;
  monitorClaude?: boolean; gameModeCountdownSeconds?: 5 | 10 | 20 | 30 }
```

UNPROVEN: `Option<ResetAfterSeconds>` inside a struct with `optional_fields`. The foundation proves each attribute on its own (a number union, an optional field) but not the pair. The stage 1a identity test covers it (the test file `tscheck/settings.ts` holds the TypeScript above as the hand-written side). If ts-rs prints `resetAfterSeconds: ResetAfterSeconds | null`, add `#[ts(optional)]` on that field.

### 3.2 Game mode (`rain-types/src/gamemode.rs`)

Changes against `src/data/types.ts` lines 82-87 and why:

| Change | Why |
|---|---|
| `WindDownItem.effect` gets a third value `leftRunning` | the user's open Claude sessions are listed but never stopped; they need their own mark (`safe` would say "nothing happens", `interrupts` would be false) |
| `GameModeState.rev: number` | every state has a number that only grows within one backend process, so a client can drop a state that arrives late (6.10) |
| `GameModeState.waitTimedOut?: boolean` | "Wait" ended after 3 minutes with work still running; the screen says so |
| `GameModeState.launchError?: string` | the game that should start after the wind-down could not be started |
| `GameModeState.last?: GameModeRecap` | what the last game mode did, shown once on the first load afterwards (6.7.7) |
| `startGameMode` may now reject (`notFound`, `conflict`); `resolveGameMode` rejects `notRunning` and `windingDown` | a real backend has real failure; the mock never failed |
| new method `markGameModeRecapSeen()` | the recap notice is shown once |

```rust
// crates/rain-types/src/gamemode.rs
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use crate::common::IconRef;

#[derive(Serialize, Deserialize, TS, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum GameModePhase { Off, Review, Countdown, WindingDown, Done }

#[derive(Serialize, Deserialize, TS, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum GameModeChoice { Wait, StopAnyway, CloseNow, Cancel }

#[derive(Serialize, Deserialize, TS, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WindDownKind { Session, Job, IdleJob, Service, Window }       // "idleJob" on the wire

#[derive(Serialize, Deserialize, TS, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WindDownEffect { Safe, Interrupts, LeftRunning }

#[derive(Serialize, Deserialize, TS, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum WindDownStatus { Pending, Stopping, Stopped }

#[derive(Serialize, Deserialize, TS, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WindDownItem {
    pub id: String,
    pub label: String,
    pub icon: IconRef,
    #[ts(inline)] pub kind: WindDownKind,
    pub count: u32,
    #[ts(inline)] pub effect: WindDownEffect,
    #[ts(inline)] pub status: WindDownStatus,
}

#[derive(Serialize, Deserialize, TS, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecapOutcome { Stopped, Forced, Failed, LeftRunning }

#[derive(Serialize, Deserialize, TS, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecapItem {
    #[ts(inline)] pub kind: WindDownKind,
    pub label: String,
    pub count: u32,
    #[ts(inline)] pub outcome: RecapOutcome,
    pub names: Vec<String>,                   // at most 5 names of jobs; empty for sessions (titles can hold prompt text)
}

#[derive(Serialize, Deserialize, TS, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct GameModeRecap {
    pub at: String,                           // ISODateTime: when the wind-down ended (or began, if it never ended)
    pub complete: bool,                       // false: the backend died during the wind-down
    pub items: Vec<RecapItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub launched_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub launch_error: Option<String>,
    pub seen: bool,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct GameModeState {
    #[ts(inline)] pub phase: GameModePhase,
    pub waiting: bool,
    pub seconds_left: u32,
    pub countdown_seconds: u32,
    pub items: Vec<WindDownItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub then_launch_game_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub wait_timed_out: Option<bool>,   // only ever Some(true)
    #[serde(default, skip_serializing_if = "Option::is_none")] pub launch_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub last: Option<GameModeRecap>,    // only in phase "off"
    pub rev: u64,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(optional_fields)]
pub struct StartGameModeBody { #[serde(default, skip_serializing_if = "Option::is_none")] pub then_launch_game_id: Option<String> }

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GameModeChoiceBody { pub choice: GameModeChoice }

// In `rain-types/src/events.rs` (foundation 5.3), added in stage 9 with this file.
pub struct GameModeTopic;
impl crate::Topic for GameModeTopic { const NAME: &'static str = "gameMode"; type Payload = GameModeState; }
```

Resulting TypeScript (changes only; everything not shown is unchanged):

```ts
export type GameModeChoice = 'wait' | 'stopAnyway' | 'closeNow' | 'cancel'
export interface WindDownItem { id: ID; label: string; icon: IconRef; kind: 'session' | 'job' | 'idleJob' | 'service' | 'window';
  count: number; effect: 'safe' | 'interrupts' | 'leftRunning'; status: 'pending' | 'stopping' | 'stopped' }
export interface RecapItem { kind: WindDownItem['kind']; label: string; count: number;
  outcome: 'stopped' | 'forced' | 'failed' | 'leftRunning'; names: string[] }
export interface GameModeRecap { at: ISODateTime; complete: boolean; items: RecapItem[];
  launchedTitle?: string; launchError?: string; seen: boolean }
export interface GameModeState { phase: 'off' | 'review' | 'countdown' | 'windingDown' | 'done';
  waiting: boolean; secondsLeft: number; countdownSeconds: number; items: WindDownItem[]; thenLaunchGameId?: ID;
  waitTimedOut?: boolean; launchError?: string; last?: GameModeRecap; rev: number }
```

The five item ids, labels, icons and kinds are constants in code (`gamemode/items.rs`), in stop order:

| Order | id | label | icon (lucide) | kind | What the row means |
|---|---|---|---|---|---|
| 1 | `wind-sessions` | Claude sessions | `bot` | `session` | open sessions of the user; effect is always `leftRunning`; never stopped |
| 2 | `wind-jobs` | Scheduled jobs | `calendar-clock` | `job` | `count` = enabled jobs; `interrupts` when a run is in progress |
| 3 | `wind-idle-jobs` | Idle jobs | `cpu` | `idleJob` | `count` = enabled idle jobs; `interrupts` when a run is in progress |
| 4 | `wind-service` | Background service | `server` | `service` | the backend itself; always `safe`, count 1 |
| 5 | `wind-window` | Rain window | `app-window` | `window` | the shell; always `safe`, count 1 |

The list is never empty. In phase `off` the items are the same five with count 0, effect `safe`, status `pending` (as the mock did).

### 3.3 Where the types enter the generator

`bindings.rs` `decls!` gets, in stage 1a: `ResetAfterSeconds, CountdownSeconds, MonitorClaudeState, Settings, SettingsPatch`; and, in stage 9 only (`gamemode.rs` does not exist before, so the hand-written copies in `types.ts` stay until then and `scripts/check.mjs` finds no duplicate): `GameModePhase, GameModeChoice, WindDownKind, WindDownEffect, WindDownStatus, WindDownItem, RecapOutcome, RecapItem, GameModeRecap, GameModeState, StartGameModeBody, GameModeChoiceBody`. `TOPIC_NAMES` gets `settings` through `SettingsTopic` (stage 1b) and `gameMode` through `GameModeTopic` (stage 9).

Internal Rust types that are NOT in `rain-types` (they never cross the API): `SettingsFile` (4.1), `MachineFile` (4.2), `Resolved` (6.1.7), `Machine` (6.7).

---------------------------------------------------------------------------------------------------

## 4. Data files

Both files are written only by the `settings` module, with `toml_edit` through `TomlFile::edit` (foundation 10.4: atomic, comments kept, CRLF kept, the two decor fixes).

### 4.1 `settings.toml` (row of foundation 10.2; owner 06)

Schema. Every key is optional. A missing key, a missing file and an empty file all mean the default.

| Key | Type | Default | Validation (hand edit and `PATCH`) | Meaning |
|---|---|---|---|---|
| `schema_version` | integer | 1 | above 1: file loaded read-only, `ConfigIssue` "written by a newer Rain" (foundation 10.3) | |
| `hide_on_blur` | bool | `true` | | the window hides when it loses focus (release builds) |
| `reset_after_seconds` | integer | `60` | one of 30, 60, 300 | Home after being hidden this long |
| `vault_path` | string | `""` | no control characters, at most 1024 characters | override of the Obsidian vault folder; `""` = the detected vault |
| `daily_note_format` | string | `""` | at most 64 characters; none of `< > : " \| ? *` or `\` or control characters; the owner of the tokens (section 04) may refuse more | override; `""` = the vault's own setting, else `YYYY-MM-DD` |
| `daily_note_folder` | string | `""` | relative: not absolute, no drive, no `..` segment; at most 128 characters; stored with `/`; surrounding `/` removed | override; `""` = the vault's own setting, else the vault root |
| `project_roots` | array of string | `[]` | at most 32 entries; each non-empty after trimming, at most 1024 characters; duplicates (case-insensitive, after normalising) removed on write | folders that hold repositories (section 02) |
| `game_folders` | array of string | `[]` | as `project_roots` | folders that hold installed games (section 02) |
| `steam_enabled` | bool | `true` | | include Steam games |
| `steam_path` | string | `""` | as `vault_path` | override of the Steam folder; `""` = detected |
| `epic_enabled` | bool | `true` | | include Epic games |
| `epic_path` | string | `""` | as `vault_path` | override of the Epic Games Launcher data folder; `""` = detected |
| `game_mode_countdown_seconds` | integer | `10` | one of 5, 10, 20, 30 | read when game mode starts |
| `monitor_claude` | bool | `false` | | the switch "Monitor Claude sessions" (the intent; the state is computed by section 03) |

`start_with_windows` is NOT in the file. The registry value is the truth (6.4); a second copy could disagree with it.

A value of the wrong type or outside its set is a parse or validation error of the whole file: the last good value is kept, a `ConfigIssue` names the file and line (foundation 10.4), and Rain does not overwrite the file until it parses again (a `PATCH` answers `conflict`, reason `configBroken`).

Rust (internal, `settings/store.rs`; no `deny_unknown_fields`, every field `#[serde(default)]`):

```rust
#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct SettingsFile {
    pub schema_version: Option<u32>,
    pub hide_on_blur: Option<bool>,                    // None = default; Option so "key absent" and "key present" differ
    pub reset_after_seconds: Option<ResetAfterSeconds>,
    pub vault_path: String, pub daily_note_format: String, pub daily_note_folder: String,
    pub project_roots: Vec<String>, pub game_folders: Vec<String>,
    pub steam_enabled: Option<bool>, pub steam_path: String,
    pub epic_enabled: Option<bool>, pub epic_path: String,
    pub game_mode_countdown_seconds: Option<CountdownSeconds>,
    pub monitor_claude: Option<bool>,
}
```

The file Rain writes on the first `PATCH` (the first write creates it; a first run writes nothing, foundation 10.1). The header is written once. It sits directly above `schema_version`, so it is that key's decor and is never removed:

```toml
# Rain settings.
# Rain writes only the settings you change in the app. You may edit this file by hand; Rain reloads it.
# A key that is missing here uses the default shown in the comment.
#
#   hide_on_blur = true                   hide the window when it loses focus
#   reset_after_seconds = 60              back to Home after being hidden this long: 30, 60 or 300
#   vault_path = ''                       Obsidian vault folder; empty = the only vault Obsidian knows
#   daily_note_format = ''                empty = the vault's own setting, else YYYY-MM-DD
#   daily_note_folder = ''                folder inside the vault; empty = the vault's own setting
#   project_roots = []                    folders that contain your repositories
#   game_folders = []                     folders that contain installed games, one folder per game
#   steam_enabled = true
#   steam_path = ''                       empty = detected
#   epic_enabled = true
#   epic_path = ''                        empty = detected
#   game_mode_countdown_seconds = 10      5, 10, 20 or 30
#   monitor_claude = false                the switch "Monitor Claude sessions"
schema_version = 1

# Example after a few changes in the app:
reset_after_seconds = 300          # a comment the user added after the value survives
project_roots = ['D:\Code', 'D:\Work']
game_folders = ['D:\Games']
monitor_claude = true
```

Paths are written as TOML literal strings (single quotes, no escaping of backslashes) so that a user reads and edits them naturally; a path that contains `'` is written as a basic string. The writer sets the representation with `Formatted::set_repr_unchecked` and a unit test reads every written value back.

What Rain writes back: only the keys of the patch. A cleared override (`""`) removes its key; an empty list writes `[]`; `monitor_claude` is written only after the install or removal succeeded (6.2). Nothing else in the file changes (comments, order, unknown keys survive). What a hand edit may do: anything the schema allows; the reload runs the same validation and reaches the UI through topic `settings` within one second after the 300 ms debounce (stage 2; in stage 1b the route re-reads the file when its length or modified time changed).

### 4.2 `machine.toml` (foundation 10.1; owner 06)

Location: `%APPDATA%\Rain\machine.toml` (sandbox: `%APPDATA%\Rain-dev\machine.toml`). It says where the data folder is and nothing else. It is read once at backend start by `Config::from_env` (`config.rs`, with the `toml` crate, before any module exists) and is not watched.

| Key | Type | Default | Validation |
|---|---|---|---|
| `schema_version` | integer | 1 | above 1: file ignored, `ConfigIssue` |
| `data_dir` | string | absent | absolute path (drive letter or UNC), at most 1024 characters |

```toml
# Rain machine file: where your data folder is. Change it in Rain (Settings, Data folder).
schema_version = 1
data_dir = 'D:\RainData'
```

Defaults when missing: no `data_dir` means `%APPDATA%\Rain\data` (sandbox `%APPDATA%\Rain-dev\data`). `RAIN_DATA_DIR` beats the file (foundation 10.1).

Rain writes it in one case: a successful data folder change (6.3). The write uses the same atomic writer. A hand edit takes effect at the next backend start.

Fallback at start: if `data_dir` cannot be used (the folder is missing and cannot be created, for example a disconnected drive), the backend uses the default folder for this run, adds `ConfigIssue { file: "machine.toml", message: "The data folder <path> could not be used. Rain uses the default folder." }`, and does not change the file. Then the user sees the default folder's contents, which is why the message names the folder.

---------------------------------------------------------------------------------------------------

## 5. Tables

Module `gamemode`; migration `crates/rain-backend/src/modules/gamemode/migrations/0001_create_record.sql` (id 1, name `create_record`). The `settings` and `lifecycle` modules have no tables.

```sql
-- One row per game mode run that reached the wind-down. History, not cache: but losing it loses only the recap.
CREATE TABLE gamemode_record (
  id                  INTEGER PRIMARY KEY AUTOINCREMENT,
  started_at          TEXT    NOT NULL,                    -- RFC 3339, when the wind-down began
  finished_at         TEXT,                                -- NULL while running, and for good if the backend died
  outcome             TEXT    NOT NULL CHECK (outcome IN ('running', 'completed', 'incomplete')),
  countdown_seconds   INTEGER NOT NULL,
  waited              INTEGER NOT NULL DEFAULT 0,          -- 1: the user chose Wait
  forced              INTEGER NOT NULL DEFAULT 0,          -- 1: something was interrupted
  then_launch_game_id TEXT,
  then_launch_title   TEXT,
  launch_result       TEXT CHECK (launch_result IN ('started', 'failed')),
  launch_error        TEXT,
  seen_at             TEXT                                 -- NULL until the recap notice was shown
);
CREATE INDEX idx_gamemode_record_started ON gamemode_record (started_at);

CREATE TABLE gamemode_record_item (
  record_id INTEGER NOT NULL REFERENCES gamemode_record (id) ON DELETE CASCADE,
  seq       INTEGER NOT NULL,
  kind      TEXT    NOT NULL CHECK (kind IN ('session', 'job', 'idleJob', 'service', 'window')),
  name      TEXT    NOT NULL,                              -- a job name, or the label of a group
  outcome   TEXT    NOT NULL CHECK (outcome IN ('stopped', 'forced', 'failed', 'leftRunning')),
  count     INTEGER NOT NULL DEFAULT 1,                    -- sessions: how many were left running
  PRIMARY KEY (record_id, seq)
);
```

Rules:

| Rule | Value |
|---|---|
| Rebuildable | No. History. Deleting `rain.db` loses the recap and nothing else (foundation 10.5). |
| Retention | the newest 20 records (`GAMEMODE_RECORDS_KEPT`); older rows are deleted in the same transaction that inserts a new record |
| Content | job names are stored (they are the user's own words in `jobs.toml`). Claude session TITLES are never stored: a title can hold prompt text. Only the number of sessions left running is stored. |
| Never logged | names and titles (foundation 13.4: the log gets counts and ids only) |
| Written | `begin_record` when the wind-down starts (outcome `running`), one `gamemode_record_item` per finished step, `finish_record` (outcome `completed`, `finished_at`, launch result) just before the service item reports `stopped`. All three run before the database is closed by core shutdown. |
| At backend start | every record still `running` becomes `incomplete`. The recap then says "Game mode did not finish". |
| Migration list | `[Migration { id: 1, name: "create_record", sql: include_str!("migrations/0001_create_record.sql") }]` |

`GameModeRecap` is built from the newest record with `outcome <> 'running'`. In phase `off` it is attached to the state; the `seen` field is `seen_at IS NOT NULL`.

---------------------------------------------------------------------------------------------------

## 6. Algorithms

### 6.1 Settings: where every value comes from

#### 6.1.1 Field by field

"Reads" is how `GET /settings` fills the field. "Changing it does" is what a `PATCH` does on the PC besides writing `settings.toml`. "Used by" names the code that reacts.

| Field | Reads | Changing it does | Used by | Live in stage |
|---|---|---|---|---|
| `hotkey` | constant `SUMMON_HOTKEY` | not changeable | `KeyHint` in Settings | 1b |
| `hotkeyAvailable` | `shell_facts.hotkey_registered`; `true` while no shell ever reported (a plain browser shows no false warning) | not changeable | Settings row "Summon hotkey" | 1b |
| `hideOnBlur` | file, default `true` | nothing on the PC | `src/keys/summon.ts` (holds the window open when `false`) | 1b |
| `resetAfterSeconds` | file, default 60 | nothing on the PC | `summon.ts` | 1b |
| `startWithWindows` | registry (6.4); `false` in sandbox and while the stage is below 2 | writes or removes `HKCU\...\Run\Rain`, clears a Windows "disabled" mark | Windows at logon | 2 |
| `dataDir` | `Config` (env, else `machine.toml`, else default), or the pending value after a change | copy-when-empty, writes `machine.toml`, restarts the backend (6.3) | every module (they re-read their files at start) | 2 |
| `vaultPath` | file override, else Obsidian's vault list (6.1.2), else `""` | `life` re-points its reader and its folder watch; never writes into the vault | section 04 | 1b stores, 7 acts |
| `dailyNoteFormat` | file override, else `<vault>\.obsidian\daily-notes.json` key `format`, else `YYYY-MM-DD` | `life` recomputes note file names | section 04 | 1b stores, 7 acts |
| `dailyNoteFolder` | file override, else the same file's key `folder`, else `""` | as above | section 04 | 1b stores, 7 acts |
| `projectRoots` | file, default `[]`, as typed | `projects` rescans | section 02 | 1b stores, 5 acts |
| `gameFolders` | file, default `[]`, as typed | `games` rescans | section 02 | 1b stores, 5 acts |
| `steamEnabled`, `epicEnabled` | file, default `true` | `games` drops or adds that source | section 02 | 1b stores, 5 acts |
| `steamPath` | file override as typed (normalised), even when the folder is missing (section 02 then reports `failed`); else detected (6.1.3), else `""` | `games` re-reads Steam | section 02 | 1b stores, 2 detects, 5 acts |
| `epicPath` | file override, else detected (6.1.4), else `""` | `games` re-reads Epic | section 02 | same |
| `calendarConnected` | constant `false` | not changeable | Settings row | 1b |
| `monitorClaude` | file, default `false` | `true`: `sessions().install_hooks()`, then writes the key. `false`: `sessions().remove_hooks()`, then writes the key. Safe to repeat. | section 03 | 8 (before: `notBuilt`) |
| `monitorClaudeState` | `sessions().hooks_state()`; `off` before stage 8. Section 03 maps its states: installed with an event received is `on`, installed with none yet is `onInactive`, and a broken, blocked or unavailable install is `needsRepair` (the message goes in the `ApiError` of the failed call or in the Health source `claude-hooks`) | not changeable | Settings row | 8 |
| `gameModeCountdownSeconds` | file, default 10 | nothing at once; read when game mode starts | `gamemode` | 1b |

#### 6.1.2 Vault and daily-note settings (detected)

Source: `%APPDATA%\obsidian\obsidian.json` (Obsidian's own list; the path comes from `SHGetKnownFolderPath(FOLDERID_RoamingAppData)`, never from a hardcoded user name).

1. Read the file (a few hundred bytes, `core::blocking::run`). Missing, unreadable or not JSON means "no vault detected"; nothing is logged above `debug`.
2. Take the object `vaults`. Each value has `path` (string); the keys `ts` and `open` are not used. Keep entries whose `path` is an existing folder.
3. Choose (decisions.md, ruling of 2026-11-22 "Which vault is written"): if `vault_path` is set in `settings.toml`, that is the vault (normalised; it stays `Resolved.vault` even when the folder does not exist, so section 04 can tell a wrong path from no path). Else, if exactly one entry is left, that one is the detected vault. Else (zero or several entries) there is no vault and `Resolved.vault` is `None`. Reason: Obsidian offers no "default vault", and Rain must not read or write a vault the user did not choose; with several vaults the user chooses one in Settings.
4. The detected single vault is shown in Settings as the effective `vaultPath` (so the user sees what Rain reads). It is not written to `settings.toml`. With several vaults and no override, `vaultPath` is `""`, exactly as when Obsidian has no vault: the Settings row shows its one placeholder "Not set" (9.2; no API field says that several vaults exist, so no second text is possible), and Life shows its "Choose your vault" empty state (section 04 6.2).
5. With a vault: read `<vault>\.obsidian\daily-notes.json`. Keys `folder` and `format` (strings; other keys ignored; the file may have only `folder`, as on the owner's PC). A missing file or key gives "none". `folder` is normalised like `daily_note_folder`; one that is absolute or holds `..` is dropped.

Cost: two small file reads, cached 30 s (6.1.5). Failure: never an error; the value is just empty.

Never logged: the vault path, any note text (foundation 13.4).

#### 6.1.3 Steam path (detected)

1. `winreg` `HKCU\Software\Valve\Steam` value `SteamPath` (spike winapi `registry.rs`: forward slashes and a lowercase drive letter; normalise to backslashes and an upper-case drive letter).
2. If missing: `HKLM\SOFTWARE\WOW6432Node\Valve\Steam` value `InstallPath`.
3. Keep it only if it is an existing folder. Else `""`.

Cost: two registry reads (tens of microseconds), cached 30 s. The meaning of the folder (libraries, manifests, play time) belongs to section 02.

#### 6.1.4 Epic path (detected, UNPROVEN)

The owner's PC has the NEW launcher (read-only check, 2026-11-22; section 02 6.10.1) and not the old one. The rules: (1) the new launcher's data folder: the registry value `HKCU\Software\Epic Games\EpicGamesLauncher` `AppDataPath` (`%NAME%` expanded), else `%LOCALAPPDATA%\com.epicgames.eglv2`; it counts when it holds `installations.jsonl`. (2) The old launcher: if `%ProgramData%\Epic\EpicGamesLauncher` exists and holds `Data\Manifests`, that `EpicGamesLauncher` folder. The effective path is (1) when it counts, else (2), else `""`; when both count, (2) is also given to section 02 as `Resolved.epic_extra_path` (`Detect::epic_extra`) so that both formats are read. A typed `epicPath` may point at either kind of folder, and then nothing extra is read. This is only a definition of what `epicPath` points at; section 02 owns the real reading of manifests and may change the definition (point O2). `""` is not a failure: the source is `off` in `GET /health` (foundation 6.9).

#### 6.1.5 Caching

All detectors sit behind `trait Detect` (a fake in tests). `Detected { vault, note_format, note_folder, steam, epic }` is computed on demand and kept for `DETECT_TTL` = 30 seconds; the age is checked at read time, never by a timer. A `PATCH` that changes `vault_path`, `steam_path` or `epic_path` clears the cache. A user who installs Steam sees it in Settings after at most 30 seconds or the next `PATCH`.

#### 6.1.6 First run, and how this PC gets its values

First run (no data folder, no files): the backend creates the data folder and writes nothing (foundation 10.1). `GET /settings` returns the defaults of 4.1 plus the detected values. Nothing personal is in the code.

| Value | First run result | How the user changes it |
|---|---|---|
| data folder | `%APPDATA%\Rain\data` | Settings, "Data folder" |
| vault | the only vault Obsidian knows, or empty (several vaults: empty, the user chooses) | Settings, "Obsidian vault" |
| Steam | detected, or empty | Settings, "Steam library" path |
| Epic | detected, or empty | Settings, "Epic library" path |
| project folders, game folders | empty | Settings |
| everything else | the defaults | Settings |

There is no wizard. Every screen that needs a value shows a true empty state whose action goes to Settings (Projects "No project folder yet", Games "No game folder yet", Life "No vault found"; the texts belong to the owning sections). The owner's PC is set up in this order: Data folder (the backend restarts and reads the existing files), Project folders, Game folders; the vault is detected. The Settings screen is the first-run step.

#### 6.1.7 What other modules read: `Resolved`

Other modules never parse `settings.toml` and never call `GET /settings`. They call the public `Service`:

```rust
// modules/settings/service.rs
impl Service {
    pub fn resolved(&self) -> Arc<Resolved>;                       // cheap; recomputes detection if the cache is older than 30 s
    pub fn watch(&self) -> watch::Receiver<Arc<Resolved>>;         // fires when settings.toml changes the value
}
pub struct Resolved {                                              // plain Rust, not an API type
    pub vault: Option<PathBuf>,                  // the chosen vault, normalised, whether or not it exists: the override when `vault_path` is set (even if the folder is missing), else the single existing detected vault, else None
    pub daily_note_folder: String,               // relative, "/" separated, may be empty
    pub daily_note_format: String,               // effective, never empty
    pub project_roots: Vec<PathBuf>,             // normalised, absolute entries only, de-duplicated
    pub game_folders: Vec<PathBuf>,
    pub steam_enabled: bool, pub steam_path: Option<PathBuf>,
    pub epic_enabled: bool,  pub epic_path: Option<PathBuf>,
    pub countdown_seconds: u32,
    pub monitor_claude: bool,
}
```

Path normalisation has ONE rule and ONE function. `settings::Service::expand_env(&self, raw: &str) -> String` expands `%NAME%` with `std::env::var` (an unknown variable stays as typed); it is a `Service` method, so section 05 calls it in `POST /paths/check` (foundation 11.2) and the glyph and `Resolved` agree. Then the shared pure function `core::paths::normalize(raw, allow_unc) -> Result<PathBuf, PathProblem>` (file `core/paths.rs`, foundation 11.1; the rules 1 to 8 are written in section 05 6.9) runs with `allow_unc = true`. So the value of `Resolved` is `normalize(expand_env(entry), true)`, and `settings::paths::normalize` no longer exists as a second function (`settings::paths` keeps only the private `Resolved` builder). An entry that fails any rule (quotes are stripped by rule 1, but `..`, `C:foo`, `\\?\` and `\\.\` paths, control characters, more than 1024 characters, no drive letter) is dropped from `Resolved` but stays in the API value, so the user sees it and the glyph says "Missing" (section 05 gives the same answer for it). The text typed by the user is stored as typed.

`GET /vault` of section 04 and the glyph of Settings must agree: both use `Resolved.vault` (the chosen vault, which may not exist; `Settings.vaultPath` shows the same value). The glyph (`checkPaths`) and `VaultStatus.connected` agree because both test the same path with `is_dir`. Section 04 takes folder and format from `Resolved.daily_note_folder` and `Resolved.daily_note_format` and reads no `daily-notes.json` itself. The vault id that section 04 needs for `obsidian://` links comes from one shared reader in this module, `Service::vault_id() -> Option<String>` (no argument; the key of the effective vault in `obsidian.json`, read in 6.1.2 once; the effective vault is matched to an entry by path: case-insensitive, `/` and `\` equal, trailing slash ignored, as in section 04 6.7, so it also works when the user typed `vaultPath` and it matches an entry), so `obsidian.json` is read in one place.

### 6.2 `PATCH /settings`

Rules (the code is one function, `settings::service::apply_patch`):

| Rule | Text |
|---|---|
| R1 | Parse the body as `SettingsPatch`. A parse error, an unknown field or a number outside its set is `400 invalid`, reason `badBody`. |
| R2 | A patch with an external-effect field (`startWithWindows`, `dataDir`, `monitorClaude`) must contain only that one field: otherwise `400 invalid`, reason `onePerRequest`, message "Change this setting on its own." Reason: "all or none" then holds without a rollback of a registry value or an installed hook. The Settings screen sends one field per change already. An empty patch `{}` is a valid no-op and returns the current state. |
| R3 | Validate every field with table V. The first failure rejects the whole patch. |
| R4 | If `settings.toml` does not parse as a whole (a syntax error, or a value of the wrong type, 4.1) or has a newer `schema_version`: `409 conflict`, reason `configBroken` or `configNewer`, before anything is applied. |
| R5 | Sandbox and real mode (foundation 2.7): `startWithWindows` is refused in sandbox (`409 conflict`, reason `sandbox`). `monitorClaude` is not refused here: it passes to `sessions().install_hooks()` or `remove_hooks()`, whose `allowed(target)` rule (section 03 6.9.8) answers `409 conflict` `sandbox` when the canonical path to the Claude settings file does not lie inside the system temp folder, or the data folder or local folder of this run (so a sandbox run with `RAIN_CLAUDE_DIR` set to a throwaway folder works, and the real file is never touched). |
| R6 | A file-only patch: one `TomlFile::edit` writes all its keys in one atomic write. An external-effect patch: do the effect first, write the key (if there is one) only after it succeeded. |
| R7 | Return the complete new `Settings` and publish it on topic `settings` (only when something changed). |

Table V (validation of values):

| Field | Check | Failure |
|---|---|---|
| any string | no control characters (code below 32 or 127); trimmed first | `invalid` `badText` |
| `vaultPath`, `steamPath`, `epicPath`, entries of the lists | at most 1024 characters; existence is NOT checked (the user may type a folder that does not exist yet; the glyph says "Missing"; the mock behaved the same) | `invalid` `tooLong` |
| `projectRoots`, `gameFolders` | at most 32 entries; entries trimmed; empty entries dropped; duplicates dropped case-insensitively after `normalize` (first spelling kept) | `invalid` `tooMany` |
| `dailyNoteFormat` | at most 64 characters; none of `< > : " \| ? *` or `\`; `/` is allowed (it makes sub-folders, as in Obsidian); then `life::Service::check_note_format` if section 04 offers it (point O4) | `invalid` `badFormat`, message "Not a valid date format" |
| `dailyNoteFolder` | not absolute, no drive, no `..` segment, at most 128 characters; stored with `/` | `invalid` `badFolder`, message "Use a folder inside the vault, like Daily" |
| `dataDir` | 6.3 | see 8.1 |
| `startWithWindows` | 6.4 | see 8.1 |
| `monitorClaude` | none (the sessions module reports its own errors) | pass-through |

Empty string means "clear the override" for `vaultPath`, `dailyNoteFormat`, `dailyNoteFolder`, `steamPath`, `epicPath`; for `dataDir` it means "back to the default folder".

Hand edit of `monitor_claude` in `settings.toml`: after every reload the settings module calls `sessions().hooks_state()` and reports a mismatch (the key says on and the state is `off` or `needsRepair`, or the key says off and the hooks are installed) as a `ConfigIssue` of file `settings.toml`; it never installs or removes hooks by itself because of a hand edit.

Sequence for a file-only patch: `refresh()` (length and modified time), R4, R3, `TomlFile::edit(|doc| set or remove each key)`, reload, compose `Settings` (6.1), publish. Cost: one file write on a blocking thread, a few milliseconds.

Sequence for `monitorClaude: true`: R2, R4, R5, `sessions().install_hooks()` (section 03; idempotent: a changed forwarder path is replaced; it reads the file back and verifies), then `edit` sets `monitor_claude = true`. If the install fails: nothing is written, the error of the sessions module is returned (it is already a plain sentence). `false`: `sessions().remove_hooks()` first. A repair is `monitorClaude: true` while the state is `needsRepair`: the same call. Hard rule: during development and tests the install only ever targets a throwaway folder (foundation 13.3 rule 2); this module never names the Claude settings file.

### 6.3 Changing the data folder

Decision: re-point, never move. Re-point is when the new folder already holds Rain files: Rain reads them as they are. If the new folder holds none and the old one has some, Rain copies them (never overwrites, never deletes the old ones). Reasons: the owner keeps the folder in a private git repository, so a silent move would damage it; a friend who sets a new folder early would otherwise lose the few settings he already changed (they live in the old folder). The change takes effect through a backend restart, not through a live re-pointing of every module, because every module already re-reads its files at start and the foundation already has a `restart` stop reason: one mechanism instead of twelve re-point hooks.

Algorithm for `PATCH { dataDir }`:

1. R2 (alone). If `RAIN_DATA_DIR` is set: `409 conflict`, reason `environment`, "The data folder is set by RAIN_DATA_DIR." If game mode is not `off`: `409`, reason `gameMode`. If `ping.busy`: `409`, reason `busy`, "Wait until the running jobs end, then try again."
2. `""` means the default folder. Otherwise `normalize`; it must be absolute (`400 badPath`, "Enter a full folder path, like D:\Rain"), must not be an existing file (`400 notAFolder`), must not equal, contain or lie inside the current data folder (`400 nested`, "Choose a folder outside the current one"), and must not lie inside the local folder (`400 nested`).
3. Create the folder if missing (`create_dir_all`). Write and delete a probe file `.rain-write-test-<pid>` in it. Failure: `400 notWritable`, "Rain cannot write to that folder."
4. Empty test: the target holds none of the known names `settings.toml, pins.toml, pools.toml, app-ids.toml, projects.toml, games.toml, jobs.toml, idle-jobs.toml, habits.toml, media\`. If it is empty and the current folder holds at least one of them: copy each known file and the `media` folder, never overwriting, files only (no symbolic links). Failure: `500 failed`, reason `copyFailed`; the partly copied files stay (harmless) and `machine.toml` is NOT changed.
5. Write `machine.toml` (key `data_dir`, or remove it for the default). Failure: `500 saveFailed`.
6. Answer `200` with `Settings` whose `dataDir` is the new folder, then after `RESTART_DELAY` = 500 ms call `shutdown::begin(Restart)`. The delay lets the page finish its own refetch.
7. The shell sees `lifecycle { stopping, restart }`, state `restarting`, starts a new backend as soon as the pipe is gone (foundation 2.1, 6.2). The new backend reads `machine.toml`, opens the new folder, `hello` has a new `startId`, the page refetches everything (foundation 5.5).

In sandbox the same code runs; if no shell is connected (plain browser with `npm run dev`) nothing starts the backend again: the developer starts it with `npm run backend:start`. The Settings screen says "Data folder changed" in both cases.

The harness cannot restart a backend by itself: `TestBackend::restart()` is the helper that stops it and starts a second one on the same folders (section 10).

### 6.4 Start with Windows

Mechanism: the registry value `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` name `Rain`, type `REG_SZ`, data `"<shell exe>" --hidden`. The program that starts is the shell, because the global hotkey lives there; the shell starts the backend (foundation 2.9). The `winreg` calls are the ones proven in spike winapi (`registry.rs`: `create_subkey`, `set_value`, `get_value::<String, _>`, `delete_value`).

```rust
// platform/registry.rs: behind a trait so tests never touch the real registry
pub trait RunKey: Send + Sync {
    fn read(&self) -> io::Result<RunState>;              // never an error for "missing"
    fn enable(&self, command: &str) -> io::Result<()>;   // writes Run\Rain, deletes StartupApproved\Run\Rain
    fn disable(&self) -> io::Result<()>;                 // deletes Run\Rain; a missing value is success
}
pub struct RunState { pub command: Option<String>, pub disabled_by_windows: bool }
```

| Item | Rule |
|---|---|
| Effective state | `command.is_some() && !disabled_by_windows`. So a user who disables Rain in Task Manager sees the switch off. |
| `disabled_by_windows` | `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run` value `Rain` is `REG_BINARY` and its first byte is odd (`0x03` is "disabled"; `0x02` and `0x06` are "enabled"). A missing key, a missing value or any other shape means "enabled". UNPROVEN (from memory of Windows behaviour, not a spike); a unit test covers the rule and acceptance check S6 checks it by hand. |
| Switch on | needs `shell_facts.shell_exe` (the path the shell reported, never a guess). No shell ever reported: `409 conflict`, reason `noShell`, "Open the Rain app once, then try again." Sandbox: `409`, reason `sandbox`. The command is `"<exe>" --hidden`; a path with a `"` is refused (`400 badPath`). Then `enable`. A failure is `500 failed`, reason `registry`, "Could not change start with Windows". |
| Switch off | `disable`. |
| Self-heal | after every `PUT /internal/shell` in real mode, if the Run value exists and differs (case-insensitive) from the command built from the reported exe, rewrite it. Reason: a reinstall to another folder or a changed install name must not leave a dead entry. Nothing is written when the value does not exist (the user has not opted in). The log says "autostart entry refreshed", without the path. |
| Default | off. Nothing runs at logon unless the user switched it on. |
| What logon does | `rain.exe --hidden`: the window is created hidden, the hotkey is registered, the backend is started, `summon` is not called. The page loads hidden, so the stream and the resources are warm at the first hotkey. |
| Cost | the warm webview (tens of megabytes) is resident from logon. This is the cost the owner accepted for a warm launcher (project notes); the switch is how it is avoided. |

UNPROVEN (foundation appendix C): the backend started from a Run-key shell is alive after that shell exits; whether `CREATE_BREAKAWAY_FROM_JOB` is allowed there. Checked by hand in acceptance check S5.

### 6.5 Hotkey and shell facts

- The shell registers `Ctrl+Backquote` as today (`lib.rs`), and after `hello` reports the facts with `PUT /internal/shell` (foundation 6.5), and again whenever a fact changes.
- Handler (core, `server/builtin.rs`, stage 1b): store in `state.shell_facts`, publish `health`, then call `settings::Service::on_shell_facts_changed()` (stage 1b): the settings module recomposes `Settings` and publishes topic `settings` only when the computed `hotkeyAvailable` changed (core cannot build the `Settings` payload). From stage 2 the handler then also calls `lifecycle::Service::on_shell_facts(&ShellFacts)`, which does only this: if `hotkey_registered` is `false` log `warn` "hotkey not registered" (the error text is the OS message) and set the `health` source `hotkey` to `failed` with that message; run the self-heal of 6.4.
- `Settings.hotkeyAvailable` is `true` when `state.shell_facts` is `None` (no shell ever reported), else `facts.hotkey_registered` (`state.shell_facts` is an `Option<ShellFacts>`, foundation 11.3; `None` is also what feeds `Health.shell.reported = false`).
- Shell rule added by this section: a second launch (the single-instance callback) retries the registration once and reports again. Reason: another program may have released the chord since; the user's own action is the only trigger, so no timer is needed.
- Not built: changing the chord. If it is wanted later the shell reads the chord from `GET /settings` and `hotkey` becomes a writable field; nothing in this design blocks it.

### 6.6 `POST /paths/check` (not in this section)

Section 05 owns the route, the types `PathCheck` and `PathKind`, the handler and the delivery in stage 3 (foundation 14.3 puts `checkPaths` in stage 3 with disk; the registry row has module `disk`). This section defines no route, no type and no constant for it. It only uses the result: the Settings rows show the glyph that section 05's `usePathCheck` gives, and an empty field has no glyph and shows the placeholder (9.2 step 3).

### 6.7 Game mode

#### 6.7.1 What game mode needs from other modules

All calls go through the other module's public `Service` (foundation 11.2). The first trait is new in `core` (point A2):

```rust
// core/winddown.rs
pub struct Counts { pub enabled: u32, pub running: u32 }       // running = runs in progress that a stop would interrupt
pub enum StopOutcome { Stopped, Forced, Failed }
pub struct Stopped { pub name: String, pub outcome: StopOutcome }
pub trait WindDownPort: Send + Sync {
    fn counts(&self) -> Counts;
    fn running_watch(&self) -> watch::Receiver<u32>;            // wakes the wait phase; no timer
    fn pause(&self, paused: bool);                              // true: start no new run; false: resume, and a job that came due meanwhile runs once
    fn stop_all(&self, budget: Duration) -> BoxFuture<'_, Vec<Stopped>>;
        // ends every run (TerminateJobObject, then checks the job is empty); records outcome `stopped` (status `idle`) with reason `gameMode` in its own history
}
```

| Needed | From | Used for |
|---|---|---|
| `state.jobs().wind_down()` and `state.idle().wind_down()` returning `&dyn WindDownPort` | section 03 | items 2 and 3 |
| `state.sessions().open_user_sessions() -> u32` | section 03 | item 1: open sessions that are not runs of Rain's own jobs (those belong to item 2) |
| `state.games().get(id) -> Option<Game>` (title, installed) | section 02 | validate `thenLaunchGameId`, name it in the recap |
| `state.games().launch_after_game_mode(id) -> BoxFuture<ApiResult<LaunchedAfterGameMode { in_job }>>` | section 02 | start the game outside Rain's process tree, never in a job (decision C10); it records "last played" like any launch; `in_job` is true when the backend itself runs inside a job and the game may close with Rain |
| `state.disk().cancel_running()` | section 05 | the silent step 1b of 6.7.5: a running scan is cancelled at once |
| `state.settings().resolved().countdown_seconds` | this section | countdown length |

Each section writes its half in its own file; point O5 lists them.

#### 6.7.2 The state machine

`gamemode/machine.rs` is pure: no clock, no I/O, no tokio (`Machine::step(&mut self, input) -> Result<Vec<Effect>, Refusal>`), so every row below is a table-driven unit test. The service turns the effects into calls.

Phases and what moves them:

| From | Event | To | Effects |
|---|---|---|---|
| `off` | `start` (items measured; some `interrupts`) | `review` | set `game_mode_active`, `pause(true)` on jobs and idle jobs, publish |
| `off` | `start` (nothing interrupts) | `countdown` | the same, plus `ends_at = now + countdown` |
| `review` | choice `wait`, not yet waiting | `review`, `waiting = true` | `wait_deadline = now + 180 s` |
| `review` waiting | running work reaches 0 | `countdown` | `waiting = false` |
| `review` waiting | `wait_deadline` passes | `review` | `waiting = false`, `waitTimedOut = true` |
| `review` | the work that interrupts reaches 0 by itself | `countdown` | the gate is satisfied |
| `review` | choice `stopAnyway` | `countdown` | `waiting = false` |
| `review` | no user action for 10 minutes (`REVIEW_ABANDON`) | `off` | as `cancel` |
| `countdown` | each second | `countdown` | `secondsLeft` recomputed from `ends_at`, publish |
| `countdown` | `ends_at` passes, or choice `closeNow` | `windingDown` | start the wind-down task (6.7.5) |
| `review`, `countdown` | choice `cancel` | `off` | in this order: (1) clear `game_mode_active` and `thenLaunchGameId`, (2) `pause(false)` on jobs and idle jobs (it runs after the flag is clear, so the scheduler, the idle sampler and the catch-up run are not refused), (3) `publish_final` (a cancelled run ends with its last state at once, no stale frame behind it) |
| `windingDown` | last step done | `done` | `publish_final` `done`, freeze the bus, begin shutdown with reason `gameMode` |

Answers to a choice, by current phase. "no-op" means `200` with the current state and no change. This keeps a double click harmless, as the mock did.

| Phase | `wait` | `stopAnyway` | `closeNow` | `cancel` |
|---|---|---|---|---|
| `off` | `409 notRunning` | `409 notRunning` | `409 notRunning` | no-op |
| `review`, not waiting | start waiting | to `countdown` | no-op | to `off` |
| `review`, waiting | no-op | to `countdown` | no-op | to `off` |
| `countdown` | no-op | no-op | to `windingDown` | to `off` |
| `windingDown`, `done` | no-op | no-op | no-op | `409 windingDown`, "Rain is already closing." |

`start` while a run is under way returns that run (idempotent). `start` in `done`: `409 conflict`, reason `stopping`. An unknown `thenLaunchGameId`: `404 notFound`, reason `unknownGame`; a game that is not installed: `409 conflict`, reason `notInstalled`, "That game is not installed." Neither changes any state. Reason for refusing instead of accepting: the wind-down would otherwise end Rain and then fail to start the game.

Order inside `start`: validate the game, then `pause(true)` on jobs and idle jobs, then measure the items (so a run that starts between the two cannot be missed), then choose `review` or `countdown`.

Every state change increments `rev` (it starts at 1 for each backend process) and publishes the whole state.

#### 6.7.3 Timings

| Name | Value | Notes |
|---|---|---|
| Countdown | 5, 10, 20 or 30 s from Settings, read at start | changing the setting during a run changes nothing |
| `GAME_MODE_WAIT_LIMIT` | 180 s | "a process that needs less than about three minutes" (the user's words). No estimate of the remaining time is made: one fixed rule is predictable. A hint from past run durations can come later without a contract change. |
| `REVIEW_ABANDON` | 10 minutes | game mode left in `review` with nobody choosing would block the scheduler for good; after 10 minutes it cancels itself |
| `GAME_MODE_ITEM_BUDGET` | 5 s per stop (jobs, idle jobs) | the stop is `TerminateJobObject`; the budget is for checking that the job is empty. If it is exceeded the step is recorded as `failed` and the wind-down goes on: the process exit closes every job handle and `KILL_ON_JOB_CLOSE` kills what is left (proven, spike process Q4). |
| `GAME_MODE_LAUNCH_BUDGET` | 10 s | the call that starts the game |
| `GAME_MODE_ITEM_MIN_VISIBLE` | 400 ms | each row stays in `stopping` at least this long, so the user sees it; the `session` row has no step |
| Whole wind-down | about 1.5 s with nothing running; about 25 s in the worst case | |
| Backend exit after `done` | core shutdown, at most 8 s (`SHUTDOWN_BUDGET`) | |
| Shell exit | pipe gone plus `GAME_MODE_EXIT_GRACE` = 3 s | |

The countdown tick is a loop of `tokio::time::sleep_until` to the next whole second of `ends_at` (a monotonic `tokio::time::Instant`), not an `interval`. `secondsLeft = ceil(ends_at - now)`. After a sleep of the PC the next wake sees `ends_at` in the past and the wind-down starts (accepted: the user asked for it).

#### 6.7.4 Items: counts and effects

Measured at start, then recomputed in `review` and `countdown` whenever `running_watch` of jobs or idle jobs changes (throttled to 4 per second) and on every countdown tick for the session count (an in-memory read).

| Item | `count` | `effect` |
|---|---|---|
| sessions | `sessions().open_user_sessions()` | always `leftRunning`; `status` stays `pending` |
| jobs | `counts().enabled` | `interrupts` if `counts().running > 0`, else `safe` |
| idle jobs | `counts().enabled` | `interrupts` if `counts().running > 0`, else `safe` |
| service, window | 1 | `safe` |

The heading of the screen counts the items with `interrupts`, as today ("2 things will be interrupted"). The sessions row never counts.

#### 6.7.5 The wind-down, in order

Runs in one task. Each stop is independent: a failure is recorded and the next step runs.

| # | Step | Row | Budget | What happens | Record |
|---|---|---|---|---|---|
| 0 | begin | none | | `begin_record` (SQL); `rev` up | row `running` |
| 1 | sessions | `wind-sessions` | 0 | nothing is stopped; the open sessions are only counted | item `leftRunning`, count N (only if N > 0) |
| 1b | scan | none (silent) | 2 s | runs when `game_mode_active` is set at start (not at the wind-down): `disk().cancel_running()`; a scan with 8 threads must not run through the countdown and the review, which is what game mode frees. No row, no record item | none |
| 2 | scheduled jobs | `wind-jobs` | 5 s | `jobs().wind_down().stop_all(5 s)`; row `stopping` for at least 400 ms, then `stopped` | one item per stopped run (name, outcome) |
| 3 | idle jobs | `wind-idle-jobs` | 5 s | the same with `idle()` | the same |
| 4 | the game | none (the "Then launch" line) | 10 s | if `thenLaunchGameId`: `games().launch_after_game_mode(id)`; success: `launch_result = started` (when `in_job` is true the recap adds "The game may close with Rain"; game mode still exits, because the user asked for it); failure or timeout: `launchError` is set on the state and `launch_result = failed` | launch result |
| 5 | background service | `wind-service` | | row `stopping`; `finish_record` (outcome `completed`); at least 400 ms; row `stopped` | |
| 6 | Rain window | `wind-window` | | row `stopping`, at least 400 ms, row `stopped` (the shell closes the window about 3 s after the pipe is gone) | |
| 7 | done | | | phase `done`, `Throttle::publish_final(state)` (the last topic frame; cancels any pending value), then `EventBus::freeze()`, then core shutdown with reason `gameMode` (6.8) | |

Why the game starts at step 4 and not later: the call can still report its failure on screen and in the record, and the work that needs the PC is already stopped. The game is a separate process that is not in any job of Rain (decision C10), so it outlives the backend. Why rows 5 and 6 report `stopped` a moment before the process really ends: `done` must be the last frame and core shutdown starts after it; what remains after `done` is closing a database and the pipe, which cannot fail visibly. Module `stop()` functions publish no frames (foundation 2.6), so nothing follows `done` on the bus.

A launch or a git refresh that is still running is ended by its module's `stop()` in core shutdown (a scan is ended earlier, by step 1b). They are not rows: they are not work the user would wait for.

#### 6.7.6 Work while game mode is not `off`

From `start` until `cancel` (or the end of Rain): `game_mode_active = true`. Jobs and idle jobs start nothing (`pause(true)`); scans, launches, git, Steam and vault refreshes refuse with `conflict`, reason `gameMode` (foundation 11.5). Reads of cached data work. On `cancel`, in this order: clear `game_mode_active`, then `pause(false)` on jobs and idle jobs (never before the flag is clear: the scheduler and the start path check the flag), then `publish_final`; a job that came due while paused runs once now (the rule "a missed run runs once" of section 03). After a finished game mode: the jobs that came due while Rain was off run once at the next start (decisions.md section 5, section 03). A run that game mode interrupts is recorded with outcome `stopped` (status `idle`) and, as for any missed run, runs once at the next start (section 03, `catch_up`).

#### 6.7.7 The record and the first load afterwards

1. The record is written as in section 5. A record that stayed `running` (the backend died) becomes `incomplete` at the next start.
2. The next backend start has `GameModeState.last` in phase `off`, built from the newest finished record, with `seen = false`.
3. The app frame (`AppShell.vue`, composable `useGameModeRecap`, 9.5) shows one notice at the first moment the window is visible, for example "Game mode stopped 2 jobs. 3 Claude sessions kept running." or, with nothing to stop, "Game mode closed Rain."; plus "The game did not start." when `launchError` is set, or "Game mode did not finish." when `complete` is false. Then it calls `POST /game-mode/recap/seen`. A hidden start at logon shows it at the first hotkey.
4. The first load after game mode "may take a little longer" (the user's words): nothing is done to hide that.

#### 6.7.8 The driver (shape of the code)

```rust
// gamemode/service.rs: one task per run; none exists while game mode is off (zero idle cost)
async fn drive(svc: Arc<Service>, mut cmds: mpsc::Receiver<Cmd>) {
    let mut jobs_rx = svc.jobs.running_watch();
    let mut idle_rx = svc.idle.running_watch();
    loop {
        let wake = svc.machine.lock().next_wake();            // next whole countdown second, wait_deadline or abandon deadline
        tokio::select! {
            Some(cmd) = cmds.recv() => svc.apply(cmd.input, cmd.reply),                 // choices come in here
            _ = tokio::time::sleep_until(wake) => svc.apply(Input::Tick, None),         // T4 and T5 (section 7)
            _ = jobs_rx.changed() => svc.apply(Input::Running { jobs: *jobs_rx.borrow() }, None),
            _ = idle_rx.changed() => svc.apply(Input::Running { idle: *idle_rx.borrow() }, None),
            _ = svc.shutdown.cancelled() => return,
        }
        match svc.machine.lock().phase() {
            Phase::Off => return,                                // cancelled
            Phase::WindingDown => { svc.wind_down().await; return }   // ends with core shutdown; the process exits
            _ => {}
        }
    }
}
```

The built driver wakes on a `Notify` that only the callers from outside signal (`start` when a driver already runs, and `choose`). The inputs the driver gives its own machine (`Items`, `Tick`) never signal it: a permit that the driver stored for itself made its `select!` ready at once, and the loop spun on one core and starved the task that writes the stream (found on the real PC, 2026-11-23). A test with paused time (`the_driver_sleeps_and_publishes_one_frame_each_second`) counts the measures and the frames.

`apply` runs `Machine::step`, performs the returned effects (`Publish`, `Pause`, `BeginWait`, `StartWindDown`) and answers the reply channel with the new state or the refusal. The lock is never held across an `await`.

### 6.8 How Rain ends, and the shell's part

Backend (foundation 2.6, with this section's numbers):

1. `gamemode` publishes `done` with `Throttle::publish_final` (the last topic frame) and then calls `EventBus::freeze()`.
2. `core::shutdown::begin(GameMode)`: mark `shutting_down`; every open stream writes `lifecycle { stopping, gameMode }` as its tail; every module's `stop()` runs in parallel with 5 s each (the job modules end any run that is left; the database commits); streams close; the accept loop stops; `PRAGMA wal_checkpoint(TRUNCATE)`; the process exits with code 0. The whole is capped at 8 s.
3. Nothing in the backend starts anything afterwards: no task, no timer, no scheduled task, no Run-key entry created for the occasion. (The Run key exists only if the user switched "Start with Windows" on; that is the next logon, not a revival.)

Shell (`relay.rs`, `backend.rs`):

```rust
// on the lifecycle frame with reason "gameMode"
backend.set_state(Stopped);                                  // api_request now answers `unavailable`, reason `stopped`
let handle = tauri::async_runtime::spawn(async move {
    wait_until_pipe_gone(&client, Duration::from_secs(5)).await;   // poll ping every 100 ms; "file not found" = gone
    tokio::time::sleep(GAME_MODE_EXIT_GRACE).await;                // 3 s: the user reads "Rain is off"
    app.exit(0);                                                    // drops the hotkey and the webview
});
backend.set_exit_task(handle.abort_handle());
```

- The window may be hidden: the same sequence runs.
- A second launch of the shell during the grace (the user clicks the Start menu entry): the single-instance callback aborts the exit task, calls `ensure_backend()` and summons the window. This is the user's own action, not a revival. Without it the second launch would summon a window that closes a second later.
- The relay never calls `ensure_backend` after reason `gameMode`.
- `app.exit(0)` ends the shell and with it its webview processes (UNVERIFIED; acceptance check G6 lists the processes that must be gone).
- `rain-hook` is not resident. A Claude session that stays open keeps calling the hook; the hook finds no pipe and exits 0 with empty output within about 20 ms (fail open, decision C2), so the session behaves as if Rain were not installed.

### 6.9 Stop-reason policy (completes foundation 2.6)

| Reason | Sent by | Refused when | What runs | What the shell does |
|---|---|---|---|---|
| `gameMode` | the `gamemode` module, internally | never | the wind-down of 6.7.5, then core shutdown | exit sequence of 6.8 |
| `stop` | `rain-backend --stop`, the installer, scripts | never | module stops end any run that is left (the log gets a `warn` with the counts) | state `stopped`; the next trigger of foundation 2.2 starts a backend again |
| `upgrade` | the shell (version or path differs) | `409 busy` when `ping.busy` | module stops | `restarting`, starts the new backend |
| `restart` | the `settings` module (data folder change) | `409 busy` when `ping.busy`; `409 gameMode` while game mode is not `off` | module stops | `restarting`, starts the new backend |

`--stop --force` and the reaction to a missing pipe stay as in foundation 1.3. A busy backend is never ended by `upgrade` or `restart`; only `stop` and `--force` end running work.

### 6.10 Two rules for the page (the algorithms; the code is in 9.5)

Reset to Home must not cancel game mode. Today the mock freezes its timers while the window is hidden, so the reset to Home after `resetAfterSeconds` never meets a running game mode. The real backend keeps counting. The reset unmounts the Game mode screen, whose `onBeforeUnmount` then calls `cancel`. Fix: the reset timer does nothing while the phase is `review`, `countdown` or `windingDown`. Shift+Esc (hide now) stays allowed: game mode goes on while the window is hidden, and the shell exits at the end. Reading the phase uses the shared `gameMode` resource that the app frame keeps alive, so no fetch is needed when the timer fires.

Late states are dropped. A response and a pushed frame travel on different channels, so the push may arrive after the response and after the screen has left. The stream client applies the state returned by `startGameMode` and `resolveGameMode` to its `gameMode` listeners before the call resolves (the same order the mock had), and drops any pushed or returned state whose `rev` is not higher than the highest it has applied since the last `hello` (a new `startId` resets it). `getGameMode` raises the mark but is never dropped.

---------------------------------------------------------------------------------------------------

## 7. Background work

No row uses a timer that foundation 11.5 does not list. Game mode's tick and limits are written into T4 and T5 of foundation 11.5 (done, point A4).

| Task | Trigger | Timer row (11.5) | Cost | In game mode | Stops |
|---|---|---|---|---|---|
| settings.toml reload | `notify` event (foundation 10.4); before stage 2 a length-and-time check on read | none (event) | none while idle | unchanged | with the backend |
| detection cache | read time; age checked on each `GET`, 30 s | none | microseconds | unchanged | none |
| autostart self-heal | the `PUT /internal/shell` call | none | one registry read | unchanged | none |
| restart after a data folder change | one `sleep(500 ms)` after the `PATCH` answer | T4 | one timer, dropped after | refused while game mode runs | fires once |
| game mode driver | exists only from `start` to `cancel` or the end | none for the task | idle between wakes | this is game mode | cancel, `done`, shutdown |
| countdown tick | `sleep_until` the next second of `ends_at`, only in `countdown` | T5 (name) | at most 30 wakes per run | | leaves `countdown` |
| wait limit (180 s), review abandon (10 min) | one `sleep_until` each, only while `review` | T4 (name) | two timers while waiting | | leaves `review` |
| running-count watchers | `watch::Receiver::changed` of jobs and idle jobs | none (event) | none | | the driver returns |
| wind-down step delays (400 ms) | one `sleep` per step | T5 | 5 short sleeps | | the task ends |
| record write | three SQL calls through the `Db` thread | none | negligible | | |
| `gamemode_record` cleanup | inside `begin_record` | none | one `DELETE` | | |

When game mode is `off` the module has no task, no timer and no memory beyond the last recap (a few hundred bytes). The idle cost of this whole section is zero wake-ups.

---------------------------------------------------------------------------------------------------

## 8. Errors

Messages are for the user, plain English, never a raw OS error alone (foundation 4.5). `source` is `settings`, `lifecycle` or `gamemode` unless the table says otherwise.

### 8.1 Settings, paths, lifecycle

| Situation | Status | `code` | `reason` | Message | The UI shows |
|---|---|---|---|---|---|
| module did not start | 503 | `unavailable` | `moduleFailed` | "Settings are not available." | ErrorRow "Settings unavailable" with Retry |
| body not valid, unknown or read-only field, number outside its set | 400 | `invalid` | `badBody` | "The request was not valid: ..." (foundation 4.5) | notice "<label> was not saved" with the message |
| external-effect field together with another | 400 | `invalid` | `onePerRequest` | "Change this setting on its own." | notice |
| too many list entries | 400 | `invalid` | `tooMany` | "Too many folders (at most 32)." | notice |
| text too long | 400 | `invalid` | `tooLong` | "That text is too long." | notice |
| control characters | 400 | `invalid` | `badText` | "That text has characters Rain cannot use." | notice |
| date format refused | 400 | `invalid` | `badFormat` | "Not a valid date format" | notice |
| daily notes folder not relative | 400 | `invalid` | `badFolder` | "Use a folder inside the vault, like Daily" | notice |
| data folder not absolute | 400 | `invalid` | `badPath` | "Enter a full folder path, like D:\Rain" | notice |
| data folder is a file | 400 | `invalid` | `notAFolder` | "That path is a file, not a folder." | notice |
| data folder inside or around the current one or the local folder | 400 | `invalid` | `nested` | "Choose a folder outside the current one." | notice |
| data folder not writable | 400 | `invalid` | `notWritable` | "Rain cannot write to that folder." | notice |
| `RAIN_DATA_DIR` set | 409 | `conflict` | `environment` | "The data folder is set by RAIN_DATA_DIR." | notice |
| running work blocks a restart | 409 | `conflict` | `busy` | "Wait until the running jobs end, then try again." | notice |
| game mode runs | 409 | `conflict` | `gameMode` | "Game mode is running." | notice |
| `settings.toml` has an error | 409 | `conflict` | `configBroken` | "settings.toml has an error on line N. Fix the file, then try again." (line from the issue) | notice; `GET /health` lists the issue |
| `settings.toml` is newer | 409 | `conflict` | `configNewer` | "settings.toml was written by a newer Rain." | notice |
| the file changed while editing | 409 | `conflict` | `configChanged` | "settings.toml changed. Try again." | notice |
| no shell reported its exe | 409 | `conflict` | `noShell` | "Open the Rain app once, then try again." | notice |
| sandbox build | 409 | `conflict` | `sandbox` | "This setting cannot be changed in a development build." | notice |
| the file cannot be written | 500 | `failed` | `saveFailed` | "Could not save settings.toml." | notice |
| registry write failed | 500 | `failed` | `registry` | "Could not change start with Windows." | notice |
| copy failed | 500 | `failed` | `copyFailed` | "Could not copy your data files to that folder." | notice |
| effect not built yet (stages before 2 or 8) | 503 | `unavailable` | `notBuilt` | "This setting is not available yet." | notice |
| `machine.toml` unusable at start | none | | | `ConfigIssue` (4.2) | `GET /health` |

Degraded sources in `GET /health`: `hotkey` (`failed` with the OS message when the shell reports `hotkeyRegistered: false`; absent otherwise). Config issues: `settings.toml`, `machine.toml`.

### 8.2 Game mode

| Situation | Status | `code` | `reason` | Message | The UI shows |
|---|---|---|---|---|---|
| module did not start | 503 | `unavailable` | `moduleFailed` | "Game mode is not available." | ErrorRow "Game mode unavailable" |
| `thenLaunchGameId` unknown | 404 | `notFound` | `unknownGame` (source `games`) | "Game not found" | notice "Game mode did not start" with the message |
| game not installed | 409 | `conflict` | `notInstalled` | "That game is not installed." | notice |
| start after `done` | 409 | `conflict` | `stopping` | "Rain is closing." | notice |
| backend shutting down | 503 | `unavailable` | `shuttingDown` | "Rain is closing." | notice |
| choice with nothing running | 409 | `conflict` | `notRunning` | "Game mode is not running." | notice |
| `cancel` after the wind-down began | 409 | `conflict` | `windingDown` | "Rain is already closing." | notice "Game mode was not cancelled" |
| body not valid | 400 | `invalid` | `badBody` | foundation text | notice |
| a stop failed (job still alive after 5 s) | none | | | recorded as `failed`; the recap says so; the wind-down goes on | the notice of the next start |
| the game did not start | none (state field) | | | `launchError` = the plain message of section 02, or "The game did not start in time." | a line under "Then launch", and the recap notice |
| the backend died during the wind-down | none | | | the record becomes `incomplete` at the next start | "Game mode did not finish." |

The Game mode screen's failure paths already exist: a rejected `startGameMode` is a notice, a rejected cancel is a notice, an unreachable backend is the ErrorRow.

---------------------------------------------------------------------------------------------------

## 9. Frontend changes

Line numbers are those of the working tree at commit `e4c7f70` (before any backend stage). Files other sections also touch are named only for the lines this section owns.

### 9.1 Contract changes (foundation 15.1)

| # | Item | From | To | Reason | Frontend files touched (full paths) | Breaking for the mock? | Stage |
|---|---|---|---|---|---|---|---|
| S6.1 | `Settings` | 14 fields | 19 fields: adds `dataDir`, `dailyNoteFolder`, `gameFolders`, `monitorClaude`, `monitorClaudeState`; effective values for the detected fields from stage 2 (3.1; in 1b the stored override or empty) | the settings this round needs | `src/data/types.ts` (the hand-written `Settings` at lines 89-93 is deleted when the barrel takes over), `src/features/settings/settingValues.ts`, `src/features/settings/settingDefs.ts`, `src/features/settings/SettingsScreen.vue`, `src/actions/types.ts` (`key?: keyof Settings` stays valid) | yes: `src/data/mock/seed/settings.ts` lines 9-24 get `dataDir: ''`, `dailyNoteFolder: ''`, `gameFolders: []`, `monitorClaude: false`, `monitorClaudeState: 'off'` | 1b |
| S6.2 | `updateSettings` | `(patch: Partial<Settings>)` | `(patch: SettingsPatch)` | the type names what may be sent; read-only fields cannot be named | `src/data/api.ts` line 90, `src/features/settings/SettingsScreen.vue` (`save`, `change`), `src/features/settings/settingValues.ts` (`patchFor`, `libraryPathPatch`), `src/features/settings/actions.ts` (lines 49 and 78) | no: `Object.assign(state.settings, patch)` in `mock/slices/settings.ts` line 18 accepts it | 1b |
| S6.3 | topic `settings` | none | `TopicPayload.settings: Settings` | a hand edit of `settings.toml` and a change from another window reach the page without a click (foundation 14.2 acceptance) | `src/data/api.ts` lines 14-21, `src/data/rest/topic-check.ts` (new, foundation 9.3; created with the `UnportedTopic` union `'sessions' | 'jobs' | 'idleJobs' | 'launch' | 'scan' | 'gameMode'`), `src/data/routing.ts`, `src/features/settings/SettingsScreen.vue` line 39, `src/keys/summon.ts` line 17 | yes: `src/data/mock/engine.ts` lines 145-152 (`listeners` is typed over every topic) gets `settings: new Set()` | 1b |
| S6.4 | `calendarConnected` | read-write boolean, row is a Connect button | read-only constant `false`, row is an info row "Calendar: Not connected" | no calendar this round; a `PATCH` naming it would now be `400` | `src/features/settings/settingDefs.ts` line 30, `src/features/settings/settingValues.ts` lines 22-23, 62-63 and 97, `src/features/settings/SettingsScreen.vue` lines 95-98, `src/features/settings/SettingRow.vue` line 154 | no | 1b |
| S6.5 | `checkPaths` and `PathCheck` | the synchronous `mockControl.pathExists` | owned by section 05 (its S.4, stage 3) | one definition of the route, types and hook | none here | no (`MockApi` of foundation 8.6; `pathExists` is deleted by section 05) | 3 |
| S6.6 | `SettingDef` | no placeholder; kind `button` | optional `placeholder`; kind `button` removed in stage 10 | empty is "not set", not "missing"; no button row is left after the calendar and the prototype rows go | `src/actions/types.ts` lines 42-49, `src/features/settings/settingDefs.ts`, `src/features/settings/SettingRow.vue`, `src/features/settings/actions.ts` lines 30-37 (`CHANGE_LABEL` loses `button`) | no | 3 (placeholder), 10 (button) |
| S6.7 | `GameModeState`, `WindDownItem` | as in `types.ts` lines 83-87 | 3.2: `rev`, `waitTimedOut?`, `launchError?`, `last?`, `effect` gains `leftRunning` | real stops, the user's sessions "left running", late-state rule, recap | `src/data/types.ts` (generated), `src/features/game-mode/GameModeScreen.vue`, `src/features/game-mode/WindDownRow.vue` | yes in principle (`rev` is required); in practice the mock slice `mock/slices/gameMode.ts` and `mock/seed/gameMode.ts` are deleted in the same stage, and `mock/seed/settings.ts` line 6 stops importing `DEFAULT_COUNTDOWN_SECONDS` (it writes `10`) | 9 |
| S6.8 | new method `markGameModeRecapSeen(): Promise<void>` | none | `POST /game-mode/recap/seen` | the recap notice is shown once | `src/data/api.ts`, `src/data/rest/client.ts`, `src/data/routing.ts`, new `src/composables/useGameModeRecap.ts` | no (never in the mock), but the name joins `RetiredMockMethod` in stage 9 (foundation 8.6) so that `createMockApi()` still type-checks | 9 |
| S6.9 | `startGameMode`, `resolveGameMode` | never reject | may reject (`notFound`, `conflict`) | real failure | `src/features/game-mode/GameModeScreen.vue` line 100 (`choose`), `src/keys/keymap.ts` lines 114-123 (`cancelGameMode`) | no | 9 |
| S6.10 | the REST client hands the returned game mode state to the stream client | not applicable | `deliver('gameMode', state)` before the call resolves; `rev` rule | late push after the screen has left (6.10) | `src/data/rest/client.ts`, `src/data/rest/stream.ts` | no | 9 |
| S6.11 | `settingDefaults` | seven entries, no path entries | adds `vaultPath: ''`, `dailyNoteFormat: ''` (was `'YYYY-MM-DD'`), `dailyNoteFolder: ''` | "Reset to default" now means "follow Obsidian again" | `src/features/settings/actions.ts` lines 20-28 | no | 1b |
| S6.12 | version text | `APP_VERSION = '0.1.0'` copied by hand | `__APP_VERSION__` from `package.json` through Vite `define` | one source for the About row | `src/features/settings/settingDefs.ts` line 67, `vite.config.ts`, `src/vite-env.d.ts`, `scripts/check.mjs` (the three versions must be equal) | no | 10 |

### 9.2 The Settings screen, step by step

The screen keeps the design contract: one 640 px column, rows grouped by `SectionHeader`, Enter acts on the selected row, no Save button, changes apply at once, icon-led and quiet. This section adds rows, not chrome.

#### Final row list (`src/features/settings/settingDefs.ts`)

Rows appear in the stage that gives them an effect (1.4). The final list, in drawing order:

```ts
export const settingDefs: SettingDef[] = [
  { id: 'hotkey', group: 'General', label: 'Summon hotkey', hint: 'Takes this chord from other apps while Rain runs', kind: 'info', key: 'hotkey' },
  { id: 'hide-on-blur', group: 'General', label: 'Hide when focus is lost', kind: 'switch', key: 'hideOnBlur' },
  { id: 'reset-after', group: 'General', label: 'Return to Home after', kind: 'select', key: 'resetAfterSeconds' },
  { id: 'start-with-windows', group: 'General', label: 'Start with Windows', kind: 'switch', key: 'startWithWindows' },
  { id: 'data-dir', group: 'General', label: 'Data folder', hint: 'Your pools, jobs and habits. Rain restarts its service when this changes', kind: 'path', key: 'dataDir' },   // stage 2

  { id: 'vault', group: 'Sources', label: 'Obsidian vault', hint: 'Follows Obsidian when it has one vault. Rain reads your daily notes and writes only the habit you tick', kind: 'path', key: 'vaultPath', placeholder: 'Not set' },   // one static placeholder for "no vault" and "several vaults, none chosen" (6.1.2)
  { id: 'note-folder', group: 'Sources', label: 'Daily notes folder', hint: 'Inside the vault. Follows Obsidian until you change it', kind: 'text', key: 'dailyNoteFolder' },   // stage 7
  { id: 'note-format', group: 'Sources', label: 'Daily note format', hint: 'Follows Obsidian until you change it', kind: 'text', key: 'dailyNoteFormat' },
  { id: 'project-roots', group: 'Sources', label: 'Project folders', hint: 'Separate several folders with a semicolon', kind: 'path', key: 'projectRoots', placeholder: 'Not set' },
  { id: 'game-folders', group: 'Sources', label: 'Game folders', hint: 'Every folder inside is one game. Separate several with a semicolon', kind: 'path', key: 'gameFolders', placeholder: 'Not set' },   // stage 5
  { id: 'steam', group: 'Sources', label: 'Steam library', kind: 'switch', key: 'steamEnabled', placeholder: 'Not found' },
  { id: 'epic', group: 'Sources', label: 'Epic library', kind: 'switch', key: 'epicEnabled', placeholder: 'Not found' },
  { id: 'calendar', group: 'Sources', label: 'Calendar', hint: 'Coming later', kind: 'info', key: 'calendarConnected' },
  { id: 'claude-monitor', group: 'Sources', label: 'Monitor Claude sessions', hint: 'Lets Rain show your sessions and answer their permission requests', kind: 'switch', key: 'monitorClaude' },   // stage 8

  { id: 'countdown', group: 'Game mode', label: 'Countdown', kind: 'select', key: 'gameModeCountdownSeconds' },

  { id: 'about', group: 'About', label: 'Rain', hint: 'A personal launcher for this PC', kind: 'info' },
]
```

Until stage 10 the three "Prototype" rows (`mock-scenario`, `mock-latency`, `mock-reset`) stay between "Game mode" and "About" (current lines 34-36); `selectChoices['mock-scenario']` stays (lines 59-64). The "Google Calendar" row becomes "Calendar" (stage 1b). `SettingDef` in `src/actions/types.ts` gets `placeholder?: string` (stage 3).

#### Step 1b (Settings becomes real; the mock keeps every other screen)

1. `src/data/types.ts`: delete `Settings` (lines 89-93); the barrel exports the generated `Settings`, `SettingsPatch`, `MonitorClaudeState` (foundation 8.8).
2. `src/data/api.ts`: line 90 `updateSettings(patch: SettingsPatch): Promise<Settings>`; add `SettingsPatch` to the import; add `settings: Settings` to `TopicPayload` (S6.3).
3. `src/data/mock/engine.ts` lines 145-152: add `health: new Set(),` and `settings: new Set(),` (the typed `listeners` needs one entry for every key of `TopicPayload`; stages 5 and 7 do the same for `projects`, `games` and `habits`). `src/data/mock/seed/settings.ts`: the five fields of S6.1.
4. `src/data/routing.ts`: `getSettings: 'real'`, `updateSettings: 'real'`, topic `settings: 'real'`.
5. `src/keys/summon.ts` line 17 becomes `useResource('settings', () => api.getSettings(), { live: 'settings' })`. `SettingsScreen.vue` line 39 the same. (One of them is always mounted, so the subscription lives for the page.)
6. `SettingsScreen.vue`:
   - line 20: `import type { Settings, SettingsPatch } from '@/data/types'`.
   - lines 72-80:

```ts
async function save(patch: SettingsPatch | null, label: string) {
  if (!patch) return
  try {
    await api.updateSettings(patch)
    refreshResources('settings')
    if ('vaultPath' in patch || 'dailyNoteFolder' in patch || 'dailyNoteFormat' in patch)
      refreshResources('vault', 'habits', 'habitDays', 'habitStats')   // stage 7: Life never shows data of the old vault (section 04 9.8)
    if ('dataDir' in patch) notice.info('Data folder changed. Rain is restarting its service.')   // stage 2
  } catch (reason) {
    notice.error(errorMessage(reason, `${label} was not saved`))
  }
}
```

   - lines 95-98 (`press`): the calendar branch goes; only the prototype button is left until stage 10:

```ts
function press(def: SettingDef) {
  if (def.id === 'mock-reset') mockControl.reset()
}
```

   and line 142 `@press="press(def)"`.
7. `settingDefs.ts` line 30: `{ id: 'calendar', group: 'Sources', label: 'Calendar', hint: 'Coming later', kind: 'info', key: 'calendarConnected' }`.
8. `settingValues.ts` becomes (the `default` branch keeps the two mock lines until stage 10):

```ts
export type SettingValue = string | boolean
export type BooleanSettingKey = 'hideOnBlur' | 'startWithWindows' | 'steamEnabled' | 'epicEnabled' | 'monitorClaude'

export const splitPaths = (text: string): string[] => text.split(';').map((part) => part.trim()).filter((part) => part !== '')
export const joinPaths = (paths: string[]): string => paths.join('; ')

export function readValue(def: SettingDef, settings: Settings): SettingValue {
  switch (def.key) {
    case 'hotkey': case 'dataDir': case 'vaultPath': case 'dailyNoteFormat': case 'dailyNoteFolder':
      return settings[def.key]
    case 'hideOnBlur': case 'startWithWindows': case 'steamEnabled': case 'epicEnabled': case 'monitorClaude':
      return settings[def.key]
    case 'calendarConnected':
      return settings.calendarConnected ? 'Connected' : 'Not connected'     // an info row shows text
    case 'resetAfterSeconds': case 'gameModeCountdownSeconds':
      return String(settings[def.key])
    case 'projectRoots': case 'gameFolders':
      return joinPaths(settings[def.key])
    default:
      /* until stage 10: the two mock rows; then only: */
      return def.id === 'about' ? `Version ${APP_VERSION}` : ''
  }
}

export function togglePatch(key: BooleanSettingKey, value: boolean): SettingsPatch {
  switch (key) {
    case 'hideOnBlur': return { hideOnBlur: value }
    case 'startWithWindows': return { startWithWindows: value }
    case 'steamEnabled': return { steamEnabled: value }
    case 'epicEnabled': return { epicEnabled: value }
    case 'monitorClaude': return { monitorClaude: value }
  }
}
```

   `patchFor` calls `togglePatch` for a boolean when `def.key` is one of the five keys (the `calendarConnected` case is deleted), and for text: `vaultPath`, `dailyNoteFormat`, `dailyNoteFolder`, `dataDir` send `value.trim()`; `projectRoots` and `gameFolders` send `splitPaths(value)`; the two select cases stay. `libraryPathPatch` returns `SettingsPatch | null`. In `buttonLabel` the calendar branch goes (line 97); until stage 10 it returns `'Reset'` for the prototype button.
9. `SettingRow.vue` line 154: `<Badge v-if="def.id === 'about'" ...>Mock data</Badge>` (so the calendar info row has none; stage 10 deletes the badge).
10. `src/features/settings/actions.ts`:
    - lines 20-28: `settingDefaults` adds `vaultPath: ''`, `dailyNoteFormat: ''` (changed), `dailyNoteFolder: ''`.
    - lines 43-52 (the switch branch of `primaryFor`): `{ [key]: !settings[key] }` no longer type-checks against `SettingsPatch` (a read-only key such as `calendarConnected` is a `keyof Settings`). The branch is guarded by `isBooleanKey(key)` (a type guard exported by `settingValues.ts`) and line 49 becomes `await api.updateSettings(togglePatch(key, !settings[key]))`.
    - line 78: `await api.updateSettings(resetPatch(key))` where `resetPatch(key: keyof typeof settingDefaults): SettingsPatch` is a `switch` over the nine keys of `settingDefaults` (`hideOnBlur`, `resetAfterSeconds`, `startWithWindows`, `vaultPath`, `dailyNoteFormat`, `dailyNoteFolder`, `steamEnabled`, `epicEnabled`, `gameModeCountdownSeconds`), each `case` returning `{ key: settingDefaults.key }`. No cast.
11. `src/data/rest/client.ts`: `getSettings` and `updateSettings` one-liners (foundation 8.3).

The Settings screen needs no change for the topic: `useResource` already replaces its data on a push.

#### Step 2 (lifecycle)

1. Row "Data folder" is added to `settingDefs.ts`; `readValue` and `patchFor` already have the `dataDir` cases. The `save` notice above becomes live.
2. "Start with Windows" works with no UI change. A refusal (`sandbox`, `noShell`, `registry`) is a notice with the backend text.
3. `src/keys/summon.ts`: nothing else.
4. `getHealth()` (route `GET /health`, owner core, foundation 14.4): `src/data/api.ts` gets `getHealth(): Promise<Health>` with the route comment, `src/data/rest/client.ts` the one-liner, `src/data/routing.ts` `getHealth: 'real'`. `getHealth` is a new method that the mock never had, so this step also CREATES `type RetiredMockMethod = 'getHealth'` and `export type MockApi = Omit<RainApi, RetiredMockMethod>` in `src/data/mock/index.ts` (foundation 8.6; the same commit changes the declared return type of `createMockApi()` to `MockApi`, and in `src/data/index.ts` the all-mock branch becomes `createMockApi() as RainApi` with the comment "retired methods are undefined here; their screens show an error row; stage 10 deletes the branch"; the router branch keeps `createRouterApi({ real, mock: createMockApi(), routing })` with `mock: MockApi`; the router types `mock` as `MockApi`; section 05 extends the union in stage 3). Without it `createMockApi()` stops type-checking in stage 2. `SettingsScreen.vue` loads it with `useResource('health', () => api.getHealth(), { live: 'health' })` and, when `configIssues` is not empty, shows one info line under the header: "<file> has an error on line N" (without a line: "<file> has an error"). No new row kind.

#### Step 3 (placeholder on top of section 05's glyph)

Section 05 (9.6) delivers `checkPaths`, `usePathCheck.ts` (argument `Ref<string>`, an empty field returns `undefined`; it imports `splitPaths` from `settingValues.ts`, where stage 1b defines it, and does not define or export one) and the first edit of `SettingRow.vue` (the `mockControl` import goes, `pathFound = usePathCheck(draft)`, the glyph is drawn only when `pathFound !== undefined`). This section builds only the rest of the row change:

1. `SettingRow.vue`: for a list row (`projectRoots`, `gameFolders`) the field text given to `usePathCheck` is the whole draft (section 05 splits it on `;`); an empty field has no glyph, because `usePathCheck` answers `undefined` for it, and the `Input` shows the placeholder (`:placeholder="def.placeholder"`, "Not set" or "Not found"). A typed folder that does not exist shows "Missing". This is a deliberate change of the contract text "an empty field never exists" so that a first run and an absent Epic launcher do not look like errors.
2. `src/actions/types.ts`: `SettingDef` gets `placeholder?: string`.

#### Step 5, 7, 8 (rows)

- Stage 5: the row `game-folders` is added; `patchFor` and `readValue` already handle `gameFolders`.
- Stage 7: the row `note-folder` is added.
- Stage 8: the row `claude-monitor` is added with its state. `SettingRow.vue` gets one optional prop `rowState?: { status: StatusKind; text: string }`: when present the hint line shows `rowState.text` instead of `def.hint`, and a `StatusDot` (with the text as its label) is drawn before the switch. `SettingsScreen.vue` computes it and passes `:row-state="stateOf(def, settings)"`:

```ts
function stateOf(def: SettingDef, current: Settings): { status: StatusKind; text: string } | undefined {
  if (def.key !== 'monitorClaude') return undefined
  switch (current.monitorClaudeState) {
    case 'on': return { status: 'success', text: 'Active' }
    case 'onInactive': return { status: 'idle', text: 'Installed. Waiting for a session to start' }
    case 'needsRepair': return { status: 'failed', text: 'Needs repair. Press Ctrl+Enter' }
    default: return undefined                    // off: the plain hint
  }
}
```

  `settingActions` (`settings/actions.ts` after line 83) adds for that row one secondary action: `{ id: 'repair', label: 'Repair Claude hooks', icon: 'rotate-ccw', slot: 'secondary', run: async ({ api, notice }) => { await api.updateSettings({ monitorClaude: true }); refreshResources('settings'); notice.success('Claude monitoring repaired') } }`. It is idempotent, so it is offered in every state.

#### Key bindings

None change. Enter toggles a switch, opens a select or focuses an input (existing). Ctrl+Enter resets, or repairs on the Claude row. The hotkey row stays read-only.

#### What the user sees

Settings opens at once from the cache and refreshes in the background. Path glyphs appear 250 ms after typing. An empty path row shows a quiet placeholder and no glyph. "Calendar: Not connected" is plain text. "Monitor Claude sessions" shows one small dot and a short line only when it is on. Nothing else is added to the chrome.

### 9.3 Routing-table entries of the migration router

| Stage | `src/data/routing.ts` |
|---|---|
| 1b | `methods.getSettings: 'real'`, `methods.updateSettings: 'real'`, `topics.settings: 'real'` |
| 2 | `methods.getHealth: 'real'` (topic `health` is already real since 1b) |
| 3 | none of this section (`checkPaths` is routed by section 05) |
| 9 | `methods.getGameMode`, `startGameMode`, `resolveGameMode`, `markGameModeRecapSeen: 'real'`; `topics.gameMode: 'real'` |
| 10 | the file is deleted |

The mock keeps its own settings (other mock slices read them) until stage 10; real and mock settings may differ meanwhile (foundation 8.6).

### 9.4 Shell, scripts and tooling files of this section

| File | Change | Stage |
|---|---|---|
| `src-tauri/src/lib.rs` | read `--hidden` (`std::env::args().any(\|a\| a == "--hidden")`) and skip `summon` in `setup` (lines 92-94); the single-instance callback (lines 63-67) takes `args`: if they contain `--hidden` return without summoning; else call `backend::on_second_launch(app)` (aborts a pending game mode exit task, `ensure_backend()`, retries the hotkey registration, reports the facts) and then `summon` | 1b (`--hidden`), 2 (rest) |
| `src-tauri/src/relay.rs` | the exit task of 6.8 on the `lifecycle` frame with reason `gameMode` | 9 |
| `src-tauri/src/facts.rs` | the report after the retry of the hotkey | 2 |
| `vite.config.ts` | `define: { __APP_VERSION__: JSON.stringify(pkg.version) }` | 10 |
| `src/vite-env.d.ts` | `declare const __APP_VERSION__: string` | 10 |
| `scripts/check.mjs` | fail when the versions of `package.json`, `Cargo.toml` (workspace) and `tauri.conf.json` differ | 10 |
| `scripts/dev.mjs`, `scripts/backend.mjs` | `--fixture <name>` (foundation 12.3) and the npm script `dev:fixture` are built in stage 1b (foundation 14.1); stage 10 only removes `dev:ui` and `VITE_RAIN_MOCK` | 1b, 10 |

### 9.5 Game mode, step 9

Files and exact changes (lines of the current tree):

`src/features/game-mode/GameModeScreen.vue`
- line 20: delete `import { mockControl } ...`.
- lines 93-101 (`choose`): the action awaits, so a rejection becomes a notice through `runAction`:

```ts
return { id, label, icon, slot, run: async ({ api: rainApi }) => { await rainApi.resolveGameMode(choice) } }
```

- lines 103-112: delete `restart`. Lines 125-126: delete `case 'done': return [restart]` (the `default` returns `[]`, so `done` shows no button).
- the off hint (line 200): `hint="It stops Rain's jobs and closes the Rain window. Your Claude sessions keep running."`.
- the review sub-line (lines 225-227) and a new done hint:

```ts
const leftRunning = computed(() => state.value?.items.find((item) => item.effect === 'leftRunning')?.count ?? 0)
const doneHint = computed(() =>
  leftRunning.value > 0 ? 'Your Claude sessions are still running. Open Rain again from the Start menu.' : 'Open Rain again from the Start menu.')
const reviewHint = computed(() => {
  if (state.value?.waiting) return 'Rain goes on as soon as they are done.'
  return state.value?.waitTimedOut ? 'They did not finish in 3 minutes. Stopping them now loses unfinished work.' : 'Stopping them now loses unfinished work.'
})
```

  and the template uses `{{ doneHint }}` and `{{ reviewHint }}`.
- after the "Then launch" paragraph (line 262), still inside `v-else`: `<p v-if="state.launchError" class="text-center text-13 text-status-failed">The game did not start: {{ state.launchError }}</p>`. It is shown in `windingDown` and `done`.
- nothing else: the screen still holds no timer and draws the pushed state.

`src/features/game-mode/WindDownRow.vue`
- lines 24-27: `progress` returns `null` when `props.item.effect === 'leftRunning'` (a left-running row has no dot).
- lines 35-41: the badge text comes from `{ safe: 'safe', interrupts: 'interrupts', leftRunning: 'left running' }` and its class `w-20` becomes `w-24`.

`src/keys/keymap.ts` lines 114-123 (Esc on the Game mode route; today a rejected call does nothing and `nav.back()` is skipped):

```ts
async function cancelGameMode() {
  const { api, notice } = actionContext()
  try {
    const { phase } = await api.getGameMode()
    if (phase === 'windingDown' || phase === 'done') return
    if (phase !== 'off') {
      await api.resolveGameMode('cancel')
      notice.info('Game mode cancelled')
    }
  } catch (reason) {
    notice.error(errorMessage(reason, 'Game mode was not cancelled'))
    // A backend that is away has nothing to cancel: leave the screen. A refusal (conflict) keeps it.
    if (typeof reason === 'object' && reason !== null && 'code' in reason && reason.code === 'conflict') return
  }
  nav.back()
}
```

(`errorMessage` is imported from `@/lib/errors`.)

`src/keys/summon.ts` (the reset must not cancel game mode, 6.10):

```ts
const mode = useResource('gameMode', () => api.getGameMode(), { live: 'gameMode' })
/** While game mode runs, going Home would unmount its screen and cancel it. */
const gameModeRunning = () => ['review', 'countdown', 'windingDown'].includes(mode.data.value?.phase ?? 'off')

function resetToHome() {
  if (gameModeRunning()) return
  nav.go('home')
  resetViewState()
  resetResources()
}
```

`src/components/rain/AppShell.vue`
- line 3 comment: "It also starts the keymap and the recap notice of the last game mode."
- line 4: drop `watch` from the `vue` import (nothing else in the file uses it); line 8: delete the `mockControl` import; line 40: replace `watch(hidden, ...)` by `useGameModeRecap()` (import from `@/composables/useGameModeRecap`).

New `src/composables/useGameModeRecap.ts`:

```ts
// The first load after game mode shows what it did, once (6.7.7).
import { watch } from 'vue'
import { api } from '@/data'
import { recapText } from '@/features/game-mode/recap'
import { shell } from '@/shell'
import { useNotice } from './useNotice'
import { useResource } from './useResource'

export function useGameModeRecap(): void {
  const mode = useResource('gameMode', () => api.getGameMode(), { live: 'gameMode' })
  const notice = useNotice()
  let shownFor = ''
  watch([() => mode.data.value?.last, shell.hidden], ([last, hidden]) => {
    if (!last || last.seen || hidden || last.at === shownFor) return
    shownFor = last.at                                    // the mark call below is not instant
    notice.info(recapText(last))
    void api.markGameModeRecapSeen().catch(() => {})
  }, { immediate: true })
}
```

New `src/features/game-mode/recap.ts`:

```ts
import type { GameModeRecap } from '@/data/types'

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`

/** One line for the notice, at most about 70 characters. */
export function recapText(recap: GameModeRecap): string {
  if (!recap.complete) return 'Game mode did not finish.'
  const sum = (outcomes: string[]) => recap.items
    .filter((item) => item.kind !== 'session' && outcomes.includes(item.outcome))
    .reduce((total, item) => total + item.count, 0)
  const stopped = sum(['stopped', 'forced'])
  const left = recap.items.filter((item) => item.outcome === 'leftRunning').reduce((total, item) => total + item.count, 0)
  const parts = [stopped > 0 ? `Game mode stopped ${plural(stopped, 'job', 'jobs')}.` : 'Game mode closed Rain.']
  if (left > 0) parts.push(`${plural(left, 'Claude session', 'Claude sessions')} kept running.`)
  if (recap.launchError) parts.push('The game did not start.')
  return parts.join(' ')
}
```

`src/data/rest/client.ts` and `src/data/rest/stream.ts` (foundation 8.3, 8.7):

```ts
// client.ts
getGameMode: async () => stream.note('gameMode', await call<GameModeState>('GET', '/game-mode')),
startGameMode: async (opts) => stream.deliver('gameMode', await call<GameModeState>('POST', '/game-mode', opts ?? {})),
resolveGameMode: async (choice) => stream.deliver('gameMode', await call<GameModeState>('POST', '/game-mode/choice', { choice })),
markGameModeRecapSeen: () => call<void>('POST', '/game-mode/recap/seen'),
```

`stream.deliver(topic, payload)` gives the payload to the subscribers of the topic exactly as a frame would, unless it is a `gameMode` state whose `rev` is not above the highest applied since the last `hello`; it returns the payload either way. `stream.note` only raises the mark. A new `startId` clears the mark.

`mock` (stage 9): delete `src/data/mock/slices/gameMode.ts` and `src/data/mock/seed/gameMode.ts`; edit `src/data/mock/index.ts` (the `gameModeSlice` import and spread), `src/data/mock/store.ts` (the `gameMode` field and its seed import), `src/data/mock/seed/settings.ts` lines 6 and 23 (`DEFAULT_COUNTDOWN_SECONDS` becomes the literal `10`), and the `finishWorkingSessions`, `finishRunningJobs` and `markGamePlayed` exports if nothing else uses them. The engine's `gameMode` listener set stays until stage 10 (it is part of the typed `listeners`). The same commit adds `getGameMode`, `startGameMode`, `resolveGameMode` and `markGameModeRecapSeen` to `RetiredMockMethod` (foundation 8.6; the first three are implemented by the deleted `gameMode.ts`, the fourth is new and the mock never had it). No stub is left behind.

Key bindings: none change (Enter, Ctrl+Enter, Esc as in spec 7.8). The Ctrl+G route and the command "Start game mode" are unchanged.

### 9.6 Replacement text for `docs/ui-spec.md` (written here; this section does not edit the spec)

The spec is edited in the stage named in each heading.

**Header (line 3) and stack (line 12), stage 10.** Replace "Status: final for the UI-only prototype. Date: 2026-11-21." with "Status: the frontend design contract. The UI was first built as a prototype on mock data (2026-11-21); the data layer is now the real backend (`docs/backend/`)." Replace "No backend. Every screen reads a typed mock layer." with "The backend is a separate Rust process (`docs/backend/00-foundation.md`). Every screen reads it through `RainApi`."

**Decision 15 (line 32), stage 10.** Replace by: "15. **One data interface.** Every screen reads `RainApi`, which maps 1:1 to the backend's REST routes. States such as empty or failing are produced by starting the backend on a fixture data folder or by stopping it, not by a switch in the app."

**2.2, last paragraph (line 93), stage 1b.** Replace by: "If registration fails (another app owns the chord), the shell reports it to the backend, Settings shows 'Another app already holds this chord' on the Summon hotkey row, and Rain still starts. Launching Rain a second time (Start menu) tries the registration again."

**2.6, first bullet (line 124), stage 9.** Replace by: "`<html data-hidden>` is set. CSS pauses the spinner; `useNow` and every interval of the page stop. The backend keeps running, and so does game mode: hiding the window never cancels it."

**2.3 exception and 2.4 (lines 100-110).** Unchanged.

**7.8 Game mode, stage 9.** Replace the section by:

> ### 7.8 Game mode (`features/game-mode/GameModeScreen.vue`)
>
> Takes the content area; a centred 480 px column. Ctrl+G (or the rail icon, or "Play in game mode") goes here and calls `api.startGameMode()`. The search field is disabled while the phase is `review`, `countdown` or `windingDown`. Hide on blur is held. Leaving the screen with Ctrl+digit cancels, with the notice "Game mode cancelled". Going Home by the 'return to Home' timer never cancels game mode (it does nothing while game mode runs). Shift+Esc hides the window and game mode goes on.
>
> State is `GameModeState` from `subscribe('gameMode')`; the screen holds no timer of its own. What game mode stops is real: Rain's scheduled jobs and idle jobs that are running, then the background service and the Rain window. The user's open Claude sessions are never stopped; they are listed as "left running".
>
> | Phase | Shows | Enter | Ctrl+Enter | Esc |
> |---|---|---|---|---|
> | `review` (only when at least one item has `effect: 'interrupts'`) | `circle-power` 32, "2 things will be interrupted" (20 px), the checklist, buttons | Wait for them | Stop anyway | Cancel |
> | `review` with `waiting: true` | same, heading "Waiting for 2 things to finish", spinner on those rows; after 3 minutes it returns to `review` with the line "They did not finish in 3 minutes." | (none) | Stop anyway | Cancel |
> | `countdown` | "Closing in" + number (`text-countdown`) + "s", a draining bar, the checklist | Close now | (none) | Cancel |
> | `windingDown` | "Winding down"; each row goes pending, stopping (`StatusDot running`), stopped (`StatusDot success`); a left-running row has no dot | (none) | (none) | (none) |
> | `done` | "Rain is off" (20 px) and one line: "Open Rain again from the Start menu." (with "Your Claude sessions are still running." first when some are open); no button. The window closes about 3 seconds after the service has stopped. | (none) | (none) | (none) |
>
> - Checklist row (40 px): icon, label, count, `Badge` "safe" (muted), "interrupts" (`--status-waiting` tint) or "left running" (muted), `StatusDot` during wind-down. Items: Claude sessions, scheduled jobs, idle jobs, background service, Rain window.
> - With nothing to interrupt, `review` is skipped and the countdown starts at once. `review` that nobody answers for 10 minutes cancels itself.
> - Started from a game: a last line "Then launch <title>" with the cover at 24 px. If the game could not be started, a line "The game did not start: <reason>" appears under it.
> - On the first load after game mode, one notice tells what it did ("Game mode stopped 2 jobs. 3 Claude sessions kept running.").
> - Buttons are real `Button`s with their `KeyHint`; the screen passes them to the action bar through `primaryOverride`.
> - Countdown length comes from `settings.gameModeCountdownSeconds` (default 10).

**7.9 Settings, stages 1b to 10.** Replace the table and the paragraph under it by:

> | Group | Row | Control |
> |---|---|---|
> | General | Summon hotkey | `KeyHint` Ctrl ` (read-only), hint "Takes this chord from other apps while Rain runs"; a failed dot when another app holds the chord |
> | General | Hide when focus is lost | `Switch` |
> | General | Return to Home after | `Select`: 30 s, 60 s, 5 min |
> | General | Start with Windows | `Switch` (off by default; on starts Rain hidden at logon) |
> | General | Data folder | path `Input` + glyph; changing it restarts the background service |
> | Sources | Obsidian vault | path `Input` + glyph (follows Obsidian when it has one vault) |
> | Sources | Daily notes folder | `Input`, folder inside the vault (follows Obsidian until changed) |
> | Sources | Daily note format | `Input` (follows Obsidian until changed) |
> | Sources | Project folders | path `Input` (semicolon-separated) + glyph |
> | Sources | Game folders | path `Input` (semicolon-separated) + glyph |
> | Sources | Steam library | `Switch` + path `Input` + glyph |
> | Sources | Epic library | `Switch` + path `Input` + glyph |
> | Sources | Calendar | text "Not connected" (no connect flow yet) |
> | Sources | Monitor Claude sessions | `Switch`; a status dot and line when on; Ctrl+Enter repairs |
> | Game mode | Countdown | `Select`: 5, 10, 20, 30 s |
> | About | Rain | version text |
>
> Group headers are `SectionHeader`s with a `Separator` above. The glyph is found (`circle-check`) or missing (`circle-x`); it is drawn 250 ms after typing, from the backend, and an empty field has no glyph (its placeholder says "Not set" or "Not found"). `SettingDef { id: string; group: string; label: string; hint?: string; placeholder?: string; kind: 'switch' | 'select' | 'path' | 'text' | 'info'; key?: keyof Settings }`.

**Section 8, stage 10.** Replace the whole section (lines 954-1209) by:

> ## 8. Data layer
>
> The app reads one interface, `RainApi` in `src/data/api.ts`, implemented by `src/data/rest/` over the bridge of the shell (Tauri command or Vite proxy). Types are generated from the Rust crate `rain-types` into `src/bindings/rain-types.ts`; `src/data/types.ts` re-exports them. The routes, bodies, errors and topics are defined in `docs/backend/00-foundation.md` (sections 4 to 9) and in the feature sections `01` to `06`. There is no sample data in the app. To see a state, start the backend on a fixture data folder (`npm run dev:fixture`) or stop it (`npm run backend:stop`).

**Section 9.1 to 9.4, stage 10.** Mark "Historical: the build plan of the prototype" and delete every mention of `data/mock/` in the tree and in the table of 9.3 (lines 1265, 1281 to 1287).

**Section 10, stage 10.** Replace the first bullet by: "Features that are not built yet are listed in `docs/backend/`." and delete the bullets that name things now built (real app discovery, Steam and Epic scanning, Claude hooks, cron, idle detection, Obsidian parsing, disk scanning, real game mode, autostart).

### 9.7 Stage 10: the deletion list

Done in one commit after stage 9, when every `routing.ts` entry is `real`.

| What | Action |
|---|---|
| `src/data/mock/` (`control.ts`, `covers.ts`, `engine.ts`, `index.ts`, `logs.ts`, `options.ts`, `search.ts`, `store.ts`, `seed/*`, `slices/*`) | delete the folder (the stages 3 to 9 already removed parts of it) |
| `src/data/router.ts`, `src/data/routing.ts` and their vitest files | delete |
| `src/data/index.ts` | becomes `export const api: RainApi = createRestApi(shell.bridge, stream)` plus `export const onStreamReset = stream.onReset`; the `VITE_RAIN_MOCK` branch and its imports go |
| `src/data/types.ts` | only `export * from '../bindings/rain-types'`, `ID`, `ISODate`, `ISODateTime`; every hand-written interface left is deleted; header comment lines 1-2 rewritten |
| `src/data/api.ts` | header comment lines 1-3 rewritten ("maps to one REST call"); `RainError` is `ApiError` (foundation 15 F1) |
| `src/vite-env.d.ts`, `package.json` | the `VITE_RAIN_MOCK` declaration and the script `dev:ui` go (`dev:fixture` exists since stage 1b, 9.4) |
| `src/main.ts` | keep the `onStreamReset` callback of section 05 9.7 (`resetLaunches()` and `resetDisk()` when `restarted`, else `reconcileDisk()`, then `refreshResources()`); delete only the mock reseed imports and `mockControl.onReseed(...)` (search for them; the line numbers are stale). Do not delete the disk reconcile |
| `src/actions/commands.ts` | delete the import (line 5), the type `Scenario` (line 47), `SCENARIOS` (lines 49-54), `scenarioCommands` (lines 56-65), the spread `...scenarioCommands` (line 135), the command `reset-mock` (`simulate-permission` is already deleted in stage 8 by section 03; find every line again by search, the numbers here are stale); header comment lines 3-4: delete "and "busy" finds the mock scenario" |
| `src/features/settings/settingDefs.ts` | delete the "Prototype" group (lines 34-36), `selectChoices['mock-scenario']` (lines 59-64); `APP_VERSION` from `__APP_VERSION__` |
| `src/features/settings/settingValues.ts` | delete the import (line 5) and the two mock lines of `readValue` (lines 30-31); header comment lines 1-3 and the doc comment of `readValue` ("Prototype rows ...") rewritten; `buttonLabel` is deleted |
| `src/features/settings/SettingsScreen.vue` | delete the import (line 19), the two mock branches of `change` (lines 83-90, which leaves `void save(patchFor(def, value), def.label)`), `press` and `@press` |
| `src/features/settings/SettingRow.vue` | delete the `Badge` (line 154) and its import, the `Button` import, the `press` emit, the `kind === 'button'` branches (lines 85 and 145-147), `buttonLabel` import |
| `src/actions/types.ts` | `SettingDef.kind` loses `'button'`; `src/features/settings/actions.ts` loses `button: 'Run'` in `CHANGE_LABEL` |
| `src/composables/useResource.ts` | reword the comments at lines 57 and 151 ("A mock may hand back ...", "scenario change and Reset mock data") |
| `src/features/game-mode/GameModeScreen.vue`, `src/components/rain/AppShell.vue` | already done in stage 9; verified here |
| `src/features/games/actions.ts` (line 38), `src/features/projects/actions.ts` (lines 59, 67), `src/features/workflows/actions.ts` (line 140), `src/features/games/CustomGameDialog.vue` (line 2) | the comments "the prototype only reports it" and "the mock only stores it" go with the real calls built in stages 4 to 8 (sections 02 and 03); verified here |
| `docs/ui-spec.md` | the edits of 9.6 |
| `localStorage 'rain.prototype'` | not touched: a leftover key in an old browser is harmless |

### 9.8 The final check that no mock code is left

`scripts/check.mjs` gets a step `no-mock` (from stage 10 on) that fails on any of these. The commands are written with `rg`; the script does the same with Node.

| # | Check | Expected |
|---|---|---|
| 1 | `rg -n "data/mock\|mockControl\|createMockApi\|VITE_RAIN_MOCK\|rain\.prototype\|setPaused\|onReseed\|pathExists\|simulatePermission" src scripts package.json vite.config.ts` | no output |
| 2 | `rg -n "Mock data\|Mock scenario\|Simulated latency\|Reset mock\|Restart prototype\|scenario-\|reset-mock\|simulate-permission" src` | no output |
| 3 | `rg -n -i "\bprototype\b" src` | no output (a JavaScript `.prototype` property would be listed and judged by hand) |
| 4 | the paths `src/data/mock`, `src/data/router.ts`, `src/data/routing.ts` | do not exist |
| 5 | `src/data/types.ts` | contains no `interface` and no `type X =` except `ID`, `ISODate`, `ISODateTime` (the foundation's duplicate-name check is also run) |
| 6 | `npm run build`, then `rg -l "Mock scenario\|Simulated latency\|mulberry32" dist` | no file |
| 7 | `docs/ui-spec.md`: sections 7.8 and 7.9 hold the text of 9.6; section 8 is the "Data layer" text | by hand |
| 8 | `npm run check` | all steps pass |

### 9.9 Stage summary

| Stage | Files of this section changed |
|---|---|
| 1a | none in the frontend or the shell |
| 1b | `types.ts`, `api.ts`, `mock/engine.ts`, `mock/seed/settings.ts`, `routing.ts`, `summon.ts`, `SettingsScreen.vue`, `settingDefs.ts`, `settingValues.ts`, `SettingRow.vue`, `settings/actions.ts`, `rest/client.ts`, `src-tauri/src/lib.rs` (`--hidden`) |
| 2 | `settingDefs.ts` (Data folder row), `lib.rs`, `facts.rs` |
| 3 | `SettingRow.vue` (`placeholder`), `actions/types.ts` |
| 5, 7, 8 | `settingDefs.ts` rows; 8 also `SettingRow.vue`, `SettingsScreen.vue`, `settings/actions.ts` |
| 9 | `GameModeScreen.vue`, `WindDownRow.vue`, `keymap.ts`, `summon.ts`, `AppShell.vue`, `useGameModeRecap.ts`, `recap.ts`, `rest/client.ts`, `rest/stream.ts`, `routing.ts`, the mock files of 9.5, `relay.rs` |
| 10 | the table of 9.7 |

---------------------------------------------------------------------------------------------------

## 10. Tests

### 10.1 Traits and fakes (foundation 12.2)

| Trait | Production | Fake in `tests/` | Used by |
|---|---|---|---|
| `Detect` (vault, daily-note settings, Steam, Epic) | files and registry | `FakeDetect` with fields set by the test | settings |
| `RunKey` | `winreg` on `HKCU` | `FakeRunKey` (an in-memory `RunState`, can fail on demand) | lifecycle |
| `WindDownPort` (jobs, idle jobs) | section 03 | `FakeWindDown`: a settable running count, a `watch`, a recorded list of calls, a configurable stop result and delay | gamemode |
| sessions counter | section 03 | `FakeSessions { open: u32 }`; it offers no stop method at all | gamemode |
| `games().get`, `launch_after_game_mode` | section 02 | `FakeGames`: known ids, installed flag, a start that succeeds, fails or hangs; records the call order | gamemode |
| `Clock` and `tokio::time::pause` | system | the controllable clock (foundation 11.4) | gamemode, settings |
| shell restart | the real shell | `TestBackend::restart()`: stops the in-process backend and starts another on the same data and local folders | settings, gamemode |

### 10.2 Unit tests (next to the code)

| Area | Tests (each names what it proves) |
|---|---|
| `rain-types` | JSON round trip of a full and a minimal `Settings`, `SettingsPatch`, `GameModeState`, `GameModeRecap`; `assert_no_nulls`; `resetAfterSeconds: 45` and `gameModeCountdownSeconds: 15` are rejected; an unknown field in `SettingsPatch` is rejected; `rev` at 2^53 - 1 serialises as a number |
| `settings::store` | set and remove keys keep every comment, the key order and CRLF; the first write creates the header above `schema_version`; paths are written as literal strings and read back equal; a path with `'` is written as a basic string and read back equal; clearing a key removes only that key; a broken file refuses `edit` |
| `settings::validate` | one case for each row of table V, and the boundary values (32 and 33 entries, 1024 and 1025 characters, a control character, `..` in the folder, a drive letter in the folder, `/` allowed in the format) |
| `Resolved` building (`Service::expand_env`, then `core::paths::normalize(.., true)`) | `%NAME%` expanded, an unknown variable left, `/` to `\`, trailing `\` removed, a drive root kept, a UNC path kept, a relative path dropped; `"C:\Fake\Library"` (with quotes) kept as `C:\Fake\Library`; `C:\a\..\b`, `C:foo` and `\\?\C:\x` dropped from `Resolved` and kept in the API value (the glyph says Missing for the same three, section 05) |
| `settings::detect` | `obsidian.json`: one vault gives that vault, two vaults and no override give no vault, an override beats both, a missing folder skipped, bad JSON, no file; `daily-notes.json`: only `folder`, both keys, an absolute folder dropped, a `..` folder dropped; Steam: forward slashes and a lowercase drive normalised, the HKLM fallback, a missing folder; Epic: present and absent |
| `settings::resolved` | override beats detection beats default for each field; clearing an override brings detection back; a relative project root is dropped from `Resolved` but stays in the API value |
| `lifecycle::autostart` | effective state for every combination of value and `StartupApproved`; the first-byte rule (`02`, `06` enabled, `03` disabled, missing, wrong type); the command is `"<exe>" --hidden`; an exe path with `"` is refused; self-heal rewrites a differing value and writes nothing when the value is absent; sandbox refuses |
| `settings::datadir` | each validation (file, nested both ways, inside the local folder, not writable via a read-only attribute probe); copy-when-empty copies the known files and `media`, never overwrites, never deletes; a target with one Rain file is re-pointed without copying; `machine.toml` written and read back; the default (empty string) removes the key |
| `config` | `RAIN_DATA_DIR` beats `machine.toml`; an unusable `data_dir` falls back to the default and records a `ConfigIssue`; a `machine.toml` with a higher version is ignored |
| `gamemode::machine` | every row of the transition table and every cell of the choice table of 6.7.2; `rev` rises on every change and only then; the countdown seconds from a controlled `Instant` (including the first and the last second and a deadline already past); the wait limit; `REVIEW_ABANDON`; the auto-advance when the interrupting work ends in `review`; `startGameMode` while running returns the same run |
| `gamemode::items` | counts and effects for each combination of running work; the session row is `leftRunning` and never counted in the heading's number |
| `gamemode::recap` | job runs aggregated per kind with at most 5 names; sessions never carry names; `complete: false` for an `incomplete` record |
| `gamemode::store` | `begin_record`, `finish_record`, `seen`; a `running` record becomes `incomplete` at start; the 21st record deletes the oldest; `ON DELETE CASCADE` removes its items |

### 10.3 Integration tests (harness, fixtures)

Fixtures (foundation 12.2; all fake, no path of anyone; `{DATA}` and `{TEMP}` are replaced by the harness when it copies):

| Fixture | Content for this section |
|---|---|
| `tests/fixtures/empty/data/` | only `.gitkeep` |
| `tests/fixtures/typical/data/settings.toml` | see below; plus the folders `vault/`, `projects-root/`, `games-root/` (each with a `.gitkeep`) |
| `tests/fixtures/broken/data/settings.toml` | `schema_version = 1`, a blank line, then `reset_after_seconds = = 60` (a syntax error on line 3) |
| `tests/fixtures/detect/obsidian-one.json`, `obsidian-two.json`, `obsidian-bad.json`, `daily-notes-folder-only.json`, `daily-notes-both.json` | inputs of the detection tests |
| `tests/fixtures/typical/data/jobs.toml` | owned by section 03; it must hold one job named "Demo wait" whose command runs for about 90 seconds (for example `ping -n 90 127.0.0.1`) so that game mode can show `review` (open point O1) |

```toml
# tests/fixtures/typical/data/settings.toml
schema_version = 1
hide_on_blur = true
reset_after_seconds = 60       # a trailing comment that must survive a patch
vault_path = '{DATA}\vault'
project_roots = ['{DATA}\projects-root']
game_folders = ['{DATA}\games-root']
game_mode_countdown_seconds = 5
monitor_claude = false
```

`tests/settings.rs` (stage 1b unless marked):

| Test | Proves |
|---|---|
| `defaults_on_empty_fixture` | `GET /settings` equals the defaults of 4.1; `hotkey` is `Ctrl+Backquote`; `calendarConnected` is `false`; `hotkeyAvailable` is `true` while no shell reported; the data folder gets no file |
| `typical_fixture_values` | the fixture's values come back as typed |
| `patch_roundtrip_keeps_comments` | a patch changes one key and the file differs from the original only on that line |
| `patch_creates_file_with_header_on_first_write` | the first write on `empty` creates the header and `schema_version` |
| `patch_rejects_unknown_and_read_only_fields` | `hotkey`, `hotkeyAvailable`, `calendarConnected`, `monitorClaudeState`, a made-up key: each `400 invalid` `badBody` |
| `patch_rejects_values_outside_the_unions` | 45 and 15: `400`; the message starts "The request was not valid" |
| `patch_external_effect_field_must_be_alone` | `{ startWithWindows: true, hideOnBlur: false }`: `400 onePerRequest`; nothing written |
| `patch_empty_object_is_a_noop` | `{}`: 200, no write, no topic frame |
| `patch_clear_override_brings_detection_back` (stage 2) | with `FakeDetect`: set `vaultPath`, then `""`: the detected vault returns and the key is gone from the file |
| `patch_not_built_before_stage` | `startWithWindows`, `dataDir`, `monitorClaude: true` answer `503 notBuilt` until their stage (this test is edited by that stage) |
| `broken_fixture_keeps_defaults_and_blocks_patch` | `GET` works with defaults; `GET /health` lists the issue with line 3; a `PATCH` is `409 configBroken` and the file is unchanged |
| `topic_settings_published_on_patch` | one frame with the new value; `resync` on connect |
| `hand_edit_reaches_topic_settings` (stage 2) | edit the file with the watcher on: a `settings` frame within 1 s |
| `shell_facts_set_hotkey_available` | stage 1b: `PUT /internal/shell` with `hotkeyRegistered: false` gives `hotkeyAvailable: false` and a `settings` frame (published by the settings module, once; a second identical `PUT` publishes none). Stage 2 adds the assertion that the `hotkey` source is `failed` in `GET /health` (it needs `lifecycle`, which does not exist before stage 2) |
| `start_with_windows_*` (stage 2) | with `FakeRunKey`: on writes the exact command, off removes it, a disabled-by-Windows value reads as off, on clears the disabled mark; sandbox `409 sandbox`; no shell facts `409 noShell`; a failing fake `500 registry` |
| `datadir_*` (stage 2) | environment set: `409 environment`; nested: `400`; busy (a fake activity gauge): `409 busy`; success: `200`, `machine.toml` written, the stream ends with `lifecycle { restart }`, `TestBackend::restart()` reads the new folder; copy-when-empty; re-point when the target has Rain files |

`tests/gamemode.rs` (stage 9; fakes of 10.1, paused clock):

| Test | Proves |
|---|---|
| `start_with_nothing_running_goes_to_countdown` | phase `countdown`, `countdownSeconds` from Settings, `game_mode_active` set, `pause(true)` called on both fakes |
| `start_with_a_running_job_goes_to_review` | phase `review`, the job item `interrupts`, the sessions item `leftRunning` |
| `wait_ends_when_the_run_ends` | `wait`, then the fake run ends: `countdown` without a user action |
| `wait_times_out_after_180_seconds` | `review`, `waiting: false`, `waitTimedOut: true` |
| `review_abandon_cancels_after_10_minutes` | phase `off`, work resumed |
| `stop_anyway_then_close_now_runs_the_stops_in_order` | the fakes record: jobs stop, idle stop, game start, then the service rows; the sessions fake received no call of any kind |
| `done_is_the_last_topic_frame` | read the stream: the last frame with a topic is `gameMode` with phase `done`; then one `lifecycle { stopping, gameMode }`; then the stream ends; a publish attempted after `done` is dropped |
| `cancel_resumes_work_and_clears_the_flag` | `game_mode_active` is false BEFORE `pause(false)` is called (a fake `WindDownPort` records the flag value at the call), then `pause(false)`, `thenLaunchGameId` gone, `publish_final` last |
| `cancel_after_the_wind_down_began_is_409` | `409 windingDown` |
| `choices_are_idempotent_by_the_table` | every cell of the choice table through the route |
| `unknown_game_404_and_not_installed_409` | no state change, no pause |
| `launch_failure_sets_launch_error_and_still_exits` | `launchError` on the state, the record has `launch_result = failed`, the stream ends |
| `stop_over_budget_is_recorded_failed_and_the_wind_down_continues` | the hanging fake exceeds 5 s of paused time; the row still reaches `stopped`; the record item is `failed` |
| `rev_only_grows` | across a full run, strictly increasing |
| `record_and_recap_after_restart` | after `done`, `TestBackend::restart()`: `GET /game-mode` in `off` has `last` with `seen: false`; `POST /game-mode/recap/seen`; the next `GET` has `seen: true` |
| `a_running_record_becomes_incomplete` | insert a `running` row, start: `last.complete` is `false` |
| `jobs_and_idle_start_nothing_while_active` | the fakes' pause flags are true from `start` to `cancel` |

`tests/source_rules.rs` gets two lines for this section: files of `modules/gamemode/` must not contain `stop_session`, `TerminateProcess`, `taskkill` or `.kill(` (the only way game mode ends anything is `WindDownPort`), and `modules/settings/` and `modules/lifecycle/` must not contain `.claude` or `settings.json` outside the permitted words of foundation 13.3 (they call the sessions module and never name the file).

`tests/process_smoke.rs` gets `gamemode_ends_the_process` (stage 9): start the real `rain-backend.exe` with `RAIN_PIPE`, `RAIN_DATA_DIR` (a copy of `empty`) and `RAIN_LOCAL_DIR`; `PATCH` the countdown to 5; `POST /game-mode`; read the stream until `lifecycle`; wait for the process to exit with code 0; the pipe is gone. About 8 seconds.

### 10.4 Frontend tests (vitest; foundation 12.3)

| File | Tests |
|---|---|
| `src/features/settings/settingValues.test.ts` | `readValue` for every key (including `calendarConnected` text); `patchFor` for each control (trim, the semicolon split, the number unions); `togglePatch` and `resetPatch` return exactly one key; no `calendarConnected` patch exists |
| `src/features/game-mode/recap.test.ts` | the sentences for: nothing stopped; 1 job; 2 jobs and 3 sessions; a launch error; `complete: false` |
| `src/data/rest/stream.test.ts` (extends the foundation's) | the `rev` rule: a lower or equal `rev` is dropped; a higher is delivered; `deliver` returns the payload either way; a new `startId` clears the mark; `note` raises but never drops |
| `src/keys/summon.test.ts` | the reset does nothing in `review`, `countdown`, `windingDown`; it runs in `off` and when the phase is unknown (the resource has no data) |
| `src/data/rest/client.test.ts` (extends) | path, method and body of `getSettings`, `updateSettings`, `checkPaths`, `getGameMode`, `startGameMode` (empty body `{}` without a game), `resolveGameMode`, `markGameModeRecapSeen`; error mapping of a `409 conflict` body |

Fake bridge answers are canned JSON files typed by the generated types (`satisfies Settings`, `satisfies GameModeState`) in `src/data/rest/fixtures/`: `settings.typical.json`, `settings.empty.json`, `gamemode.off.json`, `gamemode.review.json`, `gamemode.countdown.json`, `gamemode.winding-down.json`, `gamemode.done.json`, `gamemode.recap.json`. They replace the mock's scenarios for tests. Screens are not unit tested (as today); they are checked by running the app on a backend with a fixture (11).

### 10.5 What is checked by hand

Autostart at a real logon, the Windows disabled mark, a held hotkey, the real exit of the shell and its webview, the first load after game mode, a game that survives Rain, a hidden window during game mode. They are acceptance checks S5, S6, S9, G1 to G11 (section 11).

### 10.6 The fixture data folder once the mock is gone

One set of fixture folders serves the integration harness, the smoke check and the developer who looks at the app in a browser or in `tauri dev`. The frontend tests do not need a backend (canned JSON, 10.4).

| Need | Before (mock) | After (fixture) |
|---|---|---|
| Typical | scenario "Typical" | `npm run dev:fixture` (starts the backend on a copy of `tests/fixtures/typical/data` in `.rain-dev/data`, then vite). The copy is thrown away at the next start, so the fixture itself is never changed by a session |
| Empty | scenario "Empty" | `npm run dev:fixture -- --fixture empty` |
| Failing | scenario "Failing" | `npm run backend:stop`, or `--fixture broken` (a syntax error in `settings.toml`) |
| Busy | scenario "Busy" | run the fixture job "Demo wait" from Jobs; game mode then shows `review` |
| Reset | "Reset mock data" | stop and start the dev backend again; it recopies the fixture |

Rules for a fixture: fake names, no path of a real person, placeholders `{DATA}` and `{TEMP}` for every absolute path, no file under a real user profile, no secret. A section adds its own files to `typical` and `broken` as foundation 12.2 says; this section owns `settings.toml` and the `detect` samples. The old mock's amounts (about 125 apps, 400 habit days, 1,500 disk nodes) are not reproduced: a fixture holds a few items of each kind.

---------------------------------------------------------------------------------------------------

## 11. Acceptance checks

"Empty" means a data folder with no files (fixture `empty`). All steps run in a debug build (sandbox) unless a step says "installed build". Where a path is needed the user's own folder is meant, written `<folder>`.

### 11.1 Settings and lifecycle

| # | Stage | Check | Expected |
|---|---|---|---|
| S1 | 1b | `npm run check` on a clean tree | passes, including the smoke steps that read `GET /settings` |
| S2 | 1b | `npm run tauri dev`, open Settings, switch "Hide when focus is lost" off, `npm run backend:restart`, reopen Settings | the switch is still off; `settings.toml` in the sandbox data folder holds the header, `schema_version = 1` and `hide_on_blur = false`, nothing else |
| S3 | 1b | look at the Sources group | the row "Calendar" reads "Not connected" and has no button and no badge; the About row still has its badge until stage 10; Home and Life still say "Calendar not connected" |
| S4 | 1b | in the browser console of `http://localhost:1420`: `fetch('/api/v1/settings', { method: 'PATCH', headers: { 'X-Rain-Dev': '1', 'Content-Type': 'application/json' }, body: '{"hotkey":"x"}' })` | `400`; the body has `code: "invalid"` |
| S5 | 2, installed build | switch "Start with Windows" on; `reg query HKCU\Software\Microsoft\Windows\CurrentVersion\Run /v Rain`; sign out and sign in | the value is `"<install folder>\Rain.exe" --hidden`; after sign-in `rain.exe` and `rain-backend.exe` are running, no window is visible; Ctrl+` shows the window with data already loaded; closing the window leaves the backend running; switching the switch off removes the value. This also answers appendix C: the backend started by a Run-key shell survives that shell. |
| S6 | 2, installed build | with the switch on, disable Rain in Task Manager, Startup apps; reload Settings; then switch it on in Rain | the switch reads off after the Task Manager change; switching it on makes Task Manager show Rain as enabled |
| S7 | 2 | with Settings open, edit `settings.toml` by hand (`reset_after_seconds = 300`) | the select shows "5 min" within 1 second without a click. Then write `reset_after_seconds = = 1`: `GET /health` lists the issue with its line; changing a switch in Rain shows "settings.toml has an error on line N"; fixing the file clears the issue |
| S8 | 2 | Data folder: set a new empty folder (`<folder>`) | notice "Data folder changed"; the service restarts within about 5 s (a new `startId` in the log); the folder now holds a copy of the old known files; the old folder is unchanged; setting it back re-points without copying; a folder that already holds Rain files is read as it is; with a run in progress the change is refused with the "Wait until the running jobs end" text |
| S9 | 2 | start any program that holds Ctrl+` (a throwaway script that calls `RegisterHotKey` is enough), then start Rain | the Summon hotkey row shows a failed dot "Another app already holds this chord"; end that program, start Rain a second time from the Start menu: the dot clears |
| S10 | 2 | `rain-backend.exe` idle for 60 s | under 30 MB working set and under 0.1 s CPU gained (foundation stage 2; this section adds no timer) |
| P1 | 3 | type an existing folder, a missing folder and an empty text in "Project folders" | glyph found after about 0.3 s; glyph missing; no glyph and the placeholder "Not set"; fast typing never flashes "Missing" for a folder that exists |
| P2 | 3 | stop the backend, type in a path row | no glyph and no error flash; after `npm run backend:start` the glyph returns on the next edit |
| E1 | 1b | start on fixture `empty` | Settings shows the defaults; the data folder is still empty; the vault row shows "Not set" (in stage 1b the backend returns only the stored override, which is empty; detection starts in stage 2, see E1b); Project folders and Game folders show "Not set" without a glyph |
| E1b | 2 | start on fixture `empty` with a fake `obsidian.json` | the vault row shows the only registered vault, or "Not set" when there are none or several |

### 11.2 Game mode

For checks that need running work, use fixture `typical` with its job "Demo wait" (about 90 s) and run it from the Jobs screen.

| # | Stage | Check | Expected |
|---|---|---|---|
| G1 | 9, installed build | nothing running; Ctrl+G | "Closing in 10 s" (Settings countdown), then "Winding down" with the rows going stopping then stopped in order, then "Rain is off"; the window closes about 3 s after the last row. In PowerShell: `Get-Process rain, rain-backend -ErrorAction SilentlyContinue` is empty, and `Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" \| Where-Object CommandLine -like '*app.rain.desktop*'` is empty (this is the check that `app.exit` ends the webview) |
| G2 | 9 | run "Demo wait", Ctrl+G | the Scheduled jobs row says `interrupts`; the heading says "1 thing will be interrupted"; Enter on "Wait for them" shows "Waiting for 1 thing to finish"; when the job ends by itself the countdown starts without a key |
| G3 | 9 | run "Demo wait", Ctrl+G, Ctrl+Enter (Stop anyway), then Enter (Close now) | after the wind-down `Get-Process ping -ErrorAction SilentlyContinue` is empty; the job's run is `stopped` (status idle, reason `gameMode`) in its history after the next start, and it runs once at that start |
| G4 | 9 | have a `claude` session open in a terminal before; run game mode | the Claude sessions row says "left running" with the right count and has no dot; after Rain has exited the terminal session is still alive and answers |
| G5 | 9 | a job that runs longer than 3 minutes (edit the fixture command to `ping -n 300 127.0.0.1`), Wait | after 3:00 the screen returns to `review` with "They did not finish in 3 minutes." and the button "Wait for them" is back |
| G6 | 9 | Esc during the countdown | "Game mode cancelled", the screen goes back; a job that came due meanwhile starts once; Ctrl+` still works |
| G7 | 9, installed build | after G1 or G3, start Rain from the Start menu | Rain opens; one notice "Game mode stopped 1 job. ..." (or "Game mode closed Rain."); closing and reopening Rain shows no second notice |
| G8 | 9 | start game mode with "Demo wait" running, press Shift+Esc to hide the window, wait 90 s, press Ctrl+` | the Game mode screen is still showing; game mode was not cancelled and not reset to Home |
| G9 | 9 | add a custom game that points at a small harmless program, then "Play in game mode" | after the wind-down the program is running; `Get-CimInstance Win32_Process` shows it was started by a process that no longer exists (it is not a child of `rain-backend`) and it is still running after Rain has exited. A custom game with a missing program shows "The game did not start: ..." under "Then launch" and the notice "The game did not start." at the next start |
| G10 | 9 | kill `rain-backend.exe` in Task Manager during "Winding down"; start Rain | the notice says "Game mode did not finish." |
| G11 | 9 | start game mode, do nothing for 11 minutes with a job running | game mode has cancelled itself; the job's schedule works again |

### 11.3 End of the mock

| # | Stage | Check | Expected |
|---|---|---|---|
| F1 | 10 | the eight checks of 9.8 | all pass |
| F2 | 10 | `npm run dev:fixture` with `typical`, then with `empty`, walk every rail screen | each screen shows real fixture data or a true empty state; no screen shows sample data from the old mock (for example "Web dev" or the 400-day heatmap) |
| F3 | 10 | stop the backend (`npm run backend:stop`) and walk the screens | each shows its error row with Retry; Retry after `npm run backend:start` recovers |
| F4 | 10 | open Settings | no "Prototype" group, no "Mock data" badge; the version text equals `package.json`; the command list (Ctrl+K) has no "Mock scenario", "Reset mock data" or "Simulate permission request" |
| F5 | 10 | `npm run build` | passes; the output holds no mock code (check 6 of 9.8) |

---------------------------------------------------------------------------------------------------

## 12. Open points

Format: question, who decides, what the code does until then, what it blocks. A: the architect (changes to `00-foundation.md`). O: another section. U: the user.

### 12.1 For the architect (A)

| # | Question or request | Until decided | Blocks |
|---|---|---|---|
| A1 | The stage 1a identity check compares generated types with the hand-written ones (foundation 9.4, 14.1). `Settings` changes shape in this section (F9). Proposed: stage 1a writes `Settings` in its final shape and compares it with the TypeScript of 3.1 held in the test's own file; `src/data/types.ts` is not touched in 1a; the barrel takes over in 1b. Also unproven: `Option<enum with container-level ts(type)>` together with `optional_fields` (3.1); the fix is `#[ts(optional)]` | as proposed | 1a |
| A2 | New file `core/winddown.rs` (`WindDownPort`, `Counts`, `Stopped`, `StopOutcome`, 6.7.1) and a line in 11.1. It lives in core so that `jobs` and `idle` do not import `gamemode` | as proposed | 6 |
| A3 | DONE in foundation 5.4 (`EventBus::freeze()` exists; nothing left to ask). `EventBus::freeze()`: after `done`, every publish is dropped, so `done` stays the last topic frame even if a module publishes during its `stop()` (6.7.5). Foundation 2.6 relies on modules not publishing in `stop()`; this makes it a guarantee | the driver calls `freeze()`; a test asserts it | 9 |
| A4 | DONE: the timers are in foundation 11.5 (T2, T3, T4, T5) and the constants are in foundation Appendix A. Timers and constants. Appendix A holds: `GAME_MODE_WAIT_LIMIT` 180 s, `REVIEW_ABANDON` 10 min, `GAME_MODE_ITEM_BUDGET` 5 s, `GAME_MODE_LAUNCH_BUDGET` 10 s, `GAME_MODE_ITEM_MIN_VISIBLE` 400 ms, `RESTART_DELAY` 500 ms, `DETECT_TTL` 30 s, `GAMEMODE_RECORDS_KEPT` 20 | the constants live in `rain_types::limits` from stage 1a | 9 |
| A5 | Foundation 2.6 lists the callers of the shutdown function: add the `settings` module (reason `restart`, 6.3). `Config::from_env` reads `machine.toml` and applies the fallback of 4.2 (it runs before any module). 10.1 says the file is "written by the settings module only": true, and read by `config.rs` | as written here | 2 |
| A6 | Registry (14.4) and topics (5.3): add topic `settings`, methods `getHealth` and `markGameModeRecapSeen`, route `POST /game-mode/recap/seen`. (`checkPaths` is section 05's.) Done in foundation 5.3, 14.4 and 15. The foundation's acceptance of stage 2 ("a hand edit shows in the UI within 1 s without a click") needs topic `settings`: it is decided here | as written here | 1b |
| A7 | ACCEPTED: `RainApi.getHealth()` on `GET /health` (registry row owner core, stage 2) and one info line in the Settings screen when `configIssues` is not empty (text: "<file> has an error on line N"); see 9.1 step 2 | none; the PATCH error text only | nothing (a convenience) |
| A8 | Move the shell rules of 6.5, 6.8 and 9.4 into foundation 6.8 (`--hidden` and the second-launch callback, the hotkey retry, the exit task and its abort) and add `GAME_MODE_EXIT_GRACE` handling to 6.2 | as written here | 2, 9 |
| A9 | DECIDED in foundation 8.6: `type MockApi = Omit<RainApi, RetiredMockMethod>`, created in stage 2 by this section (with `getHealth`, 9.2 step 2) and extended by every later stage with the methods it retires (the per-stage table is in foundation 8.6), and `routing.ts` may not route a retired method to `mock`. The all-mock run (`dev:ui`) stops being complete from stage 3, which is accepted | as assumed | 3 |
| A10 | Stage 10 scripts: the version-equality check in `check.mjs` (9.4). `dev:fixture` and `--fixture` on `dev.mjs` and `backend.mjs` are built in stage 1b (foundation 12.3 and 14.1); stage 10 only removes `dev:ui` and `VITE_RAIN_MOCK` | as written here | 1b, 10 |
| A11 | DONE: foundation 14.1 (stage 1b) now says what this section says: `startWithWindows` returns `false` and a patch of it answers `503 notBuilt` until stage 2 (a stored switch with no effect would lie); `steamPath` and `epicPath` return their override or `""` | as written here | 1b |

### 12.2 For other sections (O)

| # | What this section needs or defines for another section | Who | Until then | Blocks |
|---|---|---|---|---|
| O1 | Jobs and idle jobs implement `WindDownPort` (6.7.1); `sessions().open_user_sessions()` counts alive sessions that are not runs of Rain's own jobs; run history records outcome `stopped` (status `idle`) with reason `gameMode`; the sessions module offers `install_hooks()`, `remove_hooks()`, `hooks_state() -> MonitorClaudeState` (idempotent, and sandbox-safe as in foundation 13.3); the `typical` fixture's `jobs.toml` holds a job "Demo wait" of about 90 s | 03 | fakes in the tests; `monitorClaude: true` answers `notBuilt` | 8, 9 |
| O2 | Games offer `get(id)` and `launch_after_game_mode(id)` (6.7.1; start outside any job, record last played, return within 10 s, report `in_job`). Epic: this section defines `epicPath` as the Epic Games Launcher data folder that holds `Data\Manifests` (6.1.4); section 02 may change that definition and the detector | 02 | `FakeGames`; the definition of 6.1.4 | 5, 9 |
| O3 | SETTLED by the editor: tool path overrides are not in this section. Section 02's `[tools]` in `projects.toml` holds `editor`, `terminal`, `claude` and `git` (used by projects); section 03's `claude_program` stays per job and does not read `[tools]`. | none | as written | none |
| O4 | `life::Service::check_note_format(&str) -> Result<(), String>` for the deep check of `dailyNoteFormat` (the token syntax is section 04's); `Resolved.vault`, `daily_note_folder` and `daily_note_format` are the only inputs section 04 uses for note paths | 04 | only the character rules of table V | 7 |
| O5 | This list is the interface between this section and 02, 03 and 04; each of them states its half in its own "Open points" | 02, 03, 04 | | 9 |

### 12.3 For the user (U)

| # | Question | Default until answered |
|---|---|---|
| U1 | Rain has no way to quit except game mode, `rain-backend --stop` and Task Manager (no tray, the window only hides). Wanted: a command "Quit Rain" that ends the shell and the backend without the countdown? | none; not built |
