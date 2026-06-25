//! Agent-finish notifications: terminal bell, desktop toast, status-bar banner.
//!
//! Bell and desktop toast fire only while the host terminal has keyboard focus
//! (crossterm `FocusGained` / `FocusLost`). When gaviero is backgrounded,
//! minimized, or covered by another fullscreen app, those audible/desktop
//! notifications are suppressed so they do not interrupt the user.

use gaviero_core::workspace::{Workspace, settings};
use std::path::Path;

/// Resolved notification preferences for agent-turn completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentFinishNotifyConfig {
    pub enabled: bool,
    pub sound: bool,
    pub desktop: bool,
    pub status_bar: bool,
}

impl Default for AgentFinishNotifyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            sound: true,
            desktop: true,
            status_bar: true,
        }
    }
}

pub fn resolve_config(workspace: &Workspace, root: Option<&Path>) -> AgentFinishNotifyConfig {
    let enabled = workspace
        .resolve_setting(settings::NOTIFICATIONS_AGENT_FINISHED_ENABLED, root)
        .as_bool()
        .unwrap_or(true);
    let sound = workspace
        .resolve_setting(settings::NOTIFICATIONS_AGENT_FINISHED_SOUND, root)
        .as_bool()
        .unwrap_or(true);
    let desktop = workspace
        .resolve_setting(settings::NOTIFICATIONS_AGENT_FINISHED_DESKTOP, root)
        .as_bool()
        .unwrap_or(true);
    let status_bar = workspace
        .resolve_setting(settings::NOTIFICATIONS_AGENT_FINISHED_STATUS_BAR, root)
        .as_bool()
        .unwrap_or(true);
    AgentFinishNotifyConfig {
        enabled,
        sound,
        desktop,
        status_bar,
    }
}

/// Play the terminal bell and/or spawn a desktop toast when the terminal is in
/// the foreground. Never raises the window.
pub fn notify_agent_finished(
    config: &AgentFinishNotifyConfig,
    terminal_focused: bool,
    title: &str,
    body: &str,
) {
    if !config.enabled || !terminal_focused {
        return;
    }
    if config.sound {
        play_terminal_bell();
    }
    if config.desktop {
        spawn_desktop_notification(title, body);
    }
}

pub fn play_terminal_bell() {
    use crossterm::style::Print;
    let _ = crossterm::execute!(std::io::stdout(), Print("\x07"));
}

/// Fire-and-forget desktop notification. Best-effort: missing `notify-send`
/// / `osascript` is silently ignored.
pub fn spawn_desktop_notification(title: &str, body: &str) {
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("notify-send")
            .args([
                "--urgency=low",
                "--hint=int:transient:1",
                "--app-name=Gaviero",
                title,
                body,
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            escape_applescript(body),
            escape_applescript(title),
        );
        let _ = std::process::Command::new("osascript")
            .args(["-e", &script])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
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
        let c = AgentFinishNotifyConfig::default();
        assert!(c.enabled && c.sound && c.desktop && c.status_bar);
    }

    #[test]
    fn notify_skipped_when_terminal_unfocused() {
        let c = AgentFinishNotifyConfig::default();
        notify_agent_finished(&c, false, "title", "body");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn applescript_escape_quotes() {
        assert_eq!(escape_applescript(r#"say "hi""#), r#"say \"hi\""#);
    }
}
