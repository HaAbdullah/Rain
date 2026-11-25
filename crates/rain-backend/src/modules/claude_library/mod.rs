//! Claude library: one place that shows what the Claude setup of the owner contains. Read only.
//!
//! | File | Does |
//! |---|---|
//! | `scan` | finds the files: skills, memory, instructions, agents, commands (no content read) |
//! | `frontmatter` | reads the `---` block of a markdown file (pure text code) |
//! | `setup` | plugins, MCP server names, hooks and output styles as names and short facts |
//! | `service` | the list, the text of one file, the id of a file |
//! | `routes` | the HTTP routes |
//!
//! Rules: nothing is written under the Claude folder; nothing is read before a route is called
//! (no watcher, no cache, no start-up scan); the credentials file, tokens, `env` and `headers`
//! are never read out; a file opens only when the last scan lists it. The module has no tables,
//! no background work and no service slot: the routes build what a scan needs on each call.

mod frontmatter;
mod routes;
mod scan;
mod service;
mod setup;

use super::{ModuleEntry, nothing, ready};

pub const ENTRY: ModuleEntry = ModuleEntry {
    name: "claude_library",
    migrations: &[],
    router: routes::router,
    init: |_| ready(),
    start: |_| ready(),
    stop: |_| nothing(),
};
