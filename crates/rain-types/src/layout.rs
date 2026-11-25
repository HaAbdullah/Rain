//! Launch layouts: what opens with a project and where on the screen it goes. Section 02 6.17.
//!
//! A layout is a list of items. Each item says WHAT opens (`LayoutWhat`) and WHERE it goes
//! (`LayoutZone` of one monitor). `GET /projects/{id}/layout` reads the layout of a project and
//! `PUT /projects/{id}/layout` writes it to `projects.toml`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::common::IconRef;
use crate::projects::EnvKind;

/// The most items one layout holds.
pub const LAYOUT_ITEMS_MAX: usize = 12;
/// The most monitors a layout can name.
pub const LAYOUT_MONITOR_MAX: u32 = 16;

/// A part of the work area of a monitor (the screen without the taskbar). The names are the
/// same on the wire and in `projects.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum LayoutZone {
    /// Where Windows puts the window: nothing is moved.
    #[default]
    Free,
    Full,
    LeftHalf,
    RightHalf,
    TopHalf,
    BottomHalf,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    LeftThird,
    MiddleThird,
    RightThird,
    LeftTwoThirds,
    RightTwoThirds,
}

impl LayoutZone {
    /// Every zone, `Free` first.
    pub const ALL: [LayoutZone; 15] = [
        Self::Free,
        Self::Full,
        Self::LeftHalf,
        Self::RightHalf,
        Self::TopHalf,
        Self::BottomHalf,
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
        Self::LeftThird,
        Self::MiddleThird,
        Self::RightThird,
        Self::LeftTwoThirds,
        Self::RightTwoThirds,
    ];

    /// The name that `projects.toml` uses.
    pub fn name(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Full => "full",
            Self::LeftHalf => "left-half",
            Self::RightHalf => "right-half",
            Self::TopHalf => "top-half",
            Self::BottomHalf => "bottom-half",
            Self::TopLeft => "top-left",
            Self::TopRight => "top-right",
            Self::BottomLeft => "bottom-left",
            Self::BottomRight => "bottom-right",
            Self::LeftThird => "left-third",
            Self::MiddleThird => "middle-third",
            Self::RightThird => "right-third",
            Self::LeftTwoThirds => "left-two-thirds",
            Self::RightTwoThirds => "right-two-thirds",
        }
    }

    /// The zone of a name, or `None` for a name that no zone has.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|zone| zone.name() == name)
    }
}

/// What opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LayoutWhat {
    /// A step by name: `editor`, `terminal` or `claude` (built in), or a `[[step]]` of the file
    /// (a terminal with a command, a browser page).
    Step { name: String },
    /// An app of the launcher's list, by its id.
    App { id: String },
    /// The project folder in Explorer.
    Folder,
}

/// One item of a layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct LayoutItem {
    pub what: LayoutWhat,
    #[ts(inline)]
    pub zone: LayoutZone,
    // Absent: the monitor that Rain's window is on. 1 is the monitor at the far left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor: Option<u32>,
}

/// Where the layout of a project comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum LayoutScope {
    /// The project has its own layout.
    Project,
    /// The layout under `[defaults]` serves all projects.
    Default,
    /// No layout: the steps open where Windows puts them.
    None,
}

/// A step that a layout item can name, for the picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LayoutStepOption {
    pub name: String,
    #[ts(inline)]
    pub kind: EnvKind,
    pub label: String,
    pub icon: IconRef,
}

/// `GET /projects/{id}/layout`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectLayout {
    // Empty when `scope` is `none`.
    pub items: Vec<LayoutItem>,
    #[ts(inline)]
    pub scope: LayoutScope,
    // The steps that an item can name: the built-in ones and those of `projects.toml`.
    pub steps: Vec<LayoutStepOption>,
    // How many monitors the PC has now.
    pub monitors: u32,
}

/// `PUT /projects/{id}/layout`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LayoutSave {
    // An empty list removes the layout that `all` names.
    pub items: Vec<LayoutItem>,
    // True: write the layout under `[defaults]`. False: write the project's own layout.
    pub all: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::round_trip;
    use serde_json::json;

    #[test]
    fn every_zone_has_a_name_that_reads_back() {
        for zone in LayoutZone::ALL {
            assert_eq!(LayoutZone::from_name(zone.name()), Some(zone));
            assert_eq!(
                json!(zone),
                json!(zone.name()),
                "the wire name is the file name"
            );
        }
        assert_eq!(LayoutZone::from_name("sideways"), None);
        assert_eq!(LayoutZone::default(), LayoutZone::Free);
    }

    #[test]
    fn items_round_trip_and_leave_out_an_absent_monitor() {
        let item = LayoutItem {
            what: LayoutWhat::Step {
                name: "editor".into(),
            },
            zone: LayoutZone::RightHalf,
            monitor: None,
        };
        let value = round_trip(&item);
        assert!(value.get("monitor").is_none());
        assert_eq!(value["what"], json!({"kind": "step", "name": "editor"}));
        assert_eq!(value["zone"], "right-half");

        let app = LayoutItem {
            what: LayoutWhat::App {
                id: "app-notepad".into(),
            },
            zone: LayoutZone::TopLeft,
            monitor: Some(2),
        };
        assert_eq!(round_trip(&app)["monitor"], 2);
        assert_eq!(
            round_trip(&LayoutItem {
                what: LayoutWhat::Folder,
                zone: LayoutZone::Free,
                monitor: None,
            })["what"],
            json!({"kind": "folder"})
        );
    }
}
