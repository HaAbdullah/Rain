//! VS Code themes: Rain reads the colour themes of the installed VS Code extensions and turns
//! one into its own colour tokens. The owner's wish: "match the VS Code themes".
//!
//! No state, no table, no thread and no cache. A route reads the extension folder (or one theme
//! file, or the VS Code settings) when it is called, and the window calls it only when the
//! Settings screen opens, when the window is summoned with "Follow VS Code" on, or when a theme
//! is picked. Nothing from a theme file is stored in the repository: the colours are read at run
//! time (Bearded Theme is GPL-3.0, Rain is MIT).
//!
//! - `reader.rs`: finds the themes, reads a theme file (with `include`) and the current theme.
//! - `jsonc.rs`: JSON with comments and trailing commas.
//! - `color.rs`: colour arithmetic. `tokens.rs`: the mapping from VS Code keys to Rain tokens.

mod color;
mod jsonc;
mod reader;
mod routes;
mod tokens;

pub use reader::{SystemThemeSource, ThemeSource};

use super::{ModuleEntry, nothing, ready};

pub const ENTRY: ModuleEntry = ModuleEntry {
    name: "vscode_themes",
    migrations: &[],
    router: routes::router,
    init: |_| ready(),
    start: |_| ready(),
    stop: |_| nothing(),
};
