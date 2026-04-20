//! Glob-style path patterns for file-scope declarations.
//!
//! Supports:
//! - `*`   — any run of non-`/` characters (zero or more)
//! - `**`  — any run of characters including `/`
//! - `?`   — a single non-`/` character
//! - trailing `/` — directory prefix; equivalent to `<dir>/**` while also
//!   matching the bare directory name (preserves historical semantics)
//!
//! Two core operations:
//! - [`matches`]          — does a concrete path satisfy a pattern?
//! - [`patterns_overlap`] — is there any concrete path matching both patterns?
//!
//! Pattern overlap is conservative: any uncertainty resolves to "overlap".
//! The algorithm is a memoised DP over normalised token streams; worst-case
//! `O(n*m)` in the token lengths, which stays tiny for real scope paths.

use std::collections::HashMap;

use crate::types::normalize_path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    Lit(char),
    Question,
    Star,
    StarStar,
}

fn compile(pattern: &str) -> (Vec<Tok>, bool) {
    let raw = normalize_path(pattern);
    let dir_trail = raw.ends_with('/') && raw != "/";
    let body: &str = if dir_trail {
        raw.trim_end_matches('/')
    } else {
        raw.as_str()
    };

    let mut tokens = Vec::with_capacity(body.len());
    let chars: Vec<char> = body.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '*' {
            if i + 1 < chars.len() && chars[i + 1] == '*' {
                tokens.push(Tok::StarStar);
                i += 2;
            } else {
                tokens.push(Tok::Star);
                i += 1;
            }
        } else if c == '?' {
            tokens.push(Tok::Question);
            i += 1;
        } else {
            tokens.push(Tok::Lit(c));
            i += 1;
        }
    }
    (tokens, dir_trail)
}

fn path_tokens(path: &str) -> Vec<Tok> {
    normalize_path(path).chars().map(Tok::Lit).collect()
}

fn tail_accepts_empty(ts: &[Tok]) -> bool {
    ts.iter().all(|t| matches!(t, Tok::Star | Tok::StarStar))
}

fn intersect(a: &[Tok], b: &[Tok]) -> bool {
    let mut memo: HashMap<(usize, usize), bool> = HashMap::new();
    step(a, b, 0, 0, &mut memo)
}

fn step(
    a: &[Tok],
    b: &[Tok],
    i: usize,
    j: usize,
    memo: &mut HashMap<(usize, usize), bool>,
) -> bool {
    if let Some(&v) = memo.get(&(i, j)) {
        return v;
    }
    let result = step_inner(a, b, i, j, memo);
    memo.insert((i, j), result);
    result
}

fn step_inner(
    a: &[Tok],
    b: &[Tok],
    i: usize,
    j: usize,
    memo: &mut HashMap<(usize, usize), bool>,
) -> bool {
    if i == a.len() && j == b.len() {
        return true;
    }
    if i == a.len() {
        return tail_accepts_empty(&b[j..]);
    }
    if j == b.len() {
        return tail_accepts_empty(&a[i..]);
    }

    let ta = a[i];
    let tb = b[j];

    // Both are star-like: let either advance independently.
    if is_star(ta) && is_star(tb) {
        return step(a, b, i + 1, j, memo)
            || step(a, b, i, j + 1, memo)
            || step(a, b, i + 1, j + 1, memo);
    }

    if is_star(ta) {
        if step(a, b, i + 1, j, memo) {
            return true;
        }
        return star_can_consume(ta, tb) && step(a, b, i, j + 1, memo);
    }
    if is_star(tb) {
        if step(a, b, i, j + 1, memo) {
            return true;
        }
        return star_can_consume(tb, ta) && step(a, b, i + 1, j, memo);
    }

    // Neither side is a star: lit/lit, lit/?, ?/lit, ?/?
    match (ta, tb) {
        (Tok::Lit(c1), Tok::Lit(c2)) => c1 == c2 && step(a, b, i + 1, j + 1, memo),
        (Tok::Lit(c), Tok::Question) | (Tok::Question, Tok::Lit(c)) => {
            c != '/' && step(a, b, i + 1, j + 1, memo)
        }
        (Tok::Question, Tok::Question) => step(a, b, i + 1, j + 1, memo),
        _ => unreachable!(),
    }
}

fn is_star(t: Tok) -> bool {
    matches!(t, Tok::Star | Tok::StarStar)
}

fn star_can_consume(star: Tok, other: Tok) -> bool {
    match (star, other) {
        (Tok::StarStar, _) => true,
        (Tok::Star, Tok::Lit(c)) => c != '/',
        (Tok::Star, Tok::Question) => true, // `?` never matches `/`
        (Tok::Star, Tok::Star) | (Tok::Star, Tok::StarStar) => true,
        _ => false,
    }
}

/// Does `path` satisfy `pattern`?
pub fn matches(pattern: &str, path: &str) -> bool {
    let (pat, dir_trail) = compile(pattern);
    let target = path_tokens(path);

    if dir_trail {
        let bare = pat.clone();
        if intersect(&bare, &target) && equals_literal(&bare, &target) {
            return true;
        }
        let mut extended = pat;
        extended.push(Tok::Lit('/'));
        extended.push(Tok::StarStar);
        intersect(&extended, &target)
    } else {
        intersect(&pat, &target)
    }
}

fn equals_literal(pat: &[Tok], target: &[Tok]) -> bool {
    pat.len() == target.len()
        && pat.iter().zip(target.iter()).all(|(p, t)| match (p, t) {
            (Tok::Lit(a), Tok::Lit(b)) => a == b,
            _ => false,
        })
}

/// Could any concrete path match both patterns?
pub fn patterns_overlap(a: &str, b: &str) -> bool {
    let (pa, da) = compile(a);
    let (pb, db) = compile(b);

    // A trailing `/` on pattern P expands to two alternatives:
    //   (1) exactly the bare prefix path
    //   (2) the prefix followed by `/**`
    // Overlap holds if any alternative of A overlaps any alternative of B.
    let a_alts = expand_dir(&pa, da);
    let b_alts = expand_dir(&pb, db);

    for aa in &a_alts {
        for bb in &b_alts {
            if intersect(aa, bb) {
                return true;
            }
        }
    }
    false
}

fn expand_dir(tokens: &[Tok], dir_trail: bool) -> Vec<Vec<Tok>> {
    if !dir_trail {
        return vec![tokens.to_vec()];
    }
    let mut with_tail = tokens.to_vec();
    with_tail.push(Tok::Lit('/'));
    with_tail.push(Tok::StarStar);
    vec![tokens.to_vec(), with_tail]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_exact() {
        assert!(matches("src/main.rs", "src/main.rs"));
        assert!(!matches("src/main.rs", "src/lib.rs"));
    }

    #[test]
    fn matches_directory_prefix() {
        assert!(matches("src/", "src/main.rs"));
        assert!(matches("src/", "src/nested/mod.rs"));
        assert!(matches("src/", "src"));
        assert!(!matches("src/", "srcx/foo"));
    }

    #[test]
    fn matches_star() {
        assert!(matches("plans/*.md", "plans/foo.md"));
        assert!(matches("plans/*.md", "plans/.md"));
        assert!(!matches("plans/*.md", "plans/sub/foo.md"));
        assert!(!matches("plans/*.md", "plans/foo.txt"));
    }

    #[test]
    fn matches_double_star() {
        assert!(matches("plans/**", "plans/foo.md"));
        assert!(matches("plans/**", "plans/sub/foo.md"));
        assert!(matches("**/*.rs", "src/deep/nest/lib.rs"));
    }

    #[test]
    fn matches_question() {
        assert!(matches("v?.md", "v1.md"));
        assert!(!matches("v?.md", "v10.md"));
        assert!(!matches("v?.md", "v/.md"));
    }

    #[test]
    fn overlap_literal_and_dir() {
        assert!(patterns_overlap("src/", "src/main.rs"));
        assert!(patterns_overlap("src/main.rs", "src/"));
    }

    #[test]
    fn overlap_identical() {
        assert!(patterns_overlap("plans/", "plans/"));
    }

    #[test]
    fn overlap_disjoint_globs() {
        assert!(!patterns_overlap(
            "plans/claude-*.md",
            "plans/codex-*.md"
        ));
    }

    #[test]
    fn overlap_overlapping_globs() {
        assert!(patterns_overlap("plans/*.md", "plans/*-v1.md"));
        assert!(patterns_overlap("plans/claude-*.md", "plans/*-v1.md"));
    }

    #[test]
    fn overlap_glob_vs_dir() {
        assert!(patterns_overlap("plans/", "plans/claude-*.md"));
        assert!(!patterns_overlap("plans/", "docs/claude-*.md"));
    }

    #[test]
    fn overlap_same_prefix_different_tail() {
        assert!(!patterns_overlap(
            "plans/claude-plan-v1.md",
            "plans/claude-plan-v2.md"
        ));
        assert!(patterns_overlap(
            "plans/claude-plan-v1.md",
            "plans/claude-plan-v?.md"
        ));
    }

    #[test]
    fn overlap_star_spans_single_segment() {
        assert!(!patterns_overlap("plans/*", "plans/a/b"));
        assert!(patterns_overlap("plans/**", "plans/a/b"));
    }
}
