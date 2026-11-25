//! Colour themes that Rain reads from the user's VS Code (module `vscode_themes`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One theme that an installed VS Code extension offers. `GET /themes/vscode`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct VsCodeTheme {
    // The extension id and the label, joined by a colon: `publisher.name:Theme label`.
    pub id: String,
    pub label: String,
    // The extension id (`publisher.name`).
    pub extension: String,
    // True for a dark theme (and for a high-contrast dark one).
    pub dark: bool,
}

/// The colour tokens of one theme, made from its file. `GET /themes/vscode/theme?id=`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct VsCodeThemeTokens {
    pub id: String,
    pub label: String,
    pub dark: bool,
    // Every custom property of a Rain theme without the leading `--`: `background`, `primary` ...
    #[ts(type = "Record<string, string>")]
    pub tokens: BTreeMap<String, String>,
}

/// The theme VS Code uses now. `GET /themes/vscode/current`. Both fields are left out when
/// VS Code has no settings file or no theme chosen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(optional_fields)]
pub struct VsCodeCurrentTheme {
    // The label from `workbench.colorTheme`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    // The id of the installed theme with that label, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::round_trip;
    use serde_json::json;

    #[test]
    fn a_theme_in_the_list_uses_camel_case() {
        let theme = VsCodeTheme {
            id: "pub.ext:Night".into(),
            label: "Night".into(),
            extension: "pub.ext".into(),
            dark: true,
        };
        assert_eq!(
            round_trip(&theme),
            json!({ "id": "pub.ext:Night", "label": "Night", "extension": "pub.ext", "dark": true })
        );
    }

    #[test]
    fn the_current_theme_leaves_out_what_it_does_not_know() {
        let none = VsCodeCurrentTheme {
            label: None,
            id: None,
        };
        assert_eq!(round_trip(&none), json!({}));
        let named = VsCodeCurrentTheme {
            label: Some("Night".into()),
            id: None,
        };
        assert_eq!(round_trip(&named), json!({ "label": "Night" }));
    }

    #[test]
    fn tokens_are_a_plain_object() {
        let theme = VsCodeThemeTokens {
            id: "a:b".into(),
            label: "b".into(),
            dark: false,
            tokens: BTreeMap::from([("background".to_owned(), "#ffffff".to_owned())]),
        };
        assert_eq!(
            round_trip(&theme)["tokens"],
            json!({ "background": "#ffffff" })
        );
    }
}
