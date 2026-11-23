//! Automatic session consolidation: the opt-in idle trigger
//! (`memory.session.idleTimeoutSec`, 0 = off) and the bookkeeping that
//! keeps the idle, tab-close and manual paths from consolidating the
//! same replies twice.

use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub(crate) struct IdleTrack {
    /// `seq` of the newest Assistant message seen (0 = none).
    last_seq: u64,
    /// When `last_seq` last changed, or the conversation was last busy.
    changed_at: Instant,
    /// The idle trigger fires only for replies newer than this.
    done_seq: u64,
    /// Highest reply `seq` a session consolidation was started for.
    consolidated_seq: Option<u64>,
}

#[derive(Debug, Default)]
pub(crate) struct AutoConsolidateState {
    tracks: HashMap<String, IdleTrack>,
    last_check: Option<Instant>,
}

/// `seq` of the conversation's newest Assistant message, 0 when none.
/// `ChatMessage.seq` is monotonic and survives `/compact` and `/reset`.
pub(crate) fn last_assistant_seq(c: &crate::panels::agent_chat::Conversation) -> u64 {
    c.messages
        .iter()
        .rev()
        .find(|m| matches!(m.role, crate::panels::agent_chat::ChatRole::Assistant))
        .map_or(0, |m| m.seq)
}

/// Pure decision. `convs` yields `(id, last_assistant_seq, busy)` where
/// busy = streaming or background agents running. Returns `(id, seq)`
/// for each conversation due now; does NOT mark anything consolidated.
pub(crate) fn due_conversations<'a>(
    tracks: &mut HashMap<String, IdleTrack>,
    convs: impl Iterator<Item = (&'a str, u64, bool)>,
    now: Instant,
    timeout: Duration,
) -> Vec<(String, u64)> {
    let mut seen: Vec<&'a str> = Vec::new();
    let mut due = Vec::new();
    for (id, seq, busy) in convs {
        seen.push(id);
        let Some(track) = tracks.get_mut(id) else {
            // First sight: conversations restored at startup are never
            // consolidated on sight.
            tracks.insert(
                id.to_string(),
                IdleTrack {
                    last_seq: seq,
                    changed_at: now,
                    done_seq: seq,
                    consolidated_seq: None,
                },
            );
            continue;
        };
        if busy || seq != track.last_seq {
            track.last_seq = seq;
            track.changed_at = now;
            continue;
        }
        if seq > track.done_seq && now.duration_since(track.changed_at) >= timeout {
            due.push((id.to_string(), seq));
        }
    }
    tracks.retain(|id, _| seen.contains(&id.as_str()));
    due
}

/// Record that a consolidation was started for replies up to `seq`.
pub(crate) fn mark_consolidated(
    state: &mut AutoConsolidateState,
    conv_id: &str,
    seq: u64,
    now: Instant,
) {
    match state.tracks.get_mut(conv_id) {
        Some(track) => {
            track.done_seq = track.done_seq.max(seq);
            track.consolidated_seq = Some(track.consolidated_seq.unwrap_or(0).max(seq));
        }
        None => {
            state.tracks.insert(
                conv_id.to_string(),
                IdleTrack {
                    last_seq: seq,
                    changed_at: now,
                    done_seq: seq,
                    consolidated_seq: Some(seq),
                },
            );
        }
    }
}

/// True when a consolidation was already started covering `seq`.
pub(crate) fn already_consolidated(state: &AutoConsolidateState, conv_id: &str, seq: u64) -> bool {
    seq > 0
        && state
            .tracks
            .get(conv_id)
            .and_then(|t| t.consolidated_seq)
            .is_some_and(|c| c >= seq)
}

/// Primary root for setting resolution: the graph root, else the first
/// workspace folder (same rule as the close-path run triage).
fn primary_root(app: &super::App) -> Option<std::path::PathBuf> {
    app.graph_workspace_root
        .clone()
        .or_else(|| app.workspace.roots().first().map(|p| p.to_path_buf()))
}

/// Tab close: the session-consolidator request to run before run triage,
/// when `memory.session.consolidateOnClose` is on and the conversation has
/// assistant replies not yet covered by an earlier consolidation.
pub(crate) fn on_close_request(
    app: &mut super::App,
    conv_id: &str,
) -> Option<super::commands::SessionConsolidation> {
    let enabled = app
        .workspace
        .resolve_setting(
            gaviero_core::workspace::settings::MEMORY_SESSION_CONSOLIDATE_ON_CLOSE,
            primary_root(app).as_deref(),
        )
        .as_bool()
        .unwrap_or(false);
    if !enabled {
        return None;
    }
    let seq = app
        .chat_state
        .conversations
        .iter()
        .find(|c| c.id == conv_id)
        .map(last_assistant_seq)?;
    if seq == 0 || already_consolidated(&app.auto_consolidate, conv_id, seq) {
        return None;
    }
    let req = super::commands::auto_session_consolidation(app, conv_id)?;
    mark_consolidated(&mut app.auto_consolidate, conv_id, seq, Instant::now());
    Some(req)
}

/// Event-loop tick: at most once a second, start the session consolidator
/// for every conversation idle for `memory.session.idleTimeoutSec` seconds
/// since its last new assistant reply (0 = off).
pub(crate) fn on_tick(app: &mut super::App) {
    let now = Instant::now();
    if let Some(last) = app.auto_consolidate.last_check
        && now.duration_since(last) < Duration::from_secs(1)
    {
        return;
    }
    app.auto_consolidate.last_check = Some(now);
    let timeout = app
        .workspace
        .resolve_setting(
            gaviero_core::workspace::settings::MEMORY_SESSION_IDLE_TIMEOUT_SEC,
            primary_root(app).as_deref(),
        )
        .as_u64()
        .unwrap_or(0);
    if timeout == 0 {
        return;
    }
    let convs: Vec<(String, u64, bool)> = app
        .chat_state
        .conversations
        .iter()
        .map(|c| {
            (
                c.id.clone(),
                last_assistant_seq(c),
                c.is_streaming || c.has_running_background_agents(),
            )
        })
        .collect();
    let due = due_conversations(
        &mut app.auto_consolidate.tracks,
        convs.iter().map(|(id, s, b)| (id.as_str(), *s, *b)),
        now,
        Duration::from_secs(timeout),
    );
    for (id, seq) in due {
        // No request (writer not ready yet): leave it due for the next tick.
        let Some(req) = super::commands::auto_session_consolidation(app, &id) else {
            continue;
        };
        tokio::spawn(async move {
            match req.run().await {
                Ok(r) => tracing::info!("idle session consolidation: {r:?}"),
                Err(e) => tracing::warn!("idle session consolidation failed: {e}"),
            }
        });
        mark_consolidated(&mut app.auto_consolidate, &id, seq, now);
        app.status_message = Some((
            "Session consolidator started for an idle conversation".to_string(),
            Instant::now(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TIMEOUT: Duration = Duration::from_secs(90);

    fn at(t0: Instant, secs: u64) -> Instant {
        t0 + Duration::from_secs(secs)
    }

    fn check(
        tracks: &mut HashMap<String, IdleTrack>,
        seq: u64,
        busy: bool,
        now: Instant,
    ) -> Vec<(String, u64)> {
        due_conversations(tracks, [("c", seq, busy)].into_iter(), now, TIMEOUT)
    }

    #[test]
    fn first_sight_is_never_due() {
        let t0 = Instant::now();
        let mut tracks = HashMap::new();
        assert!(check(&mut tracks, 7, false, t0).is_empty());
        assert!(check(&mut tracks, 7, false, at(t0, 500)).is_empty());
    }

    #[test]
    fn due_after_new_reply_and_timeout() {
        let t0 = Instant::now();
        let mut state = AutoConsolidateState::default();
        assert!(check(&mut state.tracks, 2, false, t0).is_empty());
        assert!(check(&mut state.tracks, 4, false, at(t0, 1)).is_empty());
        assert_eq!(
            check(&mut state.tracks, 4, false, at(t0, 92)),
            vec![("c".to_string(), 4)]
        );
        mark_consolidated(&mut state, "c", 4, at(t0, 93));
        assert!(check(&mut state.tracks, 4, false, at(t0, 200)).is_empty());
    }

    #[test]
    fn busy_restarts_the_clock() {
        let t0 = Instant::now();
        let mut tracks = HashMap::new();
        check(&mut tracks, 2, false, t0);
        check(&mut tracks, 4, false, at(t0, 1));
        assert!(check(&mut tracks, 4, true, at(t0, 50)).is_empty());
        assert!(check(&mut tracks, 4, false, at(t0, 100)).is_empty());
        assert_eq!(
            check(&mut tracks, 4, false, at(t0, 141)),
            vec![("c".to_string(), 4)]
        );
    }

    #[test]
    fn no_new_reply_is_not_due() {
        let t0 = Instant::now();
        let mut tracks = HashMap::new();
        check(&mut tracks, 3, false, t0);
        for secs in [10, 91, 200, 500] {
            assert!(check(&mut tracks, 3, false, at(t0, secs)).is_empty());
        }
    }

    #[test]
    fn unmarked_due_stays_due() {
        let t0 = Instant::now();
        let mut tracks = HashMap::new();
        check(&mut tracks, 2, false, t0);
        check(&mut tracks, 4, false, at(t0, 1));
        assert_eq!(check(&mut tracks, 4, false, at(t0, 92)).len(), 1);
        assert_eq!(check(&mut tracks, 4, false, at(t0, 93)).len(), 1);
    }

    #[test]
    fn already_consolidated_follows_marks() {
        let t0 = Instant::now();
        let mut state = AutoConsolidateState::default();
        assert!(!already_consolidated(&state, "c", 4));
        mark_consolidated(&mut state, "c", 4, t0);
        assert!(already_consolidated(&state, "c", 4));
        assert!(!already_consolidated(&state, "c", 5));
        assert!(!already_consolidated(&state, "c", 0));
        assert!(!already_consolidated(&state, "other", 4));
    }
}
