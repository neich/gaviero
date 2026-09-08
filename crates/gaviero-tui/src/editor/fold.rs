//! Foldable-region discovery and hidden-line bookkeeping for the editor.
//!
//! A [`FoldRegion`] keeps its *header* line on screen and hides everything from
//! `header + 1` through `last_hidden`. Regions come from one of three sources,
//! picked in this order:
//!
//! 1. the tree-sitter tree, when the buffer has one (every code language plus
//!    JSON / TOML / YAML);
//! 2. a markdown scanner — markdown has a language *name* but no grammar
//!    ([`gaviero_core::tree_sitter`] maps `md` to `(.., None)`), so headings and
//!    fenced blocks are found by line scan;
//! 3. an indentation scan, so plain text and unsupported languages still fold.
//!
//! The closing line of a region stays visible (`last_hidden = end - 1` for
//! tree nodes and fences): in a terminal a lone `}` under a collapsed header
//! reads far better than a block that vanishes entirely.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use gaviero_core::Tree;
use ropey::Rope;

/// What the gutter draws in the fold column of one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldMarker {
    /// Nothing folds here.
    None,
    /// A region starts here and its body is visible.
    Expanded,
    /// A region starts here and its body is hidden.
    Collapsed,
}

/// One collapsible block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldRegion {
    /// Line carrying the arrow; always stays visible.
    pub header: usize,
    /// Last line hidden when the region is collapsed (inclusive).
    pub last_hidden: usize,
}

/// Every foldable region in a buffer, keyed by header line.
///
/// At most one region per header: when several tree-sitter nodes start on the
/// same row (`fn f() {` is both a `function_item` and a `block`) the widest one
/// wins, so a line never carries two arrows.
#[derive(Clone, Debug, Default)]
pub struct FoldMap {
    regions: Vec<FoldRegion>,
}

impl FoldMap {
    /// Discover every foldable region in `text`.
    pub fn compute(text: &Rope, tree: Option<&Tree>, lang_name: Option<&str>) -> Self {
        let by_header = if let Some(tree) = tree {
            tree_regions(tree)
        } else {
            let lines = collect_lines(text);
            if lang_name == Some("markdown") {
                markdown_regions(&lines)
            } else {
                indent_regions(&lines)
            }
        };

        let regions = by_header
            .into_iter()
            .map(|(header, last_hidden)| FoldRegion {
                header,
                last_hidden,
            })
            .collect();
        Self { regions }
    }

    /// The region starting exactly at `header`, if any.
    pub fn region_at(&self, header: usize) -> Option<FoldRegion> {
        self.regions
            .binary_search_by_key(&header, |r| r.header)
            .ok()
            .map(|i| self.regions[i])
    }

    /// The innermost region containing `line`, header included.
    ///
    /// Used by the keyboard toggle, where the cursor usually sits *inside* a
    /// block rather than on its header line.
    pub fn enclosing_region(&self, line: usize) -> Option<FoldRegion> {
        if let Some(exact) = self.region_at(line) {
            return Some(exact);
        }
        // Regions are sorted by header, so the innermost candidate is the last
        // one that starts at or before `line` and still reaches it.
        self.regions
            .iter()
            .rev()
            .find(|r| r.header < line && line <= r.last_hidden)
            .copied()
    }

    pub fn headers(&self) -> impl Iterator<Item = usize> + '_ {
        self.regions.iter().map(|r| r.header)
    }

    /// Merge `collapsed` headers into the disjoint set of hidden line ranges.
    ///
    /// Headers with no matching region are ignored, which is what keeps a stale
    /// fold (left behind by an edit that reshaped the block) inert instead of
    /// hiding the wrong lines.
    pub fn hidden_lines(&self, collapsed: &BTreeSet<usize>) -> HiddenLines {
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        for &header in collapsed {
            let Some(region) = self.region_at(header) else {
                continue;
            };
            let (start, end) = (header + 1, region.last_hidden);
            if start > end {
                continue;
            }
            match ranges.last_mut() {
                // `collapsed` is ascending, so a new range can only extend or
                // sit inside the previous one — never precede it.
                Some(last) if start <= last.1 + 1 => last.1 = last.1.max(end),
                _ => ranges.push((start, end)),
            }
        }
        HiddenLines { ranges }
    }
}

/// The lines currently hidden by collapsed folds, as disjoint ascending ranges.
///
/// Doubles as the map between logical lines and the visual rows the editor
/// actually draws. Empty — the overwhelmingly common case — means the two
/// spaces coincide and every lookup is a no-op.
#[derive(Clone, Debug, Default)]
pub struct HiddenLines {
    /// Inclusive, disjoint, ascending.
    ranges: Vec<(usize, usize)>,
}

impl HiddenLines {
    pub fn contains(&self, line: usize) -> bool {
        self.ranges
            .binary_search_by(|&(start, end)| {
                if line < start {
                    Ordering::Greater
                } else if line > end {
                    Ordering::Less
                } else {
                    Ordering::Equal
                }
            })
            .is_ok()
    }

    /// How many lines before `line` are hidden.
    pub fn hidden_before(&self, line: usize) -> usize {
        let mut hidden = 0;
        for &(start, end) in &self.ranges {
            if start >= line {
                break;
            }
            hidden += end.min(line - 1) - start + 1;
        }
        hidden
    }

    pub fn total(&self) -> usize {
        self.ranges.iter().map(|&(s, e)| e - s + 1).sum()
    }

    /// Visual row that a *visible* logical line is drawn on.
    pub fn visual_row_of(&self, line: usize) -> usize {
        line - self.hidden_before(line)
    }

    /// Logical line drawn on visual row `row`.
    pub fn line_at_row(&self, row: usize) -> usize {
        let mut line = row;
        for &(start, end) in &self.ranges {
            if line < start {
                break;
            }
            line += end - start + 1;
        }
        line
    }
}

fn collect_lines(text: &Rope) -> Vec<String> {
    (0..text.len_lines())
        .map(|i| text.line(i).to_string())
        .collect()
}

fn merge(out: &mut BTreeMap<usize, usize>, header: usize, last_hidden: usize) {
    out.entry(header)
        .and_modify(|e| *e = (*e).max(last_hidden))
        .or_insert(last_hidden);
}

/// Every multi-line tree-sitter node becomes a region on its start row.
///
/// The closing row is kept visible, so a node must span at least three rows to
/// be worth an arrow — hiding the single line of a two-row node would only
/// remove its terminator.
///
/// The root is walked but never emits a region of its own: it spans the whole
/// file, and an arrow on line 1 that swallows the entire buffer is "fold all"
/// wearing a disguise. Languages whose root wraps a real block (JSON's
/// `document` around its top-level `object`) still get that block's arrow from
/// the child.
fn tree_regions(tree: &Tree) -> BTreeMap<usize, usize> {
    let mut out = BTreeMap::new();
    let root = tree.root_node();
    let mut stack: Vec<_> = (0..root.child_count())
        .filter_map(|i| root.child(i))
        .collect();

    while let Some(node) = stack.pop() {
        let start = node.start_position().row;
        let end = node.end_position().row;
        // A node confined to one row cannot contain a multi-row descendant, so
        // the walk stops there. That prunes almost every leaf in a large file.
        if end == start {
            continue;
        }
        if end > start + 1 {
            merge(&mut out, start, end - 1);
        }
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                stack.push(child);
            }
        }
    }
    out
}

/// ATX heading level (`## x` → 2), or `None` when the line is not a heading.
fn atx_level(trimmed: &str) -> Option<usize> {
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    match trimmed[hashes..].chars().next() {
        None | Some(' ') | Some('\t') => Some(hashes),
        _ => None,
    }
}

/// Leading fence marker (` ``` ` / `~~~`) as `(char, run length)`.
fn fence_marker(trimmed: &str) -> Option<(char, usize)> {
    let first = trimmed.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let run = trimmed.chars().take_while(|c| *c == first).count();
    (run >= 3).then_some((first, run))
}

/// Markdown regions: one per ATX heading section, one per fenced block.
///
/// A section runs to the line before the next heading of the same or a higher
/// level, minus trailing blank lines so folding does not swallow the blank
/// separator before the next heading.
fn markdown_regions(lines: &[String]) -> BTreeMap<usize, usize> {
    let mut out = BTreeMap::new();
    let mut headings: Vec<(usize, usize)> = Vec::new(); // (level, line)
    let mut open_fence: Option<(char, usize, usize)> = None; // (char, len, line)

    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let marker = fence_marker(trimmed);

        match (open_fence, marker) {
            (None, Some((ch, len))) => open_fence = Some((ch, len, i)),
            (Some((open_ch, open_len, start)), Some((ch, len)))
                if ch == open_ch && len >= open_len =>
            {
                if i > start + 1 {
                    merge(&mut out, start, i - 1);
                }
                open_fence = None;
            }
            // Headings inside a fence are code, not structure.
            (None, None) => {
                if let Some(level) = atx_level(trimmed) {
                    headings.push((level, i));
                }
            }
            _ => {}
        }
    }

    let last_line = lines.len().saturating_sub(1);
    for (idx, &(level, line)) in headings.iter().enumerate() {
        let mut end = headings[idx + 1..]
            .iter()
            .find(|(next_level, _)| *next_level <= level)
            .map(|(_, next_line)| next_line - 1)
            .unwrap_or(last_line);
        while end > line && lines[end].trim().is_empty() {
            end -= 1;
        }
        if end > line {
            merge(&mut out, line, end);
        }
    }

    out
}

/// Indentation width, tabs counted as four columns.
fn indent_width(line: &str) -> usize {
    line.chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum()
}

/// Fallback for buffers with no grammar: a line owns every deeper-indented line
/// that follows it. Single pass over a stack of open indents.
fn indent_regions(lines: &[String]) -> BTreeMap<usize, usize> {
    let mut out = BTreeMap::new();
    let mut stack: Vec<(usize, usize)> = Vec::new(); // (indent, header line)
    let mut last_content = 0usize;

    for (i, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let indent = indent_width(line);
        while let Some(&(open_indent, header)) = stack.last() {
            if open_indent < indent {
                break;
            }
            stack.pop();
            if last_content > header {
                merge(&mut out, header, last_content);
            }
        }
        stack.push((indent, i));
        last_content = i;
    }
    while let Some((_, header)) = stack.pop() {
        if last_content > header {
            merge(&mut out, header, last_content);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map_of(pairs: &[(usize, usize)]) -> FoldMap {
        FoldMap {
            regions: pairs
                .iter()
                .map(|&(header, last_hidden)| FoldRegion {
                    header,
                    last_hidden,
                })
                .collect(),
        }
    }

    fn collapsed(lines: &[usize]) -> BTreeSet<usize> {
        lines.iter().copied().collect()
    }

    #[test]
    fn markdown_folds_sections_by_heading_level() {
        let src = "# Title\nintro\n\n## A\na body\n\n## B\nb body\n";
        let lines = collect_lines(&Rope::from_str(src));
        let regions = markdown_regions(&lines);

        // `# Title` owns everything to the last content line.
        assert_eq!(regions.get(&0), Some(&7));
        // `## A` stops before `## B`, blank separator trimmed off.
        assert_eq!(regions.get(&3), Some(&4));
        assert_eq!(regions.get(&6), Some(&7));
    }

    #[test]
    fn markdown_ignores_headings_inside_a_fence() {
        let src = "# Real\n\n```sh\n# not a heading\necho hi\n```\n\ntail\n";
        let lines = collect_lines(&Rope::from_str(src));
        let regions = markdown_regions(&lines);

        // The fence itself folds, keeping its closing line visible.
        assert_eq!(regions.get(&2), Some(&4));
        // The `#` on line 3 never became a section.
        assert!(!regions.contains_key(&3));
        assert_eq!(regions.get(&0), Some(&7));
    }

    #[test]
    fn indent_scan_nests_regions() {
        let src = "a\n  b\n    c\n    d\ne\n";
        let lines = collect_lines(&Rope::from_str(src));
        let regions = indent_regions(&lines);

        assert_eq!(regions.get(&0), Some(&3));
        assert_eq!(regions.get(&1), Some(&3));
        // `c` has nothing under it.
        assert!(!regions.contains_key(&2));
    }

    #[test]
    fn hidden_lines_merge_nested_collapses() {
        let map = map_of(&[(0, 10), (2, 5), (20, 25)]);
        let hidden = map.hidden_lines(&collapsed(&[0, 2, 20]));

        assert!(hidden.contains(1));
        assert!(hidden.contains(10));
        assert!(!hidden.contains(0));
        assert!(!hidden.contains(11));
        assert!(hidden.contains(21));
        // 1..=10 merged from the two nested collapses, plus 21..=25.
        assert_eq!(hidden.total(), 10 + 5);
    }

    #[test]
    fn hidden_lines_map_between_logical_lines_and_visual_rows() {
        let map = map_of(&[(2, 5), (9, 11)]);
        let hidden = map.hidden_lines(&collapsed(&[2, 9]));

        // Lines 3..=5 and 10..=11 are hidden, leaving 0,1,2,6,7,8,9,12,…
        assert_eq!(hidden.visual_row_of(0), 0);
        assert_eq!(hidden.visual_row_of(2), 2);
        assert_eq!(hidden.visual_row_of(6), 3);
        assert_eq!(hidden.visual_row_of(12), 7);

        assert_eq!(hidden.line_at_row(2), 2);
        assert_eq!(hidden.line_at_row(3), 6);
        assert_eq!(hidden.line_at_row(7), 12);
        // Round-trips for every visible line.
        for line in [0, 1, 2, 6, 7, 8, 9, 12, 13] {
            assert_eq!(hidden.line_at_row(hidden.visual_row_of(line)), line);
        }
    }

    #[test]
    fn a_stale_collapsed_header_hides_nothing() {
        let map = map_of(&[(0, 4)]);
        let hidden = map.hidden_lines(&collapsed(&[0, 7]));
        assert_eq!(hidden.total(), 4);
        assert!(!hidden.contains(8));
    }

    #[test]
    fn enclosing_region_finds_the_innermost_block() {
        let map = map_of(&[(0, 20), (5, 9)]);
        assert_eq!(map.enclosing_region(7).map(|r| r.header), Some(5));
        assert_eq!(map.enclosing_region(15).map(|r| r.header), Some(0));
        assert_eq!(map.enclosing_region(5).map(|r| r.header), Some(5));
        assert_eq!(map.enclosing_region(21), None);
    }
}
