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

pub(super) fn effective_panel_constraints(app: &App, total_width: u16) -> (u16, u16) {
    if let Some(idx) = app.active_preset {
        if let Some(preset) = app.layout_presets.get(idx) {
            let ft_w = if preset.file_tree_pct > 0 {
                (total_width as u32 * preset.file_tree_pct as u32 / 100) as u16
            } else {
                0
            };
            let sp_w = if preset.side_panel_pct > 0 {
                (total_width as u32 * preset.side_panel_pct as u32 / 100) as u16
            } else {
                0
            };
            let ft_w = if preset.file_tree_pct > 0 {
                ft_w.max(1)
            } else {
                0
            };
            let sp_w = if preset.side_panel_pct > 0 {
                sp_w.max(1)
            } else {
                0
            };
            return (ft_w, sp_w);
        }
    }
    (app.file_tree_width, app.side_panel_width)
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

fn apply_width_delta(current: u16, delta: i16, min: u16, max: u16) -> u16 {
    let next = (current as i16).saturating_add(delta);
    (next.clamp(min as i16, max as i16)) as u16
}

/// Cap side panels so the editor keeps [`theme::EDITOR_MIN_WIDTH`].
fn enforce_editor_min(app: &mut App, total: u16) {
    let min_editor = theme::EDITOR_MIN_WIDTH;
    let ft = if app.panel_visible.file_tree {
        app.file_tree_width
    } else {
        0
    };
    let sp = if app.panel_visible.side_panel {
        app.side_panel_width
    } else {
        0
    };
    let budget = total.saturating_sub(min_editor);
    if ft.saturating_add(sp) <= budget {
        return;
    }
    // Prefer shrinking the panel that was just grown is handled by callers;
    // here we just clamp the overflow, shrinking side first then tree.
    let overflow = ft.saturating_add(sp).saturating_sub(budget);
    if app.panel_visible.side_panel {
        let shrink = overflow.min(app.side_panel_width.saturating_sub(theme::SIDE_PANEL_MIN_WIDTH));
        app.side_panel_width -= shrink;
        let overflow = overflow.saturating_sub(shrink);
        if overflow > 0 && app.panel_visible.file_tree {
            let shrink =
                overflow.min(app.file_tree_width.saturating_sub(theme::FILE_TREE_MIN_WIDTH));
            app.file_tree_width -= shrink;
        }
    } else if app.panel_visible.file_tree {
        let shrink = overflow.min(app.file_tree_width.saturating_sub(theme::FILE_TREE_MIN_WIDTH));
        app.file_tree_width -= shrink;
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
pub(super) fn resize_horizontal(app: &mut App, delta_cols: i16) {
    if delta_cols == 0 {
        return;
    }
    materialize_preset_widths(app);
    let total = panels_row_width(app);
    let step = theme::PANEL_WIDTH_RESIZE_STEP as i16;
    let delta = if delta_cols > 0 { step } else { -step };

    match app.focus {
        Focus::FileTree if app.panel_visible.file_tree => {
            app.file_tree_width = apply_width_delta(
                app.file_tree_width,
                delta,
                theme::FILE_TREE_MIN_WIDTH,
                theme::FILE_TREE_MAX_WIDTH,
            );
        }
        Focus::SidePanel if app.panel_visible.side_panel => {
            // Spatial: Left expands chat into the editor; Right shrinks it.
            app.side_panel_width = apply_width_delta(
                app.side_panel_width,
                -delta,
                theme::SIDE_PANEL_MIN_WIDTH,
                theme::SIDE_PANEL_MAX_WIDTH,
            );
        }
        _ => {
            // Editor/Terminal, or a hidden focused side: grow the editor by
            // shrinking the neighbor on that side.
            if delta < 0 {
                if app.panel_visible.file_tree {
                    app.file_tree_width = apply_width_delta(
                        app.file_tree_width,
                        delta,
                        theme::FILE_TREE_MIN_WIDTH,
                        theme::FILE_TREE_MAX_WIDTH,
                    );
                } else if app.panel_visible.side_panel {
                    app.side_panel_width = apply_width_delta(
                        app.side_panel_width,
                        -delta,
                        theme::SIDE_PANEL_MIN_WIDTH,
                        theme::SIDE_PANEL_MAX_WIDTH,
                    );
                }
            } else if app.panel_visible.side_panel {
                app.side_panel_width = apply_width_delta(
                    app.side_panel_width,
                    -delta,
                    theme::SIDE_PANEL_MIN_WIDTH,
                    theme::SIDE_PANEL_MAX_WIDTH,
                );
            } else if app.panel_visible.file_tree {
                app.file_tree_width = apply_width_delta(
                    app.file_tree_width,
                    delta,
                    theme::FILE_TREE_MIN_WIDTH,
                    theme::FILE_TREE_MAX_WIDTH,
                );
            }
        }
    }

    enforce_editor_min(app, total);
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
            if let Some(arr) = obj.get(&key).and_then(|v| v.as_array()) {
                if arr.len() >= 3 {
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
        assert_eq!(preset.editor_pct > 0, false);
        assert!(preset.side_panel_pct > 0);
    }

    #[test]
    fn apply_width_delta_clamps_to_min_max() {
        assert_eq!(apply_width_delta(30, 5, 12, 80), 35);
        assert_eq!(apply_width_delta(14, -5, 12, 80), 12);
        assert_eq!(apply_width_delta(78, 5, 12, 80), 80);
    }
}
