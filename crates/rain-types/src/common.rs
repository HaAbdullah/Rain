//! Small types that many other types use: the error body, `Status` and `IconRef`.
//! Ids and dates are plain `String`s (foundation section 4.3).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The class of an error. The UI may branch on it; the HTTP status maps to it (section 4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    Unavailable,
    NotFound,
    Failed,
    Invalid,
    Conflict,
}

/// The one body of every non-2xx answer (section 4.5).
///
/// `message` is one plain sentence for the user and is shown as it is.
/// `reason` is a stable machine tag in camelCase, for tests and the few UI branches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct ApiError {
    #[ts(inline)]
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// State of one thing a screen shows: a launch step, a session, a job run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Idle,
    Running,
    Waiting,
    Success,
    Failed,
}

/// Which icon family an `IconRef` points into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum IconKind {
    Lucide,
    Brand,
    Url,
    Letter,
}

/// A reference to an icon. `value` depends on `kind`: a kebab-case Lucide name, a simple-icons
/// slug, the display name for a letter tile, or (kind `url`) a relative `media/...` reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct IconRef {
    #[ts(inline)]
    pub kind: IconKind,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint: Option<String>,
    // For kind `url` only: the text whose first letter is drawn as a tile when the image
    // cannot be loaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
}

impl IconRef {
    /// A Lucide icon by its kebab-case name.
    pub fn lucide(name: &str) -> Self {
        Self::plain(IconKind::Lucide, name)
    }

    /// A letter tile for `name`.
    pub fn letter(name: &str) -> Self {
        Self::plain(IconKind::Letter, name)
    }

    /// An image reference (`media/...`) with the text for the tile that shows when it fails to load.
    pub fn image(reference: &str, fallback: &str) -> Self {
        Self {
            fallback: Some(fallback.to_owned()),
            ..Self::plain(IconKind::Url, reference)
        }
    }

    fn plain(kind: IconKind, value: &str) -> Self {
        Self {
            kind,
            value: value.to_owned(),
            tint: None,
            fallback: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::round_trip;
    use serde_json::json;

    #[test]
    fn api_error_leaves_out_absent_source_and_reason() {
        let error = ApiError {
            code: ErrorCode::NotFound,
            message: "Game not found".into(),
            source: None,
            reason: None,
        };
        assert_eq!(
            round_trip(&error),
            json!({ "code": "notFound", "message": "Game not found" })
        );
    }

    #[test]
    fn api_error_with_every_field() {
        let error = ApiError {
            code: ErrorCode::Conflict,
            message: "Busy".into(),
            source: Some("backend".into()),
            reason: Some("busy".into()),
        };
        assert_eq!(
            round_trip(&error),
            json!({ "code": "conflict", "message": "Busy", "source": "backend", "reason": "busy" })
        );
    }

    #[test]
    fn error_codes_use_camel_case_names() {
        let names: Vec<_> = [
            ErrorCode::Unavailable,
            ErrorCode::NotFound,
            ErrorCode::Failed,
            ErrorCode::Invalid,
            ErrorCode::Conflict,
        ]
        .iter()
        .map(|code| serde_json::to_value(code).unwrap())
        .collect();
        assert_eq!(
            names,
            [
                json!("unavailable"),
                json!("notFound"),
                json!("failed"),
                json!("invalid"),
                json!("conflict")
            ]
        );
    }

    #[test]
    fn an_unknown_error_code_is_rejected() {
        assert!(serde_json::from_value::<ErrorCode>(json!("broken")).is_err());
    }

    #[test]
    fn status_values_match_the_frontend_union() {
        let all = [
            (Status::Idle, "idle"),
            (Status::Running, "running"),
            (Status::Waiting, "waiting"),
            (Status::Success, "success"),
            (Status::Failed, "failed"),
        ];
        for (status, name) in all {
            assert_eq!(serde_json::to_value(status).unwrap(), json!(name));
        }
    }

    #[test]
    fn icon_ref_without_tint_has_no_tint_key() {
        let icon = IconRef {
            kind: IconKind::Letter,
            value: "A".into(),
            tint: None,
            fallback: None,
        };
        assert_eq!(round_trip(&icon), json!({ "kind": "letter", "value": "A" }));
    }

    #[test]
    fn an_image_icon_carries_its_fallback_text() {
        let icon = IconRef::image("media/app-icon/app-x?v=1", "App X");
        assert_eq!(
            round_trip(&icon),
            json!({ "kind": "url", "value": "media/app-icon/app-x?v=1", "fallback": "App X" })
        );
        assert_eq!(IconRef::letter("A").kind, IconKind::Letter);
        assert_eq!(IconRef::lucide("bot").kind, IconKind::Lucide);
    }

    #[test]
    fn icon_ref_with_tint() {
        let icon = IconRef {
            kind: IconKind::Url,
            value: "media/app-icon/app-vscode?v=3fa9".into(),
            tint: Some("#ff8800".into()),
            fallback: None,
        };
        assert_eq!(
            round_trip(&icon),
            json!({ "kind": "url", "value": "media/app-icon/app-vscode?v=3fa9", "tint": "#ff8800" })
        );
    }
}
