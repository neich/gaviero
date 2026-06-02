//! Git merge-conflict marker parsing and region detection.
//!
//! Recognizes standard `<<<<<<<` / `=======` / `>>>>>>>` hunks in working-tree
//! files. Used by the TUI Changes panel and editor conflict navigation.

/// One contiguous conflict hunk in a file (0-based line indices).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictRegion {
    pub start_line: usize,
    pub sep_line: usize,
    pub end_line: usize,
}

/// Which part of a conflict hunk a line belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictLineKind {
    /// `<<<<<<<` marker line.
    Start,
    /// Lines between `<<<<<<<` and `=======` (current branch / ours).
    Ours,
    /// `=======` separator line.
    Sep,
    /// Lines between `=======` and `>>>>>>>` (incoming / theirs).
    Theirs,
    /// `>>>>>>>` marker line.
    End,
    /// Outside any conflict hunk.
    Normal,
}

/// True if `content` contains at least one standard conflict marker line.
pub fn file_has_conflict_markers(content: &str) -> bool {
    content
        .lines()
        .any(|line| is_conflict_start(line) || is_conflict_sep(line) || is_conflict_end(line))
}

/// Find all well-formed conflict regions in `content`.
pub fn find_conflict_regions(content: &str) -> Vec<ConflictRegion> {
    let lines: Vec<&str> = content.lines().collect();
    let mut regions = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !is_conflict_start(lines[i]) {
            i += 1;
            continue;
        }
        let start = i;
        let mut sep = None;
        let mut end = None;
        i += 1;
        while i < lines.len() {
            if is_conflict_sep(lines[i]) {
                sep = Some(i);
                i += 1;
                break;
            }
            if is_conflict_start(lines[i]) || is_conflict_end(lines[i]) {
                break;
            }
            i += 1;
        }
        if let Some(sep_line) = sep {
            while i < lines.len() {
                if is_conflict_end(lines[i]) {
                    end = Some(i);
                    i += 1;
                    break;
                }
                if is_conflict_start(lines[i]) || is_conflict_sep(lines[i]) {
                    break;
                }
                i += 1;
            }
            if let Some(end_line) = end {
                regions.push(ConflictRegion {
                    start_line: start,
                    sep_line,
                    end_line,
                });
                continue;
            }
        }
        // Malformed — skip this start marker.
        i = start + 1;
    }
    regions
}

/// Classify a single line within a file that may contain conflict markers.
pub fn classify_conflict_line(line: &str, line_idx: usize, regions: &[ConflictRegion]) -> ConflictLineKind {
    for region in regions {
        if line_idx == region.start_line {
            return ConflictLineKind::Start;
        }
        if line_idx == region.sep_line {
            return ConflictLineKind::Sep;
        }
        if line_idx == region.end_line {
            return ConflictLineKind::End;
        }
        if line_idx > region.start_line && line_idx < region.sep_line {
            return ConflictLineKind::Ours;
        }
        if line_idx > region.sep_line && line_idx < region.end_line {
            return ConflictLineKind::Theirs;
        }
    }
    if is_conflict_start(line) {
        return ConflictLineKind::Start;
    }
    if is_conflict_sep(line) {
        return ConflictLineKind::Sep;
    }
    if is_conflict_end(line) {
        return ConflictLineKind::End;
    }
    ConflictLineKind::Normal
}

/// Build a per-line annotated view of `content` for conflict-aware diff rendering.
pub fn build_conflict_annotated_lines(content: &str) -> Vec<(ConflictLineKind, String)> {
    let regions = find_conflict_regions(content);
    content
        .lines()
        .enumerate()
        .map(|(i, line)| {
            let kind = classify_conflict_line(line, i, &regions);
            (kind, line.to_string())
        })
        .collect()
}

fn is_conflict_start(line: &str) -> bool {
    line.starts_with("<<<<<<<")
}

fn is_conflict_sep(line: &str) -> bool {
    line.starts_with("=======") && !line.starts_with("========")
}

fn is_conflict_end(line: &str) -> bool {
    line.starts_with(">>>>>>>")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "before
<<<<<<< HEAD
ours line
=======
theirs line
>>>>>>> branch
after
";

    #[test]
    fn detects_markers() {
        assert!(file_has_conflict_markers(SAMPLE));
        assert!(!file_has_conflict_markers("no conflicts here\n"));
    }

    #[test]
    fn finds_one_region() {
        let regions = find_conflict_regions(SAMPLE);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].start_line, 1);
        assert_eq!(regions[0].sep_line, 3);
        assert_eq!(regions[0].end_line, 5);
    }

    #[test]
    fn classifies_lines() {
        let regions = find_conflict_regions(SAMPLE);
        assert_eq!(
            classify_conflict_line("before", 0, &regions),
            ConflictLineKind::Normal
        );
        assert_eq!(
            classify_conflict_line("<<<<<<< HEAD", 1, &regions),
            ConflictLineKind::Start
        );
        assert_eq!(
            classify_conflict_line("ours line", 2, &regions),
            ConflictLineKind::Ours
        );
        assert_eq!(
            classify_conflict_line("theirs line", 4, &regions),
            ConflictLineKind::Theirs
        );
    }

    #[test]
    fn annotated_lines_cover_content() {
        let lines = build_conflict_annotated_lines(SAMPLE);
        assert_eq!(lines.len(), 7);
        assert_eq!(lines[2].0, ConflictLineKind::Ours);
        assert_eq!(lines[4].0, ConflictLineKind::Theirs);
    }
}
