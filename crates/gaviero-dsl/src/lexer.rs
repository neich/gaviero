use std::fmt;

use chumsky::span::SimpleSpan;
use logos::Logos;

/// All terminals in the Gaviero DSL grammar.
#[derive(Logos, Debug, Clone, PartialEq)]
#[logos(skip r"[ \t\r\n]+")] // whitespace
#[logos(skip r"//[^\n]*")] // line comments
pub enum Token {
    // ── Declaration keywords ─────────────────────────────────────
    #[token("client")]
    KwClient,
    #[token("agent")]
    KwAgent,
    #[token("workflow")]
    KwWorkflow,

    // ── Field keywords ───────────────────────────────────────────
    #[token("tier")]
    KwTier,
    #[token("model")]
    KwModel,
    #[token("privacy")]
    KwPrivacy,
    #[token("scope")]
    KwScope,
    #[token("owned")]
    KwOwned,
    #[token("read_only")]
    KwReadOnly,
    #[token("depends_on")]
    KwDependsOn,
    #[token("prompt")]
    KwPrompt,
    #[token("description")]
    KwDescription,
    #[token("max_retries")]
    KwMaxRetries,
    #[token("steps")]
    KwSteps,
    #[token("max_parallel")]
    KwMaxParallel,
    #[token("memory")]
    KwMemory,
    #[token("read_ns")]
    KwReadNs,
    #[token("write_ns")]
    KwWriteNs,
    #[token("importance")]
    KwImportance,
    #[token("staleness_sources")]
    KwStalenessSources,

    // ── Tier value keywords ──────────────────────────────────────
    #[token("coordinator")]
    TierCoordinator,
    #[token("reasoning")]
    TierReasoning,
    #[token("execution")]
    TierExecution,
    #[token("mechanical")]
    TierMechanical,

    // ── Privacy value keywords ───────────────────────────────────
    #[token("public")]
    PrivPublic,
    #[token("local_only")]
    PrivLocalOnly,

    // ── Punctuation ──────────────────────────────────────────────
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,

    // ── Literals ─────────────────────────────────────────────────
    /// Double-quoted string: `"hello world"`
    #[regex(r#""[^"]*""#, |lex| {
        let s = lex.slice();
        s[1..s.len() - 1].to_owned()
    })]
    Str(String),

    /// Raw block string: `#" ... "#` — may span multiple lines, no escape processing.
    #[token("#\"", lex_raw_string)]
    RawStr(String),

    /// Non-negative integer.
    #[regex(r"[0-9]+", |lex| lex.slice().parse::<u64>().ok())]
    Int(u64),

    /// Floating-point literal: `0.9`, `1.0` — must contain a decimal point.
    /// Must appear before `Ident` so logos gives it higher priority.
    #[regex(r"[0-9]+\.[0-9]+", |lex| lex.slice().parse::<f32>().ok())]
    Float(f32),

    /// Identifier: starts with letter or `_`, may contain alphanumeric, `_`, `-`.
    /// Must come after all keyword tokens so keywords have higher priority.
    #[regex(r"[a-zA-Z_][a-zA-Z0-9_\-]*", |lex| lex.slice().to_owned())]
    Ident(String),
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::KwClient => write!(f, "client"),
            Token::KwAgent => write!(f, "agent"),
            Token::KwWorkflow => write!(f, "workflow"),
            Token::KwTier => write!(f, "tier"),
            Token::KwModel => write!(f, "model"),
            Token::KwPrivacy => write!(f, "privacy"),
            Token::KwScope => write!(f, "scope"),
            Token::KwOwned => write!(f, "owned"),
            Token::KwReadOnly => write!(f, "read_only"),
            Token::KwDependsOn => write!(f, "depends_on"),
            Token::KwPrompt => write!(f, "prompt"),
            Token::KwDescription => write!(f, "description"),
            Token::KwMaxRetries => write!(f, "max_retries"),
            Token::KwSteps => write!(f, "steps"),
            Token::KwMaxParallel => write!(f, "max_parallel"),
            Token::KwMemory => write!(f, "memory"),
            Token::KwReadNs => write!(f, "read_ns"),
            Token::KwWriteNs => write!(f, "write_ns"),
            Token::KwImportance => write!(f, "importance"),
            Token::KwStalenessSources => write!(f, "staleness_sources"),
            Token::TierCoordinator => write!(f, "coordinator"),
            Token::TierReasoning => write!(f, "reasoning"),
            Token::TierExecution => write!(f, "execution"),
            Token::TierMechanical => write!(f, "mechanical"),
            Token::PrivPublic => write!(f, "public"),
            Token::PrivLocalOnly => write!(f, "local_only"),
            Token::LBrace => write!(f, "{{"),
            Token::RBrace => write!(f, "}}"),
            Token::LBracket => write!(f, "["),
            Token::RBracket => write!(f, "]"),
            Token::Str(s) => write!(f, "\"{}\"", s),
            Token::RawStr(s) => write!(f, "#\"{}\"#", s),
            Token::Int(n) => write!(f, "{}", n),
            Token::Float(v) => write!(f, "{}", v),
            Token::Ident(s) => write!(f, "{}", s),
        }
    }
}

/// Callback for `#"..."#` raw strings.
/// Scans forward from current position until the closing `"#` sentinel.
/// Returns `Some(content)` on success, `None` on unterminated string (→ lex error).
fn lex_raw_string(lex: &mut logos::Lexer<Token>) -> Option<String> {
    let rest = lex.remainder();
    match rest.find("\"#") {
        Some(end) => {
            lex.bump(end + 2); // consume content + closing `"#`
            Some(rest[..end].trim().to_owned())
        }
        None => None, // unterminated → logos produces an error token
    }
}

// ── Public API ────────────────────────────────────────────────────────────

/// Tokenise `source`.
///
/// Returns `(tokens, lex_error_spans)`.
/// `tokens` is a vec of `(Token, SimpleSpan)` ready for chumsky.
/// `lex_error_spans` is non-empty when unknown characters were found.
pub fn lex(source: &str) -> (Vec<(Token, SimpleSpan)>, Vec<SimpleSpan>) {
    let mut tokens = Vec::new();
    let mut errors = Vec::new();

    for (result, span) in Token::lexer(source).spanned() {
        let cspan = SimpleSpan::from(span.start..span.end);
        match result {
            Ok(tok) => tokens.push((tok, cspan)),
            Err(_) => errors.push(cspan),
        }
    }

    (tokens, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_tokenise() {
        let src = "client agent workflow tier model privacy scope owned read_only depends_on prompt description max_retries steps max_parallel memory read_ns write_ns importance staleness_sources";
        let (toks, errs) = lex(src);
        assert!(errs.is_empty(), "unexpected lex errors: {:?}", errs);
        let kinds: Vec<_> = toks.iter().map(|(t, _)| t).collect();
        assert!(matches!(kinds[0], Token::KwClient));
        assert!(matches!(kinds[1], Token::KwAgent));
        assert!(matches!(kinds[2], Token::KwWorkflow));
    }

    #[test]
    fn tier_and_privacy_values() {
        let (toks, errs) = lex("coordinator reasoning execution mechanical public local_only");
        assert!(errs.is_empty());
        assert!(matches!(toks[0].0, Token::TierCoordinator));
        assert!(matches!(toks[3].0, Token::TierMechanical));
        assert!(matches!(toks[4].0, Token::PrivPublic));
        assert!(matches!(toks[5].0, Token::PrivLocalOnly));
    }

    #[test]
    fn quoted_string() {
        let (toks, errs) = lex(r#""hello world""#);
        assert!(errs.is_empty());
        assert_eq!(toks.len(), 1);
        assert!(matches!(&toks[0].0, Token::Str(s) if s == "hello world"));
    }

    #[test]
    fn raw_string_single_line() {
        let src = r##"#"do the thing"#"##;
        let (toks, errs) = lex(src);
        assert!(errs.is_empty(), "lex errors: {:?}", errs);
        assert_eq!(toks.len(), 1);
        assert!(matches!(&toks[0].0, Token::RawStr(s) if s == "do the thing"));
    }

    #[test]
    fn raw_string_multiline() {
        let src = "#\"\n    first line\n    second line\n\"#";
        let (toks, errs) = lex(src);
        assert!(errs.is_empty(), "lex errors: {:?}", errs);
        assert_eq!(toks.len(), 1);
        if let Token::RawStr(s) = &toks[0].0 {
            assert!(s.contains("first line"));
            assert!(s.contains("second line"));
        } else {
            panic!("expected RawStr");
        }
    }

    #[test]
    fn identifier_with_hyphen() {
        let (toks, errs) = lex("my-agent");
        assert!(errs.is_empty());
        assert!(matches!(&toks[0].0, Token::Ident(s) if s == "my-agent"));
    }

    #[test]
    fn integer() {
        let (toks, errs) = lex("42");
        assert!(errs.is_empty());
        assert!(matches!(toks[0].0, Token::Int(42)));
    }

    #[test]
    fn float_literal() {
        let (toks, errs) = lex("0.9");
        assert!(errs.is_empty(), "lex errors: {:?}", errs);
        assert_eq!(toks.len(), 1);
        assert!(matches!(toks[0].0, Token::Float(v) if (v - 0.9).abs() < 1e-5));
    }

    #[test]
    fn memory_keywords() {
        let (toks, errs) = lex("memory read_ns write_ns importance staleness_sources");
        assert!(errs.is_empty(), "lex errors: {:?}", errs);
        assert!(matches!(toks[0].0, Token::KwMemory));
        assert!(matches!(toks[1].0, Token::KwReadNs));
        assert!(matches!(toks[2].0, Token::KwWriteNs));
        assert!(matches!(toks[3].0, Token::KwImportance));
        assert!(matches!(toks[4].0, Token::KwStalenessSources));
    }

    #[test]
    fn unknown_char_produces_error() {
        let (_, errs) = lex("agent @ bad");
        assert!(!errs.is_empty(), "expected lex error for '@'");
    }
}
