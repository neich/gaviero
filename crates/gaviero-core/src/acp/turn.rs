//! When does a Claude `--print` turn end?
//!
//! A prompt that launches background Agent/Task calls spans several parent
//! turns in one process: each `task_notification` wakes the parent for
//! another `system/init` … `result`. Verified on Claude Code 2.1.285:
//!
//! - **stream-json stdin** (interactive chat): every parent turn emits its
//!   `result` as it ends, so the first `result` arrives while the agents
//!   still run.
//! - **positional prompt** (auto-approve chat, swarm units): every `result`
//!   is held back until the agents finish, then all are flushed together
//!   just before the process exits.
//!
//! Each finished agent owes the parent exactly one wake-up turn, even when
//! several finish inside one parent turn — then the first wake-ups can be
//! empty (`result` "" after ~5 ms) and only the last one answers.
//!
//! So the turn is complete at a `result` once every parent turn has
//! reported one, no background agent is in flight, and no wake-up is still
//! owed. Stopping earlier drops the session, `kill_on_drop` takes the agents
//! with it, and the answer that summarises them is lost.
//!
//! A background launch's `tool_result` arrives immediately ("Async agent
//! launched successfully") and is only an ack; the agent finishes on its
//! `task_notification`. A refused launch (e.g. an unknown `subagent_type`)
//! answers with an error `tool_result` instead and never notifies, so it
//! stops counting there.
//!
//! A `run_in_background` shell command behaves the same way under stream-json
//! stdin (Claude Code 2.1.287): `task_started` (`task_type: local_bash`), an
//! ack ("Command running in background with ID: …"), the parent's `result`
//! ("Waiting for completion."), then `task_notification` and a wake-up turn
//! that reads the output. Under a positional prompt the CLI stops the command
//! (`status: stopped`) right after that first `result` and exits, so shells
//! are only tracked when the caller opts in with
//! [`TurnCompletion::waiting_on_shells`]. A shell counts from its
//! `task_started`, which also covers a foreground command the CLI moved to
//! the background.
//!
//! **Trust the CLI, not the tool input.** Claude Code 2.1.289 backgrounds an
//! `Agent` call by default: the model omits `run_in_background`, and only
//! `task_started` says `is_backgrounded: true` (`task_type: local_agent`).
//! Keying on the input flag let the first `result` end the turn and killed
//! the agents. So a parent-level tool becomes background on the CLI's
//! `is_backgrounded`; an explicit input flag only counts it early (and is
//! the fallback for CLIs that omit `is_backgrounded`). On top of that,
//! `background_tasks_changed` carries the CLI's own list of running
//! background tasks, and any task on it holds the turn even if the
//! per-tool classification missed it — so the next change in how Claude
//! labels its tools delays the turn instead of killing the agents.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use super::protocol::{
    BackgroundTaskInfo, StreamEvent, is_background_subagent_tool, is_subagent_tool_name,
};

/// How long to wait, once every background agent has finished and the
/// parent is idle, for Claude to start the turn that reads their results.
/// Claude emits that turn's `system/init` right after the last
/// `task_notification`; the grace only bounds a wake-up that never comes.
pub(crate) const BG_WAKE_GRACE: Duration = Duration::from_secs(15);

/// What an event means for the turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TurnProgress {
    Continue,
    /// Claude resumed the parent after a background agent finished.
    WakeTurn,
    /// This `result` ends the turn.
    Complete,
}

#[derive(Debug, Default)]
pub(crate) struct TurnCompletion {
    /// Hold the turn for background shell commands too.
    track_shells: bool,
    /// Parent-level `tool_use` ids that may yet become background tasks,
    /// mapped to whether the tool is a subagent (`Agent`/`Task`).
    parent_tools: HashMap<String, bool>,
    /// `tool_use` ids of parent-level background launches: Agent/Task, plus
    /// shell commands Claude reported as tasks when `track_shells` is set.
    bg_agents: HashSet<String>,
    /// The subset still running. Agents count from their launch (explicit
    /// flag) or their `task_started`; shells from `task_started`, which
    /// also covers a foreground command the CLI moved to the background,
    /// and never a launch it refused.
    running: HashSet<String>,
    /// `task_id`s of tasks started by a tracked launch.
    bg_task_ids: HashSet<String>,
    /// The CLI's latest `background_tasks_changed` list (shells dropped
    /// unless `track_shells`).
    cli_running: Vec<BackgroundTaskInfo>,
    /// Every `task_id` the CLI ever listed, so the `task_notification` of
    /// a task only the list knew about still owes its wake-up.
    cli_seen: HashSet<String>,
    /// Parent turns started (`system/init`).
    turns: u32,
    /// Parent turns reported (`result`).
    results: u32,
    /// Wake-up turns owed: one per finished agent, paid by each `system/init`.
    owed_wakes: u32,
}

impl TurnCompletion {
    /// Also hold the turn while a background shell command runs. Only for
    /// sessions whose prompt went in on stream-json stdin.
    pub(crate) fn waiting_on_shells() -> Self {
        Self {
            track_shells: true,
            ..Self::default()
        }
    }

    /// Feed one stream event. Error `result`s are the caller's to handle.
    pub(crate) fn observe(&mut self, event: &StreamEvent) -> TurnProgress {
        match event {
            StreamEvent::SystemInit { .. } => {
                let woke = self.turns > 0;
                self.turns += 1;
                if woke {
                    self.owed_wakes = self.owed_wakes.saturating_sub(1);
                    return TurnProgress::WakeTurn;
                }
            }
            // A subagent's own background launch wakes that subagent, not
            // the parent, so only parent-level launches are tracked.
            StreamEvent::AssistantMessage {
                tool_uses,
                parent_tool_use_id: None,
                ..
            } => {
                for tu in tool_uses.iter().filter(|tu| !tu.id.is_empty()) {
                    if is_background_subagent_tool(&tu.name, &tu.input) {
                        self.bg_agents.insert(tu.id.clone());
                        self.running.insert(tu.id.clone());
                    } else {
                        self.parent_tools
                            .insert(tu.id.clone(), is_subagent_tool_name(&tu.name));
                    }
                }
            }
            // A subagent's own tasks wake that subagent, not the parent.
            StreamEvent::TaskStarted {
                owned_by_subagent: true,
                ..
            } => {}
            // The CLI says the task ends with its tool call.
            StreamEvent::TaskStarted {
                tool_use_id,
                is_backgrounded: Some(false),
                ..
            } => {
                if self.bg_agents.remove(tool_use_id) {
                    self.running.remove(tool_use_id);
                }
            }
            // A finished agent can be woken again by its own background work.
            StreamEvent::TaskStarted {
                task_id,
                tool_use_id,
                is_backgrounded,
                ..
            } => {
                if let Some(&is_agent) = self.parent_tools.get(tool_use_id) {
                    // Without `is_backgrounded` (older CLIs) an agent needed
                    // the explicit input flag; a shell counts on its start.
                    let background = if is_agent {
                        *is_backgrounded == Some(true)
                    } else {
                        self.track_shells
                    };
                    if background {
                        self.parent_tools.remove(tool_use_id);
                        self.bg_agents.insert(tool_use_id.clone());
                    }
                }
                if self.bg_agents.contains(tool_use_id) {
                    self.running.insert(tool_use_id.clone());
                    if !task_id.is_empty() {
                        self.bg_task_ids.insert(task_id.clone());
                    }
                }
            }
            StreamEvent::BackgroundTasksChanged { tasks } => {
                self.cli_running = tasks
                    .iter()
                    .filter(|t| self.track_shells || t.task_type != "local_bash")
                    .cloned()
                    .collect();
                self.cli_seen
                    .extend(self.cli_running.iter().map(|t| t.task_id.clone()));
            }
            StreamEvent::TaskNotification {
                task_id,
                tool_use_id,
                ..
            } => {
                // Either a tracked launch, or one only the CLI's list knew
                // about: both owe the parent a wake-up.
                let untracked =
                    !self.bg_task_ids.contains(task_id) && self.cli_seen.remove(task_id);
                if self.running.remove(tool_use_id) || untracked {
                    self.owed_wakes += 1;
                }
            }
            // A launch Claude refused (unknown agent type, bad input) answers
            // with an error `tool_result` instead of an ack and never gets a
            // `task_notification`, so it must stop holding the turn here.
            StreamEvent::UserToolResults { results } => {
                for r in results.iter().filter(|r| r.is_error) {
                    if self.bg_agents.remove(&r.tool_use_id) {
                        self.running.remove(&r.tool_use_id);
                    }
                }
            }
            StreamEvent::ResultEvent { .. } => {
                self.results += 1;
                if self.parent_idle() && self.running.is_empty() {
                    let missed = self.unclassified_cli_tasks();
                    if !missed.is_empty() {
                        // Holding still works, but the classification above
                        // has drifted from the CLI again: say so in the log.
                        tracing::warn!(
                            tasks = ?missed,
                            "Claude still runs background tasks gaviero did not classify; holding the turn"
                        );
                    } else if self.owed_wakes == 0 {
                        return TurnProgress::Complete;
                    }
                }
            }
            _ => {}
        }
        TurnProgress::Continue
    }

    /// Tasks on the CLI's running list that no tracked launch accounts for.
    pub(crate) fn unclassified_cli_tasks(&self) -> Vec<&BackgroundTaskInfo> {
        self.cli_running
            .iter()
            .filter(|t| !self.bg_task_ids.contains(&t.task_id))
            .collect()
    }

    /// True for the immediate `tool_result` of a background launch.
    pub(crate) fn is_launch_ack(&self, tool_use_id: &str) -> bool {
        self.bg_agents.contains(tool_use_id)
    }

    /// Every agent is done and the parent is idle, but a wake-up is still
    /// owed: callers wait up to [`BG_WAKE_GRACE`] for it.
    pub(crate) fn awaiting_wake(&self) -> bool {
        self.turns > 0
            && self.parent_idle()
            && self.running.is_empty()
            && self.unclassified_cli_tasks().is_empty()
            && self.owed_wakes > 0
    }

    fn parent_idle(&self) -> bool {
        self.results >= self.turns
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::protocol::parse_stream_line;

    /// Index of the event that completes the turn, and whether the tracker
    /// is left waiting for a wake-up.
    fn completes_at(lines: &[&str]) -> (Option<usize>, bool) {
        let mut c = TurnCompletion::default();
        for (i, line) in lines.iter().enumerate() {
            if c.observe(&parse_stream_line(line).unwrap()) == TurnProgress::Complete {
                return (Some(i), c.awaiting_wake());
            }
        }
        (None, c.awaiting_wake())
    }

    const INIT: &str = r#"{"type":"system","subtype":"init","session_id":"s1","model":"m"}"#;
    const RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"ok"}"#;

    fn launch(id: &str) -> String {
        format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","id":"{id}","name":"Agent","input":{{"description":"{id}","run_in_background":true,"prompt":"go"}}}}]}}}}"#
        )
    }
    fn started(task: &str, id: &str) -> String {
        format!(
            r#"{{"type":"system","subtype":"task_started","task_id":"{task}","tool_use_id":"{id}","description":"{id}","is_backgrounded":true}}"#
        )
    }
    fn ack(id: &str) -> String {
        format!(
            r#"{{"type":"user","message":{{"role":"user","content":[{{"tool_use_id":"{id}","type":"tool_result","content":[{{"type":"text","text":"Async agent launched successfully."}}]}}]}}}}"#
        )
    }
    fn notify(task: &str, id: &str) -> String {
        format!(
            r#"{{"type":"system","subtype":"task_notification","task_id":"{task}","tool_use_id":"{id}","status":"completed","summary":"done"}}"#
        )
    }

    #[test]
    fn a_turn_without_background_agents_completes_at_its_result() {
        assert_eq!(completes_at(&[INIT, RESULT]), (Some(1), false));
    }

    #[test]
    fn a_turn_without_init_completes_at_its_result() {
        assert_eq!(completes_at(&[RESULT]), (Some(0), false));
    }

    /// Stream-json stdin order (Claude Code 2.1.285, two parallel agents):
    /// the first `result` comes while both run; each notification wakes
    /// the parent for another `init` … `result`.
    #[test]
    fn stdin_mode_holds_the_turn_until_the_final_wake_up() {
        let (la, sa, aa) = (launch("tu_a"), started("ta", "tu_a"), ack("tu_a"));
        let (lb, sb, ab) = (launch("tu_b"), started("tb", "tu_b"), ack("tu_b"));
        let (na, nb) = (notify("ta", "tu_a"), notify("tb", "tu_b"));
        let lines: &[&str] = &[
            INIT, &la, &sa, &aa, &lb, &sb, &ab, RESULT, // 7: agents still running
            &nb, INIT, RESULT, // 10: one agent still running
            &na, INIT, RESULT, // 13: done
        ];
        assert_eq!(completes_at(lines), (Some(13), false));
    }

    /// Positional-prompt order (Claude Code 2.1.285): wake-up turns start
    /// as agents finish, but every `result` is flushed at the very end.
    #[test]
    fn argv_mode_completes_at_the_last_flushed_result() {
        let (la, sa, aa) = (launch("tu_a"), started("ta", "tu_a"), ack("tu_a"));
        let (lb, sb, ab) = (launch("tu_b"), started("tb", "tu_b"), ack("tu_b"));
        let (na, nb) = (notify("ta", "tu_a"), notify("tb", "tu_b"));
        let lines: &[&str] = &[
            INIT, &la, &sa, &aa, &lb, &sb, &ab, &na, INIT, &nb, INIT, //
            RESULT, RESULT, RESULT, // 13: three turns, three results
        ];
        assert_eq!(completes_at(lines), (Some(13), false));
    }

    /// Both agents finished inside the first parent turn (Claude Code
    /// 2.1.285, positional prompt): the first wake-up is an empty no-op and
    /// only the second one answers.
    #[test]
    fn every_finished_agent_owes_one_wake_up_even_if_it_is_empty() {
        let (la, sa, aa) = (launch("tu_a"), started("ta", "tu_a"), ack("tu_a"));
        let (lb, sb, ab) = (launch("tu_b"), started("tb", "tu_b"), ack("tu_b"));
        let (na, nb) = (notify("ta", "tu_a"), notify("tb", "tu_b"));
        let empty = r#"{"type":"result","subtype":"success","is_error":false,"result":""}"#;
        let lines: &[&str] = &[
            INIT, &la, &sa, &aa, &lb, &sb, &ab, &na, &nb, RESULT, // 9: two wake-ups owed
            INIT, empty, // 11: one still owed
            INIT, RESULT, // 13
        ];
        assert_eq!(completes_at(lines), (Some(13), false));
        assert_eq!(completes_at(&lines[..12]), (None, true));
    }

    #[test]
    fn a_restarted_agent_holds_the_turn_again() {
        let (la, sa, aa) = (launch("tu_a"), started("ta", "tu_a"), ack("tu_a"));
        let na = notify("ta", "tu_a");
        let lines: &[&str] = &[
            INIT, &la, &sa, &aa, RESULT, &na, INIT, &sa, RESULT, // 8: agent restarted
            &na, INIT, RESULT, // 11
        ];
        assert_eq!(completes_at(lines), (Some(11), false));
    }

    #[test]
    fn an_agent_finishing_mid_turn_waits_for_the_queued_wake_up() {
        let (la, sa, aa) = (launch("tu_a"), started("ta", "tu_a"), ack("tu_a"));
        let na = notify("ta", "tu_a");
        assert_eq!(
            completes_at(&[INIT, &la, &sa, &aa, &na, RESULT]),
            (None, true)
        );
        assert_eq!(
            completes_at(&[INIT, &la, &sa, &aa, &na, RESULT, INIT, RESULT]),
            (Some(7), false)
        );
    }

    const BASH_LAUNCH: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tu_sh","name":"Bash","input":{"command":"sleep 25 && echo PROBE_DONE","run_in_background":true}}]}}"#;
    const BASH_STARTED: &str = r#"{"type":"system","subtype":"task_started","task_id":"bsh","tool_use_id":"tu_sh","description":"sleep 25 && echo PROBE_DONE","is_backgrounded":true,"task_type":"local_bash"}"#;
    const BASH_ACK: &str = r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"tu_sh","type":"tool_result","content":"Command running in background with ID: bsh."}]}}"#;
    const WAITING: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"Command is running in the background. Waiting for completion."}"#;

    fn completes_at_with_shells(lines: &[&str]) -> (Option<usize>, bool) {
        let mut c = TurnCompletion::waiting_on_shells();
        for (i, line) in lines.iter().enumerate() {
            if c.observe(&parse_stream_line(line).unwrap()) == TurnProgress::Complete {
                return (Some(i), c.awaiting_wake());
            }
        }
        (None, c.awaiting_wake())
    }

    /// Positional-prompt sessions (swarm units): the CLI stops the shell at
    /// the first `result`, so there is nothing to wait for.
    #[test]
    fn background_bash_does_not_hold_the_turn_by_default() {
        assert_eq!(
            completes_at(&[INIT, BASH_LAUNCH, BASH_STARTED, BASH_ACK, WAITING]),
            (Some(4), false)
        );
    }

    /// Stream-json stdin order recorded from Claude Code 2.1.287: the parent
    /// says it is waiting, the command finishes, and a wake-up turn reads
    /// its output and answers.
    #[test]
    fn a_waited_on_background_command_holds_the_turn_until_the_wake_up() {
        let notified = notify("bsh", "tu_sh");
        let read = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tu_cat","name":"Bash","input":{"command":"cat out"}}]}}"#;
        let read_result = r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"tu_cat","type":"tool_result","content":"PROBE_DONE"}]}}"#;
        let fin =
            r#"{"type":"result","subtype":"success","is_error":false,"result":"FINAL PROBE_DONE"}"#;
        let lines: &[&str] = &[
            INIT,
            BASH_LAUNCH,
            BASH_STARTED,
            BASH_ACK,
            WAITING, // 4: the command is still running
            &notified,
            INIT,
            read,
            read_result,
            fin, // 9
        ];
        assert_eq!(completes_at_with_shells(&lines[..5]), (None, false));
        assert_eq!(completes_at_with_shells(&lines[..6]), (None, true));
        assert_eq!(completes_at_with_shells(lines), (Some(9), false));
    }

    #[test]
    fn a_foreground_command_does_not_hold_the_turn() {
        let fg = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tu_fg","name":"Bash","input":{"command":"ls"}}]}}"#;
        assert_eq!(
            completes_at_with_shells(&[INIT, fg, RESULT]),
            (Some(2), false)
        );
    }

    /// The launch is only counted once Claude confirms the task, so a
    /// background command it refused does not hold the turn.
    #[test]
    fn a_background_command_that_never_started_does_not_hold_the_turn() {
        assert_eq!(
            completes_at_with_shells(&[INIT, BASH_LAUNCH, RESULT]),
            (Some(2), false)
        );
    }

    /// Claude moves a long foreground command to the background on its own;
    /// its `task_started` is what makes it a background command.
    #[test]
    fn a_command_moved_to_the_background_holds_the_turn() {
        let fg = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tu_sh","name":"Bash","input":{"command":"cargo build"}}]}}"#;
        let notified = notify("bsh", "tu_sh");
        let lines: &[&str] = &[
            INIT,
            fg,
            BASH_STARTED,
            BASH_ACK,
            WAITING,
            &notified,
            INIT,
            RESULT,
        ];
        assert_eq!(completes_at_with_shells(&lines[..5]), (None, false));
        assert_eq!(completes_at_with_shells(lines), (Some(7), false));
    }

    #[test]
    fn a_subagents_background_command_does_not_hold_the_parent() {
        let nested = r#"{"type":"assistant","parent_tool_use_id":"tu_a","message":{"content":[{"type":"tool_use","id":"tu_sh","name":"Bash","input":{"command":"sleep 9","run_in_background":true}}]}}"#;
        assert_eq!(
            completes_at_with_shells(&[INIT, nested, BASH_STARTED, RESULT]),
            (Some(3), false)
        );
    }

    #[test]
    fn background_command_acks_are_recognised() {
        let mut c = TurnCompletion::waiting_on_shells();
        c.observe(&parse_stream_line(BASH_LAUNCH).unwrap());
        c.observe(&parse_stream_line(BASH_STARTED).unwrap());
        assert!(c.is_launch_ack("tu_sh"));
        let mut agents_only = TurnCompletion::default();
        agents_only.observe(&parse_stream_line(BASH_LAUNCH).unwrap());
        agents_only.observe(&parse_stream_line(BASH_STARTED).unwrap());
        assert!(!agents_only.is_launch_ack("tu_sh"));
    }

    #[test]
    fn a_subagents_own_background_launch_does_not_hold_the_parent() {
        let (la, sa, aa) = (launch("tu_a"), started("ta", "tu_a"), ack("tu_a"));
        let nested = r#"{"type":"assistant","parent_tool_use_id":"tu_a","message":{"content":[{"type":"tool_use","id":"tu_n","name":"Agent","input":{"description":"n","run_in_background":true,"prompt":"go"}}]}}"#;
        let na = notify("ta", "tu_a");
        let lines: &[&str] = &[INIT, &la, &sa, &aa, nested, RESULT, &na, INIT, RESULT];
        assert_eq!(completes_at(lines), (Some(8), false));
    }

    /// A background launch Claude refused (here an unknown `subagent_type`)
    /// gets an error `tool_result` and never a `task_notification`. It used
    /// to stay in `running` forever, so the turn never completed and the
    /// wake-up grace never armed: the chat hung after its final answer.
    #[test]
    fn a_refused_background_launch_does_not_hold_the_turn() {
        let (la, sa, aa) = (launch("tu_a"), started("ta", "tu_a"), ack("tu_a"));
        let na = notify("ta", "tu_a");
        let lx = launch("tu_x");
        let refused = r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"tu_x","type":"tool_result","is_error":true,"content":"<tool_use_error>Agent type 'nonexistent-skip' not found.</tool_use_error>"}]}}"#;
        let lines: &[&str] = &[
            INIT, &la, &sa, &aa, &lx, refused, RESULT, // 6: tu_a still running
            &na, INIT, RESULT, // 9
        ];
        assert_eq!(completes_at(&lines[..7]), (None, false));
        assert_eq!(completes_at(lines), (Some(9), false));

        let mut c = TurnCompletion::default();
        for line in &lines[..6] {
            c.observe(&parse_stream_line(line).unwrap());
        }
        assert!(
            !c.is_launch_ack("tu_x"),
            "refused launch is no longer an ack"
        );
        assert!(c.is_launch_ack("tu_a"));
    }

    /// The only refused launch in a turn: the first `result` ends it.
    #[test]
    fn a_turn_whose_only_background_launch_was_refused_completes() {
        let lx = launch("tu_x");
        let refused = r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"tu_x","type":"tool_result","is_error":true,"content":"not found"}]}}"#;
        assert_eq!(
            completes_at(&[INIT, &lx, refused, RESULT]),
            (Some(3), false)
        );
    }

    /// Recorded from Claude Code 2.1.289 (stream-json stdin, fields
    /// trimmed): the model omits `run_in_background`, the CLI backgrounds
    /// the agent anyway, and the parent's first `result` lands 20 s before
    /// the agent finishes. Ending the turn there killed the agents.
    #[test]
    fn a_default_background_agent_holds_the_turn() {
        let lines: &[&str] = &[
            INIT,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tu_a","name":"Agent","input":{"description":"probe-a","subagent_type":"general-purpose","prompt":"go"}}]}}"#,
            r#"{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"ta","task_type":"local_agent","description":"probe-a"}]}"#,
            r#"{"type":"system","subtype":"task_started","task_id":"ta","tool_use_id":"tu_a","description":"probe-a","subagent_type":"general-purpose","is_backgrounded":true,"spawn_depth":1,"task_type":"local_agent"}"#,
            &ack("tu_a"),
            r#"{"type":"assistant","parent_tool_use_id":"tu_a","message":{"content":[{"type":"tool_use","id":"tu_sl","name":"Bash","input":{"command":"sleep 20"}}]}}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"result":"DONE-LAUNCHING"}"#, // 6
            r#"{"type":"system","subtype":"task_started","task_id":"tsl","owned_by_subagent":true,"tool_use_id":"tu_sl","description":"Sleep","is_backgrounded":false,"task_type":"local_bash"}"#,
            &notify("tsl", "tu_sl"),
            r#"{"type":"system","subtype":"background_tasks_changed","tasks":[]}"#,
            &notify("ta", "tu_a"),
            INIT,
            RESULT, // 12
        ];
        assert_eq!(completes_at(&lines[..7]), (None, false));
        assert_eq!(completes_at(&lines[..11]), (None, true));
        assert_eq!(completes_at(lines), (Some(12), false));
        assert_eq!(completes_at_with_shells(lines), (Some(12), false));

        let mut c = TurnCompletion::default();
        for line in &lines[..4] {
            c.observe(&parse_stream_line(line).unwrap());
        }
        assert!(c.is_launch_ack("tu_a"), "the ack is not the agent's result");
    }

    /// `is_backgrounded: false` is the CLI saying the call is foreground,
    /// even when the input asked for the background.
    #[test]
    fn a_foreground_agent_does_not_hold_the_turn() {
        let fg = r#"{"type":"system","subtype":"task_started","task_id":"ta","tool_use_id":"tu_a","description":"a","is_backgrounded":false,"task_type":"local_agent"}"#;
        let plain = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tu_a","name":"Agent","input":{"description":"a","prompt":"go"}}]}}"#;
        assert_eq!(completes_at(&[INIT, plain, fg, RESULT]), (Some(3), false));
        assert_eq!(
            completes_at(&[INIT, &launch("tu_a"), fg, RESULT]),
            (Some(3), false)
        );
    }

    /// Older CLIs omit `is_backgrounded`: an agent then needs the explicit
    /// input flag, as before.
    #[test]
    fn without_is_backgrounded_an_unflagged_agent_does_not_hold_the_turn() {
        let plain = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tu_a","name":"Agent","input":{"description":"a","prompt":"go"}}]}}"#;
        let legacy = r#"{"type":"system","subtype":"task_started","task_id":"ta","tool_use_id":"tu_a","description":"a"}"#;
        assert_eq!(
            completes_at(&[INIT, plain, legacy, RESULT]),
            (Some(3), false)
        );
    }

    /// Safety net: a task the per-tool classification knows nothing about
    /// (here a hypothetical new tool) still holds the turn while the CLI
    /// lists it, and its notification owes the parent a wake-up.
    #[test]
    fn a_task_only_the_cli_list_knows_holds_the_turn() {
        let tool = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tu_m","name":"Monitor","input":{}}]}}"#;
        let listed = r#"{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"tm","task_type":"local_monitor","description":"watch"}]}"#;
        let cleared = r#"{"type":"system","subtype":"background_tasks_changed","tasks":[]}"#;
        let notified = notify("tm", "tu_m");
        let lines: &[&str] = &[
            INIT, tool, listed, RESULT, // 3: still listed
            cleared, &notified, INIT, RESULT, // 7
        ];
        assert_eq!(completes_at(&lines[..4]), (None, false));
        assert_eq!(completes_at(&lines[..6]), (None, true));
        assert_eq!(completes_at(lines), (Some(7), false));
    }

    /// Positional-prompt sessions: the CLI stops background shells at the
    /// first `result`, so a listed shell must not hold the turn there.
    #[test]
    fn a_listed_shell_does_not_hold_a_positional_turn() {
        let listed = r#"{"type":"system","subtype":"background_tasks_changed","tasks":[{"task_id":"bsh","task_type":"local_bash","description":"sleep"}]}"#;
        assert_eq!(
            completes_at(&[INIT, BASH_LAUNCH, listed, BASH_STARTED, BASH_ACK, WAITING]),
            (Some(5), false)
        );
    }

    #[test]
    fn launch_acks_are_recognised() {
        let mut c = TurnCompletion::default();
        c.observe(&parse_stream_line(&launch("tu_a")).unwrap());
        assert!(c.is_launch_ack("tu_a"));
        assert!(!c.is_launch_ack("tu_other"));
    }
}
