//! The scorer of the launcher: an exact port of `src/lib/score.ts`.
//!
//! Home sorts the items of the backend and its own commands by one number, so both sides must
//! agree on it. `src/lib/score.vectors.json` holds cases that the vitest test and the test at
//! the bottom of this file both read; either side fails when only one of them changes.
//!
//! The port works on UTF-16 code units, because JavaScript measures strings and indexes them
//! that way (an emoji counts as 2). Every point where Rust could differ from JavaScript is named
//! in a comment next to the code that closes it.

const EXACT: u32 = 100;
const PREFIX: u32 = 80;
const WORD_START: u32 = 60;
const SUBSTRING: u32 = 40;
const SUBSEQUENCE: u32 = 20;
/// A keyword hit ranks just under the same kind of hit on the visible text.
const KEYWORD_PENALTY: u32 = 5;
/// An app that is found only by a keyword counts this share of its score (see `rank`).
const KEYWORD_SHARE: f64 = 0.6;

/// Lower-cased text as UTF-16 units. `str::to_lowercase` and `toLowerCase()` both use the
/// Unicode default mapping, including the final sigma and U+0130.
fn units(text: &str) -> Vec<u16> {
    text.to_lowercase().encode_utf16().collect()
}

/// The set of characters that JavaScript's `\s` and `trim()` treat as space. It is NOT the same
/// as `char::is_whitespace`: Rust counts U+0085, JavaScript counts U+FEFF.
fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// JavaScript's `indexOf(needle, from)` on code units. An empty needle is never searched for.
fn index_of(hay: &[u16], needle: &[u16], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() || from > hay.len() - needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

fn is_ascii_alphanumeric_unit(unit: u16) -> bool {
    (u16::from(b'a')..=u16::from(b'z')).contains(&unit)
        || (u16::from(b'0')..=u16::from(b'9')).contains(&unit)
}

/// True when `q` starts a word in `t` ("stat" in "weather-station"). The unit before the match
/// is tested against ASCII `a-z0-9` only, as `score.ts` does, so U+00E9 counts as a separator.
fn starts_word(t: &[u16], q: &[u16]) -> bool {
    let mut at = index_of(t, q, 1);
    while let Some(i) = at {
        if !is_ascii_alphanumeric_unit(t[i - 1]) {
            return true;
        }
        at = index_of(t, q, i + 1);
    }
    false
}

/// Length of the shortest run of `t` that holds the units of `q` in order, or 0. The same loop
/// as `score.ts`, including its `break` when one start no longer fits.
fn subsequence_span(t: &[u16], q: &[u16]) -> usize {
    let mut best = 0;
    let mut start = index_of(t, &q[..1], 0);
    while let Some(s) = start {
        let (mut ti, mut qi) = (s, 0);
        while ti < t.len() && qi < q.len() {
            if t[ti] == q[qi] {
                qi += 1;
            }
            ti += 1;
        }
        if qi < q.len() {
            break;
        }
        let span = ti - s;
        if best == 0 || span < best {
            best = span;
        }
        start = index_of(t, &q[..1], s + 1);
    }
    best
}

/// `Math.round` of a non-negative ratio: `f64::round` rounds halves up for these values, the
/// same as JavaScript does.
fn round_ratio(numerator: usize, denominator: usize) -> u32 {
    (numerator as f64 / denominator as f64).round() as u32
}

/// Scores one lower-cased text against one lower-cased, non-empty query.
fn score_text(q: &[u16], t: &[u16]) -> u32 {
    if t.is_empty() {
        return 0;
    }
    if t == q {
        return EXACT;
    }
    // A shorter text is a closer match: "code" ranks "Code" above "Code review notes".
    let closeness = round_ratio(9 * q.len(), t.len());
    if t.starts_with(q) {
        return PREFIX + closeness;
    }
    if starts_word(t, q) {
        return WORD_START + closeness;
    }
    if index_of(t, q, 0).is_some() {
        return SUBSTRING + closeness;
    }
    // One letter "in order" matches nearly everything, so it is not worth a result.
    if q.len() < 2 {
        return 0;
    }
    match subsequence_span(t, q) {
        0 => 0,
        span => SUBSEQUENCE + round_ratio(9 * q.len(), span),
    }
}

fn score_one(q: &[u16], text: &str, keywords: &[String]) -> u32 {
    let mut best = score_text(q, &units(text));
    for keyword in keywords {
        let found = score_text(q, &units(keyword));
        if found > 0 {
            best = best.max(found.saturating_sub(KEYWORD_PENALTY));
        }
    }
    best
}

/// Rank of `text` for `query`: 0 = no match, higher is better (exact > prefix > word start >
/// substring > letters in order). An empty query matches everything equally and returns 1.
pub fn score(query: &str, text: &str, keywords: &[String]) -> u32 {
    let trimmed = query.trim_matches(is_js_space);
    if trimmed.is_empty() {
        return 1;
    }
    // Lower-cased as a whole first, then split, like `score.ts` does (the final sigma depends on
    // the letters around it).
    let lowered = trimmed.to_lowercase();
    let q: Vec<u16> = lowered.encode_utf16().collect();
    let whole = score_one(&q, text, keywords);
    if whole > 0 {
        return whole;
    }
    // "dev web" should still find "Web dev": every word must match on its own. Such a result
    // ranks below any match of the whole query.
    let words: Vec<&str> = lowered
        .split(is_js_space)
        .filter(|word| !word.is_empty())
        .collect();
    if words.len() < 2 {
        return 0;
    }
    let mut weakest = u32::MAX;
    for word in words {
        let found = score_one(&units(word), text, keywords);
        if found == 0 {
            return 0;
        }
        weakest = weakest.min(found);
    }
    ((f64::from(weakest) / 10.0).round() as u32).max(1)
}

/// The rank of one search item: a title match first; an item that only a keyword finds counts
/// 60 percent, so the pool "Web dev" ranks above browsers that merely have the keyword "web".
pub fn rank(query: &str, title: &str, keywords: &[String]) -> u32 {
    let by_title = score(query, title, &[]);
    if by_title > 0 {
        return by_title;
    }
    let with_keywords = score(query, title, keywords);
    if with_keywords > 0 {
        ((f64::from(with_keywords) * KEYWORD_SHARE).round() as u32).max(1)
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Vectors {
        version: u32,
        cases: Vec<Case>,
    }

    #[derive(Deserialize)]
    struct Case {
        query: String,
        text: String,
        keywords: Vec<String>,
        expected: u32,
    }

    /// The cases that `src/lib/score.vectors.test.ts` writes from `score.ts`.
    const VECTORS: &str = include_str!("../../../../../src/lib/score.vectors.json");

    #[test]
    fn every_shared_vector_gives_the_value_of_score_ts() {
        let vectors: Vectors = serde_json::from_str(VECTORS).expect("the vectors file is JSON");
        assert_eq!(vectors.version, 1);
        assert!(vectors.cases.len() >= 200, "the file holds about 260 cases");
        for case in &vectors.cases {
            assert_eq!(
                score(&case.query, &case.text, &case.keywords),
                case.expected,
                "query {:?} against {:?} with {:?}",
                case.query,
                case.text,
                case.keywords
            );
        }
    }

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn a_title_beats_a_keyword_and_the_keyword_share_is_sixty_percent() {
        // "web" is a prefix match on the pool title; the browser only has the keyword.
        let pool = rank("web", "Web dev", &[]);
        let browser = rank("web", "Chrome", &words(&["browser", "web"]));
        assert!(pool > browser, "{pool} against {browser}");
        // 100 (exact keyword) - 5 = 95, times 0.6 = 57.
        assert_eq!(rank("web", "Chrome", &words(&["web"])), 57);
        assert_eq!(rank("zzz", "Chrome", &words(&["web"])), 0);
    }

    #[test]
    fn a_keyword_match_never_ranks_below_one() {
        // The multi-word fallback gives a small number for a weak match; 60 percent of it still
        // counts as a match.
        assert_eq!(rank("ab cd", "x", &words(&["ab cd"])), 57);
        assert!(rank("ab cd", "x", &words(&["ab", "cd"])) >= 1);
    }

    #[test]
    fn lengths_are_counted_in_utf16_units() {
        // A pair of surrogates is two units, so "a" + emoji is 3 units and closeness is
        // round(9 * 1 / 3) = 3 for the prefix "a".
        assert_eq!(score("a", "a\u{1F600}", &[]), PREFIX + 3);
    }

    #[test]
    fn javascript_spaces_are_not_rust_spaces() {
        assert!(is_js_space('\u{feff}'));
        assert!(is_js_space('\u{a0}'));
        assert!(!is_js_space('\u{85}'));
        // U+FEFF around the query is trimmed; U+0085 is not.
        assert_eq!(score("\u{feff}", "x", &[]), 1);
        assert_eq!(score("\u{85}", "x", &[]), 0);
    }

    #[test]
    fn an_empty_query_matches_everything_with_one() {
        assert_eq!(score("   ", "anything", &[]), 1);
    }
}
