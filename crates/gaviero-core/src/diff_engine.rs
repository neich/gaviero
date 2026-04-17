use crate::types::{DiffHunk, HunkType};
use similar::{ChangeTag, TextDiff};

/// Compute diff hunks between original and proposed file content.
/// Returns a list of DiffHunks with 0-indexed line ranges.
pub fn compute_hunks(original: &str, proposed: &str) -> Vec<DiffHunk> {
    let diff = TextDiff::from_lines(original, proposed);
    let mut hunks = Vec::new();

    const DIFF_CONTEXT_LINES: usize = 3;
    for group in diff.grouped_ops(DIFF_CONTEXT_LINES) {
        for op in &group {
            let old_start = op.old_range().start;
            let old_end = op.old_range().end;
            let new_start = op.new_range().start;
            let new_end = op.new_range().end;

            let original_text: String = diff
                .iter_changes(op)
                .filter(|c| c.tag() != ChangeTag::Insert)
                .map(|c| c.to_string_lossy().into_owned())
                .collect();

            let proposed_text: String = diff
                .iter_changes(op)
                .filter(|c| c.tag() != ChangeTag::Delete)
                .map(|c| c.to_string_lossy().into_owned())
                .collect();

            let hunk_type = match op.tag() {
                similar::DiffTag::Insert => HunkType::Added,
                similar::DiffTag::Delete => HunkType::Removed,
                similar::DiffTag::Replace => HunkType::Modified,
                similar::DiffTag::Equal => continue,
            };

            hunks.push(DiffHunk {
                original_range: (old_start, old_end),
                proposed_range: (new_start, new_end),
                original_text,
                proposed_text,
                hunk_type,
            });
        }
    }

    hunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_changes() {
        let hunks = compute_hunks("hello\n", "hello\n");
        assert!(hunks.is_empty());
    }

    #[test]
    fn test_added_lines() {
        let original = "line1\nline3\n";
        let proposed = "line1\nline2\nline3\n";
        let hunks = compute_hunks(original, proposed);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].hunk_type, HunkType::Added);
        assert!(hunks[0].proposed_text.contains("line2"));
    }

    #[test]
    fn test_removed_lines() {
        let original = "line1\nline2\nline3\n";
        let proposed = "line1\nline3\n";
        let hunks = compute_hunks(original, proposed);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].hunk_type, HunkType::Removed);
        assert!(hunks[0].original_text.contains("line2"));
    }

    #[test]
    fn test_modified_lines() {
        let original = "fn old() {}\n";
        let proposed = "fn new() {}\n";
        let hunks = compute_hunks(original, proposed);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].hunk_type, HunkType::Modified);
        assert!(hunks[0].original_text.contains("old"));
        assert!(hunks[0].proposed_text.contains("new"));
    }

    #[test]
    fn test_multiple_hunks() {
        let original = "aaa\nbbb\nccc\nddd\neee\nfff\n";
        let proposed = "aaa\nBBB\nccc\nddd\nEEE\nfff\n";
        let hunks = compute_hunks(original, proposed);
        assert_eq!(hunks.len(), 2);
    }

    #[test]
    fn test_ranges_are_correct() {
        let original = "a\nb\nc\n";
        let proposed = "a\nB\nc\n";
        let hunks = compute_hunks(original, proposed);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].original_range, (1, 2)); // line "b" is index 1
        assert_eq!(hunks[0].proposed_range, (1, 2));
    }
}
