//! Agent notifications: desktop toast and status-bar banner. Silent by design —
//! gaviero never plays a sound (no terminal BEL, no system sound).
//!
//! Two trigger points, configured independently ([`NotifyEvent`]):
//!
//! - `AgentFinished` — the turn ended and an answer is on screen
//!   (`notifications.agentFinished.*`).
//! - `AgentWaiting` — the agent stopped mid-turn and the run is blocked on a
//!   permission decision or an `AskUserQuestion` answer
//!   (`notifications.agentWaiting.*`).
//!
//! Phone delivery goes through gaviero-relay: the controller hands the relay
//! client `{conv, kind}` and the Gaviero Remote app's alert service renders
//! it. This module only handles what happens on this machine.
//!
//! **Desktop toasts fire only while the terminal is unfocused** (crossterm
//! `FocusGained` / `FocusLost`). A toast exists to reach someone looking at
//! another window; with gaviero on screen the status-bar banner and the chat
//! panel already carry the news, so a toast there would be noise over the
//! window being read.

use gaviero_core::workspace::{Workspace, settings};
use std::path::Path;

/// Which agent milestone is being announced. Selects the settings group and
/// the banner style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyEvent {
    /// Turn complete — an answer (or an error) is on screen.
    AgentFinished,
    /// Blocked mid-turn on a permission decision or a question.
    AgentWaiting,
}

impl NotifyEvent {
    /// `[enabled, desktop, statusBar]` setting keys for this event.
    fn setting_keys(self) -> [&'static str; 3] {
        match self {
            NotifyEvent::AgentFinished => [
                settings::NOTIFICATIONS_AGENT_FINISHED_ENABLED,
                settings::NOTIFICATIONS_AGENT_FINISHED_DESKTOP,
                settings::NOTIFICATIONS_AGENT_FINISHED_STATUS_BAR,
            ],
            NotifyEvent::AgentWaiting => [
                settings::NOTIFICATIONS_AGENT_WAITING_ENABLED,
                settings::NOTIFICATIONS_AGENT_WAITING_DESKTOP,
                settings::NOTIFICATIONS_AGENT_WAITING_STATUS_BAR,
            ],
        }
    }
}

/// Resolved notification preferences for one [`NotifyEvent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotifyConfig {
    pub enabled: bool,
    pub desktop: bool,
    pub status_bar: bool,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            desktop: true,
            status_bar: true,
        }
    }
}

pub fn resolve_config(
    workspace: &Workspace,
    root: Option<&Path>,
    event: NotifyEvent,
) -> NotifyConfig {
    let [enabled_key, desktop_key, status_bar_key] = event.setting_keys();
    let flag = |key: &str| {
        workspace
            .resolve_setting(key, root)
            .as_bool()
            .unwrap_or(true)
    };
    NotifyConfig {
        enabled: flag(enabled_key),
        desktop: flag(desktop_key),
        status_bar: flag(status_bar_key),
    }
}

/// Spawn a desktop toast when the terminal is unfocused. Never raises the
/// window and never makes a sound.
pub fn notify(config: &NotifyConfig, terminal_focused: bool, title: &str, body: &str) {
    if !config.enabled {
        return;
    }
    if should_toast(config, terminal_focused) {
        spawn_desktop_notification(title, body);
    }
}

/// Whether to spawn a desktop toast. Split out from [`notify`] so the focus
/// gate is testable — `spawn_desktop_notification` is fire-and-forget and
/// reports nothing back.
fn should_toast(config: &NotifyConfig, terminal_focused: bool) -> bool {
    config.desktop && !terminal_focused
}

/// Fire-and-forget desktop notification. Best-effort: missing `notify-send`
/// / `osascript` is silently ignored.
///
/// Windows: intentionally a no-op in v1 (Tier W1 / PR-5, W-D6) — the
/// status-bar banner still fires. A WinRT toast needs
/// either a PowerShell `Windows.UI.Notifications` script (slow, flashes
/// a console) or a new dependency (`tauri-winrt-notification`); revisit
/// post-W1 if users ask.
pub fn spawn_desktop_notification(title: &str, body: &str) {
    #[cfg(windows)]
    let _ = (title, body);

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        // Enforced by the API rather than each caller: the body can carry
        // arbitrary backend error text, and a raw newline would break the
        // AppleScript string literal (silent osascript failure).
        let title = sanitize_control_chars(title);
        let body = sanitize_control_chars(body);

        #[cfg(target_os = "linux")]
        spawn_silent(
            "notify-send",
            &[
                "--urgency=low",
                "--hint=int:transient:1",
                "--app-name=Gaviero",
                &title,
                &body,
            ],
        );

        #[cfg(target_os = "macos")]
        {
            let script = format!(
                "display notification \"{}\" with title \"{}\"",
                escape_applescript(&body),
                escape_applescript(&title),
            );
            spawn_silent("osascript", &["-e", &script]);
        }
    }
}

/// Spawn a fire-and-forget helper with all stdio nulled. The child is handed
/// to a detached reaper thread: `std::process::Child` does not wait on drop,
/// so without the reaper every exited notifier would linger as a zombie for
/// the lifetime of the TUI process.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn spawn_silent(bin: &str, args: &[&str]) {
    use std::process::{Command, Stdio};
    let child = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(mut child) = child {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

/// Map control characters (newlines included) to spaces — toasts are
/// single-line surfaces on every backend.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sanitize_control_chars(s: &str) -> String {
    s.replace(|c: char| c.is_control(), " ")
}

#[cfg(target_os = "macos")]
fn escape_applescript(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_all_on() {
        let c = NotifyConfig::default();
        assert!(c.enabled && c.desktop && c.status_bar);
    }

    #[test]
    fn disabled_config_does_nothing() {
        let c = NotifyConfig {
            enabled: false,
            ..Default::default()
        };
        notify(&c, true, "title", "body");
    }

    #[test]
    fn toast_fires_only_while_terminal_unfocused() {
        let on = NotifyConfig::default();
        assert!(
            should_toast(&on, false),
            "away from gaviero — toast is the point"
        );
        assert!(
            !should_toast(&on, true),
            "gaviero on screen — the status-bar banner already says it"
        );

        let no_desktop = NotifyConfig {
            desktop: false,
            ..Default::default()
        };
        assert!(!should_toast(&no_desktop, false), "desktop=false wins");
    }

    #[test]
    fn finished_and_waiting_read_distinct_setting_groups() {
        let finished = NotifyEvent::AgentFinished.setting_keys();
        let waiting = NotifyEvent::AgentWaiting.setting_keys();
        for (f, w) in finished.iter().zip(waiting.iter()) {
            assert_ne!(f, w, "settings groups must not overlap");
        }
    }

    /// End-to-end guard on the setting *names*: a typo in a key would silently
    /// fall back to `true` and be invisible in every other test here. Only
    /// keys written into the temp workspace are asserted, so the developer's
    /// own `~/.gaviero/settings.json` cannot influence the result.
    #[test]
    fn settings_file_overrides_reach_the_resolved_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".gaviero")).expect("mkdir .gaviero");
        std::fs::write(
            dir.path().join(".gaviero").join("settings.json"),
            r#"{
              "notifications": {
                "agentFinished": { "desktop": true },
                "agentWaiting": { "desktop": false }
              }
            }"#,
        )
        .expect("write settings.json");

        let ws = Workspace::single_folder(dir.path().to_path_buf());

        let waiting = resolve_config(&ws, None, NotifyEvent::AgentWaiting);
        assert!(!waiting.desktop, "agentWaiting.desktop override not applied");

        // Groups must be independent: disabling one must not disable the other.
        let finished = resolve_config(&ws, None, NotifyEvent::AgentFinished);
        assert!(finished.desktop, "agentFinished.desktop must stay independent");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn applescript_escape_quotes() {
        assert_eq!(escape_applescript(r#"say "hi""#), r#"say \"hi\""#);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn control_chars_flattened_to_spaces() {
        assert_eq!(sanitize_control_chars("a\nb\r\tc"), "a b  c");
    }
}
