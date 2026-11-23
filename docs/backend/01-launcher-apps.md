# Rain backend, section 01: launcher and apps

| | |
|---|---|
| Area | launcher and apps: search, pins, recent, apps, pools, launching of apps and pools, the `launch` topic, app icons |
| Module | `launcher` (`crates/rain-backend/src/modules/launcher/`), plus the OS code it adds to `platform/` |
| Stage | 4 (foundation section 14.3). Nothing of this section is built earlier. |
| Author | feature designer for section 01 |
| Date | 2026-11-22 |
| Status | draft 1. Binding inputs: `decisions.md`, `00-foundation.md` (draft 2), `foundation-review.md`. Where this section disagrees with the foundation it says so under "Open points". |

Reading guide: sections 1 to 8 are for the backend, section 9 for the frontend, sections 10 and 11 for testing. Words are used as in foundation section 0 (module, topic, frame, shell, backend, data folder, local folder, stage).

---------------------------------------------------------------------------------------------------

## 1. Scope

### 1.1 What this area does

Rain's launcher side: find things, pin them, remember what was used, list installed apps, keep pools (named groups of apps that start together), and start apps and pools. It also runs the one launch engine that section 02 reuses for projects and games.

| Item | Covered here |
|---|---|
| `RainApi` methods | `search`, `listPinned`, `listRecent`, `setPinned`, `listApps`, `listPools`, `savePool`, `deletePool`, `launch`, `getLaunch`, and one new method `rescanApps` (section 9.2, contract change 1.4) |
| Topic | `launch` (payload `LaunchRun`). The topic is shared with section 02: the engine is built here and runs plans for all four target types |
| Media kind | `app-icon` |
| Data files (foundation 10.2) | `pins.toml`, `pools.toml`, `app-ids.toml` |
| SQLite tables | `launcher_app` (cache), `launcher_launch` (history) |
| Platform code | `platform/shell.rs` (app list, icon render, shell execute, activate), `platform/process.rs` (`list_images`) |
| Decisions implemented | C3 (readable stable app ids in a TOML map), C4 (icons as media with relative references), C10 (ShellExecute for shortcuts, URIs and packaged apps; launched programs never in a kill-on-close job), and the "nothing hardcoded" rule |

### 1.2 What it does not do

- Disk "reveal" (`revealPath`, route `POST /shell/reveal`): section 05 builds it in stage 3. "Open file location" of an app calls it.
- Projects, games, sessions, jobs, idle jobs: their lists, ids and facts belong to sections 02 and 03. This section only reads them through two small traits (section 6.9) so that search, pins and recent can include them.
- Pool launch of a game or a project step is not a pool concern. The launch engine of this section runs the steps that section 02 plans (section 6.8).
- Hotkeys per pool, capturing open windows into a pool, reordering pins (spec section 10): out of scope.

### 1.3 Plan by stage

| Part | Built in |
|---|---|
| Everything in sections 2 to 9 for target types `app` and `pool`, the `launch` topic, media kind `app-icon`, `rescanApps` | stage 4 |
| The extension points for other modules (`ItemSource`, `LaunchPlanner`, section 6.9), stubbed in stage 4 so that the stubs of modules 02 and 03 compile | stage 4 |
| Search, pins and recent for project and game items; launch of project and game targets | stage 5 (section 02 fills in the two traits for its two modules) |
| Search of jobs and idle jobs | stage 6, built here in `launcher/search.rs` from `jobs().list()` and `idle().list()`; `REAL_SEARCH` in the router gains `job` and `idleJob` (section 03 lists the task in its 9.3) |
| Search of sessions | stage 8, built here in `launcher/search.rs` from `sessions().list()`; `REAL_SEARCH` gains `session` (section 03 lists the task in its 9.3) |
| Removal of the router special cases of this area | stage 8 (last of the search types) and stage 5 for launch; the mock files go in stage 10 (section 9.6) |

### 1.4 The single sources, in one table (the points the task asked to settle)

| Question | Decision | Reason |
|---|---|---|
| Source of the app list | `shell:AppsFolder` only. No Start Menu `.lnk` walk | Superset of the Start Menu: 125 items in 190 to 200 ms, every item has an AUMID, 88 have a target path, 25 have arguments, 37 are packaged (spike winapi Q1). One source, one pass, no merge rules |
| App id | `app-<slug of the name>`, with `-2`, `-3` on a clash, issued once and written to `app-ids.toml`, keyed by AUMID (or by target plus arguments for generated AUMIDs) | Readable, stable across rescans, renames and cache loss (decision C3) |
| `AppEntry` of a packaged app | `exePath` becomes optional and `kind: 'program' \| 'store' \| 'other'` is added | A Store app has no file. The UI shows "Store app" and hides "Open file location" |
| Icons | rendered lazily from `shell:AppsFolder\<AUMID>` by `IShellItemImageFactory`, 64 px PNG, cached on disk, served as media `app-icon`; the reference carries a content version `v` | Spike winapi Q2: all 125 items rendered, 2.7 s in total, 480 KB |
| Scoring | exact port of `src/lib/score.ts` to Rust, with golden vectors read by vitest and by Rust (section 6.3) | Home sorts items and commands by one number; the frontend contract (`api.search`) stays |
| Search over other modules | each module exposes `search_entries()` through the `ItemSource` trait; the launcher scores them | One ranking, one cap, one hidden-item rule (the owner applies it) |
| Launch of apps and pools | sequential steps, one `launch` frame per finished step, a final frame always, the run kept 10 minutes in memory, `getLaunch` as the recovery call (the frontend polls it, section 9.3) | Matches the contract the UI was built on and fixes the "lost last frame" hazard of the contract report |
| `running` | computed at `GET /apps` from a process snapshot (4.7 ms), cached 2 s, plus an 8 s "just launched" grace | No timer; best effort for Store apps and launcher shims |
| Recent and last-launched times | SQLite table `launcher_launch`, read into memory at start | Machine history belongs in SQLite |
| Pins | `pins.toml`, one ordered list across the four pinnable types | User data belongs in the data folder |
| Apps empty state | "No apps found" with the button "Scan again" (calls `rescanApps`); the old button "Open settings" is removed because Settings has nothing for apps | The only thing a user can do about an empty list is to ask for a new scan |

---------------------------------------------------------------------------------------------------

## 2. Routes

All paths are after `/api/v1`. The statuses and error body follow foundation sections 4.5 and 4.6. "Idempotent" says what a repeated call does.

| Method | Path | Request | Response | Success | Errors (code, reason) | Idempotent | Topic effects |
|---|---|---|---|---|---|---|---|
| GET | `/search` | query `q` (text, default empty), `types` (comma list of `SearchType`, default all) | `SearchItem[]` | 200 | 400 invalid `badQuery`, `queryTooLong`, `unknownType` | yes (read) | none |
| GET | `/pinned` | none | `SearchItem[]` in pin order, `score` 0 | 200 | 503 unavailable `moduleFailed` | yes | none |
| GET | `/recent` | query `limit` (1 to 50, default 12) | `SearchItem[]`, newest first, `score` 0 | 200 | 400 invalid `badLimit` | yes | none |
| PUT | `/pinned/{type}/{id}` | body `PinBody { pinned: boolean }` | none | 204 | 400 invalid `unknownType`, `badBody`; 404 notFound `unknownItem` (only when pinning an unknown item); 409 conflict `configBroken` (pins.toml has an error); 500 failed `saveFailed` | yes: pinning a pinned item keeps its place and writes nothing; unpinning a missing pin is a no-op | none |
| GET | `/apps` | none | `AppEntry[]`, sorted by name | 200 | 503 unavailable `scanFailed` (no cache and the scan failed), `scanning` (first scan over 10 s), `comBusy` | yes | none |
| POST | `/apps/scan` | none | `AppEntry[]` (fresh) | 200 | 409 conflict `gameMode`; 503 unavailable `scanFailed`, `comBusy` | yes: a scan already running is shared | none |
| GET | `/pools` | none | `Pool[]` in file order | 200 | 503 unavailable `moduleFailed` | yes | none |
| POST | `/pools` | body `PoolDraft` without `id` | `Pool` | 201 | 400 invalid (section 8); 409 conflict `configBroken`; 500 failed `saveFailed` | no (creates a second pool); the UI guards with the disabled Save button | none |
| PUT | `/pools/{id}` | body `PoolDraft` (`id` absent or equal to the path) | `Pool` | 200 | 400 invalid (section 8), `idMismatch`; 404 notFound `unknownPool`; 409 `configBroken`; 500 `saveFailed` | yes (same body, same file) | none |
| DELETE | `/pools/{id}` | none | none | 204 | 409 `configBroken`; 500 `saveFailed` | yes: an unknown id is 204 (foundation 4.6); the pool's pin and history rows are removed with it | none |
| POST | `/launches` | body `LaunchTarget` | `LaunchRun` (status `running`, step 0 `running`, the others `idle`) | 201 for a new run, 200 when the run of the same target is still running and is returned | 400 invalid `badArgs`, `argsNotAllowed`; 404 notFound `unknownApp`, `unknownPool`; 409 conflict `gameMode`; 503 unavailable `shuttingDown`, `notBuilt` (project or game before stage 5) | yes: a second call for a target that is running returns that run | topic `launch`, section below |
| GET | `/launches/{id}` | none | `LaunchRun` (current snapshot) | 200 | 404 notFound `unknownRun` | yes | none |
| GET | `/media/app-icon/{id}` | query `v` | PNG | 200, 304 | 404 notFound `unknownApp`, `noIcon` | yes | none |
| PUT | `/internal/test/apps-source` | body `{ "mode": "normal" \| "empty" \| "fail" }` | none | 204 | 404 notFound `noRoute` when not a sandbox | yes | none. Dev only (section 11.4) |

Notes:
- The app id and the pool id in paths are percent-encoded single segments (foundation 4.1). Ids use only `A-Za-z0-9._~-`, so in practice nothing needs encoding.
- `GET /search` with an empty or blank `q` returns `[]` with 200 (the UI never sends it, the mock did the same).
- `POST /launches` for `type: project` or `game` is routed to the planner of section 02 (section 6.9). Until stage 5 the planner stub answers `503 unavailable notBuilt`, message "Launching this kind of item is not available yet". The frontend router never sends these types to the backend before stage 5, so the user never sees it.
- Path parameters are read with `ApiPath` (an `ApiQuery`-style extractor with the one error body). See open point 8.
- `revealPath` (`POST /shell/reveal`) is section 05.

### Live topics

| Topic | Payload | Kind | Maximum rate | Publisher and when |
|---|---|---|---|---|
| `launch` | `LaunchRun` | one item, key `run.id` (the frontend replaces `runs[target.id]`) | at most 4 per second for the frames that are not final (`Throttle`, newest wins); the final frame is never delayed or dropped | the launch engine of this module, for every run of every target type: once after each finished step (the step that ends is `success` or `failed`, the next one is `running`), and once at the end with `status` `success` or `failed`. No frame at creation: the `201` answer is the first state |

The Topic struct is `Launch` (payload `LaunchRun`), in `crates/rain-types/src/events.rs`, added in stage 4 (foundation 5.3).

Order inside the engine at the end of a run (contract report finding 5, section 2.7): (1) set the final status, (2) write the history rows to SQLite, (3) update the in-memory "last launched" values and invalidate the process snapshot, (4) call the planner's `finished` hook, (5) cancel any pending throttled frame, (6) publish the final frame. The frontend refetches `apps`, `pools` and `recent` when it sees the final frame, so everything it reads is already up to date.

---------------------------------------------------------------------------------------------------

## 3. Types

Rust types go to `crates/rain-types/src/launcher.rs` (the file of this area, foundation 9.1) and one line each to the `decls!` list of `bindings.rs`. The Rust name equals the TypeScript name. All follow foundation 9.2: `rename_all = "camelCase"`, `#[ts(optional_fields)]` on the struct, `#[serde(default, skip_serializing_if = "Option::is_none")]` on every optional field, `#[ts(inline)]` on every field whose type is a string enum, `#[serde(deny_unknown_fields)]` on every request-body type. `u32` becomes `number`.

### 3.1 Change to a type of stage 1a: `IconRef`

`IconRef` is generated since stage 1a (`common.rs`). One optional field is added. It is additive (foundation 4.1).

```rust
// crates/rain-types/src/common.rs (existing struct, one new field at the end)
pub struct IconRef {
    #[ts(inline)] pub kind: IconKind,                 // lucide | brand | url | letter
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub tint: Option<String>,
    /// For kind `url` only: the text whose first letter is drawn as a tile when the image cannot be loaded.
    #[serde(default, skip_serializing_if = "Option::is_none")] pub fallback: Option<String>,
}
```

TypeScript after generation: `{ kind: 'lucide' | 'brand' | 'url' | 'letter'; value: string; tint?: string; fallback?: string }`.

### 3.2 New types in `launcher.rs`

```rust
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use crate::common::{IconRef, Status};

/// What a search item can be. camelCase gives "idleJob".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SearchType { App, Pool, Project, Game, Session, Job, IdleJob }

/// The four types that can be pinned and launched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PinnableType { App, Pool, Project, Game }

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct SearchItem {
    #[ts(inline)] pub r#type: SearchType,               // emitted as `type`
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub subtitle: Option<String>,
    pub icon: IconRef,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(inline)] pub status: Option<Status>,
    pub score: u32,                                     // scale of src/lib/score.ts, 0 for pins and recent
}

/// How the app is started and what the UI may show for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum AppKind { Program, Store, Other }              // program: has a file; store: packaged app; other: shell object, no file

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    pub icon: IconRef,                                  // kind url, value "media/app-icon/<id>?v=<hex8>", fallback = name
    #[ts(inline)] pub kind: AppKind,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub exe_path: Option<String>,   // absent for kind store and other
    pub keywords: Vec<String>,
    pub pinned: bool,
    pub running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub last_launched_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(optional_fields)]
pub struct PoolMember {
    pub app_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub args: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub delay_ms: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct Pool {
    pub id: String,
    pub name: String,
    pub icon: IconRef,                                  // always { kind: letter, value: name }, derived, never stored
    pub members: Vec<PoolMember>,
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub last_launched_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(optional_fields)]
pub struct PoolDraft {
    #[serde(default, skip_serializing_if = "Option::is_none")] pub id: Option<String>,
    pub name: String,
    pub members: Vec<PoolMember>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct PinBody { pub pinned: bool }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum LaunchType { App, Pool, Project, Game }        // same four values as PinnableType; a separate type because the TypeScript has two unions

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum LaunchMode { Full, EditorOnly, Terminal, Claude }   // terminal and claude: "Terminal here" and "New Claude session here" of section 02

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(optional_fields)]
pub struct LaunchTarget {
    #[ts(inline)] pub r#type: LaunchType,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")] #[ts(type = "'full' | 'editorOnly' | 'terminal' | 'claude'")] pub mode: Option<LaunchMode>,
    /// Only for type `app`: extra command-line text for this launch (a pool's "Retry failed" passes the member's `args`).
    #[serde(default, skip_serializing_if = "Option::is_none")] pub args: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct LaunchStep {
    pub label: String,
    pub icon: IconRef,
    #[ts(inline)] pub status: Status,                   // the engine uses idle, running, success, failed
    #[serde(default, skip_serializing_if = "Option::is_none")] pub error: Option<String>,
}

/// The status of a whole run: a subset of `Status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus { Running, Success, Failed }

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LaunchRun {
    pub id: String,
    pub target: LaunchTarget,
    #[ts(inline)] pub status: RunStatus,
    pub steps: Vec<LaunchStep>,
}
```

Generator notes:
- `#[ts(inline)]` on `Option<Status>` and `#[ts(type = ...)]` on `Option<LaunchMode>` are defensive: nobody has run `Option<enum>` fields through the generator yet (foundation 9.2, last paragraph). Stage 4 runs the strict type-identity check for these types before it deletes the hand-written ones. If `#[ts(inline)]` on an `Option` does not give `status?: Status` and the literal union, the `#[ts(type = "...")]` form is used on that field, as already done for `mode`.
- `Status` and `IconKind` come from `common.rs` (stage 1a; foundation 9.1 defines `IconKind`: `Lucide, Brand, Url, Letter`, camelCase). This section only adds the `fallback` field to `IconRef`.
- Query types of the routes (`SearchParams { q, types }`, `RecentParams { limit }`) are plain backend structs with `deny_unknown_fields`; they never reach the frontend and are not in `decls!`.
- Add to `decls!`: `SearchType, PinnableType, SearchItem, AppKind, AppEntry, PoolMember, Pool, PoolDraft, PinBody, LaunchType, LaunchMode, LaunchTarget, LaunchStep, RunStatus, LaunchRun`.
- Every type gets the JSON round trip test with `assert_no_nulls` (foundation 9.2, last row).

### 3.3 Resulting TypeScript (what `src/bindings/rain-types.ts` gets; shown in the repo's style for reading)

```ts
export type SearchType = 'app' | 'pool' | 'project' | 'game' | 'session' | 'job' | 'idleJob'
export type PinnableType = 'app' | 'pool' | 'project' | 'game'
export interface SearchItem { type: SearchType; id: string; title: string; subtitle?: string; icon: IconRef; status?: Status; score: number }
export type AppKind = 'program' | 'store' | 'other'
export interface AppEntry { id: string; name: string; icon: IconRef; kind: AppKind; exePath?: string; keywords: string[];
  pinned: boolean; running: boolean; lastLaunchedAt?: string }
export interface PoolMember { appId: string; args?: string; delayMs?: number }
export interface Pool { id: string; name: string; icon: IconRef; members: PoolMember[]; pinned: boolean; lastLaunchedAt?: string }
export interface PoolDraft { id?: string; name: string; members: PoolMember[] }
export interface PinBody { pinned: boolean }
export type LaunchType = 'app' | 'pool' | 'project' | 'game'
export type LaunchMode = 'full' | 'editorOnly' | 'terminal' | 'claude'
export interface LaunchTarget { type: LaunchType; id: string; mode?: 'full' | 'editorOnly' | 'terminal' | 'claude'; args?: string }
export interface LaunchStep { label: string; icon: IconRef; status: Status; error?: string }
export type RunStatus = 'running' | 'success' | 'failed'
export interface LaunchRun { id: string; target: LaunchTarget; status: RunStatus; steps: LaunchStep[] }
```

### 3.4 What changes against `src/data/types.ts` and why

| Hand-written (lines of `types.ts`) | Change | Why |
|---|---|---|
| line 11 `IconRef` (already generated in stage 1a) | adds `fallback?: string` | A failed image must fall back to a letter tile (section 9.4); today `RIcon` has no `@error` handling |
| lines 14 and 15 `SearchType`, `PinnableType` | none in shape; deleted (generated) | Rust is the source of truth |
| line 17 `SearchItem` | none | |
| lines 20 and 21 `AppEntry` | `exePath: string` becomes `exePath?: string`; new required `kind: AppKind` | 37 of 125 apps on this PC are Store apps with no file (spike winapi Q1); a fake path string would make "Open file location" and "Copy path" lie |
| line 22 `PoolMember` | none | |
| lines 23 and 24 `Pool`, `PoolDraft` | none | |
| line 27 `LaunchTarget` | adds `args?: string` | The "Retry failed" button of a pool launched plain apps and lost the member's `args` (contract report 6.1); with real args that is a visible bug |
| lines 28 and 29 `LaunchStep`, `LaunchRun` | none | |
| new `PinBody`, `LaunchType`, `LaunchMode`, `RunStatus`, `AppKind` | additions | `PinBody` is the request body; the others name unions the generator needs as types |

`LaunchType` has the same four values as `PinnableType`; the hand-written file wrote the union inline in `LaunchTarget.type`. Both are generated as named unions; no screen is affected.

---------------------------------------------------------------------------------------------------

## 4. Data files

Common rules of foundation 10.3 apply to all three files: `schema_version` first (missing means 1), a newer version is read-only, snake_case keys, `[[array]]` tables, Rain writes with `toml_edit` and keeps comments and order, a missing file means defaults, `serde(default)` on every field, unknown keys are kept and ignored. Rain creates a file at its first write and never deletes one.

Validation policy (decision of this section, reason in one line): `pins.toml` and `pools.toml` are strict, because a half-loaded pool list would silently hide a pool; if any item is invalid the whole file is refused, the last good value is kept and a `ConfigIssue` names the problem (foundation 10.4). `app-ids.toml` is lenient: a bad entry is skipped and the rest is used, because a refused id map would give every app a new id and break pins and pools.

### 4.1 `pins.toml` (owner: this section; row of foundation 10.2)

| Key | Type | Default | Validation |
|---|---|---|---|
| `schema_version` | integer | 1 | |
| `[[pin]]` | array of tables | none | at most 200 entries; the order is the pin order |
| `pin.type` | string | required | one of `app`, `pool`, `project`, `game` |
| `pin.id` | string | required | 1 to 128 characters of `A-Za-z0-9._~-` |

Duplicate `(type, id)` pairs: the first counts, the file is accepted. Unknown ids are kept (dangling pins are skipped when listing, never deleted; the entity may come back, for example a project folder on a drive that is not connected).

```toml
# pins.toml: what is pinned to Home, in order. Rain edits this file when you pin or unpin;
# you may reorder the entries or add one by hand.
schema_version = 1

[[pin]]
type = "pool"
id = "pool-web-dev"

[[pin]]
type = "app"
id = "app-notepad"
```

Rain writes: `PUT /pinned/{type}/{id}` appends a `[[pin]]` (pin) or removes the matching table (unpin). Pools deleted through Rain lose their pin in the same write sequence (the delete route removes it). A hand edit takes effect after the 300 ms debounce; the UI shows it at its next fetch. Removing a `[[pin]]` removes the comment lines directly above it (foundation 10.4); the file header comment above `schema_version` is not touched.

### 4.2 `pools.toml`

| Key | Type | Default | Validation |
|---|---|---|---|
| `schema_version` | integer | 1 | |
| `[[pool]]` | array of tables | none | at most 100 pools; the order is the display order (the UI does not sort pools) |
| `pool.id` | string | required | `pool-` prefix, then 1 to 100 characters of `A-Za-z0-9._~-`; unique in the file |
| `pool.name` | string | required | after trim 1 to 80 characters |
| `pool.members` | array of inline tables | required | 1 to 40 entries; order = start order |
| `members[].app` | string | required | an app id: 1 to 128 characters of `A-Za-z0-9._~-`. It is not checked against the app list (a pool may name an app that is gone; the UI shows "Unknown app") |
| `members[].args` | string | none | at most 2000 characters, no line break; passed as the raw command-line tail |
| `members[].delay_ms` | integer | 0 | 0 to 60000; the sum over a pool must not exceed 600000 |

```toml
# pools.toml: groups of apps that start together. Rain rewrites a pool when you save it in the pool editor.
# The editor cannot set `args` or `delay_ms` yet; set them here, then reopen Rain's Apps screen.
schema_version = 1

[[pool]]
id = "pool-web-dev"
name = "Web dev"
members = [
  { app = "app-vs-code" },
  { app = "app-terminal" },
  { app = "app-chrome", args = "--new-window http://localhost:5173", delay_ms = 2000 },
]
```

Rain writes: create (append a `[[pool]]` table with a fresh id), update (replace `name` and `members` of the table with that `id`; every other key and comment of the table stays; comments inside the old `members` array are lost, a documented limit), delete (remove the table and the comment lines above it). Ids are never changed by a rename. A hand edit may change anything that passes validation; a file that fails validation is refused as a whole and the writes of Rain return `409 conflict`, reason `configBroken`, until the user fixes it.

### 4.3 `app-ids.toml` (decision C3)

One `[[app]]` entry for every app that Rain has ever shown. It holds the id (what pins, pools and history point to) and optional per-app overrides. It stays one file: the overrides are optional keys of the same table, so no new file is needed.

| Key | Type | Default | Meaning and validation |
|---|---|---|---|
| `schema_version` | integer | 1 | |
| `app.id` | string | required | starts with `app-`, then up to 100 characters of `A-Za-z0-9._~-`; unique. A bad or duplicate entry is skipped |
| `app.key` | string | required | the match key, lower case (section 6.2). Unique; the first entry wins |
| `app.name` | string | none | the name when the id was issued, for the human reader. Rain never rewrites it |
| `app.hidden` | boolean | false | `true` removes the app from `listApps`, search, pools (shown as "Unknown app") and pins |
| `app.keywords` | array of strings | none | extra words the app is found by; at most 12, each at most 40 characters |

```toml
# app-ids.toml: the stable id of every app Rain has listed. Pins and pools refer to these ids,
# so do not change an `id` unless you also change pins.toml and pools.toml.
# Optional per app:  hidden = true   keywords = ["editor", "ide"]
schema_version = 1

[[app]]
id = "app-notepad"
key = 'aumid:microsoft.windowsnotepad_8wekyb3d8bbwe!app'
name = "Notepad"

[[app]]
id = "app-vs-code"
key = 'aumid:{6d809377-6af0-444b-8957-a3773f02200e}\microsoft vs code\code.exe'
name = "Visual Studio Code"
keywords = ["editor"]

[[app]]
id = "app-old-tool"
key = 'path:c:\tools\oldtool\oldtool.exe|'
name = "Old tool"
hidden = true
```

Rain writes: when a scan finds keys that have no entry, it appends one `[[app]]` per new key (`id`, `key`, `name`) in a single write, and writes nothing when there is nothing new. The first write creates the file with the header comment above. Entries of uninstalled apps stay (a reinstall gets its old id back); the user may delete them by hand. If the file has a syntax error, the last good map is used, new apps get provisional ids made by the same function (usually equal to what would have been written) and nothing is written until the file is fixed (a `ConfigIssue` shows in `GET /health`).

---------------------------------------------------------------------------------------------------

## 5. Tables

Both tables belong to the module `launcher` (foundation 10.5). One migration, `crates/rain-backend/src/modules/launcher/migrations/0001_create_launcher.sql`, listed in `pub const MIGRATIONS: &[Migration] = &[Migration { id: 1, name: "create_launcher", sql: include_str!("migrations/0001_create_launcher.sql") }]`.

```sql
-- Rebuildable cache: the raw items of the last scan of shell:AppsFolder. Replaced as a whole at every scan.
CREATE TABLE launcher_app (
  key          TEXT PRIMARY KEY,        -- the match key of app-ids.toml
  aumid        TEXT NOT NULL,
  name         TEXT NOT NULL,
  target       TEXT NOT NULL,           -- System.Link.TargetParsingPath, may be empty
  args         TEXT NOT NULL,           -- System.Link.Arguments, may be empty
  target_mtime INTEGER,                 -- seconds since 1970 of the target file, NULL when not a file
  scanned_at   TEXT NOT NULL            -- RFC 3339, same format as the API
) STRICT;

-- History: one row per successful use. Not rebuildable (losing it loses "recent" and "last launched").
CREATE TABLE launcher_launch (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  target_type TEXT NOT NULL CHECK (target_type IN ('app', 'pool')),   -- projects and games keep their own history (section 02: projects_opened, games_play)
  target_id   TEXT NOT NULL,
  at          TEXT NOT NULL,            -- RFC 3339 UTC with 3 fraction digits
  run_id      TEXT NOT NULL
) STRICT;
CREATE INDEX idx_launcher_launch_target ON launcher_launch (target_type, target_id, at DESC);
CREATE INDEX idx_launcher_launch_at ON launcher_launch (at DESC);
```

| Table | Cache or history | Written when | Retention |
|---|---|---|---|
| `launcher_app` | cache | at the end of a successful scan (one transaction: delete all, insert all) | replaced at each scan; at most a few hundred rows |
| `launcher_launch` | history | at the end of a run: one row per successful app step, plus one row for the pool when the whole run succeeded. Project and game targets write NO row here: their history is the tables of section 02 (`projects_opened`, `games_play`), written by their `LaunchPlanner::finished` | after each insert, rows of that target beyond the newest 20 are deleted; at backend start, rows older than 365 days are deleted; deleting a pool deletes its rows. Rows of targets that no longer exist stay until these rules remove them |

Failed launches are not stored: "recent" shows only what worked, and the failure is on screen at the time.

At backend start the module reads `launcher_launch` once into memory: a map `(target_type, target_id) -> newest at`. Every later read (`listApps`, `listPools`, `listRecent`) uses the map; SQLite is touched only by writes and at start.

Cache folder (foundation 10.6): `<local>\cache\launcher\icons\<hex16>.png`, at most 400 files and 8 MB; the folder is cleaned after each scan (files whose name is not in the current icon list are deleted).

---------------------------------------------------------------------------------------------------

## 6. Algorithms

Constants of this module (file `launcher/limits.rs`; none is shared with other crates):

| Name | Value | Used for |
|---|---|---|
| `APPS_SCAN_MAX_AGE` | 10 min | a read finds the cache older than this: it returns the cache and starts a rescan (checked at read time, no timer) |
| `START_MENU_QUIET` | 2 s | the `quiet` argument of `watch_dir`: quiet time after the last Start Menu change before the hub reports it (no timer in this module) |
| `FIRST_SCAN_WAIT` | 10 s | longest wait of `GET /apps` for the very first scan |
| `PROCESS_TTL` | 2 s | age of the process snapshot reused by `GET /apps` |
| `START_GRACE` | 8 s | an app launched by Rain counts as running for this long when no process is seen yet |
| `STEP_TIMEOUT` | 10 s (`COM_CALL_TIMEOUT`) | one start call |
| `RUN_KEEP`, `RUN_MAX` | 10 min, 64 | finished runs kept in memory |
| `SEARCH_MAX`, `RECENT_MAX`, `QUERY_MAX` | 50, 50, 200 characters | caps |
| `ICON_PX` | 64 | icon size |
| `ICON_RENDER_VERSION` | 1 | bump to make every icon reference change |
| `POOL_MAX`, `POOL_MEMBERS_MAX`, `POOL_NAME_MAX`, `ARGS_MAX`, `DELAY_MAX`, `DELAY_SUM_MAX` | 100, 40, 80, 2000, 60000 ms, 600000 ms | pool limits |

### 6.1 The app list: source, scan, cache

Code to copy: spike `winapi\src\bin\apps.rs` (`enum_apps_folder`, `co_string`). Everything COM runs on the one COM thread (foundation 11.6).

`platform::shell::list_apps() -> Result<Vec<RawApp>>` where `RawApp { aumid, name, target, args }`, all `String`:

1. Closure on the COM thread (`platform::com::run`), wrapped by the caller in `tokio::time::timeout(COM_CALL_TIMEOUT)`.
2. `let folder: IShellItem = SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)?;` then `folder.BindToHandler::<IEnumShellItems>(None, &BHID_EnumItems)`.
3. Loop `Next`. For each item: `GetDisplayName(SIGDN_NORMALDISPLAY)` is the name; cast to `IShellItem2`; `GetString(&PKEY_AppUserModel_ID)` is the AUMID (its parsing name is equal), `GetString(&PKEY_Link_TargetParsingPath)` the target, `GetString(&PKEY_Link_Arguments)` the arguments. A missing property is the empty string. Free every `PWSTR` with `CoTaskMemFree` (the spike's `co_string`).
4. Return plain strings only. Cost: 190 to 200 ms for 125 items (cold and warm).

Scan procedure (`launcher::apps::scan`, single flight: a second caller waits for the running scan and shares its result):

| Step | Where | What |
|---|---|---|
| 1 | COM thread | `list_apps()`. On error or timeout: keep the old cache, set `health` source `apps` to `failed` ("Could not read the installed apps"), return the error |
| 2 | blocking thread | for every item whose target is a file path: `std::fs::metadata(target)` for the modified time (`target_mtime`), 88 calls, a few ms. An error is not a failure (the value is NULL) |
| 3 | async | compute the `key` of every item (6.2), drop duplicates (first wins) |
| 4 | blocking thread | SQLite: one transaction, delete all rows of `launcher_app`, insert the items |
| 5 | async | rebuild the in-memory list (6.2 to 6.4): filter, ids, kinds, keywords. If new keys appeared, write `app-ids.toml` once (`TomlFile::edit`) |
| 6 | async | clean the icon cache folder; clear the "no icon" memory; set `health` source `apps` to `ok`; remember `scanned_at` |

A scan that returns zero items while the cache has items is treated as a failure (Windows always lists apps; an empty answer means the shell did not answer properly). A first scan that returns zero items is a valid empty list.

When scans run:

| Trigger | Rule |
|---|---|
| Backend start (`ModuleEntry::start`) | load `launcher_app` from SQLite into memory first (instant list), then run a scan in a spawned task unless game mode is active |
| `GET /apps` | if the memory list was never filled (no cache and no scan finished): wait for the running scan up to `FIRST_SCAN_WAIT`, then answer or `503 scanning`. Otherwise answer from memory; if the list is older than `APPS_SCAN_MAX_AGE` start a background rescan (single flight) |
| `POST /apps/scan` | run a scan now (or join the running one) and answer with the fresh list |
| Start Menu change | the module asks the config hub to watch `%ProgramData%\Microsoft\Windows\Start Menu\Programs` and `%APPDATA%\Microsoft\Windows\Start Menu\Programs` recursively through `ConfigHub::watch_dir(path, true, START_MENU_QUIET)` (`RecursiveMode::Recursive`, UNPROVEN, see open point 5). The hub does the debounce (`quiet = START_MENU_QUIET`, 2 s); the module runs one rescan on each event of the subscription and has no timer of its own. Both folder paths come from the environment variables `ProgramData` and `APPDATA`, never from a literal |
| Game mode active | no scan is started (foundation 11.5); reads still work |

Store apps installed or removed without a Start Menu change are picked up by the 10-minute rule or by "Scan again".

### 6.2 Filter, kind, key, id

**Filter.** Applied when the list is rebuilt from the raw rows (so the rules can change without a new scan). An item is dropped when any rule is true (`lower` means ASCII lower case of the whole text; `stem` is the target file name without folder and extension):

| # | Rule | Reason (from spike winapi Q1) |
|---|---|---|
| F1 | name is empty after trim | nothing to show |
| F2 | target starts with `http://` or `https://` | web links |
| F3 | target contains `://` (a URI such as `steam://...` or `com.epicgames.launcher://...`) | game links duplicate the game source of section 02 (Marvel Rivals appeared twice) |
| F4 | target ends with `.url`, `.html`, `.htm`, `.pdf`, `.txt`, `.chm`, `.rtf`, `.md` | documents and links |
| F5 | name contains `readme`, `release notes`, `documentation`, `user guide` or `user manual` | documentation |
| F6 | name contains `uninstall`, or the target stem starts with `unins`, or the target stem is `msiexec` and the arguments contain `/x` or `/uninstall` | uninstallers |
| F7 | the entry for the key has `hidden = true` in `app-ids.toml` | the user's choice |

Kept on purpose: shell objects (`::{CLSID}` targets), `.msc` and `.cpl` tools, items with an empty target (they have an AUMID and start through the shell). Never deduplicate on the name (two "Photos" exist; three Riot items share one exe with different arguments); duplicates are removed only on the key.

**Kind.** `store` when the AUMID contains `!`. Otherwise `program` when the target is not empty and does not start with `::`. Otherwise `other`.

**Key** (the match key of `app-ids.toml`):
- `aumid:<lower-cased AUMID>` normally.
- `path:<normalised lower-cased target>|<lower-cased arguments>` when the AUMID starts with `microsoft.autogenerated.` (Windows generated it; whether it survives a reinstall is UNPROVEN) and the target is not empty. Normalised: `/` to `\`, a leading `\\?\` removed, `%VAR%` parts expanded from the environment.

**Id.** Looked up by key in the id map. When there is none: `app-` plus the slug of the name. The slug keeps ASCII letters and digits in lower case, turns every other run of characters into one `-`, trims `-` at both ends and cuts at 40 characters. Names with no ASCII letter or digit (one name on this PC contains U+A789; an all-non-Latin name would be empty) use `app-<8 hex digits of the FNV-1a hash of the key>`. A clash with an existing id (also with ids of apps that are gone) appends `-2`, `-3`, and so on. A name is never used as a key or a file name. Ids are issued in scan order the first time; after that they are fixed by the file.

The prefix `app-` is reserved for apps and `pool-` for pools, so ids stay unique across the four launchable types (the frontend keys runs by `target.id` alone: contract report finding 3). Section 02 must not issue ids that start with `app-` or `pool-` (open point 4).

**Fields of `AppEntry`:**

| Field | Rule |
|---|---|
| `name` | the display name of the item |
| `kind` | as above |
| `exePath` | for `program`: the normalised target path as Windows wrote it (not lower-cased); for `store` and `other`: absent |
| `icon` | `{ kind: "url", value: "media/app-icon/<id>?v=<hex8>", fallback: <name> }`. `hex8` = first 8 hex digits of the FNV-1a hash of `key`, `target_mtime` and `ICON_RENDER_VERSION` joined with `\|`, so a changed exe or a bumped version gives a new URL |
| `keywords` | see 6.4 |
| `pinned` | from `pins.toml` |
| `running` | see 6.5 |
| `lastLaunchedAt` | from the history map |

Sort order of `GET /apps`: name case-insensitively, then id.

### 6.3 Search: the scorer and the ranking

**Decision: port `score.ts` to Rust with shared golden vectors.** Reason: Home sorts the backend's items and its own commands by one number, and `api.search` must keep its contract (ranked, capped, server side, no extra client work per key press). The alternative (scoring in the webview from the lists it already holds) would remove the duplication but would move candidate building for seven types into the frontend and would change `search` semantics; the cost of the port is about 80 lines and one test file. The risk of drift is closed by the vectors: either side fails when one changes alone.

File `launcher/score.rs`. It works on UTF-16 code units, because `score.ts` uses JavaScript string lengths and indexes.

```rust
const EXACT: u32 = 100; const PREFIX: u32 = 80; const WORD_START: u32 = 60;
const SUBSTRING: u32 = 40; const SUBSEQUENCE: u32 = 20; const KEYWORD_PENALTY: u32 = 5;

fn units(s: &str) -> Vec<u16> { s.to_lowercase().encode_utf16().collect() }

/// JavaScript `\s` and `trim()` set: NOT the same as Rust's `char::is_whitespace`
/// (Rust has U+0085, JavaScript has U+FEFF).
fn is_js_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}'
        | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

/// JavaScript `indexOf(needle, from)` on code units.
fn index_of(hay: &[u16], needle: &[u16], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() { return None; }
    (from..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

fn starts_word(t: &[u16], q: &[u16]) -> bool {
    let mut at = index_of(t, q, 1);
    while let Some(i) = at {
        // the previous unit must not be an ASCII letter or digit (score.ts: /[a-z0-9]/)
        let p = t[i - 1];
        let alnum = (b'a' as u16..=b'z' as u16).contains(&p) || (b'0' as u16..=b'9' as u16).contains(&p);
        if !alnum { return true; }
        at = index_of(t, q, i + 1);
    }
    false
}

/// Length of the shortest run of `t` that holds the units of `q` in order, or 0 (same loop as score.ts, including the `break`).
fn subsequence_span(t: &[u16], q: &[u16]) -> usize {
    let mut best = 0;
    let mut start = index_of(t, &q[..1], 0);
    while let Some(s) = start {
        let (mut ti, mut qi) = (s, 0);
        while ti < t.len() && qi < q.len() { if t[ti] == q[qi] { qi += 1 } ti += 1 }
        if qi < q.len() { break }
        let span = ti - s;
        if best == 0 || span < best { best = span }
        start = index_of(t, &q[..1], s + 1);
    }
    best
}

fn score_text(q: &[u16], t: &[u16]) -> u32 {
    if t.is_empty() { return 0 }
    if t == q { return EXACT }
    let closeness = ((9 * q.len()) as f64 / t.len() as f64).round() as u32;   // f64, like Math.round: ties go up, values are >= 0
    if t.starts_with(q) { return PREFIX + closeness }
    if starts_word(t, q) { return WORD_START + closeness }
    if index_of(t, q, 0).is_some() { return SUBSTRING + closeness }
    if q.len() < 2 { return 0 }
    match subsequence_span(t, q) { 0 => 0, span => SUBSEQUENCE + ((9 * q.len()) as f64 / span as f64).round() as u32 }
}

fn score_one(q: &[u16], text: &str, keywords: &[String]) -> u32 {
    let mut best = score_text(q, &units(text));
    for k in keywords {
        let s = score_text(q, &units(k));
        if s > 0 { best = best.max(s.saturating_sub(KEYWORD_PENALTY)) }
    }
    best
}

pub fn score(query: &str, text: &str, keywords: &[String]) -> u32 {
    let trimmed = query.trim_matches(is_js_space);
    if trimmed.is_empty() { return 1 }
    let q = units(trimmed);
    let whole = score_one(&q, text, keywords);
    if whole > 0 { return whole }
    let words: Vec<&str> = trimmed.split(is_js_space).filter(|w| !w.is_empty()).collect();
    if words.len() < 2 { return 0 }
    let mut weakest = u32::MAX;
    for w in words {
        let s = score_one(&units(w), text, keywords);
        if s == 0 { return 0 }
        weakest = weakest.min(s);
    }
    ((weakest as f64 / 10.0).round() as u32).max(1)
}
```

Points where Rust and JavaScript could differ, and the rule that closes each:

| Point | Rule |
|---|---|
| String length and indexes | code units (UTF-16), as in JavaScript. An emoji counts as 2 |
| Lower-casing | `str::to_lowercase()` against `toLowerCase()`: both use the Unicode default mapping including the final sigma; the vectors include U+0130, U+03A3, U+00DF and a surrogate pair to prove it |
| Whitespace | `is_js_space` above; the vectors include U+00A0, U+FEFF and U+0085 |
| Rounding | `f64::round` on non-negative values equals `Math.round` |
| `startsWord` | previous unit tested against ASCII `a-z0-9` only (so U+00E9 before the query counts as a separator) |

**Ranking of an item** (this is `rank()` of the mock `search.ts`, moved to `launcher/search.rs`):

```rust
const KEYWORD_SHARE: f64 = 0.6;   // a keyword match counts 60 percent: "web" must list pool "Web dev" above browsers that only have the keyword "web"
fn rank(query: &str, title: &str, keywords: &[String]) -> u32 {
    let by_title = score(query, title, &[]);
    if by_title > 0 { return by_title }
    let with_keywords = score(query, title, keywords);
    if with_keywords > 0 { ((with_keywords as f64 * KEYWORD_SHARE).round() as u32).max(1) } else { 0 }
}
```

**Search procedure** (`Service::search(q, types)`, no disk, no SQLite, no process snapshot):
1. Blank `q` returns `[]`. A `q` longer than `QUERY_MAX` characters is `400 queryTooLong`.
2. Candidates: apps from the memory list (not hidden), pools from `pools.toml` (keywords are the names of the member apps that exist), then `search_entries()` of projects and games through `ItemSource` (section 6.9). Sessions, jobs and idle jobs are mapped by THIS module from `sessions().list()`, `jobs().list()` and `idle().list()` (section 03) with the title, subtitle, icon, status and keyword rules of the table below; section 03 implements no search trait. A type that is not in `types` is not asked.
3. Every candidate is ranked; score 0 is dropped.
4. Sort by `score` descending, then title lower-cased, then id (equal scores keep a steady order, so the list does not jump while typing; `localeCompare` of the mock has no std equivalent and only matters for equal scores).
5. Cut to `SEARCH_MAX` (50). The UI keeps 50 after merging with its commands, so a lower cap would lose results.

Items: titles, subtitles, icons and keywords per type (the mock table of the contract report 3.2, now the rule for the owners of the other types):

| Type | title | subtitle | icon | status | keywords |
|---|---|---|---|---|---|
| app | name | `exePath`; for `store` and `other` the package name of the AUMID (text before `_` or `!`) when there is one, else none | `app.icon` | none | `app.keywords` |
| pool | name | `"<n> app"` or `"<n> apps"` | letter tile of the name | none | names of the member apps |
| project | name | path | project icon | none | `[branch]` |
| game | title | `Steam`, `Epic` or `Local` (section 02 decision G1; the game folder name of a folder game is not used) | cover url icon or lucide `gamepad-2` | none | the same label |
| session | title | `"<tool>: <summary>"` of a pending request, else the last path segment of `cwd` | lucide `bot` | session status | `[folder name of cwd, branch]` |
| job | name | schedule label | lucide `calendar-clock` | running, else last run status, else idle | `[schedule label, kind]` |
| idleJob | name | label of the state (Queued, Running, Paused, Done, Failed) | lucide `cpu` | queued and paused map to idle | `[resource]` |

Golden vectors: file `src/lib/score.vectors.json` (next to `score.ts`, so vitest imports it and Rust reads it with `include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../src/lib/score.vectors.json"))`). About 260 cases: 90 hand-written (one per class: exact, prefix, word start, substring, subsequence; keyword hits; multi-word fallback; empty query; one-letter query; case; the Unicode and whitespace cases of the table above; long text; text shorter than the query) and 170 generated from a seeded generator over a small alphabet (letters, digits, `-`, space, U+00E9, U+00DF, U+0130, U+03A3, a surrogate pair, U+00A0). Format: `{ "version": 1, "cases": [ { "query": "...", "text": "...", "keywords": ["..."], "expected": 83 } ] }`. The file is written by the vitest test `src/lib/score.vectors.test.ts` when `RAIN_WRITE_VECTORS=1` is set (expected values come from `score.ts`) and is only compared otherwise. `npm test` therefore fails when `score.ts` changes without regenerating, and `cargo test` fails when the Rust port disagrees. `rank` has Rust-only cases computed by hand from the rule above ("web" against "Web dev" with and without keywords).

### 6.4 Keywords of an app

`keywords` (lower case, unique, at most 12) are built at rebuild time from: the user's `keywords` in `app-ids.toml`; the target file stem (for `program`; `code` for `Code.exe`); for `store` apps the last dotted segment of the package name before `_` (`WindowsCalculator`, lower-cased); and the hints of this table, looked up by the stem or the package segment. A word that already appears as a word of the name is skipped. The table is data in `apps.rs` (not personal; extend by pull request):

| Stem or package segment | Words |
|---|---|
| `chrome`, `msedge`, `firefox`, `brave`, `opera`, `vivaldi` | browser, web, internet |
| `code`, `cursor`, `devenv`, `idea64`, `pycharm64` | editor, ide, code |
| `notepad`, `windowsnotepad`, `notepad++` | text, editor |
| `wt`, `windowsterminal`, `powershell`, `pwsh`, `cmd` | terminal, shell, console, command |
| `explorer` | files, folders |
| `mspaint`, `paint` | draw, paint, image |
| `calc`, `calculatorapp`, `windowscalculator` | math, calculator |
| `taskmgr` | processes, performance, task |
| `winword`, `excel`, `powerpnt`, `outlook` | office, documents, spreadsheet, slides, mail |
| `obsidian` | notes, markdown, vault |
| `discord`, `slack`, `teams`, `ms-teams` | chat, voice, meeting |
| `spotify`, `vlc` | music, audio, video, media |
| `steam`, `epicgameslauncher` | games, store |

### 6.5 `running`

Computed at `GET /apps` and `POST /apps/scan` only. Cost 4.7 ms for 262 processes, 147 image paths readable without administrator rights (spike process Q6); the snapshot is reused for `PROCESS_TTL` and thrown away when a run ends.

1. `platform::process::list_images() -> Vec<String>` (copy `list_images` of spike `process\src\bin\q6.rs`: Toolhelp snapshot, then `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` and `QueryFullProcessImageNameW` for each pid; unreadable processes are skipped). Run through `core::blocking::run`. Behind a trait `ProcessLister` so tests use a fake.
2. Build a `RunningIndex`: the set of normalised image paths (lower case, `/` to `\`, `\\?\` removed), the set of image file names, and for every path that contains `\windowsapps\` the pair (package name, publisher id) read from the folder name `<Name>_<Version>_<Arch>__<PublisherId>`.
3. Match per app:
   - `program`: the normalised `exePath` is in the path set.
   - Squirrel-style shortcuts (arguments contain `--processstart`): take the file name after that word and match it in the file-name set (Discord, Slack and other apps started through `Update.exe`).
   - `store`: the AUMID `<Name>_<PublisherId>!<App>` gives a pair; match any process of that pair.
   - `other`: never running.
4. Grace: an app that Rain launched successfully less than `START_GRACE` ago and has no matching process yet counts as running, so the dot does not flicker while the app is starting. This is a known approximation (an app that exits at once looks running for 8 s).

Limits, stated so nobody expects more: elevated and system processes cannot be read (their apps show "not running"); launcher shims other than the Squirrel form are not recognised; a Store app whose process lives outside `WindowsApps` shows "not running". All are acceptable for a hint that is not used for any decision.

### 6.6 Pins

Source of truth: the in-memory value of `TomlFile<PinsFile>`; no SQLite.
- `PUT /pinned/{type}/{id}` with `pinned: true`: if `(type, id)` is already pinned, return 204 without a write. Else check that the item exists: `app` in the list (not hidden), `pool` in `pools.toml`, `project` and `game` through `ItemSource::knows` (a hidden game cannot be pinned: `knows()` is false for it, and hiding a game removes its pin, section 02). Unknown: `404 unknownItem`, "Item not found". Then `edit` appends a `[[pin]]`.
- `pinned: false`: remove the table if present. An unknown id is a normal no-op (this is how a dangling pin can be cleaned by hand through the API).
- `GET /pinned`: for each pin in file order, `item_for(type, id)`; a missing item (gone, hidden) is skipped, never deleted from the file. A project whose folder is missing is NOT skipped: section 02 `item_for` returns it so that the pin stays visible (a dimmed row that opens "Locate").
- The `pinned` flag of apps and pools comes from the same value; section 02 reads it with `Service::pinned_ids(PinnableType)` for projects and games and removes a pin with `Service::unpin(PinnableType, &str)` (used when a game is hidden or deleted), so there is exactly one source.

### 6.7 Pools

- Id of a new pool: `pool-` plus the slug of the name (same slug function; empty slug gives `pool-<8 hex of the FNV-1a hash of the name and a counter>`), with `-2`, `-3` on a clash among the ids in the file. A rename keeps the id. The id of a deleted pool may be issued again; its pin and history rows were removed at delete, so the new pool starts clean.
- Validation of a draft (messages are shown by the dialog, so they are fixed text; codes are `invalid`): after trim the name is not empty (`emptyName`, "A pool needs a name"), at most 80 characters (`nameTooLong`, "A pool name can have at most 80 characters"); at least one member (`noMembers`, "A pool needs at least one app"), at most 40 (`tooManyMembers`, "A pool can hold at most 40 apps"); every member has a valid id, an `args` text of at most 2000 characters on one line, and a `delayMs` of at most 60000 (`badMember`, "A pool member is not valid"); the delays add up to at most 600000 ms (`delaysTooLong`, "The delays of a pool add up to more than 10 minutes"); the pool count stays at most 100 (`tooManyPools`). Member ids are not checked against the app list. Duplicate names and duplicate members are allowed (the editor cannot make a duplicate member; "Duplicate" copies as is).
- The saved `Pool` is returned with `icon = letter(name)`, `pinned` and `lastLaunchedAt` filled in.
- `DELETE` removes the table, the pin and the history rows of the pool.

### 6.8 Launching: the engine

**Entry:** `Service::launch(target: LaunchTarget) -> ApiResult<(LaunchRun, bool)>` (`bool` = created). While `game_mode_active` the call always returns `409 conflict gameMode`, "Game mode is on". The one exception is not in this module: game mode starts "then launch game" through `games::Service::launch_after_game_mode` (section 02), which makes no run and does not call `launch`.

1. If the backend is shutting down: `503 shuttingDown` (core rule).
2. Validate `args`: allowed only for `type: app` (`argsNotAllowed`), at most `ARGS_MAX`, one line (`badArgs`).
3. If a run for the same `(type, id)` is `running`, return it with `created = false` (route answers 200).
4. Plan by type:
   - `app`: find the app (not hidden), else `404 unknownApp` "App not found". One step: label = name, icon = app icon, action `StepAction::App { app_id, args: target.args }`.
   - `pool`: find the pool, else `404 unknownPool` "Pool not found". One step per member in member order, 1:1 (the UI reads `steps[i]` against `members[i]`): the member's app found: label = name, icon = app icon, action `StepAction::App { app_id, args: member.args }`, `delay_ms = member.delayMs`. The app is gone or hidden: label "Unknown app", icon lucide `app-window`, `preset_error` "App not found", the pool continues.
   - `project`, `game`: `state.projects()` or `state.games()` as `LaunchPlanner` returns a `LaunchPlan` (6.9). Until stage 5 the stub answers `503 notBuilt`.
5. Create the run: id `launch-<unix_ms in hex>-<counter>`, status `running`, step 0 `running`, others `idle`. Insert into the registry. `state.activity` counts the run as busy (blocks an upgrade).
6. Spawn the run task on a `TaskTracker`; return the run.

**The run task**, for each step `i`:

```
if step.preset_error { fail(i, error); if step.stops_run { break } else { next } }
if step.delay_ms > 0 { sleep(delay_ms) }                       // T5; the step shows `running` meanwhile
result = timeout(STEP_TIMEOUT, opener.open(step.action))      // section 6.8.1; `opener` is `state.opener()`
if result is Err(e) and step.shell_fallback is Some(fb) and e.code in {5, 1920, 740} {
    result = timeout(STEP_TIMEOUT, opener.open(ShellExec from fb))   // the same program through ShellExecuteEx; the first error stays in the log
}
match result {
  Ok(started) => success(i); if step.action is App { record_use(app, app_id) }
                 if step.action is Spawn and step.on_spawned is Some(cb) { cb(started.pid, started.creation_time) }
  Err(e)      => fail(i, step.error_text.map(|f| f(&e)).unwrap_or(e.ui_text)); if step.stops_run { break }
}
if i + 1 < n { steps[i+1].status = running }
publish_throttled(snapshot)                                     // after each step
```

Steps run one after the other (a failed step does not stop the others unless `stops_run`; later steps of a stopped run stay `idle`). The delay of a member is a wait before its start, as in the mock. At the end: status `failed` when any step failed, else `success`; on `success` the pool target is recorded (an app step was recorded when it succeeded). For project and game targets the engine awaits `LaunchPlanner::finished(run)` (an async method, so the owner can finish its SQL write) before the final frame and records nothing itself: section 02 writes its own history there. The final order is in section 2, "Live topics".

**Retention and recovery:** the registry holds at most `RUN_MAX` runs. A finished run is kept `RUN_KEEP` after it ended. Eviction is checked at insert time and at `getLaunch` time (no timer). A running run is never evicted. After a backend restart the registry is empty: `GET /launches/{id}` answers `404 unknownRun`. The history rows are separate (SQLite). `getLaunch` has no caller today; section 9.3 makes the frontend poll it so that a lost final frame cannot freeze the window open.

**Shutdown:** `stop()` cancels the run tasks and publishes nothing. Programs already started stay (they belong to nobody).

#### 6.8.1 How a step starts a program

The one place that starts programs is the `Opener` (a trait, so tests never start anything; foundation 11.6 and `foundation-review.md` 2, "Tests never call the real ShellExecute"). It is defined ONCE, in `core/launch_contract.rs` with `OpenRequest`, and its instance is in `AppState` (`state.opener()`). This module's engine, `games::Service::launch_after_game_mode` (section 02) and `openDailyNote` (section 04) all use that one instance; no other section defines an opener trait, and tests use the one `FakeOpener` (section 11). The production `WindowsOpener` runs on the COM thread, with `STEP_TIMEOUT` around each call.

```rust
pub enum OpenRequest {
    /// Start a packaged app by AUMID: IApplicationActivationManager::ActivateApplication (returns a pid, 160-260 ms).
    Activate { aumid: String, args: Option<String> },
    /// ShellExecuteExW: shortcuts, URIs, folders, `shell:AppsFolder\<AUMID>`, or a file with parameters and a working folder.
    ShellExec { file: String, params: Option<String>, dir: Option<String> },
    /// Direct process start (section 02: games and exact quoting). Detached, own process group, never in a job.
    Spawn { exe: PathBuf, args_raw: String, cwd: PathBuf },
}
pub trait Opener: Send + Sync {
    /// Carries out one request. `Ok` holds the pid and the process creation time (a Windows FILETIME) when the call knows them (`Spawn`, `Activate`).
    fn open(&self, req: OpenRequest) -> BoxFuture<'_, Result<Started, OpenError>>;
}
pub struct Started { pub pid: Option<u32>, pub creation_time: Option<u64> }
pub struct OpenError {
    pub ui_text: &'static str,
    /// ShellExecute result (a value of 32 or less), Win32 or HRESULT-low-word error code of the start call;
    /// None for a timeout or a pre-check (metadata NotFound). Section 02 matches on it (31 means "no handler").
    pub code: Option<u32>,
    pub log_reason: String,             // for the log only (never the command line)
}
```

`Spawn` calls `platform::process::spawn_detached` (never `std::process::Command`, which `clippy.toml` forbids) and logs `warn` when it returns `Detached::InJob` (the program then dies with the backend). `ShellExec` calls `platform::shell::shell_execute` (foundation 11.6; the one shell call of the whole backend), which is `ShellExecuteExW` with verb `open`, `nShow = SW_SHOWNORMAL`, and `fMask = SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC`, so a missing file never shows a Windows error box from a windowless process (`ShellExecuteW` has no such flag; its success cases are proven, the Ex form with the same arguments is the same shell path and is UNVERIFIED until the stage 4 check of section 11). The launched program is never inside a kill-on-close job: the backend is not in one (D15) and Rain gives user programs none. Never `explorer.exe <path>` (leaves `/factory` processes).

Decision table for `StepAction::App { app_id, args }`:

| App kind | Arguments | Call | Fallback |
|---|---|---|---|
| `store` | none or some | `Activate { aumid, args }` | on error: `ShellExec { file: "shell:AppsFolder\<AUMID>" }` (proven, 217 ms), arguments dropped |
| `program` with a target | none | `ShellExec { file: "shell:AppsFolder\<AUMID>" }`: keeps the shortcut's own arguments, working folder and run-as (UNPROVEN for classic items) | if the return value is 32 or less: `ShellExec { file: target, params: link args, dir: folder of target }` |
| `program` with a target | some | `ShellExec { file: target, params: link args + " " + args, dir: folder of target }` (the exact raw tail; no quoting is added) | none |
| `other` | none | `ShellExec { file: "shell:AppsFolder\<AUMID>" }` | none |
| `other` | some | the arguments are ignored (nothing can take them); the step still starts the app | |

Before a `program` start the engine checks `std::fs::metadata(target)` on a blocking thread with a 3 s timeout; a definite `NotFound` fails the step at once with "Executable not found" (no shell call, no dialog). Any other error or a timeout goes on to the start call.

Errors, text shown in `LaunchStep.error` (when `PlannedStep.error_text` is set, its text replaces the one below; the UI prints `"<label>: <text>"` and truncates; each is at most 40 characters; the first three are the mock's own strings):

| Cause | Text |
|---|---|
| `SE_ERR_FNF` (2), `SE_ERR_PNF` (3), metadata `NotFound`, activation `ERROR_FILE_NOT_FOUND` | Executable not found |
| `SE_ERR_ACCESSDENIED` (5), `E_ACCESSDENIED` | Access denied |
| anything else | Failed to start |
| `ERROR_ELEVATION_REQUIRED` (740) | Needs administrator rights (only when the step has no `shell_fallback`; with one, the engine starts the program through ShellExecuteEx and the step succeeds or fails on that call) |
| `ERROR_CANCELLED` (1223), the user dismissed the UAC prompt | Cancelled |
| `SE_ERR_NOASSOC` (31) | No program for this file |
| the call did not return in `STEP_TIMEOUT` | Did not respond (and the module logs `warn`; later COM calls may answer `comBusy`, foundation 11.6) |
| the member's app is gone | App not found |

"Success" means that the start call succeeded, not that a window appeared (contract report 5.7.1). An app that is already running or hands over to a running instance is a success.

### 6.9 Extension points for the other modules

Defined in `core/launch_contract.rs` (foundation 11.1 and 11.2; open point 4 is closed by this), together with `LaunchPlan`, `PlannedStep`, `StepAction`, `ShellFallback`, `OpenError` and `OpenRequest`. This module implements the engine against them; the `Service` of section 02 implements the two traits (projects and games; section 03 implements no trait here). They use only `rain-types`, `std` and the other types of that one file, which is the exception that foundation 11.2 names. The stubs of stage 4 (the `Service` of projects and games, which exist from stage 4 and implement the traits with empty answers) are why stage 4 compiles before stage 5. The engine passes the `OpenError` of a failed step to `PlannedStep.error_text`; section 02 matches on `err.code` (31 means that no program is registered for the file type), never on `ui_text` or `log_reason`.

```rust
/// Implemented ONLY by the Service of projects and games (section 02). Sessions, jobs and idle jobs are
/// mapped to `SearchItem`s by this module itself from the `list()` calls of section 03 (table in 6.3).
pub trait ItemSource: Send + Sync {
    /// Items that search may find, with the keywords of the table in 6.3. The owner already left out hidden games,
    /// disabled sources. A project whose folder is missing is still findable by `item_for` (pins). `score` is 0.
    fn search_entries(&self) -> Vec<(SearchItem, Vec<String>)>;
    /// The item for pins and recent; None when the id is gone, a game is hidden, or its source is off. A missing project is returned.
    fn item_for(&self, id: &str) -> Option<SearchItem>;
    /// True when the id exists and may be pinned (false for a hidden game). Used by setPinned.
    fn knows(&self, id: &str) -> bool { self.item_for(id).is_some() }
    /// The owner's own history, newest first: (RFC 3339 time, item). Projects and games keep their history in
    /// their own tables (section 02), so `listRecent` merges these with the app and pool rows of `launcher_launch`.
    fn recent(&self) -> Vec<(String, SearchItem)>;
}

/// Implemented by projects and games for the launch targets of section 02. The engine of this module runs the steps.
pub trait LaunchPlanner: Send + Sync {
    fn plan(&self, target: &LaunchTarget) -> ApiResult<LaunchPlan>;      // reads caches only; starts no process, walks no directory; a few `symlink_metadata` calls at most (section 02 6.5)
    fn finished<'a>(&'a self, _run: &'a LaunchRun) -> BoxFuture<'a, ()> { Box::pin(async {}) }   // awaited by the engine before the final frame; the owner writes its history here (SQL included)
}
pub struct LaunchPlan { pub steps: Vec<PlannedStep> }
pub struct PlannedStep {
    pub label: String, pub icon: IconRef,
    pub action: StepAction,
    pub delay_ms: u32,
    pub preset_error: Option<String>,   // a step that is known to fail before it starts (project folder missing)
    pub stops_run: bool,                // a failure here makes the rest pointless
    /// Called by the engine after a `Spawn` step started a process: (pid, process creation time as a Windows FILETIME).
    /// Section 02 uses it to insert the play row and to start a wait on `platform::process::open_for_wait(pid, creation_time)` (foundation 11.6) for the play time of a game. `plan()` itself starts no process and walks no directory; it may make a few `symlink_metadata` calls (section 02 6.5 `tools::find`).
    pub on_spawned: Option<Arc<dyn Fn(u32, u64) + Send + Sync>>,
    /// Used by the engine when the main start (a `Spawn`) fails with os error 5, 1920 or 740: the same program is started
    /// through ShellExecuteEx with these values. The first error code is in `OpenError.code` and the text in `log_reason`. Used by section 02 for the
    /// Windows Terminal steps (alias refused) and for a local game that needs administrator rights (740).
    pub shell_fallback: Option<ShellFallback>,
    /// When set, replaces the text the engine would take from its table (6.8.1) for a failed step; the planner knows
    /// the dynamic texts ("Program file not found: <path>", "Windows Terminal was not found"). The limit of 40
    /// characters applies only to the engine's own texts.
    pub error_text: Option<Arc<dyn Fn(&OpenError) -> String + Send + Sync>>,
}
pub struct ShellFallback { pub file: String, pub params: Option<String>, pub dir: Option<String> }
pub enum StepAction {
    App { app_id: String, args: Option<String> },
    ShellExec { file: String, params: Option<String>, dir: Option<String> },
    Spawn { exe: PathBuf, args_raw: String, cwd: PathBuf },
    Nothing,                            // a step that is already satisfied (a project that is already open): success at once
}
```

What the launcher exposes to the others: `Service::pinned_ids(PinnableType) -> HashSet<String>`; `Service::unpin(PinnableType, &str)` (removes a pin, a no-op when absent); `Service::launch(target)`. There is NO `last_used` for projects and games: `Project.lastOpenedAt` and `Game.lastPlayedAt` come from the tables of section 02 only (`projects_opened`, `games_play`), which also hold the launches made by game mode.

`listRecent` merges three lists by time, newest first: the app and pool rows of the in-memory map of `launcher_launch`, `recent()` of projects and `recent()` of games (each id of the first list turned into an item by this module, a missing item skipped), cut to `limit`. Order of apps and pools: a pool launch stamps every member app that started plus the pool (contract report 3.4), so one pool launch can produce several rows; Home removes what it already shows.

---------------------------------------------------------------------------------------------------

## 7. Background work

No row uses a timer that foundation 11.5 does not allow. The module adds no `interval`.

| Task | Trigger | Timer row | Cost | In game mode | Stops |
|---|---|---|---|---|---|
| Initial scan | module `start` (an event) | none | one COM call 190-200 ms, about 90 `metadata` calls, one SQLite write; once | not started; the cached list is used | task is tracked and aborted at `stop()` |
| Start Menu watch | `notify` events on two folder trees (`ReadDirectoryChangesW`) | none (the hub debounces with `quiet = START_MENU_QUIET`; the module adds no sleep) | no idle cost; one rescan per quiet burst (an install makes many events, one scan) | events ignored | the watcher is dropped with the hub |
| Stale rescan | read-time age check in `GET /apps` | none (checked at read time) | at most one scan per 10 minutes of use | not started | tracked task |
| Process snapshot | read-time in `GET /apps` and `POST /apps/scan` | none | 4.7 ms, reused 2 s | allowed (a read) | none |
| Icon render | a media request | none | 1 to 65 ms on the COM thread per icon | allowed | none |
| Launch run | `POST /launches` | T5 (the delay `sleep` of a member) and T4 (the 10 s step timeout), both created per step and dropped | one COM call per step | new runs refused (`409 gameMode`); running runs finish | tracked, aborted at `stop()` |
| History write and pruning | end of a run, backend start | none | one transaction | allowed | none |
| Icon cache clean | end of a scan | none | at most 400 file deletions | skipped with the scan | none |
| Run registry eviction | insert and `getLaunch` | none | trivial | none | none |

The backend idle cost of this module is zero CPU and about 100 KB of memory (125 app records, the pool and pin values, the history map). The icon bytes are never held in memory; the media route reads them from disk.

---------------------------------------------------------------------------------------------------

## 8. Errors

Error bodies are `{ code, message, source?, reason? }` (foundation 4.5). `source` is `launcher` unless the table says otherwise.

| Situation | Status | `code` | `reason` | `message` | What the UI shows |
|---|---|---|---|---|---|
| `q` longer than 200 characters | 400 | invalid | `queryTooLong` | The search text is too long | Home shows "Search unavailable" with Retry (any rejection does) |
| Unknown value in `types` | 400 | invalid | `unknownType` | Unknown item type | same |
| `limit` not 1 to 50 | 400 | invalid | `badLimit` | The limit must be between 1 and 50 | "Recent items unavailable" |
| Malformed body or query | 400 | invalid | `badBody`, `badQuery` | The request was not valid: ... (foundation) | the action's error notice |
| Pin an unknown item | 404 | notFound | `unknownItem` | Item not found | notice "Item not found" (`pinAction` shows `message`) |
| Apps: no cache and the scan failed | 503 | unavailable | `scanFailed` | Could not read the installed apps | Apps and Home show "Apps unavailable" with Retry; Retry asks for a new scan |
| Apps: first scan not finished in 10 s | 503 | unavailable | `scanning` | The app list is still loading | same |
| Windows shell not answering | 503 | unavailable | `comBusy` | Windows is not answering. Try again in a moment | same |
| Pool draft invalid | 400 | invalid | `emptyName`, `nameTooLong`, `noMembers`, `tooManyMembers`, `badMember`, `delaysTooLong`, `tooManyPools`, `idMismatch` | the fixed texts of section 6.7; `idMismatch`: "The pool id does not match" | text under the form, dialog stays open |
| Pool not found on update | 404 | notFound | `unknownPool` | Pool not found | same |
| A TOML file has an error | 409 | conflict | `configBroken` | pools.toml has an error. Fix the file and try again (the file name differs per file) | notice or dialog text; the Settings and health views show the `ConfigIssue` |
| A TOML file could not be written | 500 | failed | `saveFailed` | Could not save pools.toml | same |
| Launch: unknown app | 404 | notFound | `unknownApp` | App not found | synthetic failed run and notice (useLaunch, step 2) |
| Launch: unknown pool | 404 | notFound | `unknownPool` | Pool not found | same |
| Launch: `args` on a non-app | 400 | invalid | `argsNotAllowed` | Arguments can only be passed to an app | same |
| Launch: bad `args` | 400 | invalid | `badArgs` | The arguments are too long or have a line break | same |
| Launch in game mode | 409 | conflict | `gameMode` | Game mode is on | same |
| Launch of a project or game before stage 5 | 503 | unavailable | `notBuilt` | Launching this kind of item is not available yet | same (never reached through the router) |
| Backend stopping | 503 | unavailable | `shuttingDown` | Rain is stopping | same |
| `getLaunch` unknown | 404 | notFound | `unknownRun` | Launch not found | the frontend treats it as "the run is lost" (section 9.3) |
| Media: unknown app id | 404 | notFound | `unknownApp` | No such app | `<img>` falls back to the letter tile |
| Media: no icon can be rendered | 404 | notFound | `noIcon` | No icon | same; the id is remembered as "no icon" until the next scan |
| A step fails | (the run is `failed`; HTTP is 201 or 200) | | | the step texts of section 6.8.1 in `LaunchStep.error` | checklist line, row dot, notice "<label>: <text>" |
| `launcher_*` tables newer than this build | 503 | unavailable | `dbTooNew` | (foundation 10.5) | error rows |

Health sources (`GET /health`, foundation 6.9): `apps` is `ok`; `failed` with "Could not read the installed apps" or "Showing the last known app list" (a rescan failed but a cache exists); never `off` (there is no switch). Config issues are reported by the TOML files themselves: `pins.toml`, `pools.toml`, `app-ids.toml`.

---------------------------------------------------------------------------------------------------

## 9. Frontend changes

Line numbers are from the files as read on 2026-11-22 (HEAD `e4c7f70`). Files that do not exist yet (they are built in stage 1b: `src/data/router.ts`, `routing.ts`, `rest/client.ts`) are described by function.

### 9.1 Contract changes (foundation 15.1 template)

| # | Item | From | To | Reason | Frontend files touched (full paths) | Breaking for the mock? | Stage |
|---|---|---|---|---|---|---|---|
| 01.1 | `IconRef` | `{ kind, value, tint? }` | adds `fallback?: string` | a failed image draws a letter tile instead of a broken-image glyph | `src/components/rain/RIcon.vue` (lines 31, 35-40, 59-69, 77-79) | no (optional) | 4 |
| 01.2 | `AppEntry` | `exePath: string` | `exePath?: string` and new required `kind: AppKind` | Store apps have no file | `src/data/types.ts` (lines 20-21 deleted), `src/features/apps/AppDetail.vue` (lines 40, 69-72), `src/features/apps/actions.ts` (lines 53, 55), `src/data/mock/seed/apps.ts` (rewritten), `src/data/mock/search.ts` (line 21 compiles unchanged: `subtitle` is optional) | yes: the seed built `AppEntry` without `kind`; the seed becomes empty (section 9.6) | 4 |
| 01.3 | `LaunchTarget` | `{ type, id, mode? }` | adds `args?: string` (type `app` only) | "Retry failed" keeps the member's arguments | `src/features/apps/PoolDetail.vue` (line 62) | no (optional; the mock ignores it) | 4 |
| 01.4 | `RainApi.rescanApps` (new) | none | `rescanApps(): Promise<AppEntry[]>` // `POST /apps/scan` | the empty state and a command need a forced scan | `src/data/api.ts` (after line 37), `src/data/rest/client.ts`, `src/data/routing.ts`, `src/features/apps/actions.ts` (new export), `src/features/apps/AppsScreen.vue` (lines 178-184), `src/actions/commands.ts` (new command after line 99), `src/data/mock/slices/launcher.ts` | yes: one new method; the mock gets one line (`async rescanApps() { await delay(); return clone(state.apps) }`) so `createMockApi()` still returns a complete `RainApi` | 4 |
| 01.5 | `savePool` errors | `failed` for an empty name or no members | `invalid` with the same message text | the foundation's error classes | none (the dialog shows `message` only) | no | 4 |
| 01.6 | `deletePool` of an unknown id | `notFound` | 204 | foundation 4.6 (delete is idempotent) | none | no | 4 |
| 01.7 | `PUT /pinned` of a pinned item | moved it to the end | keeps its place | idempotent PUT; the UI never re-pins | none | no | 4 |
| 01.8 | ported types | hand-written `SearchType`, `PinnableType`, `SearchItem`, `AppEntry`, `PoolMember`, `Pool`, `PoolDraft`, `LaunchTarget`, `LaunchStep`, `LaunchRun` | generated | Rust is the source of truth | `src/data/types.ts` lines 13 to 29 deleted in the same commit as the Rust types | no (the barrel re-exports the same names) | 4 |
| 01.9 | `api.ts` comments | per-method route comments | replaced by the registry; adds the `rescanApps` row | foundation F4 | `src/data/api.ts` | no | 4 |

Not changed: `TopicPayload.launch` (still `LaunchRun`), the `Topic` union, the key bindings, every screen's data flow.

### 9.2 The REST client and the routing table (stage 4)

`src/data/rest/client.ts` gets these functions (foundation 8.3; `enc` is `encodeURIComponent`, `qs` drops `undefined` and joins arrays with commas):

```ts
search: (query, types) => call<SearchItem[]>('GET', `/search${qs({ q: query, types })}`),
listPinned: () => call<SearchItem[]>('GET', '/pinned'),
listRecent: (limit) => call<SearchItem[]>('GET', `/recent${qs({ limit })}`),
setPinned: (type, id, pinned) => call<void>('PUT', `/pinned/${type}/${enc(id)}`, { pinned }),
listApps: () => call<AppEntry[]>('GET', '/apps'),
rescanApps: () => call<AppEntry[]>('POST', '/apps/scan'),
listPools: () => call<Pool[]>('GET', '/pools'),
savePool: (draft) => draft.id === undefined
  ? call<Pool>('POST', '/pools', draft)
  : call<Pool>('PUT', `/pools/${enc(draft.id)}`, draft),
deletePool: (id) => call<void>('DELETE', `/pools/${enc(id)}`),
launch: (target) => call<LaunchRun>('POST', '/launches', target),
getLaunch: (id) => call<LaunchRun>('GET', `/launches/${enc(id)}`),
```

`call` resolves media references, so `icon.value` of kind `url` (`media/app-icon/app-vs-code?v=3fa9c1d2`) becomes a loadable URL in every answer and in every `launch` frame (the stream client resolves media for all topics except `scan`).

`src/data/routing.ts` (the only file of the routing decision; sides after stage 4):

| Method | Side | Special case |
|---|---|---|
| `listApps`, `rescanApps`, `listPools`, `savePool`, `deletePool` | `real` | none |
| `search`, `listPinned`, `listRecent`, `setPinned`, `launch`, `getLaunch` | `real` for app and pool, `mock` for the other types | the functions below |
| topic `launch` | both | `subscribe('launch', cb)` attaches to the real stream AND to the mock until stage 5, because mock launches of projects and games still emit on the mock (foundation 8.6 lists one side per topic; open point 9) |

The mixed-state functions in `src/data/router.ts` (deleted with the router):

```ts
// Types the backend already searches. Stage 5 adds project and game, stage 6 job and idleJob, stage 8 session.
const REAL_SEARCH: SearchType[] = ['app', 'pool']
const ALL: SearchType[] = ['app', 'pool', 'project', 'game', 'session', 'job', 'idleJob']

async search(query, types) {
  const wanted = types ?? ALL
  const realTypes = wanted.filter((t) => REAL_SEARCH.includes(t))
  const mockTypes = wanted.filter((t) => !REAL_SEARCH.includes(t))
  const [a, b] = await Promise.allSettled([
    realTypes.length ? real.search(query, realTypes) : Promise.resolve([]),
    mockTypes.length ? mock.search(query, mockTypes) : Promise.resolve([]),
  ])
  // A failing source loses only its own items (contract report 5.6); both failing is an error.
  if (a.status === 'rejected' && b.status === 'rejected') throw a.reason
  const items = [...(a.status === 'fulfilled' ? a.value : []), ...(b.status === 'fulfilled' ? b.value : [])]
  return items.sort((x, y) => y.score - x.score)       // Array.sort is stable: real items first on equal scores
},
async listPinned() {                                   // real pins first, then mock pins; Home moves pools to the front itself
  const [a, b] = await Promise.all([real.listPinned(), mock.listPinned()])
  return [...a, ...b]
},
async listRecent(limit) {                              // items carry no timestamp, so real first, then mock; transitional (stage 4 only)
  const [a, b] = await Promise.all([real.listRecent(limit), mock.listRecent(limit)])
  return [...a, ...b].slice(0, limit)
},
setPinned: (type, id, pinned) => (type === 'app' || type === 'pool' ? real : mock).setPinned(type, id, pinned),
async launch(target) {
  if (target.type !== 'app' && target.type !== 'pool') return mock.launch(target)
  const run = await real.launch(target)
  realRuns.add(run.id)                                 // so getLaunch goes to the same side
  return run
},
getLaunch: (id) => (realRuns.has(id) ? real : mock).getLaunch(id),
```

The mock keeps its own copy of settings and pins for projects and games; after stage 4 the mock has no apps and no pools (section 9.6), so its answers contain only the types that are still mock.

### 9.3 `src/composables/useLaunch.ts`: the lost-final-frame fix

Problem (contract report finding 2): `launch()` ends only when a topic frame with `status !== 'running'` arrives. If the stream reconnects without a backend restart (a lag, a network hiccup of the bridge), the last frame can be lost, `launch()` never resolves and `shell.holdOpen('launch')` is never released.

Changes (current lines 17, 27-45, 51-55, 67-99):

1. Import `onStreamReset` next to `api`: `import { api, onStreamReset } from '@/data'`.
2. Add a reconcile function and a watchdog:

```ts
const POLL_AFTER_MS = 5000
const POLL_EVERY_MS = 3000
let pollTimer: number | undefined

function isNotFound(reason: unknown): boolean {
  return typeof reason === 'object' && reason !== null && (reason as { code?: unknown }).code === 'notFound'
}

/** Asks the backend what became of every run we still wait for (a final frame may have been lost). */
async function reconcile(): Promise<void> {
  for (const [id, entry] of [...waiting]) {
    try {
      const current = await api.getLaunch(id)
      if (current.status === 'running') continue
      runs.value[current.target.id] = current
      entry.resolve(current)
      waiting.delete(id)
    } catch (reason) {
      // The backend does not know the run (it restarted): it will never end.
      if (isNotFound(reason)) {
        entry.resolve({ ...entry.run, status: 'failed' })
        waiting.delete(id)
      }
    }
  }
}

function schedulePoll(delay: number): void {
  if (pollTimer !== undefined || waiting.size === 0) return
  pollTimer = window.setTimeout(async () => {
    pollTimer = undefined
    await reconcile()
    schedulePoll(POLL_EVERY_MS)
  }, delay)
}
```

3. In `finished()` (line 44), after `waiting.set(...)`: call `schedulePoll(POLL_AFTER_MS)`.
4. In `follow()` (line 27), after `subscribed = true`: `onStreamReset(() => { void reconcile() })`. (`main.ts` already calls `resetLaunches()` first when the backend restarted; then `waiting` is empty and this does nothing.)
5. `resetLaunches()` (line 51) also clears the poll timer: `window.clearTimeout(pollTimer); pollTimer = undefined`.

No backend timer is involved: the polling exists only while a run is waited for and the window is held open (so it is never hidden meanwhile).

### 9.4 Icons: `src/components/rain/RIcon.vue`

Today (lines 59-69) the `url` branch has no error handling, so a 404 from the media route draws a broken image. Changes:

- Add `ref` and `watch` to the import of `vue` (line 7).
- After line 29 (`const CHART_TOKENS = 5`) add `const imageFailed = ref(false)` and `watch(() => props.icon.value, () => { imageFailed.value = false })`.
- Replace `kind` (line 31) by:

```ts
// A brand slug that Rain does not ship, or an image that cannot be loaded, falls back to a letter tile.
const kind = computed(() => {
  if (props.icon.kind === 'brand' && !brand.value) return 'letter'
  if (props.icon.kind === 'url' && imageFailed.value) return 'letter'
  return props.icon.kind
})
/** The text of the tile: the name of a letter icon, or the `fallback` of a url icon that failed. */
const letterText = computed(() => (props.icon.kind === 'url' ? (props.icon.fallback ?? '?') : props.icon.value))
```

- In `tintFor(props.icon.value)` (line 39) use `letterText.value`; in the letter `<span>` (line 78) use `{{ letterText.trim().charAt(0) }}`.
- On the `<img>` (lines 59-69) add `@error="imageFailed = true"`.

This also gives game covers and any later `url` icon the same fallback.

### 9.5 Apps screen, detail panes, actions, command

| File and lines | Change |
|---|---|
| `src/features/apps/AppDetail.vue` line 40 | `<PathText v-if="app.exePath" :path="app.exePath" />` followed by `<p v-else-if="app.kind === 'store'" class="text-13 text-muted-foreground">Store app</p>` |
| `src/features/apps/AppDetail.vue` lines 69-72 | add `v-if="app.exePath"` to the "Open file location" button (a Store app has no file) |
| `src/features/apps/actions.ts` lines 42-56 | build the list as `[launch, ...(app.exePath ? [revealAction(app.exePath, 'secondary', 'Open file location')] : []), pinAction(...), ...(app.exePath ? [copyAction('path', app.exePath)] : [])]`, so the slots keep their order and a Store app has no reveal and no copy |
| `src/features/apps/actions.ts` (new, after `isRunning`) | `export async function rescanApps({ api, notice }: Pick<ActionContext, 'api' \| 'notice'>) { const found = await api.rescanApps(); refreshResources('apps', 'pools', 'pinned', 'recent'); notice.info(found.length === 0 ? 'Still no apps found' : `Found ${found.length} apps`) }` |
| `src/features/apps/PoolDetail.vue` line 62 | `launch({ type: 'app', id: member.appId, args: member.args }, appById.value.get(member.appId)?.name)` |
| `src/features/apps/AppsScreen.vue` lines 178-184 | the empty state becomes `<EmptyState v-else-if="noApps" icon="layout-grid" title="No apps found" hint="Rain lists the apps that Windows shows in the Start menu." action-label="Scan again" @action="scan" />`; script: `const notice = useNotice()` and `async function scan() { try { await rescanApps({ api, notice }) } catch (reason) { notice.error(errorMessage(reason, 'Could not scan for apps')) } }`; imports `rescanApps` from `./actions`, `useNotice` from `@/composables/useNotice`, `errorMessage` from `@/lib/errors`. The `nav.go('settings')` call at line 183 is removed |
| `src/actions/commands.ts` after the `new-pool` entry (line 99) | `{ id: 'rescan-apps', title: 'Rescan apps', icon: 'refresh-cw', keywords: ['apps', 'installed', 'refresh', 'new app'], run: (ctx) => rescanApps(ctx) }`; add `rescanApps` to the import of `@/features/apps/actions` (line 6). `refresh-cw` must exist in `src/lib/icon-map.ts` (add it if not) |
| `src/features/apps/PoolEditorDialog.vue` | no change. The dialog cannot set `args` or `delayMs`; they are set in `pools.toml` (section 4.2) and kept when a pool is saved (lines 76-81 already keep them). Open point 12 asks the user whether the dialog should get these fields |
| `src/features/home/HomeScreen.vue`, `src/actions/targets.ts`, `src/features/home/homeRows.ts`, `src/actions/common.ts` | no change. Ids from `search`, `listPinned`, `listRecent` come from the same lists, so `useTargetResolver` always finds the entity (contract report finding 1); the property test of section 10 guards this |

### 9.6 Mock adaptation and what is deleted

The mock stays complete until stage 10 (`VITE_RAIN_MOCK=1` runs the whole app on it). In stage 4:

| File | Change |
|---|---|
| `src/data/mock/seed/apps.ts` | replaced by `export interface AppSeed { apps: AppEntry[]; pools: Pool[]; brokenAppIds: ID[] }` and `export function seed(): AppSeed { return { apps: [], pools: [], brokenAppIds: [] } }` (types imported; all rows, generators and helpers deleted). Reason: the mock must not return fake apps, pools, pins or recents next to real ones |
| `src/data/mock/store.ts` line 63 | call `seedApps()` (the argument is no longer used; drop it and the `scenario` import if unused) |
| `src/data/mock/slices/launcher.ts` | add `async rescanApps() { await delay(); return clone(state.apps) }`, and add `'rescanApps'` to the `Pick` on line 9-10. Everything else stays: `search`, `listPinned`, `listRecent`, `setPinned` still serve project, game, session, job and idle-job items |
| `src/data/mock/slices/launches.ts` | none (the router sends only project and game launches to it) |

Foundation 14.3 lists `mock/slices/launcher.ts`, the app and pool part of `launches.ts` and `library.ts`, and `seed/apps.ts` as deleted by stage 4. That cannot be done yet because the same slices still serve projects, games and the search of sessions and jobs. The exact deletion schedule (open point 1):

| Mock code | Removed in |
|---|---|
| seed rows of apps and pools; app and pool branches of `launches.ts` (`appStep`, `poolSteps`) | stage 4 (seed emptied) and stage 5 (branches deleted when `launches.ts` goes) |
| `mock/slices/launches.ts` completely (`markGamePlayed`, its only user is `slices/gameMode.ts`, moves into that file first: section 02 9.6). It implements the mock `launch` and `getLaunch`, so the same commit adds `launch` and `getLaunch` to `RetiredMockMethod` (foundation 8.6, stage 5 row) | stage 5 |
| `launcher.ts` methods `listApps`, `listPools`, `savePool`, `deletePool`, `rescanApps`, `createPool`, and the app and pool parts of `itemFor` and `recentItems` | stage 5 (with `library.ts`). The same commit adds `listApps`, `listPools`, `savePool`, `deletePool` and `rescanApps` to `RetiredMockMethod` (foundation 8.6; no stub is left behind) and removes the `seedApps()` call of `store.ts` |
| `launcher.ts` `search`, `listPinned`, `listRecent`, `setPinned` and `mock/search.ts` | stage 10 (sessions, jobs and idle jobs leave the mock in stages 6 and 8; the router special cases of this area are deleted in stage 8) |
| `seed/apps.ts` | stage 5 |

### 9.7 `docs/ui-spec.md`: replacement text (do not edit the spec now; the lead applies these in stage 4, the stage that builds the apps screen change)

**Section 4.3 table, row "App"** becomes:

| Item | Where | Enter | Ctrl+Enter | More in Ctrl+K |
|---|---|---|---|---|
| App | anywhere | Launch | Open file location (only for an app that has a file; a Store app has none) | Pin, Copy path (only for an app that has a file) |

**Section 7.1, bullet "Sources" under "With a query"** becomes:
"Sources: `api.search(query)` plus commands scored locally, merged by `score`, top 50. The backend ranks apps, pools, projects, games, sessions, jobs and idle jobs with the same scorer as `lib/score.ts` (a keyword match counts 60 percent of a title match). No groups, no type filter. The first row is selected. Alt+1..9 number the first nine rows."

**Section 7.1, "Launching from Home"** adds at the end: "The window stays open while any step runs. If the final state is lost (for example the stream reconnects) the screen asks `getLaunch` every 3 seconds after the first 5."

**Section 7.2, bullets "AppDetail", "Empty" and "Filtering"** become:
- "`AppDetail`: 32 px icon, name, `PathText` of the file (or the line "Store app" for an app without a file), running state, "In pools" (names). Buttons: Launch, and Open file location when the app has a file."
- "Empty: no pools shows one dashed 40 px row "New pool" + `KeyHint` Ctrl N in the Pools group. No apps at all: `EmptyState` `layout-grid` "No apps found", hint "Rain lists the apps that Windows shows in the Start menu.", button "Scan again" (asks the backend to read the app list again; the command "Rescan apps" does the same from Home)."
- "Filtering (chip): matches app and pool names and keywords. Keywords come from the user's `app-ids.toml`, the file name of the program, and a small built-in list of common words ("browser", "editor")."
- add: "A pool's apps can have command-line arguments and a delay; they are set in `pools.toml` and shown in the detail pane (the editor keeps them)."
- "App icons are the icons Windows shows for the app; when one cannot be loaded the row shows a letter tile."

**Section 8.3, line `AppEntry`** becomes: `export type AppKind = 'program' | 'store' | 'other'` and `export interface AppEntry { id: ID; name: string; icon: IconRef; kind: AppKind; exePath?: string; keywords: string[]; pinned: boolean; running: boolean; lastLaunchedAt?: ISODateTime }`; **`IconRef`** gains `fallback?: string` ("kind 'url': image URL; `fallback` is the text whose first letter is drawn when the image fails"); **`LaunchTarget`** gains `args?: string` ("type 'app' only"); **8.2 service interface** gains `rescanApps(): Promise<AppEntry[]>  // POST /apps/scan`.

**Section 10 "Out of scope for the prototype"**: the words "real app discovery and launching, icon extraction" are deleted at stage 4 (they are in scope from stage 4).

---------------------------------------------------------------------------------------------------

## 10. Tests

### 10.1 Unit tests (Rust, next to the code; no OS, no clock, no real files outside a temp folder)

| File | What is proven |
|---|---|
| `launcher/score.rs` | every case of `src/lib/score.vectors.json` equals `score()`; the `rank` cases (title beats keyword; "web" ranks pool "Web dev" above an app that only has the keyword; share 0.6 with the minimum 1); UTF-16 length cases (surrogate pair counts 2); `is_js_space` on U+0085 and U+FEFF |
| `launcher/apps.rs` | filter rules F1 to F7 on `tests/fixtures/launcher/apps-raw.json` (expected kept ids listed in the test); kind and key rules, including `microsoft.autogenerated.` keys; two "Photos" and three same-exe items stay separate; the U+A789 name gets a hash id; slug and id issue (clash suffixes `-2`, `-3`, ids of absent apps stay taken); keyword derivation and the hints table; `exePath` normalisation and `%VAR%` expansion |
| `launcher/running.rs` | `RunningIndex` from `tests/fixtures/launcher/processes.json` (image paths, including `\\?\`, forward slashes, different drive-letter case, a `WindowsApps` path); program match, Squirrel `--processStart` match, Store pair match, `other` never; the 8 s grace with a fake clock |
| `launcher/store.rs` | `pins.toml`, `pools.toml`, `app-ids.toml`: parse, validation failures (each rule of section 4 has a case), defaults for a missing file, round trip keeps comments, key order, unknown keys and CRLF; removing a `[[pin]]` removes only the comment lines directly above it |
| `launcher/pools.rs` | every validation rule and message of 6.7; id issue and clash; rename keeps the id; delay sum |
| `launcher/launch.rs` | the run state machine with a fake `Opener` and `tokio::time::pause`: steps run in order; delay is waited before the start; a failed step does not stop the others; `stops_run` leaves later steps `idle`; unknown member gives "App not found" and the others continue; final status; the history rows (app per success, pool only on full success); the final frame is published after the history write; throttle drops middle frames but never the final one; double launch returns the same run; eviction by count and by age; the decision table of 6.8.1 produces the expected `OpenRequest` for each app kind and argument case; error text mapping |
| `launcher/icons.rs` | hash and `v` are stable; file name is `<hex16>.png`; cache clean keeps referenced files; a failing renderer is remembered as "no icon" until a scan |
| `launcher/search.rs` | candidate assembly with fake `ItemSource`s: type filter, blank query, cap 50, sort order and tie-break; hidden items absent |

### 10.2 Integration tests (`crates/rain-backend/tests/launcher.rs`, on the harness of foundation 12.2)

Fixtures to add: `tests/fixtures/launcher/data/` (`pins.toml`, `pools.toml`, `app-ids.toml`, each with comments, a `schema_version`, and one CRLF variant `pools-crlf.toml` used by a test that copies it over `pools.toml`), `tests/fixtures/launcher/apps-raw.json` (about 40 made-up items: fake names, fake paths under `C:\Fake\...`, no personal data), `tests/fixtures/launcher/processes.json`. The folders `typical` and `empty` of foundation 12.2 get the three files too (`typical`: three apps pinned, two pools; `empty`: none).

Traits and fakes (foundation 12.2 builder `.with::<dyn T>(..)`):

| Trait | Production | Fake |
|---|---|---|
| `AppSource` (`enumerate() -> Vec<RawApp>`) | `WindowsAppSource` (`platform::shell::list_apps`) | `FakeAppSource` reads the JSON, counts calls, can be set to fail or to return nothing, and can be changed between calls to test rescans |
| `IconRenderer` | `ShellIconRenderer` | `FakeIconRenderer` returns a 1x1 PNG, can fail for chosen ids |
| `Opener` | `WindowsOpener` | `FakeOpener` records every `OpenRequest`, answers from a script, can delay or fail a call |
| `ProcessLister` | `ToolhelpProcessLister` | `FakeProcessLister` from the JSON |
| `ItemSource`, `LaunchPlanner` | the other modules | `FakeItemSource` (projects, games, sessions, jobs, idle jobs with chosen items) and `FakePlanner` |

Tests:

| Test | Steps and expectation |
|---|---|
| apps list | start with the fixture: `GET /apps` returns the expected ids in name order, `kind` and `exePath` as expected, `icon.value` of the form `media/app-icon/<id>?v=<8 hex>`; `app-ids.toml` was created once and not rewritten by a second scan |
| ids stable | rename an app in the fake, reorder the fake list, drop and re-add an app, rescan (`POST /apps/scan`): ids unchanged; a new app gets a new id; the file gains only the new `[[app]]` |
| broken id file | the `broken` variant of `app-ids.toml`: scan still works with provisional ids and `GET /health` shows the issue |
| first scan wait | `FakeAppSource` that takes 300 ms: `GET /apps` right after start returns the full list (waited), not an empty one |
| scan failure | fail the source: with a cache the list is returned and `sources` shows `failed`; with an empty cache `GET /apps` is `503 scanFailed`; `POST /apps/scan` is `503` in both cases; after the source works again a retry returns the list |
| pins | pin, unpin, pin twice (no second write: file mtime unchanged), pin an unknown id (404), pin a dangling id by hand-editing the file (skipped in `GET /pinned`, kept in the file), pin order, comments kept (byte compare of a file with comments after a pin and an unpin) |
| pools | create (201, `id` slug), update keeps id and `pinned`, delete (204, pin and history rows gone), second delete 204, each validation error with its reason, `idMismatch`, hand edit by writing the file then `GET /pools` after the debounce, a file with an error returns the last good list and the write routes answer `409 conflict configBroken`, CRLF file stays CRLF |
| launch app | `POST /launches` (app): 201 with step 0 `running`; frames `launch` arrive with the final `success`; `GET /apps` already shows `lastLaunchedAt` and `GET /recent` the app when the final frame is read; the `FakeOpener` saw one `OpenRequest` of the kind the decision table says |
| launch pool | three members, one with a delay of 200 ms (paused clock), one missing app, one failing: steps map 1:1 to members, order kept, statuses as expected, run `failed`, apps that started are in recent, the pool is not |
| double launch | second `POST /launches` for a running target returns 200 with the same run id; after the end a third returns a new run (201) |
| unknown ids | unknown app and pool: 404 with the reasons of section 8; `getLaunch` unknown: 404 |
| getLaunch | during and after a run; after 10 minutes of fake time the run is gone (404); the 65th run evicts the oldest finished one |
| search | the cases of the contract report: "web" ranks pool "Web dev" above an app with keyword "web"; blank `q` gives `[]`; `types=app` filters; cap 50 with 60 matching fake items; `FakeItemSource` items of all five other types appear, ranked together |
| property test | for 300 random queries and a random fixture (random pins, random recents, `FakeItemSource` with some hidden items): every id returned by `search`, `GET /pinned` and `GET /recent` exists in the list of its type (`GET /apps`, `GET /pools`, or the `ItemSource`), and no id appears in two types (ids are unique across the four launchable types) |
| recent | order newest first; limit; a pool launch lists the pool and its apps; deleting the pool removes it; items of an absent `ItemSource` id are skipped; history pruning keeps 20 rows per target |
| media | `GET /media/app-icon/<id>?v=..`: 200 PNG with `ETag`, `nosniff`, `Cache-Control` immutable; a second request with `If-None-Match` gives 304 and the renderer is not called again; unknown id gives 404 JSON; a failing renderer gives 404 `noIcon`, then still 404 without a new render until a scan; the file exists in `cache\launcher\icons` and a restart serves it without rendering |
| game mode | with `game_mode_active` set: `POST /launches` is `409 gameMode`; the only exception is `launch_after_game_mode` of section 02 (no run); `POST /apps/scan` is 409; `GET /apps` still works |
| topics | every `launch` frame arrives with a valid `LaunchRun`; a `resync` arrives on connect; a run that finished while no client was connected is still available by `getLaunch` |
| source rules | `tests/source_rules.rs` passes: no write to the vault, no `.claude` strings, no tracing field named `text`, `content`, `detail`, `prompt`, `body` or `secret` (the `OpenError` fields are `ui_text` and `log_reason` on purpose); only `platform/` contains `unsafe` and `Command::new` |

Process smoke (`tests/process_smoke.rs`) gets one more step in stage 4: `GET /apps` against the real binary returns without error (the real shell enumeration runs; the count is not asserted), and `GET /media/app-icon/<first id>` returns a PNG. This is the only automated test that touches the real Windows shell, and it never starts a program.

### 10.3 Frontend tests (vitest)

| File | What |
|---|---|
| `src/lib/score.vectors.test.ts` | writes the vectors when `RAIN_WRITE_VECTORS=1`, otherwise asserts every case against `score()` |
| `src/data/rest/client.test.ts` (extends the file of stage 1b) | path, method and body of the eleven methods of 9.2 on a fake `Bridge`; `savePool` picks POST or PUT by `draft.id`; `qs` encoding of `q` with spaces and `types` |
| `src/data/rest/media.test.ts` | `media/app-icon/x?v=1` inside `AppEntry.icon`, a `SearchItem.icon` and a `launch` frame step icon is rewritten |
| `src/data/router.test.ts` | `search` merges two fake sides by score with real first on ties and survives one side failing; `listPinned` and `listRecent` concatenate; `setPinned` and `launch` pick the side by type; `getLaunch` follows the run set; `subscribe('launch')` attaches to both and returns one unsubscribe |
| `src/composables/useLaunch.test.ts` | with a fake `api` and fake timers: a lost final frame is repaired by the 5 s poll (`getLaunch` returns `success`, `launch()` resolves, the hold is released); `onStreamReset` reconciles at once; `notFound` from `getLaunch` ends the wait as `failed`; no poll runs when nothing waits |

Screens and `RIcon` are not unit tested (as today); they are covered by the acceptance checks.

### 10.4 What cannot be tested automatically, and how it is checked

| Item | Check |
|---|---|
| `ShellExecuteExW` with `SEE_MASK_FLAG_NO_UI` on a missing file shows no dialog; a classic AppsFolder item starts through `shell:AppsFolder\<AUMID>`; the fallback to target plus arguments works | by hand in stage 4 (acceptance 5 and 6) |
| A Store app with arguments through `ActivateApplication` | by hand with one Store app that takes arguments, or "not tested" if none is at hand |
| Icons for all items; the letter tile fallback | acceptance 3 |
| `RecursiveMode::Recursive` on the Start Menu | acceptance 8; if it fails the module registers each sub-folder non-recursively (open point 5) |
| Idle cost | the stage 2 measurement script, repeated after stage 4 |
| The dev-only fault route | used by acceptance 9 |

---------------------------------------------------------------------------------------------------

## 11. Acceptance checks

Besides the checks of foundation 12.4 (`npm run check`) and the stage 4 row of foundation 14.3.

### 11.1 On the real PC (release-like debug build, `npm run tauri dev`, sandbox folders)

| # | Action | Expected |
|---|---|---|
| 1 | Start Rain on a data folder without the three files. Open Apps. | The list appears within about 1 second (the first scan is waited for). Between about 90 and 125 apps (this PC lists 125 items; documents, web links, uninstallers and `steam://` items are filtered). `app-ids.toml` now exists beside the data folder's other files, with one entry per app and the header comment. No Start Menu `.lnk` file is opened by Rain (nothing is written to the Start Menu) |
| 2 | Restart the backend (`npm run backend:restart`) and reopen Apps. Pin "Notepad", create a pool of three apps, restart again. | Same ids (compare `app-ids.toml` before and after; unchanged), the pin and the pool are still there, the backend start did not rewrite `app-ids.toml` |
| 3 | Look at the icons in Apps, Home and the pool editor, in `tauri dev` and in the plain browser at `http://localhost:1420`. | Real icons, also for Store apps and for apps that have a shell icon only. Clear `<local>\cache\launcher\icons` while the backend is stopped, start again: icons are made again (first view a little slower), no error. Block one icon by temporarily renaming the fake test app's exe (or use the fault route) to see the letter tile instead of a broken image. If the page does not load `http://rain-media.localhost/...` in `tauri dev`, add `img-src 'self' data: http://rain-media.localhost` to the CSP (foundation appendix C) and record it |
| 4 | Type "web", "calc", a word from a pool member's name, "dev web" in Home. | Results are ranked like the mock (title beats keyword; the pool of a matching app is found), within the perceived 30 ms; a command and an app with similar names sort by score on one scale. Compare the ranking with the shared score vectors (`src/lib/score.vectors.json`, run by `score.vectors.test.ts` and by the Rust test of `launcher/score.rs`, section 11.2) instead of against the mock: stage 4 empties the mock app and pool seed (9.6), so the mock holds no apps to compare with. A side-by-side comparison with the old mock is possible only on the stage 3 checkout, before the seed is emptied |
| 5 | Launch a classic app from Apps (Enter) and from Home. Then `npm run backend:stop`. | The app starts in under about 1 second and stays open after the backend stops (it is not in any job of Rain). Rain hides on success. A launch of a missing file (rename a throwaway test program) shows "<name>: Executable not found" in the checklist and notice, and NO Windows error dialog appears |
| 6 | Use the small test program of this stage: build it with `cargo build -p rain-backend --bin rain-test-app` (a program that sleeps for 5 minutes), make a throwaway Start Menu shortcut with `scripts/make-test-shortcut.ps1` (it creates `Rain Test App.lnk` in the user's Start Menu `Rain Test` folder; the script's `-Remove` switch deletes it), press "Scan again", launch it, stop the backend. Also launch one Store app (for example Calculator). | The test program appears in the list within the 2 second quiet time of the Start Menu watcher (or after "Scan again"); it runs after the backend is stopped; it has `running` true in Apps; Calculator starts through activation; remove the shortcut afterwards with `-Remove` and end the test program |
| 7 | Edit `pools.toml` by hand: give one member `delay_ms = 2000` and `args`, add a comment above the pool. Launch the pool. Save the pool in the editor without changes. | The pane shows `+2 s` and the arguments mono text (after the next fetch, for example `Ctrl+R`); the second member starts about 2 seconds after the first; saving in the editor keeps the delay and the arguments and the comment above the pool; one pool member that is a removed app shows "Unknown app" and fails with "App not found" while the others start; "Retry failed" starts the failed member with its arguments |
| 8 | Install or remove anything that puts a shortcut in the Start Menu (or copy any `.lnk` into the user Start Menu folder, then delete it). | The list changes after about 2 seconds of quiet (the next fetch shows it); if the recursive watch does not report sub-folders, open point 5's fallback is built before the stage ends |
| 9 | "Empty" state: with the backend running in sandbox mode, run `node scripts/backend.mjs fault apps empty`, then `Ctrl+R` in Apps. Click "Scan again". Then `node scripts/backend.mjs fault apps normal` and click "Scan again". | With the fault: Apps shows the empty state "No apps found" with the button "Scan again" (and no "Open settings"), Home shows "Nothing pinned yet" if nothing is pinned; "Scan again" with the fault still on says "Still no apps found"; with the fault off the list returns and the notice says "Found N apps". `fault apps fail` shows "Apps unavailable" with Retry |
| 10 | Stop the Windows shell wait: not testable (a hung COM call). | Covered by the unit test of the timeout; foundation appendix C |
| 11 | Close the window while a pool launches (a pool with a 20 s delay on its last member), then reopen. | The run continues; the window was held open while it ran, so closing is refused for that time (existing behaviour); after reopening, the checklist shows the true state (the poll or the next frame) |
| 12 | Idle check: after the scan, leave the backend idle for 60 seconds. | Working set under 30 MB and CPU time gained under 0.1 s (foundation stage 2 measurement) |

### 11.2 Automated

`npm run check` passes with: the score vectors in both languages, the property test of section 10.2, the strict type-identity check for the types of section 3 (a hand-written copy removed from `types.ts` in the same commit; `scripts/check.mjs` finds no duplicate names), `npm run types:check`, the integration tests above, and the process smoke including `GET /apps` and one icon.

### 11.3 The dev-only fault route and its script

`PUT /internal/test/apps-source` exists only when `rain_types::env::sandbox()` is true (otherwise `404 noRoute`). It swaps the `AppSource` of the running module for a fake that returns the normal list (`normal`), nothing (`empty`) or an error (`fail`). `scripts/backend.mjs fault apps <mode>` sends it through the pipe with Node's `http.request({ socketPath })`; the Vite proxy and the shell refuse `/internal/` as before. This is the "Busy and Failing" pattern of foundation 12.3.

---------------------------------------------------------------------------------------------------

## 12. Open points

Only what cannot be decided inside this area. Everything else is decided above, with its reason.

| # | Question | Who decides | What the code does until it is decided | Blocks |
|---|---|---|---|---|
| 1 | SETTLED in foundation 14.3 (stage 4 row, column "Mock deleted"; it follows section 9.6). Was: foundation 14.3, stage 4, column "Mock deleted": the mock files it lists cannot be deleted in stage 4 (they still serve projects, games and the search of sessions, jobs and idle jobs). Section 9.6 has the schedule: seed emptied in stage 4, `launches.ts`, the app and pool methods of `launcher.ts` and `seed/apps.ts` in stage 5, the rest in stage 10 | architect | follows section 9.6 | the stage 4 report wording |
| 2 | SETTLED in foundation 14.4 (row `rescanApps`) and 15. Was: new route `POST /apps/scan` and new `RainApi` method `rescanApps` (registry 14.4 and the contract table 15 need a row). Also `args?` on `LaunchTarget`, `fallback?` on `IconRef`, `exePath?` and `kind` on `AppEntry` | architect | as written in sections 2 and 3 | stage 4 |
| 3 | SETTLED in foundation 14.4 (row `(internal, sandbox only)`) and 12.3. Was: the dev-only fault route `PUT /internal/test/apps-source` (section 11.3): is the `/internal/test/...` pattern of foundation 12.3 open to a section without a new shared rule? | architect | built as written, sandbox only | acceptance check 9 |
| 4 | SETTLED by the editor: `ItemSource` is implemented only by projects and games; this module maps session, job and idle job items itself from the `list()` calls of section 03. The cross-section interfaces `ItemSource` and `LaunchPlanner` (section 6.9) and the id rule (section 02 must not issue ids starting with `app-` or `pool-`; section 03 ids of sessions, jobs and idle jobs are never pinned or launched so they only need to be unique within their own type). Section 02 implements the traits in its `Service` for stage 5 (section 03 implements none). The traits and the types around them live in `core/launch_contract.rs` (foundation 11.1, 11.2), so the module-call rule holds. DONE | architect, to relay to sections 02 and 03 | the stage 4 stubs return empty answers and `notBuilt`; the router sends those types to the mock | stage 5 search, pins, recent and launch of projects and games; stage 6 and 8 search |
| 5 | The config hub needs a call to watch a folder tree and tell the module, with a recursive flag (foundation 10.4 proves only non-recursive watches of the data folder and registered sub-folders). `RecursiveMode::Recursive` on the Start Menu folders is UNPROVEN | architect (hub API); verified in acceptance 8 | the module uses the recursive call; if it does not report sub-folders the module registers each Start Menu sub-folder non-recursively (about 17 folders; the foundation's proven mode) | stage 4 acceptance 8 |
| 6 | DONE in foundation 4.7 (nothing left to ask). The core media route must compute the `ETag` from `kind`, `id` and the `v` query value and answer `304` before it calls `MediaProvider::get` (foundation 4.7 says the 304 is decided before the bytes are produced, while the trait returns bytes). The `AppIcons` provider assumes this | architect | the provider is only called on an ETag miss | stage 2 media route |
| 7 | DONE in foundation 5.4 (`Throttle::publish_final`; the engine uses it, no wrapper). `core::throttle::Throttle` must let a publisher cancel its pending value and publish the final one at once (section 2, final frame never delayed) | architect | the engine wraps the call: cancel pending, then `bus.publish` | stage 4 |
| 8 | Foundation 4.5 names `ApiJson` and `ApiQuery`; this section also needs `ApiPath` (same pattern with `axum::extract::Path`) for `{type}` and `{id}` | architect | the module defines `ApiPath` locally with the same one-line derive if core has none | stage 4 |
| 9 | SETTLED in foundation 8.6 (the `launch` topic special case is there). Was: foundation 8.6 gives each topic one side; `launch` must subscribe to both sides during stages 4 and 5 (section 9.2) | architect (update 8.6 and the router test) | the router special case of section 9.2 | stage 4 and 5 |
| 10 | Unproven items to add to foundation appendix C, each with this section's defensive rule: `ShellExecuteExW` with `SEE_MASK_FLAG_NO_UI` on a missing file; a classic AppsFolder item through `shell:AppsFolder\<AUMID>` (fallback: target and arguments); the stability of `Microsoft.AutoGenerated.{GUID}` AUMIDs across reinstalls (key by target and arguments); icons through the AUMID path for every item (fallback: the target path, then the letter tile); recursive `notify` on the Start Menu | architect | defensive rules as in sections 6.1, 6.2, 6.8.1 | none |
| 11 | `IconRef.fallback` is a shared type change: sections 02 (game covers) and any later `url` icon benefit; section 02 should set `fallback` to the game title | architect to tell section 02 | section 02 may ignore it (a cover that fails then shows a "?" tile) | none |
| 12 | Does the user want `args` and `delayMs` fields in the pool editor, and a "Hide app" action? This round both are hand edits in `pools.toml` and `app-ids.toml` (documented in their commented headers). The design direction (minimal, not text-heavy) argues for waiting until the user asks | user | hand edit; the editor keeps what the file has | none |
