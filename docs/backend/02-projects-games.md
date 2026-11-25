# Rain backend, section 02: projects and games

| | |
|---|---|
| Area | projects (list, git facts, environments, opening) and games (Steam, Epic, Riot, game folders, hand-added games; hide, covers, launching), with their pins and search entries |
| Modules | `projects`, `games` (foundation section 11.2) |
| Stage | 5 (foundation section 14.3) |
| Author | backend designer, section 02 |
| Date | 2026-11-22 |
| Status | draft 1, written on top of `00-foundation.md` draft 2 and `decisions.md` |
| Binding inputs | `decisions.md` (wins), `00-foundation.md`, `foundation-review.md` section 2.2, spike files `winapi/src/vdf.rs`, `winapi/src/bin/steam.rs`, `winapi/src/bin/registry.rs`, `process/src/bin/parent.rs`, `process/src/bin/q6.rs`, `process/src/bin/q7.rs` |

Words used with one meaning only (in addition to the foundation list):

| Word | Meaning |
|---|---|
| root | one entry of `Settings.projectRoots` |
| game folder | one entry of `Settings.gameFolders` (for example `D:\Games`) |
| folder game | a game that is one direct subfolder of a game folder |
| hand-added game | a game the user added with "Add a game" (`saveCustomGame`) |
| local game | a folder game or a hand-added game. Both have `source = "custom"` (decision G1 below) |
| program file | the `.exe` Rain starts for a local game |
| guess | the program file that Rain picked by itself for a folder game (section 6.11) |
| correction | what the user changed for one game: program file, title, cover, hidden (stored in `games.toml`) |
| step | one thing a project open does: editor, terminal tab, Claude tab, browser, other app |
| game key | the stable text that identifies a game in `games.toml` and in SQLite (section 6.1) |

All paths in this document are placeholders (`D:\Games`, `C:\Dev`). Nothing personal is in code, fixtures or defaults.

---------------------------------------------------------------------------------------------------

## 1. Scope

### 1.1 What this section builds

| Part | `RainApi` methods and topics | Module | Built in |
|---|---|---|---|
| Project list: discovery in `projectRoots`, git facts, activity, missing, open, pinned | `listProjects`; topic `projects` | `projects` | stage 5 |
| Project environments and opening a project, mode `full` and `editorOnly` (and two small new modes, section 6.7) | `launch` for `target.type = "project"` | `projects` (the route `POST /launches` belongs to section 01, which calls this module) | stage 5 |
| Game library: Steam, Epic, Riot, game folders, hand-added games | `listGames`; topic `games` | `games` | stage 5 |
| Game corrections: hide, unhide, title, program file, cover | `setGameHidden`, new `updateGame` | `games` | stage 5 |
| Hand-added games | `saveCustomGame`, new `deleteCustomGame` | `games` | stage 5 |
| Starting a game, play time and last played recorded by Rain | `launch` for `target.type = "game"` | `games` | stage 5 |
| Covers as media | `GET /media/game-cover/{id}` (core route, provider registered by `games`) | `games` | stage 5 |
| Pins and search entries of projects and games | no route of its own; section 01 (`/search`, `/pinned`, `/recent`) calls the `Service` methods of section 6.15 | both | stage 5 |
| Settings field `gameFolders` | `getSettings`, `updateSettings` (section 06 owns the route; this section owns the field) | `settings` (06) | stage 5 |

### 1.2 What this section does not build

- Owned-but-not-installed Steam games (no local name list; decisions.md section 5: installed games only).
- Any network request (covers are local only; foundation 13.2).
- Starting Claude sessions as a feature of its own: Rain only starts the `claude` program in a terminal tab as the user decided. Reading, listing and answering sessions is section 03.
- Writing anything into a project folder or a game folder. Rain reads them and starts programs in them. The only writes of this section: `games.toml`, the commented `projects.toml` at first start, picked cover images in `<data>\media\`, SQLite tables of section 5.
- The route `POST /launches` itself, the run records, `getLaunch` and the `launch` topic: section 01. This section provides what that route needs (6.7 and 6.14).

### 1.3 Decisions of `decisions.md` implemented here

| Decision (section of decisions.md) | Where |
|---|---|
| Projects are every git repository directly inside the folders of `projectRoots` (4) | 6.2 |
| Opening a project: VS Code, a Windows Terminal tab in the folder, Claude Code started in that terminal; Ctrl+Enter editor only; per-project override in the data folder (4) | 6.5 to 6.7, 4.1 |
| Steam: installed games of the signed-in account, local cover art only; Epic off when not installed, not a failure (5) | 6.9, 6.10 |
| Game folders: `gameFolders` list, one game per direct subfolder, skip `_` folders and loose files, program-file guess with scoring, correction in the data folder, size from the folder, play time and last played recorded by Rain, direct start in the program's own folder (4) | 6.11 to 6.14 |
| C10 launching: ShellExecute for URIs and shortcuts, direct start where exact quoting or a working folder is needed, never inside a kill-on-close job (3) | 6.7, 6.14 |
| C4 icons and covers through `rain-media`, scheme-less relative references (3) | 6.12 |
| No polling timers; facts cached and refreshed on demand (2, foundation 11.5) | 7 |

### 1.4 Decisions made in this section (the reader may care)

| # | Decision | Reason |
|---|---|---|
| G1 | `GameSource` stays `'steam' \| 'epic' \| 'custom'`. Folder games and hand-added games are both `custom`; they share one model, `LocalGame`. The UI label of `custom` changes from "Custom" to "Local". | One model, no new tab, no wire change. A folder game and a hand-added game differ only in where the entry comes from. |
| G2 | `GameLibrary.failedSources: GameSource[]` becomes `sources: GameSourceStatus[]` with the three states `ok`, `off`, `failed`. | "Epic not installed" is not a failure (decisions.md section 5); a missing game folder is one. |
| G3 | Hidden games get a "Hidden" tab in the Games strip. The tab exists only while at least one game is hidden. | The only way back to a hidden game; no clutter otherwise. |
| G4 | A hand-added game whose program file lies inside a folder game is refused (`conflict`, `alreadyListed`). | No duplicate tiles. |
| P1 | `Project.lastOpenedAt` and `Project.dirtyFiles` become optional; `Project.sessionIds` is removed. | A project never opened from Rain has no open time; a missing `git` gives no change count; `sessionIds` duplicates `ClaudeSession.projectId` and goes stale. |
| P2 | "Terminal here" and "New Claude session here" become real (launch modes `terminal` and `claude`). | The prototype only showed a notice; with a real backend a notice that claims an action happened would be a lie. |
| P3 | The Games action "Open store page" is removed and replaced by "Show in Explorer". | There is no local source for store pages and no network request is allowed this round. |
| P4 | Two new topics, `projects` and `games` (whole lists, at most 1 frame per second). | Git facts, sizes and running state arrive after the first answer; the open screen should follow without a click. |

---------------------------------------------------------------------------------------------------

## 2. Routes

Paths are after `/api/v1`. "Idempotent" follows foundation 4.6.

| Method | Path | Request | Response | Success | Errors (code, reason) | Idempotent | Topic effects |
|---|---|---|---|---|---|---|---|
| GET | `/projects` | none | `Project[]` | 200 | 503 `unavailable` `moduleFailed` or `shuttingDown` | yes | may start a background git refresh; the result arrives on `projects` |
| POST | `/projects/refresh` (new) | none | `Project[]` | 200 | 409 `conflict` `gameMode`; 503 as above | yes | recomputes the git facts of every project now (waits at most 8 s, section 6.3), publishes `projects` |
| GET | `/projects/{id}/layout` (new `getProjectLayout`) | none | `ProjectLayout` | 200 | 404 `notFound` `unknownId`; 503 as above | yes | none |
| PUT | `/projects/{id}/layout` (new `saveProjectLayout`) | `LayoutSave` | `ProjectLayout` | 200 | 404 `notFound` `unknownId`; 400 `invalid`: `unknownStep`, `unknownApp`, `badMonitor`, `tooManyItems`; 409 `conflict`: `gameMode`, `configBroken`, `configNewer`, `configChanged`; 500 `failed` `saveFailed` | yes | `projects` (the file watcher) |
| GET | `/games` | none | `GameLibrary` | 200 | 503 `unavailable` `moduleFailed` or `shuttingDown` | yes | may start the background size measurement; results arrive on `games` |
| POST | `/games/refresh` (new) | none | `GameLibrary` | 200 | 409 `conflict` `gameMode`; 503 as above | yes | drops the guess and size caches, rescans every source, publishes `games` |
| POST | `/games` (`saveCustomGame`) | `CustomGameDraft` | `Game` | 201 | 400 `invalid`: `noName`, `noExe`, `exeNotAbsolute`, `exeNotFound`, `notAnExe`, `coverNotFound`, `coverNotImage`, `coverTooLarge`; 409 `conflict`: `alreadyListed`, `configBroken`, `configNewer`, `configChanged`; 500 `failed` `saveFailed` | no, but a second identical call gets 409 `alreadyListed` (same program file) | `games` |
| PATCH | `/games/{id}` (`setGameHidden`, new `updateGame`) | `GamePatch` | `Game` | 200 | 404 `notFound` `unknownId`; 400 `invalid`: `emptyPatch`, `notEditable`, plus the reasons of `POST /games`; 409 `conflict`: `configBroken`, `configNewer`, `configChanged`; 500 `failed` `saveFailed` | yes | `games`; hiding also removes the game's pin (section 01 pins) |
| DELETE | `/games/{id}` (new `deleteCustomGame`) | none | none | 204 | 409 `conflict`: `notRemovable` (a Steam, Epic, Riot or folder game), `configBroken`, `configNewer`, `configChanged`; 500 `failed` `saveFailed`; an unknown id is 204 | yes | `games`; also removes the pin |
| POST | `/launches` (section 01) with `target.type = "project"` | `LaunchTarget` | `LaunchRun` | 201 new run, 200 the running run is returned | 404 `notFound` `unknownId` ("Project not found"); 400 `invalid` `noSteps`; 409 `conflict` `gameMode` | yes: a second call while a run of the same project is going returns that run | `launch` (one frame per step change), `projects` after the run |
| POST | `/launches` (section 01) with `target.type = "game"` | `LaunchTarget` | `LaunchRun` | 201 new run, 200 the running run is returned | 404 `notFound` `unknownId` ("Game not found"); 409 `conflict` `gameMode` | yes: same rule as above | `launch`, `games` after the run |
| GET | `/media/game-cover/{id}` (core route, 4.7) | `v` optional | image bytes | 200, 304 | 404 `notFound` (JSON body; the `<img>` shows its fallback) | yes | none |

Request and body rules: `GamePatch` and `CustomGameDraft` use `deny_unknown_fields` (foundation 4.2). The body limit is the default 1 MiB (cover images are read from disk by path, never sent in a body).

Reasons and their messages are in section 8.

Notes on the table:

1. `setGameHidden(id, hidden)` stays a `RainApi` method. The REST client sends `PATCH /games/{id}` with `{ "hidden": <bool> }` and ignores the answer body. `updateGame(id, patch)` sends the full patch and returns the game. One route, two client methods.
2. The registry rows of foundation 14.4 for `listProjects`, `listGames`, `saveCustomGame`, `setGameHidden` and `launch` are used unchanged. DONE: the registry rows `updateGame` = `PATCH /games/{id}`, `deleteCustomGame` = `DELETE /games/{id}`, `POST /projects/refresh` and `POST /games/refresh` exist in foundation 14.4 (the two refresh routes have no `RainApi` method: they serve tests, scripts and `npm run` tools, and a later "Rescan" button).
3. Section 01 owns the one launch engine for all four target types and dispatches `POST /launches` by `target.type`. For `project` and `game` it asks the `LaunchPlanner` that this module implements (section 01 6.9; section 6.15 below): `plan(target)` returns the steps, the engine runs them and publishes every frame, and `finished(run)` (an async method that the engine awaits) writes this module's history before the final frame. The answer is the `LaunchRun` as it is at that moment: the first step is already `running`. This module has no run task of its own. A repeated call for a project or game whose run is still going returns that run (status 200, as section 01 does; 201 means a new run).
4. While `game_mode_active` is true, both launch calls and both refresh routes answer 409 `gameMode` (foundation 11.5). Reads still work and use the caches. The one exception is `games::Service::launch_after_game_mode`, which section 06 calls itself (6.14).

### 2.1 Live topics

| Topic | Payload | Kind | Max rate | Published by, and when |
|---|---|---|---|---|
| `projects` (new) | `Project[]` | whole list | 1 per second (`core::throttle::Throttle`, newest wins) | `projects` module: after a background git refresh that changed a value; after `projectRoots` or `projects.toml` changed; after a project run ended (`lastOpenedAt`, `open`); after `POST /projects/refresh` |
| `games` (new) | `GameLibrary` | whole list | 1 per second | `games` module: after a size measurement; after a Rain-started game process ended; after a launch run ended; after `games.toml`, `gameFolders`, `steamEnabled`, `steamPath`, `epicEnabled` or `epicPath` changed; after `saveCustomGame`, `updateGame`, `deleteCustomGame`; after `POST /games/refresh` |
| `launch` (section 01) | `LaunchRun` | one item, key `run.id` | per step change | this module through the run handle |

Both new topics are rows of the contract-change table (section 9.2): Rust `Topic` structs `Projects` and `Games` in `rain-types/src/events.rs` (foundation 5.3), name in `TOPIC_NAMES`, the `Topic` union in `api.ts`, the two lines of `routing.ts`. Size check: 100 projects at about 0.7 KB and 180 games at about 0.35 KB are far below `FRAME_MAX` (256 KiB). A test asserts that a library of 400 games and 200 projects stays under it.

---------------------------------------------------------------------------------------------------

## 3. Types

File `crates/rain-types/src/projects.rs` and `crates/rain-types/src/games.rs`. Conventions: foundation 9.2 (camelCase, `Option` with `skip_serializing_if` and `default`, `#[ts(optional_fields)]` on the struct, `#[ts(inline)]` on string-enum fields, ids and dates are `String`). Every type below is added to the `decls!` list of `bindings.rs`, except `SourceState`, which is declared once by the foundation (`system.rs`).

### 3.1 Projects

```rust
// projects.rs
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use crate::common::IconRef;

/// What one step of "open this project" is. Same five values as the TypeScript contract today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum EnvKind { Editor, Terminal, Browser, Claude, App }

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct EnvStep {
    #[ts(inline)]
    pub kind: EnvKind,
    pub label: String,
    pub icon: IconRef,
    /// Kept only so the mock still type-checks until stage 10. The backend never sets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    /// Text shown in mono in the detail pane: the command of a terminal step, or `claude`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct Project {
    pub id: String,                       // "proj-<slug>", section 6.1
    pub name: String,
    pub path: String,                     // backslashes, no trailing backslash
    pub icon: IconRef,
    pub branch: String,                   // "" when the folder is not readable as a repository
    /// Absent = unknown (git missing, git failed, not computed yet). 0 = clean.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dirty_files: Option<u32>,
    /// 14 counts, oldest first, last = today (local days). 14 zeros when unknown.
    #[serde(rename = "commits14d")]
    pub commits_14d: Vec<u32>,
    /// Absent = never opened from Rain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_opened_at: Option<String>,
    pub activity_score: u32,
    pub environment: Vec<EnvStep>,
    pub open: bool,
    pub pinned: bool,
    pub missing: bool,
}
```

Resulting TypeScript (the generator output is authoritative; this is the shape it must have):

```ts
export type EnvKind = "editor" | "terminal" | "browser" | "claude" | "app";
export type EnvStep = { kind: "editor" | "terminal" | "browser" | "claude" | "app", label: string, icon: IconRef,
  appId?: string, command?: string, url?: string, };
export type Project = { id: string, name: string, path: string, icon: IconRef, branch: string,
  dirtyFiles?: number, commits14d: Array<number>, lastOpenedAt?: string, activityScore: number,
  environment: Array<EnvStep>, open: boolean, pinned: boolean, missing: boolean, };
```

| Change against `src/data/types.ts` lines 31 to 37 | Why |
|---|---|
| `dirtyFiles` required to optional | absent when git is missing or has not answered; 0 would claim "No changed files" |
| `lastOpenedAt` required to optional | a project that was never opened from Rain has no time; the UI sorted on `Date.parse` and drew `RelTime` on it |
| `sessionIds` removed | duplicate of `ClaudeSession.projectId`; stale between fetches; `project-sessions.ts:14` loses one clause |
| `appId` stays, never set | the mock still fills it until stage 10 |
| everything else equal | `commits14d` keeps its wire name through `#[serde(rename)]` |

### 3.2 Games

```rust
// games.rs
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum GameSource { Steam, Epic, Riot, Custom }          // 'custom' = every local game (decision G1)

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct Game {
    pub id: String,                      // section 6.1
    pub title: String,
    #[ts(inline)]
    pub source: GameSource,
    /// "media/game-cover/<id>?v=<hex>"; absent = no art, the UI draws the title tile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_url: Option<String>,
    /// Steam, Epic: always true (only installed games are listed).
    /// Local game: true while its program file exists; a folder game without any program file is true too.
    pub installed: bool,
    pub running: bool,
    /// Logical size of the install folder. Absent until measured, or when there is no folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    pub playtime_minutes: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_played_at: Option<String>,
    pub pinned: bool,
    pub hidden: bool,
    /// Local games only. Absent = no program file found or chosen yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exe_path: Option<String>,
    /// Local games only: true while `exePath` is Rain's guess and not the user's choice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exe_guessed: Option<bool>,
    /// The folder of the game, when Rain knows it (all sources except a hand-added game).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_path: Option<String>,
    /// true only for a hand-added game (the only kind "Remove" works for).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removable: Option<bool>,
}

// `SourceState` (`Ok`, `Off`, `Failed`) is defined once in `crate::system` (foundation 6.9, 9.1); this file imports it: `use crate::system::SourceState;`

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct GameSourceStatus {
    #[ts(inline)]
    pub source: GameSource,
    #[ts(inline)]
    pub state: SourceState,
    /// One plain sentence. Shown as is for `failed`; kept for `off` and `ok` as a hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GameLibrary { pub games: Vec<Game>, pub sources: Vec<GameSourceStatus> }   // always four entries: steam, epic, riot, custom

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(optional_fields)]
pub struct CustomGameDraft {
    pub title: String,
    pub exe_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_path: Option<String>,
}

/// Every field optional; an absent field means "unchanged". An empty string clears:
/// `title: ""` = use the folder name again, `exePath: ""` = go back to the guess, `coverPath: ""` = remove the cover.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(optional_fields)]
pub struct GamePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")] pub hidden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub exe_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub cover_path: Option<String>,
}
```

Resulting TypeScript:

```ts
export type GameSource = "steam" | "epic" | "custom";
export type Game = { id: string, title: string, source: "steam" | "epic" | "custom", coverUrl?: string,
  installed: boolean, running: boolean, sizeBytes?: number, playtimeMinutes: number, lastPlayedAt?: string,
  pinned: boolean, hidden: boolean, exePath?: string, exeGuessed?: boolean, installPath?: string, removable?: boolean, };
export type GameSourceStatus = { source: "steam" | "epic" | "custom", state: "ok" | "off" | "failed", message?: string, };
export type GameLibrary = { games: Array<Game>, sources: Array<GameSourceStatus>, };
export type CustomGameDraft = { title: string, exePath: string, coverPath?: string, };
export type GamePatch = { hidden?: boolean, title?: string, exePath?: string, coverPath?: string, };
```

| Change against `src/data/types.ts` lines 40 to 44 | Why |
|---|---|
| `Game` gains `exePath`, `exeGuessed`, `installPath`, `removable` (all optional) | the edit dialog needs the current program file; "Show in Explorer" needs the folder; "Remove" only exists for hand-added games |
| `GameLibrary.failedSources` replaced by `sources` | decision G2 |
| `GamePatch` new | one body for hide, edit and reset |
| `CustomGameDraft`, `GameSource` unchanged | G1 |
| `coverUrl` is now a `media/...` reference, resolved by the REST client (foundation 8.4) | foundation F8 |

### 3.3 Settings, topics, and the one change in section 01's types

```rust
// settings.rs (section 06 owns the file; this section owns the field)
pub struct Settings      { /* ... */ pub game_folders: Vec<String> }            // TS: gameFolders: Array<string>
pub struct SettingsPatch { /* ... */ pub game_folders: Option<Vec<String>> }    // validation below

// events.rs
pub struct Projects; impl Topic for Projects { const NAME: &'static str = "projects"; type Payload = Vec<Project>; }
pub struct Games;    impl Topic for Games    { const NAME: &'static str = "games";    type Payload = GameLibrary; }
```

Validation of `gameFolders` is the rule of section 06 table V for the list fields: at most 32 entries, each at most 1024 characters, trimmed, empty entries dropped, duplicates dropped case-insensitively after normalisation, default `[]`. An entry that is not absolute stays in the API value and is dropped from `Resolved.game_folders` (06 6.1.7). The settings module does it and section 06 tests it.

`LaunchMode` has `terminal` and `claude` in section 01 3.2 (contract change S2.8). `terminal` runs only the terminal steps, `claude` only the Claude steps; both are ignored for other target types.

---------------------------------------------------------------------------------------------------

## 4. Data files

Both files are rows of foundation 10.2 (`projects.toml`, `games.toml`, folder `media\`). Common rules of foundation 10.3 apply: `schema_version` first, missing file = defaults, a newer version is read-only, a bad entry is skipped (with a warning in the log and a message in the source status) and does not fail the file, unknown keys survive. A skipped entry is reported through `health.set_file_issues` as information only: it does not block writes, and a write keeps its text unchanged. `configBroken` is returned only when the whole file failed to parse (a syntax error).

### 4.1 `projects.toml`

Owner: this section. The user edits it by hand. Rain changes it in one place only: the `layout` of `[defaults]` or of a `[[project]]`, when the user saves a layout in the Projects screen (6.17); comments and order stay. A change is picked up by the file watcher (foundation 10.4) and publishes `projects`.

Rain creates the file once, at the first start of the `projects` module, when it does not exist. The created file contains `schema_version = 1` and the commented example below, nothing else (foundation 10.1 allows this when the section says so).

Schema:

| Key | Type | Default | Validation |
|---|---|---|---|
| `schema_version` | integer | 1 | |
| `[tools]` `editor` | string, path to an editor program | `""` = detect | absolute path. If set and the file is missing, the editor step fails with "Configured editor not found" (no silent fallback: a wrong setting must be seen) |
| `[tools]` `terminal` | string, path to `wt.exe` | `""` | same rule |
| `[tools]` `claude` | string, path to `claude.exe` or `claude.cmd` | `""` | same rule |
| `[tools]` `git` | string, path to `git.exe` | `""` | same rule; a bad `git` path turns the source `git` to `failed` and the git facts stay absent |
| `[terminal]` `window` | `"current"` or `"new"` | `"current"` | `current` = `-w 0` (a new tab in the last used Windows Terminal window); `new` = `-w new` (always a new window) |
| `[terminal]` `profile` | string | `""` | name of a Windows Terminal profile (`-p`); empty = the default profile |
| `[defaults]` `steps` | array of step names | `["editor", "claude"]` | every name must be a built-in step (`editor`, `terminal`, `claude`) or defined by a `[[step]]`; an unknown name is skipped with a warning |
| `[defaults]` `forget_missing_days` | integer 0 to 3650 | 30 | days a vanished project stays listed as missing; 0 = never list missing projects |
| `[defaults]` `layout` | array of layout items | none | where each window goes for every project that has no layout or steps of its own (6.17); an item that breaks a rule is skipped and counted |
| `[[step]]` `name` | string `^[a-z0-9][a-z0-9_-]{0,39}$` | required | unique; a name equal to a built-in name replaces the built-in |
| `[[step]]` `kind` | `editor`, `terminal`, `claude`, `browser`, `app` | required | |
| `[[step]]` `label` | string, 1 to 40 characters | the built-in label of the kind | shown in the detail pane and in the checklist |
| `[[step]]` `command` | string up to 500 characters | none | `terminal` only: run this in the new tab (`cmd.exe /d /k <command>`). No double quote allowed (put a complicated command in a `.cmd` file); a step with one is skipped with a warning |
| `[[step]]` `url` | string | none | `browser` only: must start with `http://` or `https://` |
| `[[step]]` `delay_ms` | integer 0 to 30000 | 0 | wait before this step (a dev server needs a moment before a browser opens) |
| `[[step]]` `app` | string, absolute path | none | `app`: the program to start; `editor`: this editor instead of the detected one |
| `[[step]]` `args` | string up to 500 characters | none | `app`, `editor`: extra arguments after the folder, `claude`: arguments after `claude` |
| `[[step]]` `icon` | lucide name | the kind's icon | |
| `[[project]]` `path` | string, absolute | one of `path` or `id` is required | the project folder; compared case-insensitively after normalisation |
| `[[project]]` `id` | string | | `proj-...` id, alternative to `path` (the path wins when both are given) |
| `[[project]]` `name` | string | folder name | display name |
| `[[project]]` `icon` | lucide name | letter tile | |
| `[[project]]` `steps` | array of step names | `[defaults].steps` | replaces the default list for this project |
| `[[project]]` `layout` | array of layout items | none | this project's own layout; it beats `steps` and the default layout (6.17). An empty list is no layout |

The built-in steps (never written to the file):

| Name | Kind | Label | Icon | What it runs |
|---|---|---|---|---|
| `editor` | `editor` | `VS Code` (the file stem, capitalised, when another editor is configured) | lucide `code-xml` | the editor program with the project folder as its argument |
| `terminal` | `terminal` | `Terminal` | lucide `square-terminal` | a Windows Terminal tab in the project folder (the default profile) |
| `claude` | `claude` | `Claude Code` | brand `claude` | a Windows Terminal tab in the folder in which `claude` is started (the same tab stays open as a shell when Claude Code exits) |

Commented example (this is the text Rain writes at first start; the example lines are comments, so the defaults apply):

```toml
schema_version = 1

# Rain opens a project with two steps: VS Code, and a Windows Terminal tab in the folder
# that runs Claude Code. This file changes that. Nothing here is required.
# Edit this file with any editor; Rain notices the change. Rain itself writes only the `layout`
# lists below, when you save a layout in the Projects screen. Your comments and order stay.

# [tools]                       # only when Rain cannot find a program by itself
# editor   = "C:\\Path\\To\\Code.exe"
# terminal = "C:\\Path\\To\\wt.exe"
# claude   = "C:\\Path\\To\\claude.exe"
# git      = "C:\\Path\\To\\git.exe"

# [terminal]
# window  = "current"           # "current" = a tab in your last Terminal window, "new" = a new window
# profile = ""                  # a Windows Terminal profile name; empty = your default profile

# [defaults]
# steps = ["editor", "claude"]      # add "terminal" for a second, plain shell tab
# forget_missing_days = 30      # a project whose folder vanished stays listed (dimmed) this long

# A named step can be used by any project:
# [[step]]
# name    = "dev-server"
# kind    = "terminal"
# label   = "Dev server"
# command = "npm run dev"
#
# [[step]]
# name     = "preview"
# kind     = "browser"
# label    = "Browser preview"
# url      = "http://localhost:5173"
# delay_ms = 3000

# One project with its own steps:
# [[project]]
# path  = "C:\\Dev\\web-shop"
# steps = ["editor", "dev-server", "preview", "claude"]

# A layout says where each window goes. Each item names WHAT opens (one of step, app, folder)
# and WHERE (zone, and monitor). Without a layout every window opens where Windows puts it.
#   step    = "editor", "terminal", "claude", or the name of a [[step]] above
#   app     = the id of an app of Rain's app list, for example "app-notepad"
#   folder  = true: the project folder in Explorer
#   zone    = "free" (the default: not moved), "full", "left-half", "right-half", "top-half",
#             "bottom-half", "top-left", "top-right", "bottom-left", "bottom-right",
#             "left-third", "middle-third", "right-third", "left-two-thirds", "right-two-thirds"
#   monitor = 1, 2, ... counted from the left. Without it: the monitor Rain's window is on.
# Terminal items in the same zone become tabs of one Windows Terminal window.
# A layout of a project beats its steps. A layout under [defaults] serves every project that has
# neither a layout nor steps of its own.
# [defaults]
# layout = [
#   { step = "editor",   zone = "right-half" },
#   { step = "terminal", zone = "top-left" },
#   { step = "claude",   zone = "bottom-left" },
# ]
#
# [[project]]
# path   = "C:\\Dev\\web-shop"
# layout = [
#   { step = "editor", zone = "left-two-thirds", monitor = 2 },
#   { app = "app-notepad", zone = "right-third", monitor = 2 },
# ]
```

Hand edit: anything. A syntax error keeps the last good value and shows a `ConfigIssue` (foundation 10.4). Migration: `fn migrate` in `store.rs` is empty for version 1.

### 4.2 `games.toml`

Owner: this section. Rain writes it with `toml_edit` through `TomlFile::edit` (comments, order and unknown keys survive) when the user hides, edits, adds or removes a game. The file is created by the first write with a short header comment. A hand edit is picked up by the watcher and publishes `games`.

There are two kinds of table:

- `[[game]]`: the user's corrections of a game that Rain finds by itself (Steam, Epic, folder game). Keyed by the game key.
- `[[custom]]`: a hand-added game.

| Key | Type | Default | Validation |
|---|---|---|---|
| `schema_version` | integer | 1 | |
| `[[game]]` `key` | string | required | `steam:<appid>` (digits), `epic:<AppName>`, or `dir:<absolute folder path>` for a folder game. Compared case-insensitively after path normalisation (6.1). An entry without a valid key is skipped |
| `[[game]]` `hidden` | boolean | false | |
| `[[game]]` `title` | string 1 to 120 characters | the found title | used by `dir:` games only (Steam, Epic and Riot titles are not editable) |
| `[[game]]` `exe` | string | none (use the guess) | `dir:` games only. Relative to the game folder, or absolute. Must end in `.exe`. A path that does not exist makes the game "not installed" (the guess is not used behind the user's back) |
| `[[game]]` `args` | string up to 500 characters | none | `dir:` games only; extra command-line text given to the program. Hand edit only |
| `[[game]]` `cover` | string | none | `dir:` games only. `media/<file>` (relative to the data folder) or an absolute image path. Extension `png`, `jpg`, `jpeg`, `webp` or `gif` |
| `[[custom]]` `id` | string `game-...` | required | unique among all game ids; written by Rain |
| `[[custom]]` `title` | string 1 to 120 characters | required | |
| `[[custom]]` `exe` | string, absolute path | required | ends in `.exe` |
| `[[custom]]` `args` | string | none | as above |
| `[[custom]]` `cover` | string | none | as above |
| `[[custom]]` `hidden` | boolean | false | |

Duplicates: the first entry of a key or id wins, the later one is ignored with a warning.

Example (what Rain writes after the user hid one Steam game, corrected one folder game and added one game by hand):

```toml
schema_version = 1

# Your corrections and hand-added games. Rain edits this file when you use the Games screen.
# A [[game]] entry changes how Rain shows a game it finds by itself. Remove the entry to undo it.

[[game]]
key    = "steam:123456"          # hidden from the library
hidden = true

[[game]]
key   = "dir:D:\\Games\\Some Game"
title = "Some Game: Remastered"
exe   = "bin\\x64\\SomeGame.exe"       # relative to the folder
cover = "media/game-dir-some-game-3fa9c2d1.jpg"

[[custom]]
id    = "game-celeste"
title = "Celeste"
exe   = "D:\\Elsewhere\\Celeste\\Celeste.exe"
cover = "media/game-celeste-91ab07ee.png"
```

What Rain writes back: `hidden`, `title`, `exe`, `cover` of a `[[game]]` entry (the entry is created when needed and removed again when every field is back to its default); a new `[[custom]]` entry with a generated id; changes to a `[[custom]]` entry; the removal of a `[[custom]]` entry. Rain removes the picked cover file from `media\` when it is replaced or removed.

Defaults when the file is missing: no corrections, no hand-added games.

### 4.3 The folder `media\`

`<data>\media\<id>-<8 hex>.<ext>`: covers the user picked. The 8 hex digits are the FNV-1a hash of the file content, so a new picture has a new name and the cached `media/game-cover/...?v=` URL changes. Files are written by `games::covers::import` (temp file, `sync_all`, rename). The folder is created on the first import. Rain removes only files it wrote itself and that no entry refers to any more (checked at each import and each delete).

### 4.4 Settings keys used

Read through `settings::Service::resolved()` (section 06 6.1.7; the `Resolved` value is already normalised, so this section does no path normalisation of its own): `project_roots`, `game_folders`, `steam_enabled`, `steam_path`, `epic_enabled`, `epic_path`. Meaning of the two path fields in this round:

| Field | Meaning | Empty (`None`) means |
|---|---|---|
| `steamPath` | the Steam install folder (the one with `steamapps` and `config`); detected by section 06 only (06 6.1.3), this section runs no detection | source `off` (6.9) |
| `epicPath` | the folder of an Epic launcher (06 6.1.4): the new launcher's data folder (the one that holds `installations.jsonl`, 6.10.1) or the old launcher's `EpicGamesLauncher` folder (the one that holds `Data\Manifests`). A typed path may be either; Rain tells them apart by what is inside. The old manifests are searched at `<path>`, `<path>\Manifests` and `<path>\Data\Manifests`, the first that exists; detected by section 06 only | source `off` (6.10) |
| `epic_extra_path` (`Resolved` only, no setting) | the other launcher's folder when the PC has both and no `epicPath` is typed | nothing extra is read |

A change of any of the six fields makes both modules drop their caches and publish their topic (they subscribe through `settings::Service::watch`).

---------------------------------------------------------------------------------------------------

## 5. Tables

Each module has one migration file (foundation 10.5): `projects` has `MIGRATIONS = [0001_projects]`, `games` has `MIGRATIONS = [0001_games]`, ids start at 1. Timestamps are RFC 3339 text, durations are integers.

### 5.1 `projects` (file `0001_projects.sql`)

```sql
-- Which project folders Rain has seen. Lets a vanished folder be shown as "missing". Rebuildable cache
-- in the sense that losing it only forgets the missing projects; it holds no user data.
CREATE TABLE projects_known (
  path_key      TEXT PRIMARY KEY,          -- lower-case normalised path
  id            TEXT NOT NULL UNIQUE,      -- "proj-<slug>" as first given (keeps the id stable, 6.1)
  path          TEXT NOT NULL,             -- normalised path, original case
  name          TEXT NOT NULL,             -- folder name
  root_key      TEXT NOT NULL,             -- lower-case normalised root that found it
  first_seen_at TEXT NOT NULL,
  last_seen_at  TEXT NOT NULL
);
CREATE INDEX idx_projects_known_root ON projects_known (root_key);

-- History: when the user opened the project from Rain. Not a cache.
CREATE TABLE projects_opened (
  path_key  TEXT PRIMARY KEY,
  opened_at TEXT NOT NULL,                 -- the last successful open
  opens     INTEGER NOT NULL DEFAULT 1
);

-- Rebuildable cache of git facts.
CREATE TABLE projects_git_cache (
  path_key    TEXT PRIMARY KEY,
  head_sig    TEXT NOT NULL,               -- signature of HEAD, the HEAD reflog and the index (6.3)
  branch      TEXT NOT NULL,
  dirty       INTEGER,                     -- NULL = unknown
  dirty_at    TEXT,
  commits     TEXT,                        -- JSON array of 14 integers, NULL = unknown
  commits_day TEXT,                        -- the local date (YYYY-MM-DD) the array was computed for
  computed_at TEXT NOT NULL
);
```

| Table | Cache or history | Retention |
|---|---|---|
| `projects_known` | cache (no user data) | a row is deleted when its `root_key` is no longer a root in the settings, and when the project has been missing longer than `forget_missing_days` (default 30) |
| `projects_opened` | history | kept while the project has a `projects_known` row; deleted with it |
| `projects_git_cache` | cache | rows of paths without a `projects_known` row are deleted at each discovery |

### 5.2 `games` (file `0001_games.sql`)

```sql
-- History: what Rain itself knows about playing a game. Not a cache.
CREATE TABLE games_play (
  game_key       TEXT PRIMARY KEY,         -- "steam:<appid>", "epic:<AppName>", "dir:<lower-case path>", "custom:<id>"
  last_played_at TEXT,
  play_seconds   INTEGER NOT NULL DEFAULT 0,
  launches       INTEGER NOT NULL DEFAULT 0
);

-- Sessions that are running now (rows are folded into games_play when the process ends, then deleted).
CREATE TABLE games_session (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  game_key      TEXT NOT NULL,
  pid           INTEGER NOT NULL,
  process_start INTEGER NOT NULL,          -- FILETIME of the process creation; protects against pid reuse (SQLite only, never in JSON)
  folder        TEXT,                      -- install folder, for following a child process (6.13)
  started_at    TEXT NOT NULL,
  last_seen_at  TEXT NOT NULL              -- written at launch and at backend stop (6.13)
);
CREATE INDEX idx_games_session_key ON games_session (game_key);

-- Rebuildable cache: the program-file guess of a folder game.
CREATE TABLE games_guess_cache (
  folder_key TEXT PRIMARY KEY,             -- lower-case normalised folder path
  signature  TEXT NOT NULL,                -- 6.11
  exe_rel    TEXT,                         -- path relative to the folder; NULL = nothing found
  score      REAL,
  guessed_at TEXT NOT NULL
);

-- Rebuildable cache: size of a game folder.
CREATE TABLE games_size_cache (
  folder_key  TEXT PRIMARY KEY,
  size_bytes  INTEGER NOT NULL,
  measured_at TEXT NOT NULL,
  signature   TEXT NOT NULL
);
```

| Table | Cache or history | Retention |
|---|---|---|
| `games_play` | history | kept. One small row per game ever launched from Rain |
| `games_session` | state | a row lives while its process runs; at start rows of dead processes are folded and deleted (6.13) |
| `games_guess_cache` | cache | rows not refreshed for 90 days are deleted at each folder scan |
| `games_size_cache` | cache | same rule; also deleted by `POST /games/refresh` |

Steam and Epic scans are not stored: they cost a few milliseconds and are held in memory by signature (6.9).

---------------------------------------------------------------------------------------------------

## 6. Algorithms

Module files (foundation 11.2). Extra files are allowed; OS-facing code sits behind a trait so tests can replace it (section 10.2).

```
modules/projects/                          modules/games/
  mod.rs routes.rs service.rs store.rs       mod.rs routes.rs service.rs store.rs
  discovery.rs   roots -> repositories       steam.rs  vdf.rs        (copy of spike winapi vdf.rs)
  git.rs         GitRunner, parsers, HEAD    epic.rs
  facts.rs       cache, staleness, refresh   folders.rs guess.rs     (guess.rs is pure)
  dates.rs       civil dates, buckets, score size.rs
  tools.rs       finding programs            local.rs  (the LocalGame model, corrections)
  env.rs         projects.toml -> steps      covers.rs (media provider, import)
  launch.rs      plan, command lines, run    running.rs (running state, sessions, play time)
  windows.rs     open detection              launch.rs
```

### 6.1 Ids, keys, paths

**Path normalisation** `norm_path(s)`: replace `/` by `\`; strip a `\\?\` prefix; collapse repeated `\` (but keep the leading `\\` of a UNC path); remove a trailing `\` (not from `X:\`); upper-case the drive letter. The result keeps its case. `path_key(s)` = `norm_path(s)` lower-cased (Unicode `to_lowercase`). Every comparison of paths uses `path_key`. Both are pure functions with a table test (drive letter case, slashes, trailing slash, UNC, `\\?\`, double separators). They are for paths that this module reads from files and Windows (Steam files, process images) and for comparing; every path the user typed arrives already normalised in `Resolved` by `core::paths::normalize` (section 06 6.1.7), which rejects a `\\?\` path, so `norm_path` never has to accept one from settings.

**slug(text)**: lower-case; keep `a-z0-9`; every other character (also non-ASCII) becomes `-`; collapse runs of `-`; trim `-`; cut to 48 characters. An empty result becomes `x` + `fnv8(text)`. `fnv8(s)` = FNV-1a 32 bit of the UTF-8 bytes, 8 lower-case hex digits (about 10 lines of code, no crate).

| Entity | Id | Key (games only) |
|---|---|---|
| Project | `proj-<slug(folder name)>` | none (`path_key` is the key in SQLite and in `projects.toml`) |
| Steam game | `game-steam-<appid>` | `steam:<appid>` |
| Epic game | `game-epic-<slug(AppName)>` | `epic:<AppName>` |
| Folder game | `game-dir-<slug(folder name)>` | `dir:<norm_path(folder)>` (compared by `path_key`) |
| Hand-added game | `game-<slug(title)>`, then `-2`, `-3`, ... until free; written into `games.toml` | `custom:<id>` |

All ids use only `a-z0-9-`, so they satisfy foundation 4.3 and are unique across the four target types (`proj-`, `game-`, `app-`, `pool-`), which `useLaunch.runs` needs (it keys by `target.id`).

**Collisions** (two folders with the same slug, for example `D:\Games\Foo` and `E:\Games\Foo`): the id that was first given keeps it. For projects the first-given id is in `projects_known.id`. For folder games there is no table, so: sort the colliding candidates by `path_key`; the first keeps the plain id; each other gets `<id>-<fnv8(path_key)>`. This can move an id when a new colliding folder appears; it is rare, documented here, and tested. A folder game's hidden flag and correction are keyed by the folder (`key`), not by the id, so they never move. Pins use the id (section 01).

Renaming a project folder gives it a new id: its pin and last-opened time start again. The old row becomes "missing" for `forget_missing_days` days. A renamed game folder is a new game.

### 6.2 Project discovery and the known list

Input: `Settings.projectRoots`. Output: the list of project folders. Runs inside `core::blocking::run` and is cheap (one `read_dir` per root and one `metadata` per entry; 1000 entries cost about 20 ms).

```
for root in roots (normalised, duplicates by path_key dropped, relative paths skipped with a warning):
    entries = read_dir(root)                      // error: this root is "unreadable" (see below)
    for entry in entries:
        name = entry.file_name()
        skip if name starts with "." or "$"       // hidden and system folders
        skip if not a directory (metadata() follows junctions and links)
        skip unless `<entry>\.git` exists as a directory or a file   // a file = worktree or submodule
        project = { path: norm_path(entry.path()), name, root }
```

Only direct children count (decisions.md section 4). The root itself is not a project even if it is a repository. Folders without `.git` are not projects. A repository inside a subfolder of a subfolder is not found; the user adds that subfolder as a root.

**Known list and "missing".** After each discovery the module upserts the found projects into `projects_known` (insert with a new id, or update `last_seen_at`; written at most once per project per day, so a `GET` does not write). A known project that was not found now is returned with `missing = true` when all of these hold: its `root_key` is still one of the roots; `now - last_seen_at <= forget_missing_days`; and `forget_missing_days > 0`. Otherwise its row is deleted. A missing project has `branch = ""`, `dirtyFiles` absent, 14 zeros, `open = false`, `activityScore = 0`, and keeps `lastOpenedAt`, `pinned` and its environment.

A root that cannot be read (missing, access denied) is reported in the source status `projects` (state `failed`, message `Project folder not found: <path>`); the projects found earlier under that root stay known and show as missing. No error reaches `GET /projects`: the list is still returned. With no roots, or no repository, the list is empty and the screen shows "No projects yet".

The discovery runs on every `GET /projects` (it is cheap) and is not cached, except that a discovery that takes more than 8 s is abandoned and the previous result is used (network drive).

**Order of the answer** (the UI re-sorts, but ties keep this order, so it is fixed): `activityScore` descending, then `lastOpenedAt` descending (absent last), then `name` ascending (case-insensitive), then `path_key`.

### 6.3 Git facts

Tool: the `git` program (decision from the research; a Rust git crate was rejected for build size and compile time, foundation 1.6 does not list one). Found as in 6.5. All calls go through `trait GitRunner` (production: `platform::process::hidden_command`; tests: a fake with canned output).

| Fact | How | Cost |
|---|---|---|
| `branch` | read the file `<gitdir>\HEAD` (no process). If `.git` is a file, read its `gitdir: <path>` line first (relative paths are relative to the folder) and use that folder. Text `ref: refs/heads/<name>` gives `<name>`; any other `ref: ...` gives the part after `refs/`; a 40 or 64 hex digit text is a detached HEAD and gives the first 7 digits | microseconds; done on every list, also without git |
| `dirtyFiles` | `git -C <path> --no-optional-locks -c core.quotepath=off status --porcelain=v1 -z` (untracked files in the default `normal` mode: an untracked folder counts as one entry; ignored files are not shown). Count the NUL-separated records; for a record whose first or second status character is `R` or `C` the next record is the old path and is skipped | about 60 ms per repository (process creation), more for big trees |
| `commits14d` | `git -C <path> -c core.quotepath=off log --branches --no-merges --since="16 days ago" --max-count=10000 --format=%at`. Each output line is the author time (Unix seconds). A line is put in the bucket of its local date (`platform::time::local_date_of_unix`; `LocalClock` stays only as the test seam inside `platform::time` that fixes the UTC offset); lines older than 13 days before today or later than today are dropped. Index 13 is today. An exit code 128 with "does not have any commits yet" in the error text means 14 zeros, not a failure | about 60 ms |

Why author time and `--branches`: author time does not change when a commit is rebased or cherry-picked (committer time does, which would make old work look new); `--branches` counts work on every local branch once, without remote noise. The coarse `--since="16 days ago"` only keeps the output small; the exact window is cut in Rust. Why local dates: the strip is "per day" for the user, so days are local (foundation 4.3).

Every git call gets: `GIT_TERMINAL_PROMPT=0`, `GIT_PAGER=cat`, `LC_ALL=C`, `GIT_OPTIONAL_LOCKS=0`; no console window; a limit of 4 at once (`GIT_MAX_PARALLEL`, a semaphore in the module); output capped at 4 MiB; a timeout of 20 s per call (the child is killed, `kill_on_drop(true)`). A failed call leaves that fact unknown (`dirtyFiles` absent, or the old cached value kept and its age grows); it is logged once per project per run at `warn` with the project id (never the output). A repository that git refuses ("dubious ownership") is the same: facts unknown, one `warn`.

**Cache** (`projects_git_cache`) and when a fact is stale. The signature `head_sig` is a short text built from files only:

```
head_sig = HEAD text | mtime+len of <gitdir>\logs\HEAD | mtime+len of <gitdir>\index
           (if logs\HEAD does not exist: mtime of the ref file the HEAD points to and of packed-refs instead)
```

A commit, checkout, reset, merge or staging changes at least one of these. It does not see edits of unstaged files, which is why dirty also has an age limit.

| Fact | Stale when |
|---|---|
| `branch` | never cached: read every time |
| `commits` | `head_sig` differs from the cached one (compare only the HEAD part and the `logs\HEAD` part), or `commits_day` is not today. If only the day changed and the HEAD part is equal, no git call is needed: shift the cached array left by the number of days passed and fill with zeros (no commit can have appeared) |
| `dirty` | `head_sig` differs (any part, including the index), or `dirty_at` is older than 60 s (`DIRTY_TTL`) |

The check is made when a client asks (`GET /projects`), never by a timer. Flow of `GET /projects`:

1. Discover (6.2), read branches, load cache rows.
2. For every project: a cache row exists means its values go into the answer at once, stale or not. No row, or a git call that is needed for a fact that is absent in the row, means "must compute".
3. Projects that must compute are computed now, in parallel (4 at once), but the answer waits at most `FIRST_FACTS_BUDGET` = 6 s in total. Facts not ready in time are left absent in this answer and finish in the background.
4. Projects that are only stale (have values) are refreshed in the background. One background refresh task exists at a time (`facts::Refresher`); a request that finds it running only marks "run once more when finished".
5. When a background refresh changed any value it publishes `projects` (throttled).
6. In game mode nothing is refreshed and nothing is computed: the answer uses the cache only.

`POST /projects/refresh` clears the stale checks (sets every `dirty_at` and `commits_day` to NULL) and waits at most 8 s (`REFRESH_WAIT`); what is not ready then finishes in the background.

If `git` cannot be found (6.5): the source `git` is `off`, message "git was not found. Showing the branch only", every `dirtyFiles` is absent and every strip is zeros. If `git` is found it is `ok`.

### 6.4 `activityScore`, `lastOpenedAt`, `open`, `pinned`

- `activityScore` = `round( sum over i in 0..13 of commits14d[i] * (i + 1) / 14 )` (integer, index 13 = today has weight 1, index 0 weight 1/14). This is the formula of the mock (seed/projects.ts:106), so Home and Projects order as in the prototype. Tests: all zeros gives 0; one commit today gives 1; one commit 13 days ago gives 0.
- `lastOpenedAt` = `projects_opened.opened_at` for the project, formatted `YYYY-MM-DDTHH:MM:SS.mmmZ`; absent when the project was never opened from Rain. It is written when a project run ends with at least one successful step (6.7), before the final `launch` frame, because the UI refetches right after that frame (`useLaunch.ts:89`). Projects never opened are left out of `listRecent`.
- `pinned`: `id` is in `launcher.pinned_ids(PinnableType::Project)` (section 01 owns `pins.toml`).
- `open`: true when an editor window of this project exists now (below). Computed at answer time, cached 2 s (`OPEN_TTL`), false for a missing project and in game mode.

**Open detection.** `trait WindowLister` returns `Vec<WindowInfo { pid: u32, title: String }>` of the visible top-level windows (production: `platform::windows::list_windows`, `EnumWindows` + `GetWindowTextW` + `GetWindowThreadProcessId`, about 1 to 3 ms for 150 windows; UNPROVEN, no spike). A window counts for a project when both hold:

1. The image file name of its process (from `ProcessLister`, foundation 11.6) is the file name of the project's editor program (6.5), compared case-insensitively. If the image path is unreadable the window does not count.
2. The window title matches the folder name `N` (case-insensitive) by this rule: strip a leading `*` or a bullet and spaces; strip trailing ` (Workspace)` and trailing `[...]` tags such as `[Administrator]`; then the title matches when it equals `N`, or starts with `N + " - "` (also en dash and em dash in place of the hyphen), or contains `" - " + N + " - "`, or ends with `" - " + N`. VS Code puts the folder name just before its own name: `main.rs - web-shop - Visual Studio Code` and `web-shop - Visual Studio Code` both match for `N = web-shop`.

A folder name that equals a file name (`main.rs`) would match any window that shows that file; accepted, because condition 1 limits it to editor windows. Fixtures: section 10.1 lists the title table.

### 6.5 Finding programs (editor, Windows Terminal, Claude Code, git)

One function `tools::find(kind, env) -> Found | NotFound | ConfiguredMissing(path)` over `trait ToolEnv { path_dirs(), var(name), exists(path) }`. `exists` uses `std::fs::symlink_metadata` (not `metadata`): the Windows Terminal program `wt.exe` is an app execution alias, a reparse point that `metadata` can refuse with os error 1920; `symlink_metadata` sees it. Order, first hit wins:

1. The override in `[tools]` of `projects.toml`. If it is set and does not exist: `ConfiguredMissing(path)`, which is an error and not a fall-through.
2. Every folder of `PATH`, for each name of the table (names with and without the extension from `PATHEXT` limited to `.exe` and `.cmd`).
3. The known install paths of the table (environment variables expanded; a variable that is not set skips the entry).
4. `NotFound`.

| Kind | Names on `PATH` | Known install paths | Notes |
|---|---|---|---|
| editor | `Code.exe`; then `code.cmd`, mapped to `<dir of code.cmd>\..\Code.exe` (the `.cmd` lives in `...\bin`, the program one folder up) | `%LOCALAPPDATA%\Programs\Microsoft VS Code\Code.exe`, `%ProgramFiles%\Microsoft VS Code\Code.exe`, `%ProgramFiles(x86)%\Microsoft VS Code\Code.exe` | `code.cmd` itself is never started: it needs `cmd.exe` and shows a console |
| Windows Terminal | `wt.exe` | `%LOCALAPPDATA%\Microsoft\Windows\WindowsApps\wt.exe` | the alias is on `PATH` on a normal Windows 11; the fallback covers a trimmed `PATH` |
| Claude Code | `claude.exe`, `claude.cmd` | `%USERPROFILE%\.local\bin\claude.exe`, `%APPDATA%\npm\claude.cmd` | measured on the development PC: `%USERPROFILE%\.local\bin\claude.exe` |
| git | `git.exe` | `%ProgramFiles%\Git\cmd\git.exe`, `%ProgramFiles(x86)%\Git\cmd\git.exe`, `%LOCALAPPDATA%\Programs\Git\cmd\git.exe` | |

The result is not cached (a handful of `stat` calls, about 1 ms); the user installs a program and the next click works. The detected editor's file name feeds the open detection (6.4); when none is found the names `Code.exe` and `Code - Insiders.exe` are used.

### 6.6 From `projects.toml` to steps

`env::resolve(project, file) -> Vec<StepSpec>`:

1. Find the `[[project]]` entry by `path_key(path)` equal to the project's, else by `id`. Use its `steps`, else `[defaults].steps`, else `["editor", "claude"]`.
2. Each name becomes a step: a `[[step]]` of that name (a user-defined step replaces a built-in of the same name), else a built-in, else the name is skipped with a `warn`.
3. A step that fails validation (browser without a valid URL, `command` with a double quote, a bad `delay_ms`) is skipped with a `warn`; the source status `projects` gets the message `projects.toml: <n> step(s) ignored`.

`StepSpec { name, kind, label, icon, command, url, delay_ms, app, args }`. The `environment` of the `Project` JSON is this list mapped to `EnvStep` (`command` shows the `command`, or `claude` for a Claude step, or the `url`).

Autodetection from `package.json` or `Cargo.toml` is NOT done (the user decided the default environment; the mock presets were a prototype).

### 6.7 Opening a project

Called by section 01 through `LaunchPlanner::plan` and `finished` (6.9 of section 01); this module has no run task of its own.

**Plan.** `mode` selects steps: `full` or absent = all steps of the project's list (the default list `[editor, claude]` when the project has none); `editorOnly` = the steps of kind `editor` of that list; `terminal` and `claude` = the project's step of that kind when its list has one, else the built-in step of that name (`terminal`, `claude`), so "Terminal here" works with the default list, which has no `terminal` step. An empty plan is `400 invalid` `noSteps` with the message "This project has no editor step" (for `full` with an empty list: "This project has no steps"). `noSteps` can only come from `editorOnly` (no editor step in the list) or from an empty list: the `terminal` and `claude` modes fall back to the built-in step, so "no terminal/Claude step" never occurs. The project must be known (missing projects too): an unknown id is `404`. A layout changes the list and places the windows: see 6.17.

**Double click.** The engine owns it: section 01 6.8 step 3 returns the run that is still going for the same target before `plan` is called, so this module keeps no map of runs.

**Run.** This module implements `LaunchPlanner` (section 01 6.9); the engine of section 01 runs the steps. `plan(target)` reads caches only; the only I/O it may do is a few `symlink_metadata` calls of `tools::find` (6.5, at most one per program kind), never a process start, a scan or a git call (section 01 6.9). It returns the steps in order, each with its label, icon, `delay_ms`, and the action the engine runs:

```
if project.missing (from the cache of 6.2):     one step with preset_error "Folder not found: <path>" and stops_run = true; return
focus = (mode is full or absent) && project.open (the cache of 6.4, at most OPEN_TTL = 2 s old; the list fetch that precedes Enter refreshed it)
for each step spec of the plan:
    if focus && spec.kind is terminal or claude:  action = StepAction::Nothing            // already open: nothing is started again
    else: action = the command of the table below (editor: Spawn; terminal and claude: Spawn of wt.exe, with `PlannedStep.shell_fallback` = the same parameters through ShellExec (section 01 6.9); browser: ShellExec)
    error texts: PlannedStep.error_text = the texts of section 8.2 (section 01 6.9), set on every step this module plans
```

`finished(run)` (an async method: the engine awaits it before the final frame, so the SQL write `db.call(...).await` is finished before the frame goes out): when at least one step succeeded, write `projects_opened` and publish `projects` (6.4 needs `lastOpenedAt` before the UI refetches after the final frame). The delays, the step statuses and the frames belong to the engine (section 01 6.8). A failed step does not stop the later steps, except the missing folder. "Success" means "the program was started" (the same meaning as everywhere in Rain; no window is waited for).

**Focus.** An open project (an editor window exists) is not started again: the editor step is run again (VS Code, given a folder that is already open, brings that window forward; it is VS Code that moves its own window, which also avoids the Windows foreground lock that a windowless backend would hit), and the terminal and Claude steps are `StepAction::Nothing` and show success at once. The button label "Focus" (UI) matches. The other modes always start their steps.

**The steps** (all with the project folder as working folder; none inside a Job Object, none a child of a kill-on-close job, foundation C10):

| Kind | Command line (built by `launch.rs`, golden strings in tests) | Started by |
|---|---|---|
| editor | `"<editor exe>" "<folder>" <args>` | direct start through `platform::process::spawn_detached` (flags `DETACHED_PROCESS \| CREATE_NEW_PROCESS_GROUP \| CREATE_BREAKAWAY_FROM_JOB`, retry without breakaway on os error 5, null stdio; foundation 2.3). VS Code finds a window of the same folder itself, or opens a new one |
| terminal | `"<wt.exe>" -w 0 new-tab -d "<folder>" --title "<title>"` and, when the step has a `command`, ` cmd.exe /d /k "<command>"` at the end | direct start; if it fails (alias refused: Appendix C), `ShellExecuteW(open, wt.exe, <the same parameters>, <folder>)` |
| claude | `"<wt.exe>" -w 0 new-tab -d "<folder>" --title "<title>" cmd.exe /d /k ""<claude exe>" <args>"`: ONE tab in the folder; Claude Code runs inside a `cmd.exe` shell (the `cmd.exe /d /k` quoting rules of the terminal row apply), so when Claude Code exits the tab stays open as a normal shell in the folder (decisions.md, ruling of 2026-11-22) | as terminal |
| browser | the URL | `ShellExecuteW(open, <url>)` (default browser). Scheme `http` or `https` only |
| app | `"<app>" <args>` | direct start, working folder = the project folder |

Rules for the Windows Terminal lines: `-w 0` becomes `-w new` when `[terminal] window = "new"`; `-p "<profile>"` follows `new-tab` when a profile is set; `<title>` is the project name with control characters, `"` and `;` removed; every `;` in the text of a title or command is written `\;` (the semicolon is Windows Terminal's command separator); the folder is passed with no trailing backslash. All text is quoted by `launch::quote_arg` (the `CommandLineToArgvW` rules: wrap in quotes when it holds a space, tab or quote; a quote becomes `\"`; backslashes before a quote or at the end are doubled). The line is given to `Command::raw_arg` so Rust does not quote it again. The decision for a terminal step with a `command`: `cmd.exe /d /k` keeps the tab open after the command ends; it uses `cmd` whatever the user's default profile is (documented limit).

**One tab, one window call.** The default plan is `[editor, claude]`: the editor step, then ONE `wt.exe` call that opens ONE tab in the folder in which Claude Code runs. This is the user's wording in decisions.md section 4 taken literally ("a Windows Terminal tab in the folder, and Claude Code started in that terminal"). A shell-only tab exists only as the action "Terminal here" (mode `terminal`) or when a user list names a `terminal` step. When a user list holds a `terminal` step directly followed by a `claude` step, the plan holds ONE combined step (label "<terminal label> + <claude label>") that is started by ONE `wt.exe` call with a `;` between the two `new-tab` parts, so that both tabs land in the same window even when no Terminal window exists yet (two separate `wt -w 0` calls could race and open two windows). The ruling of 2026-11-22 in decisions.md settles this: ONE tab, and when Claude Code exits the tab stays open as a normal shell in that folder (which is why the Claude step runs `claude` inside `cmd.exe /d /k`, not as the tab's own program).

**Step labels and icons** are those of the `StepSpec` (6.6). The `LaunchRun.steps` has one entry per plan step.

**Not found.** A missing program fails only its own step (messages in section 8). A missing `claude` does not stop the editor or the terminal.

**After the run:** `record_opened`, then `projects` is published (the open flag and the time changed), then the final frame. The UI's refetch (`useLaunch.ts:89`) then sees the new `lastOpenedAt`; `open` may still be false for a second, because the window needs time to appear (the UI hides Rain at once after a launch; the next summon refetches).

### 6.8 The game library: how `GET /games` is assembled

```
settings = settings.resolved()          // Resolved of section 06
games.toml = TomlFile.get()
steam  = steam::scan(settings)            -> SourceResult { state, message, items }   // 6.9
epic   = epic::scan(settings)             -> SourceResult                              // 6.10
local  = local::scan(settings, games.toml)-> SourceResult                              // 6.11 + 6.12
all    = steam.items + epic.items + local.items
apply games.toml [[game]] entries by key: hidden, and (local only) title, exe, cover, args
fill: pinned (launcher pins), running (6.13), playtime and lastPlayedAt (6.13), sizeBytes (cache, 6.13), coverUrl
sort: source order steam, epic, custom; then title case-insensitive; then id   (the UI re-sorts; ties keep this)
sources = [steam, epic, custom] as GameSourceStatus; each also reported to Health.sources
```

The three scans are independent and run in parallel on the blocking pool. A scan that fails gives `failed` for its source and no items; the other sources are still returned. `unavailable` (503) is returned only when the module itself did not start. In-memory cache: the whole assembled list is reused for 2 s (`LIBRARY_TTL`, checked at read time) so a burst of fetches (Home, Games, Game mode) costs one scan; `running`, `pinned` and `hidden` are applied on top at every answer because they are cheap and change often. `POST /games/refresh` bypasses the cache.

Hidden games are included with `hidden = true` (the UI drops them from every view except the new Hidden tab). Games of an `off` source are not included.

### 6.9 Steam

Copy from the spike (winapi): `src/vdf.rs` (the 90-line parser, 333 microseconds on a 112 KB file), `src/bin/steam.rs`, `src/bin/registry.rs`. Registry access goes through `trait SteamRegistry` (production `winreg` 0.56; fake in tests).

**1. Find Steam.**

| Step | Rule |
|---|---|
| a | `steamEnabled = false` gives state `off`, message "Turned off in Settings", no items |
| b | `Resolved.steam_path` is `Some`: use it if the folder exists; if not, state `failed`, message `Steam folder not found: <path>` (a wrong setting is shown) |
| c | `Resolved.steam_path` is `None` (section 06 6.1.3 found no override and no Steam in the registry): state `off`, message "Steam is not installed" (not a failure). This section runs no detection of its own |

**2. Libraries.** Parse `<steam>\steamapps\libraryfolders.vdf`: every numeric child of `libraryfolders` has a `path`. The libraries are `<steam>` itself plus each `path`, normalised, de-duplicated by `path_key`. A library whose folder does not exist (a drive that is not plugged in) is skipped; the message becomes "<n> library folder(s) not available" with state `ok`. If the file cannot be read, only `<steam>` is used.

**3. Installed games.** For each library read `steamapps\appmanifest_*.acf` (each file at most 256 KB; a file that fails to open is retried twice, 50 ms apart with `tokio::time::sleep`, because Steam rewrites them). Parse with the VDF parser; root key `AppState`:

| Field | Use |
|---|---|
| `appid` | digits only, else skip the file |
| `name` | title; empty gives skip |
| `installdir` | folder under `steamapps\common`; `installPath = <library>\steamapps\common\<installdir>` |
| `StateFlags` | decimal; `flags & 4 != 0` means fully installed. 2 (update needed), 256, 512 and 1024 may be set at the same time and the game is still launchable. A value without bit 4 (first download: 1026) is skipped |
| `SizeOnDisk` | bytes, becomes `sizeBytes` (this is Steam's own figure; no folder walk) |
| `LastPlayed`, `LastUpdated` | Unix seconds as text; `"0"` or missing = never |

Tool entries are dropped: appids 228980 (Steamworks Common Redistributables, seen on the development PC), 1007 (Steam Client Service), 1070560, 1391110, 1628350 (Steam Linux Runtime and its soldier and sniper runtimes), and any name that starts with `Steamworks Common`, `Steam Linux Runtime`, `Proton ` or `Steam Client`. The ids other than 228980 are from memory (UNVERIFIED); the name rule is the safety net, and the user can hide anything else.

**4. Play time.** From `userdata\<account id>\config\localconfig.vdf`, path `UserLocalConfigStore / Software / Valve / Steam / apps / <appid>` (case-insensitive lookup), values `Playtime` (minutes, text) and `LastPlayed` (seconds). `lastPlayedAt` = the largest of the manifest value, the localconfig value and Rain's own record (a launch from Rain shows at once in "Recent"; Steam writes its own values only at exit). Which account:

| Order | Rule | Status |
|---|---|---|
| 1 | HKCU `Software\Valve\Steam\ActiveProcess` value `ActiveUser`, when not 0 | proven (spike) |
| 2 | `AutoLoginUser` (HKCU `Software\Valve\Steam`) is an account name; find the child of `config\loginusers.vdf` whose `AccountName` equals it; its section name is a 64-bit Steam id; account id = that id minus 76561197960265728 | UNPROVEN |
| 3 | the child of `loginusers.vdf` with `MostRecent` = `1` (same conversion) | UNPROVEN |
| 4 | the numeric folder of `userdata` whose `config\localconfig.vdf` has the newest modified time | UNPROVEN |
| none | no play time: every `playtimeMinutes` is 0; state `ok`, message "Play time is not available (Steam is signed out)" | |

Account names are never logged or stored. The file is read whole (limit 4 MiB), parsed in memory and dropped.

**5. Covers (local only).** `<steam>\appcache\librarycache\<appid>\`: first `library_600x900.jpg` in that folder; else the same name one folder down (any child folder, the new layout uses a 40-hex folder name); else `library_capsule.jpg` one folder down; else `library_capsule.jpg` in the folder; else no cover (letter tile). `header.jpg` (landscape 460 x 215) is NOT used: it crops badly in the 2:3 tile. The spike found a portrait file for 1 of 2 installed games; the rest get the title tile. No network request is made. The chosen path is kept in the library state; the media provider (6.12) serves it by game id.

**6. Signature and cache.** `sig = mtime+len of libraryfolders.vdf, of each library's steamapps folder, and of localconfig.vdf, plus steamEnabled, steamPath and the registry values read`. A repeated scan with the same signature returns the same items (a scan costs about 5 to 10 ms, so there is no SQLite cache).

**7. Running.** HKCU `Software\Valve\Steam` value `RunningAppID` (DWORD, one registry read) equal to the appid, or a process whose image lies inside `installPath` (6.13).

**Failure rules.** `libraryfolders.vdf` unreadable and no manifests anywhere: `failed`, message "Could not read the Steam library". One manifest that does not parse: skipped, one `warn` with the file name. Every manifest failed: `failed`.

### 6.10 Epic Games

There are two launchers, and a PC can have either or both. This subsection is the OLD launcher (`*.item` manifests); 6.10.1 is the NEW one. The old launcher was never seen on the owner's PC: everything about it here is UNVERIFIED and defensive, and its facts come from general knowledge of the manifest files. The new launcher was read on the owner's PC (6.10.1). One game found in both formats is one game (same `AppName`), and the new format wins.

**Find.**

| Rule | Result |
|---|---|
| `epicEnabled = false` | `off`, "Turned off in Settings" |
| `Resolved.epic_path` is `Some` (the `EpicGamesLauncher` folder, override or detected by section 06 6.1.4): the manifests are searched at `<path>`, `<path>\Manifests` and `<path>\Data\Manifests`, the first that exists | none of them exists: `failed`, `Epic folder not found: <path>` |
| `Resolved.epic_path` is `None` (an empty value means the source is off; section 06 is the only detector) | `off`, "Epic Games is not installed" |
| a folder (`epic_path`, and `epic_extra_path` when set) holds `installations.jsonl` | it is read as the new format (6.10.1), besides any manifests it also has; it needs no manifests |

**Read.** Every `*.item` file in the folder (not recursive), at most 1 MiB each, read as bytes, a UTF-8 BOM removed, parsed with `serde_json::Value` (not a typed struct: every field is optional). Fields used: `DisplayName`, `AppName`, `InstallLocation`, `InstallSize`, `LaunchExecutable`, `bIsIncompleteInstall`, `MainGameAppName`, `AppCategories`. A game is listed when all of these hold:

1. `DisplayName` and `AppName` are non-empty strings; `AppName` matches `^[A-Za-z0-9_.-]{1,128}$` (it goes into a URL).
2. `bIsIncompleteInstall` is not `true`.
3. `MainGameAppName` is absent or equals `AppName` (this drops add-ons and DLC entries).
4. `AppCategories` is absent, or contains an entry equal to `games` (case-insensitive) (this drops the launcher's own entries).
5. `InstallLocation` is a non-empty string that exists as a folder.

The game is `{ key: epic:<AppName>, title: DisplayName, installPath: InstallLocation, sizeBytes: InstallSize when it is a non-negative number, no cover, playtime 0 }`. A file that does not parse or fails a rule is skipped with one `warn` (file name only). If the folder holds at least one `*.item` file and every one failed to parse as JSON, the source is `failed` with the message "Could not read the Epic Games library" (this is how a changed file format shows). An empty folder is `ok`.

`lastPlayedAt` and the play time exist only from Rain's own record of launches made through Rain (the Epic manifests have none). Epic games the user starts outside Rain show no last-played time.

**Running.** A process whose image lies inside `InstallLocation`. **Launch**: 6.14.

#### 6.10.1 The new Epic Games Launcher ("eglv2")

Read on the owner's PC (read-only), 2026-11-22. That PC has no `.item` manifests and no `C:\ProgramData\Epic`; the new launcher keeps everything per user. Nothing personal is in the repository: fixtures use invented games and ids.

**Find.** Section 06 6.1.4 detects the data folder: the registry value `HKCU\Software\Epic Games\EpicGamesLauncher` `AppDataPath` (a `%NAME%` in it is expanded), else `%LOCALAPPDATA%\com.epicgames.eglv2` (from Windows at run time). A folder counts only when it holds `installations.jsonl`. When a PC has both launchers, `epicPath` shows the new data folder (the one in use) and `Resolved.epic_extra_path` carries the old folder, so both are read; a typed `epicPath` is read alone. Settings shows the path that is in use.

**Read `installations.jsonl`.** A journal: one JSON object per line, the first line a header (`version`, `writtenAt`, `entries`; skipped). The same installation appears several times (lines are appended while a game installs or updates): the LAST record of an `installationId` is its current state. At most 4 MiB are read (a larger file makes the source `failed`: "installations.jsonl is too large to read") and at most 20 000 lines. Each line is parsed as a plain JSON value; keys that are not listed are ignored; a line that is not a JSON object or has no `installationId` is skipped, counted and logged once (a count, no content). A game is listed when its last record has `complete = true`, `platform` is "Windows" or absent, `namespace`, `itemId` and `artifactId` match `^[A-Za-z0-9_.-]{1,128}$` (they go into a launch address and into `games.toml`), and `dir` is a folder that exists. A record whose folder is gone is "not installed", as a stale manifest is. `installations.jsonl.prev` is not used.

**The game.** `key = epic:<artifactId>` and `id = game-epic-<slug(artifactId)>` (the same as the old format, so a correction in `games.toml` such as `hidden` works for both), `installPath = dir`.

| What | Where (all below the data folder, all optional) | Without it |
|---|---|---|
| title | `cache\prod\account\<account>\library-cache.json`: the first account folder (by name, at most 16) whose `items[]` has the same `namespace` + `itemId` + `artifactId` (`appId.namespaceId`, `appId.itemId`, `appId.appName`, else `relationshipId`) gives `title` | `artifactId` |
| cover | `cache\prod\catalog-cache.json`: `items[<itemId>].item.keyImages[]`; a key image counts only when `images\<md5>.jpg` (also `.jpeg`, `.png`, `.webp`; `md5` is 32 or 40 hex characters, checked before it is part of a path) exists and is at most 8 MiB. Portrait images (taller than wide) come before landscape ones, then the larger. It is served through the usual `game-cover` media route. Nothing is downloaded | the letter tile |
| size | `cache\disk-size-cache.json`: `entries[].bytes` of the same three ids (written as a float) | none; Rain does not measure Epic folders (as for the old format) |
| play time | `cache\prod\account\<account>\playtime-cache.json`: `items[].totalTime` seconds of `artifactId`, in whole minutes; with several accounts the largest | Rain's own record (nothing counts for a game that the launcher starts) |
| last played | `last-played.json`: `{<account>: {"<namespace>:<itemId>:<artifactId>": <RFC 3339 time>}}`, the latest of any account | Rain's own record |

Play time and last played are Epic's own numbers, the way Steam's own numbers are used for Steam games (6.9): `playtimeMinutes` of an Epic game is the larger of Epic's number and Rain's record, and `lastPlayedAt` is the later of Epic's time and Rain's record. Every cache file has the size limit of 8 MiB; a file that is missing, too large or does not parse only means "no title/cover/size/time", never a failed source.

**Launch.** Through the launcher, never the game's own program (the launcher handles sign-in and anti-cheat): `ShellExecuteW(open, "com.epicgames.launcher://apps/<namespace>%3A<itemId>%3A<artifactId>?action=launch&silent=true")`. The old format keeps `apps/<AppName>`. One function (`epic::launch_url`) builds both.

**Health.** `ok` with games from either format; `off` ("Epic Games is not installed") when neither launcher exists; `failed` when no folder could be read: a journal that is not readable, is too large, or has lines and not one of them JSON gives "Could not read installations.jsonl" (the file is named, the path is not). When one of two folders works and the other does not, the games show and the source is `ok` with the sentence of the one that failed.

**Watching.** Not watched: the module never watched the old manifests either. The library is scanned when a request needs a fresh one (cached for 2 seconds), at `POST /games/refresh`, and after a change of the settings.

**Not done.** The `.prev` copy of the journal is not read when the main file cannot be read. Add-ons or DLC are not told from games in the journal (it has no category); an add-on that has its own installation record would be listed.

#### 6.10.2 Riot Games

Riot's own client lists what it has installed. Rain reads two things from the Riot Games data folder (`riotPath`, else `%ProgramData%\Riot Games`, found by section 06) and writes nothing there.

| File | What Rain takes from it |
|---|---|
| `RiotClientInstalls.json` | the client program: `rc_default`, else `rc_live` |
| `Metadata\<product>.<patchline>\` | one folder per installed product (`league_of_legends.live`, `teamfighttactics.live`). Folders without a `.` in the name (`Riot Client`, `RiotGamesApi`) are not games |
| `<name>.product_settings.yaml` | `product_install_full_path` and `shortcut_name`. A small reader takes the top-level `key: value` lines (quoted or not); the nested blocks are skipped and there is no YAML dependency |
| `<name>.ico` | the picture (see Cover) |

**Game.** One game per product whose install folder exists. Title: `shortcut_name` without `.lnk`, else the product name as words. Key `riot:<name>`, id `game-riot-<slug of name>` (for example `game-riot-league-of-legends-live`). The key works in `games.toml` like the Epic keys, so a Riot game can be hidden.

**Launch.** `ShellExecuteExW` of the client program, in its own folder, with `--launch-product=<product> --launch-patchline=<patchline>`, the same as Riot's Start menu shortcuts. Both parts are plain characters (checked). The client starts the game, so Rain watches no process and counts no play time.

**Running.** A Riot game runs when a process runs from inside its own install folder (6.13). `RiotClientServices.exe` never counts, wherever it lies, because it runs in the background most of the time. Teamfight Tactics runs through the League of Legends client and has no process of its own, so it is never shown as running.

**Size and cover.** The size is the install folder's, measured in the background and cached like the size of a game folder (6.13). The icon holds several sizes. The media route `game-cover` takes the largest picture, turns a 32-bit bitmap into a PNG (a PNG picture is used as it is) and serves it as the cover, so the Games screen shows no blank tile.

**Health.** `off` when Riot is not installed ("Riot Games is not installed") or switched off. `failed` when the folder is missing, or when games exist and the client cannot be found ("Could not read RiotClientInstalls.json"), or when every product fails ("Could not read the Riot Games library"). One broken yaml hides that product only; the source stays `ok` and says which one. A product whose install folder is gone is simply not listed. The health name is `riot`.

### 6.11 Game folders: discovery and the program-file guess

**Discovery.** For each entry of `gameFolders` (normalised, de-duplicated):

```
if the folder does not exist or cannot be read: record "Game folder not found: <path>" (the source becomes failed), continue
for each direct entry E of the folder:
    skip if E is not a directory (metadata() follows links)            // loose files
    skip if name starts with "_" or "." or "$"                          // _CommonRedist, hidden, recycle bin
    skip if name (case-insensitive) is "System Volume Information"
    skip if path_key(E) equals or lies inside a Steam library's steamapps folder (Steam lists these itself)
    skip if path_key(E) equals an Epic InstallLocation
    skip if the folder holds no file at all within 3 levels             // an empty shell is not a game
    folder game = { key: dir:<norm_path(E)>, title: name, installPath: norm_path(E) }
```

The whole discovery of one game folder reads only the direct entries (cheap). The guess and the size are separate and cached.

**The guess.** Goal: pick the program a person would start. It must not be "the largest `.exe`". The function is pure: `guess::guess(folder_name, &[FileEntry { rel: String, size: u64 }]) -> Option<Guess { rel, score }>`; a separate walker produces the file list from disk.

*Walker* (`folders.rs`): breadth-first from the game folder, only files with the extension `.exe` (case-insensitive) are recorded, with their size. Limits: depth 5 (a file directly in the game folder is depth 0; `<game>\<sub>\bin\x64\<name>.exe` is depth 3), at most 1500 DIRECTORIES entered per game (`GUESS_MAX_DIRS`; files are listed to find `.exe` names but are NOT counted, so a `Data` folder with 8000 loose files cannot use the budget up; when the limit is reached the walk stops and the file list so far is used), reparse points are not followed. At depth 3 and deeper only folders whose lower-cased name is in the program-folder list `bin`, `binaries`, `win64`, `x64`, `x86`, `game`, `release`, `shipping` are entered, and at every depth these folders are expanded first (they go to the front of the queue). Test: a `guess` fixture with 8000 loose files in `Data\` and the program at depth 3 under `<sub>\bin\x64\` must still choose that program. A folder is not entered when its name (case-insensitive) is one of these, because it holds data, runtimes or redistributables and never the game's program: starts with `_` or `.`; `redist`, `redistributable`, `redistributables`, `directx`, `vcredist`, `dotnet`, `dotnetfx`, `engine` (Unreal engine files), `content`, `paks`, `saves`, `savegames`, `logs`, `crashes`, `crashdumps`, `shadercache`, `movies`, `videos`, `sound`, `sounds`, `music`, `audio`, `textures`, `localization`, `locales`, `streamingassets`, `monobleedingedge`, `node_modules`, `__installer`; ends with `_data` (Unity data folders) or `_backupthisfolder_butdontshipit`. Folders named `tools`, `mods` and similar are entered and penalised below (a game that only has its program there still resolves).

*Name parts.* `stem` = the file name without `.exe`. `tokens(s)`: split at every non-alphanumeric character, at every lower-to-upper letter change (`CrashReportClient` gives crash, report, client) and at every letter-digit change (`unins000` gives unins, 000); lower-case. `compact(s)` = lower-case alphanumerics only. `core(stem)` = `compact` of the stem after removing the tokens `x64`, `x86`, `win64`, `win32`, `64`, `32`, `shipping`, `release`, `final`, `retail`.

*Step 1, hard drops (never a candidate).* The `compact(stem)` contains one of: `uninstall`, `uninst`, `vcredist`, `vcruntime`, `dxsetup`, `dxwebsetup`, `oalinst`, `crashreport`, `crashhandler`, `crashpad`, `crashsender`, `crashdump`, `bugreport`, `bugsplat`, `errorreport`, `werfault`, `breakpad`, `installer`, `dotnetfx`, `windowsdesktopruntime`, `redist`; or a token equals `setup`, `install`, `unins` or `instmsi`. (The word "crash" alone is NOT enough: `CrashBandicoot.exe` is a game.)

*Step 2, the score of each remaining candidate.*

```
score = 100 * S  -  6 * depth  +  size_bonus  +  arch_bonus  +  name_penalty  +  folder_penalty
```

| Term | Rule |
|---|---|
| `S` (0 to 1), resemblance of `core(stem)` to `compact(folder name)` | 1.0 when equal; 0.85 when one contains the other and the shorter has at least 4 characters; else the larger of `0.8 * jaccard` (token sets of the two names, without the filler tokens `the of and edition goty deluxe definitive complete remastered game hd`) and `0.7 * (common prefix length / longer length)` |
| `depth` | number of folders between the game folder and the file |
| `size_bonus` | `min(6, 2 * log10(size in MB + 1))`: a tie-breaker, never a driver (1 GB gives 6, 30 MB gives 3) |
| `arch_bonus` | +3 when the stem has a token `x64`, `win64` or `64`, or a folder on the path is named `x64`, `win64` or `bin64`; -3 for `x86`, `win32`, `32` in the same places; both present: 0 |
| `name_penalty` | the worst single value (not the sum) of the tokens and compacts of the stem: `launcher` -50; `updater`, `patcher`, `autoupdate` -60; `config`, `configurator`, `settings`, `options` -40; `editor`, `modtool`, `modkit`, `mod`, `sdk`, `tool`, `tools` -50; `server`, `dedicated` -50; `benchmark` -30; `viewer`, `artbook`, `soundtrack`, `manual`, `credits`, `trailer` -50; `helper`, `service`, `daemon`, `subprocess`, `webhelper`, `cef` -60; `eac`, `easyanticheat`, `battleye`, `beservice`, `anticheat` -80; `register`, `activation`, `uplay`, `origin` -60 |
| `folder_penalty` | -30 for each distinct folder on the path (below the game folder) named, case-insensitive: `tools`, `tool`, `support`, `installers`, `installer`, `setup`, `prerequisites`, `prereq`, `artbook`, `soundtrack`, `ost`, `manual`, `docs`, `documentation`, `extras`, `bonus`, `mods`, `modding`, `modkit`, `sdk`, `launcher`, `crashreporter`, `thirdparty`, `plugins`, `server`, `servers`; at most -60 in total |

*Step 3, choice.* The highest score wins. A tie is decided by smaller `depth`, then larger size, then the path in ascending order, so the result never changes between runs. No candidate gives `None` (the game has no program file; the UI offers "Choose program"). A candidate with a bad score is still chosen when it is the only one (a folder with only `launcher.exe` gives that file), because a wrong guess can be corrected and a missing one cannot be used.

Worked scores (approximate; the tests assert the winner, not the number). Notation `folder / file (MB)`.

| Layout | Candidates and scores | Winner |
|---|---|---|
| `Fixture Quest` (the user's nested `Game\<name>.exe` shape, with every kind of decoy) | `Game\FixtureQuest.exe` (180): 100 - 6 + 4.5 = 98.5. `ArtBook\FixtureQuestArtBook.exe` (350): 85 - 6 + 5 - 50 - 30 = 4. `tools\FixtureQuestEditor.exe` (300): 85 - 6 + 5 - 50 - 30 = 4. `tools\FixtureQuest.exe` (320, same name as the game, bigger, shallower than a deep game): 100 - 6 + 5 - 30 = 69. `Game\Launcher.exe` (90): -52. `Game\CrashReportClient.exe`, `unins000.exe`, `_CommonRedist\vcredist_x64.exe`, `_CommonRedist\DirectX\DXSETUP.exe`: dropped or never entered | `Game\FixtureQuest.exe` |
| `Fixture Arena` (the user's `<game>\<sub>\bin\x64\<name>.exe` shape, depth 3) | `FA_Main\bin\x64\FixtureArena.exe` (220): 100 - 18 + 4.7 + 3 = 89.7. `FA_Main\bin\x86\FixtureArena.exe` (100): 100 - 18 + 4 - 3 = 83. `FA_Main\bin\x64\FixtureArena_dedicated_server.exe` (400): 85 - 18 + 5 + 3 - 50 = 25. `Tools\MapEditor.exe`: negative. `...\FixtureArenaCrashHandler.exe`: dropped | the x64 `FixtureArena.exe` |
| `Fixture Realm` (Unreal-like: a small root program and a big deep one) | `Realm.exe` (1): 85 + 0.6 = 85.6. `Realm\Binaries\Win64\Realm-Win64-Shipping.exe` (150): 85 - 18 + 4.4 + 3 = 74.4. `Engine\...` never entered | `Realm.exe` (the root program starts the deep one) |
| `Ancient Thing` (nothing resembles the folder name) | `bin\run.exe` (30): -6 + 3 = -3. `bin\tools\unpack.exe` (5): -12 + 1.5 - 30 = -40.5. `bin\config.exe` (2): -6 + 1 - 40 = -45 | `bin\run.exe` |
| `Old Game` (only a launcher) | `launcher.exe`: -50 + small | `launcher.exe` |
| `Crash Bandicoot` | `CrashBandicoot.exe` kept (compact equals the folder), `CrashHandler.exe` dropped | `CrashBandicoot.exe` |

**Cache.** `games_guess_cache` keyed by `folder_key`. `signature` = `fnv8` of the text `<mtime of the folder>|<entry count>|<name and mtime of each direct subfolder, up to 50>`. The cached `exe_rel` is used when the signature is equal, the cache row is younger than 30 days, and the file still exists (one `stat`); otherwise the guess runs again. A game update that replaces the program in a nested folder keeps its path, so the cached answer stays valid; a game that moved its program changes the existence check.

**Cold cost.** About 10 to 30 ms per game folder (a bounded walk); the folders are guessed in parallel on the blocking pool (4 at once) under `GUESS_BUDGET` = 5 s for one `GET /games`. A folder not finished in time is listed with no `exePath` for now and finishes in the background (publishes `games`).

### 6.12 Local games: one model, corrections, hand-added games, covers

```rust
struct LocalGame {
    key: String, id: String, title: String,
    folder: Option<PathBuf>,          // folder game: its folder; hand-added: None
    exe: Option<PathBuf>,             // the program file in use
    exe_guessed: bool,
    args: Option<String>, cover: Option<CoverRef>, hidden: bool,
    origin: Origin,                   // Folder or Manual
}
```

Assembly: a folder game starts from discovery (6.11); its `[[game]]` entry (key `dir:...`) overrides `title`, `exe`, `args`, `cover`, `hidden`. A hand-added game comes from a `[[custom]]` entry. The program file in use is: the corrected `exe` (resolved against the folder when relative) if the entry has one, else the guess. `installed` = the program file exists (a folder game with no program file at all is `installed = true` and has no `exePath`; a corrected or hand-added file that does not exist gives `installed = false`, and the guess is NOT used behind the user's back).

**`saveCustomGame(draft)`** validation, in this order (the first failure is returned):

| Check | Failure |
|---|---|
| `title.trim()` empty | 400 `noName` |
| `exePath.trim()` empty | 400 `noExe` |
| not absolute (`X:\...` or `\\...`) | 400 `exeNotAbsolute` |
| not an existing file | 400 `exeNotFound` |
| extension is not `.exe` | 400 `notAnExe` |
| lies inside a folder game (`path_key` prefix of a discovered game folder) | 409 `alreadyListed`, message names the game and says "(hidden)" when it is |
| the same program file as another `[[custom]]` entry | 409 `alreadyListed` |
| `coverPath` given and not an existing file | 400 `coverNotFound` |
| cover not a PNG, JPEG, WEBP or GIF (extension and first bytes: `89 50 4E 47`, `FF D8 FF`, `RIFF....WEBP`, `GIF8`) | 400 `coverNotImage` |
| cover larger than 8 MiB (`COVER_MAX`) | 400 `coverTooLarge` |

Then: id = `game-<slug(title)>` made unique with `-2`, `-3` among ALL current game ids; the cover is imported (below); one `edit()` of `games.toml` appends the `[[custom]]` entry; the library is rebuilt; the new `Game` is returned with `201`. A failure of the write is `500 saveFailed` and the imported cover file is removed again.

**`PATCH /games/{id}`** (`GamePatch`):

| Field | Steam, Epic | Folder game | Hand-added game |
|---|---|---|---|
| `hidden` | yes | yes | yes |
| `title` | `400 notEditable` | yes; `""` = folder name again | yes; `""` is `400 noName` |
| `exePath` | `400 notEditable` | yes, same checks as above; `""` = remove the correction, use the guess | yes, same checks; `""` is `400 noExe` |
| `coverPath` | `400 notEditable` | yes; `""` = remove the cover | yes; `""` = remove |

An empty patch is `400 emptyPatch`. An unknown id is `404 unknownId`. When `hidden` becomes `true` the game's pin is removed in the same call (`launcher.unpin(PinnableType::Game, id)`). A hand-added game's `exePath` may not lie inside a folder game either (same `alreadyListed`). One `edit()` writes the change; the `[[game]]` entry of a folder game is created when needed and removed when every field is back to default. The answer is the updated `Game`.

**`DELETE /games/{id}`**: only for a hand-added game: removes the `[[custom]]` entry, its cover file, its pin, and its `games_play` row stays (history is kept). Any other game is `409 notRemovable`; an unknown id is `204`.

**Covers.** `Game.coverUrl` is `media/game-cover/<game id>?v=<hex>`, present only when a cover file exists. The provider `GameCovers` is registered once: `state.media.register("game-cover", ...)`. It maps the id to a path from the current library state (never from the URL, so no path can be asked for): Steam art path (6.9), the user's cover, or an image found in the game folder. The order for a local game: the user's `cover`; else the first existing file in the game folder (top level only) named, case-insensitive, one of `cover`, `folder`, `poster`, `boxart`, `box`, `library_600x900` with extension `png`, `jpg`, `jpeg` or `webp`; else none. `v` = hex of the file's modified time in seconds and its length. `Content-Type` from the extension. A file that has vanished since the list gives a 404 (the tile falls back to the title). The provider reads on a blocking thread and answers `ETag` and `304` before reading the bytes (foundation 4.7; copy `icons.rs` of the types-stream spike). Hand-set cover files are copied by `covers::import(id, path)`: read at most 8 MiB, check the type, name `<id>-<fnv8 of the content>.<ext>`, write to `<data>\media\` through a temp file and rename; no resizing (no image crate is added; `object-cover` crops).

Games without any local art get the title tile of the existing UI (`GameTile.vue:124`). Looking art up online is not done this round (foundation 13.2).

### 6.13 Size, play time, last played, running

**Size** (`sizeBytes`). Steam: `SizeOnDisk` of the manifest. Epic: `InstallSize` when present. Folder game: the logical size (sum of file lengths) of the folder, measured by `size::measure(folder)`: `read_dir` and `metadata` recursively, one thread, reparse points not followed, unreadable subfolders skipped; budget 2,000,000 entries; cooperative cancel flag checked per folder. It runs in the background: when `GET /games` finds a folder game whose `games_size_cache` row is missing, or has a different signature (`fnv8` of the folder's mtime and direct entry count), or is older than 7 days, it adds the folder to a queue; one task takes the queue one folder at a time (`core::blocking::run`), stores the row, and publishes `games` after each folder. The first answer for a new game has no `sizeBytes`; the UI hides the "Size" row until it arrives (`GameDetail.vue:63` already does). Hand-added games have no folder and no size. In game mode nothing is measured and the queue is dropped.

**Play time and last played.** Steam: 6.9. Everything else: Rain's own record (`games_play`): `playtimeMinutes = play_seconds / 60` (floor), `lastPlayedAt = last_played_at`. A launch (6.14) sets `last_played_at = now` and `launches += 1` before the final run frame. All timestamps are `YYYY-MM-DDTHH:MM:SS.mmmZ` (the Games screen compares them as text).

**Sessions** (how `play_seconds` grows). When Rain starts a local game by a direct start (6.14) the engine of section 01 does the start and calls `PlannedStep.on_spawned(pid, creation_time)` (6.14; the module never receives a child or a wait future). The callback inserts the `games_session` row and starts one watcher task per running game on `platform::process::open_for_wait(pid, creation_time)` (foundation 11.6: it returns an `ExitWatcher`, the same type as section 03 6.3.6, built on `RegisterWaitForSingleObject`: event driven, the operating system signals the exit; no thread is parked, no blocking-pool slot is used, no timer, no polling; the same call that the session recovery uses). When the process ends the watcher:

1. takes a process snapshot once (`ProcessLister`) and looks for other processes whose image lies inside the game's folder and that were not running before the launch (a launcher stub that starts the real game and exits would otherwise record a few seconds); if any exist it waits for all of them the same way (up to 3 rounds);
2. adds `ended - started_at` (wall clock, floor to seconds) to `games_play.play_seconds`, deletes the session row, publishes `games`.

Backend stops while the game runs (the game is detached and keeps running; game mode does exactly this): at `stop()` the module writes `last_seen_at = now` into every open `games_session` row. At the next start the module checks each open row: the process still exists and its creation time equals `process_start` (`OpenProcess` + `GetProcessTimes`): a new watcher is attached (`open_for_wait`, an `ExitWatcher`: no thread, event driven); it is gone: the time `last_seen_at - started_at` is added (a lower bound) and the row deleted. After a crash (no `stop()` write) `last_seen_at` is the launch time, so only the launch counts: documented limit, nothing is invented.

Steam and Epic games started through Rain are not watched (the process belongs to the launcher; Steam keeps its own play time). A game that needed administrator rights (os error 740) is started through `ShellExecuteW` and cannot be watched either: only the launch is recorded.

**`running`.** Computed at answer time from one process snapshot (`ProcessLister`, 4.7 ms for 262 processes) and one registry read, cached 2 s (`RUNNING_TTL`). A game is running when any of these hold: a Rain-watched session exists for it; Steam only: `RunningAppID` equals its appid; any process image path lies inside the game's install folder (`installPath`; for a hand-added game, the folder of its program file) compared with `path_key` and a trailing `\`. Protected processes whose image path cannot be read do not count. There is no timer: `running` is right at each fetch, a watched session ends push a `games` frame, and the screens refetch on show. A Steam game that ends while the window stays open on the Games screen keeps its ring until the next fetch (accepted, section 12).

### 6.14 Launching a game

Implemented as `LaunchPlanner::plan` for target type `game` (section 01 6.9); `plan` returns one step, label = the title (so the failure notice reads `<title>: <error>`), icon = the cover as `IconRef { kind: 'url', value: 'media/game-cover/...' }` when there is one, else lucide `gamepad-2`. A run of the same game that is still going is returned instead of starting a second (the engine, 200). A game that Rain itself started and that still runs (a watched session) is not started again: the step is `success` at once, nothing is spawned.

| Source | Start | Error cases |
|---|---|---|
| Steam | `ShellExecuteW(open, "steam://rungameid/<appid>")`; the appid is digits only (checked) | Steam `off` ("Steam is not installed"); `ShellExecuteW` result <= 32: 31 (no handler) "Steam is not installed", other values "Could not hand the game to Steam" |
| Epic | `ShellExecuteW(open, "com.epicgames.launcher://apps/<AppName>?action=launch&silent=true")`; `AppName` matched the charset of 6.10; for a game of the new launcher `<AppName>` is `<namespace>%3A<itemId>%3A<AppName>` (6.10.1) | Epic `off` ("Epic Games is not installed"); result 31 "Epic Games Launcher is not installed" |
| Local | step action `StepAction::Spawn { exe, args_raw, cwd }` (a direct start of the program file through `platform::process::spawn_detached(exe, raw args, working folder = the program's own folder)`; the planned step carries `on_spawned(pid, creation_time)`, which inserts the session row and starts the process wait for the play time, 6.13): flags `DETACHED_PROCESS \| CREATE_NEW_PROCESS_GROUP \| CREATE_BREAKAWAY_FROM_JOB`, retry without breakaway on os error 5, null stdio, fire and forget. Command line: `"<exe>" <args>` | no program file: "No program file. Choose one with Edit"; file missing: `Program file not found: <path>`; os error 740 (needs administrator rights): the planned step carries `shell_fallback { file: exe, params: args, dir: folder }` (section 01 6.9) and the engine then starts it through `ShellExecuteW(open, exe, args, folder)` instead (a UAC prompt appears; not watched); any other error: `Could not start the program` |

Success means "handed over or started". In `finished(run)` (awaited by the engine, so the write is done before the final frame) the module then writes `games_play` (launch time and count; for a local game the `games_session` row and the watcher were already made by `on_spawned`), and publishes `games` before the final run frame (the UI refetches after it, `useLaunch.ts:89`).

A launched game is never in a Job Object of Rain (C10; the backend is not in one either). The URI handlers start Steam or the Epic launcher when they are closed; Rain cannot know whether the game really started (it may ask the user to log in or update): this is the same for every launcher and is stated, not hidden.

**Inside a job of the caller.** `spawn_detached` first tries `CREATE_BREAKAWAY_FROM_JOB`. When that fails with os error 5 it asks `IsProcessInJob(GetCurrentProcess(), NULL)`; if the backend itself runs inside a job (started by Task Scheduler, a harness or a logon Run key with kill-on-close), it logs `warn` "launched inside a job", starts the program without breakaway and returns `Detached::InJob` instead of `Detached::Free`. The program then dies with the backend. The engine step still succeeds; `finished(run)` stores nothing special, but `launch_after_game_mode` returns the flag (below) so that game mode does not claim a safe hand-over.

**From game mode** (decision of section 06): `games::Service::launch_after_game_mode(id) -> ApiResult<LaunchedAfterGameMode { in_job: bool }>` does the same work without a `LaunchRun`, without a topic and without the `game_mode_active` check (only section 06 calls it, before the backend exits). It starts the game through the one `Opener` (`state.opener()`, `OpenRequest::Spawn`; foundation 11.6), not through the engine. It returns after the hand-over (`in_job` is true when `spawn_detached` returned `Detached::InJob`). Because there is no run, the `on_spawned` callback is never called, so `launch_after_game_mode` inserts the `games_session` row itself (pid, creation time) and writes `games_play` (launch time and count), so that a launch by game mode is counted (section 01 has no row for games). It starts no watcher, because the backend is about to exit; the next start recovers the session from that row (6.13).

### 6.15 Interfaces with other modules

**What this section implements for section 01** (section 01 6.9 is the contract; one engine runs all four target types):

| Item | Shape |
|---|---|
| `ItemSource` (projects and games) | `search_entries()`, `item_for(id)`, `knows(id)` and `recent()` as below |
| `LaunchPlanner` (projects and games) | `plan(target)` returns the steps (project: editor step, then ONE `wt` step for the Claude tab, 6.7; game: one step, 6.14); `finished(run)` writes `projects_opened` or `games_play`. `PlannedStep.on_spawned(pid, creation_time)` gives the games module the process for the play time wait; `plan()` starts no process and walks no directory; it may make a few `symlink_metadata` calls (6.5) |
| Pins | `launcher::Service::pinned_ids(PinnableType) -> HashSet<String>` and `launcher::Service::unpin(PinnableType, &str)` |
| `LaunchTarget.mode` | `terminal` and `claude` are in section 01's `LaunchMode` (S2.8) |

**What this section provides** (public `Service` methods; foundation 11.2 allows no other access):

| Method | For | Behaviour |
|---|---|---|
| `projects::Service::list_cached() -> Arc<Vec<Project>>` | 01 (pins, search, recent), 03, 06 | the last answer; if none exists, one discovery with branches and no git (never waits for git) |
| `projects::Service::search_entries() -> Vec<(SearchItem, Vec<String>)>` | 01 `/search` | non-missing projects: `SearchItem { type: "project", id, title: name, subtitle: path, icon, score: 0 }`, keywords `[branch]` |
| `projects::Service::item_for(id) -> Option<SearchItem>` | 01 `/pinned` | same item; also for a missing project (a pin the user made stays; the row opens "Locate") |
| `projects::Service::recent() -> Vec<(String, SearchItem)>` (RFC 3339 time, item) | 01 `/recent` | projects with `lastOpenedAt`, not missing |
| `projects::Service::project_id_for_path(cwd) -> Option<String>` | 03 (sets `ClaudeSession.projectId`) | the project whose path equals `cwd` or is the longest prefix of it (case-insensitive); missing projects count |
| `games::Service::search_entries()`, `item_for(id)`, `knows(id)`, `recent()` | 01 | non-hidden games: `SearchItem { type: "game", id, title, subtitle: "Steam" \| "Epic" \| "Local", icon: url cover or lucide gamepad-2, score: 0 }`, keywords `[subtitle]`; `recent` = games with `lastPlayedAt` |
| `games::Service::launch_after_game_mode(id)` | 06 | 6.14 |
| `games::Service::get(id) -> Option<Game>` | 06 (game mode shows the next game's cover and title; 06 calls this name, not `lookup`) | |
| `games::Service::any_game_running() -> bool` (async) | 03 (required: an idle job must not run while a game runs) | uses the running computation of 6.13 at call time with `RUNNING_TTL` = 2 s (one process snapshot plus one registry read on the blocking pool); it never reads a cached `Game.running` that nobody refreshed, because Rain has no timer that refreshes it. Section 03 calls it once per sample, only while an idle job is enabled and game mode is off |

None of these methods waits for git, a size or a scan that is not yet cached; they answer from what is known. Hidden games and games of `off` or `failed` sources are never offered (the pin of a hidden game was removed when it was hidden).

**From settings (06):** `settings::Service::resolved() -> Arc<Resolved>` and `watch() -> watch::Receiver<Arc<Resolved>>` (fields `project_roots`, `game_folders`, `steam_enabled`, `steam_path`, `epic_enabled`, `epic_path`). **From core and `platform`:** `core::blocking::run`, `Health::set_source(name, state, message)`, `Clock`, `MediaRegistry::register`, `ProcessLister`, `platform::process::{hidden_command, spawn_detached}`, `platform::time::{local_date_of_unix(secs) -> String, local_date(SystemTime) -> String}`, `platform::windows::list_windows()`, `platform::shell::shell_execute(verb, file, params, dir, flags) -> Result<(), u32>` (foundation 11.6), `platform::registry` helpers. The two functions that are new to the foundation (`list_windows`, `local_date_of_unix`) are in section 12.

### 6.16 Limits and numbers

| Name | Value | Where |
|---|---|---|
| `FIRST_FACTS_BUDGET` | 6 s | 6.3, wait for first git facts in `GET /projects` |
| `REFRESH_WAIT` | 8 s | `POST /projects/refresh` |
| `DIRTY_TTL` | 60 s | 6.3, read-time age check |
| git timeout, output cap | 20 s, 4 MiB | 6.3 |
| `GIT_MAX_PARALLEL` | 4 | 6.3 (foundation Appendix A) |
| `OPEN_TTL`, `RUNNING_TTL`, `LIBRARY_TTL` | 2 s each | 6.4, 6.13, 6.8 |
| `forget_missing_days` | 30 (0 to 3650) | 6.2 |
| root discovery abandon | 8 s | 6.2 |
| `GUESS_MAX_DEPTH`, `GUESS_MAX_DIRS`, `GUESS_BUDGET` | 5, 1500 directories, 5 s | 6.11 |
| guess cache validity | 30 days, plus signature | 6.11 |
| size measure budget, size cache age | 2,000,000 entries, 7 days | 6.13 |
| `COVER_MAX` | 8 MiB | 6.12 |
| manifest file limits | acf 256 KB, localconfig 4 MiB, epic item 1 MiB | 6.9, 6.10 |
| games, projects per answer | designed for 400 and 200; larger works but is not budgeted | 2.1 |

### 6.17 Launch layouts

A layout is a list of at most 12 items. An item says WHAT opens and WHERE it goes. The types are in `rain-types/src/layout.rs`.

- WHAT (`LayoutWhat`): `step` (a built-in or `[[step]]` name: editor, terminal, Claude Code, a named terminal, a browser page), `app` (the id of an app of the launcher's list) or `folder` (the project folder in Explorer).
- WHERE: a `LayoutZone` of one monitor's work area (the screen without the taskbar): `free`, `full`, the halves, the quadrants, the thirds, the two-thirds. `free` is the default and moves nothing. `monitor` is 1 for the monitor at the far left, up to 16. Without it the monitor that Rain's window is on counts; a number that no monitor has falls back to that monitor and the launch result says so.
- Which layout counts: the project's own `layout`, else its own `steps` (no layout), else the `[defaults]` layout, else the default steps. A project without any layout opens exactly as before. Only the full open (Enter) uses zones; "Open in editor only", "Terminal here" and "New Claude session here" open where Windows puts the window.
- Writing: `PUT /projects/{id}/layout` edits `projects.toml` with `toml_edit` (`layout_file.rs`): only the `layout` key of `[defaults]` (`all: true`) or of the project's `[[project]]` entry (found by path, else id; made with its `path` when missing) changes. An empty list removes the key.

How a plan becomes placed windows (`projects/launch.rs`, `core/placement.rs`, `platform/windows_place.rs`):

1. The plan gives a step a `Placement`: the zone, the monitor and a `WindowQuery` (image names, a title text, whether a window that is open already may be the one). Terminal and Claude items that share a zone and a monitor become the tabs of ONE `wt.exe -w new` call, wherever they stand in the list; a different zone is another window. A zone for the terminal always means a new window, whatever `[terminal] window` says, so the user's own terminal window is never moved. A project that is open already starts no terminal (as before) and only places its editor.
2. The launch engine takes a snapshot of the visible windows before each placed step, runs the step, then polls (every 150 ms, at most 8 s) for a window that is not in the snapshot and belongs to the started program: its process id, or one of the image names (`WindowsTerminal.exe` and `wt.exe`; the editor's image; `explorer.exe`; the program of an app; the browsers). A title rule narrows it (VS Code: the folder name; Explorer: the folder name; a store app: its name). Only the editor and Explorer may take a window that is open already (the project is being opened), and only with the title rule. A window that an earlier step of the run placed is never taken again.
3. The window is restored when it is maximised or minimised, then `SetWindowPos` moves it so that its VISIBLE part fills the zone: the invisible resize border is measured (`GetWindowRect` against the DWM extended frame bounds) and added to each edge, and the move is measured again, up to three times, because the border has another width on a monitor of another scale. The calling thread is made per-monitor DPI aware for the call, so all numbers are physical pixels. A moment later the position is checked once more and corrected once, for programs that put their window back.
4. A step whose window did not show up, whose monitor is missing or whose move was refused stays `success` and carries a `note` in the `LaunchStep` (for example "Its window did not show up, so it was left where it is"). Nothing is retried and no other window is moved. The Projects screen shows the notes in a quiet line after the launch.

Not known until a real launch: a browser that opens the page as a tab of a window that is open already makes no window, so it is reported and left; a program that runs as administrator refuses the move; a program with a fixed minimum size may not fill a small zone; UWP windows that Windows reports under another process are found by title only.

---------------------------------------------------------------------------------------------------

## 7. Background work

No timer of this section exists outside the six rows of foundation 11.5. There is no file watcher on repositories, Steam files or game folders (so there are no resident watcher handles). Everything is started by a request and ends by itself.

| Task | Trigger | Timer row (11.5) | Cost | In game mode | How it stops |
|---|---|---|---|---|---|
| Discovery of project folders | every `GET /projects` | none | one `read_dir` per root, 1 to 20 ms, blocking pool | allowed (cheap, read only) | ends by itself; abandoned after 8 s |
| Git refresh (`facts::Refresher`) | a request found stale or missing facts | T4: a 20 s kill timeout per git call; no repeating timer | up to 4 `git` processes at once, about 60 ms each; none while nothing asks | not started; the cache is served | ends by itself; at `stop()` the queue is dropped and running git children are killed (`kill_on_drop`) |
| Open detection | `GET /projects` (cache 2 s) and a project launch | none | `EnumWindows` plus one process snapshot, 2 to 8 ms | false | none |
| Project launch run | `POST /launches` | T5: the optional `delay_ms` before a step (only while the run lasts) | spawns, milliseconds | refused (409 `gameMode`) | ends by itself |
| Steam and Epic scans | `GET /games` (cache 2 s, signature check) | none | 5 to 15 ms, blocking pool | the cached result is served; no rescan | none |
| Game-folder discovery | `GET /games` | none | one `read_dir` per game folder | cached result only | none |
| Program-file guess | `GET /games` finds a folder game without a valid cache row | none | 10 to 30 ms per folder, 4 at once, 5 s budget for the answer, the rest continues | not started | ends by itself |
| Size measurement | `GET /games` finds a missing or old size row | none | one folder at a time on the blocking pool, 30 ms to several seconds | not started; queue dropped | cancel flag at `stop()` |
| Game session watcher | a Rain direct start of a local game | none: it waits on the process handle (`open_for_wait`), the operating system wakes it | one parked task per running game | keeps waiting (it is already running); lost when the backend exits | ends when the game ends; the session is recovered at the next start (6.13) |
| Session recovery | module `start()` | none | one `OpenProcess` per open row, normally 0 rows | allowed | ends by itself |
| Settings and TOML subscriptions | a change of `settings.toml`, `projects.toml`, `games.toml` | T4: the 300 ms debounce of the file watcher that already exists | drops the caches, publishes a topic | topic still published; no rescan work | none |
| `projects` and `games` frames | the events above | `core::throttle::Throttle` (not a timer of ours) | one serialisation, at most 1 per second | no frames are published while no client is connected (bus rule) | none |

Idle cost of the two modules: zero threads and zero timers while nothing asks. Memory: the last project list (about 100 KB for 100 projects) and the last game library (about 100 KB for 180 games), one prepared SQLite use per request.

Checklist item for review (foundation 11.5): `tokio::time` calls added by this section: `sleep` for `delay_ms` (T5), `sleep` for the manifest read retry of 50 ms (T4), `timeout` around git calls (T4), `timeout` around discovery (T4). `thread::spawn` added: none; the game waits are `ExitWatcher`s (`open_for_wait`), which use no thread and no blocking-pool slot, so any number of running games cost nothing.

---------------------------------------------------------------------------------------------------

## 8. Errors

Message rules: foundation 4.5 (plain words, no file content, a configured path may appear). The `source` of the body is `projects` or `games` unless stated.

### 8.1 Route errors

| Situation | Status | `code` | `reason` | `message` | UI shows |
|---|---|---|---|---|---|
| module did not start | 503 | `unavailable` | `moduleFailed` | "Projects are not available" / "Games are not available" | `ErrorRow` "Projects unavailable" / "Games unavailable" with Retry (existing) |
| backend is stopping | 503 | `unavailable` | `shuttingDown` | "Rain is stopping" | same row |
| unknown project in a launch | 404 | `notFound` | `unknownId` | "Project not found" | error notice (`useLaunch.ts:82`) |
| unknown game | 404 | `notFound` | `unknownId` | "Game not found" | error notice |
| the plan has no step: `editorOnly` on a list without an editor step, or an empty list (`terminal` and `claude` modes never give `noSteps`: 6.7 falls back to the built-in step of that name) | 400 | `invalid` | `noSteps` | "This project has no editor step" ("This project has no steps" for an empty list) | error notice |
| any launch or refresh during game mode | 409 | `conflict` | `gameMode` | "Game mode is on" | error notice |
| `saveCustomGame` or `PATCH` input | 400 | `invalid` | `noName` | "A game needs a name" | error notice, the dialog stays open (`CustomGameDialog.vue:344`) |
| | 400 | `invalid` | `noExe` | "A game needs a program file" | same |
| | 400 | `invalid` | `exeNotAbsolute` | "Use the full path of the program, for example D:\Games\Celeste\Celeste.exe" | same |
| | 400 | `invalid` | `exeNotFound` | "Program file not found: <path>" | same |
| | 400 | `invalid` | `notAnExe` | "The program file must be an .exe" | same |
| | 400 | `invalid` | `coverNotFound` | "Cover image not found: <path>" | same |
| | 400 | `invalid` | `coverNotImage` | "The cover must be a PNG, JPG, WEBP or GIF image" | same |
| | 400 | `invalid` | `coverTooLarge` | "The cover image is larger than 8 MB" | same |
| | 409 | `conflict` | `alreadyListed` | "<title> is already in the library" (adds " (hidden)" when it is hidden) | same |
| patch without a field | 400 | `invalid` | `emptyPatch` | "Nothing to change" | error notice |
| edit of a Steam or Epic game (other than hide) | 400 | `invalid` | `notEditable` | "Steam and Epic games can only be hidden" | error notice |
| remove a game that is not hand-added | 409 | `conflict` | `notRemovable` | "Only games you added yourself can be removed. Hide it instead" | error notice |
| `games.toml` or `projects.toml` cannot be saved | 500 | `failed` | `saveFailed` | "Could not save games.toml" | error notice |
| `games.toml` changed on disk while saving | 409 | `conflict` | `configChanged` | "games.toml changed. Try again" | error notice |
| `games.toml` does not parse (a skipped bad entry does not count), on `POST`, `PATCH` or `DELETE /games` | 409 | `conflict` | `configBroken` | "games.toml has an error. Fix the file, then try again" | error notice; `GET /health` lists the issue |
| `games.toml` was written by a newer Rain | 409 | `conflict` | `configNewer` | "games.toml was written by a newer Rain and is read only" | error notice |

### 8.2 Launch step errors (shown as `"<step label>: <error>"` in the notice and the checklist)

These texts reach `LaunchStep.error` through `PlannedStep.error_text` (section 01 6.9), a closure that receives the `OpenError` of the failed start and matches on `err.code` (the ShellExecute result or Win32 error code: 31 means no program is registered for the URI, so "Steam is not installed"; any other value of 32 or less is "Could not hand the game to Steam"; 740, 5 and 1920 are decided by the engine, not here), never on `ui_text` or `log_reason`: the planner of this module sets it on each step, and when it is set it replaces the engine's own table text (the 40 character limit of the engine applies only to the engine's own texts).

| Step | Situation | `error` text |
|---|---|---|
| any | project folder gone | `Folder not found: <path>` (the run ends, later steps stay idle) |
| editor | no editor found | `VS Code was not found. Set its path in projects.toml` (the label names the configured editor) |
| editor, terminal, claude, git | `[tools]` path set but missing | `Configured program not found: <path>` |
| editor, app | start failed | `Could not start <file name>` |
| terminal, claude | `wt.exe` not found | `Windows Terminal was not found` |
| terminal, claude | both the direct start and the `ShellExecute` fallback failed | `Could not open Windows Terminal` |
| claude | no `claude` found | `Claude Code was not found. Set its path in projects.toml` |
| browser | URL refused or no browser | `Could not open the browser` |
| game, Steam | | `Steam is not installed`, `Could not hand the game to Steam` |
| game, Epic | | `Epic Games is not installed`, `Epic Games Launcher is not installed` |
| game, local | | `No program file. Choose one with Edit`, `Program file not found: <path>`, `Could not start the program` |

### 8.3 Degraded sources and `GET /health` `sources`

| Source name | `ok` | `off` | `failed` |
|---|---|---|---|
| `projects` | roots read; message "<n> step(s) ignored" when `projects.toml` had bad entries | no roots set (message "No project folders are set") | a root cannot be read: `Project folder not found: <path>` |
| `git` | found and working | not found: "git was not found. Showing the branch only" | `[tools] git` set but missing, or git refused every repository |
| `steam` | listed | "Steam is not installed" or "Turned off in Settings" | `Steam folder not found: <path>`, "Could not read the Steam library" |
| `epic` | listed (also "no games") | "Epic Games is not installed" or "Turned off in Settings" | `Epic folder not found: <path>`, "Could not read the Epic Games library" |
| `gamefolders` | all folders read, or none set | never | `Game folder not found: <path>` (more than one: "... and <n> more"); `games.toml` unreadable: "games.toml has an error. Showing the last good copy" (this also appears as a `ConfigIssue`) |

`GameSourceStatus` in `GET /games` is the same information for `steam`, `epic` and `custom` (custom = `gamefolders` plus `games.toml`), so the Games screen needs no second call. A source in state `off` is not an error anywhere in the UI.

---------------------------------------------------------------------------------------------------

## 9. Frontend changes

Line numbers refer to the tree at commit `e4c7f70`. A step marked "stage 5" happens in the stage that builds this area. Rule of foundation 8.8: when a type is ported to `src/bindings/rain-types.ts`, its hand-written definition in `src/data/types.ts` is deleted in the same commit.

### 9.1 Files, in the order of work

**Data layer**

| File | Change |
|---|---|
| `src/data/types.ts` | Delete the hand-written blocks `// ---- projects ----` (`EnvStep`, `Project`, lines 31 to 37) and `// ---- games ----` (`GameSource`, `Game`, `GameLibrary`, `CustomGameDraft`, lines 39 to 44). They now come from `rain-types.ts`. Settings: nothing to edit; the generated `Settings` with `gameFolders` comes with stage 1b (section 06) |
| `src/data/api.ts` | Import `GamePatch`; `TopicPayload` (line 13 to 20) gains `projects: Project[]` and `games: GameLibrary`; `RainApi` gains `updateGame(id: ID, patch: GamePatch): Promise<Game>  // PATCH /games/:id` and `deleteCustomGame(id: ID): Promise<void>  // DELETE /games/:id` under `// projects and games` (after line 47). The route comments of the area are replaced by the registry of foundation 14.4 |
| `src/data/rest/client.ts` | `listProjects: () => call('GET', '/projects')`; `listGames: () => call('GET', '/games')`; `saveCustomGame: (d) => call('POST', '/games', d)`; `setGameHidden: (id, hidden) => call('PATCH', \`/games/${enc(id)}\`, { hidden }).then(() => undefined)`; `updateGame: (id, p) => call('PATCH', \`/games/${enc(id)}\`, p)`; `deleteCustomGame: (id) => call('DELETE', \`/games/${enc(id)}\`)`. `launch` is section 01's call; the target type needs the new modes |
| `src/data/rest/stream.ts` | topics `projects` and `games` are plain pass-through (payloads go through `resolveMedia`, which rewrites `coverUrl` and the `value` of `url` icons) |
| `src/data/rest/topic-check.ts` | the compile-time check picks the two new names up from `TOPIC_NAMES`; `projects` and `games` are added to `Topic` and to `TOPIC_NAMES` (through their structs), never to `UnportedTopic` (foundation 9.3) |
| `src/data/routing.ts` | stage 5: `listProjects`, `listGames`, `saveCustomGame`, `setGameHidden`, `updateGame`, `deleteCustomGame`: `'real'`; topics `projects`, `games`: `'real'` |
| `src/data/router.ts` | the special cases of foundation 8.6: `launch` sends `project` and `game` targets to `real` (the router already routes `app` and `pool` since stage 4, so after this stage all four go to `real`); `setPinned` for `project` and `game` goes to `real`; `search`, `listPinned` and `listRecent` now merge real results for the types `app`, `pool`, `project`, `game` and mock results for `session`, `job`, `idleJob` |
| `src/main.ts` | nothing (the reset callbacks already refetch all resources; `resetLaunches` clears project and game runs on a restart) |

**Projects**

| File | Change |
|---|---|
| `src/features/projects/ProjectsScreen.vue` | line 254: `useResource('projects', () => api.listProjects(), { live: 'projects' })`. Lines 268 to 270 (the "All projects" order): a project that was never opened has no time and must not break the sort. Replace with `const openedAt = (p: Project) => Date.parse(p.lastOpenedAt ?? '') \|\| 0` and `.sort((a, b) => openedAt(b) - openedAt(a))`. The backend returns never-opened projects in `activityScore`-then-name order and `Array.sort` is stable, so they keep that order after the opened ones |
| `src/features/projects/ProjectCard.vue` | lines 173 and 190: `<RelTime v-if="project.lastOpenedAt" :iso="project.lastOpenedAt" ... />` (the card shows no time for a project never opened) |
| `src/features/projects/ProjectDetail.vue` | lines 33 to 37: `changedFiles` returns `undefined` when `dirtyFiles === undefined`; line 74: `<p v-if="changedFiles" class="text-13 text-muted-foreground">{{ changedFiles }}</p>` |
| `src/features/projects/project-sessions.ts` | lines 207 to 211: the filter becomes `(session) => session.projectId === project.id` (the `sessionIds` clause is gone) |
| `src/features/projects/actions.ts` | line 151: `launchProject(project, mode: 'full' \| 'editorOnly' \| 'terminal' \| 'claude')`. Lines 194 to 209: "Terminal here" runs `launchProject(project, 'terminal')` and "New Claude session here" runs `launchProject(project, 'claude')` (the two prototype notices are removed). Nothing else changes: the labels "Focus" (when `project.open`) and "Locate" (when `missing`) already match the backend (6.7) |
| `src/features/home/HomeScreen.vue` | lines 47 and 48: pass `{ live: 'projects' }` and `{ live: 'games' }` so Home follows the pushes too |

**Games**

| File | Change |
|---|---|
| `src/nav/routes.ts` | line 79: `subs: ['all', 'steam', 'epic', 'custom', 'installed', 'hidden']` |
| `src/features/games/gameSource.ts` | `SOURCE_LABEL.custom` = `'Local'` (decision G1; the label shows in the strip, the detail pane and the search subtitle) |
| `src/features/games/GamesScreen.vue` | see 9.3 for the exact code. Summary: `useResource(..., { live: 'games' })` (line 41); `SUBS` gains `'hidden'` (line 27); the strip hides sources whose state is `off` and shows "Hidden" only while a game is hidden; `shown`, `inSub`, counts, `failedSources`, `errors` (lines 50 to 89) use `sources`; the Recent group is skipped in the Hidden view (line 98); `useScreen` gets the visible tabs (line 124) |
| `src/features/games/GameTile.vue` | lines 128 to 130: the download glyph only when `!game.installed && game.source !== 'custom'` |
| `src/features/games/GameDetail.vue` | lines 21 to 24: the status text covers local games: "Running", "Program file missing" (custom, not installed), "No program file" (custom, no `exePath`), "Installed", "Not installed" |
| `src/features/games/actions.ts` | rewritten, see 9.4. `customGameDialog` gets `editing: ref<Game \| null>(null)`; a new exported `openAddGame()` sets `editing = null` then `open = true` |
| `src/features/games/CustomGameDialog.vue` | one dialog for "Add a game" and "Edit game", see 9.5 |
| `src/keys/keymap.ts` | line 14: import `openAddGame`; line 295: `else openAddGame()` instead of `customGameDialog.open.value = true` |
| `src/features/game-mode/GameModeScreen.vue` | none. Lines 156 to 170 find the game by id and read `title` and `coverUrl`; both exist. A game of an `off` source is not in `games`, so the "Then launch" line is absent, as today |

**Settings.** Section 06 owns the Settings files (`settingDefs.ts`, `settingValues.ts`, `SettingRow.vue`, `SettingsScreen.vue`, `mock/seed/settings.ts`). The row `game-folders` and its `readValue` and `patchFor` cases are in 06 section 9.2 (step 1b); the row appears in stage 5, when `games` starts to read the folders. This section adds nothing to those files. The found or missing glyph calls `checkPaths` (section 05 replaces `mockControl.pathExists`).

**Mock adaptation** (the mock stays compilable until stage 10; section 9.6).

### 9.2 Contract changes (table of foundation 15.1)

| # | Item | From | To | Reason | Frontend files touched | Breaking for the mock? | Stage |
|---|---|---|---|---|---|---|---|
| S2.1 | `Project.dirtyFiles` | `number` | `number?` | absent when git is missing or has not answered | `ProjectDetail.vue` | no (mock still sets it) | 5 |
| S2.2 | `Project.lastOpenedAt` | `ISODateTime` | `ISODateTime?` | never opened from Rain | `ProjectsScreen.vue`, `ProjectCard.vue` | no | 5 |
| S2.3 | `Project.sessionIds` | `ID[]` | removed | duplicate of `ClaudeSession.projectId` | `project-sessions.ts` | yes: delete `sessionIds: []` in `mock/seed/projects.ts:87` and its comment at line 3; delete the loop in `mock/store.ts:69` to `71` | 5 |
| S2.4 | `Game` | 11 fields | adds optional `exePath`, `exeGuessed`, `installPath`, `removable` | edit dialog, Explorer reveal, remove | `GameDetail.vue`, `actions.ts`, `CustomGameDialog.vue` | no | 5 |
| S2.5 | `GameLibrary.failedSources` | `GameSource[]` | `sources: GameSourceStatus[]` | three states; Epic missing is not a failure | `GamesScreen.vue` | yes: `mock/slices/library.ts` builds `sources` from its own `failedSources` and `sourceEnabled` (9.6) | 5 |
| S2.6 | `RainApi.updateGame`, type `GamePatch` | none | `PATCH /games/:id` | corrections of a game | `api.ts`, `rest/client.ts`, `actions.ts`, `CustomGameDialog.vue` | yes: the mock slice is deleted in stage 5; the name joins `RetiredMockMethod` (9.6) | 5 |
| S2.7 | `RainApi.deleteCustomGame` | none | `DELETE /games/:id` | remove a hand-added game | `api.ts`, `rest/client.ts`, `actions.ts` | yes: the name joins `RetiredMockMethod` (9.6) | 5 |
| S2.8 | `LaunchTarget.mode` (section 01 3.2 `LaunchMode`) | `'full' \| 'editorOnly'` | section 01 3.2 already holds `terminal` and `claude` | "Terminal here" and "New Claude session here" become real | `src/features/projects/actions.ts` | no: the mock `launches.ts` is deleted in stage 5 (9.6) | 5 |
| S2.9 | `Settings.gameFolders` | none | `string[]`, default `[]` | decisions.md section 4, game folders | `settingDefs.ts`, `settingValues.ts` | none here: section 06 S6.1 adds `gameFolders: []` to the mock seed in stage 1b | 5 |
| S2.10 | `TopicPayload` | 6 topics (7 with `health`) | adds `projects: Project[]`, `games: GameLibrary` | git facts, sizes and running state arrive after the first answer | `api.ts`, `routing.ts`, `ProjectsScreen.vue`, `GamesScreen.vue`, `HomeScreen.vue` | no: the mock `subscribe` accepts any `Topic` and never emits these | 5 |
| S2.11 | `SOURCE_LABEL.custom` | `'Custom'` | `'Local'` | folder games are not "custom" | `gameSource.ts` | no | 5 |
| S2.12 | route subs of Games | five | adds `hidden` | unhide | `src/nav/routes.ts:79` | no | 5 |
| S2.13 | `Game.coverUrl`, `IconRef` of a game | image URL or data URI | `media/game-cover/<id>?v=..`, rewritten by the REST client | foundation F8 | none | no | 5 |

### 9.3 `GamesScreen.vue`, exact code of the changed parts

```ts
// line 27
const SUBS = ['all', 'steam', 'epic', 'custom', 'installed', 'hidden'] as const
type Sub = (typeof SUBS)[number]

// line 41
const library = useResource('games', () => api.listGames(), { live: 'games' })

// replaces lines 43 to 59
const allGames = computed(() => library.data.value?.games ?? [])
const sources = computed(() => library.data.value?.sources ?? [])
const hiddenCount = computed(() => allGames.value.filter((game) => game.hidden).length)

/** The tabs on screen: a source that is off has none; Hidden exists only while something is hidden. */
const visibleSubs = computed<Sub[]>(() =>
  SUBS.filter((id) => {
    if (id === 'hidden') return hiddenCount.value > 0
    return !sources.value.some((s) => s.source === id && s.state === 'off')
  }),
)

const sub = computed<Sub>({
  get: () => {
    const wanted = SUBS.find((id) => id === nav.route.value.sub) ?? 'all'
    return visibleSubs.value.includes(wanted) ? wanted : 'all'
  },
  set: (id) => nav.setSub(id),
})

const shown = computed<Game[]>(() =>
  allGames.value.filter((game) => (sub.value === 'hidden' ? game.hidden : !game.hidden)),
)

function inSub(game: Game, id: Sub): boolean {
  if (id === 'all' || id === 'hidden') return true
  return id === 'installed' ? game.installed : game.source === id
}

const filtered = computed(() =>
  shown.value.filter((game) => inSub(game, sub.value) && matches(query.value, game.title)),
)

// replaces lines 61 to 89
const failed = computed(() => sources.value.filter((s) => s.state === 'failed'))

const stripItems = computed<ViewStripItem[]>(() => {
  const loaded = library.data.value !== undefined
  const count = (id: Sub) =>
    !loaded ? undefined
    : id === 'hidden' ? hiddenCount.value
    : allGames.value.filter((game) => !game.hidden && inSub(game, id)).length
  const source = (id: GameSource): ViewStripItem => ({
    id, label: SOURCE_LABEL[id].split(' ')[0], icon: SOURCE_ICON[id].value,
    brand: SOURCE_ICON[id].kind === 'brand', count: count(id),
    warn: failed.value.some((s) => s.source === id),
  })
  const items: Record<Sub, ViewStripItem> = {
    all: { id: 'all', label: 'All', icon: 'library', count: count('all') },
    steam: source('steam'), epic: source('epic'), custom: source('custom'),
    installed: { id: 'installed', label: 'Installed', icon: 'hard-drive-download', count: count('installed') },
    hidden: { id: 'hidden', label: 'Hidden', icon: 'eye-off', count: count('hidden') },
  }
  return visibleSubs.value.map((id) => items[id])
})

/** One message per failed source that the current view would have shown, or one for the whole library. */
const errors = computed(() => {
  if (library.error.value) return ['Games unavailable']
  return failed.value
    .filter((s) => sub.value === 'all' || sub.value === 'installed' || sub.value === s.source)
    .map((s) => s.message ?? `${SOURCE_LABEL[s.source]} library unavailable`)
})

// lines 98 to 104: no Recent group in the Hidden view
const recent =
  query.value === '' && sub.value !== 'hidden'
    ? filtered.value
        .filter((game) => game.lastPlayedAt)
        .sort((a, b) => (b.lastPlayedAt ?? '').localeCompare(a.lastPlayedAt ?? ''))
        .slice(0, columns.value)
    : []

// line 124: the keymap reads `ids` when Tab is pressed, so a getter keeps it current
useScreen({ target, move, page, group, subView: { get ids() { return visibleSubs.value }, current: sub } })
```

(`GameSource` stays imported at line 19; `failedSources` and its two uses are gone.) The empty state at lines 166 to 172 is unchanged: "No games found" with "Open settings" when `shown.length === 0` and the user has nothing at all. When every source is off or empty it says the same; the hint of the existing button points to Settings where the three source rows are.

### 9.4 `src/features/games/actions.ts`, the provider

```ts
export const customGameDialog = { open: ref(false), editing: ref<Game | null>(null) }
export function openAddGame() { customGameDialog.editing.value = null; customGameDialog.open.value = true }
function openEditGame(game: Game) { customGameDialog.editing.value = game; customGameDialog.open.value = true }

const hasProgram = (g: Game) => g.source !== 'custom' || (g.installed && g.exePath !== undefined)

export const gameActions: ActionProvider<TargetOf<'game'>> = ({ item: game }) => {
  // A hidden game can only be brought back (Enter) or played from its menu.
  if (game.hidden) {
    return [
      { id: 'show', label: 'Show in library', icon: 'eye', slot: 'primary', run: toggleHidden(game) },
      ...(hasProgram(game) ? [playAction(game, 'more')] : []),
    ]
  }
  const actions: RainAction[] = [hasProgram(game) ? playAction(game, 'primary') : chooseProgram(game)]
  if (hasProgram(game)) actions.push(playInGameMode(game))     // unchanged, existing function
  actions.push(pinAction('game', game.id, game.pinned))
  if (game.installPath) actions.push(revealAction(game.installPath, 'more', 'Show in Explorer'))
  if (game.source === 'custom') actions.push(editGame(game))
  actions.push(hideAction(game))
  if (game.removable) actions.push(removeGame(game))            // danger: true, asks through ctx.confirm
  return actions
}
```

- `playAction`: today's `play` action (label "Play", icon `play`); the "Install" wording is gone because the library lists installed games only. The run is `ctx.launch({ type: 'game', id: game.id }, game.title)` as before.
- `chooseProgram`: label "Choose program", icon `folder-search`, slot `primary`, runs `openEditGame(game)`.
- `editGame`: label "Edit game", icon `pencil`, slot `more`.
- `hideAction`: today's `toggleHidden` for a visible game: `api.setGameHidden(game.id, true)`, `refreshResources('games', 'pinned', 'recent')`, notice "Hid <title>". The unhide run: `setGameHidden(id, false)`, refresh the same keys, notice "<title> is back in the library".
- `removeGame`: label "Remove from library", icon `trash-2`, `danger: true`; `confirm({ title: 'Remove <title>?', body: 'The game is taken out of Rain. Its files are not touched.', confirmLabel: 'Remove', danger: true })`, then `api.deleteCustomGame(id)`, `refreshResources('games', 'pinned', 'recent')`.
- The action `storePage` and the `STORE_NAME` table are deleted (decision P3); the import of `revealAction` comes from `@/actions/common`.

### 9.5 `CustomGameDialog.vue`

- Title and description: "Add a game" / "For a game that Steam and Epic do not list." when `editing === null`; "Edit game" / "Change what Rain starts for this game." when editing. A guessed program shows the line "Rain guessed this program from the folder. Change it if it is wrong." (muted, under the field) when `editing.exeGuessed`.
- The watch at lines 324 to 329 fills the three fields from `editing` (title, `exePath ?? ''`, cover empty = unchanged) or empties them.
- Save: `editing === null` calls `api.saveCustomGame(...)` as today; otherwise `api.updateGame(editing.id, patch)` where the patch holds only the fields that differ from the game (`title`, `exePath`, and `coverPath` only when the field is not empty; an explicit "remove cover" is a text button "Remove cover" shown when the game has a `coverUrl`, which sends `coverPath: ''`). A game-folder game gets one more text button, "Use the guess again", that sends `exePath: ''`.
- Success: `refreshResources('games', 'pinned', 'recent')`, notice "Added <title>" or "Saved <title>", close. Failure: `notice.error(errorMessage(reason, ...))` with the messages of section 8.1, the dialog stays open (existing behaviour).
- The three fields stay plain text inputs (decision of the prototype: no file picker); the placeholders become the generic examples `Celeste`, `D:\Games\Celeste\Celeste.exe`, `D:\Games\Celeste\cover.png`.

### 9.6 Mock adaptation (stage 5)

The mock keeps type-checking and working for the screens that stay on it until the deletions at the end of this stage:

| File | Change |
|---|---|
| `src/data/mock/slices/library.ts` | `listGames`: build `sources` for `steam`, `epic`, `custom` from `sourceEnabled` and the local `failedSources` (`state` = `off` / `failed` / `ok`, `message` = "Epic Games library unavailable" for `failed`). No mock `updateGame` or `deleteCustomGame` is written: the file is deleted in this same stage, and the two method names go into `RetiredMockMethod` instead |
| `src/data/mock/seed/projects.ts` | line 87 and the comment at line 3: remove `sessionIds` |
| `src/data/mock/store.ts` | lines 69 to 71: remove the loop that fills `sessionIds` |

The mock `seed/settings.ts` is not edited here: `gameFolders: []` is added in stage 1b by section 06 (S6.1). The `launches.ts` lines for the new launch modes are not edited either: the file is deleted in this stage (section 01 9.6).

**Retired mock methods (foundation 8.6).** At the end of stage 5 the method names `listProjects`, `listGames`, `saveCustomGame`, `setGameHidden`, `updateGame` and `deleteCustomGame` are added to `RetiredMockMethod` in `src/data/mock/index.ts` (section 01 adds `listApps`, `listPools`, `savePool`, `deletePool`, `rescanApps`, `launch` and `getLaunch` in the same stage; `launch` and `getLaunch` are implemented by the deleted `launches.ts`). No stub is left behind, and `routing.ts` may not route them to `'mock'`.

**Mock topic bus (stage 5).** `src/data/mock/engine.ts` declares `listeners: { [T in Topic]: Set<...> }`, so every key of `TopicPayload` needs an entry or `vue-tsc` fails. The same commit that adds `projects` and `games` to `TopicPayload` adds, after the entries of `settings` and `health`: `// The mock never publishes these two; the router sends them to the backend.` `projects: new Set(),` `games: new Set(),`.

**Deleted at the end of stage 5** (foundation 14.3): `src/data/mock/slices/library.ts`, `src/data/mock/seed/projects.ts`, `src/data/mock/seed/games.ts`, `src/data/mock/covers.ts`, and, by section 01 9.6, `src/data/mock/slices/launches.ts`, `src/data/mock/seed/apps.ts` and the app and pool methods of `src/data/mock/slices/launcher.ts`. Before `launches.ts` is deleted, `markGamePlayed` (line 170 of `launches.ts`) moves into `src/data/mock/slices/gameMode.ts`, its only user (the import at line 10 is replaced by the local function), as section 03 does for `finishRunningJobs` and `finishWorkingSessions`. In `mock/store.ts` the lines that import the deleted seeds are replaced by `projects: []` and `games: []`, the `seedApps()` call goes, and `epicFailuresLeft` and the `sourceEnabled` use for games go; `store.ts` keeps the two empty arrays and `findableGames()` until stage 10 because `search.ts`, `launcher.ts` and the game-mode slice (`markGamePlayed`, which ignores an unknown id) still compile against them. `createMockApi()` loses the `librarySlice` spread, and `RainApi` methods of this area are then served only by the real client (the router routes them to `real`). After stage 5 the router special cases for `launch`, `getLaunch`, `subscribe('launch')` and `setPinned` are deleted; only `search`, `listPinned` and `listRecent` still merge mock items for session, job and idleJob until stages 6 and 8.

### 9.7 `docs/ui-spec.md`: replacement text (the file is edited by the lead in stage 5, not by this section)

**Section 4.3, rows Project and Game** become:

| Item | Where | Enter | Ctrl+Enter | More in Ctrl+K |
|---|---|---|---|---|
| Project | anywhere | Open dev environment (Focus if its editor window is open; Locate if the folder is missing) | Open in editor only | Pin, Terminal here (a Windows Terminal tab in the folder), New Claude session here (a tab that runs `claude`), Reveal in Explorer, Copy path |
| Game | anywhere | Play; Choose program (a local game that has no usable program file); Show in library (in the Hidden view) | Play in game mode | Pin, Show in Explorer, Edit game (local games), Hide from library, Remove from library (games you added) |

**Section 7.3 (Projects)**, changes: (1) "All projects" is ordered by `lastOpenedAt` descending; projects that were never opened from Rain come after those, ordered by activity, then name; the card shows no time for them. (2) The detail pane shows "N changed files" only when git answered; without git the line is absent. (3) "Open" means an editor window of the project is open (found by its window title); the dot and the label "Focus" follow it. (4) Enter on an open project brings the editor forward and starts nothing else. (5) "Terminal here" and "New Claude session here" start a Windows Terminal tab in the folder. (6) A project is a git repository directly inside a folder of Settings > Project folders; folders without `.git` are not projects.

**Section 7.4 (Games)**, replace the strip and the dialog bullets:

- Strip: All, Steam, Epic, Local (`box`), Installed, and Hidden (`eye-off`, only while a game is hidden), each with a count. A source that is not installed or switched off has no tab. A source that failed to load shows `warn` and its message (for example "Game folder not found: D:\Games") in an `ErrorRow` with Retry; the other sources still show.
- "Local" holds the games Rain finds in the folders of Settings > Game folders (every direct subfolder is one game; the title is the folder name; the program file is guessed) and the games added by hand.
- `CustomGameDialog` (Ctrl+N: Add a game; Ctrl+K "Edit game": the same dialog): name, program file, cover image path. A guessed program file says so.
- Hidden games are left out of every tab except Hidden; in Hidden, Enter shows the game again.
- Not installed games do not occur for Steam and Epic (only installed games are listed); a local game whose program file is gone is dimmed and its Enter reads "Choose program".

**Section 7.9 (Settings), table Sources:** not replaced here. The rows `Game folders`, `Steam library` and `Epic library` and their texts are in section 06 9.2 and 9.6 (the row `game-folders` appears in stage 5).

**Section 8 (mock data layer):** `listGames` returns `sources`; `Project.sessionIds` is gone; `lastOpenedAt` and `dirtyFiles` are optional. (Section 8 is marked superseded in stage 10.)

---------------------------------------------------------------------------------------------------

## 10. Tests

### 10.1 Unit tests (pure code, in the module files)

| Test group | What it proves |
|---|---|
| `paths`: `norm_path`, `path_key` | table of 20 inputs: drive-letter case, `/` and `\`, trailing slash, double separators, UNC, `\\?\`, Unicode case |
| `slug`, `fnv8`, `unique_id` | the id rules of 6.1; non-ASCII names; collision keeps the first and suffixes the others deterministically |
| `git::parse_status_z` | counts for: empty, 3 modified, an untracked folder (1), a rename record (`R  new\0old\0` counts 1), a copy, binary garbage |
| `git::parse_log` and `dates::bucket` | a fake `LocalClock` with a fixed UTC offset and one with a DST jump: timestamps at 23:59 and 00:00 local fall in the right days; commits 14 days ago are dropped; index 13 is today; empty output gives 14 zeros |
| `dates::shift` | the cached array shifts left by the days passed |
| `dates::activity_score` | 0 for zeros, 1 for one commit today, 0 for one commit 13 days ago, the rounding of the mock formula for 3 rows copied from the mock seed |
| `git::read_head`, `head_signature` | temp folders: `ref: refs/heads/main`, a detached 40-hex, a `.git` file with `gitdir:` (relative and absolute), a missing HEAD, a signature that changes after the reflog file is touched |
| `tools::find` | fake `ToolEnv`: an override that exists, an override that is missing (error, no fall-through), `PATH` hit, `code.cmd` mapped to `..\Code.exe`, known path with and without the environment variable, nothing found; the alias case (`symlink_metadata` sees it) |
| `launch::quote_arg` | `a`, `a b`, `a"b`, trailing backslash, empty, a path with `;` |
| `launch::wt_line` (golden strings) | plain tab, tab with command, Claude only (golden string: `new-tab ... cmd.exe /d /k ""<claude exe>" <args>"`, the tab stays a shell after Claude exits), the coalesced terminal plus Claude call, `window = "new"`, a profile, a title with `;` and `"`, a command with `;` (written `\;`) |
| `env::resolve` | defaults; a `[[project]]` by path (case differs) and by id; an unknown step name skipped; a user step replacing a built-in; a browser step with a bad URL skipped; a command with `"` skipped |
| `windows::title_matches` | table: `main.rs - web-shop - Visual Studio Code`, `web-shop - Visual Studio Code`, `* main.rs - web-shop - Visual Studio Code`, `web-shop (Workspace) - Visual Studio Code`, `web-shop [Administrator] - ...` match `web-shop`; `web-shop-old - ...`, `web - ...`, `notes about web-shop - Chrome` do not; a folder name with ` - ` inside |
| `vdf` | copy the spike's tests; add: BOM, `//` comment, duplicate keys, escaped backslashes in a path, a half-written file (no panic), case-insensitive lookup |
| `steam::parse_manifest` | StateFlags 4, 6, 1026 (skipped), 0; `LastPlayed` "0"; tool entries dropped (228980, `Proton 8.0`); non-digit appid skipped |
| `steam::account` | the four rules of 6.9 on fake `SteamRegistry` and fixture files: active user, auto-login name to steam id conversion, `MostRecent`, newest folder, none |
| `steam::cover` | flat `library_600x900.jpg`, nested `<40hex>\library_600x900.jpg`, only `library_capsule.jpg`, only `header.jpg` (no cover) |
| `epic::parse_item` | fixtures in 10.3: a normal game, DLC (`MainGameAppName` differs), incomplete install, no `AppCategories`, `AppCategories` without `games`, a BOM, `InstallSize` as a string, a missing `InstallLocation`, a bad `AppName` (spaces), broken JSON |
| `guess` | every layout of 6.11 as a file list (no disk): winners as in the table; hard drops; the tie rules; Unicode folder name; `Game64.exe` over `Game.exe`; `Game.v1.2.3-GOG` folder with `gamename.exe`; only `setup.exe` gives `None`; a list with 10 000 files finishes in under 50 ms |
| `folders::walk` | real temp trees: depth 5 cut-off (a file at depth 6 is not seen), the 6000-entry budget stops the walk, skipped folder names are not entered, `_`-folders and loose files are not games, a folder with no files is not a game, a Steam library folder inside a game folder is skipped |
| `local::assemble` | corrections applied: hidden, title, relative and absolute exe, a missing corrected exe gives `installed = false` and no guess behind it, cover precedence (user, in-folder `cover.jpg`, none) |
| `covers::validate` | magic bytes of PNG, JPEG, WEBP, GIF; a `.png` that is text; 8 MiB limit; the output file name uses the content hash |
| `games_toml` edits | on a file with comments and CRLF: hide, unhide (entry removed when empty), set title, add custom, delete custom; comments, order and line ends survive; a duplicate key keeps the first |
| `running::session_time` | fake process lister and fake waiter: a launcher stub exits after 3 s and the real game runs 10 min: 10 min 3 s are counted; the 3-round limit; a recovered live pid is re-attached; a recovered dead pid adds `last_seen_at - started_at` |
| `projects::open_flag` | fake `WindowLister` and `ProcessLister`: only windows of the editor process count |

### 10.2 Traits and fakes

| Trait | Production | Fake in `tests/` |
|---|---|---|
| `GitRunner` | `platform::process::hidden_command("git")` | `FakeGit`: canned stdout and exit code per `(repo, subcommand)`, counts calls, can delay |
| `LocalClock` | `platform::time` | fixed offset; DST jump |
| `ToolEnv` | `PATH`, environment variables, `symlink_metadata` | a map |
| `WindowLister`, `ProcessLister` | `platform::windows`, Toolhelp | lists |
| `Opener` (foundation 11.6; defined once in `core/launch_contract.rs`; the launch calls `spawn_detached` and `shell_execute` are behind it) | `platform::process`, `platform::shell` | the one `FakeOpener` of section 01: records the exact `OpenRequest`s and command lines, returns a fake pid, can fail the Nth call, can hold a "process" until the test ends it |
| `SteamRegistry` | `winreg` | a struct |
| `Secrets`, `Clock`, `Settings` | foundation | foundation fakes; settings from fixture TOML |

Tests never call the real `ShellExecute` (foundation review 2.1: the URL test of the process spike opened four tabs in the owner's browser) and never start Windows Terminal, VS Code, Steam or a game.

### 10.3 Fixtures

Trees are described by text files `tests/fixtures/trees/<name>.tree`: one line per entry, `relative\path  size` (size in KB, `/` marks a folder); a harness function `materialize(tree, root)` creates them (files are `set_len(size)`, so they are cheap; the content of `.git\HEAD` and `.acf` files is given inline after the line with `<<` markers). Placeholders `{TEMP}` and `{DATA}` as foundation 12.2. No path of a real user.

| Fixture | Contents |
|---|---|
| `proj-roots-basic.tree` | root `A` with `alpha` (HEAD `ref: refs/heads/main`), `beta` (detached 40-hex), `gamma` (`.git` as a file with `gitdir:` to a folder inside the tree), `delta` (no `.git`), `.hidden` (with `.git`), a loose file; root `B` with `alpha` too (same folder name: collision), and root `C` that does not exist |
| `proj-git-outputs/` | canned git outputs: `status-clean`, `status-3-modified`, `status-rename`, `log-14-days`, `log-empty`, `log-no-commits-error`, `status-dubious-ownership-error` |
| `data-typical/projects.toml` | `[tools]` unset, one named step `dev-server`, one named step `preview`, one `[[project]]` by path for `alpha` listing four steps, `forget_missing_days = 30` |
| `data-typical/games.toml` | one hidden Steam key, one corrected folder game (title, relative exe, cover), one `[[custom]]` |
| `steam-basic.tree` | `steamapps\libraryfolders.vdf` listing the main library and a second library folder inside the tree (and a third that does not exist), `appmanifest_1000001.acf` (installed, name with an apostrophe), `appmanifest_228980.acf` (the redistributable), `appmanifest_1000002.acf` (StateFlags 1026), `appmanifest_1000003.acf` in the second library, `userdata\1111\config\localconfig.vdf` with `Playtime` and `LastPlayed` for two appids, an empty `userdata\2222`, `config\loginusers.vdf`, `appcache\librarycache\1000001\library_600x900.jpg` (a 1 KB placeholder with JPEG magic bytes) and `...\1000003\<40hex>\library_capsule.jpg` |
| `steam-signed-out` | the same without an active user; variants for each fallback rule |
| `steam-broken` | an empty `libraryfolders.vdf`, a manifest cut in half |
| `epic-basic/Manifests/*.item` | six items as listed in 10.1 plus two games that are good |
| `epic-garbage/Manifests/*.item` | three files that are not JSON |
| `game-folders-basic.tree` | the layouts of 6.11: `Fixture Quest`, `Fixture Arena`, `Fixture Realm`, `Ancient Thing`, `Old Game`, `Crash Bandicoot`, a Unicode-named folder, `_CommonRedist` and a loose `readme.txt` at the top, a folder with only an `.exe` named `setup.exe`, an empty folder, and `GameName.v1.2.3-GOG` with `gamename.exe` |
| `guess-lists/*.txt` | the same layouts as flat file lists for the pure `guess` tests |
| `covers/` | tiny files with valid magic bytes for each type, a text file renamed `.png`, a sparse file of 9 MiB |

### 10.4 Integration tests on the harness (`tests/projects.rs`, `tests/games.rs`)

Each uses `TestBackend::builder()` with the fakes above and the fixture trees under a temp folder; settings come from a `settings.toml` fixture whose roots are `{TEMP}` placeholders.

| Test | Steps and expected result |
|---|---|
| `projects_list_basic` | `GET /projects`: `alpha`, `beta`, `gamma` listed with branches `main`, 7-character sha, the branch of the `gitdir` target; `delta` and `.hidden` absent; the second `alpha` gets an id with an `-<8 hex>` suffix; the missing root `C` appears as `failed` in `GET /health` and does not fail the call |
| `projects_git_facts_cached` | first call: `FakeGit` was called for each repository (status and log); second call within 60 s and with the same HEAD files: zero calls; touch `logs\HEAD`: only that repository is asked again; the answer keeps the old values while the refresh runs and a `projects` frame arrives when it ends |
| `projects_first_facts_budget` | `FakeGit` delays 10 s for one repository: the first `GET /projects` returns within about 6 s with `dirtyFiles` absent for it and zeros, then the frame arrives with the facts |
| `projects_no_git` | `[tools] git` unset and no git on the fake `PATH`: every `dirtyFiles` absent, source `git` is `off`, branches present |
| `projects_missing` | list, delete a repository folder, list again: it is `missing = true`, `branch = ""`, keeps its id and `lastOpenedAt`; advance the clock by 31 days: it is gone |
| `projects_order_and_never_opened` | never-opened projects have no `lastOpenedAt` and come after, in score, name order; `activityScore` follows the formula |
| `project_open_full` | `POST /launches` target project: the first answer has step 0 `running`; `FakeOpener` saw exactly the command lines of 6.7 (one `Code.exe` call, one `wt.exe` call with ONE `new-tab` part); `launch` frames end in `success`; `lastOpenedAt` is set before the final frame; `projects` is published |
| `project_open_editor_only` | only the editor line; `terminal` and `claude` modes only their lines; `editorOnly` on a project whose list has no editor step: 400 `noSteps` |
| `project_terminal_mode_default_plan` | default plan `[editor, claude]` and mode `terminal`: exactly one `wt.exe` call with one `new-tab` and no editor line; mode `claude`: one `wt.exe` call with one `new-tab` that runs `claude` |
| `project_open_focus` | with a `FakeWindowLister` window `main.rs - alpha - Visual Studio Code` owned by a fake `Code.exe` process: `open = true`; a `full` launch spawns only the editor line and reports the claude step `success` |
| `project_open_missing_folder` | the folder deleted after the list: one failed step "Folder not found: ...", the others stay idle, run `failed`, no spawn |
| `project_open_tool_missing` | no editor on `PATH`: step 0 fails with the text of 8.2, the other steps still run; `[tools] editor` pointing nowhere gives "Configured program not found" |
| `project_open_double_click` | two launches while the first run is held by the `FakeOpener`: the same run id; one spawn |
| `project_wt_fallback` | the direct `wt.exe` spawn fails: the `ShellExecute` fallback gets the same parameter text |
| `project_game_mode_gate` | with `game_mode_active`: launch and refresh answer 409 `gameMode`; the list still answers |
| `projects_toml_reload` | edit `projects.toml` by hand while running: a `projects` frame arrives and `environment` changes; a syntax error keeps the old steps and shows a `ConfigIssue` in `/health` |
| `projects_example_created` | an empty data folder: after start `projects.toml` exists with only `schema_version` and comments; a second start does not rewrite it |
| `games_library_all_sources` | fixtures for Steam, Epic and a game folder: sorted answer; ids as in 6.1; `sources` has three entries, all `ok`; `lastPlayedAt` is the largest of manifest, localconfig and Rain's record; covers are `media/game-cover/...` |
| `games_epic_absent` | no Epic folder and no registry key: `epic` is `off` with "Epic Games is not installed", no failure, no Epic tab data |
| `games_epic_garbage` | three unparsable items: `epic` is `failed`; the other sources are untouched; no `unavailable` |
| `games_steam_signed_out` | no active user: games listed, playtime 0, state `ok` with the message |
| `games_steam_off_and_missing_path` | `steamEnabled = false`: `off`; a wrong `steamPath`: `failed` with the path in the message |
| `games_folder_missing` | a game folder that does not exist: `custom` is `failed` with "Game folder not found: ..."; games of the other folder still listed |
| `games_folder_guess` | `game-folders-basic`: the program of each game as in 6.11; `exeGuessed = true`; `_CommonRedist`, the loose file, the empty folder and the `setup.exe`-only folder produce no game or a game without `exePath` as designed; a second call reads `games_guess_cache` (a counter on the walker stays at 1) |
| `games_hide_unhide_pin` | pin a game (01 fake pins), `PATCH hidden = true`: the pin is gone, the game stays in the list with `hidden = true`, `games.toml` has the entry; `PATCH hidden = false`: the entry is removed (file back to the header only); idempotent |
| `games_correct_folder_game` | `PATCH exePath` (relative), `title`, `coverPath`: persisted in `games.toml`; the cover file is in `media\`; `exeGuessed` is false; `exePath: ""` returns to the guess; a bad path gives the 400 reasons |
| `games_save_custom` | all validation rows of 6.12; a cover copy lands in `media\` with the hashed name; a program inside a folder game gives 409 `alreadyListed`; the id gets `-2` on a second game with the same title; `DELETE` removes entry and cover; `DELETE` of a folder game is 409 `notRemovable`; deleting an unknown id is 204 |
| `games_launch` | `FakeOpener` sees `steam://rungameid/<appid>`, the Epic URI, and the direct start with the program's folder as working folder; `games_play` and `lastPlayedAt` updated before the final frame; an unknown id is 404; Steam `off` gives the step error text; a game started twice while its watched session lives spawns once |
| `games_session_time` | the `FakeOpener`'s process ends after a scripted time: `playtimeMinutes` grows, a `games` frame arrives; a second backend start (new `TestBackend` over the same local folder) recovers an open session as in 6.13 |
| `games_size_background` | a folder game: first answer has no `sizeBytes`; a `games` frame follows with the sum of the fixture file sizes; the second `GET` has it at once; changing the folder invalidates it |
| `games_refresh_route` | `POST /games/refresh` clears the caches (the walker counter goes up) |
| `games_media` | `GET /media/game-cover/<id>`: 200 with the content type, `ETag`, 304 on `If-None-Match` before any read; unknown id and vanished file: 404 with the JSON body; an id with `..` is just an unknown id |
| `games_toml_reload` | hand-edit `games.toml`: a `games` frame arrives; a syntax error keeps the last good state and `custom` is `failed` with the message of 8.3 |
| `search_pins_provider_rules` (with section 01) | the property test of foundation 11.2: every id returned by `search`, `pinned` and `recent` for projects and games exists in `GET /projects` or `GET /games`; hidden and `off`-source games never appear; missing projects do not appear in search or recent |
| `types_roundtrip` | JSON round-trip with a full and a minimal value of every type of section 3, `assert_no_nulls`, the 400-games and 200-projects frame under `FRAME_MAX` |

Survival test (automated; `tests/process_smoke.rs`, the main risk of this section): build `rain-test-app` (a program that sleeps), start the REAL `rain-backend` binary on a unique pipe, launch the test app through a fixture custom game (`POST /launches` of a game whose program is the test app), then end the backend once with `rain-backend --stop` and once with `TerminateProcess`; assert that the test app pid is still alive and that its parent process id is gone. The same for the editor step (the editor program of the fixture is the test app) and for a launch made by `launch_after_game_mode`. A second case starts the backend inside a kill-on-close job object created by the test; `spawn_detached` must return `Detached::InJob`, and the test asserts the `warn` line. The manual checks of section 11 stay.

### 10.5 Frontend tests (vitest, foundation 12.3)

| Test | What it proves |
|---|---|
| REST client | `listProjects`, `listGames`, `saveCustomGame`, `setGameHidden` (sends `{ hidden }` and returns `undefined`), `updateGame`, `deleteCustomGame`: method, path, body; `coverUrl` and `value` of a `url` icon are rewritten by `resolveMedia` |
| Router | the six methods and two topics go to `real` after stage 5; `launch` of a `project` target goes `real` |
| Games screen logic (pure helper tests; the tab and error computations are moved to `src/features/games/library-view.ts` so they can be tested without mounting) | with `sources` containing `epic: off` there is no Epic tab; with `hidden` games the Hidden tab exists and its count is right; a failed source gives `warn` and its message; the selected tab falls back to `all` when its tab disappears |
| Projects order helper | a list with never-opened projects sorts as in 9.1 (the comparator is a small exported function) |

Screens are not unit tested (as in the foundation).

### 10.6 What cannot be tested automatically, and how it is checked by hand

| Item | Why | Hand check (section 11) |
|---|---|---|
| Real VS Code, Windows Terminal and Claude Code starting, the tab layout, `wt` quoting with `;` | UNPROVEN on a windowless backend; real programs | 11.2 |
| Open detection by window title | no spike; depends on VS Code's title | 11.2 |
| Real Steam folder, sign-out cases | one PC | 11.3 |
| Epic manifests | Epic is not installed on the development PC | 11.3: only the "off" path can be seen; the parser is covered by fixtures built from the documented field names |
| Direct start of a real game, play time, UAC (740) case | needs a game | 11.4 |
| A very large game folder, a network game folder | cost and variance | 11.4 |
| Antivirus interference with `git` or the walker | unknowable | note in the stage report |

---------------------------------------------------------------------------------------------------

## 11. Acceptance checks

Besides the checks of foundation 12.4 (all green). Run in the real app (`npm run tauri dev`, then a release build for 11.2, because a debug build is a sandbox with other folders). Folders below are the owner's own; nothing is typed into code.

### 11.1 Projects list

1. Settings > Project folders: set the folder that holds the repositories. Open Projects. Expected: every repository directly inside it appears, with its branch, a strip of 14 bars, and "N changed files" in the detail pane within a few seconds (the first open may show the facts a moment late: a `projects` frame fills them in without a click).
2. Make a commit in one repository from a terminal. Close and reopen the Projects screen (or press Retry on an error row). Expected: its strip's last bar and its order change; a second reopening within 60 s starts no `git` process (Task Manager, or `Get-Process git` while clicking).
3. Edit a file in a repository, do not stage it. Wait 61 s, reopen Projects. Expected: the changed-file count rises by one.
4. Delete or rename a repository folder. Reopen. Expected: it is dimmed with the folder-x mark and Enter reads "Locate"; after `forget_missing_days` (set it to 0 in `projects.toml` to test) it is gone.
5. Empty: clear Project folders. Expected: "No projects yet" with the button that opens Settings. Set it to a folder with no repository: the same.
6. Rename `git.exe` out of `PATH` (or set `[tools] git` to a wrong path). Expected: no "changed files" line, no error, `GET /health` shows `git` `off`/`failed`. Undo.

### 11.2 Opening a project

1. Select a project, press Enter. Expected: the checklist in the pane shows two steps (editor, Claude) going green one after the other; VS Code opens on the folder; Windows Terminal gets one new tab in the folder that runs `claude`, in the last Terminal window, also when Terminal was closed before. Rain hides afterwards.
2. Press Enter again on the same project. Expected: the label reads "Focus"; VS Code comes forward; no new terminal tabs. If this fails, record the title VS Code shows in the stage report (open detection is unproven).
3. Ctrl+Enter on another project: only VS Code. Ctrl+K "Terminal here": one tab, no Claude. "New Claude session here": one tab running `claude`.
4. Put a `[[step]]` with `kind = "terminal"`, `command = "echo hello; echo there"` in `projects.toml` and use it for one project. Expected: a tab runs both echoes and stays open (this proves the `\;` rule); a step with `kind = "browser"` and `delay_ms = 3000` opens the browser after about 3 s.
5. Set `[tools] editor` to a wrong path. Expected: the first step fails with "Configured program not found: ...", the Claude step still runs, Rain stays visible with the notice.
6. Stop VS Code from being found (rename its folder for a minute). Expected: "VS Code was not found. Set its path in projects.toml" in the notice.

### 11.3 Steam, Epic, covers

1. Open Games. Expected: the installed Steam games, with local covers where Steam has a portrait image (the tools entry "Steamworks Common Redistributables" is not listed); play time and last played match Steam's library. The Epic tab is absent on this PC and `GET /health` shows `epic` `off`, "Epic Games is not installed".
2. Sign out of Steam, restart Steam without signing in (or kill it), reopen Games. Expected: games still listed, play time 0 or the account's last known; no error row.
3. Switch Steam off in Settings. Expected: the Steam tab disappears; no error.
4. Epic (only when it can be installed for the check): manifests appear as games; with the launcher uninstalled again the tab disappears. If it cannot be tested, say so in the stage report.

### 11.4 Game folders, hand-added games, play time

1. Settings > Game folders: add the folder with your installed games. Expected: every direct subfolder is a tile in the "Local" tab, titled with the folder name; `_` folders and loose files are absent; for each game open Ctrl+K > Edit game: the program file is the one a person would start (not a tool, redistributable, crash reporter, art book viewer or launcher). Write down every wrong guess for the stage report; correct it with Edit and check that Enter now starts the right program.
2. Size: the detail pane shows "Size" a few seconds after the first open (a `games` frame, no click). Compare with Explorer's folder properties ("Size").
3. Enter on a local game. Expected: it starts in its own folder (check the working folder in a game that writes a log there), Rain hides, and no process of the game is a child of `rain-backend.exe` (Process Explorer or `Get-CimInstance Win32_Process`, parent id). Close the game; reopen Rain: "Last played" is now, "Playtime" grew by the real time (within a minute).
4. Stop the backend (`npm run backend:stop`) while a local game runs; start it again. Expected: the game is still running, shows the running ring, and its play time is added when it ends.
5. Hide a game: it leaves every tab, the Hidden tab appears with a count of 1, Enter there shows it again; its pin is gone. Add a game by hand with Ctrl+N (a program outside the game folder, with a cover picture): it appears in Local; a second Add of the same program gives "already in the library"; Remove works for it and not for a folder game.
6. Set Game folders to a folder that does not exist. Expected: the error row "Game folder not found: <path>" with Retry on the Local and All tabs; the other sources unaffected. Set it back.
7. Empty: remove all game folders and hand-added games and switch Steam off: "No games found".

### 11.5 Idle and cost

After using all of the above, close the window and leave the backend alone for 60 s: working set under 30 MB and CPU time gained under 0.1 s (foundation stage 2 check); `Get-Process git` shows nothing; no `games` or `projects` frame without a client.

---------------------------------------------------------------------------------------------------

## 12. Open points

Each: the question, who decides, what the code does until then, what it blocks.

| # | Question | Who decides | Until decided | Blocks |
|---|---|---|---|---|
| O1 | SETTLED by the editor: section 01 6.9 wins. This section implements `ItemSource` and `LaunchPlanner`, calls `pinned_ids` and `unpin`, and section 01 now has `LaunchMode` with `terminal` and `claude` (S2.8). | none | as 6.15 | none |
| O2 | SETTLED by decisions.md, ruling of 2026-11-22: ONE Windows Terminal tab in the folder in which Claude Code runs; the tab stays open as a shell after Claude exits. | none | one tab; `[defaults] steps = ["editor", "terminal", "claude"]` in `projects.toml` gives the user a second plain tab | none |
| O3 | `[terminal] window`: a tab in the last Terminal window (`-w 0`, chosen) or always a new window. | the user | `current`, switchable in `projects.toml` | none |
| O4 | Platform functions new to the foundation: `platform::windows::list_windows()` (`EnumWindows`, `GetWindowTextW`, `GetWindowThreadProcessId`; features already in foundation 1.6), `platform::time::local_date_of_unix(secs)` (foundation 11.6), `platform::process::spawn_detached(exe, raw_args, cwd)` returning pid, creation time and `Detached::Free` or `Detached::InJob` (6.14; no wait future: the wait is `open_for_wait`, 6.13), `platform::process::{open_for_wait(pid, creation_time), creation_time(pid)}`, `platform::shell::shell_execute(verb, file, params, dir, flags)` (foundation 11.6). Section 01 needs the last two as well. | architect (core and `platform`) | specified here by signature | stage 5 |
| O5 | Topics `projects` and `games`, the routes `POST /projects/refresh` and `POST /games/refresh`, the methods `updateGame` and `deleteCustomGame`, and the registry rows of 2. | architect | used as written | stage 5 |
| O6 | Settings (section 06) adds `gameFolders` with the validation of 3.3; `steamPath` and `epicPath` get the meanings of 4.4 and are detected by section 06 only; `settings::Service::resolved()` and `watch()` exist; `GET /settings` and `checkPaths` accept the new field. | section 06 | as written | stage 5 |
| O7 | Section 03 sets `ClaudeSession.projectId` through `projects::Service::project_id_for_path(cwd)` and may not read `Project.sessionIds` (removed, S2.3). | section 03 | the method exists as in 6.15 | stage 8 |
| O8 | Section 06 (game mode) calls `games::Service::launch_after_game_mode(id)` before the backend exits and shows `games::Service::get(id)` for the next game. | section 06 | as in 6.14 | stage 9 |
| O9 | `windows` and `process` facts that are UNPROVEN and handled defensively: open detection by window title; `wt.exe` started directly from the windowless backend (fallback `ShellExecuteW`); `;` handling in `wt` arguments; Steam with several accounts; the Epic manifest format. | none: the stage 5 hand checks (11.2, 11.3) decide; findings go to the stage report | defensive designs of 6.4, 6.7, 6.9, 6.10 | none |
| O10 | A Steam game that ends while the Games screen is open keeps its running ring until the next fetch, because Steam's `RunningAppID` would need a registry change notification or a timer. `RegNotifyChangeKeyValue` on a blocking thread would fix it without polling, but was not proven and adds a thread. | architect | not built; the screens refetch on show | none |
| O11 | A launcher stub that starts the real game and exits (6.13) is followed by one process snapshot. A game that starts its real process much later than the stub's exit is under-counted. | accepted | documented limit | none |
| O12 | Product choice on Epic and Steam "not installed" games, store pages and online covers: not built (decisions.md section 5, foundation 13.2). Revisit only if the user asks. | the user | as in 1.2 | none |
| O13 | `LIBRARY_TTL`, `OPEN_TTL`, `RUNNING_TTL` and `DIRTY_TTL` are read-time checks, not timers, and are named in 6.16; the architect may want them in `rain_types::limits`. | architect | constants in this module | none |
