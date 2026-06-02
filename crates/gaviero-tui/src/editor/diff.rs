//! Line-level unified diff used by the Changes panel and the buffer-backed
//! diff view (read-only diff overlay rendered as a regular editor tab).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffKind {
    Context,
    Added,
    Removed,
    /// `<<<<<<<`, `=======`, or `>>>>>>>` marker line.
    ConflictMarker,
    /// Lines between `<<<<<<<` and `=======`.
    ConflictOurs,
    /// Lines between `=======` and `>>>>>>>`.
    ConflictTheirs,
}

/// Map a git-conflict line classification to diff rendering kind.
pub fn diff_kind_from_conflict(
    kind: gaviero_core::git_conflict::ConflictLineKind,
) -> DiffKind {
    use gaviero_core::git_conflict::ConflictLineKind;
    match kind {
        ConflictLineKind::Start | ConflictLineKind::Sep | ConflictLineKind::End => {
            DiffKind::ConflictMarker
        }
        ConflictLineKind::Ours => DiffKind::ConflictOurs,
        ConflictLineKind::Theirs => DiffKind::ConflictTheirs,
        ConflictLineKind::Normal => DiffKind::Context,
    }
}

/// Per-line conflict annotation for the Changes panel (working-tree file).
pub fn build_conflict_diff(content: &str) -> Vec<(DiffKind, String)> {
    gaviero_core::git_conflict::build_conflict_annotated_lines(content)
        .into_iter()
        .map(|(k, line)| (diff_kind_from_conflict(k), line))
        .collect()
}

/// LCS-based line diff. Each output line is one of `Context` / `Added` /
/// `Removed`. Lines from `old` and `new` that match in order become Context;
/// the rest are emitted as Added (new-only) or Removed (old-only).
pub fn build_simple_diff(old: &[&str], new: &[&str]) -> Vec<(DiffKind, String)> {
    let m = old.len();
    let n = new.len();

    let mut dp = vec![vec![0u32; n + 1]; m + 1];
    for i in 1..=m {
        for j in 1..=n {
            dp[i][j] = if old[i - 1] == new[j - 1] {
                dp[i - 1][j - 1] + 1
            } else {
                dp[i - 1][j].max(dp[i][j - 1])
            };
        }
    }

    let mut result = Vec::new();
    let (mut i, mut j) = (m, n);
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && old[i - 1] == new[j - 1] {
            result.push((DiffKind::Context, old[i - 1].to_string()));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
            result.push((DiffKind::Added, new[j - 1].to_string()));
            j -= 1;
        } else {
            result.push((DiffKind::Removed, old[i - 1].to_string()));
            i -= 1;
        }
    }

    result.reverse();
    result
}
