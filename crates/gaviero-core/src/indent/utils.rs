//! Shared utility functions for indentation computation.

/// Extract leading whitespace from a line in a rope.
pub fn leading_whitespace_at(doc: &ropey::Rope, line: usize) -> String {
    let text: String = doc.line(line).into();
    text.chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

/// Compute the visual width of a whitespace string.
pub fn indent_visual_width(ws: &str, tab_width: usize) -> usize {
    ws.chars()
        .map(|c| if c == '\t' { tab_width } else { 1 })
        .sum()
}

/// Convert a whitespace string to an indent level.
pub fn whitespace_to_level(ws: &str, indent_unit: &str, tab_width: usize) -> usize {
    if indent_unit.is_empty() {
        return 0;
    }
    let ws_width = indent_visual_width(ws, tab_width);
    let unit_width = indent_visual_width(indent_unit, tab_width);
    if unit_width == 0 {
        return 0;
    }
    ws_width / unit_width
}
