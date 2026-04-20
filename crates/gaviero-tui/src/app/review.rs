use super::*;

pub(super) fn handle_review_action(app: &mut App, action: &Action) -> bool {
    let review = match &mut app.diff_review {
        Some(r) => r,
        None => return false,
    };
    let is_interactive = review.is_interactive();

    match action {
        Action::InsertChar(']') => {
            review.pending_bracket = Some(']');
            true
        }
        Action::InsertChar('[') => {
            review.pending_bracket = Some('[');
            true
        }
        Action::InsertChar('h') if review.pending_bracket.is_some() => {
            let bracket = review.pending_bracket.take().unwrap();
            match bracket {
                ']' => review.next_hunk(),
                '[' => review.prev_hunk(),
                _ => {}
            }
            true
        }
        Action::InsertChar('a') if is_interactive => {
            review.pending_bracket = None;
            let idx = review.current_hunk;
            review.accept_hunk(idx);
            true
        }
        Action::InsertChar('r') if is_interactive => {
            review.pending_bracket = None;
            let idx = review.current_hunk;
            review.reject_hunk(idx);
            true
        }
        Action::InsertChar('A') if is_interactive => {
            review.pending_bracket = None;
            review.accept_all();
            true
        }
        Action::InsertChar('R') if is_interactive => {
            review.pending_bracket = None;
            review.reject_all();
            true
        }
        Action::InsertChar('f') if is_interactive => {
            finalize_current_review(app);
            true
        }
        Action::InsertChar('q') => {
            app.diff_review = None;
            true
        }
        Action::Quit => {
            // Ctrl+Q routes through the quit-confirm dialog; it sees the
            // pending review and offers accept-and-quit / reject-and-quit /
            // cancel. Plain `q` keeps the "dismiss review" behavior above so
            // muscle memory isn't broken.
            super::session::try_quit(app);
            true
        }
        Action::CursorDown | Action::InsertChar('j') => {
            review.pending_bracket = None;
            review.scroll_top += 1;
            true
        }
        Action::CursorUp | Action::InsertChar('k') => {
            review.pending_bracket = None;
            review.scroll_top = review.scroll_top.saturating_sub(1);
            true
        }
        Action::InsertChar('J') | Action::PageDown => {
            review.pending_bracket = None;
            review.scroll_top += theme::DIFF_PAGE_SCROLL;
            true
        }
        Action::InsertChar('K') | Action::PageUp => {
            review.pending_bracket = None;
            review.scroll_top = review.scroll_top.saturating_sub(theme::DIFF_PAGE_SCROLL);
            true
        }
        _ => {
            if review.pending_bracket.is_some() {
                review.pending_bracket = None;
            }
            false
        }
    }
}

/// Accept every pending hunk, write the finalized content to disk, reload any
/// open buffer pointing at that file, and finalize the write gate. Used by
/// the `f` key in review and by the quit-confirm "accept & quit" path.
pub(super) fn finalize_current_review(app: &mut App) {
    let review = match app.diff_review.take() {
        Some(r) => r,
        None => return,
    };
    let mut proposal = review.proposal;
    for hunk in &mut proposal.structural_hunks {
        if hunk.status == gaviero_core::types::HunkStatus::Pending {
            hunk.status = gaviero_core::types::HunkStatus::Accepted;
        }
    }
    let content = gaviero_core::write_gate::assemble_final_content(&proposal);
    let path = proposal.file_path.clone();

    if let Err(e) = std::fs::write(&path, &content) {
        tracing::error!("Failed to write finalized file {}: {}", path.display(), e);
    } else {
        for buf in &mut app.buffers {
            if buf.path.as_deref() == Some(path.as_path()) {
                let _ = buf.reload();
            }
        }
    }

    let wg = app.write_gate.clone();
    let id = proposal.id;
    tokio::spawn(async move {
        let mut gate = wg.lock().await;
        gate.finalize(id);
    });
}

pub(super) fn enter_review_mode(app: &mut App, proposal: WriteProposal, source: DiffSource) {
    if app.diff_review.is_some() {
        return;
    }
    let path = proposal.file_path.clone();
    app.open_file(&path);
    app.focus = Focus::Editor;
    app.diff_review = Some(DiffReviewState::new(proposal, source));
}

pub(super) fn enter_batch_review(app: &mut App, proposals: Vec<WriteProposal>) {
    if proposals.is_empty() {
        return;
    }

    let review_proposals: Vec<ReviewProposal> = proposals
        .into_iter()
        .map(|p| {
            let old_lines = p.original_content.lines().count();
            let new_lines = p.proposed_content.lines().count();
            let additions = new_lines.saturating_sub(old_lines)
                + p.structural_hunks
                    .iter()
                    .map(|h| h.diff_hunk.proposed_text.lines().count())
                    .sum::<usize>()
                    .min(new_lines);
            let deletions = old_lines.saturating_sub(new_lines)
                + p.structural_hunks
                    .iter()
                    .map(|h| h.diff_hunk.original_text.lines().count())
                    .sum::<usize>()
                    .min(old_lines);
            let old_content = if p.original_content.is_empty() {
                None
            } else {
                Some(p.original_content.clone())
            };
            ReviewProposal {
                path: p.file_path,
                old_content,
                new_content: p.proposed_content,
                additions,
                deletions,
            }
        })
        .collect();

    let initial_diff = if let Some(p) = review_proposals.first() {
        let old_lines: Vec<&str> = p.old_content.as_deref().unwrap_or("").lines().collect();
        let new_lines: Vec<&str> = p.new_content.lines().collect();
        build_simple_diff(&old_lines, &new_lines)
    } else {
        Vec::new()
    };

    app.batch_review = Some(BatchReviewState {
        proposals: review_proposals,
        selected_index: 0,
        scroll_offset: 0,
        diff_scroll: 0,
        cached_diff: initial_diff,
        cached_diff_index: 0,
    });
    app.left_panel = LeftPanelMode::Review;
    app.panel_visible.file_tree = true;
    app.focus = Focus::FileTree;
}

pub(super) fn handle_batch_review_action(app: &mut App, action: &Action) -> bool {
    if app.batch_review.is_none() {
        return false;
    }

    match action {
        Action::InsertChar('f') => {
            app.finalize_batch_review();
            true
        }
        Action::Quit => {
            app.cancel_batch_review();
            true
        }
        Action::CursorDown | Action::InsertChar('j') => {
            let br = app.batch_review.as_mut().unwrap();
            if app.focus == Focus::FileTree {
                if br.selected_index + 1 < br.proposals.len() {
                    br.selected_index += 1;
                    br.diff_scroll = 0;
                }
            } else {
                br.diff_scroll += 1;
            }
            true
        }
        Action::CursorUp | Action::InsertChar('k') => {
            let br = app.batch_review.as_mut().unwrap();
            if app.focus == Focus::FileTree {
                if br.selected_index > 0 {
                    br.selected_index -= 1;
                    br.diff_scroll = 0;
                }
            } else {
                br.diff_scroll = br.diff_scroll.saturating_sub(1);
            }
            true
        }
        Action::InsertChar('J') | Action::PageDown => {
            let br = app.batch_review.as_mut().unwrap();
            br.diff_scroll += theme::DIFF_PAGE_SCROLL;
            true
        }
        Action::InsertChar('K') | Action::PageUp => {
            let br = app.batch_review.as_mut().unwrap();
            br.diff_scroll = br.diff_scroll.saturating_sub(theme::DIFF_PAGE_SCROLL);
            true
        }
        Action::InsertChar('a') => {
            let br = app.batch_review.as_mut().unwrap();
            if let Some(proposal) = br.proposals.get(br.selected_index) {
                let path = proposal.path.clone();
                let content = proposal.new_content.clone();
                if let Err(e) = std::fs::write(&path, &content) {
                    tracing::error!("Failed to write {}: {}", path.display(), e);
                } else {
                    for buf in &mut app.buffers {
                        if buf.path.as_deref() == Some(path.as_path()) {
                            let _ = buf.reload();
                        }
                    }
                }
                let br = app.batch_review.as_mut().unwrap();
                br.proposals.remove(br.selected_index);
                if br.selected_index >= br.proposals.len() && br.selected_index > 0 {
                    br.selected_index -= 1;
                }
                br.diff_scroll = 0;
                br.cached_diff = Vec::new();
                br.cached_diff_index = usize::MAX;
                if br.proposals.is_empty() {
                    app.batch_review = None;
                    app.left_panel = LeftPanelMode::FileTree;
                    app.status_message =
                        Some(("All files reviewed".to_string(), std::time::Instant::now()));
                }
            }
            true
        }
        Action::InsertChar('r') => {
            let br = app.batch_review.as_mut().unwrap();
            if !br.proposals.is_empty() {
                br.proposals.remove(br.selected_index);
                if br.selected_index >= br.proposals.len() && br.selected_index > 0 {
                    br.selected_index -= 1;
                }
                br.diff_scroll = 0;
                br.cached_diff = Vec::new();
                br.cached_diff_index = usize::MAX;
                if br.proposals.is_empty() {
                    app.batch_review = None;
                    app.left_panel = LeftPanelMode::FileTree;
                    app.status_message = Some((
                        "All files reviewed — no changes applied".to_string(),
                        std::time::Instant::now(),
                    ));
                }
            }
            true
        }
        _ => false,
    }
}

pub(super) fn finalize_batch_review(app: &mut App) {
    let br = match app.batch_review.take() {
        Some(r) => r,
        None => return,
    };

    let mut written = Vec::new();
    for proposal in &br.proposals {
        if let Err(e) = std::fs::write(&proposal.path, &proposal.new_content) {
            tracing::error!("Failed to write {}: {}", proposal.path.display(), e);
        } else {
            written.push(proposal.path.clone());
        }
    }

    for path in &written {
        for buf in &mut app.buffers {
            if buf.path.as_deref() == Some(path.as_path()) {
                let _ = buf.reload();
            }
        }
    }

    app.left_panel = LeftPanelMode::FileTree;
    app.status_message = Some((
        format!("{} file(s) written", written.len()),
        std::time::Instant::now(),
    ));
}

pub(super) fn cancel_batch_review(app: &mut App) {
    let n = app
        .batch_review
        .as_ref()
        .map(|br| br.proposals.len())
        .unwrap_or(0);
    app.batch_review = None;
    app.left_panel = LeftPanelMode::FileTree;
    app.status_message = Some((
        format!("Review discarded — {} file(s) not written", n),
        std::time::Instant::now(),
    ));
}

pub(super) fn render_review_file_list(app: &mut App, frame: &mut Frame, area: Rect, focused: bool) {
    use ratatui::style::Modifier;
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, Borders, Widget};

    let border_style = if focused {
        Style::default().fg(theme::FOCUS_BORDER)
    } else {
        Style::default().fg(theme::TEXT_DIM)
    };

    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(border_style);
    let inner = block.inner(area);
    block.render(area, frame.buffer_mut());

    let br = match &mut app.batch_review {
        Some(r) => r,
        None => return,
    };

    let visible = inner.height as usize;

    if visible > 0 {
        if br.selected_index < br.scroll_offset {
            br.scroll_offset = br.selected_index;
        } else if br.selected_index >= br.scroll_offset + visible {
            br.scroll_offset = br.selected_index - visible + 1;
        }
    }

    let scroll = br.scroll_offset;

    for (row, (i, proposal)) in br
        .proposals
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible)
        .enumerate()
    {
        let y = inner.y + row as u16;
        if y >= inner.bottom() {
            break;
        }

        let is_selected = i == br.selected_index;
        let filename = proposal
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");

        let name_style = if is_selected {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
                .bg(theme::SELECTION_BG)
        } else {
            Style::default().fg(theme::TEXT_FG)
        };

        let adds = format!(" +{}", proposal.additions);
        let dels = format!(" -{}", proposal.deletions);

        let spans = vec![
            Span::styled(format!(" {}", filename), name_style),
            Span::styled(adds, Style::default().fg(theme::SUCCESS)),
            Span::styled(dels, Style::default().fg(theme::ERROR)),
        ];

        let line = Line::from(spans);
        let line_area = Rect {
            x: inner.x,
            y,
            width: inner.width,
            height: 1,
        };

        if is_selected {
            for x in inner.x..inner.right() {
                frame.buffer_mut()[(x, y)].set_bg(theme::SELECTION_BG);
            }
        }

        Widget::render(line, line_area, frame.buffer_mut());
    }

    crate::widgets::scrollbar::render_scrollbar(
        inner,
        frame.buffer_mut(),
        br.proposals.len(),
        visible,
        scroll,
    );
}

pub(super) fn render_batch_review_diff(app: &mut App, frame: &mut Frame, area: Rect) {
    use ratatui::style::Modifier;

    let br = match &mut app.batch_review {
        Some(r) => r,
        None => return,
    };

    let proposal = match br.proposals.get(br.selected_index) {
        Some(p) => p,
        None => return,
    };

    if br.cached_diff_index != br.selected_index {
        let old_lines: Vec<&str> = proposal
            .old_content
            .as_deref()
            .unwrap_or("")
            .lines()
            .collect();
        let new_lines: Vec<&str> = proposal.new_content.lines().collect();
        br.cached_diff = build_simple_diff(&old_lines, &new_lines);
        br.cached_diff_index = br.selected_index;
    }

    let diff_lines = &br.cached_diff;
    let gutter_w = theme::DIFF_GUTTER_WIDTH;
    let max_scroll = diff_lines.len().saturating_sub(1);
    if br.diff_scroll > max_scroll {
        br.diff_scroll = max_scroll;
    }
    let scroll = br.diff_scroll;

    let header = format!(" {} ", proposal.path.display());
    let header_style = Style::default()
        .fg(theme::FOCUS_BORDER)
        .add_modifier(Modifier::BOLD);
    for (i, ch) in header.chars().enumerate() {
        let x = area.x + i as u16;
        if x < area.right() && area.y < area.bottom() {
            frame.buffer_mut()[(x, area.y)]
                .set_char(ch)
                .set_style(header_style);
        }
    }

    let content_height = area.height.saturating_sub(1) as usize;
    for row in 0..content_height {
        let line_idx = scroll + row;
        if line_idx >= diff_lines.len() {
            break;
        }

        let y = area.y + 1 + row as u16;
        if y >= area.bottom() {
            break;
        }

        let (kind, text) = &diff_lines[line_idx];

        let (gutter_str, gutter_style, line_style) = match kind {
            DiffKind::Added => (
                " + │ ",
                Style::default()
                    .fg(theme::SUCCESS)
                    .add_modifier(Modifier::BOLD),
                Style::default()
                    .fg(theme::SUCCESS)
                    .bg(theme::DIFF_ADD_LINE_BG),
            ),
            DiffKind::Removed => (
                " - │ ",
                Style::default()
                    .fg(theme::ERROR)
                    .add_modifier(Modifier::BOLD),
                Style::default()
                    .fg(theme::ERROR)
                    .bg(theme::DIFF_REM_LINE_BG),
            ),
            DiffKind::Context => (
                "   │ ",
                Style::default().fg(theme::TEXT_DIM),
                Style::default().fg(theme::TEXT_FG),
            ),
        };

        for (i, ch) in gutter_str.chars().enumerate() {
            let x = area.x + i as u16;
            if x < area.right() {
                frame.buffer_mut()[(x, y)]
                    .set_char(ch)
                    .set_style(gutter_style);
            }
        }

        for (i, ch) in text.chars().enumerate() {
            let x = area.x + gutter_w + i as u16;
            if x < area.right() {
                frame.buffer_mut()[(x, y)]
                    .set_char(ch)
                    .set_style(line_style);
            }
        }
    }
}

pub(super) fn refresh_git_changes(app: &mut App) {
    let root = match app.workspace.roots().first() {
        Some(r) => r.to_path_buf(),
        None => {
            app.changes_state = None;
            return;
        }
    };
    let repo = match gaviero_core::git::GitRepo::open(&root) {
        Ok(r) => r,
        Err(_) => {
            app.changes_state = None;
            return;
        }
    };
    let entries = match repo.file_status() {
        Ok(e) => e,
        Err(_) => {
            app.changes_state = None;
            return;
        }
    };

    let workdir = repo.workdir().unwrap_or(&root).to_path_buf();

    let git_entries: Vec<ChangesEntry> = entries
        .into_iter()
        .filter(|e| !e.staged)
        .map(|e| {
            let abs_path = workdir.join(&e.path);
            let old_content = repo.head_file_content(&e.path).unwrap_or_default();
            let new_content = std::fs::read_to_string(&abs_path).unwrap_or_default();

            let old_lines: Vec<&str> = old_content.lines().collect();
            let new_lines: Vec<&str> = new_content.lines().collect();
            let diff = build_simple_diff(&old_lines, &new_lines);
            let additions = diff
                .iter()
                .filter(|(k, _)| matches!(k, DiffKind::Added))
                .count();
            let deletions = diff
                .iter()
                .filter(|(k, _)| matches!(k, DiffKind::Removed))
                .count();

            ChangesEntry {
                rel_path: e.path,
                abs_path,
                status_char: e.status.marker(),
                additions,
                deletions,
            }
        })
        .collect();

    if git_entries.is_empty() {
        app.changes_state = None;
        return;
    }

    app.changes_state = Some(ChangesState {
        entries: git_entries,
        selected_index: 0,
        scroll_offset: 0,
        diff_scroll: 0,
        cached_diff: Vec::new(),
        cached_diff_index: usize::MAX,
    });
}

pub(super) fn handle_changes_action(app: &mut App, action: &Action) -> bool {
    let cs = match &mut app.changes_state {
        Some(s) => s,
        None => return false,
    };

    match action {
        Action::CursorDown | Action::InsertChar('j') => {
            if cs.selected_index + 1 < cs.entries.len() {
                cs.selected_index += 1;
                cs.diff_scroll = 0;
            }
            true
        }
        Action::CursorUp | Action::InsertChar('k') => {
            if cs.selected_index > 0 {
                cs.selected_index -= 1;
                cs.diff_scroll = 0;
            }
            true
        }
        Action::InsertChar('J') | Action::PageDown => {
            cs.diff_scroll += theme::DIFF_PAGE_SCROLL;
            true
        }
        Action::InsertChar('K') | Action::PageUp => {
            cs.diff_scroll = cs.diff_scroll.saturating_sub(theme::DIFF_PAGE_SCROLL);
            true
        }
        Action::Enter => {
            if let Some(entry) = cs.entries.get(cs.selected_index) {
                let path = entry.abs_path.clone();
                app.open_file(&path);
                app.left_panel = LeftPanelMode::FileTree;
                app.focus = Focus::Editor;
            }
            true
        }
        Action::Quit => {
            app.left_panel = LeftPanelMode::FileTree;
            app.changes_state = None;
            true
        }
        Action::InsertChar('R') => {
            app.refresh_git_changes();
            true
        }
        _ => false,
    }
}

pub(super) fn render_changes_file_list(
    app: &mut App,
    frame: &mut Frame,
    area: Rect,
    focused: bool,
) {
    use ratatui::style::Modifier;
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, Borders, Widget};

    let border_style = if focused {
        Style::default().fg(theme::FOCUS_BORDER)
    } else {
        Style::default().fg(theme::TEXT_DIM)
    };

    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(border_style);
    let inner = block.inner(area);
    block.render(area, frame.buffer_mut());

    let cs = match &mut app.changes_state {
        Some(s) => s,
        None => {
            let msg = " No changes";
            let y = inner.y;
            if y < inner.bottom() {
                for (i, ch) in msg.chars().enumerate() {
                    let x = inner.x + i as u16;
                    if x < inner.right() {
                        frame.buffer_mut()[(x, y)]
                            .set_char(ch)
                            .set_style(Style::default().fg(theme::TEXT_DIM));
                    }
                }
            }
            return;
        }
    };

    let visible = inner.height as usize;

    if visible > 0 {
        if cs.selected_index < cs.scroll_offset {
            cs.scroll_offset = cs.selected_index;
        } else if cs.selected_index >= cs.scroll_offset + visible {
            cs.scroll_offset = cs.selected_index - visible + 1;
        }
    }

    let scroll = cs.scroll_offset;

    for (row, (i, entry)) in cs
        .entries
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible)
        .enumerate()
    {
        let y = inner.y + row as u16;
        if y >= inner.bottom() {
            break;
        }

        let is_selected = i == cs.selected_index;
        let filename = std::path::Path::new(&entry.rel_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?");

        let status_style = match entry.status_char {
            'M' => Style::default().fg(theme::WARNING),
            'A' | '?' => Style::default().fg(theme::SUCCESS),
            'D' => Style::default().fg(theme::ERROR),
            _ => Style::default().fg(theme::TEXT_DIM),
        };

        let name_style = if is_selected {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
                .bg(theme::SELECTION_BG)
        } else {
            Style::default().fg(theme::TEXT_FG)
        };

        let adds = format!(" +{}", entry.additions);
        let dels = format!(" -{}", entry.deletions);

        let spans = vec![
            Span::styled(format!(" {} ", entry.status_char), status_style),
            Span::styled(filename.to_string(), name_style),
            Span::styled(adds, Style::default().fg(theme::SUCCESS)),
            Span::styled(dels, Style::default().fg(theme::ERROR)),
        ];

        let line = Line::from(spans);
        let line_area = Rect {
            x: inner.x,
            y,
            width: inner.width,
            height: 1,
        };

        if is_selected {
            for x in inner.x..inner.right() {
                frame.buffer_mut()[(x, y)].set_bg(theme::SELECTION_BG);
            }
        }

        Widget::render(line, line_area, frame.buffer_mut());
    }

    crate::widgets::scrollbar::render_scrollbar(
        inner,
        frame.buffer_mut(),
        cs.entries.len(),
        visible,
        scroll,
    );
}

pub(super) fn render_changes_diff(app: &mut App, frame: &mut Frame, area: Rect) {
    use ratatui::style::Modifier;

    let cs = match &mut app.changes_state {
        Some(s) => s,
        None => return,
    };

    let entry = match cs.entries.get(cs.selected_index) {
        Some(e) => e,
        None => return,
    };

    if cs.cached_diff_index != cs.selected_index {
        let root = app.workspace.roots().first().map(|r| r.to_path_buf());
        let old_content = root
            .as_ref()
            .and_then(|r| gaviero_core::git::GitRepo::open(r).ok())
            .and_then(|repo| repo.head_file_content(&entry.rel_path).ok())
            .unwrap_or_default();
        let new_content = std::fs::read_to_string(&entry.abs_path).unwrap_or_default();

        let old_lines: Vec<&str> = old_content.lines().collect();
        let new_lines: Vec<&str> = new_content.lines().collect();
        cs.cached_diff = build_simple_diff(&old_lines, &new_lines);
        cs.cached_diff_index = cs.selected_index;
    }

    let diff_lines = &cs.cached_diff;
    let gutter_w = theme::DIFF_GUTTER_WIDTH;
    let max_scroll = diff_lines.len().saturating_sub(1);
    if cs.diff_scroll > max_scroll {
        cs.diff_scroll = max_scroll;
    }
    let scroll = cs.diff_scroll;

    let header = format!(" {} ", entry.rel_path);
    let header_style = Style::default()
        .fg(theme::FOCUS_BORDER)
        .add_modifier(Modifier::BOLD);
    for (i, ch) in header.chars().enumerate() {
        let x = area.x + i as u16;
        if x < area.right() && area.y < area.bottom() {
            frame.buffer_mut()[(x, area.y)]
                .set_char(ch)
                .set_style(header_style);
        }
    }

    let content_height = area.height.saturating_sub(1) as usize;
    for row in 0..content_height {
        let line_idx = scroll + row;
        if line_idx >= diff_lines.len() {
            break;
        }

        let y = area.y + 1 + row as u16;
        if y >= area.bottom() {
            break;
        }

        let (kind, text) = &diff_lines[line_idx];

        let (gutter_str, gutter_style, line_style) = match kind {
            DiffKind::Added => (
                " + │ ",
                Style::default()
                    .fg(theme::SUCCESS)
                    .add_modifier(Modifier::BOLD),
                Style::default()
                    .fg(theme::SUCCESS)
                    .bg(theme::DIFF_ADD_LINE_BG),
            ),
            DiffKind::Removed => (
                " - │ ",
                Style::default()
                    .fg(theme::ERROR)
                    .add_modifier(Modifier::BOLD),
                Style::default()
                    .fg(theme::ERROR)
                    .bg(theme::DIFF_REM_LINE_BG),
            ),
            DiffKind::Context => (
                "   │ ",
                Style::default().fg(theme::TEXT_DIM),
                Style::default().fg(theme::TEXT_FG),
            ),
        };

        for (i, ch) in gutter_str.chars().enumerate() {
            let x = area.x + i as u16;
            if x < area.right() {
                frame.buffer_mut()[(x, y)]
                    .set_char(ch)
                    .set_style(gutter_style);
            }
        }

        for (i, ch) in text.chars().enumerate() {
            let x = area.x + gutter_w + i as u16;
            if x < area.right() {
                frame.buffer_mut()[(x, y)]
                    .set_char(ch)
                    .set_style(line_style);
            }
        }
    }
}
