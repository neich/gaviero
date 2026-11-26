use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Widget,
};

/// How a tab is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabKind {
    Normal,
    /// A read-only diff overlay: orange, to signal it cannot be edited.
    DiffView,
    /// The global `$GAVIERO_ROOT/.gaviero/settings.json`, shared by every
    /// gaviero instance: violet, so an edit there is never mistaken for a
    /// workspace one.
    Global,
    /// A session-host shell tab: teal on a teal-tinted background, active
    /// or not, so it never reads as a gaviero session.
    Shell,
}

/// One activity badge: `letter` on a cell of `colour`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabBadge {
    pub letter: char,
    pub colour: Color,
}

/// Up to two badges drawn after a tab's label, in this order; `None` is
/// not drawn and takes no room.
pub type TabBadges = [Option<TabBadge>; 2];

/// Colours of a tab bar's `Normal` tabs, bar background and chrome.
/// `DiffView`, `Global` and `Shell` tabs keep their own colours on any
/// palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabPalette {
    /// The bar row behind and after the tabs.
    pub bar_bg: Color,
    pub inactive_fg: Color,
    pub inactive_bg: Color,
    pub active_fg: Color,
    pub active_bg: Color,
    /// Separators, the overflow `...` and an inactive tab's close button.
    pub dim: Color,
}

/// The editor's buffer tabs: cool grey-blue.
pub const EDITOR_PALETTE: TabPalette = TabPalette {
    bar_bg: Color::Rgb(35, 39, 46),
    inactive_fg: Color::Rgb(157, 165, 180),
    inactive_bg: Color::Rgb(35, 39, 46),
    active_fg: Color::White,
    active_bg: Color::Rgb(55, 60, 70),
    dim: Color::Rgb(99, 109, 131),
};

/// The session host's tabs: warm dark bronze, so the host bar never reads
/// as a second row of editor tabs when a gaviero tab draws its own below.
/// No blue in it, so the blue "finished" badge still stands out.
pub const HOST_PALETTE: TabPalette = TabPalette {
    bar_bg: Color::Rgb(30, 25, 20),
    inactive_fg: Color::Rgb(190, 170, 140),
    inactive_bg: Color::Rgb(30, 25, 20),
    active_fg: Color::Rgb(255, 238, 205),
    active_bg: Color::Rgb(88, 64, 38),
    dim: Color::Rgb(120, 100, 76),
};

pub struct TabBar<'a> {
    /// (name, modified, kind).
    pub titles: &'a [(String, bool, TabKind)],
    pub active: usize,
    /// Draw a close button (`x`) at the start of every tab; see `close_at_x`.
    pub closable: bool,
    /// One entry per tab: each badge shown adds a blank and its letter cell
    /// after the label, so a tab grows and shrinks as its badges come and
    /// go (`badge_count` sizes it for drawing and hit-testing alike).
    pub badges: Option<&'a [TabBadges]>,
    pub palette: TabPalette,
}

/// Cells per badge shown: a blank before it, then the letter on one cell of
/// the badge colour. A cell background only, on purpose: Konsole draws block
/// elements and shape glyphs antialiased but fills cell backgrounds without
/// antialiasing, so at fractional display scaling (cell edges between
/// device pixels) a glyph edge on a cell boundary lets that cell's own
/// background show through as a hairline. The Powerline half-circle caps
/// (U+E0B6 / U+E0B4) and then half blocks (`▐` / `▌`) around the letter
/// both left a dark line beside it, and the rounded glyphs that would not
/// (one glyph alone, `🅐` / `Ⓐ`) need a fallback font and are East Asian
/// ambiguous width, two cells in some terminals.
const BADGE_WIDTH: u16 = 2;
/// Letter colours on a badge: dark on light colours, light on dark ones.
const BADGE_DARK_FG: Color = Color::Rgb(20, 20, 20);
const BADGE_LIGHT_FG: Color = Color::Rgb(245, 245, 245);

/// The letter colour that reads on `bg`.
fn badge_letter_fg(bg: Color) -> Color {
    match bg {
        Color::Rgb(r, g, b) => {
            let luma = 299 * r as u32 + 587 * g as u32 + 114 * b as u32;
            // Perceived brightness (Rec. 601) × 1000; the cut sits low
            // because white loses to black long before mid-grey.
            if luma >= 100_000 { BADGE_DARK_FG } else { BADGE_LIGHT_FG }
        }
        _ => BADGE_DARK_FG,
    }
}

/// The close button drawn at the start of a closable tab.
const CLOSE_MARK: &str = "x";
/// Close button colour: dim until the tab is active.
const CLOSE_FG: Color = Color::Rgb(224, 108, 117);

/// Orange used for read-only diff-view tabs (e.g. git panel diff preview).
const DIFF_VIEW_FG: Color = Color::Rgb(214, 134, 50);
/// Violet for the global settings tab, with a tinted background.
const GLOBAL_FG: Color = Color::Rgb(198, 146, 233);
const GLOBAL_BG: Color = Color::Rgb(52, 40, 66);
const GLOBAL_ACTIVE_BG: Color = Color::Rgb(78, 56, 102);
/// Teal for shell tabs, tinted even when inactive. No pure green, yellow or
/// blue in it, so the activity badges stay readable on it.
const SHELL_FG: Color = Color::Rgb(120, 200, 190);
const SHELL_ACTIVE_FG: Color = Color::Rgb(210, 245, 238);
const SHELL_BG: Color = Color::Rgb(18, 44, 46);
const SHELL_ACTIVE_BG: Color = Color::Rgb(26, 82, 86);

/// The rendered label for a tab and its display width in terminal cells.
/// Width must be unicode-aware (`'●'` is 3 bytes / 1 cell, CJK chars are
/// 2 cells) or rendering and mouse hit-testing desync.
/// The label ends in `BADGE_WIDTH` blank cells per badge shown (`badges`),
/// drawn over by `render`.
fn tab_label(title: &str, modified: bool, closable: bool, badges: usize) -> (String, u16) {
    use unicode_width::UnicodeWidthStr;
    let close = if closable { CLOSE_MARK } else { "" };
    let gap = if closable { " " } else { "" };
    let prefix = if modified { "● " } else { "" };
    let slots = " ".repeat(BADGE_WIDTH as usize * badges);
    let label = format!(" {close}{gap}{prefix}{title}{slots} ");
    let width = label.as_str().width() as u16;
    (label, width)
}

impl<'a> TabBar<'a> {
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let p = self.palette;
        let bg_style = Style::default().bg(p.bar_bg);
        for x in area.x..area.right() {
            buf[(x, area.y)].set_style(bg_style);
        }

        let mut x = area.x;
        for (i, (title, modified, kind)) in self.titles.iter().enumerate() {
            let is_active = i == self.active;
            let shown = self.badge_count(i);
            let (label, label_len) = tab_label(title, *modified, self.closable, shown);

            if x + label_len > area.right() {
                // Show "..." if there are more tabs
                if x + 3 <= area.right() {
                    let dots = Line::from(Span::styled(
                        "...",
                        Style::default().fg(p.dim),
                    ));
                    let dots_area = Rect {
                        x,
                        y: area.y,
                        width: 3,
                        height: 1,
                    };
                    Widget::render(dots, dots_area, buf);
                }
                break;
            }

            let style = if *kind == TabKind::DiffView {
                let bg = if is_active {
                    Color::Rgb(55, 60, 70)
                } else {
                    Color::Rgb(35, 39, 46)
                };
                let mut s = Style::default().fg(DIFF_VIEW_FG).bg(bg);
                if is_active {
                    s = s.add_modifier(Modifier::BOLD);
                }
                s
            } else if *kind == TabKind::Global {
                let bg = if is_active { GLOBAL_ACTIVE_BG } else { GLOBAL_BG };
                let mut s = Style::default().fg(GLOBAL_FG).bg(bg);
                if is_active {
                    s = s.add_modifier(Modifier::BOLD);
                }
                s
            } else if *kind == TabKind::Shell {
                if is_active {
                    Style::default()
                        .fg(SHELL_ACTIVE_FG)
                        .bg(SHELL_ACTIVE_BG)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(SHELL_FG).bg(SHELL_BG)
                }
            } else if is_active {
                Style::default()
                    .fg(p.active_fg)
                    .bg(p.active_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(p.inactive_fg).bg(p.inactive_bg)
            };

            let tab = Line::from(Span::styled(label, style));
            let tab_area = Rect {
                x,
                y: area.y,
                width: label_len,
                height: 1,
            };
            Widget::render(tab, tab_area, buf);
            if self.closable {
                // The close button sits one cell in, after the leading pad.
                let fg = if is_active { CLOSE_FG } else { p.dim };
                buf[(x + 1, area.y)]
                    .set_symbol(CLOSE_MARK)
                    .set_style(Style::default().fg(fg));
            }
            if let Some(badges) = self.badges.and_then(|b| b.get(i)) {
                // The shown badges sit before the trailing pad, in order,
                // each a blank and then its letter on the badge colour.
                let first = x + label_len - 1 - BADGE_WIDTH * shown as u16;
                for (k, badge) in badges.iter().flatten().enumerate() {
                    let letter = first + k as u16 * BADGE_WIDTH + 1;
                    buf[(letter, area.y)].set_char(badge.letter).set_style(
                        Style::default()
                            .fg(badge_letter_fg(badge.colour))
                            .bg(badge.colour)
                            .add_modifier(Modifier::BOLD),
                    );
                }
            }

            // Separator
            x += label_len;
            if x < area.right() {
                buf[(x, area.y)]
                    .set_char('│')
                    .set_style(Style::default().fg(p.dim));
                x += 1;
            }
        }
    }

    /// How many badges tab `i` shows: the room its label makes for them.
    fn badge_count(&self, i: usize) -> usize {
        self.badges
            .and_then(|b| b.get(i))
            .map_or(0, |b| b.iter().flatten().count())
    }

    /// Return the tab index at the given x coordinate, for mouse click handling.
    pub fn tab_at_x(&self, click_x: u16, area_x: u16) -> Option<usize> {
        let mut x = area_x;
        for (i, (title, modified, _)) in self.titles.iter().enumerate() {
            let label_len =
                tab_label(title, *modified, self.closable, self.badge_count(i)).1 + 1; // +1 for separator
            if click_x >= x && click_x < x + label_len {
                return Some(i);
            }
            x += label_len;
        }
        None
    }

    /// The tab whose close button is at the given x coordinate; `None` when
    /// the bar is not closable or the click is elsewhere on a tab.
    pub fn close_at_x(&self, click_x: u16, area_x: u16) -> Option<usize> {
        if !self.closable {
            return None;
        }
        let mut x = area_x;
        for (i, (title, modified, _)) in self.titles.iter().enumerate() {
            if click_x == x + 1 {
                return Some(i);
            }
            x += tab_label(title, *modified, true, self.badge_count(i)).1 + 1;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_label_width_is_cells_not_bytes() {
        // '●' is 3 bytes but 1 cell; " ● a " = 5 cells.
        assert_eq!(tab_label("a", true, false, 0).1, 5);
        // Plain ASCII: " a " = 3 cells.
        assert_eq!(tab_label("a", false, false, 0).1, 3);
        // CJK chars occupy 2 cells each: " 日本 " = 2 + 4 = 6.
        assert_eq!(tab_label("日本", false, false, 0).1, 6);
        // The close button adds "x ": " x a " = 5 cells.
        assert_eq!(tab_label("a", false, true, 0).1, 5);
        // Each badge shown adds a blank and its letter: " x a A S " = 9.
        assert_eq!(tab_label("a", false, true, 1).1, 7);
        assert_eq!(tab_label("a", false, true, 2).1, 9);
    }

    #[test]
    fn close_button_is_the_second_cell_of_each_tab() {
        let titles = vec![
            ("ab".to_string(), false, TabKind::Normal),
            ("c".to_string(), true, TabKind::Normal),
        ];
        let bar = TabBar {
            titles: &titles,
            active: 0,
            closable: true,
            badges: None,
            palette: EDITOR_PALETTE,
        };
        // Tab 0: " x ab " (cols 0..6) + separator at 6; tab 1 starts at 7.
        assert_eq!(bar.close_at_x(1, 0), Some(0));
        assert_eq!(bar.close_at_x(3, 0), None);
        assert_eq!(bar.tab_at_x(3, 0), Some(0));
        assert_eq!(bar.close_at_x(8, 0), Some(1));
        assert_eq!(bar.tab_at_x(8, 0), Some(1));
        // The close cell is drawn as `x`.
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
        bar.render(Rect::new(0, 0, 20, 1), &mut buf);
        assert_eq!(buf[(1, 0)].symbol(), "x");
        assert_eq!(buf[(8, 0)].symbol(), "x");

        let plain = TabBar {
            titles: &titles,
            active: 0,
            closable: false,
            badges: None,
            palette: EDITOR_PALETTE,
        };
        assert_eq!(plain.close_at_x(1, 0), None);
    }

    #[test]
    fn badges_follow_the_label_and_size_the_tab() {
        let titles = vec![
            ("ab".to_string(), false, TabKind::Normal),
            ("c".to_string(), false, TabKind::Normal),
        ];
        let yellow = Color::Rgb(204, 204, 0);
        let blue = Color::Rgb(0, 0, 204);
        let agent = TabBadge { letter: 'A', colour: yellow };
        let shell = TabBadge { letter: 'S', colour: blue };
        let badges: Vec<TabBadges> = vec![[Some(agent), None], [None, Some(shell)]];
        let bar = TabBar {
            titles: &titles,
            active: 0,
            closable: true,
            badges: Some(&badges),
            palette: HOST_PALETTE,
        };
        // Tab 0: " x ab" + " A" + pad (cols 0..8), separator at 8; tab 1:
        // " x c" + " S" + pad (cols 9..16), separator at 16.
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 1));
        bar.render(Rect::new(0, 0, 30, 1), &mut buf);
        assert_eq!(buf[(5, 0)].symbol(), " ", "the blank before the badge");
        assert_eq!(buf[(5, 0)].bg, HOST_PALETTE.active_bg);
        // The letter on one cell of the badge colour, dark on a light one.
        assert_eq!(buf[(6, 0)].symbol(), "A");
        assert_eq!(buf[(6, 0)].bg, yellow);
        assert_eq!(buf[(6, 0)].fg, BADGE_DARK_FG);
        assert!(buf[(6, 0)].modifier.contains(Modifier::BOLD));
        assert_eq!(buf[(7, 0)].symbol(), " ", "the trailing pad");
        assert_eq!(buf[(7, 0)].bg, HOST_PALETTE.active_bg);
        assert_eq!(buf[(8, 0)].symbol(), "│");
        // A shell badge alone comes right after the blank: no room is kept
        // for the agent's. Light on a dark colour.
        assert_eq!(buf[(12, 0)].symbol(), "c");
        assert_eq!(buf[(13, 0)].symbol(), " ");
        assert_eq!(buf[(13, 0)].bg, HOST_PALETTE.inactive_bg);
        assert_eq!(buf[(14, 0)].symbol(), "S");
        assert_eq!(buf[(14, 0)].bg, blue);
        assert_eq!(buf[(14, 0)].fg, BADGE_LIGHT_FG);
        assert_eq!(buf[(15, 0)].symbol(), " ", "the trailing pad");
        assert_eq!(buf[(15, 0)].bg, HOST_PALETTE.inactive_bg);
        assert_eq!(buf[(16, 0)].symbol(), "│");
        // Hit-testing sizes each tab by its badges, like drawing.
        assert_eq!(bar.close_at_x(10, 0), Some(1));
        assert_eq!(bar.tab_at_x(8, 0), Some(0));
        assert_eq!(bar.tab_at_x(9, 0), Some(1));

        // Both badges: " x ab A S ", a tab-coloured blank between them.
        let both: Vec<TabBadges> = vec![[Some(agent), Some(shell)], [None, None]];
        let bar = TabBar {
            badges: Some(&both),
            ..bar
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 1));
        bar.render(Rect::new(0, 0, 30, 1), &mut buf);
        assert_eq!(buf[(6, 0)].symbol(), "A");
        assert_eq!(buf[(7, 0)].symbol(), " ");
        assert_eq!(buf[(7, 0)].bg, HOST_PALETTE.active_bg);
        assert_eq!(buf[(8, 0)].symbol(), "S");
        assert_eq!(buf[(8, 0)].bg, blue);
        assert_eq!(buf[(10, 0)].symbol(), "│");
        assert_eq!(bar.close_at_x(12, 0), Some(1));

        // No badge shown: as wide as a bar without badges.
        let none: Vec<TabBadges> = vec![[None, None], [None, None]];
        let bar = TabBar {
            badges: Some(&none),
            ..bar
        };
        let plain = TabBar {
            badges: None,
            ..bar
        };
        assert_eq!(bar.close_at_x(8, 0), Some(1));
        assert_eq!(plain.close_at_x(8, 0), Some(1));
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 1));
        bar.render(Rect::new(0, 0, 30, 1), &mut buf);
        assert_eq!(buf[(6, 0)].symbol(), "│");
    }

    #[test]
    fn shell_tabs_keep_their_own_colours_active_or_not() {
        let titles = vec![
            ("gav".to_string(), false, TabKind::Normal),
            (">_ sh".to_string(), false, TabKind::Shell),
        ];
        let draw = |active| {
            let bar = TabBar {
                titles: &titles,
                active,
                closable: false,
                badges: None,
                palette: HOST_PALETTE,
            };
            let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
            bar.render(Rect::new(0, 0, 20, 1), &mut buf);
            buf
        };
        // " gav " (0..5), separator at 5, " >_ sh " from 6.
        let inactive = draw(0);
        assert_eq!(inactive[(7, 0)].symbol(), ">");
        assert_eq!(inactive[(7, 0)].bg, SHELL_BG);
        assert_eq!(inactive[(7, 0)].fg, SHELL_FG);
        assert_ne!(SHELL_BG, HOST_PALETTE.inactive_bg, "tinted even when inactive");
        let active = draw(1);
        assert_eq!(active[(7, 0)].bg, SHELL_ACTIVE_BG);
        assert_eq!(active[(7, 0)].fg, SHELL_ACTIVE_FG);
        // The gaviero tab is drawn in the palette as before.
        assert_eq!(active[(1, 0)].bg, HOST_PALETTE.inactive_bg);
    }

    #[test]
    fn each_bar_draws_in_its_own_palette() {
        let titles = vec![
            ("ab".to_string(), false, TabKind::Normal),
            ("c".to_string(), false, TabKind::Normal),
        ];
        let draw = |palette| {
            let bar = TabBar {
                titles: &titles,
                active: 0,
                closable: false,
                badges: None,
                palette,
            };
            let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
            bar.render(Rect::new(0, 0, 20, 1), &mut buf);
            buf
        };
        for p in [EDITOR_PALETTE, HOST_PALETTE] {
            let buf = draw(p);
            // " ab " active (0..4), separator at 4, " c " inactive (5..8).
            assert_eq!(buf[(1, 0)].bg, p.active_bg);
            assert_eq!(buf[(1, 0)].fg, p.active_fg);
            assert_eq!(buf[(4, 0)].fg, p.dim);
            assert_eq!(buf[(6, 0)].bg, p.inactive_bg);
            assert_eq!(buf[(19, 0)].bg, p.bar_bg);
        }
        // The two bars stack in the session host: no shared colour.
        assert_ne!(EDITOR_PALETTE.bar_bg, HOST_PALETTE.bar_bg);
        assert_ne!(EDITOR_PALETTE.active_bg, HOST_PALETTE.active_bg);
        assert_ne!(EDITOR_PALETTE.inactive_fg, HOST_PALETTE.inactive_fg);
    }
}
