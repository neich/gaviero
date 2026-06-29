//! Hand-rolled YAML frontmatter parser for skill markdown files.
//!
//! Parses a fixed subset only — no external YAML crate.

use std::collections::HashMap;

/// Split `---`-delimited frontmatter from the body.
///
/// Returns `(Some(frontmatter_text), body)` when well-formed, or
/// `(None, entire_contents)` when no valid frontmatter block is found.
pub fn split_frontmatter(contents: &str) -> (Option<&str>, &str) {
    let Some(trimmed) = contents.strip_prefix("---") else {
        return (None, contents);
    };
    let Some(rest) = trimmed
        .strip_prefix('\n')
        .or_else(|| trimmed.strip_prefix("\r\n"))
    else {
        return (None, contents);
    };
    let Some(end) = rest.find("\n---") else {
        return (None, contents);
    };
    let (fm, body) = rest.split_at(end);
    let body = body
        .strip_prefix("\n---")
        .and_then(|b| b.strip_prefix('\n').or(Some(b)))
        .unwrap_or("");
    (Some(fm), body)
}

/// Parse frontmatter key/value lines into a map.
pub fn parse_lines(fm: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in fm.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = strip_scalar_quotes(value.trim());
        map.insert(key, value);
    }
    map
}

fn strip_scalar_quotes(s: &str) -> String {
    if s.len() >= 2 {
        if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}

/// Parse the `arguments` field: YAML flow list `[a, b, c]` or space-separated scalar.
pub fn parse_arguments(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        let inner = &trimmed[1..trimmed.len() - 1];
        if inner.trim().is_empty() {
            return Vec::new();
        }
        return inner
            .split(',')
            .map(|s| strip_scalar_quotes(s.trim()))
            .filter(|s| !s.is_empty())
            .collect();
    }
    trimmed
        .split_whitespace()
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_well_formed_frontmatter() {
        let src = "---\nname: foo\ndescription: A test skill\n---\nBody here\n";
        let (fm, body) = split_frontmatter(src);
        let fm = fm.expect("frontmatter");
        assert!(fm.contains("name: foo"));
        assert_eq!(body, "Body here\n");
    }

    #[test]
    fn split_missing_frontmatter_returns_whole_file() {
        let src = "no frontmatter here";
        let (fm, body) = split_frontmatter(src);
        assert!(fm.is_none());
        assert_eq!(body, src);
    }

    #[test]
    fn parse_flow_list_arguments() {
        assert_eq!(
            parse_arguments("[alpha, beta, gamma]"),
            vec!["alpha", "beta", "gamma"]
        );
    }

    #[test]
    fn parse_space_separated_arguments() {
        assert_eq!(parse_arguments("one two three"), vec!["one", "two", "three"]);
    }

    #[test]
    fn parse_lines_normalizes_keys_to_lowercase() {
        let map = parse_lines("Name: Foo\nDescription: Bar skill\n");
        assert_eq!(map.get("name"), Some(&"Foo".to_string()));
        assert_eq!(map.get("description"), Some(&"Bar skill".to_string()));
    }

    #[test]
    fn strip_quotes_from_scalars() {
        assert_eq!(strip_scalar_quotes("\"hello\""), "hello");
        assert_eq!(strip_scalar_quotes("'world'"), "world");
    }
}
