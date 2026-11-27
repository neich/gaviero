use super::*;

pub(super) fn toggle_fullscreen(app: &mut App) {
    if app.fullscreen_panel.is_some() {
        app.fullscreen_panel = None;
    } else {
        app.fullscreen_panel = Some(app.focus);
    }
}

pub(super) fn switch_layout(app: &mut App, n: u8) {
    let idx = n as usize;
    tracing::debug!(
        "switch_layout: n={}, presets_len={}",
        n,
        app.layout_presets.len()
    );
    if idx >= app.layout_presets.len() {
        return;
    }

    if app.fullscreen_panel.is_some() {
        app.fullscreen_panel = None;
        app.pre_fullscreen = None;
    }

    let preset = &app.layout_presets[idx];
    app.active_preset = Some(idx);

    app.panel_visible.file_tree = preset.file_tree_pct > 0;
    app.panel_visible.editor = preset.editor_pct > 0;
    app.panel_visible.side_panel = preset.side_panel_pct > 0;

    if !app.panel_visible.editor && app.focus == Focus::Editor {
        app.focus = if app.panel_visible.side_panel {
            Focus::SidePanel
        } else if app.panel_visible.file_tree {
            Focus::FileTree
        } else {
            Focus::Editor
        };
    }

    let label = format!(
        "Layout {} (tree {}%  editor {}%  side {}%)",
        idx + 1,
        preset.file_tree_pct,
        preset.editor_pct,
        preset.side_panel_pct,
    );
    app.status_message = Some((label, std::time::Instant::now()));
}

/// A preset percentage of `total`, floored at one column whenever the
/// percentage is non-zero: a visible panel must receive at least one cell, but
/// a `0` percentage means the preset hides it and must stay `0`.
fn pct_of(total: u16, pct: u16) -> u16 {
    if pct == 0 {
        0
    } else {
        ((total as u32 * pct as u32 / 100) as u16).max(1)
    }
}

/// Which panel gives up columns first when an `(explorer, side)` pair does not
/// leave the editor its minimum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShrinkFirst {
    /// The side panel yields before the explorer: the render-time default, and
    /// the order that lets a growing explorer push the side panel.
    Side,
    /// The explorer yields before the side panel, so a growing side panel
    /// pushes the explorer instead of being stopped by it.
    Explorer,
}

/// Cap a `(explorer, side)` width pair so the editor keeps at least
/// [`theme::EDITOR_MIN_WIDTH`] columns of `total`.
///
/// The pair can overshoot from four directions: the user resizing on a wide
/// terminal, a hand-written `panels.*.width` default, a restored session whose
/// widths were chosen on a wider terminal, or a preset whose percentages sum
/// past 100. All four funnel through here, so the editor cannot be squeezed
/// out by geometry that no longer fits. Panels that are hidden arrive as `0`
/// and therefore claim no budget.
///
/// The side panel is shrunk first, then the explorer (see
/// [`fit_pair_to_editor_min`] for the general order).
fn clamp_pair_to_editor_min(explorer: u16, side: u16, total: u16) -> (u16, u16) {
    fit_pair_to_editor_min(explorer, side, total, ShrinkFirst::Side)
}

/// [`clamp_pair_to_editor_min`] with the shrink order chosen by the caller.
///
/// Each panel shrinks only down to the wider of its own minimum and the shrink
/// still needed, the `first` one before the other. A terminal too narrow for
/// even those floors is left alone — ratatui's constraint solver then has the
/// last word rather than this function producing a negative split.
fn fit_pair_to_editor_min(explorer: u16, side: u16, total: u16, first: ShrinkFirst) -> (u16, u16) {
    // The arithmetic runs in `u32`: `explorer + side` can exceed `u16::MAX`
    // for a pair read from a hand-written setting, and a `u16` running total
    // would wrap instead of reporting the overflow that needs removing.
    let budget = total.saturating_sub(theme::EDITOR_MIN_WIDTH) as u32;
    let mut overflow = (explorer as u32 + side as u32).saturating_sub(budget);
    if overflow == 0 {
        return (explorer, side);
    }

    let mut explorer = explorer as u32;
    let mut side = side as u32;
    let mut take = |width: &mut u32, floor: u16| {
        let shrink = overflow.min(width.saturating_sub(floor as u32));
        *width -= shrink;
        overflow -= shrink;
    };

    match first {
        ShrinkFirst::Side => {
            take(&mut side, theme::SIDE_PANEL_MIN_WIDTH);
            take(&mut explorer, theme::FILE_TREE_MIN_WIDTH);
        }
        ShrinkFirst::Explorer => {
            take(&mut explorer, theme::FILE_TREE_MIN_WIDTH);
            take(&mut side, theme::SIDE_PANEL_MIN_WIDTH);
        }
    }

    // Both only ever shrank from `u16` values, so they still fit.
    (explorer as u16, side as u16)
}

/// Floor a configured or stored explorer width at its minimum.
///
/// The one sanitiser for an explorer width crossing a persistence boundary: a
/// hand-written `panels.fileTree.width` seed and a width restored from a
/// `state.json` are the same problem, so both go through the same rule rather
/// than each inventing its own. There is deliberately no ceiling — how wide a
/// panel may be depends on the terminal it is drawn on, so the render path
/// ([`effective_panel_constraints`]) caps it against the editor minimum
/// instead of a fixed column count.
pub(super) fn clamp_file_tree_width(width: u16) -> u16 {
    width.max(theme::FILE_TREE_MIN_WIDTH)
}

/// Side-panel sibling of [`clamp_file_tree_width`].
pub(super) fn clamp_side_panel_width(width: u16) -> u16 {
    width.max(theme::SIDE_PANEL_MIN_WIDTH)
}

/// Clamp a configured or stored terminal split percentage into its legal range.
pub(super) fn clamp_terminal_split_percent(pct: u16) -> u16 {
    pct.clamp(theme::TERMINAL_MIN_PERCENT, theme::TERMINAL_MAX_PERCENT)
}

/// Initial `(explorer, side, terminal pct)` for a session with no state yet:
/// the sanitised `panels.*` settings, else the built-in defaults.
///
/// This is the *seed* layer of the three that decide panel geometry, and the
/// only one that reads configuration:
///
/// 1. the built-in defaults in [`theme`] — a workspace with nothing configured;
/// 2. these `panels.*` settings — the user's declared starting point;
/// 3. `state.json` — the live geometry the user left behind, which
///    [`super::session::restore_session`] layers on top and which wins.
///
/// Because (3) exists, a resize never rewrites these settings: they seed a
/// first run, or a run whose session was cleared. Widths are floored here at
/// their minimums; an oversized one (a hand-written `9999`) is capped to the
/// terminal at render time like any other width.
pub(super) fn seed_panel_geometry(workspace: &Workspace) -> (u16, u16, u16) {
    use gaviero_core::workspace::settings;

    let read = |key: &str, default: u16| -> u16 {
        workspace
            .resolve_setting(key, None)
            .as_u64()
            .unwrap_or(default as u64)
            .min(u16::MAX as u64) as u16
    };

    (
        clamp_file_tree_width(read(
            settings::FILE_TREE_WIDTH,
            theme::FILE_TREE_DEFAULT_WIDTH,
        )),
        clamp_side_panel_width(read(
            settings::SIDE_PANEL_WIDTH,
            theme::SIDE_PANEL_DEFAULT_WIDTH,
        )),
        clamp_terminal_split_percent(read(
            settings::TERMINAL_SPLIT_PERCENT,
            theme::TERMINAL_DEFAULT_PERCENT,
        )),
    )
}

/// The stored absolute column widths, with a hidden panel reading as `0`: its
/// width is remembered for when it is shown again, but a hidden panel must
/// never consume any of [`clamp_pair_to_editor_min`]'s budget.
fn stored_panel_widths(app: &App) -> (u16, u16) {
    (
        if app.panel_visible.file_tree {
            app.file_tree_width
        } else {
            0
        },
        if app.panel_visible.side_panel {
            app.side_panel_width
        } else {
            0
        },
    )
}

/// The panel widths before the editor-minimum cap: the active preset's
/// percentages, else the stored absolute columns.
fn raw_panel_widths(app: &App, total_width: u16) -> (u16, u16) {
    if let Some(preset) = app
        .active_preset
        .and_then(|idx| app.layout_presets.get(idx))
    {
        return (
            pct_of(total_width, preset.file_tree_pct),
            pct_of(total_width, preset.side_panel_pct),
        );
    }
    stored_panel_widths(app)
}

pub(super) fn effective_panel_constraints(app: &App, total_width: u16) -> (u16, u16) {
    let (explorer, side) = raw_panel_widths(app, total_width);
    clamp_pair_to_editor_min(explorer, side, total_width)
}

/// Total width available to the explorer/editor/side row (full terminal width).
fn panels_row_width(app: &App) -> u16 {
    app.layout
        .status_area
        .width
        .max(app.layout.tab_area.width)
        .max(1)
}

/// Leave a layout preset so absolute column widths become authoritative,
/// seeding them from the preset's current on-screen sizes.
fn materialize_preset_widths(app: &mut App) {
    if app.active_preset.is_none() {
        return;
    }
    let total = panels_row_width(app);
    let (ft, sp) = effective_panel_constraints(app, total);
    if app.panel_visible.file_tree {
        app.file_tree_width = ft.max(theme::FILE_TREE_MIN_WIDTH);
    }
    if app.panel_visible.side_panel {
        app.side_panel_width = sp.max(theme::SIDE_PANEL_MIN_WIDTH);
    }
    app.active_preset = None;
}

/// `current + delta`, floored at `min`. No ceiling: the terminal is the limit,
/// and [`enforce_editor_min`] applies it after the step.
fn apply_width_delta(current: u16, delta: i16, min: u16) -> u16 {
    (current as i32 + delta as i32).clamp(min as i32, u16::MAX as i32) as u16
}

/// Write the editor-minimum cap back into `App`, so the *stored* widths are
/// the ones on screen.
///
/// Sibling of the read-side cap in [`effective_panel_constraints`]: this one
/// stops a resize at the point the editor and the opposite panel are both at
/// their minimums, while that one keeps a stale stored pair from being
/// honoured on a narrower terminal than the one it was chosen on. `first` is
/// the panel that yields first — never the one being grown.
fn enforce_editor_min(app: &mut App, total: u16, first: ShrinkFirst) {
    let (stored_explorer, stored_side) = stored_panel_widths(app);
    let (explorer, side) = fit_pair_to_editor_min(stored_explorer, stored_side, total, first);
    if app.panel_visible.file_tree {
        app.file_tree_width = explorer;
    }
    if app.panel_visible.side_panel {
        app.side_panel_width = side;
    }
}

/// Resize explorer / side / editor widths.
///
/// `delta_cols > 0` is Ctrl+Alt+Right; `< 0` is Ctrl+Alt+Left.
///
/// - Focus explorer → Right grows, Left shrinks (toward the right edge)
/// - Focus side panel → Left grows, Right shrinks (toward the left into the editor)
/// - Focus editor/terminal → Left takes space from explorer, Right from side
///   (both grow the editor). Focus a side panel and press Left to reclaim width.
///
/// No panel has a fixed maximum. A growing explorer or side panel first takes
/// the editor's columns down to [`theme::EDITOR_MIN_WIDTH`], then pushes the
/// opposite panel down to its own minimum; only then does it stop.
pub(super) fn resize_horizontal(app: &mut App, delta_cols: i16) {
    if delta_cols == 0 {
        return;
    }
    materialize_preset_widths(app);
    let total = panels_row_width(app);
    // Step from what is on screen. A width stored on a wider terminal (or
    // hand-written) is wider than the one drawn, and a step taken from it
    // would change nothing visible.
    enforce_editor_min(app, total, ShrinkFirst::Side);
    let step = theme::PANEL_WIDTH_RESIZE_STEP as i16;
    let delta = if delta_cols > 0 { step } else { -step };

    match app.focus {
        Focus::FileTree if app.panel_visible.file_tree => {
            app.file_tree_width =
                apply_width_delta(app.file_tree_width, delta, theme::FILE_TREE_MIN_WIDTH);
        }
        Focus::SidePanel if app.panel_visible.side_panel => {
            // Spatial: Left expands chat into the editor; Right shrinks it.
            app.side_panel_width =
                apply_width_delta(app.side_panel_width, -delta, theme::SIDE_PANEL_MIN_WIDTH);
        }
        _ => {
            // Editor/Terminal, or a hidden focused side: grow the editor by
            // shrinking the neighbor on that side.
            if delta < 0 {
                if app.panel_visible.file_tree {
                    app.file_tree_width =
                        apply_width_delta(app.file_tree_width, delta, theme::FILE_TREE_MIN_WIDTH);
                } else if app.panel_visible.side_panel {
                    app.side_panel_width = apply_width_delta(
                        app.side_panel_width,
                        -delta,
                        theme::SIDE_PANEL_MIN_WIDTH,
                    );
                }
            } else if app.panel_visible.side_panel {
                app.side_panel_width =
                    apply_width_delta(app.side_panel_width, -delta, theme::SIDE_PANEL_MIN_WIDTH);
            } else if app.panel_visible.file_tree {
                app.file_tree_width =
                    apply_width_delta(app.file_tree_width, delta, theme::FILE_TREE_MIN_WIDTH);
            }
        }
    }

    // The focused panel keeps its new width while anything else can give:
    // the side panel pushes the explorer, every other case the reverse.
    // Shrinking steps never overflow, so the order only matters when growing.
    let first = match app.focus {
        Focus::SidePanel if app.panel_visible.side_panel => ShrinkFirst::Explorer,
        _ => ShrinkFirst::Side,
    };
    enforce_editor_min(app, total, first);
}

pub(super) fn parse_layout_presets(workspace: &Workspace) -> Vec<LayoutPreset> {
    const DEFAULTS: &[(u16, u16, u16)] = &[(15, 60, 25), (15, 40, 45), (0, 100, 0), (0, 60, 40)];

    let val = workspace.resolve_setting("panels.layouts", None);
    tracing::info!("Layout presets setting: {}", val);
    let mut presets: Vec<LayoutPreset> = DEFAULTS
        .iter()
        .map(|&(ft, ed, sp)| LayoutPreset {
            file_tree_pct: ft,
            editor_pct: ed,
            side_panel_pct: sp,
        })
        .collect();

    if let Some(obj) = val.as_object() {
        for k in 1..=9u8 {
            let key = k.to_string();
            if let Some(arr) = obj.get(&key).and_then(|v| v.as_array())
                && arr.len() >= 3
            {
                let ft = arr[0].as_u64().unwrap_or(0) as u16;
                let ed = arr[1].as_u64().unwrap_or(100) as u16;
                let sp = arr[2].as_u64().unwrap_or(0) as u16;
                let idx = (k - 1) as usize;
                while presets.len() <= idx {
                    presets.push(LayoutPreset {
                        file_tree_pct: 0,
                        editor_pct: 100,
                        side_panel_pct: 0,
                    });
                }
                presets[idx] = LayoutPreset {
                    file_tree_pct: ft,
                    editor_pct: ed,
                    side_panel_pct: sp,
                };
            }
        }
    }

    presets
}

#[cfg(test)]
mod tests {
    use super::*;
    use gaviero_core::workspace::Workspace;

    #[test]
    fn layout_preset_zero_editor_pct_hides_editor() {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join(".gaviero");
        std::fs::create_dir_all(&settings).unwrap();
        std::fs::write(
            settings.join("settings.json"),
            r#"{"panels":{"layouts":{"3":[20,0,80]}}}"#,
        )
        .unwrap();

        let ws = Workspace::single_folder(dir.path().to_path_buf());
        let presets = parse_layout_presets(&ws);
        let preset = &presets[2];
        assert_eq!(preset.file_tree_pct, 20);
        assert_eq!(preset.editor_pct, 0);
        assert_eq!(preset.side_panel_pct, 80);
        assert!(preset.file_tree_pct > 0);
        assert!(!(preset.editor_pct > 0));
        assert!(preset.side_panel_pct > 0);
    }

    #[test]
    fn apply_width_delta_floors_at_min_and_has_no_ceiling() {
        assert_eq!(apply_width_delta(30, 5, 12), 35);
        assert_eq!(apply_width_delta(14, -5, 12), 12);
        // The old fixed maximums (80 / 120) are gone.
        assert_eq!(apply_width_delta(118, 5, 20), 123);
        // A hand-written width near `u16::MAX` saturates instead of wrapping.
        assert_eq!(apply_width_delta(u16::MAX - 2, 5, 12), u16::MAX);
    }

    #[test]
    fn fit_pair_shrinks_the_explorer_first_when_asked() {
        // total 100 -> budget 80; 50+40 overshoots by 10, taken from the explorer.
        assert_eq!(
            fit_pair_to_editor_min(50, 40, 100, ShrinkFirst::Explorer),
            (40, 40)
        );
        // Once the explorer is at its floor the rest comes out of the side panel.
        assert_eq!(
            fit_pair_to_editor_min(30, 80, 100, ShrinkFirst::Explorer),
            (12, 68)
        );
    }

    /// An app with explorer and side panel visible on a `total`-column row.
    fn resize_app(total: u16) -> (tempfile::TempDir, App) {
        let (dir, ws) = workspace_with_settings("{}");
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(ws, tx);
        app.active_preset = None;
        app.panel_visible.file_tree = true;
        app.panel_visible.editor = true;
        app.panel_visible.side_panel = true;
        app.file_tree_width = 30;
        app.side_panel_width = 40;
        app.layout.status_area = ratatui::layout::Rect::new(0, 0, total, 1);
        (dir, app)
    }

    #[test]
    fn side_panel_grows_past_the_old_fixed_maximum_then_pushes_the_explorer() {
        let (_dir, mut app) = resize_app(300);
        app.focus = Focus::SidePanel;
        for _ in 0..100 {
            resize_horizontal(&mut app, -1);
        }
        // Editor and explorer both at their floors; the side panel has the rest.
        assert_eq!(app.file_tree_width, theme::FILE_TREE_MIN_WIDTH);
        assert_eq!(
            app.side_panel_width,
            300 - theme::EDITOR_MIN_WIDTH - theme::FILE_TREE_MIN_WIDTH
        );
    }

    #[test]
    fn explorer_grows_past_the_old_fixed_maximum_then_pushes_the_side_panel() {
        let (_dir, mut app) = resize_app(300);
        app.focus = Focus::FileTree;
        for _ in 0..100 {
            resize_horizontal(&mut app, 1);
        }
        assert_eq!(app.side_panel_width, theme::SIDE_PANEL_MIN_WIDTH);
        assert_eq!(
            app.file_tree_width,
            300 - theme::EDITOR_MIN_WIDTH - theme::SIDE_PANEL_MIN_WIDTH
        );
    }

    #[test]
    fn editor_grows_until_both_neighbours_are_at_their_floors() {
        let (_dir, mut app) = resize_app(300);
        app.focus = Focus::Editor;
        for _ in 0..100 {
            resize_horizontal(&mut app, -1);
            resize_horizontal(&mut app, 1);
        }
        assert_eq!(app.file_tree_width, theme::FILE_TREE_MIN_WIDTH);
        assert_eq!(app.side_panel_width, theme::SIDE_PANEL_MIN_WIDTH);
    }

    #[test]
    fn a_step_starts_from_the_width_on_screen_not_a_stale_stored_one() {
        // Stored on a wider terminal: 250 columns cannot fit a 150-column row,
        // which draws the side panel at 150 - 20 - 30 = 100.
        let (_dir, mut app) = resize_app(150);
        app.side_panel_width = 250;
        app.focus = Focus::SidePanel;
        resize_horizontal(&mut app, 1);
        assert_eq!(app.side_panel_width, 100 - theme::PANEL_WIDTH_RESIZE_STEP);
        assert_eq!(app.file_tree_width, 30);
    }

    #[test]
    fn pct_of_floors_visible_panels_at_one_column_and_keeps_hidden_at_zero() {
        assert_eq!(pct_of(200, 15), 30);
        // 15% of a 4-column row rounds to 0, but a visible panel must still
        // get a cell rather than collapsing to the same width as a hidden one.
        assert_eq!(pct_of(4, 15), 1);
        // `0` is the preset's way of saying "hidden": it must stay 0, or the
        // floor above would resurrect a panel the preset switched off.
        assert_eq!(pct_of(200, 0), 0);
    }

    #[test]
    fn clamp_pair_keeps_the_editor_minimum_by_shrinking_the_side_panel_first() {
        // total 100, EDITOR_MIN_WIDTH 20 -> budget 80; 50+40 overshoots by 10.
        assert_eq!(clamp_pair_to_editor_min(50, 40, 100), (50, 30));
    }

    #[test]
    fn clamp_pair_falls_through_to_the_explorer_once_the_side_panel_is_at_its_floor() {
        // The side panel is already at SIDE_PANEL_MIN_WIDTH, so the remaining
        // overflow has to come out of the explorer.
        assert_eq!(clamp_pair_to_editor_min(70, 20, 100), (60, 20));
    }

    #[test]
    fn clamp_pair_leaves_a_fitting_pair_untouched() {
        assert_eq!(clamp_pair_to_editor_min(30, 40, 200), (30, 40));
    }

    #[test]
    fn hidden_panels_claim_no_budget() {
        assert_eq!(clamp_pair_to_editor_min(0, 0, 100), (0, 0));
    }

    #[test]
    fn clamp_pair_gives_up_at_the_floors_on_a_terminal_narrower_than_the_editor_min() {
        // total 10 < EDITOR_MIN_WIDTH 20 -> budget 0. Both panels stop at
        // their own minimums instead of the split going negative; ratatui's
        // constraint solver has the last word from there.
        assert_eq!(clamp_pair_to_editor_min(40, 40, 10), (12, 20));
    }

    #[test]
    fn clamp_pair_survives_a_pair_whose_sum_exceeds_u16() {
        // Two widths at `u16::MAX` sum past `u16`, so a `u16` running total
        // would wrap and remove the wrong number of columns. The result must
        // land exactly on the budget: 60 + 20 == 100 - EDITOR_MIN_WIDTH.
        let (explorer, side) = clamp_pair_to_editor_min(u16::MAX, u16::MAX, 100);
        assert_eq!((explorer, side), (60, 20));
        assert_eq!(explorer as u32 + side as u32, 80);
    }

    /// A workspace whose `.gaviero/settings.json` holds `json`.
    fn workspace_with_settings(json: &str) -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join(".gaviero");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join("settings.json"), json).unwrap();
        let ws = Workspace::single_folder(dir.path().to_path_buf());
        (dir, ws)
    }

    #[test]
    fn seed_geometry_falls_back_to_the_built_in_defaults() {
        let (_dir, ws) = workspace_with_settings("{}");
        assert_eq!(
            seed_panel_geometry(&ws),
            (
                theme::FILE_TREE_DEFAULT_WIDTH,
                theme::SIDE_PANEL_DEFAULT_WIDTH,
                theme::TERMINAL_DEFAULT_PERCENT,
            )
        );
    }

    #[test]
    fn seed_geometry_keeps_in_range_settings_verbatim() {
        let (_dir, ws) = workspace_with_settings(
            r#"{"panels":{"fileTree":{"width":42},"sidePanel":{"width":51},"terminal":{"splitPercent":65}}}"#,
        );
        assert_eq!(seed_panel_geometry(&ws), (42, 51, 65));
    }

    #[test]
    fn seed_geometry_sanitises_out_of_range_settings() {
        // Widths are floored at their minimums but have no fixed ceiling — the
        // render path caps them to the terminal — and the terminal percentage
        // must honour the same bounds a resize clamps to.
        let (_dir, ws) = workspace_with_settings(
            r#"{"panels":{"fileTree":{"width":9999},"sidePanel":{"width":0},"terminal":{"splitPercent":500}}}"#,
        );
        assert_eq!(
            seed_panel_geometry(&ws),
            (9999, theme::SIDE_PANEL_MIN_WIDTH, theme::TERMINAL_MAX_PERCENT)
        );
    }
}
