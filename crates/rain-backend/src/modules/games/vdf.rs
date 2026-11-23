//! A small reader for Valve's text format (VDF), which Steam uses for `libraryfolders.vdf`,
//! `appmanifest_*.acf`, `localconfig.vdf` and `loginusers.vdf`. Section 02, 6.9.
//!
//! The format is `"key" "value"` pairs, and `"key" { ... }` blocks that hold more pairs, with
//! `//` comments. Inside quotes, `\\`, `\"`, `\n` and `\t` are escapes. Keys repeat and their case
//! varies between Steam versions, so a lookup ignores case and the file order is kept.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Vdf {
    Text(String),
    Block(Vec<(String, Vdf)>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VdfError(pub &'static str);

impl Vdf {
    /// The first child with this key, ignoring case. `None` for a text value.
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Self::Block(children) => children
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(key))
                .map(|(_, value)| value),
            Self::Text(_) => None,
        }
    }

    /// Follows a path of keys: `root.path(&["AppState", "appid"])`.
    pub fn path(&self, keys: &[&str]) -> Option<&Vdf> {
        keys.iter().try_fold(self, |node, key| node.get(key))
    }

    /// The text of this node, or `None` for a block.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Block(_) => None,
        }
    }

    /// The text of the child `key`.
    pub fn text_of(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Self::text)
    }

    /// Every child with its key; empty for a text value.
    pub fn children(&self) -> &[(String, Vdf)] {
        match self {
            Self::Block(children) => children,
            Self::Text(_) => &[],
        }
    }
}

/// Reads a whole file. The result is a block that holds the top-level pairs. A text that ends
/// inside a block or a string (Steam is rewriting the file) is an error, never a panic.
pub fn parse(text: &str) -> Result<Vdf, VdfError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut parser = Parser {
        chars: text.chars().collect(),
        at: 0,
    };
    let children = parser.pairs(0)?;
    Ok(Vdf::Block(children))
}

struct Parser {
    chars: Vec<char>,
    at: usize,
}

/// Blocks nested deeper than this are not a Steam file.
const MAX_DEPTH: usize = 32;

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn skip_space_and_comments(&mut self) {
        loop {
            while self.peek().is_some_and(char::is_whitespace) {
                self.at += 1;
            }
            if self.peek() == Some('/') && self.chars.get(self.at + 1) == Some(&'/') {
                while self.peek().is_some_and(|ch| ch != '\n') {
                    self.at += 1;
                }
            } else {
                return;
            }
        }
    }

    /// Pairs up to the closing `}` of this block (depth > 0) or the end of the text (depth 0).
    fn pairs(&mut self, depth: usize) -> Result<Vec<(String, Vdf)>, VdfError> {
        if depth > MAX_DEPTH {
            return Err(VdfError("nested too deep"));
        }
        let mut out = Vec::new();
        loop {
            self.skip_space_and_comments();
            match self.peek() {
                None if depth == 0 => return Ok(out),
                None => return Err(VdfError("the text ends inside a block")),
                Some('}') if depth > 0 => {
                    self.at += 1;
                    return Ok(out);
                }
                Some('}') => return Err(VdfError("a closing brace without a block")),
                Some(_) => {}
            }
            let key = self.string()?;
            self.skip_space_and_comments();
            let value = match self.peek() {
                Some('{') => {
                    self.at += 1;
                    Vdf::Block(self.pairs(depth + 1)?)
                }
                Some(_) => Vdf::Text(self.string()?),
                None => return Err(VdfError("a key without a value")),
            };
            out.push((key, value));
        }
    }

    /// A quoted string, or a bare word (Steam writes quotes everywhere, but old files do not).
    fn string(&mut self) -> Result<String, VdfError> {
        let mut out = String::new();
        if self.peek() == Some('"') {
            self.at += 1;
            loop {
                match self.peek() {
                    None => return Err(VdfError("the text ends inside a string")),
                    Some('"') => {
                        self.at += 1;
                        return Ok(out);
                    }
                    Some('\\') => {
                        self.at += 1;
                        match self.peek() {
                            Some('n') => out.push('\n'),
                            Some('t') => out.push('\t'),
                            Some('"') => out.push('"'),
                            Some('\\') => out.push('\\'),
                            // An escape that Steam does not use: keep it as written.
                            Some(other) => {
                                out.push('\\');
                                out.push(other);
                            }
                            None => return Err(VdfError("the text ends inside a string")),
                        }
                        self.at += 1;
                    }
                    Some(ch) => {
                        out.push(ch);
                        self.at += 1;
                    }
                }
            }
        }
        while let Some(ch) = self.peek() {
            if ch.is_whitespace() || ch == '{' || ch == '}' || ch == '"' {
                break;
            }
            out.push(ch);
            self.at += 1;
        }
        if out.is_empty() {
            return Err(VdfError("an unexpected character"));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"
"AppState"
{
	"appid"		"1000001"
	"name"		"Fixture's Quest"
	// a comment
	"installdir"		"Fixture Quest"
	"StateFlags"		"4"
	"UserConfig" { "language" "english" }
}
"#;

    #[test]
    fn a_manifest_is_read_with_nested_blocks() {
        let root = parse(MANIFEST).unwrap();
        assert_eq!(
            root.path(&["AppState", "appid"]).and_then(Vdf::text),
            Some("1000001")
        );
        assert_eq!(
            root.path(&["AppState", "name"]).and_then(Vdf::text),
            Some("Fixture's Quest")
        );
        assert_eq!(
            root.path(&["appstate", "userconfig", "LANGUAGE"])
                .and_then(Vdf::text),
            Some("english")
        );
    }

    #[test]
    fn a_bom_comments_and_escaped_backslashes_are_handled() {
        let text =
            "\u{feff}// header\n\"libraryfolders\" { \"0\" { \"path\" \"D:\\\\Steam\\\\Lib\" } }";
        let root = parse(text).unwrap();
        assert_eq!(
            root.path(&["libraryfolders", "0", "path"])
                .and_then(Vdf::text),
            Some("D:\\Steam\\Lib")
        );
    }

    #[test]
    fn duplicate_keys_keep_their_order_and_the_first_wins_a_lookup() {
        let root = parse("\"a\" \"1\" \"a\" \"2\"").unwrap();
        assert_eq!(root.children().len(), 2);
        assert_eq!(root.text_of("a"), Some("1"));
    }

    #[test]
    fn a_half_written_file_is_an_error_not_a_panic() {
        for cut in [
            "\"AppState\" {",
            "\"AppState\" { \"appid\" \"10",
            "\"AppState\" { \"appid\"",
            "}",
            "\"a",
        ] {
            assert!(parse(cut).is_err(), "{cut}");
        }
        assert!(parse(&MANIFEST[..MANIFEST.len() / 2]).is_err());
    }

    #[test]
    fn an_empty_text_is_an_empty_block() {
        assert_eq!(parse("").unwrap(), Vdf::Block(Vec::new()));
    }

    #[test]
    fn bare_words_are_accepted() {
        let root = parse("key value").unwrap();
        assert_eq!(root.text_of("KEY"), Some("value"));
    }
}
