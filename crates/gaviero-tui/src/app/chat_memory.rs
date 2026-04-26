//! Chat memory helpers: builds finalized turn transcripts and schedules
//! conversation-run consolidation through the memory writer.

use gaviero_core::memory::consolidation::Consolidator;
use gaviero_core::memory::hash_path;

use super::App;
use crate::panels::agent_chat::ChatRole;

/// Build a full `user → assistant` transcript for the most recent turn of
/// `conv_id`. Returns `None` when the conversation has no preceding user
/// message (system-only message) or the assistant text is empty — same
/// skip conditions as `store_chat_turn`.
///
/// Used by the controller to feed `WriterMessage::TurnComplete` to the
/// per-turn extractor (Tier S / S3). Transcript shape is deliberately
/// plain so the extractor prompt can parse it reliably.
pub(crate) fn build_turn_transcript(
    app: &App,
    conv_id: &str,
    assistant_content: &str,
) -> Option<String> {
    let conv = app
        .chat_state
        .conversations
        .iter()
        .find(|c| c.id == conv_id)?;

    let user_text = conv
        .messages
        .iter()
        .rev()
        .find(|m| m.role == ChatRole::User)
        .map(|m| m.content.as_str())?;

    let assistant_text = assistant_content.trim();
    if assistant_text.is_empty() {
        return None;
    }

    Some(format!("USER: {user_text}\n\nASSISTANT: {assistant_text}"))
}

/// Run `Consolidator::consolidate_run` for the given conversation id.
///
/// Mirrors the post-swarm consolidation pass: promotes durable turns up-scope,
/// decays and prunes low-importance ones. Fires when a conversation is closed.
pub(crate) fn consolidate_conversation(app: &App, conv_id: &str) {
    let Some(ref memory) = app.memory else {
        return;
    };
    let Some(ref writer) = app.memory_writer else {
        return;
    };
    let Some(workspace_root) = app
        .graph_workspace_root
        .clone()
        .or_else(|| app.workspace.roots().first().map(|p| p.to_path_buf()))
    else {
        return;
    };

    let repo_id = hash_path(&workspace_root);
    let run_id = conv_id.to_string();
    let mem = memory.clone();
    let writer = writer.clone();

    tokio::spawn(async move {
        let consolidator = Consolidator::with_stores_and_writer(mem, writer);
        match consolidator.consolidate_run(&run_id, &repo_id).await {
            Ok(report) => {
                tracing::info!(
                    "consolidated conv {}: promoted={} pruned={}",
                    run_id,
                    report.promoted,
                    report.pruned
                );
            }
            Err(e) => {
                tracing::warn!("consolidate_run failed for {}: {}", run_id, e);
            }
        }
    });
}
