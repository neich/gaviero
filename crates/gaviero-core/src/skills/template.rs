//! Claude Code–style positional argument substitution for skill bodies.

/// Substitute placeholders in `body` with the provided arguments.
///
/// Supported tokens (after `$`):
/// - `ARGUMENTS[N]` — 0-based positional arg
/// - bare `ARGUMENTS` — raw argument string verbatim
/// - digit run — shorthand for `$ARGUMENTS[N]`
/// - identifier — named arg from `arg_names` positional index
///
/// Backslash escaping: count the backslash run immediately before a token;
/// odd count → literal token; even count → substitute (after emitting
/// `floor(count/2)` backslashes).
pub fn substitute(
    body: &str,
    args: &[String],
    raw_arguments: &str,
    arg_names: &[String],
) -> String {
    let bytes = body.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    let mut saw_bare_arguments = false;

    while i < len {
        if bytes[i] == b'$' {
            let (bs_count, _bs_start) = count_backslashes(bytes, i);
            if bs_count > 0 {
                out.truncate(out.len().saturating_sub(bs_count));
            }
            let emit_bs = bs_count - (bs_count % 2);
            for _ in 0..emit_bs {
                out.push('\\');
            }
            if bs_count % 2 == 1 {
                out.push('$');
                i += 1;
                continue;
            }

            let token_start = i + 1;
            if token_start >= len {
                out.push('$');
                i += 1;
                continue;
            }

            if body[token_start..].starts_with("ARGUMENTS") {
                let after = token_start + "ARGUMENTS".len();
                if after < len && bytes[after] == b'[' {
                    if let Some((idx, end)) = parse_bracket_index(&body[after..]) {
                        push_arg(&mut out, args, idx);
                        i = after + end;
                        continue;
                    }
                }
                saw_bare_arguments = true;
                out.push_str(raw_arguments);
                i = after;
                continue;
            }

            if let Some((idx, consumed)) = parse_digit_index(&body[token_start..]) {
                push_arg(&mut out, args, idx);
                i = token_start + consumed;
                continue;
            }

            if let Some((name, consumed)) = parse_identifier(&body[token_start..]) {
                if let Some(idx) = arg_names.iter().position(|n| n == &name) {
                    push_arg(&mut out, args, idx);
                }
                i = token_start + consumed;
                continue;
            }

            out.push('$');
            i += 1;
            continue;
        }

        let ch = body[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    if !args.is_empty() && !saw_bare_arguments {
        out.push_str("\nARGUMENTS: ");
        out.push_str(raw_arguments);
    }

    out
}

fn count_backslashes(bytes: &[u8], dollar_pos: usize) -> (usize, usize) {
    let mut count = 0usize;
    let mut pos = dollar_pos;
    while pos > 0 && bytes[pos - 1] == b'\\' {
        count += 1;
        pos -= 1;
    }
    (count, pos)
}

fn parse_bracket_index(s: &str) -> Option<(usize, usize)> {
    if !s.starts_with('[') {
        return None;
    }
    let close = s.find(']')?;
    let idx: usize = s[1..close].trim().parse().ok()?;
    Some((idx, close + 1))
}

fn parse_digit_index(s: &str) -> Option<(usize, usize)> {
    let mut end = 0;
    for (i, ch) in s.char_indices() {
        if ch.is_ascii_digit() {
            end = i + ch.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        return None;
    }
    let idx: usize = s[..end].parse().ok()?;
    Some((idx, end))
}

fn parse_identifier(s: &str) -> Option<(String, usize)> {
    let mut chars = s.chars();
    let first = chars.next()?;
    if !matches!(first, 'A'..='Z' | 'a'..='z' | '_') {
        return None;
    }
    let mut end = first.len_utf8();
    for ch in chars {
        if matches!(ch, 'A'..='Z' | 'a'..='z' | '0'..='9' | '_' | '-') {
            end += ch.len_utf8();
        } else {
            break;
        }
    }
    Some((s[..end].to_string(), end))
}

fn push_arg(out: &mut String, args: &[String], idx: usize) {
    if let Some(arg) = args.get(idx) {
        out.push_str(arg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_arguments_substitutes_raw() {
        let out = substitute("Use $ARGUMENTS here", &[], "foo bar", &[]);
        assert_eq!(out, "Use foo bar here");
    }

    #[test]
    fn positional_digit_shorthand() {
        let args = vec!["first".into(), "second".into()];
        let out = substitute("a=$0 b=$1", &args, "first second", &[]);
        assert_eq!(out, "a=first b=second\nARGUMENTS: first second");
    }

    #[test]
    fn named_argument_lookup() {
        let args = vec!["React".into(), "Vue".into()];
        let names = vec!["from".into(), "to".into()];
        let out = substitute("migrate $from to $to", &args, "React Vue", &names);
        assert_eq!(out, "migrate React to Vue\nARGUMENTS: React Vue");
    }

    #[test]
    fn bracket_index_arguments() {
        let args = vec!["alpha".into(), "beta".into()];
        let out = substitute("$ARGUMENTS[1]", &args, "alpha beta", &[]);
        assert_eq!(out, "beta\nARGUMENTS: alpha beta");
    }

    #[test]
    fn out_of_range_index_is_empty() {
        let out = substitute("$ARGUMENTS[5]", &["a".into()], "a", &[]);
        assert_eq!(out, "\nARGUMENTS: a");
    }

    #[test]
    fn escape_literal_dollar() {
        let out = substitute(r"\$1.00 and \$0", &[], "", &[]);
        assert_eq!(out, r"$1.00 and $0");
    }

    #[test]
    fn double_backslash_then_substitute() {
        let args = vec!["val".into()];
        let out = substitute(r"\\$0", &args, "val", &[]);
        assert_eq!(out, "\\\\val\nARGUMENTS: val");
    }

    #[test]
    fn append_when_args_nonempty_and_no_bare_arguments() {
        let args = vec!["x".into()];
        let out = substitute("hello $0", &args, "x", &[]);
        assert_eq!(out, "hello x\nARGUMENTS: x");
    }

    #[test]
    fn no_append_when_bare_arguments_present() {
        let args = vec!["x".into()];
        let out = substitute("$ARGUMENTS", &args, "x", &[]);
        assert_eq!(out, "x");
    }

    #[test]
    fn undeclared_name_stays_empty() {
        let out = substitute("$unknown", &[], "", &[]);
        assert_eq!(out, "");
    }
}
