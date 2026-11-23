//! The Moment-style date format of Obsidian's daily notes, in the direction date to text.
//! Section 04 6.3.
//!
//! Obsidian names a daily note with a Moment.js format such as `YYYY-MM-DD`. Rain only ever
//! needs to turn a date into that name (it never parses a name back into a date), so this is a
//! small formatter with English names. A token Rain cannot render is an error that names the
//! token, so Settings can say what to change; a wrong guess would make Rain look for notes that
//! do not exist.

use std::fmt;

use super::dates::CivilDate;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MomentError {
    /// The token (or the lone `[`) that Rain cannot render.
    Unsupported(String),
}

impl fmt::Display for MomentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self::Unsupported(token) = self;
        write!(
            f,
            "The daily note format uses {token}, which Rain cannot read. Change it in Settings."
        )
    }
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
/// Monday first, like `CivilDate::weekday`.
const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// Letters that Moment treats as tokens. A run of one of these letters that this file does not
/// know is refused, never copied (Moment would render it as something else).
const TOKEN_LETTERS: &str = "YMDdEWGQAaHhkmsSZzXxweglLNy";

/// Renders `format` for `date`. `[text]` is copied without the brackets; letters that are not
/// Moment tokens and all other characters are copied as they are.
pub fn render(format: &str, date: CivilDate) -> Result<String, MomentError> {
    let chars: Vec<char> = format.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '[' {
            // A literal runs to the next `]`.
            let Some(close) = chars[i + 1..].iter().position(|&x| x == ']') else {
                return Err(MomentError::Unsupported("[".into()));
            };
            out.extend(&chars[i + 1..i + 1 + close]);
            i += close + 2;
            continue;
        }
        if !c.is_ascii_alphabetic() {
            out.push(c);
            i += 1;
            continue;
        }
        // A run of one letter, taken whole.
        let run = chars[i..].iter().take_while(|&&x| x == c).count();
        // `Do` is a D followed by an `o`.
        if c == 'D' && run == 1 && chars.get(i + 1) == Some(&'o') {
            out.push_str(&ordinal(date.d));
            i += 2;
            continue;
        }
        if TOKEN_LETTERS.contains(c) {
            let token: String = chars[i..i + run].iter().collect();
            match token_text(&token, date) {
                Some(text) => out.push_str(&text),
                None => return Err(MomentError::Unsupported(token)),
            }
        } else {
            out.extend(&chars[i..i + run]);
        }
        i += run;
    }
    Ok(out)
}

/// The text of a token, or `None` for a token Rain does not render.
fn token_text(token: &str, date: CivilDate) -> Option<String> {
    let (week, week_year) = date.iso_week_and_year();
    let weekday_name = WEEKDAYS[usize::from(date.weekday())];
    Some(match token {
        "YYYY" => format!("{:04}", date.y),
        "YY" => format!("{:02}", date.y.rem_euclid(100)),
        "MMMM" => MONTHS[usize::from(date.m) - 1].to_owned(),
        "MMM" => MONTHS[usize::from(date.m) - 1][..3].to_owned(),
        "MM" => format!("{:02}", date.m),
        "M" => date.m.to_string(),
        "DD" => format!("{:02}", date.d),
        "D" => date.d.to_string(),
        "dddd" => weekday_name.to_owned(),
        "ddd" => weekday_name[..3].to_owned(),
        "dd" => weekday_name[..2].to_owned(),
        "d" => ((date.weekday() + 1) % 7).to_string(), // Sunday = 0
        "E" => (date.weekday() + 1).to_string(),       // Monday = 1
        "WW" => format!("{week:02}"),
        "W" => week.to_string(),
        "GGGG" => format!("{week_year:04}"),
        "GG" => format!("{:02}", week_year.rem_euclid(100)),
        _ => return None,
    })
}

/// 1st, 2nd, 3rd, 4th ... 11th, 12th, 13th ... 21st, 22nd, 23rd ... 31st.
fn ordinal(day: u8) -> String {
    let suffix = match (day % 100, day % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    };
    format!("{day}{suffix}")
}

/// The check that Settings runs before it saves a format (`life::Service::check_note_format`):
/// the format must render, and must not render to nothing.
pub fn validate(format: &str) -> Result<(), String> {
    let probe = CivilDate {
        y: 2026,
        m: 11,
        d: 22,
    };
    match render(format, probe) {
        Err(error) => Err(error.to_string()),
        Ok(name) if name.trim().is_empty() => {
            Err("The daily note format gives an empty name. Change it in Settings.".into())
        }
        Ok(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> CivilDate {
        CivilDate::parse(text).expect("a real date")
    }

    /// The vectors of the design (`tests/fixtures/life/moment-vectors.txt`): one per line,
    /// `format | date | expected`. An expected text that starts with `!` is the token that
    /// must be refused.
    #[test]
    fn the_vectors_of_the_design_render_as_written() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/life/moment-vectors.txt");
        let text = std::fs::read_to_string(path).expect("the vector file");
        let mut checked = 0;
        for line in text
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        {
            let parts: Vec<&str> = line.split(" | ").collect();
            assert_eq!(parts.len(), 3, "a vector has three parts: {line}");
            let got = render(parts[0], date(parts[1]));
            match parts[2].strip_prefix('!') {
                Some(token) => assert_eq!(
                    got,
                    Err(MomentError::Unsupported(token.to_owned())),
                    "{line}"
                ),
                None => assert_eq!(got.as_deref(), Ok(parts[2]), "{line}"),
            }
            checked += 1;
        }
        assert!(checked >= 20, "the vector file was read ({checked} lines)");
    }

    #[test]
    fn every_unsupported_letter_is_refused_by_name() {
        for letter in "QAaHhkmsSZzXxweglLNy".chars() {
            let format = format!("YYYY-{letter}");
            assert_eq!(
                render(&format, date("2026-11-22")),
                Err(MomentError::Unsupported(letter.to_string())),
                "{letter}"
            );
        }
        for token in [
            "DDD", "DDDD", "Y", "YYY", "MMMMM", "EE", "WWW", "GGG", "ddddd",
        ] {
            assert_eq!(
                render(token, date("2026-11-22")),
                Err(MomentError::Unsupported(token.to_owned())),
                "{token}"
            );
        }
    }

    #[test]
    fn brackets_hold_literal_text() {
        assert_eq!(render("[YYYY]", date("2026-11-22")).as_deref(), Ok("YYYY"));
        assert_eq!(
            render("[Today is] dddd", date("2026-11-22")).as_deref(),
            Ok("Today is Sunday")
        );
        assert_eq!(
            render("[", date("2026-11-22")),
            Err(MomentError::Unsupported("[".into()))
        );
        assert_eq!(render("[]", date("2026-11-22")).as_deref(), Ok(""));
    }

    #[test]
    fn letters_that_are_not_tokens_and_symbols_are_copied() {
        assert_eq!(
            render("YYYY_MM-DD (b) b,c", date("2026-11-22")).as_deref(),
            Ok("2026_11-22 (b) b,c")
        );
    }

    #[test]
    fn ordinal_suffixes_for_every_day() {
        let expect = |day: u8, text: &str| assert_eq!(ordinal(day), text);
        for (day, text) in [
            (1, "1st"),
            (2, "2nd"),
            (3, "3rd"),
            (4, "4th"),
            (10, "10th"),
            (11, "11th"),
            (12, "12th"),
            (13, "13th"),
            (14, "14th"),
            (20, "20th"),
            (21, "21st"),
            (22, "22nd"),
            (23, "23rd"),
            (24, "24th"),
            (30, "30th"),
            (31, "31st"),
        ] {
            expect(day, text);
        }
    }

    #[test]
    fn validate_names_the_token_in_a_sentence() {
        assert_eq!(validate("YYYY-MM-DD"), Ok(()));
        let message = validate("YYYY-MM-DD HH:mm").unwrap_err();
        assert!(
            message.contains("HH") && message.contains("Settings"),
            "{message}"
        );
        assert!(validate("[]").is_err(), "an empty name is refused");
    }
}
