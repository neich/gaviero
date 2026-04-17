//! Bracket-counting indent fallback.
//!
//! Works for any language without tree-sitter queries. Counts unmatched
//! opening delimiters on the line above the cursor, skipping string
//! literals and comments.

use super::IndentResult;

/// Compute indentation using bracket counting.
///
/// Scans the line at `cursor_byte` for unmatched opening delimiters,
/// skipping content inside string literals and comments.
pub fn compute_bracket_indent(
    doc: &ropey::Rope,
    cursor_byte: usize,
    _tab_width: u8,
    indent_unit: &str,
) -> IndentResult {
    let text = doc.to_string();
    let cursor_byte = cursor_byte.min(text.len());

    // Find the line containing the cursor
    let line_start = text[..cursor_byte].rfind('\n').map(|p| p + 1).unwrap_or(0);
    let line_text = &text[line_start..cursor_byte];

    // Get baseline indent (leading whitespace of current line)
    let baseline_indent = leading_whitespace(&text[line_start..]);

    // Count unmatched opening brackets on this line, skipping strings/comments
    let net = net_bracket_depth(line_text);

    let level = (baseline_indent_level(&baseline_indent, indent_unit) as i32 + net).max(0);
    let whitespace = indent_unit.repeat(level as usize);

    IndentResult { whitespace, level }
}

/// Reindent an entire document using bracket counting.
///
/// Reindent a document, only changing lines with wrong indentation.
///
/// Lines whose actual indent level matches the expected bracket depth
/// are left untouched — preserving intentional formatting like horizontal
/// lists, aligned arguments, or manually adjusted indentation.
///
/// Lines whose indent level is wrong are reindented to the correct depth.
pub fn reindent_document(content: &str, indent_unit: &str) -> String {
    let unit_width = indent_visual_width(indent_unit);
    if unit_width == 0 {
        return content.to_string();
    }

    let mut result = String::with_capacity(content.len());
    let mut depth: i32 = 0;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            result.push('\n');
            continue;
        }

        // Leading close bracket reduces depth before indenting
        let first_char = trimmed.chars().next().unwrap_or(' ');
        if is_close_bracket(first_char) {
            depth -= 1;
        }

        let expected_depth = depth.max(0) as usize;
        let actual_ws = leading_whitespace(line);
        let actual_depth = indent_visual_width(&actual_ws) / unit_width;

        if actual_depth == expected_depth {
            // Indent matches — keep the original line exactly as-is
            result.push_str(line);
        } else {
            // Indent is wrong — fix it
            for _ in 0..expected_depth {
                result.push_str(indent_unit);
            }
            result.push_str(trimmed);
        }
        result.push('\n');

        // Compute net bracket change on this line (skipping strings/comments)
        let net = net_bracket_depth(trimmed);
        if is_close_bracket(first_char) {
            depth += net + 1;
        } else {
            depth += net;
        }
    }

    // Match trailing newline of original
    if !content.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    result
}

/// Compute the cumulative bracket depth at a given line in a document.
///
/// Scans from the start of the document up to (but not including) `target_line`,
/// tracking net bracket nesting.
pub fn depth_at_line(content: &str, target_line: usize) -> i32 {
    let mut depth: i32 = 0;
    for (i, line) in content.lines().enumerate() {
        if i >= target_line {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let first_char = trimmed.chars().next().unwrap_or(' ');
        if is_close_bracket(first_char) {
            depth -= 1;
        }
        let net = net_bracket_depth(trimmed);
        if is_close_bracket(first_char) {
            depth += net + 1;
        } else {
            depth += net;
        }
    }
    depth.max(0)
}

/// Reindent a range of lines within a document.
///
/// `start_line` and `end_line` are 0-indexed, inclusive. Lines outside the
/// range are returned unchanged. Lines inside the range are reindented
/// relative to the bracket depth computed at `start_line`.
///
/// Only lines with wrong indentation are changed; correctly-indented lines
/// are preserved as-is.
pub fn reindent_line_range(
    content: &str,
    start_line: usize,
    end_line: usize,
    indent_unit: &str,
) -> String {
    let unit_width = indent_visual_width(indent_unit);
    if unit_width == 0 {
        return content.to_string();
    }

    let mut depth = depth_at_line(content, start_line);
    let mut result = String::with_capacity(content.len());

    for (i, line) in content.lines().enumerate() {
        if i < start_line || i > end_line {
            // Outside range — keep as-is
            result.push_str(line);
            result.push('\n');

            // Still track depth for lines before the range
            if i < start_line {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    let first_char = trimmed.chars().next().unwrap_or(' ');
                    if is_close_bracket(first_char) {
                        depth -= 1;
                    }
                    let net = net_bracket_depth(trimmed);
                    if is_close_bracket(first_char) {
                        depth += net + 1;
                    } else {
                        depth += net;
                    }
                }
            }
            continue;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            result.push('\n');
            continue;
        }

        let first_char = trimmed.chars().next().unwrap_or(' ');
        if is_close_bracket(first_char) {
            depth -= 1;
        }

        let expected_depth = depth.max(0) as usize;
        let actual_ws = leading_whitespace(line);
        let actual_depth = indent_visual_width(&actual_ws) / unit_width;

        if actual_depth == expected_depth {
            result.push_str(line);
        } else {
            for _ in 0..expected_depth {
                result.push_str(indent_unit);
            }
            result.push_str(trimmed);
        }
        result.push('\n');

        let net = net_bracket_depth(trimmed);
        if is_close_bracket(first_char) {
            depth += net + 1;
        } else {
            depth += net;
        }
    }

    if !content.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    result
}

/// Compute the visual width of a whitespace string (tabs = 4 columns).
fn indent_visual_width(ws: &str) -> usize {
    ws.chars().map(|c| if c == '\t' { 4 } else { 1 }).sum()
}

/// Count net unmatched opening brackets on a line, skipping strings and comments.
fn net_bracket_depth(line: &str) -> i32 {
    let mut depth: i32 = 0;
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let ch = bytes[i];
        match ch {
            // Skip double-quoted strings
            b'"' => {
                i += 1;
                while i < len {
                    if bytes[i] == b'\\' {
                        i += 2; // skip escaped char
                    } else if bytes[i] == b'"' {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
            }
            // Skip single-quoted strings/chars
            b'\'' => {
                i += 1;
                while i < len {
                    if bytes[i] == b'\\' {
                        i += 2;
                    } else if bytes[i] == b'\'' {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
            }
            // Skip line comments (//)
            b'/' if i + 1 < len && bytes[i + 1] == b'/' => {
                break; // rest of line is comment
            }
            // Skip block comments (/* ... */ — within single line)
            b'/' if i + 1 < len && bytes[i + 1] == b'*' => {
                i += 2;
                while i + 1 < len {
                    if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            // Skip # comments (Python, Ruby, Bash)
            b'#' => break,
            // Count brackets
            b'{' | b'(' | b'[' => {
                depth += 1;
                i += 1;
            }
            b'}' | b')' | b']' => {
                depth -= 1;
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    depth
}

fn is_close_bracket(ch: char) -> bool {
    matches!(ch, '}' | ')' | ']')
}

fn leading_whitespace(line: &str) -> String {
    line.chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

fn baseline_indent_level(whitespace: &str, indent_unit: &str) -> usize {
    if indent_unit.is_empty() {
        return 0;
    }
    // Count how many indent_units fit in the whitespace
    let ws_width: usize = whitespace
        .chars()
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum();
    let unit_width: usize = indent_unit
        .chars()
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum();
    if unit_width == 0 {
        return 0;
    }
    ws_width / unit_width
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_net_bracket_depth_simple() {
        assert_eq!(net_bracket_depth("fn main() {"), 1);
        assert_eq!(net_bracket_depth("}"), -1);
        assert_eq!(net_bracket_depth("let x = (a + b);"), 0);
        assert_eq!(net_bracket_depth("if (a) {"), 1); // ( and ) cancel, { opens
    }

    #[test]
    fn test_net_bracket_depth_skips_strings() {
        assert_eq!(net_bracket_depth(r#"let s = "hello {world}";"#), 0);
        assert_eq!(net_bracket_depth(r#"let s = "{";"#), 0);
        assert_eq!(net_bracket_depth(r#"print("}")"#), 0);
    }

    #[test]
    fn test_net_bracket_depth_skips_comments() {
        assert_eq!(net_bracket_depth("let x = 1; // { open brace"), 0);
        assert_eq!(net_bracket_depth("/* { */ let x = 1;"), 0);
    }

    #[test]
    fn test_net_bracket_depth_skips_hash_comments() {
        assert_eq!(net_bracket_depth("x = 1  # {open"), 0);
    }

    #[test]
    fn test_compute_bracket_indent() {
        let rope = ropey::Rope::from_str("fn main() {\n");
        let cursor = rope.line_to_byte(0) + "fn main() {".len();
        let result = compute_bracket_indent(&rope, cursor, 4, "    ");
        assert_eq!(result.level, 1);
        assert_eq!(result.whitespace, "    ");
    }

    #[test]
    fn test_compute_bracket_indent_nested() {
        let rope = ropey::Rope::from_str("    if true {\n");
        let cursor = rope.line_to_byte(0) + "    if true {".len();
        let result = compute_bracket_indent(&rope, cursor, 4, "    ");
        assert_eq!(result.level, 2);
        assert_eq!(result.whitespace, "        ");
    }

    #[test]
    fn test_reindent_document() {
        let input = "fn main() {\nlet x = 1;\nif true {\nlet y = 2;\n}\n}\n";
        let expected = "fn main() {\n    let x = 1;\n    if true {\n        let y = 2;\n    }\n}\n";
        assert_eq!(reindent_document(input, "    "), expected);
    }

    #[test]
    fn test_reindent_preserves_correct_lines() {
        // Lines with correct indent are kept exactly as-is (including trailing spaces, etc.)
        let input = "fn main() {\n    let x = [1, 2, 3];\n    let y = 2;\n}\n";
        let result = reindent_document(input, "    ");
        assert_eq!(result, input, "already correct file should be unchanged");
    }

    #[test]
    fn test_reindent_preserves_horizontal_list() {
        // A horizontal list at depth 1 has correct indent — should be preserved as-is
        let input = "fn main() {\n    let items = [1, 2, 3, 4, 5];\n}\n";
        let result = reindent_document(input, "    ");
        assert_eq!(result, input);
    }

    #[test]
    fn test_reindent_fixes_wrong_but_preserves_correct() {
        // Line 2 is wrong (0 indent instead of 1), line 3 is correct
        let input = "fn main() {\nlet x = 1;\n    let y = 2;\n}\n";
        let result = reindent_document(input, "    ");
        assert!(
            result.contains("    let x = 1;"),
            "wrong indent should be fixed"
        );
        assert!(
            result.contains("    let y = 2;"),
            "correct indent should be preserved"
        );
    }

    #[test]
    fn test_reindent_preserves_blank_lines() {
        let input = "fn main() {\n\nlet x = 1;\n}\n";
        let result = reindent_document(input, "    ");
        assert!(result.contains("\n\n"));
    }
}
