//! C1.4: zstd compression for History rows.
//!
//! Old transcripts (default `> 90 days`) are opportunistically squeezed
//! by the sleeptime pass into the `content_blob` column. Storage savings
//! are typically 5–10× on natural-language transcripts.
//!
//! The codec is **lossless**, **content-hash-preserving**, and
//! **integrity-checked on decompress**. A mismatch between the post-
//! decompress SHA-256 and the row's stored `content_hash` is treated as
//! a data-integrity alarm — it never returns silently-corrupted bytes
//! to a caller.
//!
//! This module is the codec; the storage-side dance (drop the C1.3
//! immutability trigger, UPDATE the row, reinstall the trigger, all
//! inside one transaction) lives in
//! [`crate::memory::store::MemoryStore::compress_history_row`].

use anyhow::{Context, Result, anyhow};
use sha2::{Digest, Sha256};

/// Default zstd compression level. Level 3 is the upstream default —
/// reasonable speed and ratio. We never change this at runtime (each
/// blob is self-describing thanks to the zstd frame), but newly-
/// compressed rows pick up whatever level was current when they were
/// written.
const ZSTD_LEVEL: i32 = 3;

/// Result of a successful encode: the zstd bytes plus the canonical
/// SHA-256 hex of the *original* (uncompressed) text. Callers persist
/// `bytes` into `memories.content_blob`. The hash is supplied separately
/// for crystal-clear intent — the row's `content_hash` column should
/// already equal `sha_hex` (it was set on insert from the same text);
/// returning it here lets the caller assert that invariant before
/// committing.
#[derive(Debug, Clone)]
pub struct CompressedBlob {
    pub bytes: Vec<u8>,
    pub sha_hex: String,
    /// Decompressed length in bytes. Useful for compression-ratio
    /// telemetry without a second decode.
    pub original_len: usize,
}

/// Compress a History transcript. Verifies the round-trip end-to-end
/// before returning so a corrupt decoder cannot silently land bad bytes
/// in the DB.
pub fn compress_with_verify(content: &str) -> Result<CompressedBlob> {
    let original_bytes = content.as_bytes();
    let original_len = original_bytes.len();
    let original_sha = sha_hex(original_bytes);

    let bytes = zstd::encode_all(original_bytes, ZSTD_LEVEL)
        .context("zstd encode failed")?;

    // End-to-end round-trip: decompress what we just produced and
    // verify it matches the original SHA + content. Any mismatch here
    // means the encoder produced something we'd later fail to decode
    // — abort before any storage write.
    let round_trip = zstd::decode_all(bytes.as_slice())
        .context("zstd decode round-trip failed during compress_with_verify")?;
    if round_trip.len() != original_len {
        return Err(anyhow!(
            "zstd round-trip length mismatch: original={original_len}, decoded={}",
            round_trip.len()
        ));
    }
    let round_trip_sha = sha_hex(&round_trip);
    if round_trip_sha != original_sha {
        return Err(anyhow!(
            "zstd round-trip SHA mismatch: expected={original_sha}, got={round_trip_sha}"
        ));
    }
    if round_trip != original_bytes {
        return Err(anyhow!("zstd round-trip byte mismatch despite SHA equality"));
    }

    Ok(CompressedBlob {
        bytes,
        sha_hex: original_sha,
        original_len,
    })
}

/// Decompress a `content_blob` and verify the result hashes to
/// `expected_sha_hex` (the row's `content_hash` column). The SHA gate
/// is the integrity boundary: a mismatch is **never** returned to a
/// caller as bytes — only as an error.
pub fn decompress_with_verify(blob: &[u8], expected_sha_hex: &str) -> Result<String> {
    let decoded = zstd::decode_all(blob).context("zstd decode failed")?;
    let actual_sha = sha_hex(&decoded);
    if actual_sha != expected_sha_hex {
        return Err(anyhow!(
            "history blob SHA mismatch: expected={expected_sha_hex}, got={actual_sha} \
             (data integrity alarm — row may be corrupted)"
        ));
    }
    String::from_utf8(decoded)
        .context("decompressed history blob was not valid UTF-8")
}

fn sha_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("{digest:x}")
}

/// Placeholder string installed in the `content` column of a row whose
/// canonical body has been moved to `content_blob`. Embeds the full
/// SHA-256 hex of the original (uncompressed) text so the read path
/// can verify integrity on every decompress without consulting another
/// column. Self-describing on purpose — the row carries its own audit
/// fingerprint.
///
/// Format: `"[compressed:zstd sha=<64-hex>]"`. Round-trip via
/// [`parse_compressed_placeholder`].
pub fn compressed_content_placeholder(sha_hex: &str) -> String {
    format!("[compressed:zstd sha={sha_hex}]")
}

/// Parse a [`compressed_content_placeholder`] back to its embedded SHA.
/// Returns `None` if the string isn't a valid placeholder — callers
/// treat that as a row that wasn't compressed.
pub fn parse_compressed_placeholder(s: &str) -> Option<String> {
    let inner = s.strip_prefix("[compressed:zstd sha=")?.strip_suffix(']')?;
    if inner.len() == 64 && inner.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(inner.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// C1.4 acceptance criterion #5: 1000 fixture transcripts must
    /// round-trip with SHA-matching bytes.
    #[test]
    fn round_trip_one_thousand_fixtures() {
        let mut fixtures: Vec<String> = Vec::with_capacity(1000);
        // Mix of styles: short, long, code, repeated patterns,
        // unicode, and adversarial sequences. The encoder needs to
        // survive all of them.
        for i in 0..1000 {
            let s = match i % 6 {
                0 => format!("turn-{i}: USER asks; ASSISTANT answers."),
                1 => "fn main() { println!(\"Hello, world!\"); }\n".repeat((i % 50) + 1),
                2 => format!("session-{i}\nuser: {} body here", "x".repeat(i % 200)),
                3 => format!("café résumé naïve façade ünicode мир 你好 — turn {i}"),
                4 => format!("{}{}", "\0\x01\x02\x03binary-ish", i),
                _ => "".to_string(),
            };
            fixtures.push(s);
        }

        for (idx, original) in fixtures.iter().enumerate() {
            let blob = compress_with_verify(original)
                .unwrap_or_else(|e| panic!("encode failed at {idx}: {e}"));
            let recovered = decompress_with_verify(&blob.bytes, &blob.sha_hex)
                .unwrap_or_else(|e| panic!("decode failed at {idx}: {e}"));
            assert_eq!(
                recovered, *original,
                "byte mismatch at fixture {idx} (len {})",
                original.len()
            );
        }
    }

    #[test]
    fn decompress_rejects_blob_with_wrong_sha() {
        let blob = compress_with_verify("the original content").unwrap();
        let bad_sha = "0000000000000000000000000000000000000000000000000000000000000000";
        let r = decompress_with_verify(&blob.bytes, bad_sha);
        assert!(r.is_err(), "wrong SHA must fail decompress");
        let msg = format!("{:?}", r.unwrap_err());
        assert!(
            msg.contains("data integrity alarm"),
            "alarm should be visible: {msg}"
        );
    }

    #[test]
    fn decompress_rejects_corrupted_blob() {
        let mut blob = compress_with_verify("original").unwrap();
        // Flip a byte deep enough to not be the magic header.
        let i = blob.bytes.len() / 2;
        blob.bytes[i] ^= 0xff;
        let r = decompress_with_verify(&blob.bytes, &blob.sha_hex);
        // Could fail at zstd-decode time OR at SHA check — either is
        // a correct rejection.
        assert!(r.is_err(), "corrupted blob must be rejected");
    }

    #[test]
    fn placeholder_round_trip_full_sha() {
        let sha = "abcdef0123456789aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let p = compressed_content_placeholder(sha);
        assert_eq!(p, format!("[compressed:zstd sha={sha}]"));
        assert_eq!(parse_compressed_placeholder(&p).as_deref(), Some(sha));
    }

    #[test]
    fn parse_rejects_non_placeholder_strings() {
        assert!(parse_compressed_placeholder("plain text").is_none());
        assert!(parse_compressed_placeholder("[compressed:zstd sha=tooshort]").is_none());
        assert!(parse_compressed_placeholder("[compressed:zstd sha=ZZZZ]").is_none());
    }
}
