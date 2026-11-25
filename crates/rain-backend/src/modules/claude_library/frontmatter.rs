//! The front matter of a markdown file: the `---` block at the top. Pure text code, no disk.
//!
//! This is a small reader for the part of YAML that skills, agents, commands and memory files use:
//! `key: value`, quoted values, block text (`>` and `|`), lists with `-`, and one level of nested
//! keys (`metadata:` with `type: user` below it, read as `metadata.type`). Anything it does not
//! understand is kept as plain text or skipped. It never fails.

/// A file split into its facts and its text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Parsed {
    /// In file order. A nested key is `parent.child`.
    pub facts: Vec<(String, String)>,
    /// The text after the front matter (all of the file when it has none).
    pub body: String,
}

impl Parsed {
    pub fn fact(&self, key: &str) -> Option<&str> {
        self.facts
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
            .filter(|value| !value.is_empty())
    }
}

/// The longest value of one fact.
const FACT_MAX_CHARS: usize = 600;

pub fn parse(text: &str) -> Parsed {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines: Vec<&str> = text.lines().collect();
    if lines.first().map(|line| line.trim_end()) != Some("---") {
        return Parsed {
            facts: Vec::new(),
            body: text.to_owned(),
        };
    }
    let Some(close) = lines
        .iter()
        .skip(1)
        .position(|line| matches!(line.trim_end(), "---" | "..."))
        .map(|at| at + 1)
    else {
        // An unclosed block: the head of a file that was cut. Read what is there as facts.
        return Parsed {
            facts: read_facts(&lines[1..]),
            body: String::new(),
        };
    };
    Parsed {
        facts: read_facts(&lines[1..close]),
        body: lines[close + 1..].join("\n"),
    }
}

fn indented(line: &str) -> bool {
    line.starts_with(' ') || line.starts_with('\t')
}

fn read_facts(lines: &[&str]) -> Vec<(String, String)> {
    let mut facts = Vec::new();
    let mut at = 0;
    while at < lines.len() {
        let line = lines[at];
        at += 1;
        if line.trim().is_empty() || line.trim_start().starts_with('#') || indented(line) {
            continue;
        }
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        // The lines that belong to this key: indented ones, and blank ones between them.
        let start = at;
        while at < lines.len() && (indented(lines[at]) || lines[at].trim().is_empty()) {
            at += 1;
        }
        let block: Vec<&str> = lines[start..at]
            .iter()
            .copied()
            .filter(|line| !line.trim().is_empty())
            .collect();
        let value = rest.trim();
        let block_text = matches!(value, ">" | "|" | ">-" | "|-" | ">+" | "|+");
        if value.is_empty() && !block.is_empty() && block.iter().all(|line| is_pair(line)) {
            for line in &block {
                if let Some((child, child_value)) = line.trim().split_once(':') {
                    facts.push((
                        format!("{key}.{}", child.trim()),
                        clean(&unquote(child_value.trim())),
                    ));
                }
            }
        } else if value.is_empty() || block_text {
            let joined = block
                .iter()
                .map(|line| line.trim().trim_start_matches("- ").trim())
                .collect::<Vec<_>>();
            let joiner = if value.is_empty() { ", " } else { " " };
            facts.push((key.to_owned(), clean(&joined.join(joiner))));
        } else {
            // A plain value; the indented lines below it continue the text.
            let mut text = unquote(value);
            for line in &block {
                text.push(' ');
                text.push_str(line.trim());
            }
            facts.push((key.to_owned(), clean(&text)));
        }
    }
    facts
}

/// `name: value` with a key that has no spaces (so a list entry or a sentence is not a pair).
fn is_pair(line: &str) -> bool {
    let line = line.trim();
    match line.split_once(':') {
        Some((key, _)) => !key.is_empty() && !key.contains(' ') && !line.starts_with('-'),
        None => false,
    }
}

fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 && bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"' {
        value[1..value.len() - 1]
            .replace("\\\"", "\"")
            .replace("\\n", " ")
            .replace("\\\\", "\\")
    } else if bytes.len() >= 2 && bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\'' {
        value[1..value.len() - 1].replace("''", "'")
    } else {
        value.to_owned()
    }
}

/// One line of text of a sane length.
fn clean(value: &str) -> String {
    let single = value.split_whitespace().collect::<Vec<_>>().join(" ");
    shorten(&single, FACT_MAX_CHARS)
}

/// At most `max` characters, with `...` at the end when something was cut.
pub fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let cut: String = text.chars().take(max.saturating_sub(3)).collect();
    format!("{}...", cut.trim_end())
}

/// A short line that tells what the text is about: the first line that is not blank, a rule or a
/// code fence, without the `#` of a heading. Used when a file has no `description`.
pub fn first_line(body: &str) -> String {
    body.lines()
        .map(|line| line.trim().trim_start_matches('#').trim())
        .find(|line| {
            !line.is_empty()
                && !line.starts_with("```")
                && !line.chars().all(|c| "-*_=".contains(c))
        })
        .map(|line| shorten(line, 160))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_pairs_quotes_and_nested_keys() {
        let parsed = parse(
            "---\nname: rain-project\ndescription: \"Vision, \\\"hard\\\" limits\"\nmetadata:\n  node_type: memory\n  type: project\n---\n# Body\ntext",
        );
        assert_eq!(parsed.fact("name"), Some("rain-project"));
        assert_eq!(parsed.fact("description"), Some("Vision, \"hard\" limits"));
        assert_eq!(parsed.fact("metadata.type"), Some("project"));
        assert_eq!(parsed.body, "# Body\ntext");
    }

    #[test]
    fn block_text_lists_and_continuation_lines() {
        let parsed = parse(
            "---\ndescription: >\n  first part\n  second part\ntools:\n  - Read\n  - Grep\nnote: one\n  and more\n---\n",
        );
        assert_eq!(parsed.fact("description"), Some("first part second part"));
        assert_eq!(parsed.fact("tools"), Some("Read, Grep"));
        assert_eq!(parsed.fact("note"), Some("one and more"));
    }

    #[test]
    fn a_file_without_front_matter_is_all_body() {
        let parsed = parse("# Title\n\nSome text\n");
        assert!(parsed.facts.is_empty());
        assert!(parsed.body.starts_with("# Title"));
    }

    #[test]
    fn crlf_and_a_byte_order_mark_are_handled() {
        let parsed = parse("\u{feff}---\r\nname: x\r\n---\r\nbody\r\n");
        assert_eq!(parsed.fact("name"), Some("x"));
        assert_eq!(parsed.body, "body");
    }

    #[test]
    fn an_unclosed_block_still_gives_its_facts() {
        let parsed = parse("---\nname: cut\ndescription: half a fi");
        assert_eq!(parsed.fact("name"), Some("cut"));
        assert!(parsed.body.is_empty());
    }

    #[test]
    fn long_values_are_cut() {
        let parsed = parse(&format!("---\ndescription: {}\n---\n", "a".repeat(2000)));
        let value = parsed.fact("description").unwrap();
        assert!(value.chars().count() <= FACT_MAX_CHARS);
        assert!(value.ends_with("..."));
    }

    #[test]
    fn the_first_line_skips_rules_and_fences() {
        assert_eq!(first_line("\n---\n```\n## The point\nmore"), "The point");
        assert_eq!(first_line(""), "");
    }
}
