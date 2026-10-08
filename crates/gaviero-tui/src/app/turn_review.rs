//! Turn review — the mandatory post-turn review of every file a chat turn
//! changed on disk (`gaviero_core::turn_capture`).
//!
//! The agent's edits are already on disk when this opens. There are four
//! decisions, each applied immediately: accept (`a`) or reject (`r`) the
//! selected file, accept (`A`) or reject (`R`) everything not decided yet.
//! Reject means going back to the pre-prompt version. The review ends when
//! every file has a decision. A conversation with a pending review cannot
//! send its next prompt; other conversations are unaffected, so the panel is
//! a left-panel mode rather than a modal lock.
//!
//! Every core call (drift checks, reverts) runs from a handler, never from the
//! render path: render reads the panel's list state only.
//!
//! The panel is a *list*; the *reading* is the editor's. `Enter` opens the
//! selected file in the shared read-only diff tab
//! ([`super::editing::open_change_diff`]) — the whole file with its changed
//! lines highlighted, syntax highlighting and real scrolling.
//!
//! That one viewer serves every "what changed?" panel — the git panel, the
//! HISTORY panel ([`open_history_file_diff`]) and this one. None of them paints
//! a diff of its own, so there is a single diff mechanism to keep working.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Widget};

use gaviero_core::history::ChangedFile;
use gaviero_core::turn_capture::{
    BlobRef, ChangeKind, FileChange, FileDecision, PendingReview, ResolvedDecision, RevertOutcome,
    ReviewedTurn, TurnOutcome, revert_file,
};

use super::*;

/// Cursor state for the review on screen (`App::pending_turn_reviews[active]`).
#[derive(Default)]
pub(crate) struct TurnReviewView {
    pub active: usize,
    pub selected: usize,
    pub scroll_offset: usize,
    /// A reject was refused because the file changed after the turn ended;
    /// repeating the same key (`r:<file key>` or `R`) confirms overwriting.
    pub confirm_force: Option<String>,
}

// ── Entry points ─────────────────────────────────────────────────────

pub(super) fn conv_has_pending_review(app: &App, conv_id: &str) -> bool {
    app.pending_turn_reviews
        .iter()
        .any(|r| r.set.conv_id.as_deref() == Some(conv_id))
}

/// A turn ended with changes: register its review and show it.
pub(super) fn on_turn_review_pending(
    app: &mut App,
    review: PendingReview,
    overlapped: Vec<String>,
) {
    let turn_id = review.set.turn_id.clone();
    let conv_id = review.set.conv_id.clone();
    let n = review.set.files.len();
    let outcome = review.set.outcome;
    match app
        .pending_turn_reviews
        .iter_mut()
        .find(|r| r.set.turn_id == turn_id)
    {
        Some(slot) => *slot = review,
        None => app.pending_turn_reviews.push(review),
    }
    // Overlap marks were written to the other turns' pending files; reload them.
    if !overlapped.is_empty() {
        let fresh = app.turn_capture.load_pending();
        for r in app.pending_turn_reviews.iter_mut() {
            if overlapped.contains(&r.set.turn_id)
                && let Some(updated) = fresh.iter().find(|f| f.set.turn_id == r.set.turn_id)
            {
                r.set = updated.set.clone();
            }
        }
    }

    if let Some(conv_id) = conv_id.as_deref()
        && let Some(idx) = app.chat_state.find_conv_idx(conv_id)
    {
        let tag = match outcome {
            TurnOutcome::Completed => "",
            TurnOutcome::Cancelled => " (turn cancelled)",
            TurnOutcome::Failed => " (turn failed)",
        };
        app.chat_state.add_system_message_at(
            idx,
            &format!(
                "This turn changed {n} file{}{tag}. Review the changes (keep or revert) \
                 before sending the next prompt.",
                if n == 1 { "" } else { "s" }
            ),
        );
    }
    let index = app
        .pending_turn_reviews
        .iter()
        .position(|r| r.set.turn_id == turn_id)
        .unwrap_or(0);
    show(app, index);
    push_review_frame(app, &turn_id, true);
    for other in &overlapped {
        push_review_frame(app, other, false);
    }
}

/// Show the pending review of `conv_id` (or the first one) in the left panel.
pub(super) fn open_for_conv(app: &mut App, conv_id: Option<&str>) {
    let index = conv_id
        .and_then(|c| {
            app.pending_turn_reviews
                .iter()
                .position(|r| r.set.conv_id.as_deref() == Some(c))
        })
        .unwrap_or(0);
    if index < app.pending_turn_reviews.len() {
        show(app, index);
    }
}

fn show(app: &mut App, index: usize) {
    if app.turn_review_view.active != index || app.left_panel != LeftPanelMode::TurnReview {
        app.turn_review_view = TurnReviewView {
            active: index,
            ..TurnReviewView::default()
        };
    }
    app.left_panel = LeftPanelMode::TurnReview;
    app.panel_visible.file_tree = true;
    app.focus = Focus::FileTree;
}

/// Reopen reviews persisted by a previous session (crash or quit).
pub(super) fn restore_on_startup(app: &mut App) {
    app.pending_turn_reviews = app.turn_capture.load_pending();
    if !app.pending_turn_reviews.is_empty() {
        let n = app.pending_turn_reviews.len();
        app.status_message = Some((
            format!(
                "{n} turn review{} pending from a previous session",
                if n == 1 { "" } else { "s" }
            ),
            std::time::Instant::now(),
        ));
        show(app, 0);
    }
    app.turn_capture.gc_later();
}

/// Record a write the user made through the editor (save, new file) so a
/// running turn's end scan does not attribute it to the agent. Reads the
/// bytes back: `Buffer::save` may normalize line endings.
pub(super) fn note_host_write(app: &App, path: &std::path::Path) {
    let bytes = std::fs::read(path).ok();
    app.turn_capture.ledger().record(path, bytes.as_deref());
}

// ── Helpers ──────────────────────────────────────────────────────────

fn active_review(app: &App) -> Option<&PendingReview> {
    app.pending_turn_reviews.get(app.turn_review_view.active)
}

fn selected_change(app: &App) -> Option<&FileChange> {
    active_review(app).and_then(|r| r.set.files.get(app.turn_review_view.selected))
}

/// The decision taken on a file, or `None` while it is still undecided.
/// `Keep` = accepted; `Revert` = rejected (already applied on disk).
fn decided<'a>(review: &'a PendingReview, change: &FileChange) -> Option<&'a FileDecision> {
    review.decisions.get(&change.key())
}

/// Record a decision in memory only. Every action ends in
/// [`advance_or_finish`] (or an explicit [`persist`]), which saves once — a
/// bulk `A` / `R` over hundreds of files must not save per file.
fn record_decision(app: &mut App, key: String, decision: FileDecision) {
    let view_active = app.turn_review_view.active;
    if let Some(review) = app.pending_turn_reviews.get_mut(view_active) {
        review.decisions.insert(key, decision);
    }
}

/// Save decisions so a crash keeps them (off the event loop, ordered with
/// the archive), and mirror them to the phone.
fn persist(app: &mut App) {
    let Some(review) = active_review(app).cloned() else {
        return;
    };
    push_review_frame(app, &review.set.turn_id, false);
    app.turn_capture.save_pending_later(review);
}

// ── Actions ──────────────────────────────────────────────────────────

/// Keys while the TURN REVIEW list has focus. Returns true when consumed.
pub(super) fn handle_turn_review_action(app: &mut App, action: &Action) -> bool {
    if active_review(app).is_none() {
        app.left_panel = LeftPanelMode::FileTree;
        return false;
    }
    let files = active_review(app).map(|r| r.set.files.len()).unwrap_or(0);
    // The "changed since the turn" confirmation survives only a repeat of
    // the key that raised it.
    let armed_before = app.turn_review_view.confirm_force.clone();
    let consumed = match action {
        Action::CursorDown | Action::InsertChar('j') => {
            if app.turn_review_view.selected + 1 < files {
                app.turn_review_view.selected += 1;
            }
            true
        }
        Action::CursorUp | Action::InsertChar('k') => {
            if app.turn_review_view.selected > 0 {
                app.turn_review_view.selected -= 1;
            }
            true
        }
        // The whole file with its changes highlighted, in the editor's diff tab
        // — the same viewer the git panel and the history panel open. Reading a
        // file is therefore the editor's job, and the panel has no diff keys of
        // its own: `j`/`k`/`↑`/`↓` move the list, `PgUp`/`PgDn`/`J`/`K` are
        // free, and the diff tab scrolls with the editor's own keys.
        Action::Enter => {
            open_selected_change(app);
            true
        }
        // The four decisions. Each takes effect immediately.
        Action::InsertChar('a') => {
            accept_selected(app);
            true
        }
        Action::InsertChar('r') => {
            reject_selected(app);
            true
        }
        Action::InsertChar('A') => {
            let _ = decide_rest(app, false, true);
            true
        }
        Action::InsertChar('R') => {
            let _ = decide_rest(app, true, true);
            true
        }
        Action::CycleTabForward | Action::Tab => {
            let n = app.pending_turn_reviews.len();
            if n > 1 {
                let next = (app.turn_review_view.active + 1) % n;
                show(app, next);
            }
            true
        }
        _ => false,
    };
    if consumed && app.turn_review_view.confirm_force == armed_before {
        // Nothing (re-)armed it this time: any other key cancels.
        app.turn_review_view.confirm_force = None;
    }
    consumed
}

/// Why a reject did not happen.
enum RejectError {
    /// The file changed after the turn ended; reverting would overwrite it.
    Drifted,
    /// The pre-turn content was not stored (or the revert failed).
    Refused(String),
}

fn status(app: &mut App, msg: impl Into<String>) {
    app.status_message = Some((msg.into(), std::time::Instant::now()));
}

/// Go back to the pre-prompt version of file `idx` of the active review, now.
fn reject_file(app: &mut App, idx: usize, force: bool) -> Result<(), RejectError> {
    let path = revert_one(app, idx, force)?;
    sync_editor(app, std::iter::once(path.as_path()));
    Ok(())
}

/// Revert file `idx` and record the decision. Returns the reverted path; the
/// caller syncs the editor (once, for a bulk reject).
fn revert_one(app: &mut App, idx: usize, force: bool) -> Result<std::path::PathBuf, RejectError> {
    let Some(change) = active_review(app)
        .and_then(|r| r.set.files.get(idx))
        .cloned()
    else {
        return Err(RejectError::Refused("no such file".into()));
    };
    match revert_file(&app.turn_capture, &change, force) {
        Ok(RevertOutcome::Reverted | RevertOutcome::AlreadyAtBefore) => {
            record_decision(app, change.key(), FileDecision::Revert);
            Ok(change.path)
        }
        Ok(RevertOutcome::Drifted) => Err(RejectError::Drifted),
        Ok(RevertOutcome::NotRevertible(why)) => Err(RejectError::Refused(why)),
        Err(e) => Err(RejectError::Refused(format!("{e:#}"))),
    }
}

fn sync_editor<'a>(app: &mut App, paths: impl IntoIterator<Item = &'a std::path::Path>) {
    super::agent_writes::reconcile_agent_writes(
        app,
        paths,
        super::agent_writes::WriteOrigin::AgentTurn {
            source: "turn review",
        },
    );
}

fn accept_file(app: &mut App, idx: usize) {
    if let Some(key) = active_review(app)
        .and_then(|r| r.set.files.get(idx))
        .map(FileChange::key)
    {
        record_decision(app, key, FileDecision::Keep);
    }
}

/// A decision is final once taken: a reject is already on disk, so flipping
/// it to accept would only relabel it. Returns the label when decided.
fn already_decided(app: &App, idx: usize) -> Option<&'static str> {
    let review = active_review(app)?;
    match decided(review, review.set.files.get(idx)?)? {
        FileDecision::Keep => Some("accepted"),
        _ => Some("rejected"),
    }
}

/// `a`: keep the agent's version of the selected file.
fn accept_selected(app: &mut App) {
    let idx = app.turn_review_view.selected;
    if let Some(label) = already_decided(app, idx) {
        status(app, format!("Already {label}"));
        return;
    }
    accept_file(app, idx);
    advance_or_finish(app);
}

/// `r`: go back to the selected file's pre-prompt version. A file changed
/// after the turn needs a second `r` (it overwrites those changes).
fn reject_selected(app: &mut App) {
    let idx = app.turn_review_view.selected;
    if let Some(label) = already_decided(app, idx) {
        status(app, format!("Already {label}"));
        return;
    }
    let Some((key, rel)) = selected_change(app).map(|c| (c.key(), c.rel.clone())) else {
        return;
    };
    let token = format!("r:{key}");
    let force = app.turn_review_view.confirm_force.as_deref() == Some(token.as_str());
    app.turn_review_view.confirm_force = None;
    match reject_file(app, idx, force) {
        Ok(()) => advance_or_finish(app),
        Err(RejectError::Drifted) => {
            app.turn_review_view.confirm_force = Some(token);
            status(
                app,
                format!(
                    "⚠ {rel} changed after the turn ended — press r again to reject anyway \
                     (overwrites those changes)"
                ),
            );
        }
        Err(RejectError::Refused(why)) => status(app, format!("{rel}: cannot reject — {why}")),
    }
}

/// `A` / `R`: accept or reject every file not decided yet, and end the
/// review. Files that cannot be rejected are reported and stay undecided, so
/// the review stays open on them. A file changed after the turn needs a
/// second `R` on the desktop (`interactive`); remotely it is an error.
fn decide_rest(app: &mut App, reject: bool, interactive: bool) -> Result<(), String> {
    let Some(review) = active_review(app) else {
        return Ok(());
    };
    let undecided: Vec<usize> = review
        .set
        .files
        .iter()
        .enumerate()
        .filter(|(_, c)| decided(review, c).is_none())
        .map(|(i, _)| i)
        .collect();
    if !reject {
        for idx in undecided {
            accept_file(app, idx);
        }
        advance_or_finish(app);
        return Ok(());
    }

    let force = interactive && app.turn_review_view.confirm_force.as_deref() == Some("R");
    app.turn_review_view.confirm_force = None;
    let mut reverted = Vec::new();
    let mut drifted = Vec::new();
    let mut refused = Vec::new();
    for idx in undecided {
        let rel = active_review(app)
            .map(|r| r.set.files[idx].rel.clone())
            .unwrap_or_default();
        match revert_one(app, idx, force) {
            Ok(path) => reverted.push(path),
            Err(RejectError::Drifted) => drifted.push(rel),
            Err(RejectError::Refused(why)) => refused.push(format!("{rel} ({why})")),
        }
    }
    sync_editor(app, reverted.iter().map(std::path::PathBuf::as_path));
    if !drifted.is_empty() {
        // The other rejects are already on disk; keep their decisions.
        persist(app);
        let names = drifted.join(", ");
        if !interactive {
            return Err(format!(
                "{names} changed after the turn ended; reject on the desktop"
            ));
        }
        app.turn_review_view.confirm_force = Some("R".into());
        status(
            app,
            format!(
                "⚠ {names} changed after the turn ended — press R again to reject anyway \
                 (overwrites those changes)"
            ),
        );
        return Ok(());
    }
    let refused_msg = (!refused.is_empty()).then(|| {
        format!(
            "Cannot reject {} — accept them to finish",
            refused.join(", ")
        )
    });
    if let Some(msg) = &refused_msg {
        status(app, msg.clone());
    }
    advance_or_finish(app);
    match refused_msg {
        Some(msg) if !interactive => Err(msg),
        _ => Ok(()),
    }
}

/// Move the selection to the next undecided file (saving the decisions so
/// far), or end the review when every file has a decision.
fn advance_or_finish(app: &mut App) {
    let Some(review) = active_review(app) else {
        return;
    };
    let n = review.set.files.len();
    let start = app.turn_review_view.selected;
    let next = (1..=n)
        .map(|step| (start + step) % n)
        .find(|&i| decided(review, &review.set.files[i]).is_none());
    match next {
        Some(i) => {
            app.turn_review_view.selected = i;
            persist(app);
        }
        None => finish(app),
    }
}

/// Every file is decided (and rejections are already on disk): archive the
/// review, record it, and unblock the conversation.
fn finish(app: &mut App) {
    let active = app.turn_review_view.active;
    let Some(review) = app.pending_turn_reviews.get(active).cloned() else {
        return;
    };
    let resolved = review
        .set
        .files
        .iter()
        .map(|c| {
            let result = match decided(&review, c) {
                Some(FileDecision::Revert) => ResolvedDecision::Reverted,
                _ => ResolvedDecision::Kept,
            };
            (c.key(), result)
        })
        .collect();
    let reviewed = ReviewedTurn {
        set: review.set,
        resolved,
    };
    app.pending_turn_reviews.remove(active);
    app.turn_review_view = TurnReviewView::default();

    record_history(app, &reviewed);
    let resolved = report(app, &reviewed);
    // Writing the archive and collecting unreferenced blobs walks the whole
    // store: never on the event loop.
    app.turn_capture.archive_later(reviewed);
    app.remote
        .push_frame(gaviero_remote::envelope::ServerFrame::TurnReviewResolved(
            resolved,
        ));
    app.remote.bump_global();

    if app.pending_turn_reviews.is_empty() {
        app.left_panel = LeftPanelMode::FileTree;
    } else {
        show(app, 0);
    }
}

fn record_history(app: &App, reviewed: &ReviewedTurn) {
    use gaviero_core::history::{HistoryKind, ReviewDecision, TurnReview};
    let decisions = reviewed
        .set
        .files
        .iter()
        .map(|c| {
            let (result, detail) = match reviewed.resolved.get(&c.key()) {
                Some(ResolvedDecision::Kept) | None => ("kept", None),
                Some(ResolvedDecision::Reverted) => ("reverted", None),
                Some(ResolvedDecision::RevertedHunks(n)) => {
                    ("reverted_hunks", Some(format!("{n} hunk(s)")))
                }
                Some(ResolvedDecision::Failed(why)) => ("failed", Some(why.clone())),
            };
            ReviewDecision {
                path: c.rel.clone(),
                result: result.to_string(),
                detail,
            }
        })
        .collect();
    app.history.push_after_turn(
        reviewed.set.conv_id.as_deref(),
        &reviewed.set.turn_id,
        HistoryKind::TurnReview(TurnReview { decisions }),
    );
}

fn report(app: &mut App, reviewed: &ReviewedTurn) -> gaviero_remote::envelope::TurnReviewResolved {
    let mut kept = 0;
    let mut reverted = 0;
    let mut failed: Vec<String> = Vec::new();
    let mut touched: Vec<std::path::PathBuf> = Vec::new();
    for c in &reviewed.set.files {
        match reviewed.resolved.get(&c.key()) {
            Some(ResolvedDecision::Reverted | ResolvedDecision::RevertedHunks(_)) => {
                reverted += 1;
                touched.push(c.path.clone());
            }
            Some(ResolvedDecision::Failed(why)) => failed.push(format!("{}: {why}", c.rel)),
            _ => kept += 1,
        }
    }
    if !touched.is_empty() {
        super::agent_writes::reconcile_agent_writes(
            app,
            touched.iter().map(std::path::PathBuf::as_path),
            super::agent_writes::WriteOrigin::AgentTurn {
                source: "turn review",
            },
        );
    }
    let mut msg = format!("Turn review finalized — {kept} kept, {reverted} reverted");
    if !failed.is_empty() {
        msg.push_str(&format!(
            ", {} failed:\n{}",
            failed.len(),
            failed.join("\n")
        ));
    }
    if let Some(conv_id) = reviewed.set.conv_id.as_deref()
        && let Some(idx) = app.chat_state.find_conv_idx(conv_id)
    {
        app.chat_state.add_system_message_at(idx, &msg);
    }
    app.status_message = Some((
        msg.lines().next().unwrap_or_default().to_string(),
        std::time::Instant::now(),
    ));
    gaviero_remote::envelope::TurnReviewResolved {
        turn_id: reviewed.set.turn_id.clone(),
        conv_id: reviewed.set.conv_id.clone(),
        kept,
        reverted,
        failed,
    }
}

// ── Quick recovery (HISTORY → FILES, `u`) ────────────────────────────

/// Take a *reviewed* turn back to its pre-prompt state. Only the last
/// [`RETAINED_TURNS`](gaviero_core::turn_capture::RETAINED_TURNS) turns keep
/// their contents. The first call arms, a second within 5 s runs; a file
/// changed since (a later turn, the user) is skipped, never overwritten.
pub(super) fn undo_reviewed_turn(app: &mut App, turn_id: &str) -> String {
    use gaviero_core::turn_capture::{RETAINED_TURNS, RevertOutcome, revert_file};

    if app
        .pending_turn_reviews
        .iter()
        .any(|r| r.set.turn_id == turn_id)
    {
        return "This turn's review is still pending — decide there (TURN REVIEW panel)".into();
    }
    // The archive of a just-finished review may still be queued.
    app.turn_capture.flush();
    let Some(reviewed) = app
        .turn_capture
        .recent_reviewed()
        .into_iter()
        .find(|r| r.set.turn_id == turn_id)
    else {
        return format!(
            "No stored contents for this turn — only the last {RETAINED_TURNS} reviewed turns \
             can be undone"
        );
    };
    let armed = app
        .history_undo_armed
        .take()
        .is_some_and(|(id, at)| id == turn_id && at.elapsed().as_secs() < 5);
    if !armed {
        app.history_undo_armed = Some((turn_id.to_string(), std::time::Instant::now()));
        return format!(
            "Press u again to restore {} file(s) of this turn to their pre-prompt versions",
            reviewed.set.files.len()
        );
    }

    let mut restored = Vec::new();
    let mut skipped = Vec::new();
    for c in &reviewed.set.files {
        match revert_file(&app.turn_capture, c, false) {
            Ok(RevertOutcome::Reverted) => restored.push(c.path.clone()),
            Ok(RevertOutcome::AlreadyAtBefore) => {}
            Ok(RevertOutcome::Drifted) => skipped.push(format!("{} (changed since)", c.rel)),
            Ok(RevertOutcome::NotRevertible(why)) => skipped.push(format!("{} ({why})", c.rel)),
            Err(e) => skipped.push(format!("{} ({e:#})", c.rel)),
        }
    }
    if !restored.is_empty() {
        super::agent_writes::reconcile_agent_writes(
            app,
            restored.iter().map(std::path::PathBuf::as_path),
            super::agent_writes::WriteOrigin::AgentTurn {
                source: "turn undo",
            },
        );
    }
    let mut msg = format!("Undid turn: {} file(s) restored", restored.len());
    if !skipped.is_empty() {
        msg.push_str(&format!(", skipped: {}", skipped.join("; ")));
    }
    msg
}

// ── History panel: whole-file diff ───────────────────────────────────

/// `Enter` on a file in the HISTORY panel's FILES section: read both sides back
/// from the turn-capture blob store and open them as a read-only **diff tab**.
///
/// The viewer is [`crate::app::editing::open_diff_view`] — the one the git panel
/// already uses: a regular editor buffer holding the file with its hunks, with
/// tree-sitter syntax highlighting, line-number gutter, `+`/`-` row tints,
/// wrapping, fold suppression and the editor's own scrolling. The history panel
/// contributes no rendering of its own, so the app keeps one whole-file diff
/// viewer rather than two that would drift apart.
///
/// Handler-only, so the panel's render stays pure. The workspace's current copy
/// is never read: the log keeps hashes only, and content the store has already
/// reclaimed is reported as such rather than approximated from what is on disk
/// now. Which sides have text at all is core's rule
/// ([`gaviero_core::turn_capture::file_texts`]), shared with the turn-review
/// preview, so both callers agree on what "no diff" means.
pub(super) fn open_history_file_diff(app: &mut App) {
    let Some(file) = app.history_panel.selected_file().cloned() else {
        return;
    };
    match super::editing::open_change_diff(app, &recorded_change(app, &file)) {
        Ok(()) => status(
            app,
            format!(
                "{} — {} · diff of the turn's version",
                file.path, file.change
            ),
        ),
        // Same message the turn review shows for the same file.
        Err(why) => status(app, why),
    }
}

/// A history record's [`ChangedFile`] as the `FileChange` core's diff path
/// takes, so the history panel reaches the shared viewer through exactly the
/// type the turn review already holds.
///
/// A record keeps hashes only, so the two sides are named from them; the
/// absolute path comes from the workspace root the record was written relative
/// to. A record stores no binary flag, and core decides that from the bytes.
fn recorded_change(app: &App, file: &ChangedFile) -> FileChange {
    FileChange {
        path: workspace_path(app, &file.path),
        root: app.graph_workspace_root.clone().unwrap_or_default(),
        rel: file.path.clone(),
        kind: match file.change.as_str() {
            "added" => ChangeKind::Added,
            "deleted" => ChangeKind::Deleted,
            _ => ChangeKind::Modified,
        },
        before: file.before_sha256.as_deref().map(recorded_blob),
        after: file.after_sha256.as_deref().map(recorded_blob),
        revertible: file.revertible,
        binary: false,
        overlap_with: file.overlap_with.clone(),
    }
}

/// Open the file selected in the active review in the editor's diff tab — the
/// same viewer `Enter` opens from the history panel's FILES list.
pub(super) fn open_selected_change(app: &mut App) {
    let Some(change) = selected_change(app).cloned() else {
        return;
    };
    match super::editing::open_change_diff(app, &change) {
        Ok(()) => status(app, format!("{} — diff of the turn's version", change.rel)),
        Err(why) => status(app, why),
    }
}

/// A history record's root-relative, `/`-separated `path` as an absolute one.
///
/// The tab title, the language lookup (by extension) and the tab-reuse identity
/// all key off the path, so it has to be the real file's path and not the
/// record's spelling. Falls back to the record's own path when the workspace has
/// no root, which is what a bare `App::default()`-style test sees.
fn workspace_path(app: &App, rel: &str) -> std::path::PathBuf {
    let Some(root) = app.graph_workspace_root.as_deref() else {
        return std::path::PathBuf::from(rel);
    };
    // `Path::join` accepts `/` on every platform, but a record written on
    // Windows can carry a drive-relative prefix; rebuilding segment by segment
    // keeps the result under `root` either way.
    rel.split('/')
        .filter(|seg| !seg.is_empty() && *seg != ".")
        .fold(root.to_path_buf(), |p, seg| p.join(seg))
}

/// A hash the history record kept, as the blob reference core's diff path wants.
fn recorded_blob(sha256: &str) -> BlobRef {
    BlobRef {
        sha256: Some(sha256.to_string()),
        size: 0,
        stored: true,
    }
}

// ── Remote projection ────────────────────────────────────────────────

pub(crate) fn review_dto(review: &PendingReview) -> gaviero_remote::dto::TurnReview {
    use gaviero_remote::dto as rdto;
    rdto::TurnReview {
        turn_id: review.set.turn_id.clone(),
        conv_id: review.set.conv_id.clone(),
        outcome: match review.set.outcome {
            TurnOutcome::Completed => rdto::TurnOutcome::Completed,
            TurnOutcome::Cancelled => rdto::TurnOutcome::Cancelled,
            TurnOutcome::Failed => rdto::TurnOutcome::Failed,
        },
        files: review
            .set
            .files
            .iter()
            .map(|c| rdto::TurnReviewFile {
                path: c.rel.clone(),
                change: match c.kind {
                    ChangeKind::Added => rdto::TurnFileChange::Added,
                    ChangeKind::Modified => rdto::TurnFileChange::Modified,
                    ChangeKind::Deleted => rdto::TurnFileChange::Deleted,
                },
                decision: match decided(review, c) {
                    None => rdto::TurnFileDecision::Pending,
                    Some(FileDecision::Keep) => rdto::TurnFileDecision::Keep,
                    Some(FileDecision::Revert) => rdto::TurnFileDecision::Revert,
                    Some(FileDecision::RevertHunks(_)) => rdto::TurnFileDecision::RevertHunks,
                },
                revertible: c.revertible,
                binary: c.binary,
                overlap_with: c.overlap_with.clone(),
            })
            .collect(),
        warnings: review.set.warnings.clone(),
    }
}

/// Every pending review, for `snapshot.open_turn_reviews`.
pub(crate) fn open_reviews_dto(app: &App) -> Vec<gaviero_remote::dto::TurnReview> {
    app.pending_turn_reviews.iter().map(review_dto).collect()
}

fn push_review_frame(app: &mut App, turn_id: &str, created: bool) {
    use gaviero_remote::envelope::{ServerFrame, TurnReviewEvent};
    let Some(review) = app
        .pending_turn_reviews
        .iter()
        .find(|r| r.set.turn_id == turn_id)
    else {
        return;
    };
    let event = TurnReviewEvent {
        review: review_dto(review),
    };
    app.remote.push_frame(if created {
        ServerFrame::TurnReviewPending(event)
    } else {
        ServerFrame::TurnReviewUpdated(event)
    });
    app.remote.bump_global();
}

// ── Remote (phone) actions ───────────────────────────────────────────

/// Apply a decision sent by the remote client: the same four decisions as
/// the desktop (`keep_file` = accept, `revert_file` = reject, `keep_all` /
/// `revert_all` = accept / reject the rest). `finalize` (kept for 1.2
/// clients) accepts the rest. The phone never overwrites a file changed
/// after the turn — that confirmation is desktop-only.
pub(crate) fn apply_remote_action(
    app: &mut App,
    turn_id: &str,
    action: &str,
    path: Option<&str>,
) -> Result<(), String> {
    let Some(index) = app
        .pending_turn_reviews
        .iter()
        .position(|r| r.set.turn_id == turn_id)
    else {
        return Err("unknown turn review".to_string());
    };
    if app.turn_review_view.active != index {
        app.turn_review_view = TurnReviewView {
            active: index,
            ..TurnReviewView::default()
        };
    }
    let file_idx = |app: &App, rel: &str| -> Result<usize, String> {
        app.pending_turn_reviews[index]
            .set
            .files
            .iter()
            .position(|c| c.rel == rel || c.key() == rel)
            .ok_or_else(|| format!("{rel} is not part of this turn"))
    };
    match action {
        "keep_file" | "revert_file" => {
            let idx = file_idx(app, path.ok_or("path required")?)?;
            if let Some(label) = already_decided(app, idx) {
                // Absolute actions: repeating the same decision is a no-op.
                let same = (action == "keep_file") == (label == "accepted");
                return if same {
                    Ok(())
                } else {
                    Err(format!("already {label}"))
                };
            }
        }
        _ => {}
    }
    match action {
        "keep_file" => {
            let idx = file_idx(app, path.ok_or("path required")?)?;
            app.turn_review_view.selected = idx;
            accept_file(app, idx);
            advance_or_finish(app);
        }
        "revert_file" => {
            let idx = file_idx(app, path.ok_or("path required")?)?;
            app.turn_review_view.selected = idx;
            match reject_file(app, idx, false) {
                Ok(()) => advance_or_finish(app),
                Err(RejectError::Drifted) => {
                    return Err(
                        "the file changed after the turn ended; reject it on the desktop".into(),
                    );
                }
                Err(RejectError::Refused(why)) => return Err(why),
            }
        }
        "keep_all" | "finalize" => decide_rest(app, false, false)?,
        "revert_all" => decide_rest(app, true, false)?,
        other => return Err(format!("unknown action {other}")),
    }
    Ok(())
}

// ── Rendering (reads state only) ─────────────────────────────────────

pub(super) fn render_turn_review_list(app: &mut App, frame: &mut Frame, area: Rect, focused: bool) {
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

    let Some(review) = app.pending_turn_reviews.get(app.turn_review_view.active) else {
        Line::from(Span::styled(
            " No turn review pending",
            Style::default().fg(theme::TEXT_DIM),
        ))
        .render(inner, frame.buffer_mut());
        return;
    };

    let mut header: Vec<Line> = Vec::new();
    let title = review
        .set
        .conv_id
        .as_deref()
        .and_then(|c| app.chat_state.find_conv_idx(c))
        .map(|i| app.chat_state.conversations[i].title.clone())
        .unwrap_or_else(|| "conversation".to_string());
    let outcome = match review.set.outcome {
        TurnOutcome::Completed => ("", theme::TEXT_DIM),
        TurnOutcome::Cancelled => (
            "  CANCELLED — consider R (revert whole turn)",
            theme::WARNING,
        ),
        TurnOutcome::Failed => ("  FAILED — consider R (revert whole turn)", theme::ERROR),
    };
    header.push(Line::from(vec![
        Span::styled(
            format!(" {title}"),
            Style::default()
                .fg(theme::TEXT_FG)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(outcome.0, Style::default().fg(outcome.1)),
    ]));
    let pending_others = app.pending_turn_reviews.len().saturating_sub(1);
    header.push(Line::from(Span::styled(
        format!(
            " {} file(s){}",
            review.set.files.len(),
            if pending_others > 0 {
                format!("  · {pending_others} more review(s) — Tab")
            } else {
                String::new()
            }
        ),
        Style::default().fg(theme::TEXT_DIM),
    )));
    for w in &review.set.warnings {
        header.push(Line::from(Span::styled(
            format!(" ⚠ {w}"),
            Style::default().fg(theme::WARNING),
        )));
    }
    let header_h = (header.len() as u16).min(inner.height);
    for (i, line) in header.into_iter().take(header_h as usize).enumerate() {
        line.render(
            Rect {
                y: inner.y + i as u16,
                height: 1,
                ..inner
            },
            frame.buffer_mut(),
        );
    }

    let list = Rect {
        y: inner.y + header_h,
        height: inner.height.saturating_sub(header_h),
        ..inner
    };
    let visible = list.height as usize;
    let view = &mut app.turn_review_view;
    if visible > 0 {
        if view.selected < view.scroll_offset {
            view.scroll_offset = view.selected;
        } else if view.selected >= view.scroll_offset + visible {
            view.scroll_offset = view.selected + 1 - visible;
        }
    }
    let review = &app.pending_turn_reviews[app.turn_review_view.active];
    let view = &app.turn_review_view;
    for (row, (i, c)) in review
        .set
        .files
        .iter()
        .enumerate()
        .skip(view.scroll_offset)
        .take(visible)
        .enumerate()
    {
        let y = list.y + row as u16;
        let selected = i == view.selected;
        let (decision, decision_color) = match decided(review, c) {
            None => ("        ", theme::TEXT_DIM),
            Some(FileDecision::Keep) => ("accepted", theme::SUCCESS),
            Some(FileDecision::Revert) => ("rejected", theme::ERROR),
            Some(FileDecision::RevertHunks(_)) => ("partial ", theme::WARNING),
        };
        let (kind, kind_color) = match c.kind {
            ChangeKind::Added => ('A', theme::SUCCESS),
            ChangeKind::Modified => ('M', theme::WARNING),
            ChangeKind::Deleted => ('D', theme::ERROR),
        };
        let mut flags = String::new();
        if !c.overlap_with.is_empty() {
            flags.push_str(" ⚠overlap");
        }
        if !c.revertible {
            flags.push_str(" ⊘no-revert");
        }
        if c.binary {
            flags.push_str(" bin");
        }
        let name_style = if selected {
            Style::default()
                .fg(Color::White)
                .bg(theme::SELECTION_BG)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::TEXT_FG)
        };
        let line = Line::from(vec![
            Span::styled(format!(" {decision} "), Style::default().fg(decision_color)),
            Span::styled(format!("{kind} "), Style::default().fg(kind_color)),
            Span::styled(c.rel.clone(), name_style),
            Span::styled(flags, Style::default().fg(theme::WARNING)),
        ]);
        if selected {
            for x in list.x..list.right() {
                frame.buffer_mut()[(x, y)].set_bg(theme::SELECTION_BG);
            }
        }
        line.render(
            Rect {
                y,
                height: 1,
                ..list
            },
            frame.buffer_mut(),
        );
    }
}

/// Bottom status-bar hint while the TURN REVIEW list has focus.
pub(super) fn status_hint(app: &App) -> String {
    let (n, left) = active_review(app)
        .map(|r| {
            let left = r
                .set
                .files
                .iter()
                .filter(|c| decided(r, c).is_none())
                .count();
            (r.set.files.len(), left)
        })
        .unwrap_or((0, 0));
    format!(
        "TURN REVIEW ({left} of {n} left)  a / r: accept / reject file  \
         A / R: accept / reject whole turn  Enter (or click the row again): \
         read the whole file's diff"
    )
}

/// Select a row by mouse.
///
/// Returns the row's file index and whether the click landed on the row that
/// was *already* selected. A single click only moves the selection, because the
/// list is also how a file gets accepted or rejected (`a` / `r` on the focused
/// row); clicking the selected row again opens its diff, which is the same
/// "click to view" contract the git and history panels offer.
pub(super) fn click_row(app: &mut App, relative_row: usize) -> Option<(usize, bool)> {
    let header = 2 + active_review(app)
        .map(|r| r.set.warnings.len())
        .unwrap_or(0);
    let row = relative_row.checked_sub(header)?;
    let idx = app.turn_review_view.scroll_offset + row;
    let files = active_review(app).map(|r| r.set.files.len()).unwrap_or(0);
    if idx >= files {
        return None;
    }
    let was_selected = app.turn_review_view.selected == idx;
    app.turn_review_view.selected = idx;
    Some((idx, was_selected))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gaviero_core::turn_capture::CaptureScope;

    struct Fixture {
        dir: tempfile::TempDir,
        app: App,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join(".gaviero")).unwrap();
            std::fs::write(dir.path().join(".gaviero/settings.json"), "{}").unwrap();
            let app = Self::open(dir.path());
            Self { dir, app }
        }

        fn open(root: &std::path::Path) -> App {
            let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
            App::new(Workspace::single_folder(root.to_path_buf()), tx)
        }

        fn root(&self) -> &std::path::Path {
            self.dir.path()
        }

        fn write(&self, rel: &str, body: &str) {
            std::fs::write(self.root().join(rel), body).unwrap();
        }

        fn read(&self, rel: &str) -> Option<String> {
            std::fs::read_to_string(self.root().join(rel)).ok()
        }

        fn conv(&self) -> String {
            self.app.chat_state.active_conversation_id().to_string()
        }

        /// Run a "turn" whose agent does `edit`, then hand the result to the
        /// UI exactly as the dispatch task does.
        fn turn(&mut self, id: &str, edit: impl FnOnce(&Self)) {
            let scope = CaptureScope {
                roots: vec![self.root().to_path_buf()],
                excludes: vec![],
            };
            let conv = self.conv();
            let handle = self.app.turn_capture.begin(id, Some(&conv), scope).unwrap();
            edit(self);
            let end = self
                .app
                .turn_capture
                .end(handle, TurnOutcome::Completed)
                .unwrap();
            let review = PendingReview::new(end.set);
            self.app.turn_capture.save_pending(&review).unwrap();
            on_turn_review_pending(&mut self.app, review, end.overlapped_reviews);
        }

        fn key(&mut self, c: char) {
            handle_turn_review_action(&mut self.app, &Action::InsertChar(c));
        }

        fn select(&mut self, rel: &str) {
            let idx = self.app.pending_turn_reviews[self.app.turn_review_view.active]
                .set
                .files
                .iter()
                .position(|c| c.rel == rel)
                .unwrap();
            self.app.turn_review_view.selected = idx;
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Some(state) = session_state::state_dir_for(self.dir.path()) {
                let _ = std::fs::remove_dir_all(state);
            }
        }
    }

    #[test]
    fn a_turn_with_changes_blocks_its_conversation_until_finalized() {
        let mut f = Fixture::new();
        f.write("a.txt", "before\n");
        f.turn("t1", |f| f.write("a.txt", "after\n"));

        let conv = f.conv();
        assert!(conv_has_pending_review(&f.app, &conv));
        assert_eq!(f.app.left_panel, LeftPanelMode::TurnReview);
        let err = super::super::side_panel::dispatch_prompt_core(
            &mut f.app,
            &conv,
            "next".into(),
            vec![],
            false,
        )
        .unwrap_err();
        assert!(err.contains("awaiting review"), "{err}");

        f.key('A');
        assert!(!conv_has_pending_review(&f.app, &conv));
        assert_eq!(f.read("a.txt").as_deref(), Some("after\n"), "accepted");
        f.app.turn_capture.flush();
        assert!(f.app.turn_capture.load_pending().is_empty());
    }

    #[test]
    fn accepting_a_large_turn_archives_it_and_never_leaves_it_pending() {
        let mut f = Fixture::new();
        let names: Vec<String> = (0..150).map(|i| format!("f{i:03}.txt")).collect();
        for name in &names {
            f.write(name, "before\n");
        }
        f.turn("t1", |f| {
            for name in &names {
                f.write(name, "after\n");
            }
        });
        // One single decision first (queues a save), then the rest at once.
        f.select("f000.txt");
        f.key('a');
        f.key('A');
        assert!(!conv_has_pending_review(&f.app, &f.conv()));

        f.app.turn_capture.flush();
        assert!(
            f.app.turn_capture.load_pending().is_empty(),
            "a queued save must not land after the archive"
        );
        let archived = f.app.turn_capture.recent_reviewed();
        assert_eq!(archived[0].resolved.len(), names.len());
    }

    #[test]
    fn a_conversation_with_a_pending_review_cannot_be_closed() {
        let mut f = Fixture::new();
        f.write("a.txt", "v1\n");
        f.turn("t1", |f| f.write("a.txt", "v2\n"));
        let conv = f.conv();
        f.app.focus = Focus::SidePanel;
        f.app.side_panel = SidePanelMode::AgentChat;
        super::super::controller::handle_action(&mut f.app, Action::CloseTab);
        assert!(f.app.chat_state.find_conv_idx(&conv).is_some(), "tab kept");
        assert_eq!(f.app.left_panel, LeftPanelMode::TurnReview);

        f.key('A');
        f.app.focus = Focus::SidePanel;
        super::super::controller::handle_action(&mut f.app, Action::CloseTab);
        assert!(
            f.app.chat_state.find_conv_idx(&conv).is_none(),
            "closable once the review is finalized"
        );
    }

    #[test]
    fn desktop_send_keeps_the_typed_prompt_while_blocked() {
        let mut f = Fixture::new();
        f.write("a.txt", "before\n");
        f.turn("t1", |f| f.write("a.txt", "after\n"));
        f.app.chat_state.insert_str("my next prompt");
        super::super::side_panel::send_chat_message(&mut f.app);
        assert_eq!(f.app.chat_state.take_input(), "my next prompt");
    }

    #[test]
    fn revert_file_and_revert_all_restore_pre_prompt_content() {
        let mut f = Fixture::new();
        f.write("a.txt", "a0\n");
        f.write("b.txt", "b0\n");
        f.turn("t1", |f| {
            f.write("a.txt", "a1\n");
            f.write("b.txt", "b1\n");
            f.write("new.txt", "n\n");
        });
        // Reject one file: it goes back now, the review stays open on the rest.
        f.select("a.txt");
        f.key('r');
        assert_eq!(
            f.read("a.txt").as_deref(),
            Some("a0\n"),
            "applied immediately"
        );
        assert!(conv_has_pending_review(&f.app, &f.conv()));
        // Accept the whole turn: the remaining files stay, the review ends.
        f.key('A');
        assert_eq!(f.read("b.txt").as_deref(), Some("b1\n"));
        assert_eq!(f.read("new.txt").as_deref(), Some("n\n"));
        assert!(!conv_has_pending_review(&f.app, &f.conv()));

        // Reject the whole turn.
        f.turn("t2", |f| {
            f.write("b.txt", "b2\n");
            f.write("c.txt", "c\n");
        });
        f.key('R');
        assert_eq!(f.read("b.txt").as_deref(), Some("b1\n"));
        assert_eq!(f.read("c.txt"), None, "a created file is removed");
        assert!(!conv_has_pending_review(&f.app, &f.conv()));
    }

    #[test]
    fn the_review_ends_when_every_file_has_a_decision() {
        let mut f = Fixture::new();
        f.write("a.txt", "a0\n");
        f.write("b.txt", "b0\n");
        f.turn("t1", |f| {
            f.write("a.txt", "a1\n");
            f.write("b.txt", "b1\n");
        });
        f.select("a.txt");
        f.key('a');
        assert!(conv_has_pending_review(&f.app, &f.conv()));
        assert_eq!(
            selected_change(&f.app).map(|c| c.rel.clone()).as_deref(),
            Some("b.txt"),
            "the selection moves to the next undecided file"
        );
        // Decisions are final: going back to a.txt and rejecting does nothing.
        f.select("a.txt");
        f.key('r');
        assert_eq!(f.read("a.txt").as_deref(), Some("a1\n"));
        f.select("b.txt");
        f.key('r');
        assert!(!conv_has_pending_review(&f.app, &f.conv()));
        assert_eq!(f.read("a.txt").as_deref(), Some("a1\n"));
        assert_eq!(f.read("b.txt").as_deref(), Some("b0\n"));
        f.app.turn_capture.flush();
        let archived = f.app.turn_capture.recent_reviewed();
        assert_eq!(archived[0].resolved.len(), 2);
    }

    #[test]
    fn only_the_four_decisions_and_navigation_are_bound() {
        let mut f = Fixture::new();
        f.write("a.txt", "v1\n");
        f.turn("t1", |f| f.write("a.txt", "v2\n"));
        // `J`/`K` are in the list: the panel used to spend them scrolling a diff
        // preview of its own. The editor's diff tab scrolls itself now.
        for key in ['f', 'q', 'n', 'p', ' ', 'h', 'J', 'K'] {
            assert!(
                !handle_turn_review_action(&mut f.app, &Action::InsertChar(key)),
                "{key:?} is not a turn review action"
            );
        }
        for action in [Action::PageDown, Action::PageUp] {
            assert!(
                !handle_turn_review_action(&mut f.app, &action),
                "PageUp/PageDown are not turn review actions"
            );
        }
        // `Enter` reads rather than decides — it opens the file in the editor's
        // diff tab — so it is bound like the navigation keys.
        assert!(handle_turn_review_action(&mut f.app, &Action::Enter));
        assert!(conv_has_pending_review(&f.app, &f.conv()));
    }

    /// Draw the whole app and return the terminal contents.
    fn render_to_text(app: &mut App) -> String {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut terminal = Terminal::new(TestBackend::new(140, 30)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
        let buf = terminal.backend().buffer();
        let area = buf.area;
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    /// The review is a *list* panel. If it painted the editor area — as it used
    /// to, with a hunk preview of its own — the diff tab `Enter` opens would be
    /// drawn underneath it and the user would never see it. That was the bug.
    #[test]
    fn the_editor_area_shows_the_diff_tab_and_not_a_panel_preview() {
        let mut f = Fixture::new();
        f.write("a.txt", "v1\n");
        f.turn("t1", |f| f.write("a.txt", "keep\nold\n"));
        f.app.turn_capture.flush();
        assert!(handle_turn_review_action(&mut f.app, &Action::Enter));

        let text = render_to_text(&mut f.app);
        assert!(text.contains("v1"), "the pre-turn line is drawn: {text}");
        assert!(text.contains("keep"), "the post-turn line is drawn: {text}");
        assert!(
            !text.contains("@@ "),
            "no hunk header from a panel-painted diff: {text}"
        );
    }

    /// `Enter` hands the selected file to the shared read-only diff tab — the
    /// same viewer the git panel and the HISTORY panel open — instead of the
    /// review panel rendering a diff of its own.
    #[test]
    fn enter_opens_the_selected_file_in_the_editors_diff_tab() {
        let mut f = Fixture::new();
        f.write("a.txt", "v1\n");
        f.turn("t1", |f| f.write("a.txt", "v2\n"));
        f.app.turn_capture.flush();
        assert!(handle_turn_review_action(&mut f.app, &Action::Enter));
        let buf = f
            .app
            .buffers
            .iter()
            .find(|b| b.diff_view.is_some())
            .expect("Enter opens a diff-view tab");
        assert_eq!(buf.path.as_deref(), Some(f.root().join("a.txt").as_path()));
        assert_eq!(f.app.focus, Focus::Editor);
    }

    #[test]
    fn keys_reach_the_review_through_the_real_dispatch() {
        let mut f = Fixture::new();
        f.write("a.txt", "v1\n");
        f.turn("t1", |f| f.write("a.txt", "v2\n"));
        assert_eq!(f.app.focus, Focus::FileTree);
        super::super::controller::handle_action(&mut f.app, Action::InsertChar('R'));
        assert_eq!(f.read("a.txt").as_deref(), Some("v1\n"));

        f.turn("t2", |f| f.write("a.txt", "v3\n"));
        super::super::controller::handle_action(&mut f.app, Action::InsertChar('a'));
        assert!(!conv_has_pending_review(&f.app, &f.conv()));
        assert_eq!(f.read("a.txt").as_deref(), Some("v3\n"));
    }

    #[test]
    fn rejecting_a_file_edited_after_the_turn_needs_a_second_press() {
        let mut f = Fixture::new();
        f.write("a.txt", "v1\n");
        f.turn("t1", |f| f.write("a.txt", "v2\n"));
        f.write("a.txt", "v3 user\n");
        f.key('r');
        assert_eq!(
            f.read("a.txt").as_deref(),
            Some("v3 user\n"),
            "first r only warns"
        );
        assert!(conv_has_pending_review(&f.app, &f.conv()));
        f.key('j'); // any other key cancels
        f.key('r');
        assert_eq!(f.read("a.txt").as_deref(), Some("v3 user\n"), "asks again");
        f.key('r');
        assert_eq!(f.read("a.txt").as_deref(), Some("v1\n"));
        assert!(!conv_has_pending_review(&f.app, &f.conv()));
    }

    #[test]
    fn a_reviewed_turn_can_be_undone_from_history_with_confirmation() {
        let mut f = Fixture::new();
        f.write("a.txt", "v1\n");
        f.turn("t1", |f| f.write("a.txt", "v2\n"));
        f.key('a');
        assert_eq!(f.read("a.txt").as_deref(), Some("v2\n"));

        let first = undo_reviewed_turn(&mut f.app, "t1");
        assert!(first.contains("Press u again"), "{first}");
        assert_eq!(
            f.read("a.txt").as_deref(),
            Some("v2\n"),
            "first press only arms"
        );
        let second = undo_reviewed_turn(&mut f.app, "t1");
        assert!(second.contains("1 file(s) restored"), "{second}");
        assert_eq!(f.read("a.txt").as_deref(), Some("v1\n"));
    }

    #[test]
    fn the_phone_can_decide_and_finalize_but_not_prompt_while_pending() {
        use gaviero_remote::dto::{ErrorCode, TurnReviewActionKind as K};
        use gaviero_remote::envelope::TurnReviewAction;

        let mut f = Fixture::new();
        f.write("a.txt", "v1\n");
        f.turn("t1", |f| f.write("a.txt", "v2\n"));
        let conv = f.conv();

        let err = super::super::remote::apply_remote_prompt(&mut f.app, &conv, "next", 1 << 20)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::TurnReviewPending);
        let snap = super::super::projection::build_snapshot(&f.app);
        assert_eq!(snap.open_turn_reviews.len(), 1);
        assert_eq!(snap.open_turn_reviews[0].files[0].path, "a.txt");

        let act = |action, path: Option<&str>| TurnReviewAction {
            turn_id: "t1".into(),
            action,
            path: path.map(str::to_string),
        };
        assert_eq!(
            snap.open_turn_reviews[0].files[0].decision,
            gaviero_remote::dto::TurnFileDecision::Pending
        );

        // Rejecting the only file decides the turn: the review ends.
        super::super::remote::apply_turn_review_action(
            &mut f.app,
            &act(K::RevertFile, Some("a.txt")),
        )
        .unwrap();
        assert_eq!(f.read("a.txt").as_deref(), Some("v1\n"));
        assert!(!conv_has_pending_review(&f.app, &conv));
        assert!(
            super::super::projection::build_snapshot(&f.app)
                .open_turn_reviews
                .is_empty()
        );

        let err =
            super::super::remote::apply_turn_review_action(&mut f.app, &act(K::KeepAll, None))
                .unwrap_err();
        assert_eq!(err.code, ErrorCode::UnknownTurnReview);
    }

    #[test]
    fn the_phone_never_overwrites_a_file_changed_after_the_turn() {
        use gaviero_remote::dto::TurnReviewActionKind as K;
        use gaviero_remote::envelope::TurnReviewAction;

        let mut f = Fixture::new();
        f.write("a.txt", "v1\n");
        f.turn("t1", |f| f.write("a.txt", "v2\n"));
        f.write("a.txt", "v3 user\n");
        let reject_all = TurnReviewAction {
            turn_id: "t1".into(),
            action: K::RevertAll,
            path: None,
        };
        assert!(super::super::remote::apply_turn_review_action(&mut f.app, &reject_all).is_err());
        assert!(super::super::remote::apply_turn_review_action(&mut f.app, &reject_all).is_err());
        assert_eq!(f.read("a.txt").as_deref(), Some("v3 user\n"));
        assert!(conv_has_pending_review(&f.app, &f.conv()));
    }

    #[test]
    fn pending_review_and_decisions_survive_a_restart() {
        let mut f = Fixture::new();
        f.write("a.txt", "a1\n");
        f.write("b.txt", "b1\n");
        f.turn("t1", |f| {
            f.write("a.txt", "a2\n");
            f.write("b.txt", "b2\n");
        });
        f.select("a.txt");
        f.key('r');
        f.app.turn_capture.flush();

        let reopened = Fixture::open(f.root());
        assert_eq!(reopened.pending_turn_reviews.len(), 1);
        let review = &reopened.pending_turn_reviews[0];
        assert_eq!(review.set.turn_id, "t1");
        assert_eq!(review.decisions.len(), 1, "b.txt is still undecided");
        assert_eq!(
            review.decisions.values().next(),
            Some(&FileDecision::Revert),
            "decisions persist with the review"
        );
        assert_eq!(
            f.read("a.txt").as_deref(),
            Some("a1\n"),
            "the reject already applied"
        );
        assert_eq!(reopened.left_panel, LeftPanelMode::TurnReview);
    }
}
