//! Host-side tracking of provider subagents (Claude Task, Cursor Task, …).
//!
//! The TUI locks the prompt and lists running agents from
//! [`crate::observer::AcpObserver::on_background_task_started`]. Every
//! provider session that can spawn a subagent must register here so the
//! UI contract stays the same regardless of CLI.

use crate::observer::AcpObserver;

/// One in-flight background / subagent task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingBg {
    /// Id reported to the observer — stable for the lifetime of the task.
    /// Prefer `tool_use_id` so start/finish events join.
    pub host_id: String,
    pub task_id: String,
    pub tool_use_id: String,
    pub description: String,
}

pub(crate) fn bg_status(pending: &[PendingBg]) -> String {
    match pending.len() {
        0 => "Thinking...".into(),
        1 => format!("Background agent: {}", pending[0].description),
        n => format!("{n} background agents running"),
    }
}

pub(crate) fn register_pending_bg(
    pending: &mut Vec<PendingBg>,
    task_id: &str,
    tool_use_id: &str,
    description: &str,
    observer: &dyn AcpObserver,
) {
    if let Some(existing) = pending.iter_mut().find(|p| {
        (!tool_use_id.is_empty() && p.tool_use_id == tool_use_id)
            || (!task_id.is_empty() && (p.task_id == task_id || p.host_id == task_id))
    }) {
        if existing.task_id.is_empty() && !task_id.is_empty() {
            existing.task_id = task_id.to_string();
        }
        if existing.tool_use_id.is_empty() && !tool_use_id.is_empty() {
            existing.tool_use_id = tool_use_id.to_string();
        }
        if existing.description == "subagent" && !description.is_empty() {
            existing.description = description.to_string();
        }
        observer.on_streaming_status(&bg_status(pending));
        return;
    }
    let desc = if description.is_empty() {
        "subagent".to_string()
    } else {
        description.to_string()
    };
    let host_id = if !tool_use_id.is_empty() {
        tool_use_id.to_string()
    } else if !task_id.is_empty() {
        task_id.to_string()
    } else {
        format!("anon-{}", pending.len())
    };
    pending.push(PendingBg {
        host_id: host_id.clone(),
        task_id: task_id.to_string(),
        tool_use_id: tool_use_id.to_string(),
        description: desc.clone(),
    });
    observer.on_background_task_started(&host_id, &desc);
    observer.on_streaming_status(&bg_status(pending));
}

pub(crate) fn finish_pending_bg(
    pending: &mut Vec<PendingBg>,
    task_id: &str,
    tool_use_id: &str,
    status: &str,
    summary: &str,
    observer: &dyn AcpObserver,
) {
    let idx = pending.iter().position(|p| {
        (!task_id.is_empty() && (p.task_id == task_id || p.host_id == task_id))
            || (!tool_use_id.is_empty() && (p.tool_use_id == tool_use_id || p.host_id == tool_use_id))
    });
    let Some(idx) = idx else {
        return;
    };
    let p = pending.remove(idx);
    observer.on_background_task_finished(&p.host_id, status, summary);
    observer.on_streaming_status(&bg_status(pending));
}

/// Mark every still-running subagent as killed because the parent process
/// exited. Returns how many were in flight.
pub(crate) fn finish_all_pending_killed(
    pending: &mut Vec<PendingBg>,
    observer: &dyn AcpObserver,
) -> usize {
    let n = pending.len();
    for p in pending.drain(..) {
        observer.on_background_task_finished(&p.host_id, "killed", "parent process exited");
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    struct NoopObserver;
    impl AcpObserver for NoopObserver {
        fn on_stream_chunk(&self, _: &str) {}
        fn on_tool_call_started(&self, _: &str) {}
        fn on_streaming_status(&self, _: &str) {}
        fn on_message_complete(&self, _: &str, _: &str) {}
        fn on_proposal_deferred(&self, _: &Path, _: Option<&str>, _: &str) {}
    }

    #[test]
    fn register_pending_bg_dedupes_by_tool_use_id() {
        let mut pending = Vec::new();
        let obs = NoopObserver;
        register_pending_bg(&mut pending, "", "toolu_1", "search papers", &obs);
        register_pending_bg(&mut pending, "task_abc", "toolu_1", "search papers", &obs);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].host_id, "toolu_1");
        assert_eq!(pending[0].task_id, "task_abc");
        assert_eq!(pending[0].tool_use_id, "toolu_1");
    }

    #[test]
    fn finish_pending_bg_removes_matching_task() {
        let mut pending = Vec::new();
        let obs = NoopObserver;
        register_pending_bg(&mut pending, "t1", "toolu_1", "a", &obs);
        register_pending_bg(&mut pending, "t2", "toolu_2", "b", &obs);
        finish_pending_bg(&mut pending, "", "toolu_1", "completed", "", &obs);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].task_id, "t2");
        finish_pending_bg(&mut pending, "t2", "", "completed", "", &obs);
        assert!(pending.is_empty());
        assert_eq!(bg_status(&pending), "Thinking...");
    }

    #[test]
    fn bg_status_counts_running_agents() {
        let pending = vec![
            PendingBg {
                host_id: "u1".into(),
                task_id: "a".into(),
                tool_use_id: "u1".into(),
                description: "one".into(),
            },
            PendingBg {
                host_id: "u2".into(),
                task_id: "b".into(),
                tool_use_id: "u2".into(),
                description: "two".into(),
            },
        ];
        assert_eq!(bg_status(&pending), "2 background agents running");
        assert_eq!(bg_status(&pending[..1]), "Background agent: one");
    }

    #[test]
    fn finish_all_pending_killed_drains() {
        let mut pending = Vec::new();
        let obs = NoopObserver;
        register_pending_bg(&mut pending, "t1", "u1", "a", &obs);
        assert_eq!(finish_all_pending_killed(&mut pending, &obs), 1);
        assert!(pending.is_empty());
    }
}
