//! Chat memory write-back: persists a finalized user+assistant turn into MemoryStore.
//!
//! Triggered on `Event::MessageComplete` for assistant role. Stored content uses
//! caveman-style (terse, collapsed whitespace, truncated) to keep retrieval cheap.
//! Importance is scored via a regex heuristic over decision markers — high-signal
//! turns survive consolidation; low-signal turns decay.

use std::sync::Arc;

use gaviero_core::memory::consolidation::Consolidator;
use gaviero_core::memory::{hash_path, MemoryStore, StoreOptions};

use super::App;
use crate::panels::agent_chat::ChatRole;

/// Rough decision markers — when present in user or assistant text, importance is
/// boosted so `Consolidator` promotes the memory up-scope instead of decaying it.
const DECISION_MARKERS: &[&str] = &[
    "decided",
    "decision:",
    "architecture",
    "convention",
    "from now on",
    "must ",
    "must not",
    "never ",
    "always ",
    "will use",
    "we use",
    "prefer ",
    "don't use",
    "do not use",
    "avoid ",
    "migrate",
    "migration",
    "refactor",
    "deprecate",
];

const HIGH_IMPORTANCE: f32 = 0.7;
const LOW_IMPORTANCE: f32 = 0.3;

const MAX_PART_CHARS: usize = 400;

fn importance_from_text(text: &str) -> f32 {
    let lower = text.to_lowercase();
    if DECISION_MARKERS.iter().any(|m| lower.contains(m)) {
        HIGH_IMPORTANCE
    } else {
        LOW_IMPORTANCE
    }
}

/// Caveman-style: collapse whitespace runs, trim, truncate each side to `MAX_PART_CHARS`.
fn caveman_format(user: &str, assistant: &str) -> String {
    fn squeeze(s: &str) -> String {
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    }
    fn truncate(s: String) -> String {
        if s.chars().count() <= MAX_PART_CHARS {
            s
        } else {
            let cut: String = s.chars().take(MAX_PART_CHARS).collect();
            format!("{cut}…")
        }
    }
    let u = truncate(squeeze(user));
    let a = truncate(squeeze(assistant));
    format!("Q: {u}\nA: {a}")
}

/// Persist the most recent user+assistant turn of conversation `conv_id` to memory.
///
/// No-op if memory is not initialized, conversation is missing, or the turn has
/// no preceding user message (e.g. system-only message).
pub(crate) fn store_chat_turn(app: &App, conv_id: &str, assistant_content: &str) {
    let Some(ref memory) = app.memory else {
        return;
    };

    let Some(conv) = app
        .chat_state
        .conversations
        .iter()
        .find(|c| c.id == conv_id)
    else {
        return;
    };

    let last_user: Option<&str> = conv
        .messages
        .iter()
        .rev()
        .find(|m| m.role == ChatRole::User)
        .map(|m| m.content.as_str());

    let Some(user_text) = last_user else {
        return;
    };

    let assistant_text = assistant_content;
    if assistant_text.trim().is_empty() {
        return;
    }

    let content = caveman_format(user_text, assistant_text);
    let importance = importance_from_text(&content);

    let namespace = app.chat_state.agent_settings.write_namespace.clone();
    let key = format!(
        "chat:{conv_id}:{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0),
    );
    let opts = StoreOptions {
        privacy: "public".to_string(),
        importance,
        metadata: None,
        source_file: None,
        source_hash: None,
    };

    let mem: Arc<MemoryStore> = memory.clone();
    tokio::spawn(async move {
        if let Err(e) = mem.store_with_options(&namespace, &key, &content, &opts).await {
            tracing::warn!("chat memory write-back failed: {}", e);
        } else {
            tracing::debug!(
                "stored chat turn: ns={} importance={:.2} len={}",
                namespace,
                importance,
                content.len()
            );
        }
    });
}

/// Run `Consolidator::consolidate_run` for the given conversation id.
///
/// Mirrors the post-swarm consolidation pass: promotes durable turns up-scope,
/// decays and prunes low-importance ones. Fires when a conversation is closed.
pub(crate) fn consolidate_conversation(app: &App, conv_id: &str) {
    let Some(ref memory) = app.memory else {
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

    tokio::spawn(async move {
        let consolidator = Consolidator::new(mem);
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
