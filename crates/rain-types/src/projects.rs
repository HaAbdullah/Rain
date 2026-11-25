//! The types of the projects module: what the Projects screen shows. Section 02 3.1.
//!
//! A project is a git repository that lies directly inside one of the folders of the setting
//! `projectRoots`. The list is sent whole (`GET /projects` and the topic `projects`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::common::IconRef;

/// What one step of "open this project" is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum EnvKind {
    Editor,
    Terminal,
    Browser,
    Claude,
    App,
    // The project folder in Explorer.
    Folder,
}

/// One step of the environment of a project, as the detail pane lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct EnvStep {
    #[ts(inline)]
    pub kind: EnvKind,
    pub label: String,
    pub icon: IconRef,
    // Text shown in mono in the detail pane: the command of a terminal step, or `claude`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct Project {
    // `proj-<slug of the folder name>`
    pub id: String,
    pub name: String,
    // With backslashes and without a trailing backslash.
    pub path: String,
    pub icon: IconRef,
    // Empty when the folder cannot be read as a repository.
    pub branch: String,
    // Absent: unknown (git is missing, git failed, or the answer is not computed yet). 0: clean.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dirty_files: Option<u32>,
    // 14 counts, oldest first, the last one is today (local days). 14 zeros when unknown.
    #[serde(rename = "commits14d")]
    pub commits_14d: Vec<u32>,
    // Absent: never opened from Rain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_opened_at: Option<String>,
    pub activity_score: u32,
    pub environment: Vec<EnvStep>,
    // An editor window of this project is open now.
    pub open: bool,
    pub pinned: bool,
    // The folder is gone (it was seen before).
    pub missing: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::IconKind;
    use crate::test_support::round_trip;
    use serde_json::json;

    fn letter(text: &str) -> IconRef {
        IconRef {
            kind: IconKind::Letter,
            value: text.into(),
            tint: None,
            fallback: None,
        }
    }

    fn minimal() -> Project {
        Project {
            id: "proj-alpha".into(),
            name: "alpha".into(),
            path: "D:\\Work\\alpha".into(),
            icon: letter("alpha"),
            branch: "main".into(),
            dirty_files: None,
            commits_14d: vec![0; 14],
            last_opened_at: None,
            activity_score: 0,
            environment: Vec::new(),
            open: false,
            pinned: false,
            missing: false,
        }
    }

    #[test]
    fn a_project_without_facts_leaves_them_out_and_keeps_the_wire_name_of_the_strip() {
        let json = round_trip(&minimal());
        assert!(json.get("dirtyFiles").is_none());
        assert!(json.get("lastOpenedAt").is_none());
        assert_eq!(json["commits14d"], json!(vec![0; 14]));
        assert!(json.get("commits_14d").is_none());
    }

    #[test]
    fn a_full_project_round_trips() {
        let mut project = minimal();
        project.dirty_files = Some(3);
        project.last_opened_at = Some("2026-11-22T09:30:00.000Z".into());
        project.environment = vec![EnvStep {
            kind: EnvKind::Terminal,
            label: "Dev server".into(),
            icon: letter("D"),
            command: Some("npm run dev".into()),
            url: None,
        }];
        let json = round_trip(&project);
        assert_eq!(json["dirtyFiles"], 3);
        assert_eq!(json["environment"][0]["kind"], "terminal");
    }

    #[test]
    fn two_hundred_projects_fit_in_one_frame() {
        let mut project = minimal();
        project.environment = vec![
            EnvStep {
                kind: EnvKind::Editor,
                label: "VS Code".into(),
                icon: letter("VS Code"),
                command: None,
                url: None,
            };
            3
        ];
        let list = vec![project; 200];
        let bytes = serde_json::to_vec(&list).unwrap();
        assert!(
            bytes.len() < crate::limits::FRAME_MAX,
            "{} bytes",
            bytes.len()
        );
    }
}
