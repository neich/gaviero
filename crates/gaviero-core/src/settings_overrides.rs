//! Which keys of one `.gaviero/settings.json` layer override, or are
//! overridden by, the other layers — for the editor's settings highlights.
//!
//! Layers run broadest first: global (`$GAVIERO_ROOT/.gaviero`), workspace,
//! repo ([`crate::workspace::LayeredSettings`]). A key counts when another
//! layer sets the same dotted path, looked up like
//! [`crate::workspace::Workspace::resolve_setting`] does (nested and
//! flattened `"a.b"` keys alike). Only keys whose value is not an object are
//! compared: those are the values a lookup actually returns. Keys inside
//! arrays are not settings paths and are skipped.
//!
//! A `"key+"` adds to `key` rather than replacing it
//! ([`crate::workspace::layered_value`]): it is compared as `key`, its own
//! value counts even when it is an object, and the marks say "adds to" /
//! "extended by" instead of "overrides" / "overridden by".

use serde_json::Value;

/// One object key in a settings file, positioned in characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonKeySpan {
    /// Dotted settings path (`agent.permissions.read.deny`).
    pub path: String,
    /// 0-based line of the key.
    pub line: usize,
    /// Char column of the opening quote.
    pub start_col: usize,
    /// Char column just past the closing quote.
    pub end_col: usize,
    /// The key's value starts with `{`.
    pub is_object: bool,
}

/// A key to highlight in a settings layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyMark {
    pub line: usize,
    pub start_col: usize,
    pub end_col: usize,
    /// The key as written, `+` included.
    pub path: String,
    /// Broader layers this key overrides (`global`, `workspace`).
    pub overrides: Vec<String>,
    /// More specific layers that override this key (`repo gaviero`, …).
    pub overridden_by: Vec<String>,
    /// Broader layers this `"key+"` adds to.
    pub extends: Vec<String>,
    /// More specific layers that add to this key with `"key+"`.
    pub extended_by: Vec<String>,
}

impl KeyMark {
    /// The end-of-line note, e.g. `← overridden by repo gaviero`,
    /// `← extended by repo gaviero`, `overrides global`, `adds to global`.
    pub fn hint(&self) -> String {
        let mut parts = Vec::new();
        if !self.overridden_by.is_empty() {
            parts.push(format!("← overridden by {}", self.overridden_by.join(", ")));
        }
        if !self.extended_by.is_empty() {
            parts.push(format!("← extended by {}", self.extended_by.join(", ")));
        }
        if !self.overrides.is_empty() {
            parts.push(format!("overrides {}", self.overrides.join(", ")));
        }
        if !self.extends.is_empty() {
            parts.push(format!("adds to {}", self.extends.join(", ")));
        }
        parts.join(" · ")
    }
}

/// The keys of `text` that another layer also sets. `upper` are the broader
/// layers, `lower` the more specific ones, each `(label, parsed file)`.
pub fn override_marks(text: &str, upper: &[(String, Value)], lower: &[(String, Value)]) -> Vec<KeyMark> {
    let sets = |doc: &Value, path: &str| crate::workspace::dot_get(doc, path).is_some();
    let labels = |layers: &[(String, Value)], path: &str| -> Vec<String> {
        layers
            .iter()
            .filter(|(_, doc)| sets(doc, path))
            .map(|(label, _)| label.clone())
            .collect()
    };
    json_key_spans(text)
        .into_iter()
        // Keys inside a `"key+": { … }` addition are not settings paths.
        .filter(|span| !span.path.contains("+."))
        .filter_map(|span| {
            let (key, appends) = match span.path.strip_suffix('+') {
                Some(key) => (key.to_string(), true),
                None => (span.path.clone(), false),
            };
            if span.is_object && !appends {
                return None;
            }
            let plus = format!("{key}+");
            let mut broader = labels(upper, &key);
            for label in labels(upper, &plus) {
                if !broader.contains(&label) {
                    broader.push(label);
                }
            }
            let (overrides, extends) = if appends {
                (Vec::new(), broader)
            } else {
                (broader, Vec::new())
            };
            let overridden_by = labels(lower, &key);
            let extended_by = labels(lower, &plus);
            let marked = [&overrides, &extends, &overridden_by, &extended_by]
                .iter()
                .any(|l| !l.is_empty());
            marked.then_some(KeyMark {
                line: span.line,
                start_col: span.start_col,
                end_col: span.end_col,
                path: span.path,
                overrides,
                overridden_by,
                extends,
                extended_by,
            })
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Open(bool),  // `{` (false) or `[` (true)
    Close,       // `}` or `]`
    Colon,
    Comma,
    Str(String),
    Other,
}

struct Token {
    tok: Tok,
    line: usize,
    start_col: usize,
    end_col: usize,
}

/// Tokenize leniently: an unterminated string ends at the line break, and
/// `//` / `/* */` comments are skipped, so a half-edited file still yields
/// the keys before the damage.
fn tokenize(text: &str) -> Vec<Token> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let (mut i, mut line, mut col) = (0usize, 0usize, 0usize);
    while i < chars.len() {
        let c = chars[i];
        let start = col;
        match c {
            '\n' => {
                line += 1;
                col = 0;
                i += 1;
                continue;
            }
            c if c.is_whitespace() => {}
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                i += 2;
                col += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    if chars[i] == '\n' {
                        line += 1;
                        col = 0;
                    } else {
                        col += 1;
                    }
                    i += 1;
                }
                i += 2;
                col += 2;
                continue;
            }
            '"' => {
                let mut s = String::new();
                i += 1;
                col += 1;
                while i < chars.len() && chars[i] != '"' && chars[i] != '\n' {
                    if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] != '\n' {
                        s.push(chars[i + 1]);
                        i += 2;
                        col += 2;
                    } else {
                        s.push(chars[i]);
                        i += 1;
                        col += 1;
                    }
                }
                if chars.get(i) == Some(&'"') {
                    i += 1;
                    col += 1;
                }
                out.push(Token {
                    tok: Tok::Str(s),
                    line,
                    start_col: start,
                    end_col: col,
                });
                continue;
            }
            _ => {
                let tok = match c {
                    '{' => Tok::Open(false),
                    '[' => Tok::Open(true),
                    '}' | ']' => Tok::Close,
                    ':' => Tok::Colon,
                    ',' => Tok::Comma,
                    _ => Tok::Other,
                };
                out.push(Token {
                    tok,
                    line,
                    start_col: start,
                    end_col: start + 1,
                });
            }
        }
        i += 1;
        col += 1;
    }
    out
}

/// Every object key in `text` outside arrays, with its dotted path.
pub fn json_key_spans(text: &str) -> Vec<JsonKeySpan> {
    struct Frame {
        is_array: bool,
        name: Option<String>,
    }
    let tokens = tokenize(text);
    let mut stack: Vec<Frame> = Vec::new();
    let mut last_key: Option<String> = None;
    let mut out = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        match &token.tok {
            Tok::Open(is_array) => {
                let after_colon = i > 0 && tokens[i - 1].tok == Tok::Colon;
                stack.push(Frame {
                    is_array: *is_array,
                    name: if after_colon { last_key.take() } else { None },
                });
            }
            Tok::Close => {
                stack.pop();
            }
            Tok::Str(key)
                if tokens.get(i + 1).is_some_and(|t| t.tok == Tok::Colon)
                    && stack.last().is_some_and(|f| !f.is_array)
                    && (i == 0 || tokens[i - 1].tok != Tok::Colon) =>
            {
                last_key = Some(key.clone());
                if stack.iter().any(|f| f.is_array) {
                    continue;
                }
                let mut path: Vec<&str> = stack.iter().filter_map(|f| f.name.as_deref()).collect();
                path.push(key);
                out.push(JsonKeySpan {
                    path: path.join("."),
                    line: token.line,
                    start_col: token.start_col,
                    end_col: token.end_col,
                    is_object: tokens.get(i + 2).is_some_and(|t| t.tok == Tok::Open(false)),
                });
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_spans_carry_dotted_paths_and_positions() {
        let text = "{\n  \"agent\": {\n    \"availableTools\": [\"Read\"],\n    \"permissions\": { \"read\": { \"deny\": [] } }\n  },\n  \"mcp.extraServers\": [{\"name\": \"x\"}]\n}";
        let spans = json_key_spans(text);
        let paths: Vec<&str> = spans.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "agent",
                "agent.availableTools",
                "agent.permissions",
                "agent.permissions.read",
                "agent.permissions.read.deny",
                "mcp.extraServers",
            ]
        );
        let tools = &spans[1];
        assert_eq!((tools.line, tools.start_col, tools.end_col), (2, 4, 20));
        assert!(spans[0].is_object && !tools.is_object);
    }

    #[test]
    fn key_spans_survive_a_half_edited_file() {
        let text = "{\n  // note\n  \"a\": 1,\n  \"b\": \"unterminated\n  \"c\": true\n";
        let paths: Vec<String> = json_key_spans(text).into_iter().map(|s| s.path).collect();
        assert_eq!(paths, ["a", "b", "c"]);
    }

    #[test]
    fn marks_name_the_overriding_and_overridden_layers() {
        let text = "{\n  \"agent\": { \"availableTools\": [], \"model\": \"claude:opus\" },\n  \"editor.tabSize\": 2,\n  \"only\": 1\n}";
        let upper = vec![("global".to_string(), json!({"agent": {"availableTools": ["Read"]}}))];
        let lower = vec![
            ("repo app".to_string(), json!({"editor": {"tabSize": 4}})),
            ("repo lib".to_string(), json!({"agent.availableTools": ["Bash"]})),
        ];
        let marks = override_marks(text, &upper, &lower);
        assert_eq!(marks.len(), 2);
        assert_eq!(marks[0].path, "agent.availableTools");
        assert_eq!(marks[0].overrides, ["global"]);
        assert_eq!(marks[0].overridden_by, ["repo lib"]);
        assert_eq!(
            marks[0].hint(),
            "← overridden by repo lib · overrides global"
        );
        assert_eq!(marks[1].path, "editor.tabSize");
        assert_eq!(marks[1].line, 2);
        assert_eq!(marks[1].hint(), "← overridden by repo app");
    }

    #[test]
    fn plus_keys_add_to_and_are_extended_by_other_layers() {
        // The workspace layer: one plain key, one addition.
        let text = "{\n  \"mcp\": { \"extraServers\": [] },\n  \"files.exclude+\": { \"target\": true }\n}";
        let upper = vec![("global".to_string(), json!({"files": {"exclude": {"x": true}}}))];
        let lower = vec![
            ("repo app".to_string(), json!({"mcp.extraServers+": [{"name": "a"}]})),
            ("repo lib".to_string(), json!({"files": {"exclude": {}}})),
        ];
        let marks = override_marks(text, &upper, &lower);
        assert_eq!(marks.len(), 2, "{marks:?}");
        assert_eq!(marks[0].path, "mcp.extraServers");
        assert_eq!(marks[0].extended_by, ["repo app"]);
        assert!(marks[0].overridden_by.is_empty() && marks[0].overrides.is_empty());
        assert_eq!(marks[0].hint(), "← extended by repo app");
        // An object-valued addition is marked, and its own keys are not.
        assert_eq!(marks[1].path, "files.exclude+");
        assert_eq!(marks[1].extends, ["global"]);
        assert_eq!(marks[1].overridden_by, ["repo lib"]);
        assert_eq!(marks[1].hint(), "← overridden by repo lib · adds to global");
    }
}
