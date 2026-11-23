//! Search: ranking and ordering of the candidates. Section 01 6.3.
//!
//! The candidates are collected by the `Service` (apps and pools from its own lists, projects
//! and games through `ItemSource`). This file only ranks, orders and caps them, so it needs no
//! disk, no SQLite and no process snapshot.

use rain_types::{SearchItem, SearchType};

use super::limits::SEARCH_MAX;
use super::score::rank;

/// An item that search may find, with the words that find it besides its title.
pub struct Candidate {
    pub item: SearchItem,
    pub keywords: Vec<String>,
}

/// Ranks every candidate, drops the ones that do not match, and returns the best `SEARCH_MAX`.
///
/// The order is the score (high first), then the title in lower case, then the id. Equal scores
/// keep a steady order, so the list does not jump while a person types.
pub fn search(query: &str, candidates: Vec<Candidate>) -> Vec<SearchItem> {
    let mut found: Vec<SearchItem> = candidates
        .into_iter()
        .filter_map(|candidate| {
            let score = rank(query, &candidate.item.title, &candidate.keywords);
            (score > 0).then_some(SearchItem {
                score,
                ..candidate.item
            })
        })
        .collect();
    found.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
            .then_with(|| a.id.cmp(&b.id))
    });
    found.truncate(SEARCH_MAX);
    found
}

/// Reads the `types` query value: a comma list of search types. `None` means all of them.
pub fn parse_types(list: Option<&str>) -> Result<Option<Vec<SearchType>>, String> {
    let Some(list) = list.filter(|list| !list.trim().is_empty()) else {
        return Ok(None);
    };
    let mut types = Vec::new();
    for name in list.split(',').map(str::trim) {
        let parsed =
            serde_json::from_value::<SearchType>(serde_json::Value::String(name.to_owned()))
                .map_err(|_| name.to_owned())?;
        types.push(parsed);
    }
    Ok(Some(types))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rain_types::IconRef;

    fn candidate(id: &str, title: &str, keywords: &[&str]) -> Candidate {
        Candidate {
            item: SearchItem {
                r#type: SearchType::App,
                id: id.into(),
                title: title.into(),
                subtitle: None,
                icon: IconRef::letter(title),
                status: None,
                score: 0,
            },
            keywords: keywords.iter().map(|word| (*word).to_owned()).collect(),
        }
    }

    #[test]
    fn items_are_ranked_dropped_when_they_do_not_match_and_ordered_steadily() {
        let found = search(
            "code",
            vec![
                candidate("app-3", "Code review notes", &[]),
                candidate("app-2", "code", &[]),
                candidate("app-1", "Code", &[]),
                candidate("app-9", "Notepad", &["text"]),
                candidate("app-5", "Editor", &["code"]),
            ],
        );
        let ids: Vec<&str> = found.iter().map(|item| item.id.as_str()).collect();
        // Two exact matches (100) tie and sort by title then id; the keyword hit comes last.
        assert_eq!(ids, ["app-1", "app-2", "app-3", "app-5"]);
        assert_eq!(found[0].score, 100);
        assert!(found.iter().all(|item| item.score > 0));
    }

    #[test]
    fn the_list_is_cut_to_fifty() {
        let many: Vec<Candidate> = (0..60)
            .map(|n| candidate(&format!("app-{n:02}"), &format!("Match {n:02}"), &[]))
            .collect();
        let found = search("match", many);
        assert_eq!(found.len(), SEARCH_MAX);
        assert_eq!(found[0].id, "app-00");
    }

    #[test]
    fn a_title_match_beats_a_keyword_match() {
        let found = search(
            "web",
            vec![
                candidate("app-chrome", "Chrome", &["browser", "web"]),
                candidate("pool-web-dev", "Web dev", &[]),
            ],
        );
        assert_eq!(found[0].id, "pool-web-dev");
    }

    #[test]
    fn the_types_list_is_read_and_unknown_names_are_named() {
        assert_eq!(parse_types(None).unwrap(), None);
        assert_eq!(parse_types(Some("")).unwrap(), None);
        assert_eq!(
            parse_types(Some("app, idleJob")).unwrap(),
            Some(vec![SearchType::App, SearchType::IdleJob])
        );
        assert_eq!(parse_types(Some("app,robot")).unwrap_err(), "robot");
    }
}
