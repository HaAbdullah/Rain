//! Writing back into `jobs.toml` and `idle-jobs.toml`. Section 03 4.1.1 and 6.11.10.
//!
//! Rain writes only two things into these files: the `enabled` key of an entry the user toggled,
//! and the `id` of that same entry when it has none (so a later rename of `name` cannot orphan
//! its history). Everything else, comments included, stays as the user wrote it
//! (`toml_edit`, decision C7). Both files use these functions.

use std::cmp::Ordering;

use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, Value};

use super::entry::derive_ids;
use crate::core::blocking;
use crate::core::error::{ApiError, ApiResult};
use crate::store::hub::ConfigHub;
use crate::store::toml_file::{set_value, string_value, write_atomic};

/// The error for an entry that is not there any more (the user moved or removed it while Rain was
/// saving), so the answer is `conflict` `configChanged`.
fn entry_gone(file: &str) -> ApiError {
    ApiError::conflict(
        "configChanged",
        format!("{file} changed while saving. Try again."),
    )
}

fn text_of(table: &Table, key: &str) -> Option<String> {
    table.get(key).and_then(Item::as_str).map(str::to_owned)
}

/// Sets `enabled` in the entry with the id `id`. `table_name` is `job` or `idle_job`, `prefix`
/// the prefix of derived ids (`job-`, `idle-`). Finds the entry the way the reader does: by its
/// `id` key, or, without one, by the id derived from its name.
pub fn set_enabled(
    doc: &mut DocumentMut,
    file: &str,
    table_name: &str,
    prefix: &str,
    id: &str,
    enabled: bool,
) -> ApiResult<()> {
    let array: &mut ArrayOfTables = doc
        .get_mut(table_name)
        .and_then(Item::as_array_of_tables_mut)
        .ok_or_else(|| entry_gone(file))?;
    let raw: Vec<(Option<String>, Option<String>)> = array
        .iter()
        .map(|table| (text_of(table, "id"), text_of(table, "name")))
        .collect();
    let position = derive_ids(prefix, &raw)
        .iter()
        .position(|candidate| candidate == id)
        .ok_or_else(|| entry_gone(file))?;
    let has_id = raw[position].0.is_some();
    let entry = array.get_mut(position).ok_or_else(|| entry_gone(file))?;

    set_value(entry, "enabled", Value::from(enabled));
    if !has_id {
        set_value(entry, "id", string_value(id));
        // `id` goes first. The sort is stable, so the other keys keep their order.
        entry.sort_values_by(|a, _, b, _| match (a.get() == "id", b.get() == "id") {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => Ordering::Equal,
        });
    }
    Ok(())
}

/// Writes `text` as the file `name` of the data folder when the file does not exist. It holds
/// only `schema_version` and comments, so the user can see the format (section 03 4.1.3).
/// An existing file, however small, is never touched.
pub async fn ensure_default_file(hub: &ConfigHub, name: &'static str, text: &'static str) {
    let path = hub.file_path(name);
    let result = blocking::run(move || {
        if path.exists() {
            return Ok(());
        }
        write_atomic(&path, text)
    })
    .await;
    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::warn!(file = name, error = %e, "could not write the default file"),
        Err(_) => {} // the blocking task failed and said so in the log
    }
}

/// Changes a `save` error of `TomlFile` to the reason the registry names (`saveFailed`).
pub fn saved_reason(error: ApiError) -> ApiError {
    if error.body.reason.as_deref() == Some("writeFailed") {
        error.with_reason("saveFailed")
    } else {
        error
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::toml_file::render;

    const FILE: &str = "schema_version = 1\r\n\
# my jobs\r\n\
\r\n\
# the first job\r\n\
[[job]]\r\n\
name = \"Back up\" # keep this\r\n\
kind = \"cron\"\r\n\
enabled = false   # not yet\r\n\
\r\n\
[[job]]\r\n\
id = \"second\"\r\n\
name = \"Second\"\r\n";

    fn edit(text: &str, id: &str, enabled: bool) -> ApiResult<String> {
        let mut doc: DocumentMut = text.parse().unwrap();
        set_enabled(&mut doc, "jobs.toml", "job", "job-", id, enabled)?;
        Ok(render(&doc, text.contains("\r\n")))
    }

    #[test]
    fn toml_edit_enabled_keeps_comments() {
        let changed = edit(FILE, "job-back-up", true).unwrap();
        // The entry had no id, so the derived one is written first. Nothing else is lost.
        assert!(
            changed.contains("[[job]]\r\nid = 'job-back-up'\r\nname = \"Back up\" # keep this")
        );
        assert!(changed.contains("enabled = true   # not yet"));
        assert!(changed.contains("# my jobs"));
        assert!(changed.contains("# the first job"));
        assert!(!changed.contains("false"), "{changed}");
        // CRLF is kept everywhere.
        assert!(!changed.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn toggling_back_leaves_only_the_id_as_a_difference() {
        let on = edit(FILE, "job-back-up", true).unwrap();
        let off = edit(&on, "job-back-up", false).unwrap();
        assert_eq!(off.replace("id = 'job-back-up'\r\n", ""), FILE);
    }

    #[test]
    fn an_entry_with_an_id_gets_no_new_id_and_a_missing_enabled_goes_to_its_end() {
        let changed = edit(FILE, "second", true).unwrap();
        assert!(changed.ends_with("id = \"second\"\r\nname = \"Second\"\r\nenabled = true\r\n"));
        assert_eq!(changed.matches("id = ").count(), 1);
    }

    #[test]
    fn an_entry_that_is_gone_gives_config_changed() {
        let error = edit(FILE, "job-nobody", true).unwrap_err();
        assert_eq!(error.body.reason.as_deref(), Some("configChanged"));
        let empty: DocumentMut = "schema_version = 1\n".parse().unwrap();
        let mut empty = empty;
        let error = set_enabled(&mut empty, "jobs.toml", "job", "job-", "x", true).unwrap_err();
        assert_eq!(error.body.reason.as_deref(), Some("configChanged"));
    }
}
