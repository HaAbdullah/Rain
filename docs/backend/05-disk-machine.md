# Rain backend, section 05: disk and machine

| | |
|---|---|
| Area | disk (drives, scan, subtree, reveal, path check) and machine (user idle time) |
| Modules | `disk`, `machine` (foundation section 11.2), and `hardware` (section 13, added later).  The route `POST /paths/check` is owned and built here, in `disk` (open point 1); section 06 does not define it |
| Stage | 3, "Read-only sources" (foundation section 14.3). |
| Author | feature designer for disk and machine |
| Date | 2026-11-22 |
| Status | draft 1. Binding inputs: `decisions.md`, `00-foundation.md` (draft 2), `foundation-review.md` |

How to read this file: it follows the template of foundation section 16. Words such as "shell", "backend", "topic", "frame" and "stage" have the meaning of the foundation. "Arena" is the in-memory copy of one finished scan (section 6.5). "Budgeted tree" is the small tree the UI gets (section 6.5). "Not accessible" is the UI word for a folder that Windows did not let Rain read.

Sources the code is copied from (all below `scratchpad/backend/spikes/`, appendix D of the foundation): `winapi/src/walk.rs` (walker, arena, budgeted cut), `winapi/src/bin/walk.rs` (test driver), `winapi/src/bin/sysinfo.rs` (drives, idle time), `process/src/bin/q7.rs` (`reveal`).

---------------------------------------------------------------------------------------------------

## 1. Scope

### 1.1 What this area does

| Part | What | Stage |
|---|---|---|
| Drives | List the fixed local drives with letter, label, size and used bytes. Read live, never cached. | 3 |
| Disk scan | Walk one drive with 8 threads and no administrator rights, keep the result in memory, report progress on topic `scan`, cancel on request. | 3 |
| Budgeted tree and lazy subtree | The scan result is a small tree (about 2000 nodes). A folder that is not in it is fetched by a second route. | 3 |
| Reveal | Show a file or folder selected in Explorer. | 3 |
| Path check | Tell the Settings screen whether the folders a user typed exist. Replaces `mockControl.pathExists`. | 3 |
| Machine state | Seconds since the last keyboard or mouse input, as `MachineState`. Also the public function `machine::Service::idle_ms()` that the idle jobs of section 03 sample. | 3 |

### 1.2 What it does not do

- No deletion, moving or cleaning of files (UI spec 7.7: "No delete in the prototype").
- No administrator rights, no raw MFT read (decision 2026-11-22: normal parallel walk).
- No scan history, no saved scan, no scan after restart (section 5: no tables).
- No disk settings. There is no `disk.toml` and no exclude list (section 4).
- No CPU, GPU or memory load reading, and no "full screen app is running" reading. No screen needs them in this round (section 6.10). Section 03 may ask for them through the architect.
- No removable, optical or network drives in the drive list (section 6.1).
- No `machine` topic (decided by the architect, foundation-review 2.5). The page asks `GET /machine` when it needs it.

### 1.3 `RainApi` methods and topics

| `RainApi` | Route | Module | Status against the current `api.ts` |
|---|---|---|---|
| `listDrives` | `GET /drives` | disk | unchanged signature |
| `startScan` | `POST /scans` | disk | unchanged signature; body `{ driveId }` |
| `getScan` | `GET /scans/{id}` | disk | unchanged signature; `root` only on `done` |
| `cancelScan` | `DELETE /scans/{id}` | disk | unchanged signature; also frees a finished scan (section 6.2) |
| `getScanNode` | `GET /scans/{id}/node?path=` | disk | NEW (decision C11) |
| `revealPath` | `POST /shell/reveal` | disk | unchanged signature; the rules of "which paths" are new (section 6.9) |
| `checkPaths` | `POST /paths/check` | disk | NEW; replaces `mockControl.pathExists` |
| `getMachineState` | `GET /machine` | machine | unchanged signature |
| `subscribe('scan', ..)` | stream topic `scan` | disk | unchanged name; the payload never carries `root` (section 2, S.5) |

Topic: `scan`. No other topic.

### 1.4 Decisions of `decisions.md` that this section implements

- Section 5, "Disk scan": normal parallel folder walk, no administrator rights; unreadable folders are counted and reported as "not accessible", never a failure.
- C11: a budgeted tree in the scan result plus a lazy route for a subtree. The whole tree of a big drive is never sent in one response.
- Section 6, "No personal details": no path of this PC is in any fixture or document. Tests build their trees in a temp folder or in a fake.
- Foundation 11.5: timers T5 (scan progress) and T6 (arena expiry) are used; nothing else.
- Foundation 11.6: walker rules, COM thread for reveal, drive and idle calls.

---------------------------------------------------------------------------------------------------

## 2. Routes

Paths are after `/api/v1`. Every route is in the registry of foundation 14.4 (rows `getScanNode` and `checkPaths`, module `disk`).

| Method | Path | Request | Response | Success | Errors (code, reason) | Idempotent | Topic effects |
|---|---|---|---|---|---|---|---|
| GET | `/drives` | none | `Drive[]` | 200 | `unavailable` `timeout` | yes | none |
| POST | `/scans` | body `StartScanBody { driveId }` | `DiskScan` (status `scanning`, no `root`) | 201 for a new scan; 200 when a scan of the same drive is already running and is returned as it is | 400 `invalid` `badBody`; 404 `notFound` `unknownDrive`; 409 `conflict` `gameMode`; 503 `unavailable` `shuttingDown` | A second call for the same drive while it scans returns the running scan (200). A call for another drive cancels the running scan and starts the new one. | publishes `scan` for the new scan and, if one was cancelled, a final `cancelled` frame for the old one |
| GET | `/scans/{id}` | none | `DiskScan`; `root` is present only when `status` is `done` and the tree is still held | 200 | 404 `notFound` `unknownScan` | yes. Reading restarts the 10 minute expiry (T6) | none |
| GET | `/scans/{id}/node` | query `path` (a `path` taken from a node of this scan) | `DiskNode` of that folder, with its children cut by the budget | 200 | 400 `invalid` `badQuery`, `notADirectory`; 404 `notFound` `unknownScan`, `unknownPath`; 409 `conflict` `scanNotDone`, `scanExpired` | yes. Restarts the expiry | none |
| DELETE | `/scans/{id}` | none | none | 204 | 404 `notFound` `unknownScan` | yes: on a scan that is not running and has no tree it does nothing | scanning: one final `cancelled` frame. Done: no frame, the tree is freed |
| POST | `/shell/reveal` | body `RevealBody { path }` | none | 204 | 400 `invalid` `badBody`, `badPath`, `networkPath`; 404 `notFound` `pathMissing`; 503 `unavailable` `pathSlow`, `comBusy`, `timeout`; 500 `failed` `revealFailed` | not idempotent in effect (it opens a window), safe to repeat | none |
| POST | `/paths/check` | body `PathsCheckBody { paths }` | `PathCheck[]`, same order and length | 200 | 400 `invalid` `badBody`, `tooMany` (more than 32 paths), `tooLong` (a path of more than 1024 characters) | yes | none |
| GET | `/machine` | none | `MachineState` | 200 | 503 `unavailable` `idleUnavailable` | yes | none |

Body limit: all bodies are far below `BODY_LIMIT` (1 MiB). URL: the node route carries a path in the query. The URL limit of 2048 bytes (foundation 3.4) allows a folder path of about 600 characters (each backslash costs three bytes after percent-encoding). A deeper folder answers `400 invalid` `urlTooLong` from the router and the UI shows the message. Windows allows 32 000 characters, but a folder deeper than 600 is rare; this is a known limit, not a bug (test in section 10).

Status 200 versus 201 of `POST /scans`: the REST client treats both as success (foundation 4.6).

### 2.1 Live topics

| Topic | Payload | Whole list or one item and key | Maximum rate | Publisher and when |
|---|---|---|---|---|
| `scan` | `DiskScan` without `root` (one scan, about 200 bytes) | one item, key `scan.id` | 4 per second (one frame every 250 ms while counters change) | the scan's progress task while it runs (T5); the finaliser exactly once with the final status (`done`, `failed`, `cancelled`); `cancel` for a scan it ends itself |

Payload size rule (the foundation forbids a frame over 256 KiB): the topic carries counters and a status, never the tree. `DiskScan.root` is NEVER present in a frame. A `done` frame has `progress: 1`, the final `filesSeen` and `notAccessible`, and no `root`. The tree is fetched by `GET /scans/{id}`. A frame is about 200 bytes, so `FRAME_MAX` cannot be reached; a test asserts that every frame of a scan with 5000 folders is under 1 KiB (section 10).

Replay rule between the answer of `POST /scans` and the first frame: the UI subscribes only after the answer arrives, so a frame published in between is lost. The stream has no replay (foundation 5.2). The rule is: (1) every frame is a full snapshot of the scan; (2) `GET /scans/{id}` returns the same snapshot at any time (the record of a scan stays for the life of the process, the last 8 are kept); (3) the final frame is published exactly once and is the last frame of that scan id; (4) the frontend calls `getScan` once right after it subscribes and after every `resync`, and applies the answer with the merge rule "a final state beats `scanning`; of two `scanning` snapshots the one with more `filesSeen` wins". So a scan that ends in 20 ms (an empty folder) is not lost, and a frame lost to a lag is repaired by the next `resync`. The code is in section 9 (`scanState.ts`, `scanDrive`, `reconcileDisk`).

---------------------------------------------------------------------------------------------------

## 3. Types

File `crates/rain-types/src/disk.rs`. One line per type is added to the `decls!` list in `bindings.rs`. The Rust names equal the TypeScript names that exist today; new names are marked NEW. All rules of foundation 9.2 apply: `camelCase`, optional fields absent (never `null`), string unions as enums with `#[ts(inline)]` on the field, `u64` becomes `number`.

```rust
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The eight colour classes of the treemap. Same names as the existing TypeScript union.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum FileCategory { Video, Image, Audio, Archive, Code, Document, App, Other }

/// What a node is. `Other` is NEW: the one node per folder that stands for the entries
/// that are not listed one by one ("other (1,204 items)").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum NodeKind { Dir, File, Other }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ScanStatus { Scanning, Done, Failed, Cancelled }

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct Drive {
    pub id: String,            // "drive-c": "drive-" + lower-case letter. Stable for a letter.
    pub letter: String,        // "C" (no colon; the UI adds it)
    pub label: String,         // the volume label, or "Local Disk" when Windows has none (never empty)
    pub total_bytes: u64,      // always > 0 (drives with 0 are not listed)
    pub used_bytes: u64,       // total - available to this user
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct DiskNode {
    pub name: String,
    pub path: String,          // see "path rules" below
    pub size_bytes: u64,       // logical size of the whole subtree; 0 for a folder Windows did not let Rain read
    #[ts(inline)]
    pub kind: NodeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(inline)]
    pub category: Option<FileCategory>,   // files: by extension. Folders: only a folder sent WITHOUT children (the dominant one by bytes)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<DiskNode>>,  // absent for a file, an empty folder, a denied folder and a truncated folder. Never `[]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,          // NEW. Only ever `true`. The folder has content that is not in this answer: fetch it with getScanNode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub denied: Option<bool>,             // NEW. Only ever `true`. Windows did not let Rain read this folder ("not accessible").
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct DiskScan {
    pub id: String,                       // "scan-20261122-093000-1"
    pub drive_id: String,
    #[ts(inline)]
    pub status: ScanStatus,
    pub progress: f64,                    // 0..1, rounded to 3 digits; at most 0.99 until done; then 1
    pub files_seen: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,            // only when status is failed; short, shown as it is
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_accessible: Option<u64>,      // NEW. Folders Windows did not let Rain read. Absent when 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<DiskNode>,           // only in the answer of GET /scans/{id} when status is done
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MachineState {
    pub user_active: bool,                // idle time under 60 s
    pub idle_minutes: u32,                // whole minutes since the last input (0 while active)
}

// ---- NEW: bodies and the path check ----

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartScanBody { pub drive_id: String }

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RevealBody { pub path: String }

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PathsCheckBody { pub paths: Vec<String> }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PathKind { Dir, File }

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct PathCheck {
    pub path: String,                     // the string that was sent, unchanged, so the caller can match it
    pub exists: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(inline)]
    pub kind: Option<PathKind>,           // present when exists is true
}
```

Topic struct (in `events.rs`, added in stage 3 with `DiskScan`): `pub struct Scan; impl Topic for Scan { const NAME: &'static str = "scan"; type Payload = DiskScan; }`.

The query struct of the node route, `ScanNodeQuery { path: String }`, is private to `disk/routes.rs`. It is not generated to TypeScript, because the REST client builds the query with its `qs` helper. It uses the `ApiQuery` extractor, so a missing `path` is `400 invalid` `badQuery`.

`PathCheck`, `StartScanBody`, `RevealBody` and `PathsCheckBody` are generated so that `rest/client.ts` can type its calls (`satisfies StartScanBody`).

### 3.1 Resulting TypeScript (what `rain-types.ts` contains; double quotes and `Array<T>` as the generator writes them)

```ts
export type FileCategory = "video" | "image" | "audio" | "archive" | "code" | "document" | "app" | "other";
export type NodeKind = "dir" | "file" | "other";
export type ScanStatus = "scanning" | "done" | "failed" | "cancelled";
export type Drive = { id: string, letter: string, label: string, totalBytes: number, usedBytes: number, };
export type DiskNode = { name: string, path: string, sizeBytes: number, kind: "dir" | "file" | "other",
  category?: "video" | "image" | "audio" | "archive" | "code" | "document" | "app" | "other",
  children?: Array<DiskNode>, truncated?: boolean, denied?: boolean, };
export type DiskScan = { id: string, driveId: string, status: "scanning" | "done" | "failed" | "cancelled",
  progress: number, filesSeen: number, error?: string, notAccessible?: number, root?: DiskNode, };
export type MachineState = { userActive: boolean, idleMinutes: number, };
export type StartScanBody = { driveId: string, };
export type RevealBody = { path: string, };
export type PathsCheckBody = { paths: Array<string>, };
export type PathKind = "dir" | "file";
export type PathCheck = { path: string, exists: boolean, kind?: "dir" | "file", };
```

### 3.2 What changes against `src/data/types.ts` and why

| Type | Today (`types.ts`) | After | Why |
|---|---|---|---|
| `FileCategory` | same union | same | none; the hand-written line (76) is deleted when the generated one is used |
| `Drive` | same fields | same | none; `label` is now never empty (fallback "Local Disk") |
| `DiskNode.kind` | `'dir' \| 'file'` | adds `'other'` | a folder with thousands of entries cannot list them all; one `other` node keeps the sums exact and the treemap area honest |
| `DiskNode.truncated` | none | `truncated?: boolean` | a folder of the budgeted tree whose children are not in the answer. Without it the UI would say "This folder is empty" (the one treatment of a folder without children, `DiskScreen.vue:159`) |
| `DiskNode.denied` | none | `denied?: boolean` | a folder Windows refused. The UI shows a lock icon and "Not accessible" instead of "This folder is empty" |
| `DiskScan.notAccessible` | none | `notAccessible?: number` | the count the Disk screen shows (section 9) |
| `DiskScan.root` | in every `done` snapshot, also on the topic | only in `GET /scans/{id}`; never in a frame | the frame limit (section 2.1) |
| `MachineState` | same | same | none; the meaning of `userActive` is now written down (60 s) |
| `PathCheck`, bodies, `PathKind` | none | new | new routes |

Path rules (the screen compares paths byte for byte; the mock obeyed the same rules):

- Root: `C:\` (drive letter in upper case, colon, one backslash). Root `name` is `C:`.
- Child: parent path, plus one backslash unless the parent already ends with one, plus the name: `C:\Users`, `C:\Users\x`.
- Original case of the file system. Backslashes. No `\\?\` prefix. No trailing backslash except the root.
- The `other` node's path is its folder's path plus `\*` (for the root `C:\*`). A star cannot occur in a Windows file name, so the path is unique among siblings, and `trailTo` (which descends only `kind: 'dir'`) never reaches it.
- A name that is not valid UTF-16 is shown with the replacement character; its path cannot be revealed (accepted, `notFound`).

Node invariants (the treemap draws leaf sizes while the list shows each folder's own size, so these must hold; a test checks them on every tree, section 10):

1. A folder sent with `children` has children whose sizes add up exactly to its `sizeBytes`. The entries not listed one by one are inside the `other` child.
2. A folder has at most one `other` child, and `other` is never a folder: it has no children and `kind: "other"`.
3. A folder either has `children`, or has `truncated: true` (content exists, not sent), or has `denied: true` (not readable), or is empty. Never two of these; never `children: []`.
4. A folder sent without `children` that has content carries `category` (the dominant one by bytes), so the treemap colours its block correctly. The frontend already honours it: `dominantCategory` reads `node.category` of a childless node (`treemap.ts:90`).
5. A file always has a `category`. An `other` node and a `denied` folder have none.

---------------------------------------------------------------------------------------------------

## 4. Data files

None. This area owns no file in the data folder.

Reasons: nothing the user decides lives here. Drives and scan results are machine facts. There is no exclude list, no scan setting and no list of favourite folders in the UI or in `decisions.md`. The extension table (section 6.6) is a compiled `match`, not a file: no user asked to change it. If a later round adds an exclude list or a category override, it goes into `settings.toml` (section 06), not into a new file.

Defaults when a file is missing: not applicable. What Rain writes back: nothing.

---------------------------------------------------------------------------------------------------

## 5. Tables

None. `disk::MIGRATIONS` and `machine::MIGRATIONS` are empty slices (`&[]`).

Reasons:

- The scan result is not a cache table. A big drive has more than a million nodes. The foundation decided to hold it in memory for at most 10 minutes after its last read (T6) and then drop it. Writing it to SQLite would cost seconds and tens of megabytes and gain nothing, because a scan is a deliberate action and takes seconds to minutes.
- The scan record (id, drive, status, counters) is a few hundred bytes, kept for the last 8 scans in memory. No screen shows a history, and a backend restart ends every scan anyway (`main.ts` calls `resetDisk()` when `startId` changes).
- Drives and idle time are read live.
- Retention: not applicable. If a scan history is wanted later, it is a new table `disk_scan` added by a new migration.

Because there is no store on disk, `modules/disk/store.rs` holds the in-memory `ScanTable` (records, the arena slot, the expiry deadline). `modules/machine/` has no `store.rs` (nothing to store); this is the only deviation from the file list of foundation 11.2.

---------------------------------------------------------------------------------------------------

## 6. Algorithms

### 6.1 Drives (`GET /drives`)

Trait and types (the trait is the seam for tests, foundation 12.2):

```rust
// modules/disk/drives.rs
pub struct DriveInfo { pub drive: Drive, pub root: PathBuf, pub root_name: String }   // prod: root "C:\", root_name "C:"
pub trait DriveSource: Send + Sync { fn list(&self) -> Vec<DriveInfo>; }
```

Production code (`platform/fs.rs`, copy `fixed_drives()` of `winapi/src/bin/sysinfo.rs`), run through `core::blocking::run`:

1. `GetLogicalDrives()` gives a bit mask; bit 0 is `A:`.
2. For each set bit: `GetDriveTypeW("X:\\")`. Keep only the value 3 (`DRIVE_FIXED`; the constant is written in the code because the named one needs the feature `Win32_System_WindowsProgramming`). This skips removable, optical, network and RAM drives. An external hard disk usually reports fixed and is listed; a USB flash stick is not. Decision: fixed local volumes only, as the research proposed. Reason: nothing was spiked for the others and a sleeping network drive can block a call for a long time.
3. `GetVolumeInformationW(root)` for the label. If it fails the label stays empty.
4. `GetDiskFreeSpaceExW(root, &mut available, &mut total, &mut free)`. `usedBytes = total - available` (foundation 11.6: use the figure available to the caller; with disk quotas it is the one the user sees in Explorer). If the call fails, or `total` is 0, the drive is skipped (an empty card reader, an offline disk). This also removes a division by zero in `DriveList.vue:72` (`usedBytes / totalBytes`).
5. Label: if empty (this PC's C: has an empty label), write `"Local Disk"`. The UI title is `"{letter}: {label}"`, so the row reads "C: Local Disk".
6. `id = "drive-" + lowercase letter`, `letter` upper case. Sorted by letter.

No `SetErrorMode`: the volume calls are made only for fixed drives, which never show the "no disk" dialog. (This avoids a new `windows` feature, `Win32_System_Diagnostics_Debug`.)

Cost: about 35 microseconds per drive (spike). Not cached. The blocking helper is used because a fixed external disk that sleeps may need seconds to answer; the request limit of 15 s then ends the call with `unavailable` `timeout`. A volume that is mounted into a folder without a letter is not listed (UNPROVEN, not needed on this PC, which has one fixed volume).

`startScan` looks the drive up with the same function; there is no drive cache that can be stale.

### 6.2 The scan: states, one at a time, ids

State of one scan (records are in `ScanTable`):

```
            POST /scans                      walker returns Ok
(none) ---------------------> scanning -------------------------> done        (tree held in the arena slot)
                                 |  \ root unreadable / walker panic
                                 |   '----------------------------> failed
                                 '--- DELETE, new scan of another drive, game mode, backend stop ---> cancelled
done --- DELETE, new scan, 10 min after the last read ---> done (record kept, tree gone: `root` absent)
```

Rules:

1. One scan at a time in the whole process, not one per drive. Reasons: the UI shows only the newest scan; two scans fight for the same disk and double the memory; the mock does the same (`startScan` cancels every running scan).
2. `startScan(drive)`, in this order, under the table lock (a `std::sync::Mutex`, never held across an `.await`):
   - refuse in game mode (`409 gameMode`) and while shutting down (`503 shuttingDown`);
   - look the drive up (before taking the lock, because it is blocking): unknown id gives `404 unknownDrive`;
   - if the running scan is for the same drive, return its snapshot with status 200;
   - if another scan is running, end it as `cancelled`: set its cancel flag, set its record to `cancelled`, publish its final frame (section 6.2.1);
   - drop the old arena and its expiry task (so that the peak memory is one tree, not two);
   - create the record with id `scan-<UTC yyyymmdd-hhmmss>-<n>` (`n` is a counter of this process; the date part is cut from `now_iso()`), status `scanning`;
   - start the walker thread and the progress task (6.2.2, 6.3);
   - return the snapshot with status 201.
3. Ids are readable and do not repeat after a restart within a day. The frontend resets its disk state when `startId` changes anyway.
4. The last 8 records are kept in a `VecDeque`; older ones are removed. A call with a removed id is `404 unknownScan`.

#### 6.2.1 Ending a scan, exactly once

Only one function, `ScanTable::finish(id, status, ..)`, changes a record from `scanning` to a final status. It does this under the lock, and only if the status is still `scanning`. It also publishes the final frame (`bus.publish::<Scan>` is synchronous and takes microseconds, so it may run under the lock) and releases the activity counter (`state.activity`, which keeps `GET /ping` `busy` true while a scan runs; the call is made through a guard object, the exact method name is core's). Three callers: the finaliser task (done, failed), `cancel` (cancelled), `stop` of the module (cancelled, no frame, foundation 2.6). So:

- the final frame is published once;
- after the final frame no progress frame of that scan can follow, because the progress task publishes only while it sees `status == scanning` under the same lock;
- cancel is synchronous. It does not wait for the walker threads. The record becomes `cancelled` and the frame goes out at once; the walker threads see the flag and return within milliseconds (3.1 ms measured), or, if one is stuck in a slow `read_dir`, whenever it comes back. Its result is then dropped (the finaliser sees a final status and discards the arena). This is why the UI's hold on the window (`holdOpen('scan')`) can never leak because a thread hangs.

#### 6.2.2 Progress

- `progress = min(0.99, bytesSeen / drive.usedBytes)`; 0 when `usedBytes` is 0. Never lower than the previous value (both counters only grow). On `done` it is 1. Rounded to 3 digits.
- `filesSeen` is the file counter (files only, not folders; the UI says "184,203 files").
- `notAccessible` is the denied-folder counter, sent in every frame once it is above 0.
- The progress task (T5) loops `tokio::time::sleep(250 ms)` and, if the counters changed since the last frame, publishes. That is at most 4 frames per second without a `Throttle`. It ends when the scan ends. The bus drops the frame at no cost when no client is connected (foundation 5.3).
- The counters are updated by the walker once per directory (three `fetch_add` calls), not per file. So the counters move in small jumps at the speed of the walk; that is fine for a bar.

#### 6.2.3 What the arena slot holds

`ArenaSlot { scan_id, arena: Arc<Arena>, root_path: String, deadline: Mutex<tokio::time::Instant> }`, and one expiry task (T6): it sleeps until `deadline`; if the deadline moved meanwhile it sleeps again; when it fires it drops the slot. Every read of the tree (`getScan` with `done`, the node route) moves the deadline to now plus `SCAN_ARENA_TTL` (10 minutes). `DELETE` of a done scan, a new scan, and the module `stop` drop the slot and abort the task. At most one slot exists.

After the slot is gone: `GET /scans/{id}` still answers 200 with `status: done` and no `root` (the UI treats a `done` scan without `root` as "no result" and shows the drive list again, `DiskScreen.vue:42`); the node route answers `409 scanExpired`.

The client keeps its tree 5 minutes after the user leaves Disk (`diskCache.ts`) and now also tells the backend to free it at that moment (section 9, `resetDisk`). A user who stays on the Disk screen for more than 10 minutes without opening a folder that needs a fetch can meet `scanExpired`; the message says to scan again (section 8). This is accepted: the alternative is a timer on the client that touches the scan, which breaks the idle rule for little gain.

### 6.3 The walker

Copy `walk.rs` of the winapi spike and change what is listed here. The shape: one directory at a time; list it; skip reparse points; collect files; walk the sub-folders in parallel with `rayon` work stealing; a folder that cannot be read becomes a "denied" node.

Seam for tests (the spike calls `std::fs::read_dir` inline; here it goes through a trait so that denied folders, errors and slow folders can be faked without touching a disk or the ACLs):

```rust
// modules/disk/walk.rs
pub struct RawEntry { pub name: OsString, pub attrs: u32, pub len: u64 }   // attrs: Windows file attributes
impl RawEntry { fn is_dir(&self) -> bool { self.attrs & 0x10 != 0 } fn is_reparse(&self) -> bool { self.attrs & 0x400 != 0 } }
pub trait DirSource: Send + Sync {
    fn list(&self, dir: &Path) -> io::Result<Vec<RawEntry>>;
}
pub struct StdDirs;      // production
impl DirSource for StdDirs {
    fn list(&self, dir: &Path) -> io::Result<Vec<RawEntry>> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(dir)? {
            let Ok(e) = e else { continue };                  // an entry that vanished: not an error of the folder
            let Ok(md) = e.metadata() else { continue };      // on Windows this is free: the data of the find call
            out.push(RawEntry { name: e.file_name(), attrs: md.file_attributes(), len: md.len() });
        }
        Ok(out)
    }
}
```

The spike's `read_dir` loop already builds temporary vectors per directory, so the extra `Vec` costs nothing measurable. (Entries that fail to read are counted in `other_err` by the spike; here that counter is only logged.)

Counters (all `AtomicU64`, one struct shared by the threads): `files`, `dirs`, `bytes`, `denied` (folders not accessible), `reparse` (skipped), `other_err`.

The recursive function:

```rust
pub struct Ctx<'a> { pub src: &'a dyn DirSource, pub cancel: &'a AtomicBool, pub c: &'a Counters, pub keep: usize }
pub type CatBytes = [u64; 8];                        // bytes per FileCategory (index = category as usize)

pub fn walk_dir(path: &Path, name: &str, depth: u32, ctx: &Ctx) -> (DirNode, CatBytes) {
    let mut node = DirNode::new(name);
    let mut cats = [0u64; 8];
    if ctx.cancel.load(Relaxed) { return (node, cats); }            // checked at the top of every directory
    if depth >= SCAN_MAX_DEPTH {                                    // 256 levels: a loop the reparse check missed, or a pathological tree
        ctx.c.denied.fetch_add(1, Relaxed); node.denied = true; return (node, cats);
    }
    let entries = match ctx.src.list(path) {
        Ok(v) => v,
        Err(e) => {
            // Not accessible = the folder exists but Windows refused it (access denied) or it could not be read for any other
            // reason except "it is gone". A folder that vanished while we scanned (NotFound) is simply empty.
            if e.kind() != io::ErrorKind::NotFound { ctx.c.denied.fetch_add(1, Relaxed); node.denied = true; }
            else { ctx.c.other_err.fetch_add(1, Relaxed); }
            return (node, cats);
        }
    };
    let (mut files, mut subs): (Vec<FileNode>, Vec<(PathBuf, String)>) = (Vec::new(), Vec::new());
    for e in entries {
        if e.is_reparse() { ctx.c.reparse.fetch_add(1, Relaxed); continue; }        // never followed, never counted
        let n = e.name.to_string_lossy().into_owned();
        if e.is_dir() { subs.push((path.join(&e.name), n)); }
        else {
            cats[category_of(&n) as usize] += e.len;                                 // counts ALL files, also those not kept
            files.push(FileNode { name: n.into_boxed_str(), size: e.len });
        }
    }
    // counters, then keep the `keep` largest files and fold the rest, exactly as in the spike (other_files = (count, bytes))
    ...
    let results: Vec<(DirNode, CatBytes)> = subs.par_iter().map(|(p, n)| walk_dir(p, n, depth + 1, ctx)).collect();
    for (d, c) in results { node.size += d.size; for i in 0..8 { cats[i] += c[i]; } node.dirs.push(d); }
    node.dominant = dominant(&cats);                                // Option<FileCategory>, one byte in the node
    (node, cats)
}
```

Details that differ from the spike, and why:

| Change | Why |
|---|---|
| `DirNode` loses `file_count`; gains `dominant: u8` (255 = none) | The "other (N items)" label counts entries of the folder (section 6.5), so the subtree file count is not needed (8 bytes less per folder). The dominant category is needed to colour a truncated folder. |
| `walk_dir` returns the category bytes of the subtree next to the node | The array (64 bytes) lives on the stack of the recursion and is dropped after the parent merged it; the node keeps only one byte. |
| Depth cap 256 | rayon work stealing and recursion use stack. The pool gets 8 MiB stacks (virtual, committed on use) and the cap stops anything deeper (counted as not accessible). |
| `NotFound` is not "denied" | A folder that disappeared during a scan (a temp folder) is not a permission problem. |
| `denied` also covers errors other than access denied | An unreadable folder is unreadable for the user whatever the code; one number is enough. |
| Source behind `DirSource` | tests |

Reparse points (junctions, symbolic links, mount points, OneDrive and other cloud placeholders, app execution aliases): attribute `0x400` set means skipped. They are never followed, never counted as files and never as folders. This removes loops and double counting (`C:\Users\All Users`, `Documents and Settings`). Proven in the spike: a test junction was reported as `reparse_skipped=1` and not followed. Known limitation (UNPROVEN, to be looked at in the acceptance check): a OneDrive file that is kept on the device may still carry the placeholder reparse point; if so its size is missing from the total. The skipped count is logged at `info` at the end of a scan so this is visible. Following them is not an option: a cloud placeholder must never be opened (it would download it).

Rules for entries:

- No administrator rights. A folder Windows refuses (`System Volume Information`, other users' folders, parts of `WindowsApps`) is kept as an empty node with `denied`, and counted. The scan continues. This is not a failure.
- Files that are locked by another program (`pagefile.sys`, `hiberfil.sys`, `swapfile.sys`) still appear in the directory listing with their size, because the size comes from the listing and the file is never opened. They count.
- Names: `to_string_lossy`. Long paths: `std::fs` adds the `\\?\` prefix by itself for paths over 260 characters. The prefix never reaches the API, because the API paths are built from names, not from the OS path.
- The per-directory file cap `keep = 64` (`SCAN_KEEP_FILES_PER_DIR`): only the 64 largest files of a folder are kept as nodes; the others become `other_files = (count, bytes)`. It bounds memory in a folder with 100 000 files (spike: 64 B per kept file node).
- Hard links (for example `WinSxS`) are counted once per name, as Explorer does. The total can exceed what the volume really uses.

The scan thread (T5; one `std::thread::Builder::new().name("rain-scan-<id>")` per scan, not the blocking pool, foundation 11.6):

```rust
fn run_walk(src: Arc<dyn DirSource>, root: PathBuf, root_name: String, cancel: Arc<AtomicBool>, c: Arc<Counters>) -> Outcome {
    let pool = match rayon::ThreadPoolBuilder::new()
        .num_threads(SCAN_WALK_THREADS)                          // 8: measured best (1,2,4,8,16,32 threads = 2.11,1.19,0.68,0.41,0.56,0.56 s)
        .stack_size(8 << 20)
        .thread_name(|i| format!("rain-walk-{i}"))
        .start_handler(|_| platform::thread::set_below_normal())  // SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL)
        .build() { Ok(p) => p, Err(e) => return Outcome::Failed(...) };
    let ctx = Ctx { src: &*src, cancel: &cancel, c: &c, keep: SCAN_KEEP_FILES_PER_DIR };
    let (tree, _) = pool.install(|| walk_dir(&root, &root_name, 0, &ctx));
    drop(pool);                                                   // the 8 threads end here: nothing stays resident
    if cancel.load(Relaxed) { return Outcome::Cancelled; }
    if tree.denied { return Outcome::Failed(format!("Drive {root_name} could not be read")); }   // the root itself was refused
    Outcome::Done(flatten(tree))
}
```

The thread wraps `run_walk` in `catch_unwind` and sends the result (`Ok(outcome)` or the panic) over a `tokio::sync::oneshot` to the finaliser task (spawned with `core::task::spawn_named`). A panic becomes `failed` with the text "The scan stopped unexpectedly" and is logged at `error`.

Below-normal priority is UNPROVEN (the spike did not try it). It is on by default and has a constant `SCAN_BELOW_NORMAL = true`. Acceptance check 3 measures the scan time with and without; if it is more than 20% slower with it, set the constant to false. `THREAD_MODE_BACKGROUND_BEGIN` (which also lowers the I/O priority) is NOT used: it can make a scan many times slower and was never measured.

### 6.4 Memory and time

| Item | Value | Source |
|---|---|---|
| Memory of the arena | about 64 bytes per kept file, about 150 bytes per folder (names included); 60 to 200 MB for a drive with 1 000 000 files (the larger figure when there are very many tiny folders) | spike winapi: 1M files and 50k folders synthetic 61 MB, keep 12: 49 MB; C:\Windows keep 64 peak 47 MB; worst case with tiny folders about 200 MB |
| Memory while walking | the arena plus the tree being built (the tree is moved into the arena, no copy); the rayon pool stacks are virtual | spike |
| Memory after drop | working set back to about 11 MB in the spike | spike |
| Time, warm cache | 410 000 entries per second with 8 threads; a million files in 2 to 3 s | spike, C:\Windows: 195 236 files and 49 954 folders in 0.41 s with 8 threads |
| Time, cold cache, with antivirus | UNPROVEN. Expect 10 to 60 s for a million files. Nothing breaks if it takes longer: the scan is asynchronous (D13) and shows progress. | extrapolation |
| JSON of the budgeted tree | about 350 KB (2000 nodes with a path each). The 221 KB in foundation appendix A is without the `path` field; measured in stage 3; the test limit is 700 KB | estimate, test in section 10 |
| Cancel latency | 3.1 ms after the flag in the spike (the flag is checked at the top of each folder) | spike |

The idle budget (30 MB) does not apply while a scan runs or while its tree is held (T6, at most 10 minutes after the last read). After that the backend is back under the budget (acceptance check 2).

### 6.5 The arena and the budgeted tree

`flatten(tree) -> Arena` (copy of the spike): all folders go into one `Vec<DirRec>` indexed by `u32` (root is 0); children are `Vec<u32>`.

```rust
pub struct DirRec { pub name: Box<str>, pub size: u64, pub files: Vec<FileNode>, pub other_files: (u64, u64),
                    pub children: Vec<u32>, pub denied: bool, pub dominant: u8 }
pub struct Arena { pub dirs: Vec<DirRec> }
```

`cut(arena, root_id, root_path, budget, first_level, per_folder) -> DiskNode` (copy of `budget_cut` of the spike; changes below):

1. Start with the root folder as one collapsed node in a max-heap ordered by size.
2. Pop the largest collapsed folder. Its candidates are its sub-folders and its kept files. Sort them by size, largest first, ties by name (so the output is the same every time).
3. Per folder limit: `first_level = 200` (`SCAN_OPEN_FOLDER_CHILDREN`) for the folder the request is about, `per_folder = 12` (`SCAN_PER_FOLDER`) for every folder below it.
4. Everything past the limit, plus the folder's `other_files`, is folded into one `other` node of kind `Other`: `name = "other (N items)"` (digits grouped with commas, "1 item" when N is 1), `path = "<folder path>\*"`, `sizeBytes` = the sum, no children. `N` = the number of entries of this folder that are not listed one by one (folded folders and files, plus `other_files.0`). One detail the spike did not have: if exactly one candidate is past the limit and `other_files.0 == 0`, it is listed instead of folded, because a node "other (1 item)" costs the same and says less.
5. The expansion costs `candidates kept + 1 if an other node exists` nodes. If the node count would then pass the budget (`SCAN_TREE_BUDGET = 2000`), the folder stays collapsed and the next one is tried. The expanded folder's children are pushed into the heap as collapsed folders.
6. Build the `DiskNode` tree. For each folder:
   - expanded: `children` = its nodes (never `[]`);
   - collapsed with content: no `children`, `truncated: true`, `category` = `dominant` of the arena record (the dominant category by bytes of its whole subtree);
   - denied: `denied: true`, `sizeBytes: 0`;
   - empty: nothing of the above.
   Files: `kind: file`, `category` from the extension (6.6). Folders that are expanded get no `category`.
7. The sums: an expanded folder's `sizeBytes` is its record's `size`, and its children plus `other` add up to it exactly (the spike measured 0 violations on C:\Windows; a property test repeats this, section 10).

Cost of one cut (spike): 1.8 ms for 2000 nodes, a lazy subtree 5 microseconds. The handlers still run the cut inside `core::blocking::run` because path lookup and sorting in a folder with 100 000 entries can take longer (rule: more than about 5 ms goes to the blocking pool).

The two requests that use it:

| Request | `cut` arguments |
|---|---|
| `GET /scans/{id}` with `done` | root id 0, `root_path` = the drive root, budget 2000, first level 200, per folder 12 |
| `GET /scans/{id}/node?path=P` | the id found for `P`, `root_path` = the canonical path built from the arena (not the caller's string), the same numbers |

Resolving `P` to an arena id: take the root path of the scan; compare `P` with it case-insensitively (ASCII) after replacing `/` by `\` and removing a trailing `\`; if `P` is not the root and not below it, `404 unknownPath`; split the rest on `\` and descend by exact name match among `children`; if an exact match fails, one case-insensitive match is tried; no match gives `404 unknownPath`. A path that names a file, an `other` node (ends with `\*`) or a folder that is not in the arena gives `400 notADirectory` / `404 unknownPath`. The cost is the number of children per level, which is at most a few thousand.

Why a request for the open folder returns up to 200 children and the deeper levels only 12: the list on the left shows the open folder's children, so that folder deserves detail; the treemap draws at most four levels below it and 12 children per folder fill a 864x640 canvas. The frontend fetches a folder again when it is opened and its children are partial (section 9, `needsLoad`).

### 6.6 File categories by extension

`modules/disk/category.rs`. One function, `category_of(file_name: &str) -> FileCategory`.

Rule: take the text after the last `.` of the file name, only if something precedes that dot (so `.gitignore` and `Makefile` have no extension and are `other`); compare in ASCII lower case; an extension longer than 8 bytes is `other`. The extension list is one `match`:

| Category | Extensions |
|---|---|
| video | mp4 mkv avi mov wmv webm m4v flv mpg mpeg m2ts mts bik |
| image | jpg jpeg png gif bmp webp heic heif tif tiff svg ico raw cr2 nef arw dng psd xcf |
| audio | mp3 wav flac aac ogg opus m4a wma aiff mid |
| archive | zip 7z rar tar gz tgz bz2 xz zst iso img vhd vhdx vmdk cab wim pak |
| code | rs ts tsx js jsx mjs cjs vue py go c h cpp hpp cs java kt swift rb php sh ps1 bat cmd json toml yaml yml xml html css scss sql lock |
| document | pdf doc docx xls xlsx ppt pptx odt ods txt md rtf csv epub |
| app | exe dll msi sys msix appx lib pdb obj |
| other | everything else |

Decision: `ts` is `code`, not `video`. The research list had it in both. MPEG transport streams use `.m2ts` and `.mts`, and this is a developer's PC. Folders carry no category except a collapsed one (dominant by bytes, 6.3). The `other` node carries none (it is drawn in the "other" colour by the frontend's default).

### 6.7 Sizes

Logical size (what Explorer shows as "Size" and what the UI means). It comes free with the directory listing. Size on disk would cost a call per file (27 microseconds, about 27 s per million files single-threaded, spike) and was not chosen. Consequences, all accepted and none an error: the scan total differs from `Drive.usedBytes` (metadata files, `System Volume Information`, folders that were not accessible, hard links counted per name, compression and sparse files, skipped placeholders). `root.sizeBytes` is what was measured; it is never forced to equal `usedBytes`. The UI does not show the difference (section 9).

### 6.8 What the Disk screen shows for "not accessible"

- During the scan: nothing (the bar and the file count only).
- Result view, root open: in the strip, left of the total size, a lock icon with the folder count and a tooltip: "N folders that Windows did not let Rain read. Their size is not in the total." The tooltip is the only sentence; it is shown only when `scan.notAccessible > 0`.
- A folder row that is `denied`: a `lock` icon instead of the folder icon, and "—" instead of the size.
- Opening a `denied` folder: the empty state "Not accessible" with the `lock` icon.
- Not shown: the difference to `usedBytes`, the names of the folders, a list. Reason: the user's direction is minimal and icon-led; the count answers "why does it not add up".

### 6.9 Paths: reveal and check

One shared function normalises what a caller typed; it is pure and unit tested. It lives in `core/paths.rs` (foundation 11.1), because the settings module builds `Resolved` with the same function (section 06 6.1.7) and there must be no second normaliser; the disk module only maps its `PathProblem` to the reasons of this section.

```rust
// core/paths.rs (called as core::paths::normalize by disk and settings)
pub enum PathProblem { Empty, TooLong, BadChar, NotAbsolute, Dotdot, NotLocal }
pub fn normalize(raw: &str, allow_unc: bool) -> Result<PathBuf, PathProblem>;
```

Rules, in this order:

0. For `POST /paths/check` only: call `settings::Service::expand_env(raw)` (the public `Service` method of section 06 6.1.7 that the settings module itself uses) first, so that a value such as `%USERPROFILE%\Games` shows the same glyph that the backend result gives; the rest of the rules then run on its result. `revealPath` does not call it: its input is an absolute path taken from a row.
1. Trim whitespace; remove one pair of surrounding double quotes. Empty gives `Empty`.
2. More than 1024 characters gives `TooLong` (`PATH_MAX_CHARS`).
3. Any control character or any of `< > " | * ?` gives `BadChar`. (`*` and `?` also stop the synthetic `other` path `...\*` from ever being revealed.)
4. Replace `/` with `\`.
5. A path that starts with `\\?\` or `\\.\` gives `NotLocal`. A path that starts with `\\` (UNC) gives `NotLocal` unless `allow_unc` (the path check and the settings module pass `allow_unc = true`; reveal passes `false`).
6. Otherwise it must start with a letter, a colon and a backslash (`C:\`). Anything else (`C:foo`, `C:`, `foo\bar`) gives `NotAbsolute`.
7. Split on `\`; drop empty segments and `.`; a `..` segment gives `Dotdot` (not resolved: a typed `..` is a mistake or an attack, and resolving needs the file system).
8. Join; upper-case the drive letter; remove a trailing backslash except for the root `X:\`.

`~` is never expanded. An environment variable is expanded only in step 0, for the path check. For `revealPath` nothing is expanded (the path comes from a row of this module's own scan). The settings module builds `Resolved` with this same function (`allow_unc = true`, section 06 6.1.7), so an entry that gives `exists: false` here because a rule failed (quotes are stripped by rule 1; `C:foo`, `C:\a\..\b` and `\\?\C:\x` fail) is also not in `Resolved`, and the glyph and the value that projects and games use agree.

The probe (shared by reveal and check): is the thing there, and is it a folder?

```rust
pub enum Probe { Missing, Dir, File, Slow }
async fn probe(&self, path: PathBuf, follow_links: bool) -> Probe
```

- It runs `std::fs::metadata` (check; follows links, so a broken link is `Missing`) or `symlink_metadata` (reveal; a link itself can be shown) on the blocking pool.
- A semaphore of 4 (`PATH_PROBE_PARALLEL`) limits the probes; the permit is moved INTO the blocking closure, so a stuck probe keeps its permit until the thread returns. The whole probe (permit wait plus call) has a timeout of 2 s (`PATH_PROBE_TIMEOUT`); on timeout the answer is `Slow` and a `warn` is logged (without the path). This protects the 8 blocking threads from a dead network drive or a sleeping disk.
- `PermissionDenied` is `Missing` for the check (Rain cannot use the folder either) and counts as present for reveal (Explorer shows the item selected in its parent).
- The probe is a trait (`PathProbe`) so tests can fake a slow answer.

`POST /paths/check`:

- `paths` has at most 32 entries (`PATHS_CHECK_MAX`), else `400 invalid` `tooMany`; an entry of more than 1024 characters (`PATH_MAX_CHARS`) is `400 invalid` `tooLong`; a body that does not parse is `badBody`. Each entry is normalised with `allow_unc = true`; any other normalisation problem is NOT an error: that entry answers `exists: false`. These limits are the single definition; section 06 relies on them.
- The entries are probed in parallel (bounded by the semaphore). The answer has the same order and length as the request; `path` echoes the string that was sent.
- A path is never logged (it may be under the user's profile). The handler logs the count at `debug`.
- UNC paths are probed with the same 2 s timeout; a share that does not answer is `exists: false`.
- Which paths may be checked: any string. Reading existence is what any process of this user can do already.

`POST /shell/reveal`:

1. `normalize(path, allow_unc = false)`. `Empty`, `TooLong`, `BadChar`, `NotAbsolute`, `Dotdot` give `400 invalid` reason `badPath`; `NotLocal` gives `400 invalid` reason `networkPath`.
2. Probe with `symlink_metadata`. `Missing` gives `404 notFound` `pathMissing`; `Slow` gives `503 unavailable` `pathSlow`. Reason for the probe: `SHOpenFolderAndSelectItems` on a missing item opens the parent or fails in an unclear way; the user must hear "no longer exists".
3. `platform::com::run(move || platform::shell::reveal(&path))` with a 10 s timeout (foundation 11.6): `SHParseDisplayName(path)`, then `SHOpenFolderAndSelectItems(pidl, None, 0)`, then `CoTaskMemFree(pidl)`. Copy `reveal()` of `process/src/bin/q7.rs`; the spike measured 218 ms and verified the selection. The route answers when the shell call returns, not when Explorer has drawn the window. A COM failure is `500 failed` `revealFailed`; a COM timeout is `503 unavailable` `comBusy` (foundation 11.6 says later COM calls then also fail until restart).
4. 204.

What is refused and why:

| Refused | Reason |
|---|---|
| UNC and device paths (`\\server\share`, `\\?\`, `\\.\`) in reveal | A network path that does not answer can block the single COM thread and with it every icon and launch call. Rain is a launcher for this PC. The check route accepts UNC (a plain file call with a timeout) and shows "Missing" if it does not answer. |
| Relative paths, `C:foo`, `..` segments | Their meaning depends on a current folder that the backend does not have. |
| `* ? < > \| "` and control characters | Not valid in a Windows name; a wildcard would be the `other` node, which is not a file. |
| A path that does not exist | `404 pathMissing`. |
| Anything that is not a path of a file or folder | The route never starts a program. `SHOpenFolderAndSelectItems` only selects; it does not open a file. |

Everything else is allowed: any existing local file or folder, in or outside the user profile, including system folders. Reveal does not read, change or delete anything.

The shell call and the foreground window: whether the Explorer window comes to the front when the call is made from the windowless backend is UNPROVEN (the spike checked the selection only). Acceptance check 6 looks at it; the fallback is in open point 5.

### 6.10 Machine state and idle time

```rust
// modules/machine/service.rs
pub trait IdleSource: Send + Sync { fn idle_ms(&self) -> Option<u64>; }   // production: platform::idle
pub struct Service { src: Arc<dyn IdleSource> }
impl Service {
    /// Milliseconds since the last keyboard or mouse input of this session. None when Windows refuses.
    pub fn idle_ms(&self) -> Option<u64> { self.src.idle_ms() }
    /// What GET /machine returns.
    pub fn state(&self) -> Option<MachineState> {
        let ms = self.idle_ms()?;
        Some(MachineState { user_active: ms < USER_ACTIVE_BELOW_MS, idle_minutes: (ms / 60_000) as u32 })
    }
}
```

`platform/idle.rs` (copy `idle_time()` of `winapi/src/bin/sysinfo.rs`):

```rust
pub fn idle_millis() -> Option<u64> {
    unsafe {
        let mut li = LASTINPUTINFO { cbSize: size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        if !GetLastInputInfo(&mut li).as_bool() { return None; }
        let now = GetTickCount64() as u32;                    // 32 bits on purpose: dwTime is a 32-bit tick count
        Some(now.wrapping_sub(li.dwTime) as u64)              // the wrap after 49.7 days of uptime is handled by wrapping_sub
    }
}
```

Facts and decisions:

- It works from a windowless process in the user's session (spike: 0 to 562 ms while the user was active). The backend is a per-user process in the interactive session (not a service), which is required: a service in session 0 would read nothing useful.
- `userActive` is true when the idle time is under 60 s (`USER_ACTIVE_BELOW_MS = 60_000`). `idleMinutes` is the idle time in whole minutes. So `userActive == false` implies `idleMinutes >= 1`.
- The per-job thresholds (`idleAfterMinutes`, 5 to 30 minutes) belong to the idle jobs (section 03). `MachineState` is a plain fact and does not know them. Section 03 uses `idle_ms()` only and compares it with each job's own threshold. The status line in `IdleView.vue` therefore reads "PC idle for N min" without "Idle jobs can run" (open point 4).
- Cost: one call, microseconds. No timer, no thread, no cache. The 30 s sampling of idle jobs (T3) belongs to section 03 and calls `machine::Service::idle_ms()`; this module has no timer of its own.
- When the user looks at Rain, the page is the input, so `GET /machine` practically always answers "active" while the UI shows it. The route is for the idle module and for a future widget; the acceptance check uses a script that does not touch the keyboard (section 11).
- Failure: if Windows refuses the call, `GET /machine` answers `503 unavailable` `idleUnavailable`, and `idle_ms()` returns `None`, which the idle module must read as "not idle" (safe).
- No CPU, GPU or memory readings are built: no screen shows them (the contract has no field) and the unproven `GetSystemTimes` and performance-counter calls would only be code nobody uses. Section 03 may ask for them (open point 4).

### 6.11 Numbers (new constants; open point 2 asks for them in appendix A of the foundation)

| Name | Value | Used in |
|---|---|---|
| `SCAN_TREE_BUDGET` (exists) | 2000 nodes | 6.5 |
| `SCAN_PER_FOLDER` (exists) | 12 children per folder below the first level | 6.5 |
| `SCAN_ARENA_TTL` (exists) | 10 min | 6.2.3 |
| `SCAN_WALK_THREADS` (exists) | 8 | 6.3 |
| `SCAN_OPEN_FOLDER_CHILDREN` | 200 | 6.5 |
| `SCAN_KEEP_FILES_PER_DIR` | 64 | 6.3 |
| `SCAN_MAX_DEPTH` | 256 | 6.3 |
| `SCAN_PROGRESS_TICK` | 250 ms | 6.2.2 |
| `SCAN_RECORDS_KEPT` | 8 | 6.2 |
| `SCAN_BELOW_NORMAL` | true | 6.3 |
| `PATH_PROBE_TIMEOUT` | 2 s | 6.9 |
| `PATH_PROBE_PARALLEL` | 4 | 6.9 |
| `PATHS_CHECK_MAX` | 32 | 6.9 |
| `PATH_MAX_CHARS` | 1024 | 6.9 |
| `USER_ACTIVE_BELOW_MS` | 60 000 | 6.10 |

---------------------------------------------------------------------------------------------------

## 7. Background work

| Task | Trigger | Timer row of 11.5 | Cost | In game mode | How it stops |
|---|---|---|---|---|---|
| Scan coordinator thread `rain-scan-<id>` and the rayon pool of 8 threads `rain-walk-<i>` | event: `POST /scans` | none (a thread, not a timer) | CPU on 8 threads at below-normal priority for seconds to minutes; memory 60 to 200 MB at the end of a big scan | refused (`409 gameMode`); a running scan is cancelled by game mode through `disk::Service::cancel_running()` (section 06 calls it) | the cancel flag (checked at the top of each folder); the pool is dropped when the walk returns, so no thread stays |
| Progress task (one per scan) | event: scan running; wakes every 250 ms while it runs | T5 | one wake-up and at most one frame per 250 ms; zero when no client is connected (the bus checks receivers) | ends with the scan | ends with the final status, or aborted by `finish` |
| Finaliser task (one per scan) | event: the thread's result arrives on a oneshot | none (awaits an event) | negligible | n/a | ends after it ran |
| Arena expiry task (at most one) | started when a scan ends `done`; sleeps until a moving deadline | T6 | one sleep of at most 10 minutes, a re-check if the deadline moved | not started in game mode (no scan runs) | fires and drops the arena; aborted by `DELETE`, a new scan, or module `stop` |
| Path probes | event: a request | T4 (the 2 s timeout of one call) | one blocking call, at most 4 at once | still served (read only) | the timeout; a stuck call ends when the OS returns |
| COM call for reveal | event: a request | T4 (the 10 s timeout) | about 220 ms | still served | the timeout |
| Drives, machine state | event: a request | none | microseconds | still served | none |

Nothing runs in the background without a request. In particular: no polling of drives, no polling of idle time, no watcher. `Service::stop()` of `disk`: set the cancel flag of a running scan, drop the arena and the expiry task, publish nothing (foundation 2.6 step 3). Threads started by this area, for the review checklist of 11.5: `rain-scan-<id>` (one per scan, ends with it) and the 8 `rain-walk-<i>` (built per scan, dropped with it). `tokio::time` uses: the 250 ms `sleep` loop (T5), the arena expiry `sleep_until` (T6), the two `timeout` calls (T4).

---------------------------------------------------------------------------------------------------

## 8. Errors

Body: the one error body of foundation 4.5. "UI" says what the Disk or Settings screen does with it today.

| Situation | Status | `code` | `reason` | `message` | UI |
|---|---|---|---|---|---|
| Body of `POST /scans` is not valid (missing or extra field) | 400 | `invalid` | `badBody` | "The request was not valid: ..." (foundation 4.5) | `runAction` shows the notice |
| Unknown drive id | 404 | `notFound` | `unknownDrive` | "Drive not found" | notice (same text as the mock) |
| Scan requested in game mode | 409 | `conflict` | `gameMode` | "Game mode is active. Scans start again after it." | notice |
| Scan requested while the backend shuts down | 503 | `unavailable` | `shuttingDown` | "Rain is stopping" | notice |
| The drive's root cannot be read (denied, offline), or the walker failed | (scan ends) | n/a | n/a | `DiskScan.error` = "Drive C: could not be read" | `ErrorRow` under the drive with Retry (existing) |
| Walker thread panicked | (scan ends) | n/a | n/a | `DiskScan.error` = "The scan stopped unexpectedly" | same |
| Unknown scan id (get, node, delete) | 404 | `notFound` | `unknownScan` | "Scan not found" | notice. `cancelScan` of an unknown id stays a 404 because the UI built on the mock does the same; the frontend's fire-and-forget cleanup ignores it |
| Node requested while the scan is not `done` | 409 | `conflict` | `scanNotDone` | "The scan is not finished yet" | notice (cannot happen from the UI: a folder is opened only from a result) |
| Node requested after the tree was freed (10 minutes, new scan, delete) | 409 | `conflict` | `scanExpired` | "This scan is too old. Scan the drive again." | `open-folder` fails; `runAction` shows the notice; Ctrl+R rescans |
| Node path not in this scan | 404 | `notFound` | `unknownPath` | "That folder is not in this scan" | notice |
| Node path is a file or an `other` node | 400 | `invalid` | `notADirectory` | "That is not a folder" | notice |
| Node path missing (no `path` query) | 400 | `invalid` | `badQuery` | foundation text | notice |
| Node URL longer than 2048 bytes | 400 | `invalid` | `urlTooLong` | "The path is too long for this request" | notice |
| Reveal: empty, too long, bad character, relative, `..` | 400 | `invalid` | `badPath` | "That path cannot be shown in Explorer" | notice (`revealAction` already shows the message) |
| Reveal: network or device path | 400 | `invalid` | `networkPath` | "Rain does not open network locations in Explorer" | notice |
| Reveal: the file or folder is gone | 404 | `notFound` | `pathMissing` | "That file or folder no longer exists" | notice |
| Reveal: the location did not answer in 2 s | 503 | `unavailable` | `pathSlow` | "That location did not answer" | notice |
| Reveal: COM thread busy or timed out | 503 | `unavailable` | `comBusy` | "Windows did not answer. Try again." | notice |
| Reveal: the shell call failed | 500 | `failed` | `revealFailed` | "Explorer could not show that item" | notice |
| Check: more than 32 paths | 400 | `invalid` | `tooMany` | "Too many paths to check at once" | not reachable from Settings (at most 5 per row) |
| Check: a path of more than 1024 characters | 400 | `invalid` | `tooLong` | "That path is too long" | the glyph stays unknown |
| Machine: Windows refused `GetLastInputInfo` | 503 | `unavailable` | `idleUnavailable` | "Could not read the idle time" | `IdleView` shows "Machine state unavailable" (existing text) |
| Drives: a drive call hangs | 503 | `unavailable` | `timeout` | foundation text | `ErrorRow` "Drives unavailable" with Retry (existing) |
| Backend not running | 503 | `unavailable` | `backendDown` | shell's text (foundation 6.6) | the same rows |

Degraded sources in `GET /health` `sources`: none. A drive that is missing or a folder that is denied is data, not a source failure; the machine call fails per request. `disk` and `machine` report nothing to `state.health`.

Error messages never contain file names or note text. A reveal or node message does not repeat the path (the caller knows it).

---------------------------------------------------------------------------------------------------

## 9. Frontend changes

All line numbers are those of the repository at commit `e4c7f70` (the prototype). The foundation's stage 1b changes some of these files first (new `rest/` folder, router); lines in files that stage 1b creates are described by function.

### 9.1 Contract-change table (foundation 15.1)

| # | Item | From | To | Reason | Frontend files touched (full paths) | Breaking for the mock? | Stage |
|---|---|---|---|---|---|---|---|
| S.1 | `DiskNode` | `kind: 'dir' \| 'file'`; `children?` | `kind: 'dir' \| 'file' \| 'other'`; adds `truncated?`, `denied?` | budgeted tree (C11), unreadable folders | `src/data/types.ts` (delete lines 76 to 80 and 65 after the generated types exist), `src/features/disk/FolderList.vue` (lines 42 to 45, 63 to 65), `src/features/disk/DiskScreen.vue` (lines 78 to 82, 108 to 111, 159, 173 to 178), `src/features/disk/actions.ts` (the `diskNodeActions` provider, lines 116 to 141, and the new `loadFolder`), `src/features/disk/folders.ts` (new `needsLoad`, `replaceNode`) | No: the mock tree has no `other`, `truncated` or `denied`; they are optional or a wider union | 3 |
| S.2 | `DiskScan` | `root?` in every `done` snapshot and in the topic; no count | `root?` only in the answer of `getScan`; adds `notAccessible?` | frame size rule (section 2.1); the count of section 6.8 | `src/features/disk/actions.ts` (`scanDrive`, lines 38 to 65), new `src/features/disk/scanState.ts`, new `src/features/disk/NotAccessible.vue`, `src/features/disk/DiskScreen.vue` (strip, line 154) | No (the mock still sends `root` in frames; the new `scanDrive` handles both) | 3 |
| S.3 | `RainApi.getScanNode(scanId, path)` | none | `GET /scans/{id}/node?path=` returns a `DiskNode` | the lazy subtree (C11) | `src/data/api.ts` (disk block, lines 76 to 81, and the import line 6), `src/data/rest/client.ts`, `src/data/routing.ts`, `src/data/mock/slices/disk.ts` | Yes: the method is a `RetiredMockMethod` of `MockApi` (9.8) and the disk slice is deleted | 3 |
| S.4 | `RainApi.checkPaths(paths)` and type `PathCheck` | `mockControl.pathExists(path)` (synchronous, mock only) | `checkPaths(paths: string[]): Promise<PathCheck[]>` | a real check is asynchronous and reads the file system | `src/data/api.ts`, `src/data/rest/client.ts`, `src/data/routing.ts`, `src/features/settings/SettingRow.vue` (lines 16, 59 to 64, 113 to 118), new `src/features/settings/usePathCheck.ts`, `src/data/mock/control.ts` (lines 28, 61 to 64 and the import at line 9), `src/data/mock/slices/disk.ts` | Yes: the mock `pathExists` is deleted; `checkPaths` is a `RetiredMockMethod` (9.8) | 3 |
| S.5 | topic `scan` payload | a whole `DiskScan`, tree included when done | a `DiskScan` without `root` | frame limit 256 KiB | `src/features/disk/actions.ts` | No | 3 |
| S.6 | `Drive.label` | possibly empty on a real PC | never empty; "Local Disk" when Windows has none | row title "C: " | none (`DriveList.vue:40` is unchanged) | No | 3 |
| S.7 | `revealPath` | resolves for any string | rejects with `invalid` / `notFound` / `unavailable` for the cases of section 8 | a real reveal can fail | none (`src/actions/common.ts:57` already shows the message) | No | 3 |
| S.8 | `cancelScan` | cancels a running scan; no-op otherwise | also frees the tree of a `done` scan | idle memory (T6 is the safety net) | `src/features/disk/actions.ts` (`resetDisk`) | No | 3 |
| S.9 | `MachineState` | no rule for `userActive` | `userActive` = idle under 60 s; `idleMinutes` whole minutes | the real value needs a definition | none | No | 3 |

### 9.2 `src/data/api.ts`

Line 6 (import): add `DiskNode`, `PathCheck` to the list of imported types. Replace the disk block (lines 76 to 81) with:

```ts
  // disk
  listDrives(): Promise<Drive[]>                                               // GET  /drives
  startScan(driveId: ID): Promise<DiskScan>                                    // POST /scans            body { driveId }
  getScan(id: ID): Promise<DiskScan>                                           // GET  /scans/:id        `root` only when status is 'done'
  getScanNode(scanId: ID, path: string): Promise<DiskNode>                     // GET  /scans/:id/node?path=
  cancelScan(id: ID): Promise<void>                                            // DELETE /scans/:id      cancels a running scan, frees a finished one
  revealPath(path: string): Promise<void>                                      // POST /shell/reveal     body { path }
  checkPaths(paths: string[]): Promise<PathCheck[]>                            // POST /paths/check      body { paths }
```

Line 67 stays (`getMachineState`, `GET /machine`).

### 9.3 `src/data/rest/client.ts` (created in stage 1b; add these lines)

```ts
listDrives: () => call<Drive[]>('GET', '/drives'),
startScan: (driveId) => call<DiskScan>('POST', '/scans', { driveId } satisfies StartScanBody, { media: false }),
getScan: (id) => call<DiskScan>('GET', `/scans/${enc(id)}`, undefined, { media: false }),
getScanNode: (id, path) => call<DiskNode>('GET', `/scans/${enc(id)}/node${qs({ path })}`, undefined, { media: false }),
cancelScan: (id) => call<void>('DELETE', `/scans/${enc(id)}`),
revealPath: (path) => call<void>('POST', '/shell/reveal', { path } satisfies RevealBody),
checkPaths: (paths) => call<PathCheck[]>('POST', '/paths/check', { paths } satisfies PathsCheckBody),
getMachineState: () => call<MachineState>('GET', '/machine'),
```

`{ media: false }` on the scan calls: the answers contain no media and can be large (foundation 8.3). The stream client must not run `resolveMedia` on topic `scan` (foundation 8.7 already says so).

### 9.4 `src/data/routing.ts` (the only file a stage edits to move a method)

Set to `'real'`: `listDrives`, `startScan`, `getScan`, `getScanNode`, `cancelScan`, `revealPath`, `checkPaths`, `getMachineState`; topic `scan`. Keep the rest as it was. The mock keeps its copy of `getMachineState` (in `mock/slices/jobs.ts`, line 108, deleted with that slice in stage 6) and the router never calls it. Section 03 retires `getMachineState` by adding it to `RetiredMockMethod` at stage 6 (03 9.3), when it deletes that slice.

### 9.5 `src/features/disk/`

New file `scanState.ts` (pure, unit tested):

```ts
// Which snapshot of a scan to keep when several arrive in any order (the topic frame, the answer
// of getScan, a reconnect). A final state beats 'scanning'; of two 'scanning' ones the newer wins.
import type { DiskScan } from '@/data/types'

export function newer(current: DiskScan | null, incoming: DiskScan): DiskScan {
  if (!current || current.id !== incoming.id) return incoming
  if (current.status !== 'scanning') return current           // already final: nothing replaces it
  if (incoming.status === 'scanning' && incoming.filesSeen < current.filesSeen) return current
  return incoming
}
```

`folders.ts` (add at the end; the file stays free of d3):

```ts
/** True when opening `node` needs a fetch: its children are missing, or only the largest are there. */
export function needsLoad(node: DiskNode, loaded: ReadonlySet<string>): boolean {
  if (node.kind !== 'dir' || node.denied || loaded.has(node.path)) return false
  return node.truncated === true || (node.children?.some((child) => child.kind === 'other') ?? false)
}

/** A copy of `root` in which the folder with the path of `replacement` is replaced by it. Shares everything else. */
export function replaceNode(root: DiskNode, replacement: DiskNode): DiskNode {
  if (root.path === replacement.path) return replacement
  if (!root.children || !isInside(replacement.path, root.path)) return root
  const index = root.children.findIndex((child) => child.kind === 'dir' && isInside(replacement.path, child.path))
  if (index < 0) return root
  const children = root.children.slice()
  children[index] = replaceNode(children[index], replacement)
  return { ...root, children }
}
```

`actions.ts` (header comment unchanged). Replace lines 13 to 18 (`diskView`) with:

```ts
export const diskView = {
  /** The newest scan, kept current from the 'scan' topic while it runs. null = no scan yet. */
  scan: shallowRef<DiskScan | null>(null),
  /** The folder shown in the result view. null = the drive's root. */
  folderPath: ref<string | null>(null),
  /** Folders of this scan that were fetched whole with getScanNode (so they are not fetched again). Not reactive on purpose. */
  loaded: new Set<string>(),
}
```

Replace `resetDisk` (lines 26 to 31):

```ts
/** Forgets the scan and the open folder, and tells the backend to free the tree. For a reseed, a restart and the 5 minute drop. */
export function resetDisk(): void {
  stopFollowing?.()
  const scan = diskView.scan.value
  if (scan) void api.cancelScan(scan.id).catch(() => {})   // frees a finished tree; a running scan is cancelled; an unknown id is ignored
  diskView.scan.value = null
  diskView.folderPath.value = null
  diskView.loaded.clear()
}
```

(`import { api } from '@/data'` is added at the top. `actions.ts` is imported at start-up by the registry; `@/data` is already loaded by then.)

Replace `scanDrive` (lines 33 to 65). Same contract as before (rejects if the scan cannot start; a scan that fails later ends as `failed` in `diskView.scan`), plus the replay rule of section 2.1:

```ts
let refresh: (() => Promise<void>) | null = null

export async function scanDrive(driveId: Drive['id'], ctx: ActionContext): Promise<void> {
  if (starting) return
  starting = true
  let started: DiskScan
  try {
    started = await ctx.api.startScan(driveId)
  } finally {
    starting = false
  }
  // Only now that the scan exists is the old result replaced.
  stopFollowing?.()
  diskView.folderPath.value = null
  diskView.loaded.clear()
  diskView.scan.value = started
  if (started.status !== 'scanning') return

  const release = ctx.shell.holdOpen('scan')
  let finishing = false
  function end() {
    unsubscribe()
    release()
    stopFollowing = null
    refresh = null
  }
  /** One place decides what a snapshot means (frame, getScan answer, reconnect). */
  async function apply(incoming: DiskScan): Promise<void> {
    if (finishing || incoming.id !== started.id) return
    const merged = newer(diskView.scan.value, incoming)
    if (merged.status === 'scanning') {
      diskView.scan.value = merged
      return
    }
    finishing = true
    try {
      diskView.scan.value = merged.status === 'done' ? await withRoot(merged, ctx) : merged
    } catch (reason) {
      diskView.scan.value = { ...merged, status: 'failed', error: errorMessage(reason, 'Could not load the result') }
    } finally {
      end()
    }
  }
  const unsubscribe = ctx.api.subscribe('scan', (frame) => void apply(frame))
  stopFollowing = end
  refresh = async () => apply(await ctx.api.getScan(started.id))
  void refresh().catch(() => {})   // the scan may have ended before the subscription existed
}

/** A done frame carries no tree: fetch it. The hold on the window is released only after this. */
async function withRoot(done: DiskScan, ctx: ActionContext): Promise<DiskScan> {
  const full = done.root ? done : await ctx.api.getScan(done.id)
  if (!full.root) return { ...full, status: 'failed', error: 'The result is no longer available' }
  diskView.loaded.add(full.root.path)
  return full
}

/** After a stream reconnect that was not a restart (main.ts): ask how the followed scan stands. */
export async function reconcileDisk(): Promise<void> {
  await refresh?.().catch(() => {})
}
```

(`import { errorMessage } from '@/lib/errors'` and `import { newer } from './scanState'` are added. The `diskView.scan` assignment in `scanDrive` keeps the same single-flight guard `starting`.)

Add before `diskNodeActions`:

```ts
/** Fetches the folder whole when its children are missing or partial, and puts it into the scan's tree. */
export async function loadFolder(node: DiskNode, api: RainApi): Promise<void> {
  const scan = diskView.scan.value
  if (!scan?.root || !needsLoad(node, diskView.loaded)) return
  const full = await api.getScanNode(scan.id, node.path)
  const now = diskView.scan.value
  if (now?.id !== scan.id || !now.root) return          // a newer scan replaced this one while we waited
  diskView.loaded.add(node.path)
  diskView.scan.value = { ...now, root: replaceNode(now.root, full) }
}
```

In `diskNodeActions` (lines 116 to 141): the file branch (line 123) stays. Add one branch first: `if (node.kind === 'other') return []` (an "other" row has nothing to open, reveal or copy; the screen also keeps it out of the target, below). Replace the `open-folder` action (lines 128 to 135) with:

```ts
    {
      id: 'open-folder',
      label: 'Open folder',
      icon: 'folder-open',
      slot: 'primary',
      async run({ api }) {
        await loadFolder(node, api)                       // before navigating, so the list is whole when it appears
        diskView.folderPath.value = node.path
      },
    },
```

Why the fetch is done before the folder is opened (and not after): `DiskScreen.vue:64` selects the first row when the folder changes; fetching first means the list is drawn once, whole, and the selection logic is unchanged. A denied folder needs no fetch (`needsLoad` is false) and opens as an empty state.

`diskCache.ts`: no change. (It calls `resetDisk`, which now also frees the backend tree after the 5 minute drop.)

`FolderList.vue`: replace `iconOf` (lines 42 to 45):

```ts
function iconOf(node: DiskNode) {
  const name =
    node.kind === 'other' ? 'ellipsis'
    : node.denied ? 'lock'
    : node.kind === 'dir' ? 'folder'
    : ICON_BY_CATEGORY[node.category ?? 'other']
  return { kind: 'lucide', value: name } as const
}
```

and the size cell (lines 63 to 65) to `{{ item.node.denied ? '—' : formatBytes(item.node.sizeBytes) }}`. Both icons (`ellipsis`, `lock`) exist in `src/lib/icon-map.ts` (lines 336 and 406).

New `NotAccessible.vue` (strip badge, section 6.8):

```vue
<script setup lang="ts">
// "N folders not accessible": a lock and the count. The tooltip says what it means (spec 7.7).
import { LockIcon } from '@lucide/vue'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { formatNumber } from '@/lib/format'

defineProps<{ count: number }>()
</script>

<template>
  <TooltipProvider :delay-duration="300">
    <Tooltip>
      <TooltipTrigger as-child>
        <span class="flex shrink-0 items-center gap-1 text-xs text-muted-foreground" role="img"
              :aria-label="`${formatNumber(count)} folders not accessible`">
          <LockIcon :size="12" :stroke-width="1.75" aria-hidden="true" />
          {{ formatNumber(count) }}
        </span>
      </TooltipTrigger>
      <TooltipContent>
        {{ formatNumber(count) }} folders that Windows did not let Rain read. Their size is not in the total.
      </TooltipContent>
    </Tooltip>
  </TooltipProvider>
</template>
```

`DiskScreen.vue`:

| Lines | Change |
|---|---|
| 19 to 29 (imports) | add `import NotAccessible from './NotAccessible.vue'` |
| 78 to 82 (`target`) | `return node && node.kind !== 'other' ? { type: 'diskNode', id: node.path, item: node } : null`. An `other` row is selectable (the arrows pass over it) but has no action target. |
| 108 to 111 (`openPanel`) | first line: `if (row.node.kind === 'other') return` (no panel for an `other` row) |
| 154 (strip, before the size span) | `<NotAccessible v-if="trail.length === 1 && scan?.notAccessible" :count="scan.notAccessible" class="px-2" />` (shown only while the root is open) |
| 159 | `<EmptyState v-if="!folder.children?.length" :icon="folder.denied ? 'lock' : 'folder-open'" :title="folder.denied ? 'Not accessible' : 'This folder is empty'" />` |
| 173 to 178 (summary) | `<PathText v-if="selectedNode.kind !== 'other'" :path="selectedNode.path" />`. For an `other` row the summary shows only the size and share line, which already reads "3.1 GB, 4% of node_modules" and the row's own name tells it is the rest. |

`DiskScreen.vue:64` (`watch(folder, ...)`) stays as it is: the tree is whole before the folder changes, so the watcher runs once per navigation.

`treemap.ts` and `TreemapCanvas.vue`: NO change. Checked in the code: an `other` node is a leaf (no `children`), so `layoutTreemap` gives it area from `sizeBytes` and `categoryOf` falls to `node.category ?? 'other'` (line 108); a `truncated` folder is a leaf too and `dominantCategory` reads its `category` (line 90); a `denied` folder has size 0 and is not drawn (`isDrawn` needs 2 px). The label of an `other` rectangle is its name "other (1,204 items)".

`DriveList.vue`: no change (`title()` at line 40 shows "C: Local Disk"; the meter at line 72 is safe because `totalBytes > 0`).

`src/actions/commands.ts` (command `scan-c`, lines 80 to 90): no change. A rejected `listDrives` already shows the notice.

### 9.6 Settings: the path glyph

New `src/features/settings/usePathCheck.ts`:

```ts
// Asks the backend whether the folders in a path field exist (api.checkPaths), 300 ms after the last keystroke.
import { useDebounceFn } from '@vueuse/core'
import { ref, watch, type Ref } from 'vue'
import { api } from '@/data'
import { splitPaths } from './settingValues'   // defined once, in section 06 stage 1b; not defined or exported here

const DEBOUNCE_MS = 300

/** true = every folder exists, false = one does not, undefined = no glyph: the field is empty (the row shows its placeholder, section 06 6.6), or the answer is not known (yet, or the backend does not answer). */
export function usePathCheck(text: Ref<string>): Ref<boolean | undefined> {
  const found = ref<boolean>()
  let latest = 0
  async function run(): Promise<void> {
    const paths = splitPaths(text.value)
    const mine = ++latest
    if (paths.length === 0) {
      found.value = undefined
      return
    }
    try {
      const results = await api.checkPaths(paths)
      if (mine === latest) found.value = results.every((result) => result.exists && result.kind === 'dir')
    } catch {
      if (mine === latest) found.value = undefined
    }
  }
  const later = useDebounceFn(run, DEBOUNCE_MS)
  void run()                                   // the first answer does not wait
  watch(text, () => void later())
  return found
}
```

All path rows of Settings are folders (vault, project folders, Steam, Epic; the game folders row of section 06 as well), so "found" means `exists` and `kind === 'dir'`.

`SettingRow.vue`: remove the import at line 16 (`mockControl`) and add `import { usePathCheck } from './usePathCheck'`; replace lines 59 to 64 (the computed `pathFound`) with `const pathFound = usePathCheck(draft)`; replace lines 113 to 118 (the glyph) with:

```vue
        <span v-if="hasPathGlyph" class="flex size-4 items-center justify-center">
          <StatusDot
            v-if="pathFound !== undefined"
            :status="pathFound ? 'success' : 'failed'"
            :label="pathFound ? 'Found' : 'Missing'"
          />
        </span>
```

(The span keeps its 16 px box, so the row does not jump while the answer is pending.) The check runs on the typed text (`draft`), as the mock did, so the glyph follows the user's typing.

### 9.7 `src/main.ts`

Foundation 8.7 gives the block after stage 1b. Add the disk reconcile:

```ts
import { reconcileDisk, resetDisk } from '@/features/disk/actions'
...
onStreamReset(({ restarted }) => {
  if (restarted) { resetLaunches(); resetDisk() }
  else void reconcileDisk()                  // a plain reconnect or a lag: a final scan frame may have been missed
  refreshResources()
})
```

The `mockControl.onReseed` block (lines 12 to 19, with `resetDisk()` at line 15) stays until stage 10.

### 9.8 Mock adaptation (the mock must stay compilable until stage 10)

Foundation 8.6 decides the shape: `type MockApi = Omit<RainApi, RetiredMockMethod>` in `src/data/mock/index.ts`. The type was created in stage 2 by section 06 (with `'getHealth'`); this section EXTENDS the union in stage 3, together with the deletion of the disk slice and seed. There is no stub.

| File | Change |
|---|---|
| `src/data/mock/seed/disk.ts` | delete (333 lines) |
| `src/data/mock/slices/disk.ts` | delete |
| `src/data/mock/store.ts` | delete line 12 (import of `seedDrives`), the fields `drives`, `scans`, `trees` (lines 50 to 53) and their initial values (lines 89 and 90 and the one for `trees`); remove the unused type imports |
| `src/data/mock/index.ts` | remove `diskSlice` from the mock object; extend `type RetiredMockMethod = 'getHealth' \| 'listDrives' \| 'startScan' \| 'getScan' \| 'getScanNode' \| 'cancelScan' \| 'revealPath' \| 'checkPaths'` and `export type MockApi = Omit<RainApi, RetiredMockMethod>`; `createMockApi()` is declared to return `MockApi` and the all-mock branch of `src/data/index.ts` casts it (`createMockApi() as RainApi`; retired methods are undefined there, their screens show an error row, stage 10 deletes the branch; foundation 8.5); the router (`src/data/router.ts`) types its `mock` argument as `MockApi`, and `routing.ts` fails to compile when a retired method is routed to `'mock'` |
| `src/data/mock/control.ts` | delete `pathExists` from the interface (line 27 and its comment line 28), its implementation (lines 61 to 64), the import of `MISSING_PATHS` (line 9) |
| `src/data/mock/seed/settings.ts` | keep `MISSING_PATHS` (line 30) until section 06's stage deletes the settings seed; it is unused after this change and a linter warning is acceptable |
| `src/features/settings/SettingRow.vue` | see 9.6 |

`VITE_RAIN_MOCK=1` (the all-mock mode of foundation 2.8) therefore loses its Disk screen until stage 10 removes the mode. This is accepted by foundation 8.6: the flag is only a development aid.

### 9.9 Types barrel

In `src/data/types.ts` delete in the stage-3 commit: line 65 (`MachineState`), line 76 (`FileCategory`), line 77 (`Drive`), line 78 (`DiskNode`), lines 79 and 80 (`DiskScan`). `scripts/check.mjs` fails on a duplicate name. The type-identity check (foundation 9.4) is run for `Drive`, `MachineState`, `FileCategory` against the old hand-written lines before they are deleted; `DiskNode` and `DiskScan` differ on purpose (S.1, S.2) and are checked against the table of section 3.1.

### 9.10 Replacement text for `docs/ui-spec.md` (the architect applies it in stage 3; this section does not edit the file)

**Line 8 (amendment list):** replace "`mockControl` has `onReseed` and `pathExists`" with "`mockControl` has `onReseed`; path fields in Settings ask `api.checkPaths`".

**Settings table, lines 938, 940, 941, 942 (the rows with a glyph):** append once, under the table, the sentence: "The found (`circle-check`) or missing (`circle-x`) glyph comes from `api.checkPaths` 300 ms after the last keystroke. It shows only a folder that exists; a file, an empty field or a missing path is the missing glyph. No glyph shows while the answer is pending or when the backend does not answer."

**Section 7.7 Disk, replace the bullets "Scan failed" and "The tree is kept" and add the new ones; the other bullets stay:**

- **Scanning:** unchanged (a `Progress`, "184,203 files", Cancel). The scan runs in the backend and goes on if the window is hidden or closed.
- **Result:** unchanged in layout. The tree is a budgeted tree: about 2000 nodes, the largest 200 entries of the root and the largest 12 of every folder below it. What is not listed is one row per folder, `ellipsis` icon, "other (1,204 items)", with its size and share. An `other` row has no actions (Enter, Ctrl+Enter and the Ctrl+K panel do nothing on it). Its block in the map is in the "Other" colour.
- Opening a folder (Enter, double-click, breadcrumb) fetches that folder whole from the backend when the folder is not complete yet; the list appears when it is whole. A rare failure shows as a notice ("This scan is too old. Scan the drive again.").
- **Not accessible:** folders Windows did not let Rain read are counted. With the root open, the strip shows a `lock` icon and the count before the total size; the tooltip reads "N folders that Windows did not let Rain read. Their size is not in the total." A folder like that is a row with a `lock` icon and "—" for the size; opening it shows the empty state `lock` "Not accessible". The total of the scan is what Rain measured (logical size, as Explorer's "Size"); it does not have to equal "used" in the drive row.
- **Drive row:** line one is "C: Local Disk" when the volume has no label. Only fixed local drives are listed.
- **Scan failed:** `ErrorRow` under the drive with the backend's short text ("Drive C: could not be read"), Retry.
- Keys: unchanged (Enter opens, Backspace up, Ctrl+Enter reveals, Ctrl+R scans or rescans). Reveal shows the item selected in Explorer and then hides Rain, unless Keep open is on.
- The tree is kept in a module-level cache for 5 minutes after leaving Disk (one-shot timer), then dropped on the client and in the backend.

**Table row at line 313:** `| Disk node | Disk | Open folder (folders only) | Reveal in Explorer | Rescan, Copy path |` is unchanged; add under the table: "An `other` row has no actions."

**API block, lines 1062 to 1065:** replace with the disk block of section 9.2 of `05-disk-machine.md` (same text).

**Types block, lines 1154 to 1156:** replace with

```ts
export interface DiskNode { name: string; path: string; sizeBytes: number; kind: 'dir' | 'file' | 'other';
  category?: FileCategory; children?: DiskNode[]; truncated?: boolean; denied?: boolean }
export interface DiskScan { id: ID; driveId: ID; status: 'scanning' | 'done' | 'failed' | 'cancelled';
  progress: number; filesSeen: number; error?: string; notAccessible?: number; root?: DiskNode }
export interface PathCheck { path: string; exists: boolean; kind?: 'dir' | 'file' }
```

and add the sentence: "`root` is in the answer of `getScan` only; the `scan` topic never carries it."

**Line 998 (`RainError`):** not this section (foundation F1).

### 9.11 Stage and order

All of the above is stage 3. Order inside the stage: (1) types and routes in the backend with the fakes; (2) the REST client lines and the routing rows for `checkPaths`, `listDrives` and `getMachineState` (the smallest chain, proves the path through the bridge); (3) `startScan`, `getScan`, the topic and the new `scanDrive`; (4) the node route and `loadFolder`; (5) reveal; (6) the Settings glyph; (7) delete the mock disk files and the generated-type swap. The app builds and runs after every step because the router moves one method at a time.

---------------------------------------------------------------------------------------------------

## 10. Tests

### 10.1 Traits and fakes (foundation 12.2)

| Trait | Production | Fake (in `tests/common/`) |
|---|---|---|
| `DriveSource` | `platform::fs` Win32 calls | `FakeDrives(Vec<DriveInfo>)` with roots in a temp folder |
| `DirSource` | `StdDirs` (`read_dir`) | `FakeDirs`: an in-memory tree; can answer `PermissionDenied` or another error for a path, attach a delay per call, set a flag after N calls, and mark an entry as a reparse point |
| `Revealer` | `platform::shell::reveal` on the COM thread | `FakeRevealer` records the paths and can fail |
| `PathProbe` | `std::fs::metadata` / `symlink_metadata` | `FakeProbe` with a delay per path |
| `IdleSource` | `platform::idle` | `FakeIdle(Option<u64>)` |

The test builder sets them: `builder().with::<dyn DirSource>(..).with::<dyn DriveSource>(..)`. A second hook, `builder().game_mode_active(true)`, sets `state.game_mode_active`. Tests never call the real `SHOpenFolderAndSelectItems` (it opens a window in the user's face; foundation 11.6).

### 10.2 Unit tests

| File | Test (what it proves) |
|---|---|
| `category.rs` | every extension of the table maps to its category; upper case (`A.MP4`); `.gitignore`, `Makefile`, `archive.tar.gz` (gz is archive), a name ending with a dot, an 9-letter extension are handled; `ts` is `code`; `lock` is `code` |
| `walk.rs` with `FakeDirs` | sizes add up (folder size is the sum of files and sub-folders); `files` and `dirs` counters; a reparse entry is skipped and counted, not followed; a `PermissionDenied` folder is `denied` with size 0 and `denied` counter 1; `NotFound` is not counted as denied; another error is counted as denied; the depth cap turns a 300-level chain into a denied node at 256; 70 files in one folder keep 64 and fold 6 into `other_files` with the right bytes; cancel flag set after 3 calls stops the walk and returns a valid partial tree; category bytes: `dominant` is the category with most bytes including folded files |
| `arena.rs` (`cut`) | the sum invariant on hand-made trees; at most `budget` nodes; limit 12 below the first level and 200 at the first; one candidate past the limit is listed, not folded; the `other` node's name ("other (1 item)", "other (1,204 items)"), path (`<dir>\*`, root `C:\*`), size and kind; collapsed folder with content is `truncated` with `category`; denied folder has `denied` and size 0; empty folder has none of them; ties are ordered by name (output is the same on two runs); a folder that does not fit the budget stays collapsed and the next largest is tried |
| `arena.rs` (property) | a deterministic pseudo-random generator (a small LCG in the test, no new crate) builds 500 trees of 1 to 5000 folders with random sizes and depths; for each, for the root cut and for a cut at a random folder, assert invariants 1 to 5 of section 3 and `nodes <= budget` |
| `paths.rs` | the table of `normalize`: `C:\a\b`, `c:/a/b`, `"C:\a b"` (quotes), trailing `\`, `C:\` (root), `C:\a\..\b` (`Dotdot`), `C:foo`, `C:`, `foo\bar`, `\\srv\share` (`NotLocal`, and accepted with `allow_unc`), `\\?\C:\a`, `\\.\pipe\x`, `C:\a*`, `C:\a?`, a control character, 1025 characters, empty |
| `paths.rs` | `ids`: scan id format `scan-YYYYMMDD-HHMMSS-n`; drive id `drive-c` |
| `service.rs` | merge rule of `finish`: only `scanning` can end; a second `finish` is ignored (final frame once); progress is monotonic and capped at 0.99 |
| `machine` | `state()`: 0 ms -> active, 0 min; 59 999 -> active, 0; 60 000 -> not active, 1; 5 999 999 -> 99 min; `None` -> `None`; `wrapping_sub` across the 32-bit wrap (a test of the helper with `now = 5`, `last = u32::MAX - 4` gives 10) |
| `rain-types` | JSON round trip of every type of section 3 with a full and a minimal value; `assert_no_nulls`; `DiskNode` with a recursive child; the largest `size_bytes` a node can hold stays under `JSON_INT_MAX` |

### 10.3 Integration tests (`crates/rain-backend/tests/disk.rs`, `tests/machine.rs`)

Fixture trees are built at run time in a temp folder by `tests/common/tree.rs` (`TreeSpec`), so no file with a user's path is ever committed, and file sizes are set with `File::set_len` (instant). The named fixtures:

| Fixture | Content (sizes in bytes) |
|---|---|
| `disk-basic` | `a\` with files 100, 200, 300; `b\c\` with 70 files of 10 bytes each; `empty\`; `big.bin` 10 000; `notes.TXT` 5 (so root = 11 605 + 700 = 12 305; `b` = 700; `a` = 600) |
| `disk-denied` (in `FakeDirs`) | root with `ok\` (50 bytes) and `locked\` (answers `PermissionDenied`) and `gone\` (answers `NotFound`) |
| `disk-slow` (in `FakeDirs`) | 2000 folders with 5 files each and 20 ms delay per folder: a scan that takes seconds |
| `disk-synth` (in `FakeDirs`) | generated on demand: 200 000 files in 20 000 folders (no disk), for the size tests |
| `disk-junction` (real disk, Windows only) | `real\` (100 bytes) and a junction `link\` to it made with `mklink /J` through `hidden_command` (test only, with `#[allow(clippy::disallowed_methods)]` and a comment) |

Tests:

1. Drives: `GET /drives` with two fake drives returns them sorted by letter; empty label becomes "Local Disk"; `usedBytes = total - free`; a drive with total 0 is not listed; no drives gives `[]`.
2. Scan, happy path (`disk-basic`): `POST /scans` gives 201, status `scanning`, no `root`; wait for the `done` frame on the stream; the frame has no `root`; `GET /scans/{id}` has `root` with the exact sizes above; `b\c` has 64 files and one `other` node "other (6 items)" of 60 bytes; `filesSeen` is 75; `status` and `progress` are 1; `notAccessible` is absent.
3. Resync repair: connect the stream late (after `done`), then `GET /scans/{id}` equals the final state (the replay rule).
4. Frames: collect every `scan` frame of `disk-slow`: each is under 1 KiB; at most 4 per second (count in a 1 s window); `progress` never decreases and stays below 1 until the final frame; the final frame is the last frame of that id and there is exactly one.
5. Cancel: on `disk-slow`, `DELETE /scans/{id}` gives 204, a `cancelled` frame arrives within 100 ms, a second `DELETE` gives 204, `GET` shows `cancelled` without `root`; a new `POST /scans` works at once and its first frame has the new id. The walker threads end (the test waits until `Service::threads_alive()` (test hook) is 0 within 2 s).
6. Same drive twice: a second `POST /scans` for the same drive while it scans returns 200 and the same id. A scan of another drive cancels the first (a `cancelled` frame for the old id).
7. Denied (`disk-denied`): the scan is `done`, `notAccessible` is 1 (and `gone` is not counted), `locked` is a node with `denied: true` and size 0, the root size is 50.
8. Root unreadable: a fake whose root answers `PermissionDenied` ends `failed` with `error: "Drive T: could not be read"` (letter of the fake), no `root`.
9. Reparse: `disk-junction`: the junction is not followed (root size is 100, not 200). (Real disk. Skipped, with a visible "ignored" reason, if `mklink` is not allowed.)
10. Node route: `GET /scans/{id}/node?path=<path of b>` returns that folder with its children; a path of a file gives 400 `notADirectory`; an unknown path 404 `unknownPath`; the `other` node's path gives 400 `notADirectory`; case-insensitive match works for a path typed in another case; a scan that is not done gives 409 `scanNotDone`.
11. Expiry (T6, `tokio::time::pause`): after `done`, advance 10 minutes: node route gives 409 `scanExpired`, `GET /scans/{id}` is 200 `done` without `root`. A read at minute 9 moves the deadline (no expiry at minute 11). `DELETE` of a done scan frees the tree at once (node route gives `scanExpired`), the record stays `done`. A new scan drops the old tree.
12. Size tests (`disk-synth`): the `done` answer has at most 2000 nodes and its JSON is under 700 000 bytes; the node answer of the biggest folder has at most 2000 nodes; each frame under 1 KiB; the sum invariant holds on every folder of both answers (a recursive check in the test).
13. Game mode: `POST /scans` with game mode active gives 409 `gameMode`; a running scan is cancelled by `Service::cancel_running()`.
14. Module stop: `POST /internal/shutdown` during a running scan exits within the budget and publishes no `scan` frame after the `lifecycle` frame.
15. Reveal (`FakeRevealer`): an existing temp file is passed to the fake with the normalised path and the route answers 204; a missing file gives 404 `pathMissing`; `\\srv\share\x` gives 400 `networkPath`; `C:\a\..\b`, a relative path, `C:\a*` and an empty string give 400 `badPath`; a fake that fails gives 500 `revealFailed`; a fake probe that sleeps 5 s gives 503 `pathSlow` after about 2 s.
16. Path check: a temp folder is `exists` with `kind: dir`; a temp file is `exists` with `kind: file`; a missing path, a relative path, a path with `*`, a UNC path (fake probe: slow) are `exists: false`; the order and `path` echo are kept; 33 paths give 400 `tooMany`; a path of 1025 characters gives 400 `tooLong`; unknown body field gives 400 `badBody`; an empty list gives `[]`.
17. Machine: `FakeIdle(Some(0))` -> `{"userActive":true,"idleMinutes":0}`; `Some(61_000)` -> `{"userActive":false,"idleMinutes":1}`; `None` -> 503 `idleUnavailable`.
18. Topic and `resync` rule of foundation 12.2: the `scan` frame arrives with the expected payload and a `resync` arrives on connect.

Every route has success, invalid body, not found and idempotency cases (rule of foundation 12.2): covered by tests 1, 2, 5, 6, 10, 15, 16 and 17.

### 10.4 Frontend tests (`npm test`, vitest)

| File | What |
|---|---|
| `src/features/disk/scanState.test.ts` | `newer`: a final state is never replaced; two `scanning` snapshots in reversed order keep the larger `filesSeen`; a snapshot of another scan id replaces the current one; `done` after `scanning` wins |
| `src/features/disk/folders.test.ts` | `needsLoad`: false for a file, for a denied folder, for a loaded path, for a complete folder; true for a truncated folder and for a folder with an `other` child. `replaceNode`: replaces a deep folder, shares the untouched siblings (same object), returns the same root when the path is not found |
| `src/features/disk/actions.test.ts` | `scanDrive` with a fake `api` and a fake `shell`: a `done` frame without `root` leads to one `getScan`, the hold is released after the result is stored; `getScan` after subscribe repairs a scan that ended before the subscription; a failing `getScan` ends as `failed` with a message; the hold is always released; `resetDisk` calls `cancelScan` once and swallows its rejection; `open-folder` calls `getScanNode` once for a truncated folder and not again for the same folder |
| `src/data/rest/client.test.ts` (extends the file of stage 1b) | the eight methods of 9.3 produce the exact method, path, query and body (`/scans/scan-1/node?path=C%3A%5CUsers`) with the fake bridge |
| `src/features/settings/usePathCheck.test.ts` | fake timers: the first call is immediate; typing three times within 300 ms makes one more call; an older answer arriving later is ignored; `kind: 'file'` gives false; a rejected call gives `undefined`; an empty field gives `undefined` without a call (no glyph) |

Screens are not unit tested (foundation 12.3); they are checked by hand (section 11).

### 10.5 What cannot be tested automatically

Real Explorer selection and which window is in front (check 6). The real scan of `C:\` (time, memory, the not-accessible count: checks 2 to 4). The real idle time (check 8). Below-normal priority effect (check 3). Each is a manual check below; none of them is replaced by a fake test.

---------------------------------------------------------------------------------------------------

## 11. Acceptance checks

Besides `npm run check` (foundation 12.4), which must pass. All checks are made in a debug build (sandbox; no real data is touched: a scan only reads).

| # | Action | Expected |
|---|---|---|
| 1 | Open Disk (Ctrl+7) with the backend running. | One row "C: Local Disk" (the volume has an empty label), "N GB of M GB used", a meter. The numbers are within 1% of the drive's properties in Explorer. |
| 2 | Press Enter on the drive. Watch the Disk screen, then Task Manager (details tab, `rain-backend.exe`). | Progress bar and the file count update at most four times a second; the count reaches a plausible number (hundreds of thousands to millions). The scan ends and the result appears. Write down the time and the file count in the stage report (this is the first cold-or-warm measurement of a real drive). The memory of `rain-backend.exe` during and just after the scan is below 300 MB. Ten minutes after the last interaction with Disk it is back under 30 MB working set (T6). If the UI was left, the tree is freed after 5 minutes (S.8). |
| 3 | Run a scan twice: once with `SCAN_BELOW_NORMAL = true`, once with `false` (temporary build). | The scan with priority below normal is at most 20% slower; if it is more, the constant stays false. Write both times in the report. While the scan runs, the PC stays responsive (type in another window). |
| 4 | In the result: the strip of the root. Then open `C:\Windows` and `C:\Users` (a folder with many entries). | A lock icon and a count above 0 appear at the root (for example `System Volume Information`). Hover shows the sentence of section 6.8. A folder with a `lock` row shows "—"; opening it shows "Not accessible". `C:\Windows` shows about the size of Explorer's "Size" for that folder within 2% (logical size; the spike measured 44.3 GB and 195 236 files on this PC at that time). The root size is below the drive's "used" and nothing says it should be equal. A folder with more than 200 entries ends with an `ellipsis` row "other (N items)", which has no actions, and its block in the map is grey. |
| 5 | Create a junction `mklink /J %TEMP%\rain-test-link C:\Windows\System32` and scan again. | The scan finishes; `rain-test-link` is not counted twice (the folder shows no size of its own: junctions are skipped). Remove the junction afterwards (`rmdir`, never `del /s`). |
| 6 | Select a file in the result, press Ctrl+Enter. Then select a folder and press Ctrl+Enter. | Explorer opens with the item selected (about 0.2 s), comes to the front, and Rain hides. If Explorer opens behind other windows, record it and use open point 5. A file that was deleted after the scan: the notice reads "That file or folder no longer exists". |
| 7 | Open Settings. Type a folder that does not exist in the vault path; then a file; then an existing folder; then `C:\Windows;C:\nonexistent`. | After about 0.3 s the glyph shows missing, missing, found, missing. Clearing the field shows missing at once. With the backend stopped the glyph disappears. |
| 8 | From a PowerShell window, without touching keyboard or mouse afterwards: `Start-Sleep 90; Invoke-RestMethod http://localhost:1420/api/v1/machine` (needs `npm run dev`; a GET passes the proxy guard). | `userActive: false` and `idleMinutes: 1`. Immediately after moving the mouse the same call shows `userActive: true`, `idleMinutes: 0`. |
| 9 | Press Ctrl+R in the result, then Esc and Ctrl+7 again within 5 minutes. | The old result is replaced by the progress; coming back within 5 minutes shows the tree at once; after 5 minutes away it shows the drive list. |
| 10 | Start a scan, then press Ctrl+Enter. | "Scan cancelled", the drive list returns within a second, no thread of the scan stays (the CPU of `rain-backend.exe` returns to 0). A new scan starts normally. |
| 11 | Start a scan, then restart the backend (`npm run backend:restart`). | The screen returns to the drive list (the restart resets the disk state); no stuck "Scanning". |
| 12 | Start a scan, close the window (hide), wait for the scan to end, open the window. | The result is shown (the scan ran on; `reconcileDisk`/`getScan` after the reconnect gave the final state). |
| 13 | Idle measurement: with no client and no scan, `rain-backend.exe` for 60 s. | Under 30 MB working set and under 0.1 s CPU gained (the stage 2 rule; this stage adds no timer while idle). |

What "empty" looks like:

- No drive listed: the empty state "No drives found" (existing). Only possible with every fixed drive offline.
- A folder with nothing in it: "This folder is empty". A folder Windows refused: "Not accessible" with a lock.
- A drive that cannot be read: the drive row stays and an `ErrorRow` "Drive C: could not be read" with Retry appears under it.
- A scan with no tree left (10 minutes): the drive list is shown; opening a folder of a stale client tree gives the notice "This scan is too old. Scan the drive again."
- A path field with no text: the missing glyph. Idle with no information: "Machine state unavailable" (existing).

---------------------------------------------------------------------------------------------------

## 12. Open points

Only what cannot be decided inside this area.

| # | Question | Who decides | What the code does until then | Blocks |
|---|---|---|---|---|
| 1 | SETTLED by the editor: the registry (foundation 14.4) now has `getScanNode` and lists `checkPaths` -> `POST /paths/check` under module `disk`, section 05. Section 06 deleted its own design of the route, the types and the hook. | none | as written | nothing |
| 2 | Appendix A of the foundation needs the new constants of section 6.11 (`SCAN_OPEN_FOLDER_CHILDREN`, `SCAN_KEEP_FILES_PER_DIR`, `SCAN_MAX_DEPTH`, `SCAN_PROGRESS_TICK`, `SCAN_RECORDS_KEPT`, `SCAN_BELOW_NORMAL`, `PATH_PROBE_TIMEOUT`, `PATH_PROBE_PARALLEL`, `PATHS_CHECK_MAX`, `PATH_MAX_CHARS`, `USER_ACTIVE_BELOW_MS`) in `rain_types::limits`. Also: the "about 221 KB" of `SCAN_TREE_BUDGET` is without the `path` field of each node; with paths the figure is about 350 KB (to be measured in stage 3; the test limit is 700 KB). | architect | the constants are defined in `rain_types::limits` as listed | nothing |
| 3 | SETTLED by foundation 14.1: stage 1a defines only the `Topic` trait and the topic struct `HealthTopic`; the `Scan` topic struct is added in stage 3 with `DiskScan`. | none | stage 3 adds `DiskScan`, `DiskNode`, `ScanStatus`, `NodeKind`, `FileCategory` and the `scan` topic struct together | nothing |
| 4 | Section 03 (idle jobs): (a) DECIDED by the editor: `userActive` stays at 60 s; the status line in `IdleView.vue` becomes "PC idle for N min" (built from `idleMinutes`), without "Idle jobs can run"; section 03 uses `idle_ms()` only. (b) The T3 sampler calls `machine::Service::idle_ms() -> Option<u64>` (None means "not idle"). (c) If idle jobs also want CPU, GPU, memory load or a "full-screen app is running" test, this section has none of them built (no consumer, nothing spiked; options: `GetSystemTimes`, performance counters, `SHQueryUserNotificationState`); ask through the architect with the need. | section 03 and the architect | `GET /machine` and `idle_ms()` exist as written | nothing in this stage |
| 5 | Explorer in front after `reveal`: the spike proved the selection (218 ms) but not that the window comes to the front when the call is made from the windowless backend (Windows foreground rules). Check 6 decides. Fallback if it fails: the shell, which owns the visible window and therefore the foreground right, makes the same API call; the route and `RainApi.revealPath` stay unchanged; the architect adds a shell-local path (like `GET /local/backend`) or a second internal call. | architect, after check 6 | the backend makes the call | nothing |
| 6 | SETTLED: section 06 calls `disk().cancel_running()` in its silent step 1b (6.7.5) when game mode starts. Original text: the wind-down must call `disk::Service::cancel_running()` and `is_scanning()` (public, section 7). `state.activity` is the way `ping.busy` learns about the scan; the exact method name is core's (this section assumes a guard that counts while it lives). | section 06, architect | `cancel_running()` and the guard exist | nothing |
| 7 | SETTLED by foundation 8.6: `MockApi = Omit<RainApi, RetiredMockMethod>` (created in stage 2 by section 06, extended here, 9.8); no stub is kept, the disk slice and seed are deleted in stage 3. The side effect: `VITE_RAIN_MOCK=1` has no Disk screen from stage 3 until stage 10. If the all-mock mode must keep working, the architect keeps `seed/disk.ts` and the slice and only changes the routing rows. | architect | the stub of 9.8 | nothing |
| 8 | OneDrive: a file kept on the device may carry the placeholder reparse point and then be skipped, so its size is missing from the total (UNPROVEN, section 6.3). The check 4 compares a folder with Explorer. If the difference is large, the options are to count reparse FILES that are not marked "recall on access" (needs the attribute bits `0x400000` and `0x40000`), at the price of a little more code; the user decides whether the OneDrive folder matters. | the user, after check 4 | all reparse points are skipped and counted; the count is logged at `info` | nothing |

---------------------------------------------------------------------------------------------------

## 13. Hardware (module `hardware`)

The Hardware screen shows the disks, the keyboard and the mouse, one tile each. The screen is opened rarely, so the module costs nothing in between: no thread, no timer, no cache, no watcher of devices, and nothing at start except opening `hardware.toml`. Every read happens inside the call that asks for it.

**Routes.** `GET /hardware` returns `HardwareTile[]` (`listHardware`). `POST /hardware/{id}/open` (`openHardware`, 204) starts what the tile of the last read opens. Errors of the open route: 409 `hardwareNotRead` (no read yet, or an old id), 409 `nothingToOpen`, 500 `openFailed`. The Disk tile needs no call: the screen shows its own Disk screen.

**One read.** The drives (through `disk::Service::list_drives`), the HID device list and the list of installed programs run side by side. Then every Logitech channel is asked, side by side. Each part has its own limit of 1.5 s (`DEVICE_LIMIT`). A part that is late gives `None`: its tile is empty with a `problem` text, the others are not touched. The blocking readers get a deadline of 85 % of the limit, so what they have read by then is returned and not thrown away.

**Files of the module.**
- `inputs.rs`: sorts HID collections into keyboards, mice and HID++ channels. A device that has a keyboard interface of its own and a mouse collection on the same interface (mouse keys) is a keyboard. A vendor with `hidpp` in the table (Logitech) is never listed from its plain interfaces, because a receiver has a keyboard and a mouse interface whatever is paired.
- `hidpp.rs`: the HID++ 2.0 conversation, over a `Port` trait.
- `vendor.rs`: the vendor table (Logitech now; one row more for a new vendor) and how a program is found in the installed programs of Windows (`DisplayIcon`, else `InstallLocation` plus the file name; the file must exist).
- `model.rs`: `hardware.toml`. `source.rs`: the `Source` seam. `platform/hid.rs`: the only file that touches HID.

**HID.** The `hidapi` crate (feature `windows-native`: no C compiler, no DLL) lists the devices and opens one path. It was chosen over raw `windows` calls because it already handles enumeration, the report descriptor and reads with a timeout, which would be about 300 lines of unsafe code here. Listing opens no device. A channel is opened in the shared mode, so G HUB and VIA keep working.

**HID++ requests (read only).** Long report `0x11`, 20 bytes, software id `0xA`. The channel is the collection with usage page `0xFF00` usage 2 (USB, receivers) or `0xFF43` usage `0x0202` (Bluetooth LE). A ping (root function 1) goes to device 255 (a cable or Bluetooth) and to slots 1 to 6 (a receiver) at once; whoever answers is read: root `GetFeature` for `0x0005`, then the name (functions 0 and 1) and the type (function 2); then the battery from the first feature the device has: `0x1004` function 1, else `0x1001` function 0, else `0x1000` function 0. Nothing else is sent: no function that sets a value, lights an LED or changes a threshold. A test checks that every request is on this list. A percent from `0x1001` is worked out from the voltage and is marked `estimated`.

**Tiles.** Ids: `disk`, `keyboard`, `mouse`, then `keyboard-2` and so on. A kind with no device keeps a tile with `found: false`. The default of Enter: the vendor program, else the vendor download page; a keyboard with the VIA raw HID interface (`0xFF60`, `0x61`): the VIA program, else `https://usevia.app`. Programs and pages start through `state.opener()` (`ShellExec`).

**`hardware.toml`.** `[[tile]]` with `id` (a tile id or a kind), `name`, `hidden`, `opens` (a program path, a web address, "disk" or "none"). A rule by id beats a rule by kind. A broken file keeps the last good copy and shows in `/health`. A program path that does not exist leaves the tile with nothing to open and a `problem`.

**Left out.** Earbuds and headphones: Windows keeps a Bluetooth battery value, but on this PC its time stamp was three days old, so it is a guess. A tile is shown only when a clean reading exists.

**Tests.** Pure: HID++ answers from byte fixtures, the vendor table, `hardware.toml`, the sorting of HID collections (unit tests in the module). On a backend with a fake PC (`tests/hardware.rs`): tiles, the slow device, the rules, the open route, and "no read at start, a new read for every call". One test (`real_pc`, ignored) reads this PC.
