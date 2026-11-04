//! Hand-rolled YAML frontmatter parser for skill markdown files.
//!
//! Parses a fixed subset only — no external YAML crate.

use std::collections::HashMap;

/// Split `---`-delimited frontmatter from the body.
///
/// Returns `(Some(frontmatter_text), body)` when well-formed, or
/// `(None, entire_contents)` when no valid frontmatter block is found.
pub fn split_frontmatter(contents: &str) -> (Option<&str>, &str) {
    // Editors on Windows often save with a UTF-8 BOM; blank lines before
    // the opening fence are tolerated too.
    let start = contents
        .trim_start_matches('\u{feff}')
        .trim_start_matches(['\r', '\n']);
    let Some(trimmed) = start.strip_prefix("---") else {
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

/// Parse top-level frontmatter keys into a map.
///
/// Only unindented `key: value` lines are keys; the indented lines after one
/// belong to it. That covers block scalars (`description: |` keeps line
/// breaks, `>` folds them), block lists (`- a` lines become `[a, b]`), and
/// plain values wrapped onto further lines. A nested map (`metadata:` with
/// indented `name: …` under it) never overrides a top-level key.
pub fn parse_lines(fm: &str) -> HashMap<String, String> {
    let indented = |l: &str| l.starts_with([' ', '\t']);
    let lines: Vec<&str> = fm.lines().collect();
    let mut map = HashMap::new();
    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        i += 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || indented(raw) {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();

        let mut block: Vec<&str> = Vec::new();
        while i < lines.len() && (lines[i].trim().is_empty() || indented(lines[i])) {
            block.push(lines[i].trim());
            i += 1;
        }
        while block.last().is_some_and(|l| l.is_empty()) {
            block.pop();
        }

        let value = match value {
            "|" | "|-" | "|+" => block.join("\n"),
            ">" | ">-" | ">+" => fold_lines(&block),
            "" if !block.is_empty() && block.iter().all(|l| l.is_empty() || l.starts_with("- ")) => {
                let items: Vec<String> = block
                    .iter()
                    .filter_map(|l| l.strip_prefix("- "))
                    .map(|item| strip_scalar_quotes(item.trim()))
                    .collect();
                format!("[{}]", items.join(", "))
            }
            "" => String::new(),
            plain if block.is_empty() => strip_scalar_quotes(plain),
            plain => strip_scalar_quotes(&format!("{plain} {}", fold_lines(&block))),
        };
        map.insert(key, value);
    }
    map
}

/// Fold block lines YAML-style: lines join with a space, a blank line is a
/// line break.
fn fold_lines(block: &[&str]) -> String {
    let mut out = String::new();
    for line in block {
        if line.is_empty() {
            out.push('\n');
        } else {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push(' ');
            }
            out.push_str(line);
        }
    }
    out
}

fn strip_scalar_quotes(s: &str) -> String {
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        return s[1..s.len() - 1].to_string();
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
    trimmed.split_whitespace().map(|s| s.to_string()).collect()
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
        assert_eq!(
            parse_arguments("one two three"),
            vec!["one", "two", "three"]
        );
    }

    #[test]
    fn parse_lines_normalizes_keys_to_lowercase() {
        let map = parse_lines("Name: Foo\nDescription: Bar skill\n");
        assert_eq!(map.get("name"), Some(&"Foo".to_string()));
        assert_eq!(map.get("description"), Some(&"Bar skill".to_string()));
    }

    #[test]
    fn a_bom_and_crlf_still_split() {
        let src = "\u{feff}---\r\nname: foo\r\ndescription: Bar\r\n---\r\nBody\r\n";
        let (fm, _body) = split_frontmatter(src);
        let map = parse_lines(fm.expect("frontmatter"));
        assert_eq!(map.get("description"), Some(&"Bar".to_string()));
        assert_eq!(map.get("name"), Some(&"foo".to_string()));
    }

    #[test]
    fn block_scalars_lists_and_wrapped_values() {
        let map = parse_lines(
            "description: >\n  Folded across\n  two lines.\nnotes: |\n  kept\n  apart\n\
             paths:\n  - src/**\n  - 'docs/*.md'\nsummary: starts here\n  and wraps\n",
        );
        assert_eq!(map["description"], "Folded across two lines.");
        assert_eq!(map["notes"], "kept\napart");
        assert_eq!(map["paths"], "[src/**, docs/*.md]");
        assert_eq!(parse_arguments(&map["paths"]), vec!["src/**", "docs/*.md"]);
        assert_eq!(map["summary"], "starts here and wraps");
    }

    #[test]
    fn nested_keys_do_not_override_top_level_ones() {
        let map = parse_lines("name: deploy\nmetadata:\n  name: other\n  version: 2\ndescription: Ship it\n");
        assert_eq!(map["name"], "deploy");
        assert_eq!(map["description"], "Ship it");
        assert!(!map.contains_key("version"));
    }

    #[test]
    fn strip_quotes_from_scalars() {
        assert_eq!(strip_scalar_quotes("\"hello\""), "hello");
        assert_eq!(strip_scalar_quotes("'world'"), "world");
    }
}
