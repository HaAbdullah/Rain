# Rain UI prototype: final specification

Status: final for the UI-only prototype. Date: 2026-11-21.

> **Amendments after the first test with the owner (2026-11-21). They win over the text below.**
> - The bottom action bar (section 3.6) is removed. The frame is the rail, the header and the content area. Results of actions show in a small notice pill at the bottom of the window, only while there is something to say. Enter, Ctrl+Enter and Ctrl+K work as described; their labels are in the Ctrl+K panel and on the buttons in detail panes. Screens no longer provide a `summary`.
> - F11 and a header button toggle true full screen (the window covers the taskbar), not maximise. Double-clicking the header still maximises.
> - The icon package is `@lucide/vue`; `RainApi.search` takes `SearchType[]`; `mockControl` has `onReseed` and `pathExists`; `useSelection` has `resetOn`; `useSearch` has `disabled`.

Base concept: **home-first** (winner on the daily-use and clarity lenses), built on **workspace-first's single show/hide window** (winner on buildability), with palette-first's safety rules, flat result list and fixed detail pane grafted in.

Stack (fixed): Vue 3 + TypeScript + Vite, Tailwind CSS v4, shadcn-vue (Reka UI), Lucide icons, Tauri 2 on Windows 11 (WebView2). No backend. Every screen reads a typed mock layer.

---

## 1. Decisions at a glance

1. **One window, one size, show/hide only.** 1280x800 logical px, resizable, size and position remembered by the stock window-state plugin. Rain never resizes its own window. F11 is the native maximise toggle.
2. **Fixed frame on every screen:** 56 px icon rail (left), 56 px header with the one search field (top), 40 px action bar (bottom). Only the content area changes.
3. **Ctrl+` always lands the caret in the search field.** The query and the scope chip are cleared on every summon, so `Ctrl+``, type, Enter always launches, whatever section is showing.
4. **Scope is visible.** No chip in the search field = search everything (results replace the content as one flat ranked list). Chip = filter this section in place. Going to a section adds the chip; a summon removes it.
5. **Home is one vertical list plus a read-only Today panel.** No 2D board, so Home with and without a query uses the same list navigation. The cursor always starts on the first pinned row; "Needs you" sits above it.
6. **Focus rests in the search field; selection is state, not DOM focus.** It is implemented once (`useSelection`, `ItemList`, `ItemGrid`). Dialogs, popovers and Settings controls use native Reka focus and hand it back on close.
7. **Enter never approves, deletes or stops anything.** Enter = primary, Ctrl+Enter = secondary, Ctrl+K = everything else. The action bar always names both.
8. **Permissions have their own chords:** Ctrl+Y allow once, Ctrl+D deny, only in Workflows where the exact command is on screen. "Always allow" has no chord (button or Ctrl+K only).
9. **One fixed detail pane** in Apps, Projects, Games, Workflows and Life. No pane toggle, no pushed panes, no splitters.
10. **Esc ladder is three steps:** close overlay, clear query, hide. Shift+Esc hides at once. Ctrl+1 goes Home.
11. **Tab means one thing:** next sub-view (the strip under the header). Ctrl+Up/Down means next group.
12. **Game mode is gated.** If something would be interrupted, the countdown does not start until "Wait" or "Stop anyway" is chosen. Otherwise it starts at once.
13. **System fonts, one dark theme, no toast library, no router, no state library.** Added packages: `d3-hierarchy`, `simple-icons` only.
14. **Idle cost:** only the active screen is mounted, Disk is a lazy chunk, all intervals and the spinner stop while hidden, and after 60 s hidden Rain resets to Home.
15. **Mock layer has scenarios** (Typical, Empty, Busy, Failing), simulated latency, and one failing item per section in Typical, all behind one `RainApi` interface that maps 1:1 to REST later.

### Where the judges disagreed, and the call

| Topic | Call | Reason (one line) |
|---|---|---|
| Base concept (2 votes home-first, 1 workspace-first) | home-first behaviour on workspace-first's window model | Takes the winner on use and clarity and removes the one thing buildability penalised most (window tricks). |
| Focus in search field vs real DOM focus | Focus in search field | Two lenses preferred in-place filtering and "typing always works"; the cost is paid once in three shared files. |
| Home: spatial board vs list | List plus a glance panel | Removes the "two navigation models" strain and the most bespoke component, keeps "summon answers six of ten tasks". |
| Compact summon setting | Cut | It reintroduces resizing a frameless window; if wanted later it is a second window, not a resize. |
| Standard/Wide pair vs one resizable size | One remembered size, F11 = native maximise | Stock plugin, no custom sizes, and Disk still gets a big canvas with one key. |
| Game-mode gate (2 for, 1 against) | Keep the gate, cut Retry/Skip/Force | Prevents the most expensive mistake; the mock stays a four-phase machine. |
| Rail labels setting | Cut | One rail width for the prototype; the header names the section and tooltips name the icons. |
| Hold-Ctrl digit overlay on the rail | Cut | A key-state timer that misfires on every summon because Ctrl is already held. |
| "Always allow" chord | No chord | A permanent grant must not sit one modifier away from the routine approve. |
| Today panel on Home (buildability wanted it gone) | Keep, read-only, no continue-playing cover | It is why habits and agenda cost one press; it reuses list endpoints, no aggregate endpoint. |
| Right-click menu | Right-click opens the same ActionPanel | Mouse users reach every action without a second action surface. |
| Icon package | `@lucide/vue` | npm reports `lucide-vue-next` deprecated; verify at install, ship one icon package only. |
| Alt digits on rows | Shown only while Alt is held | Keeps rows text-light; listed as an owner question. |

---

## 2. Window model

### 2.1 Native window (Tauri)

| Property | Value |
|---|---|
| label | `main` |
| width x height | 1280 x 800 logical px |
| minWidth x minHeight | 1040 x 680 |
| position | `center: true` on first run, then remembered |
| first-run clamp | if the monitor work area is smaller than 1280x800, use 90% of the work area |
| decorations | `false` |
| shadow | `true` (gives Windows 11 rounded corners) |
| transparent | `false` |
| backgroundColor | `#171718` |
| visible | `false` at start (shown only by the hotkey) |
| resizable | `true` |
| skipTaskbar | `true` |

Plugins: `tauri-plugin-single-instance` (registered first), `tauri-plugin-global-shortcut`, `tauri-plugin-window-state` (save size, position and maximised state; do **not** save visibility).

Capabilities: `core:default`, `core:window:default`, `core:window:allow-start-dragging`, `core:window:allow-hide`, `core:window:allow-show`, `core:window:allow-set-focus`, `core:window:allow-toggle-maximize`, `core:window:allow-close`.

There is no second window, no compact mode, no programmatic `setSize` or `setPosition` anywhere.

### 2.2 Ctrl+` (registered in Rust as `Ctrl+Backquote`)

| Window state | Result |
|---|---|
| Hidden | Show, focus, caret in the search field |
| Visible and focused | Hide |
| Visible, not focused (kept open) | Focus, caret in the search field |

On every summon:
- The query is cleared and the scope chip is removed.
- Hidden for less than `resetAfterSeconds` (default 60): the same screen and selection are shown.
- Hidden for longer: Home, cursor on the first pinned row.
- The active screen's resources are reloaded (`reloadOnSummon`).

If registration fails (another app owns the chord), Rain logs it and still starts; Settings shows "Hotkey unavailable".

### 2.3 Escape

One step per press, fixed order:

1. Close the open overlay (action panel, dialog, shortcut sheet, select), or leave a focused Settings field.
2. Clear a non-empty query.
3. Hide the window.

Exceptions: on the Game mode screen Esc cancels game mode and returns to the previous screen. Shift+Esc hides at once from any state.

### 2.4 Hide on blur and hide after launch

- Hide on blur is **on** in release builds, **off** in dev builds (`import.meta.env.DEV`) and in the browser.
- It is suspended while: the window is kept open (F10, pin icon lit), any overlay is open, a launch checklist is running, a disk scan is running, game mode is in `review`, `countdown` or `windingDown`.
- Rain hides after every successful launch (app, pool, project, game, focus terminal, open note) unless kept open. A launch with a failed step keeps Rain visible with the failure on screen.
- Hide on blur lives in the webview layer (`shell/tauri.ts`, `onFocusChanged`) so holds can be checked in one place: `shell.holdOpen(reason)` returns a release function.

### 2.5 Drag region and window controls

- Drag region: the header background, the section title, and the 56x56 cell at the top of the rail. Each carries `data-tauri-drag-region`; interactive children do not. Double-clicking it maximises (native behaviour).
- CSS: `[data-tauri-drag-region] { app-region: drag; }`.
- Controls, right end of the header, 32x32 ghost icon buttons:
  - Waiting pill (`hand` + count, amber; hidden at 0; click goes to Workflows).
  - Keep open (`pin`; lit in `--ring` when on; F10).
  - Hide (`x`; tooltip "Hide" + `Esc`).
- No minimise, no close. Closing means hiding (`CloseRequested` is prevented and hides).

### 2.6 While hidden

- `<html data-hidden>` is set. CSS pauses the spinner; `useNow`, the mock tickers and every interval stop.
- Two one-shot timers are allowed: reset to Home after `resetAfterSeconds`, and drop the disk tree 5 minutes after leaving Disk.
- Only the active screen is mounted (`<component :is>`, no KeepAlive).

### 2.7 Plain browser (`npm run dev`)

Same root component, full viewport. `BrowserShell.hide()` sets `hidden = true` and the app renders `HiddenPlaceholder` (background colour, one dim line "Rain is hidden", `Ctrl` `` ` `` keycaps). A page-level Ctrl+` toggles it, so summon rules can be tested without Tauri. No component imports `@tauri-apps/api` directly; only `src/shell/tauri.ts` does.

---

## 3. App shell and navigation

### 3.1 Frame (at 1280x800)

```
+------+-------------------------------------------------------------+
| mark |  Title      [ chip | search field              ]   o  pin x |  header 56
+------+-------------------------------------------------------------+
| rail |                                                             |
|  56  |   content 1224 x 704  (ScreenFrame: strip, main, pane)      |
|      |                                                             |
+------+-------------------------------------------------------------+
|      |  summary / notice                 Enter .. Ctrl+Enter .. Ctrl+K |  action bar 40
+------+-------------------------------------------------------------+
```

Layout variables (in `tokens.css`): `--rail-w: 56px; --header-h: 56px; --footer-h: 40px; --strip-h: 40px; --row-h: 40px; --row-lg-h: 56px; --pane-w: 340px; --pane-w-wide: 420px`.

All content grids use `fr` units and `aspect-ratio`. Pixel sizes in section 7 are the values at 1280x800, not fixed sizes.

### 3.2 Header (`AppHeader`)

Grid: `132px minmax(0, 1fr) auto`, height 56, background `--surface-1`, bottom border `--border`.

- Column 1: section title, 14 px / 600, `--foreground`, left padding 16. It is the only navigation word on screen. While global results are showing it reads "Home".
- Column 2: `SearchField`, height 36, max-width 720, radius 8, background `--surface-2`, border 1 px `--border-strong`, text 16 px, caret `--ring`. No focus ring (it is always focused). Leading `search` icon 16 px, or the scope chip.
  - Scope chip: 24 px high, radius 6, background `--selected`, section icon 16 px + `x` 12 px. Click `x` removes it.
  - Placeholder: "Search everything" without a chip; "Filter <section>" with a chip.
  - A 1 px `--ring` loader line under the header while the active screen is loading (opacity fade only).
- Column 3: the controls from 2.5, 4 px gap, right padding 12.

### 3.3 Icon rail (`IconRail`)

Width 56, background `--sidebar`, right border `--sidebar-border`.

- Top cell 56x56: Rain mark (`cloud-rain`, 20 px, `--muted-foreground`), drag region.
- Buttons 40x40, radius 8, 8 px apart, centred. Icon 20 px, stroke 1.75.
- Idle: icon `--sidebar-foreground`. Hover: background `--hover`. Current: background `--selected`, icon `--foreground`.
- Top group: the seven sections. Bottom group (`margin-top: auto`, 8 px bottom padding): Game mode, Settings.
- Tooltip to the right after 300 ms: name + keycap.
- Workflows badge: amber pill (min 16x16, 12 px / 600, top-right) with the count of sessions waiting on you. With no waiting session and at least one job whose last run failed: an 8 px `--status-failed` dot instead.
- No labels, no digit overlay.

### 3.4 Routes

No vue-router. `src/nav/routes.ts` holds the table; `useNav()` holds `route: { name, sub?, params? }`. In the browser the path is mirrored to `location.hash` so a refresh keeps the screen.

| Path | Name | Title | Lucide icon | Key | Rail | Sub-views (`sub`) |
|---|---|---|---|---|---|---|
| `/` | `home` | Home | `house` | Ctrl+1 | top | none |
| `/apps` | `apps` | Apps | `layout-grid` | Ctrl+2 | top | none |
| `/projects` | `projects` | Projects | `folder-git-2` | Ctrl+3 | top | none |
| `/games` | `games` | Games | `gamepad-2` | Ctrl+4 | top | `all`, `steam`, `epic`, `custom`, `installed` |
| `/workflows/:sub` | `workflows` | Workflows | `workflow` | Ctrl+5 | top | `sessions`, `jobs`, `idle` |
| `/life/:sub` | `life` | Life | `calendar-check` | Ctrl+6 | top | `month`, `year` |
| `/disk` | `disk` | Disk | `hard-drive` | Ctrl+7 | top | none |
| `/game-mode` | `gameMode` | Game mode | `circle-power` | Ctrl+G | bottom | none |
| `/settings` | `settings` | Settings | `settings` | Ctrl+, | bottom | none |

```ts
// src/nav/routes.ts
export type RouteName = 'home' | 'apps' | 'projects' | 'games' | 'workflows' | 'life' | 'disk' | 'gameMode' | 'settings'
export interface RouteDef {
  name: RouteName; path: string; title: string; icon: string   // lucide name
  combo: string                                                 // e.g. 'Ctrl+Digit1'
  rail: 'top' | 'bottom'; subs?: readonly string[]; defaultSub?: string
  filterable: boolean                                           // false for home and gameMode
  component: () => Promise<{ default: Component }>              // lazy import of the screen
}
export interface Route { name: RouteName; sub?: string; params?: Record<string, string> }
```

`useNav()` returns `{ route, displayed, go(name, sub?, params?), back() }`. `displayed` is `'home'` whenever `scope === null && query !== ''`, otherwise `route.name`. The rail and the title follow `displayed`.

### 3.5 Search and scope

```ts
// src/composables/useSearch.ts
useSearch(): { query: Ref<string>; scope: Ref<RouteName | null>; setScope(n: RouteName | null): void; clear(): void; focus(): void }
useSectionQuery(name: RouteName): ComputedRef<string>   // '' unless scope === name
```

- `nav.go(name)` (Ctrl+digit, rail click, "Show in ...") sets `scope = name` when the route is `filterable`, and clears the query.
- Summon sets `scope = null` and clears the query.
- Ctrl+F toggles the chip for the current section.
- Backspace on an empty query steps out one level: Disk folder up if inside a folder, otherwise removes the chip.
- No chip and a non-empty query: `HomeScreen` renders the flat result list in place of the current screen. Esc clears the query and the previous screen is back.
- With a chip, the screen filters its own items (case-insensitive substring on name and keywords via `lib/score.ts#matches`) and selects the first match.

### 3.6 Action bar (`ActionBar`)

Height 40, background `--surface-1`, top border `--border`, padding 0 12.

- Left: 16 px icon + one line, 13 px, `--muted-foreground`, truncated. It shows the selected item's summary (from the screen). A notice replaces it for 3 s (`useNotice`): info, success (`--status-success` icon) or error (`--status-failed` icon; stays until the next key press). There are no toasts.
- Right, each a clickable ghost button: primary label 13 px / 500 `--foreground` + `KeyHint` Enter; 1x12 px divider; secondary label 13 px `--muted-foreground` + `KeyHint` Ctrl Enter; divider; "Actions" + `KeyHint` Ctrl K.
- A missing secondary action is not drawn. With no selection only "Actions" is drawn.

### 3.7 Overlays (all mounted once in `AppShell`)

| Overlay | Component | Opened by |
|---|---|---|
| Action panel | `ActionPanel` (`Popover` + `Command`, 320 px, anchored bottom-right; at the pointer on right-click) | Ctrl+K, right-click |
| Shortcut sheet | `CheatSheet` (`Dialog`, 720 px) | F1, Ctrl+/ |
| Log viewer | `LogDialog` (`Dialog`, 880x560) | Ctrl+L, job/idle Enter |
| Confirm | `ConfirmDialog` (`AlertDialog`) | destructive actions |

Feature dialogs (pool editor, custom game) live in their feature folder and register with `useOverlay(openRef)`. While any overlay is registered open, the global keymap is suspended and hide on blur is held.

### 3.8 Mouse rules

- Sections with a detail pane: click selects, double-click runs the primary action.
- Home list (no pane): click runs the primary action.
- Right-click selects and opens `ActionPanel` at the pointer.
- Hover shows `--hover` fill only. Nothing is revealed on hover except tooltips.
- A click on empty space returns focus to the search field.

---

## 4. Global keyboard map

### 4.1 Rules

1. **Plain keys type.** Letters, digits and space always go to the search field. No single-key shortcuts.
2. **Arrows move, Enter acts.** Enter = primary, Ctrl+Enter = secondary, Ctrl+K = the full list with shortcuts.
3. **Enter never approves, deletes or stops.** Those are Ctrl+Y, Ctrl+D, or an item in Ctrl+K behind `ConfirmDialog`. There is no Delete-key binding.
4. **Ctrl+digit goes to a section** in rail order. Alt+digit runs the Nth row on Home.
5. **Ctrl+letter has one meaning everywhere** (table below). A chord that does not apply to the selected item does nothing and is not listed in Ctrl+K.
6. **Function keys act on the window** (F1 help, F10 keep open, F11 maximise).
7. **Tab is the next sub-view. Ctrl+Up/Down is the next group. PageUp/PageDown is a page** (a page of rows, or a month/year in Life).
8. **Esc goes back, three steps at most.**
9. Left/Right move the selection in grids and belong to the caret in lists. Home/End always belong to the caret.
10. One table (`src/keys/keymap.ts`) binds the keys, fills the tooltips and generates the shortcut sheet.

### 4.2 Key table

| Group | Key | Action | Applies when |
|---|---|---|---|
| OS | Ctrl+` | Summon / hide | Always (global hotkey) |
| Window | Esc | Back: overlay, query, hide | Always |
| Window | Shift+Esc | Hide now | Always |
| Window | F1 or Ctrl+/ | Shortcut sheet | Always |
| Window | F10 | Keep open on/off | Always |
| Window | F11 | Maximise / restore | Always |
| Go | Ctrl+1 .. Ctrl+7 | Home, Apps, Projects, Games, Workflows, Life, Disk | Always |
| Go | Ctrl+, | Settings | Always |
| Go | Ctrl+G | Game mode | Always |
| Go | Tab / Shift+Tab | Next / previous sub-view | Screen has a `ViewStrip` |
| Move | Up / Down | Move selection | Always |
| Move | Left / Right | Move selection | Grid screens |
| Move | PageUp / PageDown | Page up / down; previous / next month or year in Life | Always |
| Move | Ctrl+Up / Ctrl+Down | Previous / next group (habit in Life Year) | Grouped screens |
| Move | Backspace (empty query) | Step out: Disk folder up, else remove chip | Always |
| Act | Enter | Primary action | Item selected |
| Act | Ctrl+Enter | Secondary action | Item has one |
| Act | Ctrl+K | Action panel | Item selected |
| Act | Alt+1 .. Alt+9 | Run the Nth numbered row | Home list |
| Act | Ctrl+P | Pin / unpin to Home | app, pool, project, game |
| Act | Ctrl+N | New: pool (Apps), custom game (Games) | Apps, Games |
| Act | Ctrl+R | Run now / rescan / re-read | job, idle job, Disk, Life; otherwise reload the screen's data |
| Act | Ctrl+L | Log or transcript | session, job, idle job (also from Home rows) |
| Act | Ctrl+Y | Allow once | Workflows > Sessions, waiting session selected |
| Act | Ctrl+D | Deny | Workflows > Sessions, waiting session selected |
| Command | Ctrl+T | Open today's daily note, then hide | Always |
| Command | Ctrl+F | Section filter on / off (scope chip) | Filterable screens |

### 4.3 Enter and Ctrl+Enter per item type

| Item | Where | Enter | Ctrl+Enter | More in Ctrl+K |
|---|---|---|---|---|
| App | anywhere | Launch | Open file location | Pin, Copy path |
| Pool | anywhere | Launch all | Edit pool | Pin, Duplicate, Delete (confirm) |
| Project | anywhere | Open dev environment (Focus if open; Locate if missing) | Open in editor only | Pin, Terminal here, New Claude session here, Reveal in Explorer, Copy path |
| Game | anywhere | Play (Install if not installed) | Play in game mode | Pin, Open store page, Hide from library |
| Session | Workflows | Focus terminal | Open folder in editor | Allow once, Always allow, Deny (waiting only), Transcript, Stop session (confirm), Copy folder path |
| Job | Workflows | Open runs and log | Enable / disable | Run now, Copy cron |
| Idle job | Workflows | Open log | Enable / disable | Run now (ignore idle) |
| Session, job, idle job | Home | Show in Workflows | (none) | Transcript / log, Run now |
| Day cell | Life | Open that day's note | Open today's note | Re-read vault |
| Drive | Disk | Scan | (none) | (none) |
| Disk node | Disk | Open folder (folders only) | Reveal in Explorer | Rescan, Copy path |
| Setting | Settings | Change (toggle, open, or edit) | Reset to default | (none) |
| Command | Home | Run | (none) | (none) |

Game mode screen: see 7.8 (Enter and Ctrl+Enter are named on the buttons).

### 4.4 Implementation rules for the handler (`useKeymap`)

- One `keydown` listener on `window`, capture phase. Match on `event.code` plus modifiers (`Digit1`, `Backquote`, `Comma`, `Slash`, `KeyK`), so layouts do not matter.
- Ignore events with `event.isComposing`.
- If an overlay is registered open: handle nothing (Reka handles Esc and arrows inside it).
- If the target is an input other than the search field (Settings field): handle only Esc (blur and refocus search) and Ctrl+digit.
- `preventDefault` on every bound key, and on this WebView2 block list even when unbound: Ctrl+P, Ctrl+R, F5, Ctrl+F, F3, Ctrl+G, Ctrl+N, Ctrl+T, Ctrl+L, Ctrl+D, Ctrl+J, Ctrl+U, F7, Alt+Left, Alt+Right, Ctrl+Plus, Ctrl+Minus, Ctrl+0, Tab (outside overlays), and a lone Alt key-up. F12 and Ctrl+Shift+I stay enabled in dev builds only.
- Up/Down always `preventDefault` (the caret never jumps). Left/Right `preventDefault` only when the screen's `move()` returns `true`.
- Holding Alt sets `altHeld` (cleared on key-up and on window blur); `ListRow` shows its digit only while it is true.
- In the browser shell, Ctrl+Backquote toggles the simulated hide. In Tauri the page never sees it.

```ts
// src/keys/keymap.ts
export type KeyGroup = 'Window' | 'Go' | 'Move' | 'Act' | 'Command'
export interface KeyBinding {
  id: string
  combo: string | string[]          // 'Ctrl+KeyK', 'F11', 'Shift+Escape'
  group: KeyGroup
  label: string                     // shown in the shortcut sheet and tooltips
  when?: (ctx: KeyContext) => boolean
  run: (ctx: KeyContext) => void
}
export interface KeyContext { route: Route; screen: ScreenController | null; target: ActionTarget | null; search: ReturnType<typeof useSearch> }
export const keymap: KeyBinding[]
export function formatCombo(combo: string): string[]   // 'Ctrl+Enter' -> ['Ctrl', '↵']
```

---

## 5. Design tokens

Dark is the only theme. `<html class="dark">` is set in `index.html` so shadcn-vue's `dark:` utilities apply. No light theme in the prototype (a second token set is not cheap to test).

### 5.1 `src/styles/tokens.css`

```css
@import "tailwindcss";
@import "tw-animate-css";

@custom-variant dark (&:is(.dark *));

:root {
  color-scheme: dark;

  /* ---- shadcn-vue variables ---- */
  --background: #171718;
  --foreground: #f2f2f3;
  --card: #262729;
  --card-foreground: #f2f2f3;
  --popover: #202121;
  --popover-foreground: #f2f2f3;
  --primary: #0a78cc;              /* one step darker than #0b8ce9 so white 14px text reaches 4.5:1 */
  --primary-foreground: #ffffff;
  --secondary: #262729;
  --secondary-foreground: #f2f2f3;
  --muted: #262729;
  --muted-foreground: #a6a7ab;
  --accent: #313235;               /* hover and highlight inside shadcn components */
  --accent-foreground: #f2f2f3;
  --destructive: #e5484d;
  --destructive-foreground: #ffffff;
  --border: #2d2e32;
  --input: #494a50;
  --ring: #0b8ce9;
  --radius: 0.5rem;                /* 8px; sm 4, md 6, lg 8, xl 12 */

  --chart-1: #4f8fd6;
  --chart-2: #3fa7a0;
  --chart-3: #c77d3a;
  --chart-4: #7c6bd6;
  --chart-5: #6fae5a;

  --sidebar: #202121;
  --sidebar-foreground: #a6a7ab;
  --sidebar-primary: #0a78cc;
  --sidebar-primary-foreground: #ffffff;
  --sidebar-accent: #313235;
  --sidebar-accent-foreground: #f2f2f3;
  --sidebar-border: #2d2e32;
  --sidebar-ring: #0b8ce9;

  /* ---- Rain: surfaces ---- */
  --surface-0: #141414;            /* wells: treemap, heatmap, log, code block */
  --surface-1: #202121;            /* rail, header, action bar, detail pane */
  --surface-2: #262729;            /* cards, tiles, inputs, keycaps */
  --hover: #313235;                /* pointer hover */
  --selected: #3c3d42;             /* keyboard selection fill */
  --border-strong: #494a50;        /* inputs, overlay outline */
  --text-3: #84858c;               /* placeholder, disabled, hints only */

  /* ---- Rain: status (always paired with a shape, see StatusDot) ---- */
  --status-running: #3aa0f5;
  --status-waiting: #fd9903;
  --status-success: #00cd72;
  --status-failed: #e5484d;
  --status-idle: #84858c;

  /* ---- Rain: heatmap, one hue ---- */
  --heat-0: #262729;
  --heat-1: #12395a;
  --heat-2: #0f5a94;
  --heat-3: #0b7bcc;
  --heat-4: #3aa5f2;

  /* ---- Rain: file categories (treemap) ---- */
  --file-video: #7c6bd6;
  --file-image: #3fa7a0;
  --file-audio: #c77d3a;
  --file-archive: #b8a13a;
  --file-code: #4f8fd6;
  --file-document: #6fae5a;
  --file-app: #c2607a;
  --file-other: #55565e;

  /* ---- Rain: layout ---- */
  --rail-w: 56px;
  --header-h: 56px;
  --footer-h: 40px;
  --strip-h: 40px;
  --row-h: 40px;
  --row-lg-h: 56px;
  --pane-w: 340px;
  --pane-w-wide: 420px;

  /* ---- Rain: elevation and motion ---- */
  --shadow-overlay: 0 8px 24px rgb(0 0 0 / 0.4), inset 0 1px 0 rgb(255 255 255 / 0.06);
  --dur: 100ms;
  --dur-overlay: 80ms;
  --ease: cubic-bezier(0, 0, 0.2, 1);

  /* ---- Rain: type ---- */
  --font-sans: "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif;
  --font-mono: "Cascadia Mono", "Cascadia Code", Consolas, monospace;
}

@theme inline {
  --color-background: var(--background);
  --color-foreground: var(--foreground);
  --color-card: var(--card);
  --color-card-foreground: var(--card-foreground);
  --color-popover: var(--popover);
  --color-popover-foreground: var(--popover-foreground);
  --color-primary: var(--primary);
  --color-primary-foreground: var(--primary-foreground);
  --color-secondary: var(--secondary);
  --color-secondary-foreground: var(--secondary-foreground);
  --color-muted: var(--muted);
  --color-muted-foreground: var(--muted-foreground);
  --color-accent: var(--accent);
  --color-accent-foreground: var(--accent-foreground);
  --color-destructive: var(--destructive);
  --color-destructive-foreground: var(--destructive-foreground);
  --color-border: var(--border);
  --color-input: var(--input);
  --color-ring: var(--ring);
  --color-chart-1: var(--chart-1);
  --color-chart-2: var(--chart-2);
  --color-chart-3: var(--chart-3);
  --color-chart-4: var(--chart-4);
  --color-chart-5: var(--chart-5);
  --color-sidebar: var(--sidebar);
  --color-sidebar-foreground: var(--sidebar-foreground);
  --color-sidebar-primary: var(--sidebar-primary);
  --color-sidebar-primary-foreground: var(--sidebar-primary-foreground);
  --color-sidebar-accent: var(--sidebar-accent);
  --color-sidebar-accent-foreground: var(--sidebar-accent-foreground);
  --color-sidebar-border: var(--sidebar-border);
  --color-sidebar-ring: var(--sidebar-ring);

  --color-surface-0: var(--surface-0);
  --color-surface-1: var(--surface-1);
  --color-surface-2: var(--surface-2);
  --color-hover: var(--hover);
  --color-selected: var(--selected);
  --color-border-strong: var(--border-strong);
  --color-text-3: var(--text-3);
  --color-status-running: var(--status-running);
  --color-status-waiting: var(--status-waiting);
  --color-status-success: var(--status-success);
  --color-status-failed: var(--status-failed);
  --color-status-idle: var(--status-idle);
  --color-heat-0: var(--heat-0);
  --color-heat-1: var(--heat-1);
  --color-heat-2: var(--heat-2);
  --color-heat-3: var(--heat-3);
  --color-heat-4: var(--heat-4);

  --radius-sm: calc(var(--radius) - 4px);   /* 4  keycaps */
  --radius-md: calc(var(--radius) - 2px);   /* 6  controls, tags, chips */
  --radius-lg: var(--radius);               /* 8  rows, cards, tiles, covers */
  --radius-xl: calc(var(--radius) + 4px);   /* 12 popovers, dialogs */
  --radius-cell: 3px;                       /* heatmap cells */

  --font-sans: var(--font-sans);
  --font-mono: var(--font-mono);
  --text-13: 0.8125rem;                     /* utility: text-13 */
  --text-13--line-height: 1.125rem;
  --text-countdown: 2rem;                   /* utility: text-countdown */
  --text-countdown--line-height: 2.5rem;
}

@layer base {
  * { border-color: var(--border); }
  html { font-size: 16px; }
  body {
    background: var(--background); color: var(--foreground);
    font-family: var(--font-sans); font-size: 0.875rem; line-height: 1.25rem;
    font-variant-numeric: tabular-nums; -webkit-font-smoothing: antialiased;
    overflow: hidden; user-select: none;
  }
  [data-tauri-drag-region] { app-region: drag; }
  ::-webkit-scrollbar { width: 10px; height: 10px; }
  ::-webkit-scrollbar-thumb { background: var(--selected); border-radius: 5px; border: 2px solid transparent; background-clip: content-box; }
  ::-webkit-scrollbar-track { background: transparent; }
  .selectable { user-select: text; }       /* paths, commands, logs */
  html[data-hidden] * { animation-play-state: paused !important; }
  @media (prefers-reduced-motion: reduce) { * { transition: none !important; animation: none !important; } }
}
```

### 5.2 Type scale

Tailwind's default names are kept so shadcn-vue components keep their sizes.

| Utility | Size / line | Weight | Use |
|---|---|---|---|
| `text-xs` | 12 / 16 | 400, 500 | Group headers, meta, keycaps, badges, cover titles, mono paths |
| `text-13` | 13 / 18 | 400, 500 | Action bar, subtitles, agenda, table cells |
| `text-sm` | 14 / 20 | 400, 500, 600 | Rows, body, buttons, section title (600) |
| `text-base` | 16 / 24 | 400, 600 | Search input, pane titles (600) |
| `text-xl` | 20 / 28 | 600 | Empty-state titles, game-mode heading |
| `text-countdown` | 32 / 40 | 600 | Game-mode countdown number |

Nothing below 12 px. Mono (`font-mono`, 12 px) for paths, commands, cron strings, logs.
Text tiers: `--foreground` primary, `--muted-foreground` secondary, `--text-3` placeholder, disabled and hints only.

### 5.3 Spacing, shape, icons

- Spacing: 4, 8, 12, 16, 24, 32 (Tailwind `1 2 3 4 6 8`). Pane padding 16. Grid gap 12. List padding 8.
- Heights: control 32 (`h-8`), small control 24 (`h-6`), row 40, large row 56, header 56, strip 40, action bar 40, keycap 20.
- Radii: 3 heatmap cell, 4 keycap, 6 controls and chips, 8 rows, cards, tiles and covers, 12 overlays.
- Elevation: none inside the window; surfaces are flat and divided by 1 px `--border`. Overlays only: `box-shadow: var(--shadow-overlay)` and a 1 px `--border-strong` outline.
- Selection: rows get `background: var(--selected)`; tiles get `outline: 2px solid var(--ring); outline-offset: 0`. Native focus (inside overlays and Settings fields) uses `outline: 2px solid var(--ring); outline-offset: 2px`.
- Icons: Lucide, stroke 1.75. 20 px in the rail and as the leading icon of a row; 16 px in controls, status, type and trailing positions; 12 px inside chips; 32 px in empty states. Brand marks from `simple-icons` (named imports only): `siSteam`, `siEpicgames`, `siObsidian`, `siClaude`, `siGithub`, `siGooglechrome`, `siGooglecalendar`, `siDiscord`. Anything else is a letter tile.

### 5.4 Status vocabulary (shape plus colour, app-wide)

| State | Lucide | Colour | Used for |
|---|---|---|---|
| idle | `circle` | `--status-idle` | Idle session, job with no runs |
| running | `loader-circle` (spins) | `--status-running` | Working session, running job, launch step |
| waiting | `hand` | `--status-waiting` | Waiting on you |
| success | `circle-check` | `--status-success` | Done, last run ok, step ok |
| failed | `circle-x` | `--status-failed` | Failed |
| stale | `circle-dashed` | `--status-idle` | Working session silent for 60 s or more |
| paused | `circle-pause` | `--status-idle` | Disabled job, paused idle job |
| queued | `circle-dot` | `--status-idle` | Idle job waiting for idleness |

Tags (`Badge`) are coloured text on a 15% tint: `background: color-mix(in srgb, <colour> 15%, transparent)`.

### 5.5 Motion

- Transitions: `color`, `background-color`, `border-color`, `opacity` only, `var(--dur) var(--ease)`.
- No transforms, no layout, enter or exit animation, no blur, no show animation for the window.
- Overlays fade in `var(--dur-overlay)`. Remove the `zoom-*` and `slide-*` classes from the generated `DialogContent`, `AlertDialogContent`, `PopoverContent`, `TooltipContent`, `SelectContent`.
- The spinner (`loader-circle`, 1 s linear rotate) is the only loop. It exists only while something runs and pauses under `html[data-hidden]`.
- Skeletons appear only after 150 ms of loading and do not pulse (remove `animate-pulse`).
- The game-mode countdown repaints once per second.

### 5.6 Edits to generated shadcn-vue files (foundation owner, once)

1. `command/CommandItem.vue`: highlighted state uses `bg-selected` instead of `bg-accent`.
2. `toggle/index.ts` (toggle variants): on-state uses `bg-selected`.
3. `tooltip/TooltipContent.vue`: `bg-popover text-popover-foreground border border-border-strong`, 12 px, no arrow.
4. `skeleton/Skeleton.vue`: remove `animate-pulse`.
5. Overlay contents: remove zoom and slide classes (5.5).
6. Replace `lucide-vue-next` imports with `@lucide/vue` in `src/components/ui` (verify the deprecation on npm at install; if it is not deprecated, keep `lucide-vue-next` everywhere instead; ship exactly one icon package).

---

## 6. Shared components and composables (built once, in `src/components/rain` and `src/composables`)

### 6.1 Components

| Name | Props / slots | Purpose |
|---|---|---|
| `AppShell` | none | The frame. Mounts `IconRail`, `AppHeader`, the active screen, `ActionBar`, the overlays, `HiddenPlaceholder`. Starts `useKeymap`. |
| `IconRail` | none (reads `routes`, `useNav`, sessions and jobs resources for the badge) | Section navigation. |
| `AppHeader` | none | Title, `SearchField`, window controls, loader line. |
| `SearchField` | none (binds `useSearch`) | The one text input. Exposes `focus()`. |
| `ActionBar` | none (reads `useScreenState`, `useNotice`) | Summary or notice, primary, secondary, Actions. |
| `ActionPanel` | none (opened via `useActionPanel().open(target, anchor?)`) | `Popover` + `Command`: every action for the target, filterable, each with its `KeyHint`; danger items last in `--destructive`. |
| `CheatSheet` | none | `Dialog` generated from `keymap` plus section 4.3's registry labels, grouped Window, Go, Move, Act, Command. |
| `LogDialog` | opened via `useLogDialog().open({ title, runs?, selectedRunId?, load })` | Left 240 px run list (when `runs` given, native Reka listbox), right mono log in `ScrollArea` on `--surface-0`; `LogLine.level` colours the line's 2 px left bar. |
| `ConfirmDialog` | opened via `useConfirm()({ title, body, confirmLabel, danger })` returning `Promise<boolean>` | `AlertDialog`. Default focus on Cancel. |
| `HiddenPlaceholder` | none | Browser-only stand-in for the hidden window. |
| `ScreenFrame` | `pane?: 'none' \| 'md' \| 'wide'` (default `'none'`); slots `strip`, default, `pane` | Content layout: optional 40 px strip, main area, optional fixed pane on the right. |
| `DetailPane` | `title: string`, `icon?: IconRef`, `status?: StatusKind`; slots default, `actions` | Pane body: 16 px padding, `--surface-1`, left border, title 16/600, scrolls; `actions` slot pinned at the bottom. |
| `ViewStrip` | `items: { id: string; label: string; icon: string; brand?: boolean; count?: number; warn?: boolean }[]`, `modelValue: string`; slot `end` | The one sub-view strip (40 px, under the header). Icon 16 px + 13 px label + optional count. Selected item has `--selected` fill. Tab cycles it. |
| `ItemList` | `groups: SelGroup<T>[]`, `selectedId: string \| null`, `rowSize?: 'md' \| 'lg'`, `clickRuns?: boolean`; slot `row({ item, selected, index })`; emits `update:selectedId`, `activate(item)`, `context(item, event)` | Scrollable grouped list in `ScrollArea`. Draws `SectionHeader` per titled group, keeps the selected row in view. |
| `ItemGrid` | `groups: SelGroup<T>[]`, `selectedId`, `minCol?: number` or `columns?: number`, `gap?: number` (12); slot `tile({ item, selected })`; same emits; exposes `columns: Ref<number>` | CSS grid (`repeat(auto-fill, minmax(minCol, 1fr))` or fixed columns). Reports the live column count for 2D arrow movement. |
| `ListRow` | `icon?: IconRef`, `title: string`, `subtitle?: string`, `status?: StatusKind`, `selected: boolean`, `size?: 'md' \| 'lg'`, `digit?: number`, `dimmed?: boolean`; slot `trailing` | 40 or 56 px row: 20 px icon, 14 px title, 13 px muted subtitle (same line in `md`, second line in `lg`), trailing slot, `StatusDot`, Alt digit `KeyHint` while Alt is held. Radius 8, 8 px side padding, 12 px gap. |
| `Tile` | `selected: boolean`, `dimmed?: boolean`, `ring?: 'none' \| 'success'`; default slot | Grid card frame: `--surface-2`, radius 8, 1 px `--border`, selection outline. |
| `SectionHeader` | `title: string`, `count?: number` | 28 px group header, 12 px / 500, `--muted-foreground`, 8 px left padding. |
| `EmptyState` | `icon: string`, `title: string`, `hint?: string`, `actionLabel?: string`, `actionKeys?: string`; emits `action` | Centred: 32 px icon, 20 px title, 13 px hint, one `Button` with its `KeyHint`. |
| `ErrorRow` | `message: string`, `retryLabel?: string`; emits `retry` | Inline 40 px row: `triangle-alert` in `--status-failed`, message, Retry button. Used instead of toasts. |
| `KeyHint` | `keys: string` (combo such as `'Ctrl+Enter'`), `label?: string` | Keycaps via `formatCombo`. Cap: min 20x20, padding 0 5, radius 4, `--surface-2`, 1 px `--border`, 12 px / 500 `--muted-foreground`. Glyphs: Enter `↵`, Backspace `⌫`, Tab `⇥`, arrows `↑ ↓ ← →`; text for Ctrl, Shift, Alt, Esc, F-keys. |
| `StatusDot` | `status: StatusKind`, `size?: 12 \| 16`, `label?: string` | Glyph + colour from 5.4. `label` becomes `aria-label` and the tooltip. |
| `RIcon` | `icon: IconRef`, `size?: number` (20) | Renders `lucide` (via `icon-map`), `brand` (simple-icons path), `url` (`img`, lazy, async decode) or `letter` (rounded tile, initial, tinted by name hash). |
| `IconStack` | `icons: IconRef[]`, `max?: number` (4), `size?: number` (16) | Row of small icons with "+n". |
| `LaunchChecklist` | `steps: LaunchStep[]` | One 32 px line per step: icon, label, `StatusDot`, error text when failed. |
| `Meter` | `value: number` (0..1), `tone?: 'neutral' \| 'primary'`, `width?: number` | Thin 4 px share bar (plain element, not `Progress`). |
| `RelTime` | `iso: ISODateTime`, `short?: boolean` | "2m", "3h", "yesterday" via `lib/time.ts`; updates from `useNow`. |
| `PathText` | `path: string` | Mono 12 px, middle-truncated, `.selectable`. |

`StatusKind = Status | 'stale' | 'paused' | 'queued'`.

shadcn-vue components to generate up front: `button`, `input`, `label`, `command`, `dialog`, `alert-dialog`, `popover`, `tooltip`, `badge`, `scroll-area`, `skeleton`, `separator`, `switch`, `select`, `toggle-group` (with `toggle`), `progress`, `checkbox`, `breadcrumb`. Nobody else runs the shadcn-vue CLI.

### 6.2 Composables and contracts

```ts
// src/composables/useScreen.ts — the one contract between a screen and the shell
export type Dir = 'up' | 'down' | 'left' | 'right'
export interface ScreenController {
  target: Ref<ActionTarget | null>          // selected item: feeds ActionBar, Ctrl+K and chords
  summary?: Ref<string>                     // action bar, left side
  move(dir: Dir): boolean                   // false = not handled (lets Left/Right reach the caret)
  page?(dir: -1 | 1): void
  group?(dir: -1 | 1): void
  subView?: { ids: readonly string[]; current: Ref<string> }   // Tab cycles
  stepOut?(): boolean                       // Backspace on empty query; false = not handled
  digits?: Ref<ActionTarget[]>              // Alt+1..9 targets (Home only)
  primaryOverride?: Ref<RainAction[] | null>// Game mode: replaces registry actions
}
export function useScreen(controller: ScreenController): void   // call once in each screen's setup

// src/composables/useSelection.ts
export interface SelGroup<T> { id: string; title?: string; count?: number; items: T[] }
export function useSelection<T extends { id: string }>(
  groups: Ref<SelGroup<T>[]>,
  opts?: { columns?: Ref<number>; initial?: (all: T[]) => string | undefined; persistKey?: string },
): { selectedId: Ref<string | null>; selected: ComputedRef<T | null>;
     move(dir: Dir): boolean; page(dir: -1 | 1): void; group(dir: -1 | 1): void; select(id: string): void }
// Lists: left/right return false. Grids: 2D within a group, up/down cross groups keeping the column.
// If the selected id disappears (filter, reload) the nearest remaining item is selected.
// persistKey stores the id in useViewState so it survives a hide of less than resetAfterSeconds.

// src/composables/useResource.ts — cached async state shared by Home and the sections
export interface Resource<T> { data: Ref<T | undefined>; loading: Ref<boolean>; showSkeleton: Ref<boolean>; error: Ref<RainError | null>; reload(): Promise<void> }
export function useResource<T>(key: string, fetcher: () => Promise<T>, opts?: { live?: Topic }): Resource<T>
export function resetResources(): void    // scenario change, reset to Home

// others
useNav(); useSearch(); useSectionQuery(name)
useViewState<T extends object>(key: string, initial: T): T   // reactive, cleared on reset to Home
useNow(stepMs = 30_000): Ref<number>                          // ticks only while visible
useNotice(): { notice: Ref<Notice | null>; info(t: string): void; success(t: string): void; error(t: string): void }
useOverlay(open: Ref<boolean>): void                          // registers an overlay; restores search focus on close
useActionPanel(); useLogDialog(); useConfirm()
useLaunch(): { runs: Ref<Record<ID, LaunchRun>>; launch(target: LaunchTarget): Promise<LaunchRun> }  // holds the window open, hides on success, notices on failure
```

Resource keys (shared cache): `apps`, `pools`, `projects`, `games`, `sessions`, `jobs`, `idleJobs`, `machine`, `habits`, `habitDays:<from>:<to>`, `events:<from>:<to>`, `vault`, `drives`, `settings`, `pinned`, `recent`.

### 6.3 Shell interface

```ts
// src/shell/shell.ts
export interface Shell {
  readonly kind: 'tauri' | 'browser'
  readonly hidden: Readonly<Ref<boolean>>
  readonly keepOpen: Ref<boolean>               // F10
  show(): Promise<void>
  hide(): Promise<void>
  toggleMaximize(): Promise<void>
  holdOpen(reason: string): () => void          // suspends hide on blur until the returned function is called
  onSummon(cb: () => void): () => void
  onHide(cb: () => void): () => void
}
export const shell: Shell                       // src/shell/index.ts picks TauriShell when isTauri(), else BrowserShell
```

### 6.4 Action registry

```ts
// src/actions/types.ts
export type ItemType = 'app' | 'pool' | 'project' | 'game' | 'session' | 'job' | 'idleJob' | 'command'
                     | 'day' | 'drive' | 'diskNode' | 'setting'
export type ActionTarget =
  | { type: 'app'; id: ID; item: AppEntry }       | { type: 'pool'; id: ID; item: Pool }
  | { type: 'project'; id: ID; item: Project }    | { type: 'game'; id: ID; item: Game }
  | { type: 'session'; id: ID; item: ClaudeSession } | { type: 'job'; id: ID; item: ScheduledJob }
  | { type: 'idleJob'; id: ID; item: IdleJob }    | { type: 'command'; id: ID; item: CommandDef }
  | { type: 'day'; id: ID; item: HabitDay }       | { type: 'drive'; id: ID; item: Drive }
  | { type: 'diskNode'; id: ID; item: DiskNode }  | { type: 'setting'; id: ID; item: SettingDef }
export interface ActionContext {
  where: 'home' | 'section'
  api: RainApi; nav: ReturnType<typeof useNav>; shell: Shell
  notice: ReturnType<typeof useNotice>; confirm: ReturnType<typeof useConfirm>
  launch: ReturnType<typeof useLaunch>['launch']; openLog: ReturnType<typeof useLogDialog>['open']
}
export interface RainAction {
  id: string; label: string; icon: string          // lucide name
  slot: 'primary' | 'secondary' | 'more'
  keys?: string                                    // extra chord, e.g. 'Ctrl+KeyY'
  danger?: boolean
  run(ctx: ActionContext): void | Promise<void>
}
export type ActionProvider<T extends ActionTarget = ActionTarget> = (target: T, ctx: ActionContext) => RainAction[]

// src/actions/registry.ts
export function actionsFor(target: ActionTarget, ctx: ActionContext): RainAction[]
// exactly one 'primary' per target; at most one 'secondary'. Enter runs primary, Ctrl+Enter secondary,
// an action's `keys` chord is live while its target is selected. ActionBar, ActionPanel and CheatSheet read this.
```

Each feature folder owns one `actions.ts` exporting its `ActionProvider`s. `registry.ts` imports them by fixed path and is not edited after the foundation phase.

`CommandDef { id: string; title: string; icon: string; keywords: string[]; keys?: string; run(ctx: ActionContext): void }`, defined in `src/actions/commands.ts`: Go to each of the 9 routes, Game mode, Open today's note, Scan drive C:, New pool, Shortcut sheet, Keep open, and the prototype commands (Simulate permission request, Mock scenario: Typical / Empty / Busy / Failing, Reset mock data). Commands are scored client-side with `lib/score.ts` and merged into the result list.

---

## 7. Screen specs

Shared states for every screen:
- **Loading:** header loader line at once; `Skeleton` in the shape of the content after 150 ms.
- **Empty:** `EmptyState` with the section icon, one line, one button.
- **Error:** `ErrorRow` at the top of the list or pane ("<Source> unavailable", Retry). Never a toast, never a blank screen.
- **Selected:** `--selected` fill on rows, 2 px `--ring` outline on tiles.
- **Running:** `StatusDot` running. Multi-step launches show `LaunchChecklist`.
- The first item is selected when a screen opens unless stated otherwise.

### 7.1 Home and launcher (`features/home/HomeScreen.vue`)

Layout: `ScreenFrame pane="md"`. Main 884 px: `ItemList` (`clickRuns`), 8 px padding. Pane 340 px: `TodayPanel` (read-only, not in the cursor path).

**Empty query: four groups, never more than 14 rows (688 px, fits 704).**

| Group | Content | Cap | Alt digits |
|---|---|---|---|
| Needs you (absent when empty) | Sessions waiting on you, then jobs whose last run failed. Header shows the full count. | 3 | none |
| Pinned | Pinned pools first, then other pinned items, in pin order | 5 | 1..5 |
| Projects | Top 3 by `activityScore`, not already pinned | 3 | next |
| Recent | Most recently launched, not already listed | 3 | up to 9 in total |

- The cursor starts on the first Pinned row on every summon (first Projects row if nothing is pinned). Needs you is one Up away. Enter after a bare summon therefore never changes meaning.
- Row (`ListRow md`): `RIcon` 20, title, subtitle, then trailing: `StatusDot` when relevant, type icon 16 px `--text-3`, digit while Alt is held.
  - Type icons: app `app-window`, pool `layers`, project `folder-git-2`, game `gamepad-2`, session `bot`, job `calendar-clock`, idle job `cpu`, command `square-terminal`.
  - Subtitles: waiting session shows the request on one line ("Bash: npm install"); failed job shows "Failed 06:00, exit 1"; project shows its branch; a pool shows an `IconStack` of its members in the trailing slot instead. Other rows have no subtitle.
- Action bar summary: the item's one-line detail (path, schedule, playtime).

**With a query (no chip): one flat ranked list.**
- Sources: `api.search(query)` plus commands scored locally, merged by `score`, top 50. No groups, no type filter. The first row is selected. Alt+1..9 number the first nine rows.
- Subtitle only when two titles collide (project path, game source).
- No match: `EmptyState` `search-x` "No match", no button.
- A failed source appears as one `ErrorRow` at the end ("Steam library unavailable", Retry).

**Launching from Home:** the row's `StatusDot` runs, the action bar shows "Launching Web dev 2/3", Rain hides on success. On failure the row shows failed and the notice names the step ("Capture tool: executable not found"); Enter retries.

**TodayPanel** (`features/home/TodayPanel.vue`), 16 px padding, three blocks divided by `Separator`:
1. Header: "Today" 16/600 and the date "Sat 21 Nov" 12 px muted.
2. Habits: one row of 28x28 tiles, one per habit (max 7). Done: icon `--foreground` on `--heat-3`. Not yet: icon `--muted-foreground` on `--surface-2`. Skip: a dash. Tooltip: name and value. Click goes to Life.
3. Agenda: next 4 events from now. Row 32 px: 3 px calendar colour bar, time (12 px mono), title (13 px). The current event has a `--hover` fill. Calendar not connected: one line "Calendar not connected" linking to Settings.
4. Bottom: `Button variant="outline"` "Today's note" with `notebook-pen` and `KeyHint` Ctrl T.

Keys: section 4.3. Home has no chip, no sub-views.

### 7.2 Apps and pools (`features/apps/AppsScreen.vue`)

Layout: `ScreenFrame pane="md"`. Main: `ItemList` with two groups, "Pools" then "Apps" (A to Z). Pane: `PoolDetail` or `AppDetail`.

- Pool row: `RIcon`, name, trailing `IconStack` (members, max 4), pin glyph (`pin`, 12 px) when pinned.
- App row: `RIcon`, name, trailing 12 px `StatusDot success` when running, pin glyph when pinned.
- `PoolDetail`: name, "Last launched" `RelTime`, members as 40 px rows (app icon, name, args in mono muted, delay as "+2 s" when set). `actions` slot: `Button` "Launch all" + `KeyHint` Enter, `Button variant="ghost"` "Edit" + `KeyHint` Ctrl Enter.
  - While launching, the member list is replaced by `LaunchChecklist`. A partial failure stays until the next launch or a selection change, with a "Retry failed" button.
- `AppDetail`: 32 px icon, name, `PathText` exe path, running state, "In pools" (names). Buttons: Launch, Open file location.
- `PoolEditorDialog` (`features/apps/PoolEditorDialog.vue`, `Dialog` 480 px): name `Input`, then a `Command` with `CommandInput` and one `CommandItem` + `Checkbox` per app, checked members first. Footer: Cancel, Save (Ctrl+Enter). Opened by Ctrl+N (new) and "Edit pool".
- Delete pool: Ctrl+K, then `ConfirmDialog`.
- Empty: no pools shows one dashed 40 px row "New pool" + `KeyHint` Ctrl N in the Pools group. No apps at all: `EmptyState` `layout-grid` "No apps found", button "Open settings".
- Filtering (chip): matches app and pool names and keywords.

### 7.3 Projects (`features/projects/ProjectsScreen.vue`)

Layout: `ScreenFrame pane="md"`. Main (852 px inside 16 px padding): two `ItemGrid` groups in one selection.

- Group "Most active": 3 fixed columns, cards `aspect-ratio: 16 / 9` (about 276x155). Top 3 by `activityScore`.
- Group "All projects": 5 fixed columns, cards 88 px high (about 161 wide), ordered by `lastOpenedAt` descending.
- Large card (`ProjectCard size="lg"`, in a `Tile`, 12 px padding): letter tile 32 + name 16/600; `git-branch` 12 px + branch in mono 12 px; `RelTime short` top-right; one `IconStack` (environment tools); commit strip of 14 bars along the bottom (plain elements, 24 px max height, `--heat-2`, today's bar `--heat-4`); `StatusDot` top-right corner when a Claude session lives there (the session's status).
- Small card (`size="sm"`): letter tile 24 + name 14/500 on line one; branch mono 12 px + `RelTime short` on line two.
- Open project: 8 px `--status-success` dot after the name; Enter reads "Focus".
- Missing folder: card at 50% opacity with `folder-x`; Enter reads "Locate" and goes to Settings.
- Pane `ProjectDetail`: name, `PathText`, branch, "3 changed files", the environment as rows (`EnvStep`: icon, label, command or URL in mono), sessions here (`StatusDot` + title, click goes to Workflows). Buttons: "Open" + Enter, "Editor only" + Ctrl Enter. During a launch the environment rows become `LaunchChecklist`.
- Empty: `EmptyState` `folder-git-2` "No projects yet", hint "Add a project folder in Settings", button "Open settings".
- Arrows move in 2D; Up from the first small row goes to the large card above the same column position (column index scaled 5 to 3).

### 7.4 Games (`features/games/GamesScreen.vue`)

Layout: `ScreenFrame pane="md"` with a strip.

- Strip (`ViewStrip`): All (`library`), Steam (brand), Epic (brand), Custom (`box`), Installed (`hard-drive-download`), each with a count. A source that failed to load shows `warn` (a `triangle-alert` 12 px in `--status-waiting`). Tab cycles.
- Main: `ItemGrid` `minCol=132`, gap 12 (6 columns at default size). Groups: "Recent" (the last-played games that fit one row, by the live column count), then "Library" A to Z. Hidden games are left out.
- Tile (`GameTile`): cover `aspect-ratio: 2 / 3`, radius 8, `object-fit: cover`, `loading="lazy" decoding="async"`; source badge 20 px bottom-left on a `--surface-1` disc; title on one 12 px line under the cover, truncated.
  - Not installed: cover at 45% opacity with a centred `download` 20 px.
  - Running: `Tile ring="success"`.
  - No art: `--surface-2` tile with the title centred, 14/500.
- Pane `GameDetail`: cover 200 px wide, title 16/600, source, last played `RelTime`, playtime "42 h", size "38 GB", installed state. Buttons: "Play" (or "Install") + Enter, "Play in game mode" + Ctrl Enter.
- `CustomGameDialog` (Ctrl+N): name, exe path, cover path `Input`s. Mock only stores it.
- Source error: `ErrorRow` above the grid ("Epic library unavailable", Retry). The other sources still show.
- Empty: `EmptyState` `gamepad-2` "No games found", button "Open settings".
- "Play in game mode" calls `api.startGameMode({ thenLaunchGameId })` and goes to `/game-mode`.

### 7.5 Workflows (`features/workflows/WorkflowsScreen.vue`)

Layout: `ScreenFrame pane="wide"` with a strip. Strip: Sessions (`bot`), Jobs (`calendar-clock`), Idle (`cpu`), each with a count; Sessions shows the waiting count in `--status-waiting`. Each sub-view is its own component and folder.

**(a) Sessions (`sessions/SessionsView.vue`)**

- `ItemList rowSize="lg"`, groups in fixed order that never collapse and are omitted when empty: Waiting on you, Failed, Working, Done, Idle.
- On open, the first waiting session is selected, otherwise the first row.
- Row: `StatusDot` 16; title 14 px; line two: folder name + branch in mono 12 px (middle-truncated); trailing `RelTime short` of `lastActivityAt`.
  - Waiting: line two is the request on one line in `--foreground` ("Bash: npm install"), and the row has a 2 px `--status-waiting` bar on its left edge.
  - Stale (running and silent for 60 s or more): `StatusDot stale`, tooltip "No activity for 3 m".
- Pane `SessionDetail`: title; `PathText` cwd; branch, model, started `RelTime`, tokens ("48k"); last message (13 px, 3 lines max); then:
  - **Request card** (waiting only): `Badge` with the tool, the exact command or diff in a mono block on `--surface-0` (max height 220, scrolls, `.selectable`), and three buttons in a row: "Allow once" (primary) + `KeyHint` Ctrl Y, "Always allow" (outline, no keycap), "Deny" (ghost) + `KeyHint` Ctrl D.
  - Last 8 events, one line each: time (mono 12 px), kind icon 12 px, text (13 px, truncated).
  - `actions` slot: "Focus terminal" + Enter, "Transcript" + `KeyHint` Ctrl L.
- After an answer the selection moves to the next waiting session and a notice reads "Allowed, 1 waiting". Five blocked sessions take five Ctrl+Y presses, each with its command on screen.
- Stop session: Ctrl+K, then `ConfirmDialog`.
- Empty: `EmptyState` `bot` "No sessions", hint "Sessions started in a terminal appear here". No button.

Why this beats the default app: waiting sessions are always first and also on Home; they are answered in place; the next one is selected automatically; groups never collapse; silent sessions are marked stale instead of showing a stuck spinner; the rail and the header show the count on every screen.

**(b) Jobs (`jobs/JobsView.vue`)**

- `ItemList` with one untitled group and a 28 px column header row (12 px muted): Name, Schedule, Next, Last 10.
- Order: last run failed, then running, then by `nextRunAt`; disabled jobs last.
- Row (grid `24px minmax(0,1fr) 168px 88px 104px 24px`): `StatusDot` (running, else last result, else idle); name + kind icon 12 px (`clock` cron, `bot` Claude); `scheduleLabel` 13 px muted ("Daily 06:00"); next run `RelTime` ("in 9 h"); run strip: the last 10 runs as 8x8 px squares, 2 px gap, radius 2 (success `--status-success`, failed `--status-failed`, running `--status-running`, none `--surface-2`); `circle-pause` when disabled. Disabled rows are at 60% opacity.
- Pane `JobDetail`: name, cron in mono, schedule label, "Enabled" `Switch` (click, or Ctrl+Enter), next run, then the last 20 runs as 32 px rows (status square, start time, duration, exit code). Buttons: "Runs and log" + Enter, "Run now" + `KeyHint` Ctrl R.
- Enter opens `LogDialog` with the job's runs on the left and the most recent failed run selected (else the latest). The log loads on demand with `api.getRunLog(runId)`.
- Run now: the row shows running for the mock duration, then a new run is added.
- Empty: `EmptyState` `calendar-clock` "No scheduled jobs".

**(c) Idle (`idle/IdleView.vue`)**

- A 40 px status line above the list: `monitor` icon + "PC active. Idle jobs start after 10 min idle." (13 px muted) from `MachineState`.
- `ItemList rowSize="lg"`. Row: `StatusDot` (state); name; line two: state in words + ETA ("Running, 12 min left", "Queued", "Paused"); trailing resource icon (`cpu` or `gpu`) and a `Progress` 160 px wide.
- Pane `IdleJobDetail`: name, trigger ("After 10 min idle"), resource, last run, `Progress`, "Enabled" `Switch`. Buttons: "Log" + Enter, "Run now" + `KeyHint` Ctrl R.
- Empty: `EmptyState` `cpu` "No idle jobs".

### 7.6 Life (`features/life/LifeScreen.vue`)

Layout: `ScreenFrame pane="md"` with a strip. Rain only reads; there is no editing anywhere on this screen.

- Strip: Month (`calendar`), Year (`calendar-range`). `end` slot: `chevron-left`, period label ("November 2026" or "2026", 13 px), `chevron-right`, "Today" ghost button, and a `lock` icon whose tooltip reads "Rain reads your vault and never writes to it".
- **Month view (`MonthHeatmap.vue`):** one row per habit. Grid `132px repeat(<days>, minmax(0, 20px)) 40px 40px`, gap 3: habit icon 16 + name 13 px; one square cell per day; streak ("5"); completion ("82%"). A header row shows day numbers (12 px, `--text-3`, every day; weekends in `--muted-foreground`).
- **Year view (`YearHeatmap.vue`):** habit chips above the grid (`ToggleGroup type="single"`: "All" + one chip per habit with icon and name, 12 px). Grid: 53 week columns x 7 rows, `grid-auto-flow: column`, 12 px cells, 3 px gap (820 px wide with the weekday labels). Month labels above, M / W / F labels on the left, 12 px `--text-3`. "All" colours by completion ratio.
- Cells (radius 3):
  - Value: `--heat-1..4` (boolean true = `--heat-3`; numbers scaled by `value / target`, clamped; four steps).
  - Miss (note exists, not done): `--heat-0`.
  - Skip: `--heat-0` with a centred 6x2 px dash in `--text-3`.
  - No note that day: transparent with a 1 px `--selected` border (hollow).
  - Today: 1 px `--foreground` outline. Selected: 2 px `--ring` outline.
  - Future days are not drawn.
- Cursor: arrows move the day cell in 2D (Month: Up/Down change habit, Left/Right change day and roll into the next month). PageUp/PageDown: previous / next month or year. Ctrl+Up/Down in Year: previous / next habit chip. The cursor starts on today.
- Action bar summary reads out the cell: "Tue 3 Nov, Reading 30 min, streak 5". Nothing depends on hover.
- Typing with the chip filters habits by name (Month: rows; Year: selects the best match).
- Pane `DayPanel`: the selected date 16/600; each habit's value as a row (icon, name, value or "skipped" or a dash); "No note for this day" when `noteExists` is false; `Button variant="outline"` "Today's note" + `KeyHint` Ctrl T; `Separator`; `Agenda` (read-only): groups Today and Tomorrow, rows 40 px (3 px colour bar, time 12 px mono, title 13 px, location 12 px muted), current event with `--hover` fill.
- Enter on a day with no note shows the notice "No note for this day". Rain never creates one.
- States: vault not found: the main area is `EmptyState` `calendar-check` "Choose your vault", button "Open settings"; the agenda still shows. Calendar not connected: `EmptyState` inside the agenda only ("Connect calendar"). A habit whose property is missing from the notes: its Month row shows `triangle-alert` 12 px and the tooltip "Property 'meditate' not found in notes".

### 7.7 Disk (`features/disk/DiskScreen.vue`, loaded with `defineAsyncComponent`)

`d3-hierarchy` is imported only inside this folder. Nothing is read until a scan is started.

- **Before a scan:** a centred 560 px column, `ItemList rowSize="lg"` of drives. Row: `hard-drive` 20, "C: System" 14 px, line two "412 GB of 931 GB used", trailing `Progress` 160 px. Enter = Scan.
- **Scanning:** the same column shows the drive row, a `Progress` (determinate), "184,203 files" and a ghost button "Cancel scan" + `KeyHint` Ctrl Enter. Cancel is an explicit action; Esc keeps its meaning. Hide on blur is held.
- **Result:** `ScreenFrame pane="none"` with a strip holding a `Breadcrumb` (drive, then folders; click to jump) and, in `end`, total size.
  - Left, 360 px, right border: `ItemList` of the current folder's children by size. Row 32 px: `folder` or file-category icon 16, name, size (12 px mono), `Meter` 48 px (share of the parent).
  - Right: `TreemapCanvas.vue`, one `<canvas>` filling the space (864x640 at default size), background `--surface-0`. Layout: `d3.treemap()` squarified, padding 1, depth cap 4 below the current folder. Rectangles under 2 px are not drawn. Rectangles of at least 80x24 px carry a 12 px label. Fill = file category colour; folders are drawn as 1 px `--border-strong` frames.
  - Legend: a 24 px row under the map, 8 swatches (10x10, radius 2) with 12 px labels.
  - One selection: Up/Down move in the list and the matching rectangle gets a 2 px `--foreground` outline. Clicking a rectangle selects its top-level ancestor in the list; double-click opens that folder. Hit-testing walks the hierarchy from the pointer; no per-rectangle listeners.
  - Redraw on resize through one `ResizeObserver`, debounced 100 ms.
- Keys: Enter opens the selected folder; Backspace on an empty query goes up; Ctrl+Enter reveals in Explorer; Ctrl+R rescans. Typing with the chip filters the list.
- Summary: full path, size, share ("C:\Dev\node_modules, 4.1 GB, 12% of Dev").
- No delete in the prototype.
- Scan failed: `ErrorRow` under the drive ("Access denied", Retry).
- The tree is kept in a module-level cache for 5 minutes after leaving Disk (one-shot timer), then dropped.

### 7.8 Game mode (`features/game-mode/GameModeScreen.vue`)

Takes the content area; a centred 480 px column. Ctrl+G (or the rail icon, or "Play in game mode") goes here and calls `api.startGameMode()`. The search field is disabled while the phase is `review`, `countdown` or `windingDown`. Hide on blur is held. Leaving the screen with Ctrl+digit cancels, with the notice "Game mode cancelled".

State is `GameModeState` from `subscribe('gameMode')`; the screen holds no timer of its own.

| Phase | Shows | Enter | Ctrl+Enter | Esc |
|---|---|---|---|---|
| `review` (only when at least one item has `effect: 'interrupts'`) | `circle-power` 32, "2 things will be interrupted" (20 px), the checklist, buttons | Wait for them | Stop anyway | Cancel |
| `review` with `waiting: true` | Same, heading "Waiting for 2 things to finish", spinner on those rows | (none) | Stop anyway | Cancel |
| `countdown` | "Closing in" + number (`text-countdown`) + "s", a draining `Progress`, the checklist | Close now | (none) | Cancel |
| `windingDown` | "Winding down", each row goes pending, stopping (`StatusDot running`), stopped (`StatusDot success`) | (none) | (none) | (none) |
| `done` | "Rain is off" (20 px), hint "Nothing is left running. Open Rain again from the Start menu.", button "Restart prototype" | Restart prototype | (none) | (none) |

- Checklist row (40 px): icon, label, count, `Badge` "safe" (muted) or "interrupts" (`--status-waiting` tint), `StatusDot` during wind-down. Items: Claude sessions, scheduled jobs, idle jobs, background service, Rain window.
- With nothing to interrupt, `review` is skipped and the countdown starts at once.
- Started from a game: a last line "Then launch <title>" with the cover at 24 px.
- Buttons are real `Button`s with their `KeyHint`; the screen passes them to the action bar through `primaryOverride`.
- Countdown length comes from `settings.gameModeCountdownSeconds` (default 10).

### 7.9 Settings (`features/settings/SettingsScreen.vue`)

Layout: `ScreenFrame pane="none"`, one 640 px column, 24 px padding, scrolls. No category navigation, no Save button; changes apply at once. Rows are the selectable items (`SettingRow.vue`, 48 px: label 14 px, optional hint 12 px muted, control on the right). Enter activates the control: a `Switch` toggles, a `Select` opens, an `Input` takes focus (Esc returns to search). Typing with the chip filters rows by label.

| Group | Row | Control |
|---|---|---|
| General | Summon hotkey | `KeyHint` Ctrl ` (read-only), hint "Takes this chord from other apps while Rain runs" |
| General | Hide when focus is lost | `Switch` |
| General | Return to Home after | `Select`: 30 s, 60 s, 5 min |
| General | Start with Windows | `Switch` |
| Sources | Obsidian vault | path `Input` + found (`circle-check`) or missing (`circle-x`) glyph |
| Sources | Daily note format | `Input` (default `YYYY-MM-DD`) |
| Sources | Project folders | path `Input` (semicolon-separated) + glyph |
| Sources | Steam library | `Switch` + path `Input` + glyph |
| Sources | Epic library | `Switch` + path `Input` + glyph |
| Sources | Google Calendar | `Button` "Connect" / "Disconnect" (mock toggles it) |
| Game mode | Countdown | `Select`: 5, 10, 20, 30 s |
| Prototype | Mock scenario | `Select`: Typical, Empty, Busy, Failing |
| Prototype | Simulated latency | `Switch` |
| Prototype | Reset mock data | `Button` |
| About | Rain | version text + `Badge` "Mock data" |

Group headers are `SectionHeader`s with a `Separator` above. `SettingDef { id: string; group: string; label: string; hint?: string; kind: 'switch' | 'select' | 'path' | 'text' | 'button' | 'info'; key?: keyof Settings }`.

---

## 8. Mock data layer

### 8.1 Files

```
src/data/
  types.ts            all interfaces below
  api.ts              RainApi, Topic, TopicPayload, RainError
  index.ts            export const api: RainApi = createMockApi()   (later: createRestApi(baseUrl))
  mock/
    index.ts          createMockApi(): assembles the slices below into one RainApi
    engine.ts         delay(), fail(), emit()/subscribe bus, seeded PRNG (mulberry32), ago()/fromNow(), setPaused()
    control.ts        prototype-only controls (not part of RainApi)
    search.ts         scorer over all seeded items
    covers.ts         generated abstract SVG covers (data URIs) from a title hash; no real artwork
    logs.ts           generated LogLine[] per run or session
    seed/
      apps.ts  projects.ts  games.ts  workflows.ts  life.ts  disk.ts  gameMode.ts  settings.ts
```

Each seed file exports `seed(scenario: Scenario): <Entities>`. Seeds use `ago()` so relative times stay plausible on any day; habit days and the disk tree come from the seeded PRNG so they are the same on every load. IDs are stable strings (`app-vscode`, `pool-web-dev`, `proj-weather-station`).

```ts
// src/data/mock/control.ts
export type Scenario = 'typical' | 'empty' | 'busy' | 'failing'
export interface PrototypeOptions { scenario: Scenario; latency: boolean }
export const mockControl: {
  options: Readonly<PrototypeOptions>            // reactive, persisted in localStorage('rain.prototype')
  setScenario(s: Scenario): void                 // reseeds, then resetResources()
  setLatency(on: boolean): void
  simulatePermissionRequest(): void              // moves one working session to waiting
  reset(): void
  setPaused(paused: boolean): void               // AppShell calls this with shell.hidden
}
```

Timing with latency on: list and get calls 150 to 400 ms; `search` 0 to 30 ms; each launch step 400 to 900 ms; a scan takes 4 s with progress every 200 ms; game-mode tick 1 s; each wind-down item 500 ms. With latency off: 0 ms for lists, the rest unchanged. While visible, every 20 s one working session gets a new event. All mock tickers stop when paused.

### 8.2 Service interface

Every method returns a Promise and maps to one REST call. Rejections are `RainError`.

```ts
// src/data/api.ts
export interface RainError { code: 'unavailable' | 'notFound' | 'failed'; message: string; source?: string }

export interface TopicPayload {
  sessions: ClaudeSession[]
  jobs: ScheduledJob[]
  idleJobs: IdleJob[]
  launch: LaunchRun
  scan: DiskScan
  gameMode: GameModeState
}
export type Topic = keyof TopicPayload

export interface RainApi {
  // search and pins
  search(query: string, types?: ItemType[]): Promise<SearchItem[]>            // GET  /search?q=&types=
  listPinned(): Promise<SearchItem[]>                                          // GET  /pinned
  listRecent(limit: number): Promise<SearchItem[]>                             // GET  /recent?limit=
  setPinned(type: PinnableType, id: ID, pinned: boolean): Promise<void>        // PUT  /pinned/:type/:id

  // apps and pools
  listApps(): Promise<AppEntry[]>                                              // GET  /apps
  listPools(): Promise<Pool[]>                                                 // GET  /pools
  savePool(draft: PoolDraft): Promise<Pool>                                    // POST /pools | PUT /pools/:id
  deletePool(id: ID): Promise<void>                                            // DELETE /pools/:id

  // launching (apps, pools, projects, games)
  launch(target: LaunchTarget): Promise<LaunchRun>                             // POST /launches
  getLaunch(id: ID): Promise<LaunchRun>                                        // GET  /launches/:id

  // projects and games
  listProjects(): Promise<Project[]>                                           // GET  /projects
  listGames(): Promise<GameLibrary>                                            // GET  /games
  saveCustomGame(draft: CustomGameDraft): Promise<Game>                        // POST /games
  setGameHidden(id: ID, hidden: boolean): Promise<void>                        // PATCH /games/:id

  // Claude sessions
  listSessions(): Promise<ClaudeSession[]>                                     // GET  /sessions
  answerPermission(sessionId: ID, answer: PermissionAnswer): Promise<ClaudeSession> // POST /sessions/:id/permission
  focusSession(id: ID): Promise<void>                                          // POST /sessions/:id/focus
  stopSession(id: ID): Promise<void>                                           // POST /sessions/:id/stop
  getSessionTranscript(id: ID): Promise<LogLine[]>                             // GET  /sessions/:id/transcript

  // scheduled jobs
  listJobs(): Promise<ScheduledJob[]>                                          // GET  /jobs
  setJobEnabled(id: ID, enabled: boolean): Promise<ScheduledJob>               // PATCH /jobs/:id
  runJob(id: ID): Promise<JobRun>                                              // POST /jobs/:id/runs
  getRunLog(runId: ID): Promise<LogLine[]>                                     // GET  /runs/:id/log

  // idle jobs
  listIdleJobs(): Promise<IdleJob[]>                                           // GET  /idle-jobs
  setIdleJobEnabled(id: ID, enabled: boolean): Promise<IdleJob>                // PATCH /idle-jobs/:id
  runIdleJob(id: ID): Promise<IdleJob>                                         // POST /idle-jobs/:id/run
  getIdleJobLog(id: ID): Promise<LogLine[]>                                    // GET  /idle-jobs/:id/log
  getMachineState(): Promise<MachineState>                                     // GET  /machine

  // life (read-only)
  getVaultStatus(): Promise<VaultStatus>                                       // GET  /vault
  listHabits(): Promise<Habit[]>                                               // GET  /habits
  listHabitDays(from: ISODate, to: ISODate): Promise<HabitDay[]>               // GET  /habit-days?from=&to=
  listEvents(from: ISODate, to: ISODate): Promise<CalendarEvent[]>             // GET  /events?from=&to=
  openDailyNote(date: ISODate): Promise<{ opened: boolean }>                   // POST /daily-notes/:date/open

  // disk
  listDrives(): Promise<Drive[]>                                               // GET  /drives
  startScan(driveId: ID): Promise<DiskScan>                                    // POST /scans
  getScan(id: ID): Promise<DiskScan>                                           // GET  /scans/:id
  cancelScan(id: ID): Promise<void>                                            // DELETE /scans/:id
  revealPath(path: string): Promise<void>                                      // POST /shell/reveal

  // game mode
  getGameMode(): Promise<GameModeState>                                        // GET  /game-mode
  startGameMode(opts?: { thenLaunchGameId?: ID }): Promise<GameModeState>      // POST /game-mode
  resolveGameMode(choice: GameModeChoice): Promise<GameModeState>              // POST /game-mode/choice

  // settings
  getSettings(): Promise<Settings>                                             // GET  /settings
  updateSettings(patch: Partial<Settings>): Promise<Settings>                  // PATCH /settings

  // live updates (later: one WebSocket or SSE stream)
  subscribe<T extends Topic>(topic: T, cb: (payload: TopicPayload[T]) => void): () => void
}
```

### 8.3 Types

```ts
// src/data/types.ts
export type ID = string
export type ISODate = string        // '2026-11-21'
export type ISODateTime = string    // '2026-11-21T09:30:00.000Z'
export type Status = 'idle' | 'running' | 'waiting' | 'success' | 'failed'

export interface IconRef { kind: 'lucide' | 'brand' | 'url' | 'letter'; value: string; tint?: string }

// ---- search ----
export type SearchType = 'app' | 'pool' | 'project' | 'game' | 'session' | 'job' | 'idleJob'
export type PinnableType = 'app' | 'pool' | 'project' | 'game'
export interface SearchItem { type: SearchType; id: ID; title: string; subtitle?: string; icon: IconRef; status?: Status; score: number }

// ---- apps and pools ----
export interface AppEntry { id: ID; name: string; icon: IconRef; exePath: string; keywords: string[];
  pinned: boolean; running: boolean; lastLaunchedAt?: ISODateTime }
export interface PoolMember { appId: ID; args?: string; delayMs?: number }
export interface Pool { id: ID; name: string; icon: IconRef; members: PoolMember[]; pinned: boolean; lastLaunchedAt?: ISODateTime }
export interface PoolDraft { id?: ID; name: string; members: PoolMember[] }

// ---- launching ----
export interface LaunchTarget { type: 'app' | 'pool' | 'project' | 'game'; id: ID; mode?: 'full' | 'editorOnly' }
export interface LaunchStep { label: string; icon: IconRef; status: Status; error?: string }
export interface LaunchRun { id: ID; target: LaunchTarget; status: 'running' | 'success' | 'failed'; steps: LaunchStep[] }

// ---- projects ----
export interface EnvStep { kind: 'editor' | 'terminal' | 'browser' | 'claude' | 'app'; label: string; icon: IconRef;
  appId?: ID; command?: string; url?: string }
export interface Project { id: ID; name: string; path: string; icon: IconRef; branch: string; dirtyFiles: number;
  commits14d: number[]; lastOpenedAt: ISODateTime; activityScore: number; environment: EnvStep[];
  sessionIds: ID[]; open: boolean; pinned: boolean; missing: boolean }

// ---- games ----
export type GameSource = 'steam' | 'epic' | 'custom'
export interface Game { id: ID; title: string; source: GameSource; coverUrl?: string; installed: boolean; running: boolean;
  sizeBytes?: number; playtimeMinutes: number; lastPlayedAt?: ISODateTime; pinned: boolean; hidden: boolean }
export interface GameLibrary { games: Game[]; failedSources: GameSource[] }
export interface CustomGameDraft { title: string; exePath: string; coverPath?: string }

// ---- Claude sessions ----
export type PermissionAnswer = 'once' | 'always' | 'deny'
export interface PermissionRequest { id: ID; tool: string; summary: string; detail: string; requestedAt: ISODateTime }
//   summary: one line for rows ('npm install'); detail: the exact command or diff for the pane
export interface SessionEvent { at: ISODateTime; kind: 'prompt' | 'tool' | 'edit' | 'message' | 'permission' | 'error'; text: string }
export interface ClaudeSession { id: ID; title: string; cwd: string; projectId?: ID; branch?: string; model: string;
  status: Status; startedAt: ISODateTime; lastActivityAt: ISODateTime; tokens: number; lastMessage: string;
  request?: PermissionRequest; recent: SessionEvent[] }
//   status: running = working, waiting = waiting on you, success = done, idle, failed
//   stale is derived in the UI: status === 'running' && now - lastActivityAt >= 60 s

// ---- scheduled and idle jobs ----
export interface JobRun { id: ID; jobId: ID; status: Status; startedAt: ISODateTime; durationMs?: number; exitCode?: number }
export interface LogLine { ts: ISODateTime; level: 'info' | 'warn' | 'error'; text: string }
export interface ScheduledJob { id: ID; name: string; kind: 'cron' | 'claude'; cron: string; scheduleLabel: string;
  enabled: boolean; running: boolean; nextRunAt?: ISODateTime; recentRuns: JobRun[] }   // newest first, at most 20
export interface IdleJob { id: ID; name: string; enabled: boolean; state: 'queued' | 'running' | 'paused' | 'success' | 'failed';
  progress: number; resource: 'cpu' | 'gpu'; idleAfterMinutes: number; etaMinutes?: number; lastRunAt?: ISODateTime }
export interface MachineState { userActive: boolean; idleMinutes: number }

// ---- life ----
export interface Habit { id: ID; name: string; icon: IconRef; property: string; kind: 'boolean' | 'number'; target?: number; unit?: string }
export type HabitValue = boolean | number | 'skip' | null          // null = note exists, property not set
export interface HabitDay { date: ISODate; noteExists: boolean; values: Record<ID, HabitValue> }   // one row per daily note
export interface VaultStatus { connected: boolean; path?: string; missingProperties: string[] }
export interface CalendarEvent { id: ID; title: string; start: ISODateTime; end: ISODateTime; allDay: boolean;
  calendar: string; color: string; location?: string }

// ---- disk ----
export type FileCategory = 'video' | 'image' | 'audio' | 'archive' | 'code' | 'document' | 'app' | 'other'
export interface Drive { id: ID; letter: string; label: string; totalBytes: number; usedBytes: number }
export interface DiskNode { name: string; path: string; sizeBytes: number; kind: 'dir' | 'file'; category?: FileCategory; children?: DiskNode[] }
export interface DiskScan { id: ID; driveId: ID; status: 'scanning' | 'done' | 'failed' | 'cancelled';
  progress: number; filesSeen: number; error?: string; root?: DiskNode }

// ---- game mode ----
export type GameModeChoice = 'wait' | 'stopAnyway' | 'closeNow' | 'cancel'
export interface WindDownItem { id: ID; label: string; icon: IconRef; kind: 'session' | 'job' | 'idleJob' | 'service' | 'window';
  count: number; effect: 'safe' | 'interrupts'; status: 'pending' | 'stopping' | 'stopped' }
export interface GameModeState { phase: 'off' | 'review' | 'countdown' | 'windingDown' | 'done';
  waiting: boolean; secondsLeft: number; countdownSeconds: number; items: WindDownItem[]; thenLaunchGameId?: ID }

// ---- settings ----
export interface Settings { hotkey: string; hotkeyAvailable: boolean; hideOnBlur: boolean; resetAfterSeconds: 30 | 60 | 300;
  startWithWindows: boolean; vaultPath: string; dailyNoteFormat: string; projectRoots: string[];
  steamEnabled: boolean; steamPath: string; epicEnabled: boolean; epicPath: string;
  calendarConnected: boolean; gameModeCountdownSeconds: 5 | 10 | 20 | 30 }
```

### 8.4 Sample data amounts

All sample data is generic: common application names, invented project names under `C:\Dev\`, invented game titles with generated covers, a vault at `D:\Vault`, no person's name, account or real path.

| Entity | Typical | Busy | Empty | Notes for Typical |
|---|---|---|---|---|
| Apps | 24 | 150 | 0 | 6 running, 4 pinned; Microsoft apps use letter tiles |
| Pools | 4 | 9 | 0 | 3 pinned ("Web dev", "Writing", "Streaming"); 2 to 4 members; one with args and a delay |
| Projects | 11 | 40 | 0 | 3 clearly most active, 2 open, 1 missing folder, 3 with a live session |
| Games | 28 | 180 | 0 | Steam 16, Epic 8, custom 4; 20 installed; 2 without cover; 6 played in the last week |
| Claude sessions | 7 | 14 (5 waiting) | 0 | 2 waiting (one Bash command, one file edit diff), 2 working (1 stale), 1 done, 1 idle, 1 failed; 8 events each |
| Scheduled jobs | 8 | 30 | 0 | 5 cron, 3 Claude; 1 last-run failed, 1 running, 1 disabled; 20 runs each; logs of 30 to 60 lines generated on demand |
| Idle jobs | 3 | 8 | 0 | 1 queued, 1 paused at 40%, 1 failed |
| Habits | 5 | 7 | 0 (vault not connected) | 3 boolean, 2 numeric with targets; one with a missing property |
| Habit days | 400 | 400 | 0 | Trailing 400 days; about 6% of days without a note; about 3% skips |
| Calendar events | 9 over 3 days | 20 | 0 (not connected) | 2 calendars, 1 all-day, 1 in progress now |
| Drives | 2 | 3 | 2 | C: scans fine; D: always fails with "Access denied" |
| Disk tree | about 1,500 nodes, depth up to 6 | about 8,000 | n/a | Generated; all 8 categories present |
| Wind-down items | 5 | 5 | 5 (all safe) | 2 with `interrupts`, so `review` appears |
| Commands | about 22 | same | same | Defined in `actions/commands.ts`, not in the mock |

One failing item per section in Typical (no setting needed to reach an error state):

| Section | Failing item |
|---|---|
| Home | A failed job in "Needs you"; launching the "Streaming" pool fails on one member |
| Apps | Pool "Streaming": member "Capture tool" fails with "Executable not found" |
| Projects | "old-blog": folder missing |
| Games | Epic source fails on the first load; Retry succeeds |
| Workflows | Session "Migrate database schema" failed; job "Nightly backup" exit 1; idle job "Thumbnail cache" failed |
| Life | Habit "Meditate": property not found in notes |
| Disk | Drive D: scan fails at 40% |
| Game mode | Two items interrupt, so the gate shows |
| Settings | Epic path shows the missing glyph |

Scenario "Failing": `listGames`, `listJobs` and `listEvents` reject with `unavailable`; the vault is not connected; every launch fails on its second step; every scan fails. Retry does not fix them.

---

## 9. File and folder structure, and work split

### 9.1 Tree

```
index.html                      <html class="dark">
vite.config.ts                  @ alias to src, @tailwindcss/vite
components.json                 shadcn-vue config (foundation only)
src/
  main.ts
  App.vue                       renders <AppShell />
  styles/tokens.css             section 5.1
  shell/        shell.ts  index.ts  tauri.ts  browser.ts
  nav/          routes.ts  useNav.ts
  keys/         keymap.ts  useKeymap.ts
  actions/      types.ts  registry.ts  commands.ts  context.ts
  composables/  useScreen.ts  useSelection.ts  useSearch.ts  useResource.ts  useViewState.ts
                useNow.ts  useNotice.ts  useOverlay.ts  useLaunch.ts  useGridColumns.ts
  lib/          utils.ts (cn)  time.ts  format.ts (bytes, minutes, counts)  score.ts  icon-map.ts  brand-icons.ts
  components/
    ui/         generated shadcn-vue components (6.1)
    rain/       AppShell.vue IconRail.vue AppHeader.vue SearchField.vue ActionBar.vue ActionPanel.vue
                CheatSheet.vue LogDialog.vue ConfirmDialog.vue HiddenPlaceholder.vue ScreenFrame.vue
                DetailPane.vue ViewStrip.vue ItemList.vue ItemGrid.vue ListRow.vue Tile.vue SectionHeader.vue
                EmptyState.vue ErrorRow.vue KeyHint.vue StatusDot.vue RIcon.vue IconStack.vue
                LaunchChecklist.vue Meter.vue RelTime.vue PathText.vue
  data/         section 8.1
  features/
    home/       HomeScreen.vue  TodayPanel.vue  ResultRow.vue  actions.ts (none; uses the others)
    apps/       AppsScreen.vue  PoolDetail.vue  AppDetail.vue  PoolEditorDialog.vue  actions.ts
    projects/   ProjectsScreen.vue  ProjectCard.vue  ProjectDetail.vue  actions.ts
    games/      GamesScreen.vue  GameTile.vue  GameDetail.vue  CustomGameDialog.vue  actions.ts
    workflows/  WorkflowsScreen.vue  actions.ts
                sessions/ SessionsView.vue SessionRow.vue SessionDetail.vue RequestCard.vue
                jobs/     JobsView.vue JobRow.vue JobDetail.vue RunStrip.vue
                idle/     IdleView.vue IdleRow.vue IdleJobDetail.vue
    life/       LifeScreen.vue  MonthHeatmap.vue  YearHeatmap.vue  HeatCell.vue  DayPanel.vue  Agenda.vue
                useDayCursor.ts  actions.ts
    disk/       DiskScreen.vue  DriveList.vue  FolderList.vue  TreemapCanvas.vue  treemap.ts  diskCache.ts  actions.ts
    game-mode/  GameModeScreen.vue  WindDownRow.vue
    settings/   SettingsScreen.vue  SettingRow.vue  settingDefs.ts  actions.ts
src-tauri/
  tauri.conf.json  Cargo.toml  capabilities/default.json  src/lib.rs  src/main.rs
```

`lib/icon-map.ts` maps every Lucide name used in data (`IconRef.kind === 'lucide'`, route icons, action icons) to its component, so icons stay tree-shaken. If a name is missing in the installed Lucide version, the foundation owner substitutes the component and keeps the key. Static icons inside a component are imported directly from the icon package.

Dependencies: `vue`, `@lucide/vue`, `reka-ui`, `@vueuse/core`, `class-variance-authority`, `clsx`, `tailwind-merge`, `tw-animate-css`, `tailwindcss`, `@tailwindcss/vite`, `d3-hierarchy` (+ `@types/d3-hierarchy`), `simple-icons`, `@tauri-apps/api`, `@tauri-apps/cli`. Not used: vue-router, pinia, a toast library, a date library, a fuzzy-search library, an animation library, web fonts.

### 9.2 Phase 0: foundation owner (one engineer, before anyone else starts)

In this order:

1. Project scaffold, Tailwind v4, `tokens.css`, shadcn-vue init, generate the components in 6.1, apply the edits in 5.6.
2. `data/types.ts`, `data/api.ts`, the whole `data/mock/` folder including every seed file at the amounts in 8.4, `mock/control.ts`.
3. `shell/` (interface, `BrowserShell`, a `TauriShell` stub), `nav/`, `composables/`, `lib/`.
4. `actions/types.ts`, `registry.ts`, `commands.ts`, and a stub `features/<x>/actions.ts` per feature holding the Enter and Ctrl+Enter actions from 4.3 (so the launcher works across all types from day one).
5. `keys/keymap.ts`, `useKeymap.ts`.
6. Every component in `components/rain/`.
7. A stub `<Name>Screen.vue` in each feature folder (an `EmptyState` with the section icon) and the complete `routes.ts`.

Exit check: in the browser, Ctrl+` toggles the placeholder, Ctrl+1..7 / Ctrl+, / Ctrl+G switch stubs, the rail badge shows the waiting count, typing shows ranked results on Home, Enter "launches" with a notice, Ctrl+K lists actions, F1 lists the keys.

After Phase 0 these are frozen: `routes.ts`, `registry.ts`, `tokens.css`, `components/ui/`, `components/rain/`, `data/types.ts`, `data/api.ts`. A change request goes to the foundation owner.

### 9.3 Phase 1: parallel work (no shared files)

| Owner | Owns (and only edits) | Notes |
|---|---|---|
| A | `features/home/` | Result list, groups and caps, Alt digits, `TodayPanel` |
| B | `features/apps/`, `data/mock/seed/apps.ts` | List, details, pool editor, launch checklist |
| C | `features/projects/`, `data/mock/seed/projects.ts` | Two-size grid, detail, launch |
| D | `features/games/`, `data/mock/seed/games.ts`, `data/mock/covers.ts` | Cover grid, filter strip, detail |
| E | `features/workflows/` (three sub-folders; can be split across up to three people), `data/mock/seed/workflows.ts`, `data/mock/logs.ts` | Sessions, Jobs, Idle |
| F | `features/life/`, `data/mock/seed/life.ts` | Heatmaps, day cursor, agenda |
| G | `features/disk/`, `data/mock/seed/disk.ts` | Lazy chunk, canvas treemap |
| H | `features/game-mode/`, `features/settings/`, `data/mock/seed/gameMode.ts`, `data/mock/seed/settings.ts` | Phase machine screen; settings rows |
| T | `src-tauri/`, `src/shell/tauri.ts` | Hotkey, single instance, window state, hide on blur, capabilities |

Rules for Phase 1:
- A screen talks to the rest of the app only through: `api`, `useResource`, `useScreen`, `useSelection`, `useSectionQuery`, `useNav`, `useNotice`, `useLaunch`, the overlay openers, and `components/rain`.
- A screen never adds a `keydown` listener, never imports from another feature folder, never imports `@tauri-apps/api`, and never starts an interval (use `useNow` or `subscribe`).
- Each screen sets its `target` so the action bar is never empty when something is selected.
- Each owner checks their screen in all four scenarios and at 1040x680, 1280x800 and maximised.

### 9.4 Phase 2: integration (foundation owner)

Walk the ten daily tasks from hidden (launch app, launch pinned pool, open project, play a game, check waiting sessions, approve one, read why a job failed, glance at today's habits, open today's note, scan a drive) in the Tauri build, fix seams, and confirm idle cost (no timers and no animation while hidden; Home after 60 s).

---

## 10. Out of scope for the prototype

- Any backend: real app discovery and launching, icon extraction, Steam/Epic scanning, Claude Code hooks, cron, idle detection, Obsidian parsing, Google Calendar, disk scanning, real game-mode shutdown.
- OS notifications, tray icon, autostart registration, installer, signing, auto-update.
- Hotkey recorder (the hotkey is shown, not changed) and per-pool global hotkeys.
- Light theme, interface scale, rail labels, compact summon, a second window, any programmatic window resizing.
- "Capture open windows" for pools, multi-select, drag and drop, reordering pins.
- Creating or editing jobs and idle jobs; starting or prompting Claude sessions from Rain.
- Writing to the vault or the calendar in any form.
- Deleting or moving files in Disk.
- Web search fallback, calculator, clipboard history or other launcher extras.
- List virtualisation (mock lists are short; add Reka's virtualizer with the real backend).
- Localisation, telemetry, accounts, sync.
- Automated tests beyond a smoke test of the keymap and `useSelection`.
