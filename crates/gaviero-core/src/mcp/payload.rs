//! Estimated size of one logical MCP tool payload.
//!
//! One convention, shared by `--mcp-stats` and the `repo_outline` budget:
//! the payload is serialized as compact JSON (serde's default, which does
//! not escape non-ASCII), and its UTF-8 byte length is divided by four,
//! rounded up. It is the same rule as `history::tokens::
//! estimate_json_text_tokens`, restated here because `mcp` must not
//! import `history`.
//!
//! This is an estimate of *one* copy of the application payload. It is not
//! a provider token count, and it excludes the MCP envelope, the duplicate
//! `content` text rmcp sends alongside `structuredContent`, and any
//! pretty-printing a client applies.

use serde::Serialize;

/// Compact-JSON byte length of `value`, or `None` if it cannot serialize.
pub fn compact_json_len<T: Serialize + ?Sized>(value: &T) -> Option<usize> {
    serde_json::to_vec(value).ok().map(|v| v.len())
}

/// Estimated tokens for `bytes` of compact JSON: `ceil(bytes / 4)`.
pub fn estimated_tokens(bytes: usize) -> usize {
    bytes.div_ceil(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_utf8_bytes_of_compact_json() {
        // ASCII: {"a":"b"} = 9 bytes → 3 tokens.
        let ascii = serde_json::json!({"a": "b"});
        assert_eq!(compact_json_len(&ascii), Some(9));
        assert_eq!(estimated_tokens(9), 3);
        // Non-ASCII stays raw UTF-8: "→" is 3 bytes, not a 6-byte \u escape.
        let unicode = serde_json::json!({"a": "→"});
        assert_eq!(compact_json_len(&unicode), Some(11));
        // Escapes count as written: "a\"b\n" serializes to 8 bytes in quotes.
        let escaped = serde_json::json!("a\"b\n");
        assert_eq!(
            compact_json_len(&escaped),
            Some(serde_json::to_string(&escaped).unwrap().len())
        );
        assert_eq!(compact_json_len(&escaped), Some(8));
        assert_eq!(estimated_tokens(0), 0);
        assert_eq!(estimated_tokens(1), 1);
        assert_eq!(estimated_tokens(4), 1);
        assert_eq!(estimated_tokens(5), 2);
    }
}
