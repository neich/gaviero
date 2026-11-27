//! Markdown formatting for the chat panel and in-editor preview.
//!
//! Converts markdown text into a sequence of styled lines (`ChatLine`), each
//! composed of one or more `StyledSegment`s so a single visual line can carry
//! mixed inline styling (bold, italic, inline code, links) on top of its base
//! style. Handles tables, headings, code blocks, lists, and horizontal rules.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use crate::editor::markdown::{SegmentKind, parse_inline};
use crate::theme;

/// A run of contiguous characters sharing a single style.
#[derive(Clone)]
pub struct StyledSegment {
    pub text: String,
    pub style: Style,
    /// Markdown destination for `[text](url)` / autolink runs. Wrapped pieces
    /// of the same link keep this so preview clicks can follow it.
    pub href: Option<String>,
}

impl StyledSegment {
    pub fn new(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
            href: None,
        }
    }
}

/// How a rendered row goes back into copied text.
///
/// Copy must give back what the agent wrote, not the panel's layout: a long
/// shell command wrapped over three rows has to paste as one line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RowCopy {
    /// Leading characters that only exist on screen (the code-block indent,
    /// the hanging indent of a wrapped list item). Copy leaves them out.
    pub skip: usize,
    /// `Some(sep)` when the row is a soft wrap of the row above: copy joins
    /// the two with `sep` (the whitespace the wrap dropped) instead of `\n`.
    pub join: Option<String>,
    /// The whole row only exists on screen (a code block's header): selection
    /// copy leaves it out, newline included.
    pub omit: bool,
    /// A clickable `[copy]` button on this row.
    pub button: Option<CopyButton>,
}

/// A code block's `[copy]` button: where its label sits in the row, and what
/// it copies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyButton {
    /// Char index of the label's first character in the row text.
    pub start: usize,
    /// Label length in chars.
    pub len: usize,
    /// The block's source lines, exactly as the agent wrote them (tabs kept,
    /// the fence's own indent removed, no trailing newline).
    pub text: String,
}

/// The button label drawn on a code block's header row.
pub const COPY_BUTTON_LABEL: &str = "[copy]";

/// A single rendered line for the chat panel.
///
/// Holds one or more styled segments so a single visual line can mix inline
/// styles (bold, italic, inline code, links) on top of the line's base style.
#[derive(Clone)]
pub struct ChatLine {
    pub segments: Vec<StyledSegment>,
    pub copy: RowCopy,
}

impl ChatLine {
    /// Build a line with a single style covering the whole text.
    pub fn single(text: impl Into<String>, style: Style) -> Self {
        let text = text.into();
        if text.is_empty() {
            Self {
                segments: Vec::new(),
                copy: RowCopy::default(),
            }
        } else {
            Self {
                segments: vec![StyledSegment::new(text, style)],
                copy: RowCopy::default(),
            }
        }
    }

    fn with_copy(mut self, copy: RowCopy) -> Self {
        self.copy = copy;
        self
    }

    /// Plain-text concatenation of all segments (no style information).
    pub fn text(&self) -> String {
        self.segments.iter().map(|s| s.text.as_str()).collect()
    }

    /// Style of the first segment, used as a fallback for whole-line
    /// effects like the browse-mode background highlight.
    pub fn primary_style(&self) -> Style {
        self.segments.first().map(|s| s.style).unwrap_or_default()
    }
}

/// Format markdown content into styled lines for the chat panel.
///
/// Processes tables with box-drawing characters, renders headings with visual
/// markers, indents code blocks, formats lists, and converts inline
/// `**bold**`, `*italic*`, `` `code` ``, and `[text](url)` markers into styled
/// segments that render with the corresponding visual attributes.
pub fn format_chat_markdown(text: &str, width: usize, base_style: Style) -> Vec<ChatLine> {
    format_chat_markdown_mapped(text, width, base_style).0
}

/// [`format_chat_markdown`] plus a source-line → rendered-line index map.
///
/// `map[i]` is the first output line produced by source line `i`, so the
/// markdown preview can scroll to the rendered position of the editor's top
/// visible line. Source lines swallowed by a multi-line construct (table rows,
/// fenced code) inherit the index of the line that opened it, and the map is
/// non-decreasing, so it is safe to index with any source line and compare
/// results.
pub fn format_chat_markdown_mapped(
    text: &str,
    width: usize,
    base_style: Style,
) -> (Vec<ChatLine>, Vec<usize>) {
    format_markdown(text, width, base_style, false)
}

/// [`format_chat_markdown`] for the agent chat transcript: every closed fenced
/// code block also gets a header row with a [`CopyButton`] that copies the
/// block's source exactly — the reliable way to copy a shell command.
pub fn format_chat_markdown_with_copy_buttons(
    text: &str,
    width: usize,
    base_style: Style,
) -> Vec<ChatLine> {
    format_markdown(text, width, base_style, true).0
}

fn format_markdown(
    text: &str,
    width: usize,
    base_style: Style,
    copy_buttons: bool,
) -> (Vec<ChatLine>, Vec<usize>) {
    let mut output = Vec::new();
    // Expand tabs to 4 spaces to prevent terminal rendering artifacts.
    // Tab characters in ratatui cells cause cursor jumps that corrupt display.
    let expanded_text = text.replace('\t', "    ");
    let src_lines: Vec<&str> = expanded_text.lines().collect();
    // Same line split as `src_lines` (only tabs differ): what a copy button
    // copies comes from here, so tabs survive.
    let raw_lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    let mut in_code_block = false;

    let code_style = Style::default().fg(theme::CODE_GREEN);
    let heading_style = Style::default()
        .fg(theme::TEXT_BRIGHT)
        .add_modifier(Modifier::BOLD);
    let quote_style = Style::default().fg(theme::TOOL_DIM);
    let rule_style = Style::default().fg(theme::BORDER_DIM);
    let table_border_style = Style::default().fg(theme::BORDER_DIM);
    let table_header_style = Style::default()
        .fg(theme::TEXT_BRIGHT)
        .add_modifier(Modifier::BOLD);
    let reasoning_style = Style::default()
        .fg(theme::REASONING_FG)
        .add_modifier(Modifier::ITALIC);
    let mut in_thinking_block = false;
    // Filled at the top of every iteration; entries for source lines consumed
    // inside a multi-line construct stay `MAX` and are back-filled below.
    let mut src_map = vec![usize::MAX; src_lines.len()];

    while i < src_lines.len() {
        src_map[i] = output.len();
        let line = src_lines[i];
        let trimmed = line.trim();

        // Thinking / reasoning block tags: <think> ... </think>
        if trimmed == "<think>" || trimmed.starts_with("<think>") {
            in_thinking_block = true;
            // If there's content after <think> on the same line, render it
            let after_tag = trimmed.strip_prefix("<think>").unwrap_or("");
            let after_tag = after_tag.strip_suffix("</think>").unwrap_or(after_tag);
            if !after_tag.is_empty() {
                push_inline_wrapped(
                    &mut output,
                    after_tag.trim(),
                    "  ",
                    "  ",
                    width,
                    reasoning_style,
                );
            }
            if trimmed.ends_with("</think>") {
                in_thinking_block = false;
            }
            i += 1;
            continue;
        }
        if trimmed == "</think>" || trimmed.ends_with("</think>") {
            // Render any text before the closing tag
            let before_tag = trimmed.strip_suffix("</think>").unwrap_or("");
            if !before_tag.is_empty() {
                push_inline_wrapped(
                    &mut output,
                    before_tag.trim(),
                    "  ",
                    "  ",
                    width,
                    reasoning_style,
                );
            }
            in_thinking_block = false;
            i += 1;
            continue;
        }
        if in_thinking_block {
            if trimmed.is_empty() {
                output.push(ChatLine::single(String::new(), reasoning_style));
            } else {
                push_inline_wrapped(&mut output, trimmed, "  ", "  ", width, reasoning_style);
            }
            i += 1;
            continue;
        }

        // Code block fences
        if is_fence(trimmed) {
            if copy_buttons
                && !in_code_block
                && let Some(header) = code_block_header(&raw_lines, i, width)
            {
                output.push(header);
            }
            in_code_block = !in_code_block;
            i += 1;
            continue;
        }

        if in_code_block {
            let budget = width.saturating_sub(2).max(1);
            for row in crate::widgets::render_utils::wrap_rows(line, budget) {
                output.push(
                    ChatLine::single(format!("  {}", row.text), code_style).with_copy(RowCopy {
                        skip: 2,
                        join: row.join,
                        ..RowCopy::default()
                    }),
                );
            }
            i += 1;
            continue;
        }

        // Table detection: current line has |, next line is separator
        if is_table_row(trimmed)
            && i + 1 < src_lines.len()
            && is_table_separator(src_lines[i + 1].trim())
        {
            let mut rows: Vec<Vec<String>> = Vec::new();
            rows.push(parse_table_cells(trimmed));
            i += 2; // skip header + separator
            while i < src_lines.len() && is_table_row(src_lines[i].trim()) {
                rows.push(parse_table_cells(src_lines[i].trim()));
                i += 1;
            }
            render_table(
                &rows,
                width,
                base_style,
                table_border_style,
                table_header_style,
                &mut output,
            );
            continue;
        }

        // Empty line
        if trimmed.is_empty() {
            output.push(ChatLine::single(String::new(), base_style));
            i += 1;
            continue;
        }

        // Horizontal rule
        if (trimmed.starts_with("---") || trimmed.starts_with("***") || trimmed.starts_with("___"))
            && trimmed.chars().filter(|c| !c.is_whitespace()).count() >= 3
            && trimmed
                .chars()
                .all(|c| c == '-' || c == '*' || c == '_' || c == ' ')
        {
            let rule: String = "─".repeat(width.min(60));
            output.push(ChatLine::single(rule, rule_style));
            i += 1;
            continue;
        }

        // Headings
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|c| *c == '#').count().min(6);
            if trimmed
                .get(level..level + 1)
                .is_none_or(|c| c == " " || c.is_empty())
            {
                let marker = match level {
                    1 => "█ ",
                    2 => "▌ ",
                    _ => "▎ ",
                };
                push_inline_wrapped(
                    &mut output,
                    trimmed[level..].trim(),
                    marker,
                    "  ",
                    width,
                    heading_style,
                );
                i += 1;
                continue;
            }
        }

        // Block quotes
        if let Some(quoted) = trimmed.strip_prefix('>') {
            push_inline_wrapped(
                &mut output,
                quoted.trim_start(),
                "│ ",
                "│ ",
                width,
                quote_style,
            );
            i += 1;
            continue;
        }

        // Unordered list items
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
            let indent = (line.len() - line.trim_start().len()) / 2;
            let pad: String = "  ".repeat(indent);
            push_inline_wrapped(
                &mut output,
                &trimmed[2..],
                &format!("{}• ", pad),
                &format!("{}  ", pad),
                width,
                base_style,
            );
            i += 1;
            continue;
        }

        // Ordered list items
        if let Some(dot_pos) = trimmed.find(". ") {
            let prefix = &trimmed[..dot_pos];
            if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit()) {
                let marker = format!("{}. ", prefix);
                let cont_indent: String = " ".repeat(marker.chars().count());
                push_inline_wrapped(
                    &mut output,
                    &trimmed[dot_pos + 2..],
                    &marker,
                    &cont_indent,
                    width,
                    base_style,
                );
                i += 1;
                continue;
            }
        }

        // Tool call and file proposal markers: [Read ...], [Write ...], [wrote ...], etc.
        // Style with dim color to visually distinguish from regular text
        if trimmed.starts_with('[') && trimmed.ends_with(']') && !trimmed.contains("](") {
            let marker_style = Style::default().fg(theme::TOOL_DIM);
            for row in crate::widgets::render_utils::wrap_rows(&format!("  {}", trimmed), width) {
                output.push(ChatLine::single(row.text, marker_style).with_copy(RowCopy {
                    join: row.join,
                    ..RowCopy::default()
                }));
            }
            i += 1;
            continue;
        }

        // Regular text: preserve inline styling (bold/italic/code/links).
        push_inline_wrapped(&mut output, trimmed, "", "", width, base_style);
        i += 1;
    }

    let (lines, reflowed) = reflow_overwide_lines(output, width);
    // Give the swallowed source lines the index of the construct that opened
    // them, then translate every pre-reflow index into its final position.
    let mut last = 0;
    for entry in src_map.iter_mut() {
        if *entry == usize::MAX {
            *entry = last;
        } else {
            last = *entry;
        }
        *entry = reflowed.get(*entry).copied().unwrap_or(lines.len());
    }
    (lines, src_map)
}

/// Re-wrap any rendered line that still exceeds `width` display columns.
///
/// Catches formatted output that can exceed the panel budget after layout; uses
/// the line's primary style for the wrapped parts. Table rows are left intact —
/// they use multiline cells instead of post-hoc line breaking.
///
/// Also returns an old-index → new-index map (with a trailing entry for the
/// one-past-the-end index) so a caller holding pre-reflow line indices — the
/// source-line map — can translate them.
fn reflow_overwide_lines(lines: Vec<ChatLine>, width: usize) -> (Vec<ChatLine>, Vec<usize>) {
    if width == 0 {
        let identity = (0..=lines.len()).collect();
        return (lines, identity);
    }
    let mut out = Vec::with_capacity(lines.len());
    let mut index_map = Vec::with_capacity(lines.len() + 1);
    for line in lines {
        index_map.push(out.len());
        let text = line.text();
        if UnicodeWidthStr::width(text.as_str()) <= width || is_table_rendered_line(&text) {
            out.push(line);
            continue;
        }
        let style = line.primary_style();
        for (j, (wrapped, sep)) in word_wrap_segments(&line.segments, width, width)
            .into_iter()
            .enumerate()
        {
            // The first piece keeps the row's own copy rule; the rest wrap it.
            let copy = if j == 0 {
                line.copy.clone()
            } else {
                RowCopy {
                    join: Some(sep),
                    ..RowCopy::default()
                }
            };
            if wrapped.is_empty() {
                out.push(ChatLine::single(String::new(), style).with_copy(copy));
            } else {
                out.push(ChatLine {
                    segments: wrapped,
                    copy,
                });
            }
        }
    }
    index_map.push(out.len());
    (out, index_map)
}

/// A fenced-code-block fence (opening or closing), given the trimmed line.
fn is_fence(trimmed: &str) -> bool {
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

/// The header row of the fenced code block opening at `lines[open]`: the
/// fence's language and a `[copy]` button carrying the block's source.
///
/// `None` while the block has no closing fence — a reply still streaming,
/// where copying half a command could run something else — for a block with
/// no content, and when the panel is too narrow for the button.
fn code_block_header(lines: &[&str], open: usize, width: usize) -> Option<ChatLine> {
    let close = (open + 1..lines.len()).find(|&j| is_fence(lines[j].trim()))?;
    let fence = lines[open];
    // A fence nested in a list is indented; its content carries the same
    // indent, which is layout, not part of the code.
    let indent = fence.chars().take_while(|c| *c == ' ').count();
    let body: Vec<&str> = lines[open + 1..close]
        .iter()
        .map(|line| strip_leading_spaces(line, indent))
        .collect();
    let first = body.iter().position(|l| !l.trim().is_empty())?;
    let last = body.iter().rposition(|l| !l.trim().is_empty())?;
    let text = body[first..=last].join("\n").trim_end().to_string();

    let label_w = COPY_BUTTON_LABEL.chars().count();
    if width < label_w + 1 {
        return None;
    }
    let lang: String = fence
        .trim()
        .trim_start_matches(['`', '~'])
        .split_whitespace()
        .next()
        .unwrap_or("")
        .chars()
        .take(24)
        .collect();
    // `  lang ───── [copy]`, dropping the language, then the indent, when
    // the panel is too narrow for them.
    let candidates = [
        if lang.is_empty() { None } else { Some(format!("  {lang} ")) },
        Some("  ".to_string()),
        Some(String::new()),
    ];
    let left = candidates
        .into_iter()
        .flatten()
        .find(|left| UnicodeWidthStr::width(left.as_str()) + 1 + label_w <= width)?;
    let fill = width - UnicodeWidthStr::width(left.as_str()) - 1 - label_w;

    let mut segments = Vec::with_capacity(4);
    if !left.is_empty() {
        segments.push(StyledSegment::new(
            left.clone(),
            Style::default().fg(theme::TOOL_DIM),
        ));
    }
    segments.push(StyledSegment::new(
        format!("{} ", "─".repeat(fill)),
        Style::default().fg(theme::BORDER_DIM),
    ));
    segments.push(StyledSegment::new(
        COPY_BUTTON_LABEL,
        Style::default()
            .fg(theme::ACCENT)
            .add_modifier(Modifier::BOLD),
    ));
    Some(ChatLine {
        segments,
        copy: RowCopy {
            omit: true,
            button: Some(CopyButton {
                start: left.chars().count() + fill + 1,
                len: label_w,
                text,
            }),
            ..RowCopy::default()
        },
    })
}

/// `line` without up to `n` leading spaces.
fn strip_leading_spaces(line: &str, n: usize) -> &str {
    let mut rest = line;
    for _ in 0..n {
        match rest.strip_prefix(' ') {
            Some(r) => rest = r,
            None => break,
        }
    }
    rest
}

/// True for box-drawn table borders and data rows (already width-constrained).
fn is_table_rendered_line(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    let first = t.chars().next().unwrap_or(' ');
    matches!(first, '│' | '┌' | '├' | '└')
}

/// Word-wrap a markdown text fragment with inline styling preserved, prepending
/// `first_prefix` to the first wrapped line and `cont_prefix` to subsequent
/// lines. Prefixes inherit `base_style`.
fn push_inline_wrapped(
    output: &mut Vec<ChatLine>,
    text: &str,
    first_prefix: &str,
    cont_prefix: &str,
    width: usize,
    base_style: Style,
) {
    let inline = parse_inline_styled(text, base_style);

    // Width budget shrinks by the prefix display width on each wrapped line.
    let first_prefix_w = UnicodeWidthStr::width(first_prefix);
    let cont_prefix_w = UnicodeWidthStr::width(cont_prefix);
    let first_budget = width.saturating_sub(first_prefix_w).max(1);
    let cont_budget = width.saturating_sub(cont_prefix_w).max(1);

    let wrapped = word_wrap_segments(&inline, first_budget, cont_budget);

    if wrapped.is_empty() {
        if !first_prefix.is_empty() {
            output.push(ChatLine::single(first_prefix.to_string(), base_style));
        } else {
            output.push(ChatLine::single(String::new(), base_style));
        }
        return;
    }

    for (j, (wrapped_segments, sep)) in wrapped.into_iter().enumerate() {
        let prefix = if j == 0 { first_prefix } else { cont_prefix };
        // A continuation row's prefix is only the hanging indent: copy drops
        // it and rejoins the row to the one above.
        let copy = if j == 0 {
            RowCopy::default()
        } else {
            RowCopy {
                skip: cont_prefix.chars().count(),
                join: Some(sep),
                ..RowCopy::default()
            }
        };
        let mut segments: Vec<StyledSegment> = Vec::with_capacity(wrapped_segments.len() + 1);
        if !prefix.is_empty() {
            segments.push(StyledSegment::new(prefix.to_string(), base_style));
        }
        segments.extend(wrapped_segments);
        if segments.is_empty() {
            output.push(ChatLine::single(String::new(), base_style).with_copy(copy));
        } else {
            output.push(ChatLine { segments, copy });
        }
    }
}

/// Convert markdown inline segments into styled segments, mapping each marker
/// kind to a concrete `Style` on top of `base_style`.
fn parse_inline_styled(text: &str, base_style: Style) -> Vec<StyledSegment> {
    // Colors mirror the source-editor markdown highlights in `theme.rs`
    // (`markup.bold`, `markup.italic`, `markup.code`, `markup.link`) so the
    // preview reads the same as the source view. The BOLD/ITALIC text
    // modifiers alone render too subtly on many terminals — the visible
    // distinction comes from the colour change.
    const BOLD_FG: ratatui::style::Color = ratatui::style::Color::Rgb(229, 192, 123);
    const ITALIC_FG: ratatui::style::Color = ratatui::style::Color::Rgb(198, 120, 221);

    parse_inline(text)
        .into_iter()
        .filter_map(|seg| {
            if seg.text.is_empty() {
                return None;
            }
            let (style, href) = match seg.kind {
                SegmentKind::Plain => (base_style, None),
                SegmentKind::Bold => (base_style.fg(BOLD_FG).add_modifier(Modifier::BOLD), None),
                SegmentKind::Italic => (
                    base_style.fg(ITALIC_FG).add_modifier(Modifier::ITALIC),
                    None,
                ),
                SegmentKind::Code => (Style::default().fg(theme::CODE_GREEN), None),
                SegmentKind::Link(url) => (
                    base_style
                        .fg(theme::ACCENT)
                        .add_modifier(Modifier::UNDERLINED),
                    Some(url),
                ),
            };
            Some(StyledSegment {
                text: seg.text,
                style,
                href,
            })
        })
        .collect()
}

/// Word-wrap styled segments into multiple lines, preserving styles.
///
/// Each output line is a sequence of segments (style runs) covering exactly the
/// characters that fit within the line budget. `first_width` is the budget for
/// the first wrapped line and `cont_width` for subsequent continuation lines,
/// so the caller can reserve room for prefix indents.
///
/// Each line comes with the whitespace the wrap dropped just before it (empty
/// for the first line and for a break inside a word), so copy can rejoin it.
fn word_wrap_segments(
    segments: &[StyledSegment],
    first_width: usize,
    cont_width: usize,
) -> Vec<(Vec<StyledSegment>, String)> {
    // Flatten to (char, style, href) triples, expanding tabs to 4 spaces to match
    // `render_utils::word_wrap`'s behaviour. href rides along so two adjacent
    // links that share a style (both accent+underline) do not merge.
    let mut chars: Vec<(char, Style, Option<String>)> = Vec::new();
    for seg in segments {
        for ch in seg.text.chars() {
            if ch == '\t' {
                for _ in 0..4 {
                    chars.push((' ', seg.style, seg.href.clone()));
                }
            } else {
                chars.push((ch, seg.style, seg.href.clone()));
            }
        }
    }

    if chars.is_empty() {
        return Vec::new();
    }

    let mut result: Vec<(Vec<StyledSegment>, String)> = Vec::new();
    let mut start = 0;
    let mut sep = String::new();
    while start < chars.len() {
        let budget = if result.is_empty() {
            first_width
        } else {
            cont_width
        }
        .max(1);

        let mut display_w = 0usize;
        let mut end = start;
        while end < chars.len() {
            let cw = UnicodeWidthChar::width(chars[end].0).unwrap_or(1);
            if display_w + cw > budget {
                break;
            }
            display_w += cw;
            end += 1;
        }
        if end == start {
            end = start + 1;
        }

        let break_at = if end < chars.len() {
            let mut bp = end;
            while bp > start && chars[bp].0 != ' ' {
                bp -= 1;
            }
            if bp == start { end } else { bp + 1 }
        } else {
            end
        };

        // Trim trailing spaces on non-final wrapped lines for visual cleanliness.
        let mut line_end = break_at;
        if break_at < chars.len() {
            while line_end > start && chars[line_end - 1].0 == ' ' {
                line_end -= 1;
            }
        }

        result.push((
            coalesce_segments(&chars[start..line_end]),
            std::mem::take(&mut sep),
        ));
        sep = chars[line_end..break_at].iter().map(|(c, _, _)| *c).collect();
        start = break_at;
    }

    result
}

/// Group consecutive characters with the same style and href into segments.
fn coalesce_segments(chars: &[(char, Style, Option<String>)]) -> Vec<StyledSegment> {
    let mut segments: Vec<StyledSegment> = Vec::new();
    let mut iter = chars.iter();
    if let Some((ch, style, href)) = iter.next() {
        let mut current_text = String::new();
        let mut current_style = *style;
        let mut current_href = href.clone();
        current_text.push(*ch);
        for (ch, style, href) in iter {
            if *style == current_style && *href == current_href {
                current_text.push(*ch);
            } else {
                segments.push(StyledSegment {
                    text: std::mem::take(&mut current_text),
                    style: current_style,
                    href: current_href,
                });
                current_text.push(*ch);
                current_style = *style;
                current_href = href.clone();
            }
        }
        if !current_text.is_empty() {
            segments.push(StyledSegment {
                text: current_text,
                style: current_style,
                href: current_href,
            });
        }
    }
    segments
}

/// Paint pre-formatted markdown lines into a ratatui buffer (editor preview pane).
///
/// When `hovered_href` matches a segment's destination, that run (and any
/// wrapped continuation of the same link) is painted with [`link_hover_style`].
pub fn render_lines_to_buffer(
    lines: &[ChatLine],
    area: Rect,
    buf: &mut Buffer,
    scroll_top: usize,
    clear_style: Style,
    hovered_href: Option<&str>,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    for row in 0..area.height as usize {
        let line_idx = scroll_top + row;
        let y = area.y + row as u16;

        for col in 0..area.width {
            let cx = area.x + col;
            if cx < buf.area().right() && y < buf.area().bottom() {
                buf[(cx, y)].set_char(' ').set_style(clear_style);
            }
        }

        if line_idx >= lines.len() {
            continue;
        }

        let line = &lines[line_idx];
        let x_start = area.x.saturating_add(1);
        let mut cx = x_start;
        for seg in &line.segments {
            cx = crate::widgets::render_utils::write_text(
                buf,
                cx,
                y,
                area.right(),
                &seg.text,
                segment_style(seg, hovered_href),
            );
            if cx >= area.right() {
                break;
            }
        }
    }
}

/// Highlight used while the pointer is over a followable preview link.
/// Background colour is the cue — `BOLD` alone is too subtle on many terminals.
pub fn link_hover_style() -> Style {
    Style::default()
        .fg(theme::PANEL_BG)
        .bg(theme::ACCENT)
        .add_modifier(Modifier::UNDERLINED | Modifier::BOLD)
}

fn segment_style(seg: &StyledSegment, hovered_href: Option<&str>) -> Style {
    if hovered_href.is_some() && seg.href.as_deref() == hovered_href {
        link_hover_style()
    } else {
        seg.style
    }
}

/// Display-column hit test: `visual_col` is 0-based from the start of the
/// rendered line (prefixes like list bullets included).
pub fn href_at(line: &ChatLine, visual_col: usize) -> Option<&str> {
    let mut col = 0usize;
    for seg in &line.segments {
        let w = UnicodeWidthStr::width(seg.text.as_str());
        if visual_col < col + w {
            return seg.href.as_deref();
        }
        col += w;
    }
    None
}

/// Map a click in the preview inner rect onto a link destination.
///
/// `text_origin_x` is the column where line text starts (`inner.x + 1`, matching
/// [`render_lines_to_buffer`]).
pub fn preview_link_at(
    lines: &[ChatLine],
    scroll: usize,
    text_origin_x: u16,
    inner: Rect,
    col: u16,
    row: u16,
) -> Option<&str> {
    if !inner.contains((col, row).into()) || col < text_origin_x {
        return None;
    }
    let line_idx = scroll + (row - inner.y) as usize;
    let line = lines.get(line_idx)?;
    href_at(line, (col - text_origin_x) as usize)
}

/// A case-insensitive find match in rendered preview text: `start..end` are
/// char offsets into [`ChatLine::text`] of `lines[line]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreviewMatch {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// Lowercase one char to one char, so offsets in the folded text are the
/// offsets in the original.
fn fold_case(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Every case-insensitive occurrence of `query` in the rendered lines, top to
/// bottom. Overlapping occurrences are all reported, like the editor's find.
pub fn preview_find_matches(lines: &[ChatLine], query: &str) -> Vec<PreviewMatch> {
    let needle: Vec<char> = query.chars().map(fold_case).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (line, l) in lines.iter().enumerate() {
        let hay: Vec<char> = l.text().chars().map(fold_case).collect();
        if hay.len() < needle.len() {
            continue;
        }
        for start in 0..=hay.len() - needle.len() {
            if hay[start..start + needle.len()] == needle[..] {
                out.push(PreviewMatch {
                    line,
                    start,
                    end: start + needle.len(),
                });
            }
        }
    }
    out
}

/// Repaint the find matches visible in `area` (same origin as
/// [`render_lines_to_buffer`]); the match starting at `current` stands out.
pub fn paint_preview_matches(
    lines: &[ChatLine],
    matches: &[PreviewMatch],
    current: Option<(usize, usize)>,
    area: Rect,
    buf: &mut Buffer,
    scroll_top: usize,
) {
    let x_start = area.x.saturating_add(1);
    for m in matches {
        if m.line < scroll_top || m.line >= scroll_top + area.height as usize {
            continue;
        }
        let Some(line) = lines.get(m.line) else {
            continue;
        };
        let y = area.y + (m.line - scroll_top) as u16;
        let col_of = |chars: usize| -> usize {
            line.text()
                .chars()
                .take(chars)
                .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
                .sum()
        };
        let style = if current == Some((m.line, m.start)) {
            Style::default()
                .fg(theme::PANEL_BG)
                .bg(theme::ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().bg(theme::SEARCH_HIGHLIGHT_BG)
        };
        for col in col_of(m.start)..col_of(m.end) {
            let x = x_start as usize + col;
            if x >= area.right() as usize || y >= buf.area().bottom() {
                break;
            }
            let cell = &mut buf[(x as u16, y)];
            cell.set_style(cell.style().patch(style));
        }
    }
}

/// A mouse selection in the rendered markdown preview. Positions are
/// `(rendered line, char offset)` into [`ChatLine::text`]; the later end is
/// exclusive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewSelection {
    pub anchor: (usize, usize),
    pub end: (usize, usize),
    /// The button is still down.
    pub dragging: bool,
    /// The pointer left the cell it was pressed on: the release ends a
    /// selection instead of clicking (link / heading fold).
    pub moved: bool,
    /// Screen cell the button went down on.
    pub press: (u16, u16),
    /// `(buffer, text width, rendered line count)` of the layout the positions
    /// index into; a preview laid out differently drops the selection.
    pub layout: (usize, usize, usize),
}

impl PreviewSelection {
    /// `(start, end)` in document order.
    pub fn range(&self) -> ((usize, usize), (usize, usize)) {
        if self.anchor <= self.end {
            (self.anchor, self.end)
        } else {
            (self.end, self.anchor)
        }
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.end
    }
}

/// `(line, char)` under display column `visual_col` of rendered line `line`
/// (0-based from where [`render_lines_to_buffer`] starts the text). Past the
/// end of a line is its end; past the last line is the end of the last line.
pub fn preview_text_pos(lines: &[ChatLine], line: usize, visual_col: usize) -> (usize, usize) {
    let Some(last) = lines.len().checked_sub(1) else {
        return (0, 0);
    };
    if line > last {
        return (last, lines[last].text().chars().count());
    }
    let mut col = 0usize;
    for (idx, ch) in lines[line].text().chars().enumerate() {
        // Same width rule as `render_utils::write_text`, which drew the row.
        let w = UnicodeWidthChar::width(ch).unwrap_or(1);
        if col + w > visual_col {
            return (line, idx);
        }
        col += w;
    }
    (line, lines[line].text().chars().count())
}

/// The text a selection over rendered rows copies: from `start` to `end`
/// (`(row, char)`, end exclusive), rows fetched through `row`.
///
/// Gives back the source text, not the layout: a soft-wrapped row is rejoined
/// to the row above with the whitespace the wrap dropped, screen-only indents
/// (code block, hanging list indent) are left out, and so are screen-only
/// rows. Only real line breaks become newlines. Stops at the first row `row`
/// does not have.
pub fn selection_copy_text<'a>(
    row: impl Fn(usize) -> Option<(String, Option<&'a RowCopy>)>,
    start: (usize, usize),
    end: (usize, usize),
) -> Option<String> {
    let ((sl, sc), (el, ec)) = (start, end);
    if start == end {
        return None;
    }
    let mut result = String::new();
    let mut started = false;
    for line_idx in sl..=el {
        let Some((line, copy)) = row(line_idx) else {
            break;
        };
        if copy.is_some_and(|c| c.omit) {
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let skip = copy.map_or(0, |c| c.skip).min(chars.len());
        let start_c = if line_idx == sl {
            sc.min(chars.len())
        } else {
            0
        }
        .max(skip);
        let end_c = if line_idx == el {
            ec.min(chars.len())
        } else {
            chars.len()
        };

        if started {
            match copy.and_then(|c| c.join.as_deref()) {
                Some(sep) => result.push_str(sep),
                None => result.push('\n'),
            }
        }
        started = true;

        if start_c < end_c {
            result.extend(&chars[start_c..end_c]);
        }
    }
    if result.is_empty() {
        None
    } else {
        Some(result)
    }
}

/// What a preview selection copies (see [`selection_copy_text`]).
pub fn preview_selection_text(lines: &[ChatLine], selection: &PreviewSelection) -> Option<String> {
    let (start, end) = selection.range();
    selection_copy_text(
        |i| lines.get(i).map(|l| (l.text(), Some(&l.copy))),
        start,
        end,
    )
}

/// Paint the selected part of the preview rows visible in `area` (same origin
/// as [`render_lines_to_buffer`]).
pub fn paint_preview_selection(
    lines: &[ChatLine],
    selection: &PreviewSelection,
    area: Rect,
    buf: &mut Buffer,
    scroll_top: usize,
) {
    if selection.is_empty() {
        return;
    }
    let ((sl, sc), (el, ec)) = selection.range();
    let x_start = area.x.saturating_add(1) as usize;
    let first = sl.max(scroll_top);
    let last = el.min((scroll_top + area.height as usize).saturating_sub(1));
    for line_idx in first..=last {
        let Some(line) = lines.get(line_idx) else {
            break;
        };
        let y = area.y + (line_idx - scroll_top) as u16;
        if y >= buf.area().bottom() {
            break;
        }
        let from = if line_idx == sl { sc } else { 0 };
        let to = if line_idx == el { ec } else { usize::MAX };
        let mut x = x_start;
        for (idx, ch) in line.text().chars().enumerate() {
            let w = UnicodeWidthChar::width(ch).unwrap_or(1);
            if idx >= to || x + w > area.right() as usize {
                break;
            }
            if idx >= from {
                for cx in x..x + w {
                    let cell = &mut buf[(cx as u16, y)];
                    cell.set_style(cell.style().bg(theme::SELECTION_BG));
                }
            }
            x += w;
        }
    }
}

/// First rendered line whose heading slug matches `fragment`, if any.
pub fn preview_line_for_fragment(lines: &[ChatLine], fragment: &str) -> Option<usize> {
    use crate::editor::markdown::{heading_slug, strip_heading_marker};
    let want = heading_slug(fragment);
    if want.is_empty() {
        return None;
    }
    lines
        .iter()
        .position(|line| heading_slug(strip_heading_marker(&line.text())) == want)
}

// ── Table formatting ─────────────────────────────────────────

fn is_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.len() > 2
}

fn is_table_separator(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|')
        && trimmed
            .chars()
            .all(|c| c == '|' || c == '-' || c == ':' || c == ' ')
        && trimmed.contains('-')
}

fn parse_table_cells(line: &str) -> Vec<String> {
    let trimmed = line.trim().trim_matches('|');
    trimmed
        .split('|')
        .map(|cell| strip_inline_markers(cell.trim()))
        .collect()
}

fn render_table(
    rows: &[Vec<String>],
    max_width: usize,
    base_style: Style,
    border_style: Style,
    header_style: Style,
    output: &mut Vec<ChatLine>,
) {
    if rows.is_empty() {
        return;
    }

    let num_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if num_cols == 0 {
        return;
    }

    // Compute column widths
    let mut col_widths: Vec<usize> = vec![0; num_cols];
    for row in rows {
        for (j, cell) in row.iter().enumerate() {
            if j < num_cols {
                col_widths[j] = col_widths[j].max(UnicodeWidthStr::width(cell.as_str()));
            }
        }
    }

    // Cap widths to fit within max_width
    // Total: borders (num_cols + 1) + padding (num_cols * 2) + content
    let overhead = num_cols + 1 + num_cols * 2;
    let content_budget = max_width.saturating_sub(overhead);
    let col_widths = fit_column_widths(&col_widths, content_budget);

    // Build box-drawing lines
    let top = build_table_border(&col_widths, '┌', '┬', '┐', '─');
    let mid = build_table_border(&col_widths, '├', '┼', '┤', '─');
    let bot = build_table_border(&col_widths, '└', '┴', '┘', '─');

    output.push(ChatLine::single(top, border_style));

    for (row_idx, row) in rows.iter().enumerate() {
        let style = if row_idx == 0 {
            header_style
        } else {
            base_style
        };

        let wrapped_cells: Vec<Vec<String>> = col_widths
            .iter()
            .enumerate()
            .map(|(j, w)| {
                let cell = row.get(j).map(|s| s.as_str()).unwrap_or("");
                wrap_cell_content(cell, *w)
            })
            .collect();
        let row_height = wrapped_cells.iter().map(|c| c.len()).max().unwrap_or(1);

        for line_idx in 0..row_height {
            let line = build_table_row_line(&wrapped_cells, &col_widths, line_idx);
            output.push(ChatLine::single(line, style));
        }

        if row_idx == 0 && rows.len() > 1 {
            output.push(ChatLine::single(mid.clone(), border_style));
        }
    }

    output.push(ChatLine::single(bot, border_style));
}

/// Fit columns whose natural (unwrapped) widths are `natural` into `budget`
/// display columns of cell content.
///
/// Shrinking every column by the same proportion starves the narrow ones: a
/// 4-wide `#` column beside a 700-wide prose column collapses to a single
/// character and wraps `D-01` one letter per line, while the prose column still
/// keeps hundreds of columns it does not need. Instead this water-fills —
/// walking columns narrowest first, every column that already fits under the
/// running fair share keeps its natural width, and only the columns wider than
/// their share split whatever is left over.
fn fit_column_widths(natural: &[usize], budget: usize) -> Vec<usize> {
    let num_cols = natural.len();
    if num_cols == 0 {
        return Vec::new();
    }
    // Too narrow to give every column even one cell (the table will overflow
    // the panel either way) — fall back to minimal columns.
    if budget < num_cols {
        return vec![1; num_cols];
    }
    if natural.iter().sum::<usize>() <= budget {
        return natural.to_vec();
    }

    let mut order: Vec<usize> = (0..num_cols).collect();
    order.sort_by_key(|&j| natural[j]);

    let mut widths = vec![0usize; num_cols];
    let mut remaining = budget;
    for (k, &j) in order.iter().enumerate() {
        let cols_left = num_cols - k;
        let share = remaining / cols_left;
        if natural[j] <= share {
            widths[j] = natural[j];
            remaining -= natural[j];
            continue;
        }
        // Every column from here on is wider than its share: split the rest
        // evenly and hand the division remainder to the widest ones.
        let mut extra = remaining % cols_left;
        for &wide in &order[k..] {
            widths[wide] = share;
        }
        for &wide in order[k..].iter().rev() {
            if extra == 0 {
                break;
            }
            widths[wide] += 1;
            extra -= 1;
        }
        break;
    }
    widths
}

/// Word-wrap cell text to `col_width` display columns; empty cells yield one blank line.
fn wrap_cell_content(cell: &str, col_width: usize) -> Vec<String> {
    let budget = col_width.max(1);
    if cell.is_empty() {
        return vec![String::new()];
    }
    crate::widgets::render_utils::word_wrap(cell, budget)
}

/// Pad a single cell line to exactly `col_width` display columns.
fn pad_cell_line(line: &str, col_width: usize) -> String {
    let display_w = UnicodeWidthStr::width(line);
    if display_w >= col_width {
        return line.to_string();
    }
    let mut padded = line.to_string();
    for _ in 0..(col_width - display_w) {
        padded.push(' ');
    }
    padded
}

/// Build one visual row of a table at `line_idx` across wrapped cell lines.
fn build_table_row_line(
    wrapped_cells: &[Vec<String>],
    col_widths: &[usize],
    line_idx: usize,
) -> String {
    let mut line = String::new();
    for (j, w) in col_widths.iter().enumerate() {
        line.push('│');
        line.push(' ');
        let cell_line = wrapped_cells
            .get(j)
            .and_then(|lines| lines.get(line_idx))
            .map(|s| s.as_str())
            .unwrap_or("");
        line.push_str(&pad_cell_line(cell_line, *w));
        line.push(' ');
    }
    line.push('│');
    line
}

fn build_table_border(
    col_widths: &[usize],
    left: char,
    mid: char,
    right: char,
    fill: char,
) -> String {
    let mut s = String::new();
    for (j, w) in col_widths.iter().enumerate() {
        s.push(if j == 0 { left } else { mid });
        // +2 for padding on each side of cell content
        for _ in 0..(*w + 2) {
            s.push(fill);
        }
    }
    s.push(right);
    s
}

// ── Inline marker stripping ─────────────────────────────────

/// Strip markdown inline formatting markers, returning plain text.
fn strip_inline_markers(text: &str) -> String {
    let segments = parse_inline(text);
    segments.into_iter().map(|s| s.text).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection(anchor: (usize, usize), end: (usize, usize)) -> PreviewSelection {
        PreviewSelection {
            anchor,
            end,
            dragging: false,
            moved: true,
            press: (0, 0),
            layout: (0, 0, 0),
        }
    }

    #[test]
    fn preview_text_pos_maps_columns_to_chars_and_clamps() {
        let lines = vec![
            ChatLine::single("a界b", Style::default()),
            ChatLine::single("xy", Style::default()),
        ];
        assert_eq!(preview_text_pos(&lines, 0, 0), (0, 0));
        // `界` is two columns wide: both of them are char 1.
        assert_eq!(preview_text_pos(&lines, 0, 1), (0, 1));
        assert_eq!(preview_text_pos(&lines, 0, 2), (0, 1));
        assert_eq!(preview_text_pos(&lines, 0, 3), (0, 2));
        assert_eq!(preview_text_pos(&lines, 0, 99), (0, 3), "past the end is the end");
        assert_eq!(preview_text_pos(&lines, 7, 0), (1, 2), "past the last line");
        assert_eq!(preview_text_pos(&[], 0, 0), (0, 0));
    }

    #[test]
    fn preview_selection_copies_source_text_in_either_direction() {
        let text = "Some **bold** words\n\n```\n  indented code\n```";
        let lines = format_chat_markdown(text, 40, Style::default());
        let texts: Vec<String> = lines.iter().map(ChatLine::text).collect();
        let word = texts.iter().position(|t| t.contains("bold")).unwrap();
        let code = texts.iter().position(|t| t.contains("indented")).unwrap();

        // Markdown markers are not part of the rendered text.
        let start = texts[word].find("bold").unwrap();
        let sel = selection((word, start), (word, start + 4));
        assert_eq!(preview_selection_text(&lines, &sel).as_deref(), Some("bold"));
        // Dragged backwards: same text.
        let back = selection((word, start + 4), (word, start));
        assert_eq!(preview_selection_text(&lines, &back).as_deref(), Some("bold"));

        // The code block's screen indent is left out; its own indent stays.
        let end = texts[code].chars().count();
        let sel = selection((word, 0), (code, end));
        assert_eq!(
            preview_selection_text(&lines, &sel).as_deref(),
            Some("Some bold words\n\n  indented code")
        );
        assert_eq!(preview_selection_text(&lines, &selection((0, 2), (0, 2))), None);
    }

    #[test]
    fn preview_selection_rejoins_a_soft_wrapped_paragraph() {
        let text = "one two three four five six seven eight";
        let lines = format_chat_markdown(text, 12, Style::default());
        assert!(lines.len() > 1, "the paragraph must wrap");
        let last = lines.len() - 1;
        let end = lines[last].text().chars().count();
        let sel = selection((0, 0), (last, end));
        assert_eq!(preview_selection_text(&lines, &sel).as_deref(), Some(text));
    }

    #[test]
    fn preview_selection_paints_only_the_selected_cells() {
        let lines = vec![
            ChatLine::single("hello", Style::default()),
            ChatLine::single("world", Style::default()),
        ];
        let area = Rect::new(0, 0, 10, 2);
        let mut buf = Buffer::empty(area);
        render_lines_to_buffer(&lines, area, &mut buf, 0, Style::default(), None);
        paint_preview_selection(&lines, &selection((0, 3), (1, 2)), area, &mut buf, 0);
        let selected = |x: u16, y: u16| buf[(x, y)].style().bg == Some(theme::SELECTION_BG);
        // Text starts one column in.
        let row0: Vec<bool> = (0..7).map(|x| selected(x, 0)).collect();
        assert_eq!(row0, [false, false, false, false, true, true, false]);
        let row1: Vec<bool> = (0..7).map(|x| selected(x, 1)).collect();
        assert_eq!(row1, [false, true, true, false, false, false, false]);
    }

    #[test]
    fn copy_button_carries_the_exact_block_source() {
        let text = "Run:\n\n```powershell\n\tcargo build\ncargo test -p gaviero-core\n\n```\nafter";
        let lines = format_chat_markdown_with_copy_buttons(text, 40, Style::default());
        let headers: Vec<&ChatLine> = lines.iter().filter(|l| l.copy.button.is_some()).collect();
        assert_eq!(headers.len(), 1);
        let header = headers[0];
        let button = header.copy.button.as_ref().unwrap();
        // Tabs kept, trailing blank line dropped, no trailing newline.
        assert_eq!(button.text, "\tcargo build\ncargo test -p gaviero-core");
        assert!(header.copy.omit, "the header is never part of a selection copy");
        let row: Vec<char> = header.text().chars().collect();
        let label: String = row[button.start..button.start + button.len].iter().collect();
        assert_eq!(label, COPY_BUTTON_LABEL);
        assert!(header.text().starts_with("  powershell "), "{}", header.text());
        assert_eq!(UnicodeWidthStr::width(header.text().as_str()), 40);

        // The plain formatter (editor preview, code graph) draws no header.
        let plain = format_chat_markdown(text, 40, Style::default());
        assert_eq!(plain.len() + 1, lines.len());
        assert!(plain.iter().all(|l| l.copy.button.is_none()));
    }

    #[test]
    fn copy_button_waits_for_the_closing_fence_and_skips_empty_blocks() {
        // Still streaming: copying half a command is how `rm -rf /tmp/build`
        // becomes `rm -rf /`.
        let streaming =
            format_chat_markdown_with_copy_buttons("```sh\nrm -rf /tmp/bu", 40, Style::default());
        assert!(streaming.iter().all(|l| l.copy.button.is_none()));
        let empty = format_chat_markdown_with_copy_buttons("```\n\n```", 40, Style::default());
        assert!(empty.iter().all(|l| l.copy.button.is_none()));
    }

    #[test]
    fn copy_button_drops_the_indent_of_a_fence_nested_in_a_list() {
        let text = "1. Build:\n   ```bash\n   cargo build --release\n     nested\n   ```";
        let lines = format_chat_markdown_with_copy_buttons(text, 40, Style::default());
        let button = lines.iter().find_map(|l| l.copy.button.as_ref()).unwrap();
        assert_eq!(button.text, "cargo build --release\n  nested");
    }

    #[test]
    fn copy_button_fits_a_narrow_panel() {
        let text = "```a-very-long-language-name\nls\n```";
        for width in 7..=20 {
            let lines = format_chat_markdown_with_copy_buttons(text, width, Style::default());
            let header = lines.iter().find(|l| l.copy.button.is_some()).unwrap();
            assert!(UnicodeWidthStr::width(header.text().as_str()) <= width);
            assert!(header.text().ends_with(COPY_BUTTON_LABEL));
        }
        let lines = format_chat_markdown_with_copy_buttons(text, 6, Style::default());
        assert!(lines.iter().all(|l| l.copy.button.is_none()));
    }

    #[test]
    fn test_strip_inline_markers() {
        assert_eq!(strip_inline_markers("**bold** text"), "bold text");
        assert_eq!(strip_inline_markers("*italic* text"), "italic text");
        assert_eq!(strip_inline_markers("`code` text"), "code text");
    }

    #[test]
    fn test_table_detection() {
        assert!(is_table_row("| a | b |"));
        assert!(!is_table_row("not a table"));
        assert!(is_table_separator("|---|---|"));
        assert!(is_table_separator("| --- | :---: |"));
    }

    #[test]
    fn source_map_points_at_the_rendered_position_of_each_source_line() {
        // Constructs that break naive 1:1 line mapping: a fence (dropped),
        // code lines (kept), a table (many source lines → one iteration), and
        // a paragraph long enough to wrap into several rendered lines.
        let text = "# Title\n\
                    \n\
                    ```rust\n\
                    let x = 1;\n\
                    ```\n\
                    | a | b |\n\
                    |---|---|\n\
                    | 1 | 2 |\n\
                    tail paragraph that is definitely wider than the panel budget\n\
                    last";
        let width = 24;
        let (lines, map) = format_chat_markdown_mapped(text, width, Style::default());

        assert_eq!(map.len(), text.lines().count(), "one entry per source line");
        assert!(
            map.windows(2).all(|w| w[0] <= w[1]),
            "map must be non-decreasing: {map:?}"
        );
        assert!(
            map.iter().all(|&idx| idx <= lines.len()),
            "map must stay in range: {map:?}"
        );

        // Anchors: heading, the code line inside the fence, and the last line.
        assert!(lines[map[0]].text().contains("Title"));
        assert!(lines[map[3]].text().contains("let x = 1;"));
        assert!(lines[map[9]].text().contains("last"));
        // Table rows 6 and 7 are swallowed by the row-5 iteration, so they
        // inherit its start rather than pointing past the table.
        assert_eq!(map[5], map[6]);
        assert_eq!(map[6], map[7]);
    }

    #[test]
    fn source_map_is_empty_for_empty_input() {
        let (lines, map) = format_chat_markdown_mapped("", 40, Style::default());
        assert!(lines.is_empty());
        assert!(map.is_empty());
    }

    #[test]
    fn test_format_table() {
        let text = "| Name | Value |\n|------|-------|\n| foo | 42 |\n| bar | 99 |";
        let style = Style::default();
        let lines = format_chat_markdown(text, 40, style);
        // Should produce: top border, header, mid border, 2 data rows, bottom border
        assert!(lines.len() >= 6);
        assert!(lines[0].text().contains('┌'));
        assert!(lines[1].text().contains("Name"));
        assert!(lines[2].text().contains('├'));
    }

    #[test]
    fn test_format_heading() {
        let text = "# Hello World";
        let style = Style::default();
        let lines = format_chat_markdown(text, 40, style);
        assert!(!lines.is_empty());
        assert!(lines[0].text().contains("█ Hello World"));
    }

    #[test]
    fn test_format_code_block() {
        let text = "```\nlet x = 1;\n```";
        let style = Style::default();
        let lines = format_chat_markdown(text, 40, style);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].text().contains("let x = 1;"));
    }

    #[test]
    fn test_format_code_block_wraps_long_lines() {
        let long = "x".repeat(50);
        let text = format!("```\n{long}\n```");
        let style = Style::default();
        let width = 20;
        let lines = format_chat_markdown(&text, width, style);
        assert!(lines.len() > 1, "long code lines should wrap");
        for line in &lines {
            assert!(
                UnicodeWidthStr::width(line.text().as_str()) <= width,
                "wrapped code line {:?} exceeds width {}",
                line.text(),
                width
            );
            assert!(
                line.text().starts_with("  "),
                "code lines keep the 2-space indent"
            );
        }
    }

    #[test]
    fn test_tool_marker_wraps_long_line() {
        let style = Style::default();
        let width = 30;
        let lines = format_chat_markdown(&format!("[Read {}]", "a".repeat(60)), width, style);
        assert!(lines.len() > 1);
        for line in &lines {
            assert!(UnicodeWidthStr::width(line.text().as_str()) <= width);
        }
    }

    #[test]
    fn test_all_output_lines_fit_width() {
        let text = "plain paragraph with enough words to require wrapping across the panel\n\n| Col A | Col B |\n|-------|-------|\n| alpha | beta |\n\n```\nfn main() { println!(\"hello\"); }\n```";
        let style = Style::default();
        let width = 24;
        let lines = format_chat_markdown(text, width, style);
        for line in &lines {
            assert!(
                UnicodeWidthStr::width(line.text().as_str()) <= width,
                "line {:?} exceeds width {}",
                line.text(),
                width
            );
        }
    }

    #[test]
    fn test_bold_text_preserves_style() {
        let text = "This is **important** info";
        let style = Style::default();
        let lines = format_chat_markdown(text, 60, style);
        assert_eq!(lines.len(), 1);
        // Plain-text concatenation strips the ** markers …
        assert_eq!(lines[0].text(), "This is important info");
        // … but the bold run keeps its own segment with both the BOLD
        // modifier and the bold foreground color so it is visually distinct
        // (BOLD alone renders too subtly in many terminals).
        let bold_seg = lines[0]
            .segments
            .iter()
            .find(|s| s.text == "important")
            .expect("expected a segment containing the bold text");
        assert!(
            bold_seg.style.add_modifier.contains(Modifier::BOLD),
            "bold segment must carry the BOLD modifier"
        );
        assert!(
            bold_seg.style.fg.is_some(),
            "bold segment must set a foreground color so the change is visible"
        );
    }

    #[test]
    fn test_italic_text_preserves_style() {
        let text = "An *italic* word";
        let style = Style::default();
        let lines = format_chat_markdown(text, 60, style);
        assert_eq!(lines.len(), 1);
        let italic_seg = lines[0]
            .segments
            .iter()
            .find(|s| s.text == "italic")
            .expect("expected italic segment");
        assert!(italic_seg.style.add_modifier.contains(Modifier::ITALIC));
        assert!(italic_seg.style.fg.is_some());
    }

    #[test]
    fn test_inline_code_preserves_style() {
        let text = "Run `cargo test` now";
        let style = Style::default();
        let lines = format_chat_markdown(text, 60, style);
        assert_eq!(lines.len(), 1);
        let code_seg = lines[0]
            .segments
            .iter()
            .find(|s| s.text == "cargo test")
            .expect("expected inline code segment");
        assert_eq!(code_seg.style.fg, Some(theme::CODE_GREEN));
    }

    #[test]
    fn test_bold_inside_list_item() {
        let text = "- a **bold** item";
        let style = Style::default();
        let lines = format_chat_markdown(text, 60, style);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].text().starts_with("• "));
        let bold_seg = lines[0]
            .segments
            .iter()
            .find(|s| s.text == "bold")
            .expect("expected bold segment inside list item");
        assert!(bold_seg.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_tabs_expanded_in_code_block() {
        let text = "```\n\tindented\n\t\tdouble\n```";
        let style = Style::default();
        let lines = format_chat_markdown(text, 60, style);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].text().contains("    indented"));
        assert!(lines[1].text().contains("        double"));
        // Ensure no tab characters remain
        for line in &lines {
            assert!(
                !line.text().contains('\t'),
                "tab character should be expanded"
            );
        }
    }

    #[test]
    fn test_wide_char_word_wrap() {
        // CJK chars are 2 display columns each. 5 chars = 10 columns.
        let text = "日本語テスト";
        let style = Style::default();
        // Width 6 columns: fits 3 CJK chars (6 cols), then wraps
        let lines = format_chat_markdown(text, 6, style);
        assert_eq!(lines.len(), 2);
        assert_eq!(UnicodeWidthStr::width(lines[0].text().as_str()), 6);
        assert_eq!(UnicodeWidthStr::width(lines[1].text().as_str()), 6);
    }

    #[test]
    fn test_wide_char_table_column_width() {
        let text = "| Name | 値 |\n|------|---|\n| foo | 日本 |";
        let style = Style::default();
        let lines = format_chat_markdown(text, 40, style);
        // The column for "日本" should be 4 display cols wide (not 2)
        let data_row = lines[3].text(); // top, header, separator, first data row
        // "日本" has display width 4, cell should be padded correctly
        assert!(
            data_row.contains("日本"),
            "data row should contain wide chars"
        );
    }

    #[test]
    fn test_table_multiline_cells_show_full_content() {
        let text = "| Feature | Description |\n|---------|-------------|\n| Auth | Supports JWT tokens and refresh rotation |\n| API | REST and GraphQL endpoints |";
        let style = Style::default();
        let width = 30;
        let lines = format_chat_markdown(text, width, style);

        let joined = lines
            .iter()
            .map(|l| l.text())
            .collect::<Vec<_>>()
            .join("\n");
        // Every word of the long cell survives; where it wraps depends on the
        // column width, so assert on the words rather than on a phrase.
        for word in ["Supports", "JWT", "tokens", "refresh", "rotation"] {
            assert!(
                joined.contains(word),
                "full cell text must appear, {word:?} missing from:\n{joined}"
            );
        }
        assert!(
            !joined.contains('…'),
            "tables must not truncate cells with ellipsis"
        );

        for line in &lines {
            if is_table_rendered_line(&line.text()) {
                assert!(
                    UnicodeWidthStr::width(line.text().as_str()) <= width,
                    "table line {:?} exceeds width {}",
                    line.text(),
                    width
                );
            }
        }
    }

    #[test]
    fn fit_column_widths_only_shrinks_the_columns_that_are_too_wide() {
        // Natural widths already fit — nothing moves.
        assert_eq!(fit_column_widths(&[3, 4], 20), vec![3, 4]);
        // A narrow index column and a narrow label column keep their full
        // width; the prose column absorbs the whole deficit.
        assert_eq!(fit_column_widths(&[4, 18, 700], 240), vec![4, 18, 218]);
        // When every column is over its share they split evenly, remainder to
        // the widest.
        assert_eq!(fit_column_widths(&[50, 50], 21), vec![10, 11]);
        // Budget below one cell per column degrades to minimal columns.
        assert_eq!(fit_column_widths(&[9, 9, 9], 2), vec![1, 1, 1]);
    }

    #[test]
    fn test_table_narrow_columns_are_not_starved_by_a_prose_column() {
        let prose = "Two claims, one system: isomorphic generation of exercises \
                     with increasing difficulty and automatic grading built from \
                     generated behavioural tests plus static and dynamic analysis \
                     enforcing the taught architecture over one deployment.";
        let text = format!(
            "| # | Topic | Decision |\n|---|-------|----------|\n| D-01 | Thesis | {prose} |"
        );
        let style = Style::default();
        let width = 100;
        let lines = format_chat_markdown(&text, width, style);

        let row = lines
            .iter()
            .map(|l| l.text())
            .find(|t| t.contains("D-01"))
            .expect("first data row should render");
        // Both narrow columns fit on the row's first visual line instead of
        // wrapping a character at a time.
        assert!(
            row.contains("D-01") && row.contains("Thesis"),
            "narrow columns must keep their natural width, got {row:?}"
        );

        for line in &lines {
            if is_table_rendered_line(&line.text()) {
                assert!(
                    UnicodeWidthStr::width(line.text().as_str()) <= width,
                    "table line {:?} exceeds width {}",
                    line.text(),
                    width
                );
            }
        }
    }

    #[test]
    fn test_table_row_height_matches_tallest_wrapped_cell() {
        let text = "| Col | Text |\n|-----|------|\n| a | word1 word2 word3 word4 |";
        let style = Style::default();
        let width = 18;
        let lines = format_chat_markdown(text, width, style);

        let body_rows: Vec<String> = lines
            .iter()
            .map(|l| l.text())
            .skip_while(|t| !t.starts_with('├'))
            .skip(1)
            .take_while(|t| !t.starts_with('└'))
            .collect();
        assert!(
            body_rows.len() >= 2,
            "tall cell should span multiple │ rows, got {body_rows:?}"
        );
        assert!(body_rows.iter().all(|t| t.starts_with('│')));
    }

    #[test]
    fn preview_href_survives_wrap_and_hit_test() {
        let text = "see [example](https://example.com) here";
        let lines = format_chat_markdown(text, 80, Style::default());
        assert_eq!(href_at(&lines[0], 0), None, "plain prefix is not a link");
        // "see " is 4 cols; the label "example" starts at col 4.
        assert_eq!(href_at(&lines[0], 4), Some("https://example.com"));
        assert_eq!(href_at(&lines[0], 10), Some("https://example.com"));
        assert_eq!(href_at(&lines[0], 12), None);

        let wrapped = format_chat_markdown(
            "aaa [this-is-a-very-long-link-label](docs/foo.md) bbb",
            16,
            Style::default(),
        );
        assert!(
            wrapped.iter().any(|l| {
                l.segments
                    .iter()
                    .any(|s| s.href.as_deref() == Some("docs/foo.md"))
            }),
            "wrapped link must keep its href"
        );
    }

    #[test]
    fn preview_link_at_accounts_for_scroll_and_origin() {
        let lines = format_chat_markdown("plain\nsee [docs](README.md) now", 80, Style::default());
        let inner = Rect::new(10, 5, 40, 10);
        let origin = inner.x + 1;
        // Line 1 is "see docs now"; "docs" starts after "see " (4 cols).
        assert_eq!(
            preview_link_at(&lines, 0, origin, inner, origin + 4, inner.y + 1),
            Some("README.md")
        );
        assert_eq!(
            preview_link_at(&lines, 1, origin, inner, origin + 4, inner.y),
            Some("README.md")
        );
        assert_eq!(
            preview_link_at(&lines, 0, origin, inner, origin, inner.y),
            None
        );
    }

    #[test]
    fn preview_line_for_fragment_matches_rendered_heading() {
        let lines = format_chat_markdown("# Install\n\nbody\n", 40, Style::default());
        assert_eq!(preview_line_for_fragment(&lines, "install"), Some(0));
        assert_eq!(preview_line_for_fragment(&lines, "missing"), None);
    }

    #[test]
    fn preview_find_matches_rendered_text_case_insensitively() {
        // `**bold**` renders without its markers, so the match is on what is shown.
        let lines = format_chat_markdown("# Título\n\nA **bold** Bold word\n", 40, Style::default());
        let body = lines.iter().position(|l| l.text().contains("bold")).unwrap();
        let matches = preview_find_matches(&lines, "BOLD");
        assert_eq!(matches.len(), 2);
        assert!(matches.iter().all(|m| m.line == body && m.end - m.start == 4));
        let text: Vec<char> = lines[body].text().chars().collect();
        let first: String = text[matches[0].start..matches[0].end].iter().collect();
        assert_eq!(first, "bold");
        // Char offsets, not bytes: the accented heading still matches.
        assert_eq!(preview_find_matches(&lines, "título").len(), 1);
        assert!(preview_find_matches(&lines, "").is_empty());
        assert!(preview_find_matches(&lines, "**").is_empty());
    }

    #[test]
    fn paint_preview_matches_styles_only_the_matched_cells() {
        let lines = vec![ChatLine::single("say hello", Style::default())];
        let matches = preview_find_matches(&lines, "hello");
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        render_lines_to_buffer(&lines, area, &mut buf, 0, Style::default(), None);
        paint_preview_matches(&lines, &matches, Some((0, 4)), area, &mut buf, 0);
        // Text starts one column in; "hello" sits at columns 5..10.
        assert_eq!(buf[(4, 0)].bg, Style::default().bg.unwrap_or_default());
        assert_eq!(buf[(5, 0)].bg, theme::ACCENT);
        assert_eq!(buf[(9, 0)].bg, theme::ACCENT);
        assert_ne!(buf[(10, 0)].bg, theme::ACCENT);
    }

    #[test]
    fn hovered_link_uses_hover_style_for_every_matching_href() {
        let lines = format_chat_markdown(
            "see [example](https://example.com) here",
            80,
            Style::default(),
        );
        let hovered = Some("https://example.com");
        let styles: Vec<_> = lines[0]
            .segments
            .iter()
            .map(|s| (s.text.as_str(), segment_style(s, hovered)))
            .collect();
        assert!(
            styles
                .iter()
                .any(|(t, st)| *t == "example" && *st == link_hover_style()),
            "hovered label must use link_hover_style, got {styles:?}"
        );
        assert!(
            styles
                .iter()
                .any(|(t, st)| *t == "see " && *st != link_hover_style()),
            "plain prefix must stay unhovered"
        );
        assert_eq!(
            segment_style(&lines[0].segments[0], None),
            lines[0].segments[0].style,
            "no hover → original styles"
        );
    }
}
