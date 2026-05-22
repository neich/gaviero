use super::*;

/// Outcome of applying a proposal to disk.
///
/// `Stale` means the file on disk no longer matches the content the proposal
/// was diffed against — applying would silently overwrite the user's (or
/// another agent's) edit. The caller surfaces this and skips the write.
enum ApplyOutcome {
    Written,
    Stale { path: std::path::PathBuf },
    Failed { path: std::path::PathBuf, error: String },
}

/// Apply a proposal to disk, with stale-check, atomic-ish write, and
/// (for writes) parent-dir creation:
///
/// 1. **Stale check.** If `expected_old` is `Some` it must match the current
///    on-disk content; otherwise we abort to avoid silently overwriting an
///    edit that landed after the proposal was created. `None` means the
///    proposal expected the path to NOT exist (new-file proposal): we abort
///    if the path now exists.
///
/// 2. **Parent-dir creation.** When the proposal is a new-file create and
///    the path's parent directory is missing, we create it before writing.
///    Without this, accepting a new-file proposal in batch mode used to
///    fail silently with a `tracing::error!` and no user feedback.
///
/// 3. **Delete vs write.** When `is_deletion` is true the function calls
///    `fs::remove_file` instead of writing. `new_content` is ignored in
///    that branch.
///
/// 4. **TOCTOU narrowing.** New-file creates use `O_EXCL` so a path that
///    materialised between the stale-check and the write fails cleanly
///    instead of clobbering. Existing-file writes go through a sibling
///    tempfile + atomic `rename`, with a re-read drift check immediately
///    before the rename. The rename itself is atomic on POSIX same-fs
///    moves; the re-read narrows (does not fully close) the window
///    between the initial stale-check and the rename.
fn apply_proposal_to_disk(
    path: &std::path::Path,
    expected_old: Option<&str>,
    new_content: &str,
    is_deletion: bool,
) -> ApplyOutcome {
    use std::path::PathBuf;

    let on_disk = match std::fs::read_to_string(path) {
        Ok(s) => Some(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            return ApplyOutcome::Failed {
                path: path.to_path_buf(),
                error: e.to_string(),
            };
        }
    };

    if on_disk.as_deref() != expected_old {
        return ApplyOutcome::Stale {
            path: path.to_path_buf(),
        };
    }

    if is_deletion {
        return match std::fs::remove_file(path) {
            Ok(()) => ApplyOutcome::Written,
            // Already gone — treat as success (idempotent delete).
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => ApplyOutcome::Written,
            Err(e) => ApplyOutcome::Failed {
                path: path.to_path_buf(),
                error: e.to_string(),
            },
        };
    }

    // New-file proposal: ensure parent directory exists, then create
    // exclusively so a concurrent writer that materialised the path
    // between the stale-check and now fails the open with AlreadyExists
    // (we surface it as Stale).
    if expected_old.is_none() {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            return ApplyOutcome::Failed {
                path: PathBuf::from(parent),
                error: format!("creating parent directory: {}", e),
            };
        }
        return match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(mut f) => {
                use std::io::Write;
                if let Err(e) = f.write_all(new_content.as_bytes()) {
                    let _ = std::fs::remove_file(path);
                    return ApplyOutcome::Failed {
                        path: path.to_path_buf(),
                        error: e.to_string(),
                    };
                }
                ApplyOutcome::Written
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => ApplyOutcome::Stale {
                path: path.to_path_buf(),
            },
            Err(e) => ApplyOutcome::Failed {
                path: path.to_path_buf(),
                error: e.to_string(),
            },
        };
    }

    // Existing-file overwrite: tempfile-then-rename, with a final drift
    // check immediately before the rename. This narrows the TOCTOU
    // window left open by the simple read-then-write pattern.
    let tmp_path = temp_sibling_path(path);
    if let Err(e) = std::fs::write(&tmp_path, new_content) {
        return ApplyOutcome::Failed {
            path: tmp_path,
            error: e.to_string(),
        };
    }
    let recheck = match std::fs::read_to_string(path) {
        Ok(s) => Some(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_path);
            return ApplyOutcome::Failed {
                path: path.to_path_buf(),
                error: format!("re-reading before rename: {}", e),
            };
        }
    };
    if recheck.as_deref() != expected_old {
        let _ = std::fs::remove_file(&tmp_path);
        return ApplyOutcome::Stale {
            path: path.to_path_buf(),
        };
    }
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return ApplyOutcome::Failed {
            path: path.to_path_buf(),
            error: format!("rename {}: {}", tmp_path.display(), e),
        };
    }

    ApplyOutcome::Written
}

/// Build a sibling tempfile path for the atomic rename pattern. The path
/// is a hidden dotfile next to `target`, suffixed with the pid and a
/// monotonic nanosecond stamp so concurrent finalize calls don't collide.
fn temp_sibling_path(target: &std::path::Path) -> std::path::PathBuf {
    use std::ffi::OsString;
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let stem = target
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_else(|| OsString::from("file"));
    let mut name = OsString::from(".gaviero-tmp-");
    name.push(&stem);
    name.push(format!("-{}-{}", pid, nanos));
    parent.join(name)
}

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
    let is_deletion = proposal.is_deletion;
    let content = gaviero_core::write_gate::assemble_final_content(&proposal);
    let path = proposal.file_path.clone();
    let expected_old = if proposal.original_content.is_empty() && !path.exists() {
        None
    } else {
        Some(proposal.original_content.as_str())
    };

    match apply_proposal_to_disk(&path, expected_old, &content, is_deletion) {
        ApplyOutcome::Written => {
            for buf in &mut app.buffers {
                if buf.path.as_deref() == Some(path.as_path()) {
                    let _ = buf.reload();
                }
            }
        }
        ApplyOutcome::Stale { path } => {
            tracing::warn!(
                "Refusing to apply stale proposal for {} — disk changed since proposal was created",
                path.display()
            );
            let msg = format!("⚠ Stale: {} changed on disk; review skipped", path.display());
            app.chat_state.add_system_message(&msg);
            app.status_message = Some((msg, std::time::Instant::now()));
        }
        ApplyOutcome::Failed { path, error } => {
            tracing::error!("Failed to write finalized file {}: {}", path.display(), error);
            let msg = format!("✖ Failed to apply {}: {}", path.display(), error);
            app.chat_state.add_system_message(&msg);
            app.status_message = Some((msg, std::time::Instant::now()));
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

    let incoming: Vec<ReviewProposal> = proposals
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
                id: p.id,
                source: p.source,
                conv_id: p.conv_id,
                conflicts_with: p.conflicts_with,
                superseded: p.status == gaviero_core::types::ProposalStatus::Superseded,
                path: p.file_path,
                old_content,
                new_content: p.proposed_content,
                additions,
                deletions,
                is_deletion: p.is_deletion,
            }
        })
        .collect();

    if let Some(state) = app.batch_review.as_mut() {
        let selected_path = state
            .proposals
            .get(state.selected_index)
            .map(|p| p.path.clone());
        merge_into_batch(&mut state.proposals, incoming);
        link_conflicts_by_path(&mut state.proposals);
        sort_review_proposals(&mut state.proposals, &app.workspace);
        state.selected_index =
            remap_selected_index_after_sort(&state.proposals, selected_path.as_deref());
        // Diff cache index referred to the pre-sort ordering, drop it.
        state.cached_diff = Vec::new();
        state.cached_diff_index = usize::MAX;
        return;
    }

    let mut review_proposals = incoming;
    link_conflicts_by_path(&mut review_proposals);
    sort_review_proposals(&mut review_proposals, &app.workspace);

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
        filter_source: None,
    });
    app.left_panel = LeftPanelMode::Review;
    app.panel_visible.file_tree = true;
    app.focus = Focus::FileTree;
}

/// Append new proposals into an open batch, deduping by `id` so that tasks
/// for the same conversation re-sending the same proposal don't accumulate.
/// Keeps the caller responsible for re-sorting afterwards.
fn merge_into_batch(existing: &mut Vec<ReviewProposal>, incoming: Vec<ReviewProposal>) {
    for proposal in incoming {
        if existing.iter().any(|p| p.id == proposal.id) {
            continue;
        }
        existing.push(proposal);
    }
}

/// Mutual `conflicts_with` links for every row targeting the same path.
/// Backstop when staggered task completion missed gate-side pairing.
fn link_conflicts_by_path(proposals: &mut [ReviewProposal]) {
    use std::collections::HashMap;
    use std::path::PathBuf;
    let mut by_path: HashMap<PathBuf, Vec<usize>> = HashMap::new();
    for (idx, p) in proposals.iter().enumerate() {
        by_path.entry(p.path.clone()).or_default().push(idx);
    }
    for indices in by_path.values() {
        if indices.len() < 2 {
            continue;
        }
        let ids: Vec<u64> = indices.iter().map(|&i| proposals[i].id).collect();
        for &idx in indices {
            let row = &mut proposals[idx];
            for &peer_id in &ids {
                if peer_id != row.id && !row.conflicts_with.contains(&peer_id) {
                    row.conflicts_with.push(peer_id);
                }
            }
        }
    }
}

/// After re-sorting the file list, keep the same path selected when possible.
fn remap_selected_index_after_sort(
    proposals: &[ReviewProposal],
    selected_path: Option<&std::path::Path>,
) -> usize {
    if proposals.is_empty() {
        return 0;
    }
    if let Some(path) = selected_path {
        if let Some(idx) = proposals.iter().position(|p| p.path == path) {
            return idx;
        }
    }
    0
}

fn release_review_hold_ids_async(app: &App, ids: Vec<u64>) {
    if ids.is_empty() {
        return;
    }
    let wg = app.write_gate.clone();
    tokio::spawn(async move {
        let mut gate = wg.lock().await;
        gate.release_review_hold_ids(&ids);
    });
}

/// Mirror gate-side conflict / supersede updates into an open batch inbox.
pub(super) fn sync_batch_proposal_from_gate(app: &App, proposal_id: u64) {
    if app.batch_review.is_none() {
        return;
    }
    let wg = app.write_gate.clone();
    let tx = app.event_tx.clone();
    if let Ok(gate) = wg.try_lock() {
        if let Some((conflicts_with, status)) = gate.review_proposal_fields(proposal_id) {
            let superseded = status == gaviero_core::types::ProposalStatus::Superseded;
            drop(gate);
            let _ = tx.send(crate::event::Event::BatchProposalSynced {
                id: proposal_id,
                conflicts_with,
                superseded,
            });
        }
        return;
    }
    tokio::spawn(async move {
        let gate = wg.lock().await;
        if let Some((conflicts_with, status)) = gate.review_proposal_fields(proposal_id) {
            let superseded = status == gaviero_core::types::ProposalStatus::Superseded;
            let _ = tx.send(crate::event::Event::BatchProposalSynced {
                id: proposal_id,
                conflicts_with,
                superseded,
            });
        }
    });
}

pub(super) fn apply_batch_proposal_sync(
    app: &mut App,
    proposal_id: u64,
    conflicts_with: Vec<u64>,
    superseded: bool,
) {
    let Some(br) = app.batch_review.as_mut() else {
        return;
    };
    let mut touched_selected = false;
    if let Some(row) = br.proposals.iter_mut().find(|p| p.id == proposal_id) {
        row.conflicts_with = conflicts_with.clone();
        row.superseded = superseded;
        touched_selected = br
            .proposals
            .get(br.selected_index)
            .is_some_and(|p| p.id == proposal_id);
    }
    for row in br.proposals.iter_mut() {
        if row.id == proposal_id {
            continue;
        }
        if row.conflicts_with.contains(&proposal_id) {
            for &peer in &conflicts_with {
                if peer != row.id && !row.conflicts_with.contains(&peer) {
                    row.conflicts_with.push(peer);
                }
            }
        }
    }
    if touched_selected {
        br.cached_diff = Vec::new();
        br.cached_diff_index = usize::MAX;
    }
}

/// Truncate a provider name to a short badge label. Drops a trailing
/// ellipsis when the input exceeds `max` chars; preserves short names as-is.
fn truncate_source(source: &str, max: usize) -> String {
    if source.chars().count() <= max {
        source.to_string()
    } else {
        let mut out: String = source.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// Width (in chars) of the agent-source label column in the batch review
/// file list. Sizes to fit the longest source name, capped by the panel
/// width minus a reservation for the status, symbol, filename and stats
/// columns. Bounded below by 3 so a very narrow panel still shows a couple
/// chars of identification, and so Alt+5..9 layout swaps grow the column
/// instead of leaving it stuck at a hardcoded 10 chars.
fn badge_label_width(inner_width: u16, longest_source: usize, multi_root: bool) -> usize {
    const STATUS_W: usize = 3; // " X "
    const SYMBOL_W: usize = 2; // "⊘ ", "⚠ ", or two-space pad
    const TRAIL_W: usize = 1; // trailing space after the label
    const FILENAME_MIN: usize = 10;
    const STATS_W: usize = 10; // " +N -M" budget
    const RESERVED: usize = STATUS_W + SYMBOL_W + TRAIL_W + FILENAME_MIN + STATS_W;

    let multi_root_w = if multi_root { 2 } else { 0 };
    let avail = (inner_width as usize).saturating_sub(multi_root_w + RESERVED);
    longest_source.min(avail).max(3)
}

/// Collect the unique provider sources present in `proposals`, in
/// first-seen order. Used to drive the per-source filter cycle.
fn unique_filter_sources(proposals: &[ReviewProposal]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for p in proposals {
        if !seen.iter().any(|s| s == &p.source) {
            seen.push(p.source.clone());
        }
    }
    seen
}

/// Cycle the active filter through `[None, source_0, source_1, ...]`.
/// `forward = true` follows Alt+o (next), `false` follows Alt+i (previous).
fn cycle_filter_source(
    current: Option<&str>,
    sources: &[String],
    forward: bool,
) -> Option<String> {
    if sources.is_empty() {
        return None;
    }
    // The cycle has `sources.len() + 1` slots: index 0 = None, 1.. = sources.
    let current_idx = match current {
        None => 0,
        Some(s) => sources
            .iter()
            .position(|x| x == s)
            .map(|i| i + 1)
            .unwrap_or(0),
    };
    let len = sources.len() + 1;
    let next_idx = if forward {
        (current_idx + 1) % len
    } else {
        (current_idx + len - 1) % len
    };
    if next_idx == 0 {
        None
    } else {
        Some(sources[next_idx - 1].clone())
    }
}

/// Sort proposals so files in the same workspace folder cluster together.
fn sort_review_proposals(
    proposals: &mut [ReviewProposal],
    workspace: &gaviero_core::workspace::Workspace,
) {
    proposals.sort_by(|a, b| {
        let fa = workspace
            .folder_for_worktree_path(&a.path)
            .map(|p| p.to_path_buf());
        let fb = workspace
            .folder_for_worktree_path(&b.path)
            .map(|p| p.to_path_buf());
        fa.cmp(&fb).then_with(|| a.path.cmp(&b.path))
    });
}

pub(super) fn handle_batch_review_action(app: &mut App, action: &Action) -> bool {
    if app.batch_review.is_none() {
        return false;
    }

    match action {
        Action::CycleTabForward | Action::CycleTabBack => {
            let forward = matches!(action, Action::CycleTabForward);
            let br = app.batch_review.as_mut().unwrap();
            let sources = unique_filter_sources(&br.proposals);
            br.filter_source = cycle_filter_source(br.filter_source.as_deref(), &sources, forward);
            true
        }
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
            let (path, content, expected_old, is_deletion, peer_ids, superseded) = {
                let br = app.batch_review.as_ref().unwrap();
                match br.proposals.get(br.selected_index) {
                    Some(p) => (
                        p.path.clone(),
                        p.new_content.clone(),
                        p.old_content.clone(),
                        p.is_deletion,
                        p.conflicts_with.clone(),
                        p.superseded,
                    ),
                    None => return true,
                }
            };
            if superseded {
                let msg = format!(
                    "⊘ Cannot apply {} — superseded by a conflicting proposal that was already accepted",
                    path.display()
                );
                app.chat_state.add_system_message(&msg);
                app.status_message = Some((msg, std::time::Instant::now()));
                return true;
            }
            let outcome =
                apply_proposal_to_disk(&path, expected_old.as_deref(), &content, is_deletion);
            match outcome {
                ApplyOutcome::Written => {
                    for buf in &mut app.buffers {
                        if buf.path.as_deref() == Some(path.as_path()) {
                            let _ = buf.reload();
                        }
                    }
                }
                ApplyOutcome::Stale { path: stale_path } => {
                    tracing::warn!(
                        "Refusing to apply stale proposal for {} — disk changed since proposal",
                        stale_path.display()
                    );
                    let msg = format!(
                        "⚠ Stale: {} changed on disk; not applied",
                        stale_path.display()
                    );
                    app.chat_state.add_system_message(&msg);
                    app.status_message = Some((msg, std::time::Instant::now()));
                    return true;
                }
                ApplyOutcome::Failed { path: fp, error } => {
                    tracing::error!("Failed to write {}: {}", fp.display(), error);
                    let msg = format!("✖ Failed to apply {}: {}", fp.display(), error);
                    app.chat_state.add_system_message(&msg);
                    app.status_message = Some((msg, std::time::Instant::now()));
                }
            }
            {
                let (removed_id, empty) = {
                    let br = app.batch_review.as_mut().unwrap();
                    // Mark any peer conflicts as superseded so the user sees the
                    // pick-one resolution take effect without removing the rows.
                    if !peer_ids.is_empty() {
                        for peer in br.proposals.iter_mut() {
                            if peer_ids.contains(&peer.id) {
                                peer.superseded = true;
                            }
                        }
                    }
                    let removed_id = br.proposals.get(br.selected_index).map(|p| p.id);
                    br.proposals.remove(br.selected_index);
                    if br.selected_index >= br.proposals.len() && br.selected_index > 0 {
                        br.selected_index -= 1;
                    }
                    br.diff_scroll = 0;
                    br.cached_diff = Vec::new();
                    br.cached_diff_index = usize::MAX;
                    let empty = br.proposals.is_empty();
                    (removed_id, empty)
                };
                if let Some(id) = removed_id {
                    release_review_hold_ids_async(app, vec![id]);
                }
                if empty {
                    app.batch_review = None;
                    app.left_panel = LeftPanelMode::FileTree;
                    app.status_message =
                        Some(("All files reviewed".to_string(), std::time::Instant::now()));
                }
            }
            true
        }
        Action::InsertChar('r') => {
            let (removed_id, empty) = {
                let br = app.batch_review.as_mut().unwrap();
                let removed_id = if br.proposals.is_empty() {
                    None
                } else {
                    Some(br.proposals[br.selected_index].id)
                };
                if !br.proposals.is_empty() {
                    br.proposals.remove(br.selected_index);
                    if br.selected_index >= br.proposals.len() && br.selected_index > 0 {
                        br.selected_index -= 1;
                    }
                    br.diff_scroll = 0;
                    br.cached_diff = Vec::new();
                    br.cached_diff_index = usize::MAX;
                }
                let empty = br.proposals.is_empty();
                (removed_id, empty)
            };
            if let Some(id) = removed_id {
                release_review_hold_ids_async(app, vec![id]);
            }
            if empty {
                app.batch_review = None;
                app.left_panel = LeftPanelMode::FileTree;
                app.status_message = Some((
                    "All files reviewed — no changes applied".to_string(),
                    std::time::Instant::now(),
                ));
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

    let hold_ids: Vec<u64> = br.proposals.iter().map(|p| p.id).collect();
    release_review_hold_ids_async(app, hold_ids);

    // Pick-one for conflict pairs: when both halves of a conflict survive into
    // apply-all, the first one wins and the rest get marked superseded. We
    // mutate a local Vec so the visible-superseded check stays simple.
    let mut proposals = br.proposals.clone();
    let mut superseded_ids: std::collections::HashSet<u64> = proposals
        .iter()
        .filter(|p| p.superseded)
        .map(|p| p.id)
        .collect();
    for proposal in proposals.iter() {
        if superseded_ids.contains(&proposal.id) {
            continue;
        }
        for peer in &proposal.conflicts_with {
            superseded_ids.insert(*peer);
        }
    }
    for p in proposals.iter_mut() {
        if superseded_ids.contains(&p.id) {
            p.superseded = true;
        }
    }

    let mut written = Vec::new();
    let mut stale = Vec::new();
    let mut failed = Vec::new();
    let mut superseded_count = 0_usize;
    for proposal in &proposals {
        if proposal.superseded {
            tracing::warn!(
                "Skipping superseded proposal for {}",
                proposal.path.display()
            );
            app.chat_state.add_system_message(&format!(
                "⊘ Superseded: {} not applied (conflict resolved by peer)",
                proposal.path.display()
            ));
            superseded_count += 1;
            continue;
        }
        match apply_proposal_to_disk(
            &proposal.path,
            proposal.old_content.as_deref(),
            &proposal.new_content,
            proposal.is_deletion,
        ) {
            ApplyOutcome::Written => written.push(proposal.path.clone()),
            ApplyOutcome::Stale { path } => {
                tracing::warn!(
                    "Refusing to apply stale proposal for {} — disk changed since proposal",
                    path.display()
                );
                app.chat_state.add_system_message(&format!(
                    "⚠ Stale: {} changed on disk; skipped",
                    path.display()
                ));
                stale.push(path);
            }
            ApplyOutcome::Failed { path, error } => {
                tracing::error!("Failed to write {}: {}", path.display(), error);
                app.chat_state.add_system_message(&format!(
                    "✖ Failed to apply {}: {}",
                    path.display(),
                    error
                ));
                failed.push(path);
            }
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
    let mut summary = format!("{} file(s) written", written.len());
    if superseded_count > 0 {
        summary.push_str(&format!(", {} superseded (skipped)", superseded_count));
    }
    if !stale.is_empty() {
        summary.push_str(&format!(", {} stale (skipped)", stale.len()));
    }
    if !failed.is_empty() {
        summary.push_str(&format!(", {} failed", failed.len()));
    }
    app.status_message = Some((summary, std::time::Instant::now()));
}

pub(super) fn cancel_batch_review(app: &mut App) {
    let (n, hold_ids) = app
        .batch_review
        .as_ref()
        .map(|br| {
            (
                br.proposals.len(),
                br.proposals.iter().map(|p| p.id).collect::<Vec<_>>(),
            )
        })
        .unwrap_or((0, Vec::new()));
    release_review_hold_ids_async(app, hold_ids);
    app.batch_review = None;
    app.left_panel = LeftPanelMode::FileTree;
    app.status_message = Some((
        format!("Review discarded — {} file(s) not written", n),
        std::time::Instant::now(),
    ));
}

/// One row in the rendered review-file-list: either a folder-group header
/// or a proposal row. Headers are non-selectable and appear above each
/// group when the workspace has more than one root.
enum ReviewRow<'a> {
    Header(&'a str),
    Entry(usize),
}

/// Build the rendered row sequence for the batch review file list.
///
/// In single-folder workspaces, returns one Entry per proposal in order.
/// In multi-folder workspaces, walks the (already folder-sorted) proposals
/// and inserts a Header row whenever the folder root changes.
fn build_review_rows<'a>(
    proposals: &[ReviewProposal],
    workspace: &'a gaviero_core::workspace::Workspace,
) -> Vec<ReviewRow<'a>> {
    let folders = workspace.folders();
    let mut rows = Vec::with_capacity(proposals.len() + folders.len());

    if folders.len() <= 1 {
        for i in 0..proposals.len() {
            rows.push(ReviewRow::Entry(i));
        }
        return rows;
    }

    let mut current_folder: Option<&std::path::Path> = None;
    for (i, p) in proposals.iter().enumerate() {
        let folder = workspace.folder_for_worktree_path(&p.path);
        if folder != current_folder {
            let label = folder
                .and_then(|root| {
                    folders
                        .iter()
                        .find(|f| f.path.as_path() == root)
                        .map(|f| f.display_name())
                })
                .unwrap_or("(other)");
            rows.push(ReviewRow::Header(label));
            current_folder = folder;
        }
        rows.push(ReviewRow::Entry(i));
    }
    rows
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

    let multi_root = app.workspace.folders().len() > 1;
    let rows = build_review_rows(
        app.batch_review.as_ref().map(|r| r.proposals.as_slice()).unwrap_or(&[]),
        &app.workspace,
    );

    let br = match &mut app.batch_review {
        Some(r) => r,
        None => return,
    };
    let filter_source = br.filter_source.clone();

    let longest_source = br
        .proposals
        .iter()
        .map(|p| p.source.chars().count())
        .max()
        .unwrap_or(0);
    let label_w = badge_label_width(inner.width, longest_source, multi_root);

    let visible = inner.height as usize;

    // Scroll is in render-row space (header rows count toward scrolling).
    let selected_row = rows
        .iter()
        .position(|r| matches!(r, ReviewRow::Entry(i) if *i == br.selected_index))
        .unwrap_or(0);

    if visible > 0 {
        if selected_row < br.scroll_offset {
            br.scroll_offset = selected_row;
        } else if selected_row >= br.scroll_offset + visible {
            br.scroll_offset = selected_row - visible + 1;
        }
    }

    let scroll = br.scroll_offset;

    for (row_idx, row) in rows.iter().enumerate().skip(scroll).take(visible) {
        let y = inner.y + (row_idx - scroll) as u16;
        if y >= inner.bottom() {
            break;
        }

        match row {
            ReviewRow::Header(label) => {
                let header_text = format!(" ▾ {}", label);
                let line = Line::from(Span::styled(
                    header_text,
                    Style::default()
                        .fg(theme::TEXT_DIM)
                        .add_modifier(Modifier::BOLD),
                ));
                let line_area = Rect {
                    x: inner.x,
                    y,
                    width: inner.width,
                    height: 1,
                };
                Widget::render(line, line_area, frame.buffer_mut());
            }
            ReviewRow::Entry(i) => {
                let proposal = &br.proposals[*i];
                let is_selected = *i == br.selected_index;
                let filtered_out = filter_source
                    .as_deref()
                    .is_some_and(|f| f != proposal.source);
                let in_conflict = !proposal.conflicts_with.is_empty();
                let filename = proposal
                    .path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?");

                let base_fg = if filtered_out {
                    theme::TEXT_DIM
                } else {
                    theme::TEXT_FG
                };
                let name_style = if is_selected {
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                        .bg(theme::SELECTION_BG)
                } else {
                    Style::default().fg(base_fg)
                };

                let adds = format!(" +{}", proposal.additions);
                let dels = format!(" -{}", proposal.deletions);

                let (status_char, status_color) = if proposal.is_deletion {
                    ('D', theme::ERROR)
                } else if proposal.old_content.is_none() {
                    ('A', theme::SUCCESS)
                } else {
                    ('M', theme::WARNING)
                };
                let status_fg = if filtered_out {
                    theme::TEXT_DIM
                } else {
                    status_color
                };

                let badge_label = truncate_source(&proposal.source, label_w);
                let badge_color = if proposal.superseded {
                    theme::TEXT_DIM
                } else if in_conflict {
                    theme::WARNING
                } else if filtered_out {
                    theme::TEXT_DIM
                } else {
                    theme::FOCUS_BORDER
                };
                let badge_text = if proposal.superseded {
                    format!("⊘ {:<width$} ", badge_label, width = label_w)
                } else if in_conflict {
                    format!("⚠ {:<width$} ", badge_label, width = label_w)
                } else {
                    format!("  {:<width$} ", badge_label, width = label_w)
                };

                let prefix = if multi_root { "  " } else { "" };
                let spans = vec![
                    Span::raw(prefix),
                    Span::styled(
                        format!(" {} ", status_char),
                        Style::default().fg(status_fg),
                    ),
                    Span::styled(badge_text, Style::default().fg(badge_color)),
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
        }
    }

    crate::widgets::scrollbar::render_scrollbar(
        inner,
        frame.buffer_mut(),
        rows.len(),
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

    let header = if proposal.is_deletion {
        format!(" {} (DELETE) ", proposal.path.display())
    } else {
        format!(" {} ", proposal.path.display())
    };
    let header_style = if proposal.is_deletion {
        Style::default().fg(theme::ERROR).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(theme::FOCUS_BORDER)
            .add_modifier(Modifier::BOLD)
    };
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

    if area.height > 1 {
        let content_area = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: area.height - 1,
        };
        let diff_indices: Vec<usize> = diff_lines
            .iter()
            .enumerate()
            .filter(|(_, (k, _))| !matches!(k, DiffKind::Context))
            .map(|(i, _)| i)
            .collect();
        crate::widgets::scrollbar::render_scrollbar_with_diff_markers(
            content_area,
            frame.buffer_mut(),
            diff_lines.len(),
            content_height,
            scroll,
            &diff_indices,
        );
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

    if area.height > 1 {
        let content_area = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: area.height - 1,
        };
        let diff_indices: Vec<usize> = diff_lines
            .iter()
            .enumerate()
            .filter(|(_, (k, _))| !matches!(k, DiffKind::Context))
            .map(|(i, _)| i)
            .collect();
        crate::widgets::scrollbar::render_scrollbar_with_diff_markers(
            content_area,
            frame.buffer_mut(),
            diff_lines.len(),
            content_height,
            scroll,
            &diff_indices,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{ApplyOutcome, apply_proposal_to_disk, merge_into_batch, temp_sibling_path};
    use crate::app::state::ReviewProposal;

    fn make_review_proposal(id: u64, path: &str) -> ReviewProposal {
        ReviewProposal {
            id,
            source: "agent".into(),
            conv_id: None,
            conflicts_with: Vec::new(),
            superseded: false,
            path: path.into(),
            old_content: None,
            new_content: String::new(),
            additions: 0,
            deletions: 0,
            is_deletion: false,
        }
    }

    #[test]
    fn merge_into_batch_appends_new_proposals() {
        let mut existing = vec![make_review_proposal(1, "src/a.rs")];
        let incoming = vec![
            make_review_proposal(2, "src/b.rs"),
            make_review_proposal(3, "src/c.rs"),
        ];
        merge_into_batch(&mut existing, incoming);
        let ids: Vec<u64> = existing.iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn cycle_filter_source_walks_none_sources_then_back() {
        let sources = vec!["claude".to_string(), "codex".to_string()];

        let s1 = super::cycle_filter_source(None, &sources, true);
        assert_eq!(s1.as_deref(), Some("claude"));
        let s2 = super::cycle_filter_source(s1.as_deref(), &sources, true);
        assert_eq!(s2.as_deref(), Some("codex"));
        let s3 = super::cycle_filter_source(s2.as_deref(), &sources, true);
        assert_eq!(s3, None);
        // Backward from None goes to the last source.
        let back = super::cycle_filter_source(None, &sources, false);
        assert_eq!(back.as_deref(), Some("codex"));
    }

    #[test]
    fn cycle_filter_source_is_inert_when_no_sources() {
        let sources: Vec<String> = Vec::new();
        assert_eq!(super::cycle_filter_source(None, &sources, true), None);
        assert_eq!(super::cycle_filter_source(Some("x"), &sources, false), None);
    }

    #[test]
    fn unique_filter_sources_preserves_first_seen_order() {
        let proposals = vec![
            ReviewProposal {
                source: "codex".into(),
                ..make_review_proposal(1, "a")
            },
            ReviewProposal {
                source: "claude".into(),
                ..make_review_proposal(2, "b")
            },
            ReviewProposal {
                source: "codex".into(),
                ..make_review_proposal(3, "c")
            },
        ];
        let unique = super::unique_filter_sources(&proposals);
        assert_eq!(unique, vec!["codex", "claude"]);
    }

    #[test]
    fn truncate_source_keeps_short_and_truncates_long() {
        assert_eq!(super::truncate_source("claude", 10), "claude");
        assert_eq!(super::truncate_source("very-long-name", 6), "very-…");
    }

    #[test]
    fn badge_label_width_grows_with_panel_width() {
        // Wide panel fits the full 20-char source name.
        assert_eq!(super::badge_label_width(80, 20, false), 20);
        // Same source on a narrower panel gets squeezed.
        let narrow = super::badge_label_width(40, 20, false);
        assert!(narrow < 20, "narrow={}, expected < 20", narrow);
        // Multi-root reserves 2 extra chars for the folder-group indent.
        let single = super::badge_label_width(40, 20, false);
        let multi = super::badge_label_width(40, 20, true);
        assert!(multi <= single, "multi={}, single={}", multi, single);
    }

    #[test]
    fn badge_label_width_floors_at_three_on_tight_panels() {
        // Even a near-zero panel keeps at least 3 chars so the user sees
        // something identifying.
        assert_eq!(super::badge_label_width(0, 20, false), 3);
        assert_eq!(super::badge_label_width(10, 20, false), 3);
    }

    #[test]
    fn badge_label_width_caps_at_longest_source() {
        // A short longest_source must not balloon the column even on huge
        // panels — wasted padding hurts readability.
        assert_eq!(super::badge_label_width(200, 5, false), 5);
    }

    #[test]
    fn link_conflicts_by_path_links_same_path_rows() {
        let mut rows = vec![
            make_review_proposal(1, "src/shared.rs"),
            make_review_proposal(2, "src/shared.rs"),
            make_review_proposal(3, "src/other.rs"),
        ];
        super::link_conflicts_by_path(&mut rows);
        assert_eq!(rows[0].conflicts_with, vec![2]);
        assert_eq!(rows[1].conflicts_with, vec![1]);
        assert!(rows[2].conflicts_with.is_empty());
    }

    #[test]
    fn merge_into_batch_links_path_conflicts() {
        let mut existing = vec![make_review_proposal(10, "src/x.rs")];
        let incoming = vec![make_review_proposal(20, "src/x.rs")];
        merge_into_batch(&mut existing, incoming);
        super::link_conflicts_by_path(&mut existing);
        assert_eq!(existing[0].conflicts_with, vec![20]);
        assert_eq!(existing[1].conflicts_with, vec![10]);
    }

    #[test]
    fn merge_into_batch_dedupes_by_id() {
        let mut existing = vec![
            make_review_proposal(1, "src/a.rs"),
            make_review_proposal(2, "src/b.rs"),
        ];
        // id=2 already present — must be dropped from incoming.
        let incoming = vec![
            make_review_proposal(2, "src/b.rs"),
            make_review_proposal(3, "src/c.rs"),
        ];
        merge_into_batch(&mut existing, incoming);
        let ids: Vec<u64> = existing.iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn apply_writes_when_disk_matches_expected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.rs");
        std::fs::write(&path, "old\n").unwrap();
        let outcome = apply_proposal_to_disk(&path, Some("old\n"), "new\n", false);
        assert!(matches!(outcome, ApplyOutcome::Written));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
    }

    #[test]
    fn apply_refuses_stale_when_disk_changed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.rs");
        std::fs::write(&path, "user-edit\n").unwrap();
        // Proposal expected "old\n" but user edited to "user-edit\n".
        let outcome = apply_proposal_to_disk(&path, Some("old\n"), "agent-new\n", false);
        assert!(matches!(outcome, ApplyOutcome::Stale { .. }));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "user-edit\n");
    }

    #[test]
    fn apply_refuses_when_proposal_expected_new_but_path_now_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("created-since.rs");
        std::fs::write(&path, "raced\n").unwrap();
        let outcome = apply_proposal_to_disk(&path, None, "agent-new\n", false);
        assert!(matches!(outcome, ApplyOutcome::Stale { .. }));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "raced\n");
    }

    #[test]
    fn apply_creates_parent_dir_for_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/deep/file.rs");
        let outcome = apply_proposal_to_disk(&path, None, "fn main() {}\n", false);
        assert!(matches!(outcome, ApplyOutcome::Written));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "fn main() {}\n");
    }

    #[test]
    fn apply_creates_new_file_when_expected_old_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("brand-new.rs");
        let outcome = apply_proposal_to_disk(&path, None, "hi\n", false);
        assert!(matches!(outcome, ApplyOutcome::Written));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hi\n");
    }

    #[test]
    fn apply_deletes_when_is_deletion_and_disk_matches() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doomed.rs");
        std::fs::write(&path, "fn doomed() {}\n").unwrap();
        let outcome = apply_proposal_to_disk(&path, Some("fn doomed() {}\n"), "", true);
        assert!(matches!(outcome, ApplyOutcome::Written));
        assert!(!path.exists(), "file should have been removed");
    }

    #[test]
    fn apply_delete_refuses_when_disk_drifted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doomed.rs");
        std::fs::write(&path, "user-edit\n").unwrap();
        let outcome = apply_proposal_to_disk(&path, Some("fn doomed() {}\n"), "", true);
        assert!(matches!(outcome, ApplyOutcome::Stale { .. }));
        assert!(path.exists(), "file should be preserved on stale-skip");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "user-edit\n");
    }

    #[test]
    fn apply_delete_is_idempotent_when_already_gone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("already-gone.rs");
        // expected_old=None mirrors a delete proposal whose original snapshot
        // recorded the file as nonexistent — degenerate but allowed.
        let outcome = apply_proposal_to_disk(&path, None, "", true);
        assert!(matches!(outcome, ApplyOutcome::Written));
        assert!(!path.exists());
    }

    #[test]
    fn apply_existing_file_leaves_no_temp_file() {
        // The tempfile-then-rename path must not leave the sibling tempfile
        // behind on a successful write.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.rs");
        std::fs::write(&path, "old\n").unwrap();
        let outcome = apply_proposal_to_disk(&path, Some("old\n"), "new\n", false);
        assert!(matches!(outcome, ApplyOutcome::Written));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
        // No leftover tempfile in the directory.
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name())
            .collect();
        assert_eq!(entries.len(), 1, "expected only the target file, got {:?}", entries);
    }

    #[test]
    fn apply_new_file_fails_atomically_when_path_materialised() {
        // The new-file branch uses O_EXCL — if the path appeared between the
        // initial stale-check and the open, we surface Stale rather than
        // clobbering. We can't easily race this in a single thread, so we
        // simulate it by pre-creating the file and asserting Stale.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("raced.rs");
        std::fs::write(&path, "raced\n").unwrap();
        let outcome = apply_proposal_to_disk(&path, None, "agent-new\n", false);
        assert!(matches!(outcome, ApplyOutcome::Stale { .. }));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "raced\n");
    }

    #[test]
    fn temp_sibling_path_is_in_same_dir_and_distinct() {
        let target = std::path::Path::new("/tmp/some/dir/file.rs");
        let tmp = temp_sibling_path(target);
        assert_eq!(tmp.parent(), target.parent());
        assert_ne!(tmp.file_name(), target.file_name());
        assert!(
            tmp.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".gaviero-tmp-")
        );
    }
}
