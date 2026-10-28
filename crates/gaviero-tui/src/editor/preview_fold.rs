//! Collapsible `#` / `##` / `###` sections in the rendered markdown preview.
//!
//! Folding happens on the source: a collapsed section's body is dropped before
//! `format_chat_markdown_mapped` runs, so tables, fences and wrapping stay the
//! formatter's business. Folds are keyed by heading text plus occurrence, not
//! by line number, so editing the source in split view keeps each fold on its
//! section; a fold whose heading no longer exists is inert.

use std::collections::{HashMap, HashSet};

/// Deepest heading level the preview can fold (`###`).
pub const MAX_FOLD_LEVEL: u8 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HeadingKey {
    pub level: u8,
    pub title: String,
    /// Occurrence of this `(level, title)` pair, counted from the top.
    pub nth: usize,
}

/// A foldable heading in the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewHeading {
    pub key: HeadingKey,
    /// Source line of the heading itself.
    pub line: usize,
    /// Exclusive end of the body a fold hides. Runs to the next heading of the
    /// same or a higher level, minus trailing blank lines so a collapsed
    /// section keeps its separation from the next heading.
    pub body_end: usize,
}

impl PreviewHeading {
    pub fn body_lines(&self) -> usize {
        self.body_end.saturating_sub(self.line + 1)
    }
}

/// A heading as laid out in the rendered preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedHeading {
    pub key: HeadingKey,
    /// First rendered line of the heading.
    pub start: usize,
    /// Exclusive end of the heading's rendered lines (it can wrap).
    pub end: usize,
    pub collapsed: bool,
    /// Source lines a fold hides; 0 means there is nothing to fold.
    pub body_lines: usize,
}

/// Preview source with the collapsed sections removed.
pub struct FoldedSource {
    pub text: String,
    /// Original source line → line of `text`. A hidden line maps to the
    /// heading that hides it.
    pub line_map: Vec<usize>,
    /// Line of `text` → original source line.
    pub kept: Vec<usize>,
    pub headings: Vec<PreviewHeading>,
}

impl FoldedSource {
    /// Whether `heading` is on screen, i.e. not inside a collapsed parent.
    pub fn is_visible(&self, heading: &PreviewHeading) -> bool {
        self.line_map
            .get(heading.line)
            .and_then(|&k| self.kept.get(k))
            .is_some_and(|&orig| orig == heading.line)
    }
}

/// ATX heading level and title, using the preview formatter's rule
/// (`#` run, then a space or end of line, on the trimmed line).
fn atx_heading(line: &str) -> Option<(u8, &str)> {
    let trimmed = line.trim();
    if !trimmed.starts_with('#') {
        return None;
    }
    let level = trimmed.chars().take_while(|c| *c == '#').count().min(6);
    if !trimmed
        .get(level..level + 1)
        .is_none_or(|c| c == " " || c.is_empty())
    {
        return None;
    }
    Some((level as u8, trimmed[level..].trim()))
}

/// Level of every source line the formatter renders as a heading (`None`
/// otherwise). Mirrors its `<think>` and fence handling so a `#` line inside
/// code is never treated as a section.
fn heading_levels<'a>(lines: &[&'a str]) -> Vec<Option<(u8, &'a str)>> {
    let mut in_code = false;
    let mut in_think = false;
    lines
        .iter()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with("<think>") {
                in_think = !trimmed.ends_with("</think>");
                return None;
            }
            if trimmed.ends_with("</think>") {
                in_think = false;
                return None;
            }
            if in_think {
                return None;
            }
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                in_code = !in_code;
                return None;
            }
            if in_code {
                return None;
            }
            atx_heading(line)
        })
        .collect()
}

fn foldable_headings(lines: &[&str]) -> Vec<PreviewHeading> {
    let levels = heading_levels(lines);
    let mut seen: HashMap<(u8, String), usize> = HashMap::new();
    let mut out = Vec::new();
    for (line, entry) in levels.iter().enumerate() {
        let Some((level, title)) = *entry else {
            continue;
        };
        if level > MAX_FOLD_LEVEL {
            continue;
        }
        let end = levels[line + 1..]
            .iter()
            .position(|e| e.is_some_and(|(l, _)| l <= level))
            .map_or(lines.len(), |off| line + 1 + off);
        let mut body_end = end;
        while body_end > line + 1 && lines[body_end - 1].trim().is_empty() {
            body_end -= 1;
        }
        let nth = seen.entry((level, title.to_string())).or_insert(0);
        out.push(PreviewHeading {
            key: HeadingKey {
                level,
                title: title.to_string(),
                nth: *nth,
            },
            line,
            body_end,
        });
        *nth += 1;
    }
    out
}

/// Every foldable heading in `source`, top to bottom.
pub fn preview_headings(source: &str) -> Vec<PreviewHeading> {
    let expanded = source.replace('\t', "    ");
    let lines: Vec<&str> = expanded.lines().collect();
    foldable_headings(&lines)
}

/// Drop the bodies of the sections in `folded` from `source`.
pub fn fold_source(source: &str, folded: &HashSet<HeadingKey>) -> FoldedSource {
    // Same tab expansion and line split as the formatter, so line indices agree.
    let expanded = source.replace('\t', "    ");
    let lines: Vec<&str> = expanded.lines().collect();
    let headings = foldable_headings(&lines);
    let collapsed_at: HashMap<usize, usize> = headings
        .iter()
        .filter(|h| folded.contains(&h.key))
        .map(|h| (h.line, h.body_end))
        .collect();

    let mut text = String::with_capacity(expanded.len());
    let mut line_map = vec![0; lines.len()];
    let mut kept = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let out = kept.len();
        // One '\n' per line, never a join: `lines()` drops a final empty
        // line, which would shift every index after it.
        text.push_str(lines[i]);
        text.push('\n');
        kept.push(i);
        line_map[i] = out;
        match collapsed_at.get(&i) {
            Some(&body_end) => {
                for slot in &mut line_map[i + 1..body_end] {
                    *slot = out;
                }
                i = body_end.max(i + 1);
            }
            None => i += 1,
        }
    }
    FoldedSource {
        text,
        line_map,
        kept,
        headings,
    }
}

/// Fold set for "outline depth": collapse every section at `level`, so the
/// preview shows headings down to `level` and nothing below them.
pub fn folds_for_depth(headings: &[PreviewHeading], level: u8) -> HashSet<HeadingKey> {
    headings
        .iter()
        .filter(|h| h.key.level == level && h.body_lines() > 0)
        .map(|h| h.key.clone())
        .collect()
}

/// Next outline depth after `current` (`None` = fully expanded), skipping
/// levels that have nothing to fold. Wraps back to `None`.
pub fn next_outline_depth(headings: &[PreviewHeading], current: Option<u8>) -> Option<u8> {
    let start = current.map_or(1, |d| d + 1);
    (start..=MAX_FOLD_LEVEL).find(|&level| !folds_for_depth(headings, level).is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "\
# Top
intro

## A
a body
### A1
a1 body

## B
```
# not a heading
```

# Second
tail
";

    fn key(level: u8, title: &str) -> HeadingKey {
        HeadingKey {
            level,
            title: title.to_string(),
            nth: 0,
        }
    }

    #[test]
    fn sections_end_at_same_or_higher_level_and_skip_code() {
        let hs = preview_headings(DOC);
        let summary: Vec<(&str, usize, usize)> = hs
            .iter()
            .map(|h| (h.key.title.as_str(), h.line, h.body_end))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("Top", 0, 12),
                ("A", 3, 7),
                ("A1", 5, 7),
                ("B", 8, 12),
                ("Second", 13, 15),
            ]
        );
    }

    #[test]
    fn folding_hides_the_body_and_maps_hidden_lines_to_the_heading() {
        let folded = HashSet::from([key(2, "A")]);
        let f = fold_source(DOC, &folded);
        let lines: Vec<&str> = f.text.lines().collect();
        assert_eq!(&lines[..4], &["# Top", "intro", "", "## A"]);
        assert_eq!(lines[4], "", "the blank line before ## B stays");
        assert_eq!(lines[5], "## B");
        assert_eq!(f.line_map[4], 3);
        assert_eq!(f.line_map[6], 3);
        assert_eq!(f.line_map[8], 5);
        let a1 = f.headings.iter().find(|h| h.key.title == "A1").unwrap();
        assert!(!f.is_visible(a1));
        let b = f.headings.iter().find(|h| h.key.title == "B").unwrap();
        assert!(f.is_visible(b));
    }

    #[test]
    fn trailing_empty_line_keeps_its_index() {
        let f = fold_source("# T\nbody\n\n", &HashSet::new());
        assert_eq!(f.text.lines().count(), 3);
        assert_eq!(f.kept, vec![0, 1, 2]);
    }

    #[test]
    fn repeated_titles_get_distinct_keys() {
        let hs = preview_headings("## Notes\nx\n## Notes\ny\n");
        assert_eq!(hs[0].key.nth, 0);
        assert_eq!(hs[1].key.nth, 1);
        let f = fold_source("## Notes\nx\n## Notes\ny\n", &HashSet::from([hs[1].key.clone()]));
        assert_eq!(f.text, "## Notes\nx\n## Notes\n");
    }

    #[test]
    fn outline_depth_cycles_through_present_levels() {
        let hs = preview_headings(DOC);
        assert_eq!(next_outline_depth(&hs, None), Some(1));
        assert_eq!(next_outline_depth(&hs, Some(1)), Some(2));
        assert_eq!(next_outline_depth(&hs, Some(2)), Some(3));
        assert_eq!(next_outline_depth(&hs, Some(3)), None);

        let only_h2 = preview_headings("## A\nx\n## B\ny\n");
        assert_eq!(next_outline_depth(&only_h2, None), Some(2));
        assert_eq!(next_outline_depth(&only_h2, Some(2)), None);
    }
}
