//! The launcher types: search, apps, pools, pins and launch runs. Feature section 01, 3.2.
//!
//! They are all in the `declarations!` list of `bindings.rs`, so the TypeScript copy is generated
//! and the frontend does not declare them by hand.

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

/// The four kinds of thing that can be pinned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PinnableType {
    App,
    Pool,
    Project,
    Game,
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

/// How an app is started and what the UI may show for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum AppKind {
    /// Has a program file.
    Program,
    /// A packaged (Store) app: no file.
    Store,
    /// A shell object: no file.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    // Kind `url`, value `media/app-icon/<id>?v=<hex8>`, `fallback` is the name.
    pub icon: IconRef,
    #[ts(inline)]
    pub kind: AppKind,
    // Absent for kind `store` and `other`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exe_path: Option<String>,
    pub keywords: Vec<String>,
    pub pinned: bool,
    pub running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_launched_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(optional_fields)]
pub struct PoolMember {
    pub app_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay_ms: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct Pool {
    pub id: String,
    pub name: String,
    // Always a letter tile of the name; derived, never stored.
    pub icon: IconRef,
    pub members: Vec<PoolMember>,
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_launched_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(optional_fields)]
pub struct PoolDraft {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    pub members: Vec<PoolMember>,
}

/// The body of `PUT /pinned/{type}/{id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct PinBody {
    pub pinned: bool,
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
    // Only for type `app`: extra command-line text for this launch (a pool's "Retry failed"
    // passes the member's `args`).
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
            fallback: None,
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

    #[test]
    fn an_app_entry_leaves_out_the_path_of_a_store_app() {
        let app = AppEntry {
            id: "app-calculator".into(),
            name: "Calculator".into(),
            icon: IconRef::image("media/app-icon/app-calculator?v=0a1b2c3d", "Calculator"),
            kind: AppKind::Store,
            exe_path: None,
            keywords: vec!["math".into()],
            pinned: false,
            running: false,
            last_launched_at: None,
        };
        let json = round_trip(&app);
        assert_eq!(json["kind"], "store");
        assert!(json.get("exePath").is_none());
        assert_eq!(json["icon"]["fallback"], "Calculator");
    }

    #[test]
    fn a_pool_and_its_draft_use_camel_case_and_refuse_unknown_fields() {
        let pool = Pool {
            id: "pool-web".into(),
            name: "Web".into(),
            icon: IconRef::letter("Web"),
            members: vec![PoolMember {
                app_id: "app-chrome".into(),
                args: Some("--new-window".into()),
                delay_ms: Some(2000),
            }],
            pinned: true,
            last_launched_at: Some("2026-11-22T10:00:00.000Z".into()),
        };
        let json = round_trip(&pool);
        assert_eq!(json["members"][0]["appId"], "app-chrome");
        assert_eq!(json["members"][0]["delayMs"], 2000);
        assert!(
            serde_json::from_value::<PoolDraft>(
                json!({ "name": "x", "members": [], "pinned": true })
            )
            .is_err()
        );
        let draft: PoolDraft =
            serde_json::from_value(json!({ "name": "x", "members": [] })).unwrap();
        assert_eq!(draft.id, None);
    }

    #[test]
    fn a_pin_body_reads_one_boolean_and_nothing_else() {
        let body: PinBody = serde_json::from_value(json!({ "pinned": true })).unwrap();
        assert!(body.pinned);
        assert!(serde_json::from_value::<PinBody>(json!({ "pinned": true, "x": 1 })).is_err());
        assert!(serde_json::from_value::<PinBody>(json!({})).is_err());
    }

    #[test]
    fn a_pinnable_type_has_the_four_values() {
        for name in ["app", "pool", "project", "game"] {
            let kind: PinnableType = serde_json::from_value(json!(name)).unwrap();
            assert_eq!(serde_json::to_value(kind).unwrap(), json!(name));
        }
        assert!(serde_json::from_value::<PinnableType>(json!("job")).is_err());
    }
}
