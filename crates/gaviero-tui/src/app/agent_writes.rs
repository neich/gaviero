//! Reconcile the editor with files an agent (or any other writer) put on disk.
//!
//! Agents write through their own tools, so by the time the host hears about an
//! edit the bytes are already on disk. There is no accept/reject decision left
//! to make. The only question is whether the editor's copy can be refreshed,
//! and that is a property of the buffer, not a preference of the user:
//!
//! | buffer state    | action                     |
//! |-----------------|----------------------------|
//! | open + clean    | reload from disk           |
//! | open + modified | keep the buffer, warn      |
//! | not open        | nothing (already on disk)  |
//!
//! **This module never writes to disk.** An agent turn is all-or-nothing: the
//! harness reverts every edit the turn made when it fails or is cancelled
//! (`agent_session/tool_agent/mod.rs`), so undoing a single file of a
//! *successful* turn from the UI would leave a tree the agent never produced.
//! Partial undo is therefore not offered at all. Providers that hold a write
//! gate keep their per-file accept/reject — but that is a *pre*-write choice
//! (`review.rs`), where "reject" means the file is never written rather than
//! written and then partially taken back.
//!
//! Every provider funnels through [`reconcile_agent_writes`]: in-process API
//! providers report the set of files they wrote (`ToolAgentEditsPending`), and
//! every other writer is observed one file at a time by the file watcher
//! (`Event::FileChanged`). Both get the same treatment, so an edit is handled
//! identically whichever agent made it and however it reached the host.

use super::*;

/// Where a set of on-disk writes came from, as far as the host can tell.
///
/// Wording only — this never changes what happens to a buffer.
#[derive(Debug, Clone, Copy)]
pub(super) enum WriteOrigin<'a> {
    /// A finished agent turn that reported the set of files it wrote, so the
    /// host knows the edits belong together.
    AgentTurn { source: &'a str },
    /// A single path changed on disk with no attribution (file watcher).
    Unattributed,
}

/// What reconciliation did to one path's buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathSync {
    /// Buffer was open and clean; it now matches disk.
    Reloaded,
    /// Buffer was open with unsaved edits; it was left alone.
    Conflict,
    /// Nothing for the editor to do (not open, already in sync, or unreadable).
    Untouched,
}

/// Bring every open buffer touched by `paths` back in line with disk, then
/// report what happened on the status line.
///
/// Buffers are matched with [`Buffer::paths_refer_to_same_file`] so a path the
/// caller spelled differently (absolute vs. relative, a different workspace
/// root) still finds its buffer.
pub(super) fn reconcile_agent_writes<'a>(
    app: &mut App,
    paths: impl IntoIterator<Item = &'a Path>,
    origin: WriteOrigin<'_>,
) {
    let mut reloaded: Vec<std::path::PathBuf> = Vec::new();
    let mut conflicted: Vec<std::path::PathBuf> = Vec::new();
    let mut total = 0_usize;

    for path in paths {
        total += 1;
        // No buffer: the file was never opened, and disk is already correct.
        let Some(idx) = buffer_index_for(app, path) else {
            continue;
        };
        match sync_buffer(app, idx, path, &origin) {
            PathSync::Reloaded => reloaded.push(path.to_path_buf()),
            PathSync::Conflict => conflicted.push(path.to_path_buf()),
            PathSync::Untouched => {}
        }
    }

    if let Some(msg) = summarize(&origin, &reloaded, &conflicted, total) {
        app.status_message = Some((msg, std::time::Instant::now()));
    }
}

/// Index of the buffer editing `path`, if the editor has it open.
fn buffer_index_for(app: &App, path: &Path) -> Option<usize> {
    app.buffers.iter().position(|b| {
        b.path
            .as_deref()
            .is_some_and(|buf_path| Buffer::paths_refer_to_same_file(buf_path, path))
    })
}

/// Converge one buffer on disk, or leave it alone and say why not.
fn sync_buffer(app: &mut App, idx: usize, path: &Path, origin: &WriteOrigin<'_>) -> PathSync {
    let disk_content = match std::fs::read_to_string(Buffer::resolve_editor_path(path)) {
        Ok(c) => c,
        // Deleted, unreadable, or not text: not a change the editor can adopt.
        Err(_) => return PathSync::Untouched,
    };

    let buf = &mut app.buffers[idx];

    // The post-open grace window exists to distrust the *watcher*: it replays
    // stale metadata right after a reopen, and a spurious reload would reset the
    // cursor and undo history for no reason. A turn that reported its writes is
    // not that — the host knows the file changed — so the window is skipped.
    if matches!(origin, WriteOrigin::Unattributed) && buf.should_suppress_post_open_watch() {
        buf.note_disk_sync(disk_content);
        return PathSync::Untouched;
    }
    if buf.should_ignore_external_change(&disk_content) {
        // Already in sync, or this is the echo of the editor's own save.
        buf.note_disk_sync(disk_content);
        return PathSync::Untouched;
    }
    if buf.modified {
        // `Buffer::reload` replaces the text wholesale and clears the undo
        // stack, so reloading would silently destroy unsaved edits. The agent's
        // write stands either way — this module never reverts it — so the only
        // useful thing left to do is tell the user their copy is now stale.
        return PathSync::Conflict;
    }

    match buf.reload() {
        Ok(()) => PathSync::Reloaded,
        Err(_) => PathSync::Untouched,
    }
}

/// One status line describing the whole reconciliation.
///
/// `None` means there is nothing the user needs to know: either no buffer was
/// involved, or the caller is the watcher reporting an unremarkable change to a
/// file the editor does not hold open (which would otherwise spam the status
/// bar on every build artefact, log rotation, and `git` operation).
fn summarize(
    origin: &WriteOrigin<'_>,
    reloaded: &[std::path::PathBuf],
    conflicted: &[std::path::PathBuf],
    total: usize,
) -> Option<String> {
    let actor = match origin {
        WriteOrigin::AgentTurn { source } => (*source).to_string(),
        WriteOrigin::Unattributed => "an external writer".to_string(),
    };

    if !conflicted.is_empty() {
        return Some(format!(
            "⚠ {actor} changed {} on disk while you had unsaved edits — your copy was kept",
            list_names(conflicted)
        ));
    }
    if !reloaded.is_empty() {
        return Some(format!(
            "{actor} changed {} — reloaded to match disk",
            list_names(reloaded)
        ));
    }
    // Only a known turn reports files the editor does not hold open; the
    // transcript already lists them, so keep this to a bare count.
    match origin {
        WriteOrigin::AgentTurn { .. } if total > 0 => Some(format!(
            "{actor} wrote {total} file{} — none open in the editor",
            if total == 1 { "" } else { "s" }
        )),
        _ => None,
    }
}

/// Join up to three file names for a status line, eliding the rest.
fn list_names(paths: &[std::path::PathBuf]) -> String {
    const MAX: usize = 3;
    let mut names: Vec<String> = paths
        .iter()
        .take(MAX)
        .map(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string())
        })
        .collect();
    if paths.len() > MAX {
        names.push(format!("+{} more", paths.len() - MAX));
    }
    names.join(", ")
}
