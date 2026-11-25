# 07 VS Code themes

Module `vscode_themes`. Rain reads the colour themes of the VS Code extensions that are installed
on this PC and turns one of them into its own colour tokens, so that Rain can look like the
owner's editor (he uses Bearded Theme).

## Rules

- **Nothing from a theme file is stored in the repository.** Bearded Theme is GPL-3.0 and Rain is
  MIT. The files are read from `~/.vscode/extensions` when a route is called. The tests use a
  small theme that was made up for them.
- **No state, no thread, no cache.** A route reads the folder (or one file) when it is called and
  forgets it. The window calls it when the Settings list opens, when a theme is picked, and for
  "Follow VS Code" at start, at each summon and when Settings opens. Reading is done with
  `spawn_blocking`.
- **Seam:** `ThemeSource` gives the extensions folder and the settings file. The real one
  (`SystemThemeSource`) uses `%USERPROFILE%\.vscode\extensions` and
  `%APPDATA%\Code\User\settings.json`. Tests put a fake in with `TestBackend::with::<dyn ThemeSource>`.
  Only VS Code itself is looked at (not Insiders or VSCodium).

## Routes (under `/api/v1`)

| Route | Answer |
|---|---|
| `GET /themes/vscode` | `VsCodeTheme[]`: `id`, `label`, `extension`, `dark`. Sorted by label. `id` is `publisher.name:Label` (split at the first colon). Empty when VS Code is not installed |
| `GET /themes/vscode/theme?id=` | `VsCodeThemeTokens`: `id`, `label`, `dark`, `tokens` (a name to value map, every custom property of a Rain theme without the leading `--`). 404 for an unknown id, `failed` when the file cannot be read |
| `GET /themes/vscode/current` | `VsCodeCurrentTheme`: `label` from `workbench.colorTheme`, and `id` when an installed theme has that label. Both are left out when there is nothing |

## Reading

- An extension counts when its `package.json` has `contributes.themes`. A label like `%key%` is
  looked up in `package.nls.json`. A folder listed in `.obsolete` is skipped. Of two versions of
  one extension (same `publisher.name`) the later folder name wins.
- `uiTheme` `vs-dark` and `hc-black` are dark, the others light.
- Theme files, `package.json` and settings are JSON with comments and trailing commas (`jsonc.rs`).
  A file over 4 MB is not read. A theme may `include` another file: its colours lie under the
  colours of the including file. An include (or a theme path) that leaves the extension folder is
  refused, and so is a chain longer than 5.

## The mapping (`tokens.rs`)

Pure functions; the tests check that **every** token of `tokens.css` gets a value (the test reads
the CSS file for the names) and that the contrast promises hold for sparse and odd themes.

- Layers: `background` = `editor.background`; `surface-0` is that mixed toward black;
  `surface-1` (and `popover`) = `editorWidget.background`; `surface-2` (`card`, `muted`,
  `secondary`) is `surface-1` mixed 5% toward the text; `hover` (`accent`) =
  `list.hoverBackground`; `selected` = `list.activeSelectionBackground`. See-through colours are
  laid over the colour beneath them. A missing key is a mix of its neighbours. Each layer is
  nudged to differ from the one before it.
- Text: `foreground` = `editor.foreground`; `muted-foreground` = `descriptionForeground`;
  `text-3` is `muted-foreground` mixed 30% toward the background; `sidebar-foreground` =
  `sideBar.foreground` or `foreground`.
- `sidebar` = `activityBar.background` or `sideBar.background`. `border` = `panel.border`,
  `editorGroup.border` or `sideBar.border`. `input` and `border-strong` = `input.border`. `ring` =
  `focusBorder`.
- Accent (`primary`): the first solid colour of `button.background`, `activityBarBadge.background`,
  `badge.background`, `progressBar.background`, `textLink.foreground`; else a see-through
  `button.background` laid over the background; else `focusBorder`. `primary-foreground` is
  `button.foreground` when it reads on the accent, else white or near-black.
- Status and charts: the `terminal.ansi*` colours (blue, yellow, green, red, then cyan, magenta);
  a missing one is the colour VS Code itself uses by default. `status-failed` and `destructive`
  also try `errorForeground`.
- Heat steps: the accent, in four steps of lightness. File categories: the terminal colours and
  mixes of them. The labels on both (`on-heat`, `on-file`) are near-black on a dark theme, white
  on a light one.
- Scrollbar: `scrollbarSlider.background` and `.hoverBackground`. Selection: `selection.background`.
  The scrims and the overlay shadow are built from the theme's own surface colours.

**Contrast, by nudging the lightness** (hue and saturation stay; if one direction cannot reach
the ratio the other is tried, then pure white or black): `foreground`, `muted-foreground` and
`text-3` 4.5:1 on all six layers; status colours 4.5:1 on the plain layers and 3:1 on the selected
row; `ring`, `input`, `border-strong` and chart colours 3:1; `border` 1.4:1 (a divider that is
too faint is brought up); scrollbar thumb 2:1; text on a primary, destructive, selection, heat
or file fill 4.5:1.
