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
}
