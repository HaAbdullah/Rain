//! Reading and setting ONE boolean property (`gym: true`) in the YAML frontmatter at the top of
//! a Markdown note, as text. Section 04 6.8, copied from the spike `frontmatter.rs` (Q8).
//!
//! This file is PURE: text in, text out. It has no access to the disk, so it cannot write
//! anything by construction (a rule of `tests/source_rules.rs` keeps it that way). The one
//! function that touches a note on disk is in `vault_write.rs`.
//!
//! Design decision: a careful LINE-BASED editor, not a YAML library.
//!   * A YAML library parses text into values and writes values back as text. Writing back
//!     re-formats the block: comments vanish, quoting and key order change, line endings
//!     change, the BOM may go. "Every other byte unchanged" cannot be met that way.
//!   * Rain only ever touches ONE token (`true` or `false`) or inserts ONE line, so it can be
//!     exact: find the line, find the token inside it, replace those bytes, copy the rest.
//!   * The price is that Rain must recognise its key line itself. It does that conservatively:
//!     whenever the text is not something it is sure about, it returns an error and writes
//!     nothing (`NotABoolean`, `DuplicateKey`, `UnreadableFrontmatter`). A refused tick is a
//!     better failure than a damaged note.
//!   * The tests use a real YAML parser (`yaml-rust2`, a dev-dependency only) as an independent
//!     ORACLE: after every edit, the parsed values of all other keys must be identical.

use std::fmt;

/// The byte order mark that some editors put in front of a note. It is kept as it is.
const BOM: &str = "\u{feff}";

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum NoteError {
    /// The property name has characters that could be confused with YAML syntax.
    InvalidKey,
    /// The key appears more than once at the top level of the frontmatter.
    DuplicateKey,
    /// The key exists but its value is not a plain `true` or `false` (a string, a list, empty,
    /// a block scalar ...).
    NotABoolean,
    /// The note starts with `---` and has a closing `---`, but the lines between do not look
    /// like YAML properties.
    UnreadableFrontmatter,
}

impl fmt::Display for NoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            NoteError::InvalidKey => "property name contains characters Rain will not write",
            NoteError::DuplicateKey => "property appears more than once in the note",
            NoteError::NotABoolean => "property exists but is not a true/false value",
            NoteError::UnreadableFrontmatter => {
                "the top of the note does not look like YAML properties"
            }
        })
    }
}

impl std::error::Error for NoteError {}

// ---------------------------------------------------------------------------------------------
// Public API (pure functions)
// ---------------------------------------------------------------------------------------------

/// Value of the boolean property `key`.
/// `Ok(None)`: no frontmatter, key absent, or the value is not a plain true or false. (A heat
/// map then shows "not done".) Only the tests read a single key; the program reads all of them
/// with `read_bools`.
#[cfg(test)]
pub fn read_bool(note: &str, key: &str) -> Result<Option<bool>, NoteError> {
    check_key(key)?;
    let body = note.strip_prefix(BOM).unwrap_or(note);
    let lines = split_lines(body);
    match classify(&lines) {
        Fm::Block { close } => Ok(match find_key(&lines, close, key)? {
            Some(Hit {
                val: Val::Bool(b), ..
            }) => Some(b),
            _ => None,
        }),
        _ => Ok(None),
    }
}

/// Every top-level property of the frontmatter whose value is a plain `true` or `false`
/// (also `True`, `TRUE`, `False`, `FALSE`), in the order of the note, keys as written (case
/// kept, quotes removed). Never fails: a note must not break the check.
///   * a key that occurs twice is left out,
///   * a value that is not a plain boolean (`yes`, a string, a number) is left out,
///   * nested keys and body lines are ignored,
///   * no frontmatter, an unterminated fence or an implausible block give an empty list.
pub fn read_bools(note: &str) -> Vec<(String, bool)> {
    let body = note.strip_prefix(BOM).unwrap_or(note);
    let lines = split_lines(body);
    let Fm::Block { close } = classify(&lines) else {
        return Vec::new();
    };
    // Every top-level key with its value, in order. The same key twice shows up twice.
    let mut keys: Vec<(&str, Val)> = Vec::new();
    for line in &lines[1..close] {
        if line.content.starts_with([' ', '\t']) {
            continue; // a continuation or a nested key
        }
        if let Some((Key::Plain(name), after_colon)) = parse_key(line.content) {
            keys.push((name, parse_value(line.content, after_colon).0));
        }
    }
    keys.iter()
        .filter(|(name, _)| keys.iter().filter(|(other, _)| other == name).count() == 1)
        .filter_map(|(name, val)| match val {
            Val::Bool(b) => Some(((*name).to_owned(), *b)),
            _ => None,
        })
        .collect()
}

/// Return `note` with property `key` set to `value`. Only these three edits ever happen:
///   1. the key exists with a boolean value: that one token is replaced (comment, spacing,
///      quoting of the key, line ending all stay). Already equal: the note is returned
///      unchanged, even `True` stays `True`.
///   2. the note has frontmatter without the key: one line `key: true` is inserted just before
///      the closing `---`.
///   3. the note has no frontmatter: a block `---`, `key: true`, `---` is put at the very top
///      (after a BOM).
///
/// The line ending used for new lines is the one the note already uses (the opening fence's, or
/// the first line ending found; LF if none).
pub fn set_bool(note: &str, key: &str, value: bool) -> Result<String, NoteError> {
    check_key(key)?;
    let (bom, body) = match note.strip_prefix(BOM) {
        Some(rest) => (BOM, rest),
        None => ("", note),
    };
    let lines = split_lines(body);
    let word = if value { "true" } else { "false" };

    match classify(&lines) {
        Fm::Absent => {
            let eol = lines
                .first()
                .map(|l| l.eol)
                .filter(|e| !e.is_empty())
                .unwrap_or("\n");
            Ok(format!("{bom}---{eol}{key}: {word}{eol}---{eol}{body}"))
        }
        Fm::Implausible => Err(NoteError::UnreadableFrontmatter),
        Fm::Block { close } => match find_key(&lines, close, key)? {
            Some(Hit {
                val: Val::Bool(b), ..
            }) if b == value => Ok(note.to_string()),
            Some(Hit {
                val: Val::Bool(_),
                range,
            }) => Ok(format!(
                "{bom}{}{word}{}",
                &body[..range.start],
                &body[range.end..]
            )),
            Some(_) => Err(NoteError::NotABoolean),
            None => {
                // Insert before the closing fence, using the opening fence's own line ending.
                let eol = lines[0].eol;
                let at = lines[close].start;
                Ok(format!(
                    "{bom}{}{key}: {word}{eol}{}",
                    &body[..at],
                    &body[at..]
                ))
            }
        },
    }
}

/// Like `set_bool`, but it never adds anything: a note without frontmatter, or with
/// frontmatter that lacks the key, comes back unchanged. This is what unticking uses, so
/// that an untick of a property that is not there writes nothing. Every refusal of `set_bool`
/// is a refusal here too.
pub fn set_bool_existing(note: &str, key: &str, value: bool) -> Result<String, NoteError> {
    check_key(key)?;
    let body = note.strip_prefix(BOM).unwrap_or(note);
    let lines = split_lines(body);
    match classify(&lines) {
        Fm::Absent => Ok(note.to_string()),
        Fm::Implausible => Err(NoteError::UnreadableFrontmatter),
        Fm::Block { close } => match find_key(&lines, close, key)? {
            None => Ok(note.to_string()),
            Some(_) => set_bool(note, key, value),
        },
    }
}

/// Text of a brand new note that holds only these properties. `eol` is "\n" (Obsidian's
/// default) or "\r\n".
pub fn new_note(props: &[(&str, bool)], eol: &str) -> Result<String, NoteError> {
    let mut out = format!("---{eol}");
    for (i, (k, v)) in props.iter().enumerate() {
        check_key(k)?;
        if props[..i].iter().any(|(p, _)| p == k) {
            return Err(NoteError::DuplicateKey);
        }
        out.push_str(&format!("{k}: {v}{eol}"));
    }
    out.push_str(&format!("---{eol}"));
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------------------------

/// Property names Rain will read or write: no YAML syntax characters, no edge whitespace.
pub fn check_key(key: &str) -> Result<(), NoteError> {
    let bad_start =
        key.starts_with(|c: char| "-?:[]{}#&*!|>'\"%@`,".contains(c) || c.is_whitespace());
    let bad_any = key
        .chars()
        .any(|c| ":#[]{}\"'\\,&*!|>%@`".contains(c) || c.is_control());
    if key.is_empty() || bad_start || bad_any || key.ends_with(char::is_whitespace) {
        Err(NoteError::InvalidKey)
    } else {
        Ok(())
    }
}

struct Line<'a> {
    /// Byte offset of the line start inside the text that was split.
    start: usize,
    /// Text without the line ending.
    content: &'a str,
    /// "\n", "\r\n" or "" (last line without ending).
    eol: &'a str,
}

fn split_lines(text: &str) -> Vec<Line<'_>> {
    let mut out = Vec::new();
    let mut pos = 0;
    for raw in text.split_inclusive('\n') {
        let (content, eol) = if let Some(c) = raw.strip_suffix("\r\n") {
            (c, "\r\n")
        } else if let Some(c) = raw.strip_suffix('\n') {
            (c, "\n")
        } else {
            (raw, "")
        };
        out.push(Line {
            start: pos,
            content,
            eol,
        });
        pos += raw.len();
    }
    out
}

enum Fm {
    /// No frontmatter (also: starts with `---` but never closes, which Obsidian treats as
    /// plain text).
    Absent,
    /// Opening fence on line 0, closing fence on line `close`, plausible YAML lines between.
    Block { close: usize },
    /// Fences present but the lines between are not property-like.
    Implausible,
}

fn is_fence(content: &str, allow_dots: bool) -> bool {
    let t = content.trim_end_matches([' ', '\t']);
    t == "---" || (allow_dots && t == "...")
}

fn classify(lines: &[Line]) -> Fm {
    match lines.first() {
        Some(l) if is_fence(l.content, false) => {}
        _ => return Fm::Absent,
    }
    let Some(close) = (1..lines.len()).find(|&i| is_fence(lines[i].content, true)) else {
        return Fm::Absent;
    };
    if lines[1..close].iter().all(|l| plausible(l.content)) {
        Fm::Block { close }
    } else {
        Fm::Implausible
    }
}

/// Could this line belong to a top-level YAML mapping? (A blank line, a comment, an indented
/// continuation, a block-list item or a `key:` line.)
fn plausible(content: &str) -> bool {
    content.trim().is_empty()
        || content.starts_with([' ', '\t', '#'])
        || content == "-"
        || content.starts_with("- ")
        || parse_key(content).is_some()
}

enum Key<'a> {
    Plain(&'a str),
    /// A key Rain does not try to understand (escapes, `? ` complex keys ...). It can never
    /// match the key that is looked for.
    Complex,
}

/// Parse the key at the start of a line. Returns the key and the byte index just after the
/// colon.
fn parse_key(content: &str) -> Option<(Key<'_>, usize)> {
    let first = content.chars().next()?;
    if first == '"' || first == '\'' {
        let rest = &content[1..];
        let close = rest.find(first)?;
        let inner = &rest[..close];
        let after = 1 + close + 1;
        let tail = &content[after..];
        if (first == '"' && inner.contains('\\')) || (first == '\'' && tail.starts_with('\'')) {
            return Some((Key::Complex, 0));
        }
        let tail_trim = tail.trim_start_matches([' ', '\t']);
        if !tail_trim.starts_with(':') {
            return None;
        }
        let after_colon = after + (tail.len() - tail_trim.len()) + 1;
        if !colon_ends_key(&content[after_colon..]) {
            return None;
        }
        return Some((Key::Plain(inner), after_colon));
    }
    let colon = find_plain_colon(content)?;
    if "-?[]{}&*!|>%@`#,".contains(first) {
        return Some((Key::Complex, 0));
    }
    let key = content[..colon].trim_end();
    if key.is_empty() {
        None
    } else {
        Some((Key::Plain(key), colon + 1))
    }
}

fn colon_ends_key(after: &str) -> bool {
    after.is_empty() || after.starts_with([' ', '\t'])
}

/// First `:` that is followed by whitespace or the end of the line, before any ` #` comment.
fn find_plain_colon(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    for (i, &byte) in b.iter().enumerate() {
        match byte {
            b':' if colon_ends_key(&s[i + 1..]) => return Some(i),
            b'#' if i > 0 && (b[i - 1] == b' ' || b[i - 1] == b'\t') => return None,
            _ => {}
        }
    }
    None
}

#[derive(Debug, PartialEq, Eq)]
enum Val {
    Bool(bool),
    Empty,
    Other,
}

struct Hit {
    val: Val,
    /// Byte range of the value token inside the BODY text (the BOM is already stripped).
    range: std::ops::Range<usize>,
}

/// Find the single top-level line for `key` between the fences. Several matches is an error.
fn find_key(lines: &[Line], close: usize, key: &str) -> Result<Option<Hit>, NoteError> {
    let mut found: Option<Hit> = None;
    for l in &lines[1..close] {
        let Some((Key::Plain(name), after_colon)) = parse_key(l.content) else {
            continue;
        };
        if name != key {
            continue;
        }
        if found.is_some() {
            return Err(NoteError::DuplicateKey);
        }
        let (val, r) = parse_value(l.content, after_colon);
        found = Some(Hit {
            val,
            range: l.start + r.start..l.start + r.end,
        });
    }
    Ok(found)
}

/// Value token after the colon. A comment starts at a `#` that follows whitespace.
fn parse_value(content: &str, after_colon: usize) -> (Val, std::ops::Range<usize>) {
    let rest = &content[after_colon..];
    let vstart = after_colon + (rest.len() - rest.trim_start_matches([' ', '\t']).len());
    let b = content.as_bytes();
    let mut end = content.len();
    for i in vstart..content.len() {
        if b[i] == b'#' && (i == vstart || b[i - 1] == b' ' || b[i - 1] == b'\t') {
            end = i;
            break;
        }
    }
    let text = content[vstart..end].trim_end_matches([' ', '\t']);
    let val = match text {
        "" => Val::Empty,
        "true" | "True" | "TRUE" => Val::Bool(true),
        "false" | "False" | "FALSE" => Val::Bool(false),
        _ => Val::Other,
    };
    (val, vstart..vstart + text.len())
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use yaml_rust2::{Yaml, YamlLoader};

    // ---- explicit cases: (name, input, key, value, expected output) ----------------------
    #[test]
    fn exact_outputs() {
        let cases: &[(&str, &str, &str, bool, &str)] = &[
            (
                "flip false to true",
                "---\ngym: false\n---\nbody\n",
                "gym",
                true,
                "---\ngym: true\n---\nbody\n",
            ),
            (
                "flip true to false",
                "---\ngym: true\n---\nbody\n",
                "gym",
                false,
                "---\ngym: false\n---\nbody\n",
            ),
            (
                "already true: untouched",
                "---\ngym: true\n---\nbody\n",
                "gym",
                true,
                "---\ngym: true\n---\nbody\n",
            ),
            (
                "True stays True when asked true",
                "---\ngym: True\n---\n",
                "gym",
                true,
                "---\ngym: True\n---\n",
            ),
            (
                "True flips to false lowercase",
                "---\ngym: True\n---\n",
                "gym",
                false,
                "---\ngym: false\n---\n",
            ),
            (
                "CRLF kept",
                "---\r\ngym: false\r\nread: true\r\n---\r\nbody\r\n",
                "gym",
                true,
                "---\r\ngym: true\r\nread: true\r\n---\r\nbody\r\n",
            ),
            (
                "CRLF insert uses CRLF",
                "---\r\nread: true\r\n---\r\nbody\r\n",
                "gym",
                true,
                "---\r\nread: true\r\ngym: true\r\n---\r\nbody\r\n",
            ),
            (
                "LF insert uses LF",
                "---\nread: true\n---\nbody\n",
                "gym",
                false,
                "---\nread: true\ngym: false\n---\nbody\n",
            ),
            (
                "BOM kept (replace)",
                "\u{feff}---\ngym: false\n---\n",
                "gym",
                true,
                "\u{feff}---\ngym: true\n---\n",
            ),
            (
                "BOM kept (insert)",
                "\u{feff}---\na: 1\n---\n",
                "gym",
                true,
                "\u{feff}---\na: 1\ngym: true\n---\n",
            ),
            (
                "BOM kept (no frontmatter)",
                "\u{feff}hello\n",
                "gym",
                true,
                "\u{feff}---\ngym: true\n---\nhello\n",
            ),
            (
                "trailing comment kept",
                "---\ngym: false   # morning run\n---\n",
                "gym",
                true,
                "---\ngym: true   # morning run\n---\n",
            ),
            (
                "extra spacing kept",
                "---\ngym:    false\n---\n",
                "gym",
                true,
                "---\ngym:    true\n---\n",
            ),
            (
                "key order and comments kept",
                "---\n# top\nb: 1\n\ngym: false\n# mid\na: [x, y]\n---\n",
                "gym",
                true,
                "---\n# top\nb: 1\n\ngym: true\n# mid\na: [x, y]\n---\n",
            ),
            (
                "quoted key",
                "---\n\"gym\": false\n---\n",
                "gym",
                true,
                "---\n\"gym\": true\n---\n",
            ),
            (
                "single quoted key",
                "---\n'gym': false\n---\n",
                "gym",
                true,
                "---\n'gym': true\n---\n",
            ),
            (
                "space before colon",
                "---\ngym : false\n---\n",
                "gym",
                true,
                "---\ngym : true\n---\n",
            ),
            (
                "similar keys ignored",
                "---\ngymnast: false\ngym2: false\nmy gym: false\n---\n",
                "gym",
                true,
                "---\ngymnast: false\ngym2: false\nmy gym: false\ngym: true\n---\n",
            ),
            (
                "nested key ignored",
                "---\nobj:\n  gym: false\n---\n",
                "gym",
                true,
                "---\nobj:\n  gym: false\ngym: true\n---\n",
            ),
            (
                "key in body ignored",
                "---\na: 1\n---\ngym: false\n",
                "gym",
                true,
                "---\na: 1\ngym: true\n---\ngym: false\n",
            ),
            (
                "insert into empty frontmatter",
                "---\n---\nbody",
                "gym",
                true,
                "---\ngym: true\n---\nbody",
            ),
            (
                "no frontmatter, LF",
                "# Title\ntext\n",
                "gym",
                true,
                "---\ngym: true\n---\n# Title\ntext\n",
            ),
            (
                "no frontmatter, CRLF",
                "# Title\r\ntext\r\n",
                "gym",
                false,
                "---\r\ngym: false\r\n---\r\n# Title\r\ntext\r\n",
            ),
            ("empty file", "", "gym", true, "---\ngym: true\n---\n"),
            (
                "one line, no newline",
                "hello",
                "gym",
                true,
                "---\ngym: true\n---\nhello",
            ),
            (
                "no final newline kept",
                "---\ngym: false\n---\nlast line",
                "gym",
                true,
                "---\ngym: true\n---\nlast line",
            ),
            (
                "file ends right after closing fence",
                "---\ngym: false\n---",
                "gym",
                true,
                "---\ngym: true\n---",
            ),
            (
                "closing fence with dots",
                "---\ngym: false\n...\nbody\n",
                "gym",
                true,
                "---\ngym: true\n...\nbody\n",
            ),
            (
                "fences with trailing spaces",
                "---  \ngym: false\n---\t\n",
                "gym",
                true,
                "---  \ngym: true\n---\t\n",
            ),
            (
                "body horizontal rules untouched",
                "---\ngym: false\n---\ntext\n---\nmore\n---\n",
                "gym",
                true,
                "---\ngym: true\n---\ntext\n---\nmore\n---\n",
            ),
            (
                "mixed endings: only token changes",
                "---\ngym: false\r\nb: 1\n---\r\n",
                "gym",
                true,
                "---\ngym: true\r\nb: 1\n---\r\n",
            ),
            (
                "insert in mixed endings uses opening fence ending",
                "---\r\nb: 1\n---\n",
                "gym",
                true,
                "---\r\nb: 1\ngym: true\r\n---\n",
            ),
            (
                "unterminated --- treated as text: block prepended",
                "---\nsome text\n",
                "gym",
                true,
                "---\ngym: true\n---\n---\nsome text\n",
            ),
            (
                "block scalar and list values untouched",
                "---\ndesc: |\n  gym: false\n  more\ntags:\n  - a\n  - b\ngym: false\n---\n",
                "gym",
                true,
                "---\ndesc: |\n  gym: false\n  more\ntags:\n  - a\n  - b\ngym: true\n---\n",
            ),
            (
                "unicode values and emoji",
                "---\ntitle: \"Café ☕ 日本語\"\ngym: false\n---\nÜber\n",
                "gym",
                true,
                "---\ntitle: \"Café ☕ 日本語\"\ngym: true\n---\nÜber\n",
            ),
            (
                "hash glued to value is not a comment",
                "---\ngym: false\nnote: a#b\n---\n",
                "gym",
                true,
                "---\ngym: true\nnote: a#b\n---\n",
            ),
            (
                "key with dash",
                "---\nmorning-run: false\n---\n",
                "morning-run",
                true,
                "---\nmorning-run: true\n---\n",
            ),
        ];
        for (name, input, key, value, want) in cases {
            let got = set_bool(input, key, *value).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!(&got, want, "case: {name}");
            // every successful edit must be idempotent and readable
            assert_eq!(
                set_bool(&got, key, *value).unwrap(),
                got,
                "idempotent: {name}"
            );
            assert_eq!(
                read_bool(&got, key).unwrap(),
                Some(*value),
                "read back: {name}"
            );
        }
    }

    #[test]
    fn refusals_leave_everything_alone() {
        use NoteError::*;
        let cases: &[(&str, &str, NoteError)] = &[
            ("string value", "---\ngym: \"true\"\n---\n", NotABoolean),
            (
                "yes is not a YAML 1.2 bool",
                "---\ngym: yes\n---\n",
                NotABoolean,
            ),
            ("empty value", "---\ngym:\n---\n", NotABoolean),
            (
                "word after true",
                "---\ngym: true story\n---\n",
                NotABoolean,
            ),
            ("list value", "---\ngym: [true]\n---\n", NotABoolean),
            (
                "block scalar value",
                "---\ngym: |\n  true\n---\n",
                NotABoolean,
            ),
            (
                "value on next line",
                "---\ngym:\n  true\n---\n",
                NotABoolean,
            ),
            ("anchor/tag", "---\ngym: !!bool true\n---\n", NotABoolean),
            (
                "duplicate",
                "---\ngym: true\ngym: false\n---\n",
                DuplicateKey,
            ),
            (
                "duplicate quoted + plain",
                "---\ngym: true\n\"gym\": false\n---\n",
                DuplicateKey,
            ),
            (
                "hr, prose, hr is not properties",
                "---\nThis is a sentence.\n---\nbody\n",
                UnreadableFrontmatter,
            ),
            (
                "column-0 junk line",
                "---\ngym: true\nbroken line here\n---\n",
                UnreadableFrontmatter,
            ),
        ];
        for (name, input, want) in cases {
            assert_eq!(set_bool(input, "gym", true), Err(*want), "{name}");
        }
        for bad in [
            "", " gym", "gym ", "a:b", "a#b", "- x", "[x]", "a\"b", "a\nb", "{a}",
        ] {
            assert_eq!(set_bool("x", bad, true), Err(InvalidKey), "key {bad:?}");
            assert_eq!(read_bool("x", bad), Err(InvalidKey));
        }
    }

    #[test]
    fn read_values() {
        let r = |n: &str| read_bool(n, "gym").unwrap();
        assert_eq!(r("---\ngym: true\n---\n"), Some(true));
        assert_eq!(r("---\ngym: false # x\n---\n"), Some(false));
        assert_eq!(r("---\ngym: TRUE\n---\n"), Some(true));
        assert_eq!(r("---\nother: true\n---\n"), None);
        assert_eq!(r("gym: true\n"), None); // no fences
        assert_eq!(r("---\ngym: true\n"), None); // never closed
        assert_eq!(r("---\ngym: \"true\"\n---\n"), None);
        assert_eq!(r("---\nprose here\n---\n"), None); // unreadable frontmatter reads as nothing
        assert_eq!(r("\u{feff}---\r\ngym: true\r\n---\r\n"), Some(true));
        assert_eq!(
            read_bool("---\ngym: true\ngym: false\n---\n", "gym"),
            Err(NoteError::DuplicateKey)
        );
    }

    #[test]
    fn new_note_holds_only_the_properties() {
        assert_eq!(
            new_note(&[("gym", true), ("read", false)], "\n").unwrap(),
            "---\ngym: true\nread: false\n---\n"
        );
        assert_eq!(
            new_note(&[("gym", true)], "\r\n").unwrap(),
            "---\r\ngym: true\r\n---\r\n"
        );
        assert_eq!(new_note(&[], "\n").unwrap(), "---\n---\n");
        assert_eq!(
            new_note(&[("a", true), ("a", false)], "\n"),
            Err(NoteError::DuplicateKey)
        );
        assert_eq!(new_note(&[("a:b", true)], "\n"), Err(NoteError::InvalidKey));
        // and a created note can be ticked again
        let n = new_note(&[("gym", false)], "\n").unwrap();
        assert_eq!(set_bool(&n, "gym", true).unwrap(), "---\ngym: true\n---\n");
    }

    // ---- set_bool_existing (unticking) ---------------------------------------------------

    #[test]
    fn set_bool_existing_never_adds_a_property() {
        let same = |note: &str| {
            assert_eq!(
                set_bool_existing(note, "gym", false).unwrap(),
                note,
                "{note:?}"
            )
        };
        same(""); // empty file
        same("# Title\ntext\n"); // no frontmatter
        same("\u{feff}hello\n"); // BOM and no frontmatter
        same("---\nread: true\n---\nbody\n"); // frontmatter without the key
        same("---\r\nread: true\r\n---\r\n"); // the same with CRLF
        same("---\ngym: false\n---\n"); // already false
        same("---\nobj:\n  gym: true\n---\n"); // a nested key is not the key
    }

    #[test]
    fn set_bool_existing_edits_an_existing_property_like_set_bool() {
        let cases: &[(&str, &str)] = &[
            (
                "---\ngym: true\n---\nbody\n",
                "---\ngym: false\n---\nbody\n",
            ),
            (
                "---\r\ngym: true\r\nread: true\r\n---\r\n",
                "---\r\ngym: false\r\nread: true\r\n---\r\n",
            ),
            (
                "\u{feff}---\ngym: True\n---\n",
                "\u{feff}---\ngym: false\n---\n",
            ),
            (
                "---\ngym: true   # morning run\n---\n",
                "---\ngym: false   # morning run\n---\n",
            ),
            ("---\n\"gym\": true\n---\n", "---\n\"gym\": false\n---\n"),
        ];
        for (input, want) in cases {
            assert_eq!(set_bool_existing(input, "gym", false).unwrap(), *want);
            assert_eq!(
                set_bool_existing(input, "gym", false).unwrap(),
                set_bool(input, "gym", false).unwrap()
            );
        }
    }

    #[test]
    fn set_bool_existing_refuses_what_set_bool_refuses() {
        use NoteError::*;
        for (note, want) in [
            ("---\ngym: yes\n---\n", NotABoolean),
            ("---\ngym: true\ngym: false\n---\n", DuplicateKey),
            (
                "---\nThis is a sentence.\n---\nbody\n",
                UnreadableFrontmatter,
            ),
        ] {
            assert_eq!(set_bool_existing(note, "gym", false), Err(want), "{note:?}");
        }
        assert_eq!(set_bool_existing("x", "a:b", false), Err(InvalidKey));
    }

    // ---- read_bools ----------------------------------------------------------------------

    fn bools(note: &str) -> Vec<(String, bool)> {
        read_bools(note)
    }

    fn pair(name: &str, value: bool) -> (String, bool) {
        (name.to_owned(), value)
    }

    #[test]
    fn read_bools_finds_top_level_booleans_in_note_order() {
        assert_eq!(bools(""), vec![]);
        assert_eq!(bools("# Title\ngym: true\n"), vec![]); // no frontmatter
        assert_eq!(bools("---\ngym: true\nbody\n"), vec![]); // never closed
        assert_eq!(
            bools("---\ngym: true\nread: false\ntitle: x\n---\n"),
            vec![pair("gym", true), pair("read", false)]
        );
        assert_eq!(
            bools("---\na: True\nb: FALSE # note\nc: TRUE\nd: False\n---\n"),
            vec![
                pair("a", true),
                pair("b", false),
                pair("c", true),
                pair("d", false)
            ]
        );
    }

    #[test]
    fn read_bools_leaves_out_what_is_not_a_plain_boolean_once() {
        // a key twice is left out, a value that is not a plain boolean is left out
        assert_eq!(
            bools("---\ngym: true\ngym: false\nread: true\n---\n"),
            vec![pair("read", true)]
        );
        assert_eq!(
            bools("---\na: yes\nb: \"true\"\nc: 1\nd:\ne: [true]\nf: true story\ng: true\n---\n"),
            vec![pair("g", true)]
        );
        // nested keys and body lines are ignored
        assert_eq!(
            bools("---\nobj:\n  gym: true\nread: true\n---\ngym: false\n"),
            vec![pair("read", true)]
        );
    }

    #[test]
    fn read_bools_keeps_keys_as_written_and_removes_quotes() {
        assert_eq!(
            bools("---\n\"Gym Time\": true\n'read': false\nMorning-Run: true\n---\n"),
            vec![
                pair("Gym Time", true),
                pair("read", false),
                pair("Morning-Run", true)
            ]
        );
    }

    #[test]
    fn read_bools_handles_crlf_bom_and_implausible_blocks() {
        assert_eq!(
            bools("\u{feff}---\r\ngym: true\r\n---\r\nbody\r\n"),
            vec![pair("gym", true)]
        );
        assert_eq!(bools("---\nThis is a sentence.\n---\nbody\n"), vec![]);
        assert_eq!(bools("---\n---\n"), vec![]);
    }

    // ---- invariant checker + oracle ------------------------------------------------------

    /// Independent frontmatter extraction for the oracle (does not use the code under test).
    fn oracle_block(note: &str) -> Option<String> {
        let t = note.strip_prefix(BOM).unwrap_or(note);
        let ls: Vec<&str> = t
            .split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l))
            .collect();
        if ls.first().map(|l| l.trim_end()) != Some("---") {
            return None;
        }
        let end = ls[1..]
            .iter()
            .position(|l| matches!(l.trim_end(), "---" | "..."))?;
        Some(ls[1..1 + end].join("\n"))
    }

    /// key -> debug text of value, for every top-level key. None when the oracle cannot parse.
    fn oracle_map(note: &str) -> Option<Vec<(String, String)>> {
        let block = oracle_block(note)?;
        let docs = YamlLoader::load_from_str(&block).ok()?;
        match docs.first() {
            Some(Yaml::Hash(h)) => Some(
                h.iter()
                    .map(|(k, v)| (format!("{k:?}"), format!("{v:?}")))
                    .collect(),
            ),
            None => Some(vec![]), // empty block
            _ => None,
        }
    }

    fn oracle_gym(m: &[(String, String)]) -> Option<String> {
        m.iter()
            .find(|(k, _)| k == "String(\"gym\")")
            .map(|(_, v)| v.clone())
    }

    fn lines_of(s: &str) -> Vec<&str> {
        s.split_inclusive('\n').collect()
    }

    /// The core guarantee, checked structurally and with the oracle.
    fn check_invariants(input: &str, value: bool) {
        let out = match set_bool(input, "gym", value) {
            Ok(o) => o,
            Err(_) => return, // refusals are allowed; set_bool is pure so nothing was written
        };
        let word = if value { "true" } else { "false" };
        assert_eq!(
            read_bool(&out, "gym").unwrap(),
            Some(value),
            "read-back\nIN:{input:?}\nOUT:{out:?}"
        );
        assert_eq!(
            set_bool(&out, "gym", value).unwrap(),
            out,
            "idempotent\nIN:{input:?}"
        );

        // BOM is never added or removed
        assert_eq!(
            input.starts_with(BOM),
            out.starts_with(BOM),
            "bom\nIN:{input:?}"
        );

        if out != input {
            let (a, b) = (
                lines_of(input.strip_prefix(BOM).unwrap_or(input)),
                lines_of(out.strip_prefix(BOM).unwrap_or(&out)),
            );
            if b.len() == a.len() {
                // Replacement: exactly one line differs and only by the true/false token.
                let diff: Vec<usize> = (0..a.len()).filter(|&i| a[i] != b[i]).collect();
                assert_eq!(diff.len(), 1, "one changed line\nIN:{input:?}\nOUT:{out:?}");
                let (x, y) = (a[diff[0]], b[diff[0]]);
                let toks = ["true", "True", "TRUE", "false", "False", "FALSE"];
                let ok = (0..=x.len()).any(|i| {
                    toks.iter().any(|t1| {
                        x[i..].starts_with(t1) && {
                            let (pre, post) = (&x[..i], &x[i + t1.len()..]);
                            y.starts_with(pre)
                                && y.ends_with(post)
                                && &y[pre.len()..y.len() - post.len()] == word
                        }
                    })
                });
                assert!(ok, "only the token changed\nIN:{x:?}\nOUT:{y:?}");
            } else {
                // Insertion: lines were only added, original lines all still present in order.
                let mut it = b.iter();
                for l in &a {
                    assert!(
                        it.any(|m| m == l),
                        "original line lost {l:?}\nIN:{input:?}\nOUT:{out:?}"
                    );
                }
                let added = b.len() - a.len();
                assert!(
                    added == 1 || added == 3,
                    "added {added} lines\nIN:{input:?}\nOUT:{out:?}"
                );
            }
        }

        // The oracle: every other key keeps exactly its parsed value.
        if let (Some(before), Some(after)) = (oracle_map(input), oracle_map(&out)) {
            let strip = |m: &Vec<(String, String)>| {
                m.iter()
                    .filter(|(k, _)| k != "String(\"gym\")")
                    .cloned()
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                strip(&before),
                strip(&after),
                "other keys changed\nIN:{input:?}\nOUT:{out:?}"
            );
            let g = oracle_gym(&after).expect("gym parsed by oracle");
            assert_eq!(
                g,
                format!("Boolean({value})"),
                "oracle sees gym\nOUT:{out:?}"
            );
        } else if oracle_map(input).is_some() {
            panic!("oracle could parse input but not output\nIN:{input:?}\nOUT:{out:?}");
        }

        // Unticking through `set_bool_existing` agrees with `set_bool` whenever the key exists.
        if !value && read_bool(input, "gym").ok().flatten().is_some() {
            assert_eq!(set_bool_existing(input, "gym", false).unwrap(), out);
        }
    }

    // ---- randomized shapes ---------------------------------------------------------------
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
            &v[(self.next() % v.len() as u64) as usize]
        }
        fn chance(&mut self, pct: u64) -> bool {
            self.next() % 100 < pct
        }
    }

    fn gen_note(r: &mut Rng) -> String {
        let eol = *r.pick(&["\n", "\r\n"]);
        let mixed = r.chance(10);
        let le = |r: &mut Rng| {
            if mixed && r.chance(40) {
                if eol == "\n" { "\r\n" } else { "\n" }
            } else {
                eol
            }
        };
        let mut s = String::new();
        if r.chance(15) {
            s.push_str(BOM);
        }
        let shape = r.next() % 10;
        let body_lines = [
            "# Title",
            "some text",
            "",
            "gym: true",
            "---",
            "- [ ] task",
            "Übung ñ ☕",
            "```yaml",
            "gym: false",
            "***",
        ];
        if shape == 0 {
            // no frontmatter at all
        } else if shape == 1 {
            s.push_str("---");
            s.push_str(le(r)); // opening fence that never closes
            s.push_str("just text");
            s.push_str(le(r));
        } else {
            s.push_str(r.pick(&["---", "---", "---  "]));
            s.push_str(eol);
            let fm_lines: &[&str] = &[
                "# comment",
                "",
                "title: Some Title",
                "title: \"Quoted: value # not comment\"",
                "title: 'it''s'",
                "tags: [a, b]",
                "date: 2026-11-22",
                "url: http://x.y/z?a=b#frag",
                "n: 5",
                "other: true",
                "gymnast: true",
                "gym2: false",
                "my gym: true",
                "obj:\n  gym: true\n  z: 1",
                "list:\n  - a\n  - b",
                "list2:\n- a\n- b",
                "desc: |\n  line1\n  gym: false\n\n  line3",
                "desc2: >-\n  folded\n  text",
                "k: v # trailing",
                "emoji: \"☕ ok\"",
                "empty:",
            ];
            let n = r.next() % 8;
            for _ in 0..n {
                let l = *r.pick(fm_lines);
                for part in l.split('\n') {
                    s.push_str(part);
                    s.push_str(le(r));
                }
            }
            let gym_variants = [
                "",
                "",
                "gym: true",
                "gym: false",
                "gym:   true   # note",
                "gym: True",
                "gym: FALSE # x",
                "gym: \"true\"",
                "gym: yes",
                "gym:",
                "gym: truely",
                "\"gym\": true",
                "'gym': false",
                "gym : false",
                "gym: [true]",
                "gym: 1",
                "gym: true#glued",
            ];
            let g = *r.pick(&gym_variants);
            if !g.is_empty() {
                s.push_str(g);
                s.push_str(le(r));
            }
            if r.chance(10) {
                s.push_str("x: y");
                s.push_str(le(r)); // key after gym
            }
            s.push_str(r.pick(&["---", "---", "..."]));
            if r.chance(90) {
                s.push_str(le(r));
            }
        }
        let nb = r.next() % 5;
        for i in 0..nb {
            s.push_str(r.pick(&body_lines));
            if i + 1 < nb || r.chance(70) {
                s.push_str(le(r));
            }
        }
        s
    }

    /// `read_bools` against the YAML parser: everything it returns is a boolean for the oracle
    /// too, and a key that the note holds once on a line of its own is not missed.
    fn check_read_bools(note: &str) {
        let ours = read_bools(note);
        let Some(oracle) = oracle_map(note) else {
            return; // the oracle cannot parse this note: nothing to compare with
        };
        for (name, value) in &ours {
            let want = format!("Boolean({value})");
            let seen = oracle
                .iter()
                .find(|(k, _)| *k == format!("String({name:?})"))
                .map(|(_, v)| v.as_str());
            assert_eq!(
                seen,
                Some(want.as_str()),
                "read_bools: {name}\nNOTE:{note:?}"
            );
        }
        for (key, value) in &oracle {
            let name = key
                .strip_prefix("String(\"")
                .and_then(|k| k.strip_suffix("\")"));
            let Some(name) = name else { continue };
            if value != "Boolean(true)" && value != "Boolean(false)" {
                continue;
            }
            let on_own_line = note
                .lines()
                .filter(|line| line.trim_end().starts_with(&format!("{name}:")))
                .count();
            if on_own_line == 1 && ["gym", "other", "gymnast", "gym2"].contains(&name) {
                assert!(
                    ours.iter().any(|(n, _)| n == name),
                    "read_bools missed {name}\nNOTE:{note:?}"
                );
            }
        }
    }

    #[test]
    fn randomized_invariants_20000_notes() {
        let seed: u64 = std::env::var("RAIN_SEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        let mut r = Rng(seed);
        let (mut edited, mut refused, mut oracle_agree) = (0, 0, 0);
        for _ in 0..20_000 {
            let note = gen_note(&mut r);
            for value in [true, false] {
                match set_bool(&note, "gym", value) {
                    Ok(_) => edited += 1,
                    Err(_) => refused += 1,
                }
                check_invariants(&note, value);
            }
            check_read_bools(&note);
            // reader vs oracle: when the oracle can parse and sees gym as a bool, we must agree.
            if let (Some(m), Ok(ours)) = (oracle_map(&note), read_bool(&note, "gym"))
                && let Some(g) = oracle_gym(&m)
            {
                let want = if g == "Boolean(true)" {
                    Some(true)
                } else if g == "Boolean(false)" {
                    Some(false)
                } else {
                    None
                };
                assert_eq!(
                    ours, want,
                    "reader disagrees with YAML parser\nNOTE:{note:?}\noracle:{g}"
                );
                oracle_agree += 1;
            }
        }
        println!(
            "randomized: {edited} edits ok, {refused} refusals, {oracle_agree} reader/oracle agreements"
        );
        assert!(edited > 10_000 && refused > 100 && oracle_agree > 3_000);
    }

    /// The invariant check must be able to fail: a mutated editor is caught. Each mutation is
    /// a plausible bug; the checker has to notice every one of them.
    #[test]
    fn the_checker_catches_broken_editors() {
        let result = |f: fn(&str, &str, bool) -> String| {
            std::panic::catch_unwind(|| {
                let input = "---\r\ngym: false   # keep\r\nother: 1\r\n---\r\nbody\r\n";
                let out = f(input, "gym", true);
                // The same three checks `check_invariants` makes, on the mutated output.
                assert_eq!(out.matches("# keep").count(), 1, "comment kept");
                assert!(
                    out.contains("\r\n") && !out.contains("\r\r"),
                    "line ends kept"
                );
                assert_eq!(out.lines().count(), input.lines().count(), "no line added");
                assert!(out.contains("gym: true"), "ticked");
            })
            .is_err()
        };
        // 1: drops the trailing comment, 2: turns CRLF into LF, 3: adds a line instead of
        // replacing, 4: does nothing.
        assert!(result(|i, _, _| i
            .replace("   # keep", "")
            .replace("false", "true")));
        assert!(result(|i, _, _| i
            .replace("\r\n", "\n")
            .replace("false", "true")));
        assert!(result(
            |i, _, _| i.replace("---\r\nbody", "gym: true\r\n---\r\nbody")
        ));
        assert!(result(|i, _, _| i.to_owned()));
        // The real editor passes the same checks.
        assert!(!result(|i, k, v| set_bool(i, k, v).unwrap()));
    }
}
