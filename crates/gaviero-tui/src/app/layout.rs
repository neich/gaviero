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
    tracing::debug!("switch_layout: n={}, presets_len={}", n, app.layout_presets.len());
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
    app.panel_visible.side_panel = preset.side_panel_pct > 0;

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
            let ft_w = if preset.file_tree_pct > 0 { ft_w.max(1) } else { 0 };
            let sp_w = if preset.side_panel_pct > 0 { sp_w.max(1) } else { 0 };
            return (ft_w, sp_w);
        }
    }
    (app.file_tree_width, app.side_panel_width)
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
