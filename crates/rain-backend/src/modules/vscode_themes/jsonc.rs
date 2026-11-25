//! JSON with comments and trailing commas, the format of VS Code theme and settings files.
//! `parse` removes both and hands the rest to serde_json. Pure.

use serde_json::Value;

/// Reads a JSONC text. A byte order mark at the start is ignored.
pub fn parse(text: &str) -> Result<Value, serde_json::Error> {
    serde_json::from_str(&clean(text))
}

/// The text with `//` and `/* */` comments and the commas before `}` and `]` taken out. Text
/// inside strings is never touched.
fn clean(text: &str) -> String {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                // Copy the whole string, honouring the backslash.
                out.push(c);
                i += 1;
                while i < chars.len() {
                    let s = chars[i];
                    out.push(s);
                    i += 1;
                    if s == '\\' && i < chars.len() {
                        out.push(chars[i]);
                        i += 1;
                    } else if s == '"' {
                        break;
                    }
                }
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i = skip_block_comment(&chars, i);
                // A comment separates tokens like a space does.
                out.push(' ');
            }
            ',' if next_is_closer(&chars, i + 1) => i += 1,
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// The index after the `/* ... */` that starts at `from`.
fn skip_block_comment(chars: &[char], from: usize) -> usize {
    let mut i = from + 2;
    while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
        i += 1;
    }
    i + 2
}

/// True when the next token after `from` (skipping space and comments) is `}` or `]`.
fn next_is_closer(chars: &[char], from: usize) -> bool {
    let mut i = from;
    while i < chars.len() {
        match chars[i] {
            c if c.is_whitespace() => i += 1,
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => i = skip_block_comment(chars, i),
            '}' | ']' => return true,
            _ => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plain_json_is_unchanged() {
        assert_eq!(
            parse(r#"{"a": [1, 2], "b": "x"}"#).unwrap(),
            json!({"a": [1, 2], "b": "x"})
        );
    }

    #[test]
    fn line_and_block_comments_are_dropped() {
        let text = "{\n // the theme\n \"a\": 1, /* inline */ \"b\": 2 // end\n}";
        assert_eq!(parse(text).unwrap(), json!({"a": 1, "b": 2}));
    }

    #[test]
    fn trailing_commas_are_dropped() {
        let text = "{\"a\": [1, 2,], \"b\": {\"c\": 3,},}";
        assert_eq!(parse(text).unwrap(), json!({"a": [1, 2], "b": {"c": 3}}));
    }

    #[test]
    fn a_comma_before_a_comment_before_the_closer_is_dropped() {
        let text = "{\"a\": 1, // last\n}";
        assert_eq!(parse(text).unwrap(), json!({"a": 1}));
    }

    #[test]
    fn comment_marks_inside_strings_stay() {
        let text =
            r#"{"url": "http://x//y", "t": "/* not a comment */", "q": "a\"//b", "c": ",}"}"#;
        assert_eq!(
            parse(text).unwrap(),
            json!({"url": "http://x//y", "t": "/* not a comment */", "q": "a\"//b", "c": ",}"})
        );
    }

    #[test]
    fn a_byte_order_mark_is_ignored() {
        assert_eq!(parse("\u{feff}{\"a\": 1}").unwrap(), json!({"a": 1}));
    }

    #[test]
    fn broken_text_is_an_error() {
        assert!(parse("{\"a\": ").is_err());
        assert!(parse("").is_err());
    }
}
