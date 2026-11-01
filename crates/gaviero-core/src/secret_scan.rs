//! High-precision secret patterns for redacting text before it reaches a
//! model or a log.
//!
//! The rule set is deliberately small: each pattern targets a credential
//! format with a recognisable prefix or framing (private-key blocks, provider
//! API keys) so a hit is almost always a real secret. It is a tripwire, not a
//! guarantee — generic high-entropy strings are not flagged, because entropy
//! scoring separates secrets from ordinary identifiers poorly.
//!
//! Rules are tried in list order; when two hits overlap, the earlier rule
//! wins (`sk-ant-…` reports `anthropic-api-key`, never `openai-style-key`).

use std::sync::LazyLock;

use regex::Regex;

/// One secret found in a text: the rule id and the byte range it covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretHit {
    pub rule: &'static str,
    pub start: usize,
    pub end: usize,
}

static RULES: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    [
        (
            "private-key-block",
            r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?-----END [A-Z0-9 ]*PRIVATE KEY-----", // pragma: allowlist secret
        ),
        ("anthropic-api-key", r"sk-ant-[A-Za-z0-9_\-]{20,}"), // pragma: allowlist secret
        ("openai-style-key", r"\bsk-(?:proj-)?[A-Za-z0-9_\-]{20,}"), // pragma: allowlist secret
        ("aws-access-key-id", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"), // pragma: allowlist secret
        ("github-token", r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36,}\b"), // pragma: allowlist secret
        ("github-fine-grained-pat", r"\bgithub_pat_[A-Za-z0-9_]{60,}\b"), // pragma: allowlist secret
        ("slack-token", r"\bxox[abprs]-[A-Za-z0-9\-]{10,}"), // pragma: allowlist secret
        ("google-api-key", r"\bAIza[0-9A-Za-z_\-]{35}\b"), // pragma: allowlist secret
        ("stripe-live-key", r"\b[rs]k_live_[0-9A-Za-z]{20,}\b"), // pragma: allowlist secret
        (
            "dotenv-secret-assignment",
            r#"(?m)^[ \t]*(?:export[ \t]+)?[A-Z0-9_]*(?:SECRET|TOKEN|PASSWORD|PASSWD|API_KEY|PRIVATE_KEY)[A-Z0-9_]*[ \t]*=[ \t]*["']?[^\s"']{8,}"#, // pragma: allowlist secret
        ),
    ]
    .into_iter()
    .map(|(id, pattern)| (id, Regex::new(pattern).expect("secret_scan rule compiles")))
    .collect()
});

/// Every secret in `text`, non-overlapping and in byte order.
pub fn find_secrets(text: &str) -> Vec<SecretHit> {
    let mut hits: Vec<SecretHit> = Vec::new();
    for (rule, regex) in RULES.iter() {
        for m in regex.find_iter(text) {
            let overlaps = hits.iter().any(|h| m.start() < h.end && h.start < m.end());
            if !overlaps {
                hits.push(SecretHit {
                    rule,
                    start: m.start(),
                    end: m.end(),
                });
            }
        }
    }
    hits.sort_by_key(|h| h.start);
    hits
}

/// `text` with every hit replaced by `[REDACTED:<rule>]`, plus the distinct
/// rule ids in hit order.
pub fn redact(text: &str) -> (String, Vec<&'static str>) {
    let hits = find_secrets(text);
    if hits.is_empty() {
        return (text.to_string(), Vec::new());
    }
    let mut out = String::with_capacity(text.len());
    let mut rules: Vec<&'static str> = Vec::new();
    let mut cursor = 0;
    for hit in &hits {
        out.push_str(&text[cursor..hit.start]);
        out.push_str(&format!("[REDACTED:{}]", hit.rule));
        cursor = hit.end;
        if !rules.contains(&hit.rule) {
            rules.push(hit.rule);
        }
    }
    out.push_str(&text[cursor..]);
    (out, rules)
}

/// Redact every string value inside `value` in place (object keys are left
/// alone). Returns the distinct rule ids that fired.
pub fn redact_json(value: &mut serde_json::Value) -> Vec<&'static str> {
    let mut rules: Vec<&'static str> = Vec::new();
    redact_json_into(value, &mut rules);
    rules
}

fn redact_json_into(value: &mut serde_json::Value, rules: &mut Vec<&'static str>) {
    match value {
        serde_json::Value::String(s) => {
            let (redacted, fired) = redact(s);
            if !fired.is_empty() {
                *s = redacted;
                for rule in fired {
                    if !rules.contains(&rule) {
                        rules.push(rule);
                    }
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                redact_json_into(item, rules);
            }
        }
        serde_json::Value::Object(map) => {
            for (_, item) in map.iter_mut() {
                redact_json_into(item, rules);
            }
        }
        _ => {}
    }
}

/// Up to 12 characters of context on each side of `hit`, with the secret
/// itself replaced — safe to show in a warning or write to an audit log.
pub fn excerpt(text: &str, hit: &SecretHit) -> String {
    let before: String = {
        let head = &text[..hit.start];
        let chars: Vec<char> = head.chars().rev().take(12).collect();
        chars.into_iter().rev().collect()
    };
    let after: String = text[hit.end..].chars().take(12).collect();
    format!("{before}[REDACTED:{}]{after}", hit.rule)
}

// ── Fixture classification ──────────────────────────────────────────────
//
// Heuristics that mark a hit as "probably a test fixture". They downgrade a
// finding (logged, not shown); they never drop it, because anything that
// writes a file can fake them.

/// Markers secret scanners honour to allowlist one line.
pub const ALLOWLIST_MARKERS: &[&str] =
    &["pragma: allowlist secret", "gitleaks:allow", "trufflehog:ignore"];

/// The full line of `text` holding `hit`.
pub fn line_around<'a>(text: &'a str, hit: &SecretHit) -> &'a str {
    let start = text[..hit.start].rfind('\n').map_or(0, |i| i + 1);
    let end = text[hit.end..].find('\n').map_or(text.len(), |i| hit.end + i);
    &text[start..end]
}

/// Does `line` carry an allowlist marker (`// pragma: allowlist secret`, …)?
pub fn has_allowlist_marker(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    ALLOWLIST_MARKERS.iter().any(|m| lower.contains(m))
}

/// Provider prefixes stripped before judging a value, longest first.
const VALUE_PREFIXES: &[&str] = &[
    "sk-ant-admin01-",
    "sk-ant-api03-",
    "github_pat_",
    "sk-proj-",
    "sk-ant-",
    "sk_live_",
    "rk_live_",
    "xoxa-",
    "xoxb-",
    "xoxp-",
    "xoxr-",
    "xoxs-",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "AKIA",
    "ASIA",
    "AIza",
    "sk-",
];

/// Words real credentials do not contain.
const PLACEHOLDER_WORDS: &[&str] = &[
    "example", "dummy", "fake", "test", "sample", "canary", "placeholder", "changeme",
    "xxxx", "redacted", "foobar", "notreal",
];

/// The random part of a matched value: the payload between PEM headers,
/// the right-hand side of a `KEY=value`, or the value minus its provider
/// prefix — alphanumerics only.
fn value_body(rule: &str, value: &str) -> String {
    let raw: &str = match rule {
        "private-key-block" => {
            let after = value.split_once("-----\n").map_or(value, |(_, rest)| rest);
            let after = after.split_once("-----\\n").map_or(after, |(_, rest)| rest);
            after.rsplit_once("-----END").map_or(after, |(body, _)| body)
        }
        "dotenv-secret-assignment" => value.split_once('=').map_or(value, |(_, v)| v),
        _ => VALUE_PREFIXES
            .iter()
            .find_map(|p| value.strip_prefix(p))
            .unwrap_or(value),
    };
    let raw = raw.replace("\\n", "");
    raw.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// Longest run of consecutive ascending characters (`abcd`, `1234`),
/// ignoring case.
fn longest_ascending_run(body: &str) -> usize {
    let lower: Vec<u8> = body.to_ascii_lowercase().into_bytes();
    let mut best = usize::from(!lower.is_empty());
    let mut run = best;
    for pair in lower.windows(2) {
        if pair[1] == pair[0].wrapping_add(1) {
            run += 1;
            best = best.max(run);
        } else {
            run = 1;
        }
    }
    best
}

/// Does the value matched by `rule` look like a placeholder rather than a
/// real credential? Real keys are random; fixtures are sequential
/// (`ABCDEFGH…`), repetitive (`aaaa…`), short PEM bodies, or spell a
/// placeholder word (`EXAMPLE`, `dummy`, `canary`, …).
pub fn is_dummy_value(rule: &str, value: &str) -> bool {
    let body = value_body(rule, value);
    let lower = value.to_ascii_lowercase();
    if PLACEHOLDER_WORDS.iter().any(|w| lower.contains(w)) {
        return true;
    }
    if rule == "private-key-block" && body.len() < 64 {
        return true;
    }
    if body.len() < 8 {
        return false;
    }
    let mut counts = [0usize; 128];
    for b in body.bytes() {
        counts[usize::from(b & 0x7f)] += 1;
    }
    let distinct = counts.iter().filter(|c| **c > 0).count();
    let top = counts.iter().copied().max().unwrap_or(0);
    distinct <= 4 || top * 2 >= body.len() || longest_ascending_run(&body) >= 6
}

/// Does `path` look like test code or test data? Directory components
/// (`tests/`, `__tests__/`, `fixtures/`, `testdata/`, `spec/`, …) or
/// test-file names (`*_test.*`, `test_*.py`, `*.test.*`, `*.spec.*`,
/// `conftest.py`). Rust inline `#[cfg(test)]` modules need the file
/// contents and are checked by the caller.
pub fn is_test_path(path: &std::path::Path) -> bool {
    const DIRS: &[&str] = &[
        "tests", "test", "__tests__", "__mocks__", "fixtures", "fixture", "testdata",
        "test_data", "spec", "specs",
    ];
    let text = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    let mut parts: Vec<&str> = text.split('/').filter(|p| !p.is_empty()).collect();
    let Some(name) = parts.pop() else {
        return false;
    };
    if parts.iter().any(|p| DIRS.contains(p)) {
        return true;
    }
    let stem = name.split('.').next().unwrap_or(name);
    name == "conftest.py"
        || stem.starts_with("test_")
        || stem.ends_with("_test")
        || stem.ends_with("_tests")
        || stem.ends_with("_spec")
        || name.contains(".test.")
        || name.contains(".spec.")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules_in(text: &str) -> Vec<&'static str> {
        find_secrets(text).into_iter().map(|h| h.rule).collect()
    }

    const PEM: &str = "-----BEGIN RSA PRIVATE KEY-----\nMIIEow\n-----END RSA PRIVATE KEY-----"; // pragma: allowlist secret

    #[test]
    fn private_key_block_positive() {
        assert_eq!(rules_in(&format!("key:\n{PEM}\n")), vec!["private-key-block"]);
    }

    #[test]
    fn private_key_block_negative() {
        assert!(rules_in("-----BEGIN PUBLIC KEY-----\nabc\n-----END PUBLIC KEY-----").is_empty());
    }

    #[test]
    fn anthropic_key_positive() {
        assert_eq!(
            rules_in("ANTHROPIC=sk-ant-api03-abcdefghijklmnopqrstuvwxyz"), // pragma: allowlist secret
            vec!["anthropic-api-key"]
        );
    }

    #[test]
    fn anthropic_key_negative() {
        assert!(rules_in("sk-ant-short").is_empty()); // pragma: allowlist secret
    }

    #[test]
    fn openai_key_positive() {
        assert_eq!(
            rules_in("key sk-proj-abcdefghijklmnopqrstuvwx"), // pragma: allowlist secret
            vec!["openai-style-key"]
        );
    }

    #[test]
    fn openai_key_negative() {
        assert!(rules_in("task-list and sk-abc").is_empty());
    }

    #[test]
    fn aws_key_positive() {
        assert_eq!(rules_in("id AKIAABCDEFGHIJKLMNOP end"), vec!["aws-access-key-id"]); // pragma: allowlist secret
    }

    #[test]
    fn aws_key_negative() {
        assert!(rules_in("AKIAshort and XAKIAABCDEFGHIJKLMNOPQ").is_empty()); // pragma: allowlist secret
    }

    #[test]
    fn github_token_positive() {
        let token = format!("ghp_{}", "a".repeat(36)); // pragma: allowlist secret
        assert_eq!(rules_in(&token), vec!["github-token"]);
    }

    #[test]
    fn github_token_negative() {
        assert!(rules_in("ghp_tooShort123").is_empty()); // pragma: allowlist secret
    }

    #[test]
    fn github_fine_grained_positive() {
        let token = format!("github_pat_{}", "A1_".repeat(21)); // pragma: allowlist secret
        assert_eq!(rules_in(&token), vec!["github-fine-grained-pat"]);
    }

    #[test]
    fn github_fine_grained_negative() {
        assert!(rules_in("github_pat_short").is_empty()); // pragma: allowlist secret
    }

    #[test]
    fn slack_token_positive() {
        assert_eq!(rules_in("xoxb-1234567890-abcdef"), vec!["slack-token"]); // pragma: allowlist secret
    }

    #[test]
    fn slack_token_negative() {
        assert!(rules_in("xoxb-12").is_empty()); // pragma: allowlist secret
    }

    #[test]
    fn google_key_positive() {
        let key = format!("AIza{}", "B".repeat(35)); // pragma: allowlist secret
        assert_eq!(rules_in(&key), vec!["google-api-key"]);
    }

    #[test]
    fn google_key_negative() {
        assert!(rules_in("AIzaShort").is_empty()); // pragma: allowlist secret
    }

    #[test]
    fn stripe_key_positive() {
        assert_eq!(rules_in("sk_live_abcdefghijklmnopqrstu"), vec!["stripe-live-key"]); // pragma: allowlist secret
    }

    #[test]
    fn stripe_key_negative() {
        assert!(rules_in("sk_test_abcdefghijklmnopqrstu").is_empty()); // pragma: allowlist secret
    }

    #[test]
    fn dotenv_assignment_positive() {
        assert_eq!(
            rules_in("DATABASE_PASSWORD=hunter2hunter2\n"), // pragma: allowlist secret
            vec!["dotenv-secret-assignment"]
        );
        assert_eq!(
            rules_in("export MY_API_KEY=\"abcdefgh12\""), // pragma: allowlist secret
            vec!["dotenv-secret-assignment"]
        );
    }

    #[test]
    fn dotenv_assignment_negative() {
        assert!(rules_in("PORT=8080\nTOKEN_TTL=60\n").is_empty());
    }

    #[test]
    fn redact_removes_every_secret() {
        let text = format!("a AKIAABCDEFGHIJKLMNOP b {PEM} c"); // pragma: allowlist secret
        let (out, rules) = redact(&text);
        assert!(!out.contains("AKIAABCDEFGHIJKLMNOP")); // pragma: allowlist secret
        assert!(!out.contains("MIIEow"));
        assert!(out.contains("[REDACTED:aws-access-key-id]"));
        assert_eq!(rules, vec!["aws-access-key-id", "private-key-block"]);
    }

    #[test]
    fn redact_without_hits_is_identity() {
        let (out, rules) = redact("nothing to see");
        assert_eq!(out, "nothing to see");
        assert!(rules.is_empty());
    }

    #[test]
    fn redact_json_walks_nested_values() {
        let key = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz"; // pragma: allowlist secret
        let mut value = serde_json::json!({"a": [{"b": format!("key {key}")}], key: 1});
        let rules = redact_json(&mut value);
        assert_eq!(rules, vec!["anthropic-api-key"]);
        assert_eq!(value["a"][0]["b"], "key [REDACTED:anthropic-api-key]");
        // Keys are untouched.
        assert!(value.get(key).is_some());
    }

    #[test]
    fn overlapping_rules_report_the_first_only() {
        let hits = find_secrets("sk-ant-api03-abcdefghijklmnopqrstuvwxyz"); // pragma: allowlist secret
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].rule, "anthropic-api-key");
    }

    #[test]
    fn excerpt_hides_the_secret() {
        let text = "prefix text AKIAABCDEFGHIJKLMNOP suffix text"; // pragma: allowlist secret
        let hits = find_secrets(text);
        let ex = excerpt(text, &hits[0]);
        assert_eq!(ex, "prefix text [REDACTED:aws-access-key-id] suffix text");
    }

    // ── Fixture classification ──────────────────────────────────────

    #[test]
    fn allowlist_marker_is_found_on_the_hit_line_only() {
        let text = "a\nlet k = \"AKIAABCDEFGHIJKLMNOP\"; // pragma: allowlist secret\nb"; // pragma: allowlist secret
        let hit = &find_secrets(text)[0];
        let line = line_around(text, hit);
        assert!(line.starts_with("let k"), "{line}");
        assert!(has_allowlist_marker(line));
        assert!(has_allowlist_marker("x // gitleaks:allow"));
        assert!(!has_allowlist_marker("let k = 1;"));
    }

    #[test]
    fn dummy_values_are_recognised() {
        // Sequential, repeated, placeholder-word and short-PEM fixtures.
        assert!(is_dummy_value("aws-access-key-id", "AKIAABCDEFGHIJKLMNOP")); // pragma: allowlist secret
        assert!(is_dummy_value("aws-access-key-id", "AKIAIOSFODNN7EXAMPLE")); // pragma: allowlist secret
        assert!(is_dummy_value("github-token", &format!("ghp_{}", "a".repeat(36)))); // pragma: allowlist secret
        assert!(is_dummy_value("anthropic-api-key", "sk-ant-api03-abcdefghijklmnopqrstuvwxyz")); // pragma: allowlist secret
        assert!(is_dummy_value("slack-token", "xoxb-1234567890-abcdef")); // pragma: allowlist secret
        assert!(is_dummy_value("private-key-block", PEM));
        assert!(is_dummy_value("dotenv-secret-assignment", "DUMMY_SECRET=canary-env-7f3a91")); // pragma: allowlist secret
    }

    #[test]
    fn random_looking_values_are_not_dummy() {
        assert!(!is_dummy_value("aws-access-key-id", "AKIAQ7XZ4M2KD9PL3VWN")); // pragma: allowlist secret
        assert!(!is_dummy_value("anthropic-api-key", "sk-ant-api03-Zq8Lr2Vx9Kp4Tm7Nw3Hj6Bf1Yd5Cs0Ge")); // pragma: allowlist secret
        assert!(!is_dummy_value("dotenv-secret-assignment", "DB_PASSWORD=hunter2hunter2")); // pragma: allowlist secret
    }

    #[test]
    fn test_paths_are_recognised() {
        use std::path::Path;
        for p in [
            "crates/x/tests/egress_live.rs",
            "web/__tests__/auth.ts",
            "pkg/testdata/key.txt",
            "app/fixtures/creds.json",
            "src/auth_test.go",
            "tests_dir/test_auth.py",
            "src/auth.test.ts",
            "src/auth.spec.js",
            "conftest.py",
        ] {
            assert!(is_test_path(Path::new(p)), "{p}");
        }
        for p in ["src/secret_scan.rs", "src/contest.rs", "src/latest/config.json"] {
            assert!(!is_test_path(Path::new(p)), "{p}");
        }
    }
}
