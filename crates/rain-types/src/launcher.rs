//! The launcher types that the launch contract of the backend names: `SearchItem`,
//! `LaunchTarget` and `LaunchRun`. Foundation 11.1 (`core/launch_contract.rs`) and section 01 3.2.
//!
//! These are written here, in their final shape, so that `ItemSource` and `LaunchPlanner` can
//! exist before the launcher is built. They are NOT in the `declarations!` list of `bindings.rs`
//! yet: the frontend still declares the same names by hand, and the check of `npm run
//! types:barrel` fails when a name is in both places. The launcher stage (section 01, stage 4)
//! adds the rest of this file (`AppEntry`, `Pool`, ...) and one line per type to the list, in the
//! same commit that deletes the hand-written ones from `src/data/types.ts`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::common::{IconRef, Status};

/// What a search item can be. camelCase gives `idleJob`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SearchType {
    App,
    Pool,
    Project,
    Game,
    Session,
    Job,
    IdleJob,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct SearchItem {
    #[ts(inline)]
    pub r#type: SearchType,
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    pub icon: IconRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(inline)]
    pub status: Option<Status>,
    // On the scale of `src/lib/score.ts`; 0 for pins and recent items.
    pub score: u32,
}

/// The four kinds of thing that can be launched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum LaunchType {
    App,
    Pool,
    Project,
    Game,
}

/// How a project is opened. `terminal` and `claude` are "Terminal here" and "New Claude session
/// here" of section 02.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum LaunchMode {
    Full,
    EditorOnly,
    Terminal,
    Claude,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(optional_fields)]
pub struct LaunchTarget {
    #[ts(inline)]
    pub r#type: LaunchType,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "'full' | 'editorOnly' | 'terminal' | 'claude'")]
    pub mode: Option<LaunchMode>,
    // Only for type `app`: extra command-line text for this launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct LaunchStep {
    pub label: String,
    pub icon: IconRef,
    #[ts(inline)]
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The status of a whole run: a subset of `Status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    Running,
    Success,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LaunchRun {
    pub id: String,
    pub target: LaunchTarget,
    #[ts(inline)]
    pub status: RunStatus,
    pub steps: Vec<LaunchStep>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::IconKind;
    use crate::test_support::round_trip;
    use serde_json::json;

    fn icon() -> IconRef {
        IconRef {
            kind: IconKind::Letter,
            value: "Code".into(),
            tint: None,
        }
    }

    #[test]
    fn a_search_item_names_its_type_and_leaves_out_absent_fields() {
        let item = SearchItem {
            r#type: SearchType::IdleJob,
            id: "idle-1".into(),
            title: "Backup".into(),
            subtitle: None,
            icon: icon(),
            status: None,
            score: 0,
        };
        assert_eq!(
            round_trip(&item),
            json!({
                "type": "idleJob", "id": "idle-1", "title": "Backup", "score": 0,
                "icon": { "kind": "letter", "value": "Code" }
            })
        );
    }

    #[test]
    fn a_launch_target_rejects_unknown_fields_and_reads_its_mode() {
        let target: LaunchTarget =
            serde_json::from_value(json!({ "type": "project", "id": "p", "mode": "editorOnly" }))
                .unwrap();
        assert_eq!(target.mode, Some(LaunchMode::EditorOnly));
        assert!(
            serde_json::from_value::<LaunchTarget>(json!({ "type": "app", "id": "a", "x": 1 }))
                .is_err()
        );
    }

    #[test]
    fn a_launch_run_round_trips() {
        let run = LaunchRun {
            id: "run-1".into(),
            target: LaunchTarget {
                r#type: LaunchType::Pool,
                id: "pool-web".into(),
                mode: None,
                args: None,
            },
            status: RunStatus::Running,
            steps: vec![LaunchStep {
                label: "Editor".into(),
                icon: icon(),
                status: Status::Running,
                error: None,
            }],
        };
        round_trip(&run);
    }
}
