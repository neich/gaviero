use anyhow::{Context, Result};
use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

// ── UI Color & Number Constants ──────────────────────────────────
// Centralized values used across all panels. Change here to retheme.
// Not all are referenced yet — panels are being migrated incrementally.

/// Dark panel background.
pub const PANEL_BG: Color = Color::Rgb(30, 33, 40);
/// Editor current line highlight.
pub const CURRENT_LINE_BG: Color = Color::Rgb(55, 60, 70);
/// Text selection background.
pub const SELECTION_BG: Color = Color::Rgb(77, 120, 204);
/// Search match highlight.
pub const SEARCH_HIGHLIGHT_BG: Color = Color::Rgb(120, 90, 30);
/// Focused panel border / accent.
pub const FOCUS_BORDER: Color = Color::Rgb(97, 175, 239);
/// Unfocused panel border (same value as TEXT_DIM; reserved for semantic use).
#[allow(unused)] pub const UNFOCUS_BORDER: Color = Color::Rgb(99, 109, 131);
/// Default foreground text.
pub const TEXT_FG: Color = Color::Rgb(171, 178, 191);
/// Dimmed / secondary text.
pub const TEXT_DIM: Color = Color::Rgb(99, 109, 131);
/// Bright foreground text (headings, titles).
pub const TEXT_BRIGHT: Color = Color::Rgb(215, 218, 224);
/// Accent color (links, keywords in UI).
pub const ACCENT: Color = Color::Rgb(97, 175, 239);
/// Warning / info color (yellow).
pub const WARNING: Color = Color::Rgb(229, 192, 123);
/// Success color (green).
pub const SUCCESS: Color = Color::Rgb(80, 200, 80);
/// Error color (red).
pub const ERROR: Color = Color::Rgb(220, 80, 80);
/// Diff added line background (used by diff_overlay).
#[allow(unused)] pub const DIFF_ADDED_BG: Color = Color::Rgb(40, 65, 42);
/// Diff added line (accepted) background.
#[allow(unused)] pub const DIFF_ADDED_ACCEPTED_BG: Color = Color::Rgb(45, 74, 48);
/// Diff removed line background (used by diff_overlay).
#[allow(unused)] pub const DIFF_REMOVED_BG: Color = Color::Rgb(65, 40, 40);
/// Diff removed line (rejected/crossed) background.
#[allow(unused)] pub const DIFF_REMOVED_REJECTED_BG: Color = Color::Rgb(55, 45, 45);
/// Tab bar background.
pub const TAB_BG: Color = Color::Rgb(36, 40, 47);
/// Input field background.
pub const INPUT_BG: Color = Color::Rgb(44, 49, 58);
/// Browse mode / highlighted message background.
pub const BROWSE_BG: Color = Color::Rgb(55, 62, 75);
/// Tool call / muted label text.
pub const TOOL_DIM: Color = Color::Rgb(127, 132, 142);
/// Code / string literal green.
pub const CODE_GREEN: Color = Color::Rgb(152, 195, 121);
/// Subtle border / rule color.
pub const BORDER_DIM: Color = Color::Rgb(75, 82, 99);
/// Dark subtle background for panels.
pub const DARK_BG: Color = Color::Rgb(60, 65, 75);
/// Cyan for special markers (file proposals, info).
pub const INFO_CYAN: Color = Color::Rgb(86, 182, 194);
/// Property / struct field red.
pub const PROPERTY_RED: Color = Color::Rgb(224, 108, 117);
/// Image attachment badge background.
pub const IMAGE_BADGE_BG: Color = Color::Rgb(80, 60, 100);
/// Badge / tag background.
pub const BADGE_BG: Color = Color::Rgb(60, 68, 80);
/// Code block dark background.
pub const CODE_BLOCK_BG: Color = Color::Rgb(40, 44, 52);
/// Diff added line foreground background (subtle green).
pub const DIFF_ADD_LINE_BG: Color = Color::Rgb(30, 50, 30);
/// Diff removed line foreground background (subtle red).
pub const DIFF_REM_LINE_BG: Color = Color::Rgb(50, 30, 30);
/// Numeric / constant orange.
pub const NUMERIC_ORANGE: Color = Color::Rgb(209, 154, 102);
/// Medium gray for secondary labels.
pub const MEDIUM_GRAY: Color = Color::Rgb(140, 145, 155);
/// Focused list-item selection background.
pub const FOCUSED_SELECTION_BG: Color = Color::Rgb(55, 100, 180);
/// Swarm tier badge: Coordinator.
pub const TIER_COORDINATOR: Color = Color::Rgb(180, 120, 220);
/// Swarm tier badge: Reasoning.
pub const TIER_REASONING: Color = Color::Rgb(80, 160, 230);
/// Swarm tier badge: Execution.
pub const TIER_EXECUTION: Color = Color::Rgb(80, 200, 120);
/// Swarm tier badge: Mechanical.
pub const TIER_MECHANICAL: Color = Color::Rgb(220, 200, 80);
/// Activity line: tool call.
pub const ACTIVITY_TOOL_CALL: Color = Color::Rgb(80, 200, 220);
/// Activity line: status change.
pub const ACTIVITY_STATUS: Color = Color::Rgb(200, 180, 80);
/// Terminal cursor foreground (inverted).
pub const CURSOR_INVERT_FG: Color = Color::Black;
/// Selected item bright text.
pub const SELECTED_BRIGHT: Color = Color::White;
/// Focused panel header bright foreground.
pub const PANEL_HEADER_FOCUSED_FG: Color = Color::Rgb(230, 235, 245);
/// Unfocused panel header dimmed foreground.
pub const PANEL_HEADER_UNFOCUSED_FG: Color = Color::Rgb(120, 128, 145);

// ── Magic Number Constants ──────────────────────────────────────

/// Default tab display width.
#[allow(unused)] pub const DEFAULT_TAB_WIDTH: u8 = 4;
/// Crossterm event poll timeout in milliseconds.
#[allow(unused)] pub const CROSSTERM_POLL_MS: u64 = 50;
/// UI tick interval in milliseconds (~30fps).
#[allow(unused)] pub const TICK_INTERVAL_MS: u64 = 33;
/// Default number of messages to keep on /compact.
#[allow(unused)] pub const DEFAULT_COMPACT_KEEP: usize = 6;
/// Maximum lines to search upward for indent baseline.
#[allow(unused)] pub const MAX_BASELINE_SEARCH_LINES: usize = 5;
/// Broadcast bus channel capacity.
#[allow(unused)] pub const BUS_CHANNEL_CAPACITY: usize = 256;
/// Terminal resize step (percentage per key press).
pub const TERMINAL_RESIZE_STEP: u16 = 5;
/// Terminal maximum split percentage.
pub const TERMINAL_MAX_PERCENT: u16 = 80;
/// Terminal minimum split percentage.
pub const TERMINAL_MIN_PERCENT: u16 = 10;
/// Chat panel page-scroll lines.
#[allow(unused)] pub const CHAT_PAGE_SCROLL: usize = 20;
/// Diff viewer page-scroll lines.
pub const DIFF_PAGE_SCROLL: usize = 10;
/// Mouse scroll delta (lines per wheel event).
#[allow(unused)] pub const MOUSE_SCROLL_DELTA: usize = 3;
/// Diff gutter width (columns).
pub const DIFF_GUTTER_WIDTH: u16 = 5;
/// Chat input area height (rows).
#[allow(unused)] pub const INPUT_AREA_HEIGHT: u16 = 3;
/// Maximum autocomplete popup height (rows).
#[allow(unused)] pub const AUTOCOMPLETE_MAX_HEIGHT: u16 = 8;
/// Status message display duration (seconds).
#[allow(unused)] pub const STATUS_MESSAGE_DURATION_SECS: u64 = 5;
/// Diff context lines for unified diff.
#[allow(unused)] pub const DIFF_CONTEXT_LINES: usize = 3;
/// Maximum task planning retry attempts.
#[allow(unused)] pub const MAX_PLAN_ATTEMPTS: usize = 2;

#[derive(Debug, Deserialize)]
struct ThemeFile {
    #[serde(default)]
    highlights: HashMap<String, StyleDef>,
    #[serde(default)]
    ui: HashMap<String, StyleDef>,
}

#[derive(Debug, Deserialize)]
struct StyleDef {
    fg: Option<String>,
    bg: Option<String>,
    #[serde(default)]
    bold: bool,
    #[serde(default)]
    italic: bool,
    #[serde(default)]
    underline: bool,
}

#[derive(Debug)]
pub struct Theme {
    highlights: HashMap<String, Style>,
    ui: HashMap<String, Style>,
}

impl Theme {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path).context("reading theme file")?;
        let file: ThemeFile = toml::from_str(&content).context("parsing theme file")?;

        let highlights = file
            .highlights
            .into_iter()
            .map(|(k, v)| (k, style_from_def(&v)))
            .collect();

        let ui = file
            .ui
            .into_iter()
            .map(|(k, v)| (k, style_from_def(&v)))
            .collect();

        Ok(Self { highlights, ui })
    }

    /// Get the style for a highlight group, with dot-notation fallback.
    /// E.g., `keyword.function` falls back to `keyword` if no exact match.
    pub fn highlight_style(&self, group: &str) -> Option<Style> {
        if let Some(style) = self.highlights.get(group) {
            return Some(*style);
        }
        // Fallback: strip last segment
        if let Some(dot_pos) = group.rfind('.') {
            return self.highlight_style(&group[..dot_pos]);
        }
        None
    }

    /// Get a UI style by name.
    pub fn ui_style(&self, name: &str) -> Style {
        self.ui.get(name).copied().unwrap_or_default()
    }

    /// Default text style.
    pub fn default_style(&self) -> Style {
        Style::default().fg(Color::Rgb(171, 178, 191)) // #abb2bf
    }

    /// Create a built-in default theme (used when no theme file is found).
    pub fn builtin_default() -> Self {
        // Colors chosen to work on dark gray terminal backgrounds (~40-65 range).
        // Foreground colors are bright enough for contrast; background colors
        // are either omitted (inherit terminal bg) or high enough to stand out.
        let mut highlights = HashMap::new();
        highlights.insert("comment".into(), Style::default().fg(Color::Rgb(127, 132, 142)));
        highlights.insert("string".into(), Style::default().fg(Color::Rgb(152, 195, 121)));
        highlights.insert("constant.numeric".into(), Style::default().fg(Color::Rgb(209, 154, 102)));
        highlights.insert("constant.builtin".into(), Style::default().fg(Color::Rgb(209, 154, 102)).add_modifier(Modifier::BOLD));
        highlights.insert("keyword".into(), Style::default().fg(Color::Rgb(198, 120, 221)));
        highlights.insert("keyword.function".into(), Style::default().fg(Color::Rgb(198, 120, 221)));
        highlights.insert("function".into(), Style::default().fg(Color::Rgb(97, 175, 239)));
        highlights.insert("function.call".into(), Style::default().fg(Color::Rgb(97, 175, 239)));
        highlights.insert("type".into(), Style::default().fg(Color::Rgb(229, 192, 123)));
        highlights.insert("variable".into(), Style::default().fg(Color::Rgb(171, 178, 191)));
        highlights.insert("number".into(), Style::default().fg(Color::Rgb(209, 154, 102)));
        highlights.insert("property".into(), Style::default().fg(Color::Rgb(224, 108, 117)));
        highlights.insert("operator".into(), Style::default().fg(Color::Rgb(200, 204, 212)));
        highlights.insert("punctuation".into(), Style::default().fg(Color::Rgb(157, 165, 180)));
        // Markdown
        highlights.insert("markup.heading".into(), Style::default().fg(Color::Rgb(97, 175, 239)).add_modifier(Modifier::BOLD));
        highlights.insert("markup.bold".into(), Style::default().fg(Color::Rgb(229, 192, 123)).add_modifier(Modifier::BOLD));
        highlights.insert("markup.italic".into(), Style::default().fg(Color::Rgb(198, 120, 221)));
        highlights.insert("markup.link".into(), Style::default().fg(Color::Rgb(97, 175, 239)));
        highlights.insert("markup.link.url".into(), Style::default().fg(Color::Rgb(152, 195, 121)));
        highlights.insert("markup.code".into(), Style::default().fg(Color::Rgb(152, 195, 121)));
        highlights.insert("markup.code.block".into(), Style::default().fg(Color::Rgb(127, 132, 142)));
        highlights.insert("markup.list".into(), Style::default().fg(Color::Rgb(209, 154, 102)));
        highlights.insert("markup.quote".into(), Style::default().fg(Color::Rgb(127, 132, 142)));

        let mut ui = HashMap::new();
        ui.insert("line_number".into(), Style::default().fg(Color::Rgb(99, 109, 131)));
        ui.insert("line_number.active".into(), Style::default().fg(Color::Rgb(171, 178, 191)));
        ui.insert("cursor".into(), Style::default().bg(Color::Rgb(82, 139, 255)));
        ui.insert("selection".into(), Style::default().bg(Color::Rgb(77, 120, 204)));
        ui.insert("diff.added".into(), Style::default().bg(Color::Rgb(45, 74, 48)));
        ui.insert("diff.removed".into(), Style::default().bg(Color::Rgb(74, 45, 45)));
        ui.insert("status_bar".into(), Style::default().fg(Color::Rgb(215, 218, 224)).bg(Color::Rgb(44, 49, 58)));

        Self { highlights, ui }
    }
}

fn style_from_def(def: &StyleDef) -> Style {
    let mut style = Style::default();
    if let Some(fg) = &def.fg {
        if let Some(color) = parse_hex_color(fg) {
            style = style.fg(color);
        }
    }
    if let Some(bg) = &def.bg {
        if let Some(color) = parse_hex_color(bg) {
            style = style.bg(color);
        }
    }
    if def.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    if def.italic {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if def.underline {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    style
}

fn parse_hex_color(hex: &str) -> Option<Color> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_default() {
        let theme = Theme::builtin_default();
        assert!(theme.highlight_style("comment").is_some());
        assert!(theme.highlight_style("keyword").is_some());
        assert!(theme.highlight_style("keyword.function").is_some());
    }

    #[test]
    fn test_highlight_fallback() {
        let theme = Theme::builtin_default();
        // "keyword.control" should fall back to "keyword"
        let kw_style = theme.highlight_style("keyword").unwrap();
        let ctrl_style = theme.highlight_style("keyword.control").unwrap();
        assert_eq!(kw_style, ctrl_style);
    }

    #[test]
    fn test_parse_hex_color() {
        assert_eq!(parse_hex_color("#ff0000"), Some(Color::Rgb(255, 0, 0)));
        assert_eq!(parse_hex_color("#00ff00"), Some(Color::Rgb(0, 255, 0)));
        assert_eq!(parse_hex_color("invalid"), None);
    }

    #[test]
    fn test_load_theme_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.toml");
        std::fs::write(
            &path,
            r##"
[highlights]
"comment" = { fg = "#6a737d", italic = true }
"keyword" = { fg = "#c678dd" }

[ui]
"status_bar" = { fg = "#abb2bf", bg = "#21252b" }
"##,
        )
        .unwrap();

        let theme = Theme::load(&path).unwrap();
        assert!(theme.highlight_style("comment").is_some());
        assert!(theme.highlight_style("keyword").is_some());
    }
}
