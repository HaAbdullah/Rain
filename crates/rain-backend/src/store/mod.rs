//! How the backend keeps the user's data. Foundation section 10.
//!
//! - `toml_file`: one TOML file of the user's data folder, loaded, edited and written safely.
//! - `hub`: the data folder, the one file watcher and the folder watches of the modules.
//! - `db`: SQLite for history and caches, on its own thread.
//! - `migrations`: the schema of every module, one list per module.

pub mod db;
pub mod hub;
pub mod migrations;
pub mod toml_file;
