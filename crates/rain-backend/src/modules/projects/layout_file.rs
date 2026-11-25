//! Writing a layout into `projects.toml`. Section 02 6.17.
//!
//! The one place where Rain changes this file besides the example it writes when there is none.
//! The change is an edit of the text, so the user's comments and the order of the file stay:
//! only the `layout` key of `[defaults]` or of one `[[project]]` is set or removed. A project that
//! has no `[[project]]` entry yet gets one, with its `path`.

use rain_types::{LayoutItem, LayoutWhat};
use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value};

use super::ids::path_key;
use crate::core::error::{ApiError, ApiResult};

/// Whose layout is written.
pub enum Scope<'a> {
    /// The layout of `[defaults]`: it serves every project.
    Defaults,
    /// The layout of one project. `key` and `id` find its entry; `path` names a new entry.
    Project {
        key: &'a str,
        id: &'a str,
        path: &'a str,
    },
}

fn cannot_edit(what: &str) -> ApiError {
    ApiError::conflict(
        "configChanged",
        format!(
            "projects.toml has {what} in a form that Rain cannot change. Edit the file by hand."
        ),
    )
}

/// The items as the value of the `layout` key: one inline table for each item, on its own line.
fn layout_value(items: &[LayoutItem]) -> Value {
    let mut array = Array::new();
    for item in items {
        let mut table = InlineTable::new();
        match &item.what {
            LayoutWhat::Step { name } => {
                table.insert("step", name.as_str().into());
            }
            LayoutWhat::App { id } => {
                table.insert("app", id.as_str().into());
            }
            LayoutWhat::Folder => {
                table.insert("folder", true.into());
            }
        }
        table.insert("zone", item.zone.name().into());
        if let Some(number) = item.monitor {
            table.insert("monitor", i64::from(number).into());
        }
        table.fmt();
        let mut value = Value::InlineTable(table);
        value.decor_mut().set_prefix("\n  ");
        array.push_formatted(value);
    }
    array.set_trailing_comma(true);
    array.set_trailing("\n");
    Value::Array(array)
}

/// Sets the `layout` of `table`, or removes it when there are no items.
fn set_in(table: &mut dyn toml_edit::TableLike, items: &[LayoutItem]) {
    if items.is_empty() {
        table.remove("layout");
    } else {
        table.insert("layout", Item::Value(layout_value(items)));
    }
}

/// Changes `document` so that the layout of `scope` is `items`. An empty list removes the layout.
pub fn set_layout(
    document: &mut DocumentMut,
    scope: &Scope<'_>,
    items: &[LayoutItem],
) -> ApiResult<()> {
    match scope {
        Scope::Defaults => {
            if items.is_empty() && !document.contains_key("defaults") {
                return Ok(());
            }
            if !document.contains_key("defaults") {
                document.insert("defaults", Item::Table(Table::new()));
            }
            let defaults = document["defaults"]
                .as_table_like_mut()
                .ok_or_else(|| cannot_edit("[defaults]"))?;
            set_in(defaults, items);
            Ok(())
        }
        Scope::Project { key, id, path } => set_project(document, key, id, path, items),
    }
}

fn set_project(
    document: &mut DocumentMut,
    key: &str,
    id: &str,
    path: &str,
    items: &[LayoutItem],
) -> ApiResult<()> {
    if !document.contains_key("project") {
        if items.is_empty() {
            return Ok(());
        }
        document.insert("project", Item::ArrayOfTables(ArrayOfTables::new()));
    }
    let entries = document["project"]
        .as_array_of_tables_mut()
        .ok_or_else(|| cannot_edit("the [[project]] list"))?;
    let by_path = |table: &Table| {
        table
            .get("path")
            .and_then(Item::as_str)
            .is_some_and(|text| path_key(text) == key)
    };
    let by_id = |table: &Table| {
        table.get("path").is_none() && table.get("id").and_then(Item::as_str) == Some(id)
    };
    let found = entries
        .iter()
        .position(by_path)
        .or_else(|| entries.iter().position(by_id));
    match found {
        Some(index) => {
            if let Some(entry) = entries.get_mut(index) {
                set_in(entry, items);
            }
        }
        None if items.is_empty() => {}
        None => {
            let mut entry = Table::new();
            entry.insert("path", Item::Value(path.into()));
            set_in(&mut entry, items);
            entries.push(entry);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::projects::env::{Builtins, ProjectsFile, analyze};
    use rain_types::LayoutZone;

    fn item(step: &str, zone: LayoutZone, monitor: Option<u32>) -> LayoutItem {
        LayoutItem {
            what: LayoutWhat::Step { name: step.into() },
            zone,
            monitor,
        }
    }

    const HAND_WRITTEN: &str = r#"schema_version = 1

# My own notes about this file.
[terminal]
window = "new"   # I like a window of its own

[[step]]
name = "dev-server"   # started by hand sometimes
kind = "terminal"
command = "npm run dev"

[[project]]
path = "D:/Work/Alpha/"
steps = ["editor", "dev-server"]   # alpha is special

[[project]]
id = "proj-beta"
"#;

    fn edit(text: &str, scope: &Scope<'_>, items: &[LayoutItem]) -> String {
        let mut document: DocumentMut = text.parse().unwrap();
        set_layout(&mut document, scope, items).unwrap();
        document.to_string()
    }

    fn analysis_of(text: &str) -> crate::modules::projects::env::Analysis {
        let file: ProjectsFile = toml::from_str(text).unwrap();
        analyze(
            &file,
            &Builtins {
                editor_label: "VS Code".into(),
            },
        )
    }

    #[test]
    fn a_default_layout_is_added_and_the_comments_and_order_stay() {
        let items = [
            item("editor", LayoutZone::RightHalf, None),
            item("terminal", LayoutZone::LeftHalf, Some(2)),
        ];
        let text = edit(HAND_WRITTEN, &Scope::Defaults, &items);
        for kept in [
            "# My own notes about this file.",
            "window = \"new\"   # I like a window of its own",
            "name = \"dev-server\"   # started by hand sometimes",
            "steps = [\"editor\", \"dev-server\"]   # alpha is special",
        ] {
            assert!(text.contains(kept), "lost: {kept}\n{text}");
        }
        assert!(text.contains("[defaults]\nlayout = [\n  { step = \"editor\", zone = \"right-half\" },\n  { step = \"terminal\", zone = \"left-half\", monitor = 2 },\n]"), "{text}");
        let analysis = analysis_of(&text);
        assert_eq!(analysis.ignored, 0);
        assert_eq!(analysis.default_layout, items);
    }

    #[test]
    fn a_project_layout_goes_into_its_own_entry_found_by_path_in_any_spelling() {
        let items = [item("editor", LayoutZone::Full, None)];
        let scope = Scope::Project {
            key: "d:\\work\\alpha",
            id: "proj-alpha",
            path: "D:\\Work\\Alpha",
        };
        let text = edit(HAND_WRITTEN, &scope, &items);
        assert_eq!(
            text.matches("[[project]]").count(),
            2,
            "no new entry:\n{text}"
        );
        assert!(text.contains("# alpha is special"));
        let analysis = analysis_of(&text);
        assert_eq!(analysis.projects[0].layout, items);
        assert!(analysis.projects[1].layout.is_empty());
    }

    #[test]
    fn a_project_found_by_id_gets_the_layout_in_its_entry() {
        let items = [item("claude", LayoutZone::BottomHalf, None)];
        let scope = Scope::Project {
            key: "e:\\other\\beta",
            id: "proj-beta",
            path: "E:\\Other\\Beta",
        };
        let text = edit(HAND_WRITTEN, &scope, &items);
        assert_eq!(text.matches("[[project]]").count(), 2);
        assert_eq!(analysis_of(&text).projects[1].layout, items);
    }

    #[test]
    fn a_project_without_an_entry_gets_one_with_its_path() {
        let items = [
            LayoutItem {
                what: LayoutWhat::App {
                    id: "app-notepad".into(),
                },
                zone: LayoutZone::TopRight,
                monitor: None,
            },
            LayoutItem {
                what: LayoutWhat::Folder,
                zone: LayoutZone::Free,
                monitor: None,
            },
        ];
        let scope = Scope::Project {
            key: "d:\\work\\gamma",
            id: "proj-gamma",
            path: "D:\\Work\\Gamma",
        };
        let text = edit(HAND_WRITTEN, &scope, &items);
        assert_eq!(text.matches("[[project]]").count(), 3);
        let analysis = analysis_of(&text);
        let entry = analysis.entry_for("d:\\work\\gamma", "proj-gamma").unwrap();
        assert_eq!(entry.layout, items);
        assert_eq!(analysis.ignored, 0);
    }

    #[test]
    fn a_file_with_only_comments_gets_its_tables_and_keeps_the_comments() {
        let text = "schema_version = 1\n\n# [defaults]\n# steps = [\"editor\"]\n";
        let out = edit(
            text,
            &Scope::Defaults,
            &[item("editor", LayoutZone::Full, None)],
        );
        assert!(
            out.starts_with(
                "schema_version = 1
"
            ),
            "{out}"
        );
        assert!(
            out.contains(
                "# [defaults]
# steps = [\"editor\"]
"
            ),
            "{out}"
        );
        assert_eq!(analysis_of(&out).default_layout.len(), 1);
    }

    #[test]
    fn an_empty_list_removes_the_layout_and_nothing_else() {
        let items = [item("editor", LayoutZone::Full, None)];
        let with = edit(HAND_WRITTEN, &Scope::Defaults, &items);
        let without = edit(&with, &Scope::Defaults, &[]);
        assert!(!without.contains("layout"), "{without}");
        assert!(without.contains("# My own notes about this file."));
        // Removing what is not there changes nothing at all.
        assert_eq!(edit(HAND_WRITTEN, &Scope::Defaults, &[]), HAND_WRITTEN);
        let scope = Scope::Project {
            key: "d:\\work\\nothing",
            id: "proj-nothing",
            path: "D:\\Work\\Nothing",
        };
        assert_eq!(edit(HAND_WRITTEN, &scope, &[]), HAND_WRITTEN);
    }

    #[test]
    fn a_second_save_replaces_the_first() {
        let scope = Scope::Project {
            key: "d:\\work\\alpha",
            id: "proj-alpha",
            path: "D:\\Work\\Alpha",
        };
        let first = edit(
            HAND_WRITTEN,
            &scope,
            &[item("editor", LayoutZone::Full, None)],
        );
        let second = edit(
            &first,
            &scope,
            &[item("terminal", LayoutZone::TopLeft, Some(3))],
        );
        assert_eq!(second.matches("layout").count(), 1, "{second}");
        let analysis = analysis_of(&second);
        assert_eq!(
            analysis.projects[0].layout,
            [item("terminal", LayoutZone::TopLeft, Some(3))]
        );
    }
}
