//! Agent notifications: sound, desktop toast, status-bar banner.
//!
//! Two trigger points, configured independently ([`NotifyEvent`]):
//!
//! - `AgentFinished` — the turn ended and an answer is on screen
//!   (`notifications.agentFinished.*`).
//! - `AgentWaiting` — the agent stopped mid-turn and the run is blocked on a
//!   permission decision or an `AskUserQuestion` answer
//!   (`notifications.agentWaiting.*`).
//!
//! **Sound ignores terminal focus.** The whole point of the alert is to reach a
//! user who has alt-tabbed away, so BEL / the system sound fires whether or not
//! gaviero has keyboard focus. The two events use different system sounds so
//! they are distinguishable without looking.
//!
//! **Desktop toasts stay focus-gated** (crossterm `FocusGained` / `FocusLost`):
//! unchanged from the original behaviour, where a backgrounded, minimized, or
//! fullscreen-covered gaviero must not stack toasts over whatever the user is
//! doing.

use gaviero_core::workspace::{Workspace, settings};
use std::path::Path;

/// Which agent milestone is being announced. Selects both the settings group
/// and the sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyEvent {
    /// Turn complete — an answer (or an error) is on screen.
    AgentFinished,
    /// Blocked mid-turn on a permission decision or a question.
    AgentWaiting,
}

impl NotifyEvent {
    /// `[enabled, sound, desktop, statusBar]` setting keys for this event.
    fn setting_keys(self) -> [&'static str; 4] {
        match self {
            NotifyEvent::AgentFinished => [
                settings::NOTIFICATIONS_AGENT_FINISHED_ENABLED,
                settings::NOTIFICATIONS_AGENT_FINISHED_SOUND,
                settings::NOTIFICATIONS_AGENT_FINISHED_DESKTOP,
                settings::NOTIFICATIONS_AGENT_FINISHED_STATUS_BAR,
            ],
            NotifyEvent::AgentWaiting => [
                settings::NOTIFICATIONS_AGENT_WAITING_ENABLED,
                settings::NOTIFICATIONS_AGENT_WAITING_SOUND,
                settings::NOTIFICATIONS_AGENT_WAITING_DESKTOP,
                settings::NOTIFICATIONS_AGENT_WAITING_STATUS_BAR,
            ],
        }
    }
}

/// How an audible notification is produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SoundStyle {
    /// Win32 system sound on Windows, terminal BEL elsewhere.
    #[default]
    Auto,
    /// Terminal BEL (`\x07`) only.
    Bell,
    /// Platform system sound only, falling back to BEL where none exists.
    System,
    /// BEL *and* the system sound — for hosts where either might be dropped.
    Both,
}

impl SoundStyle {
    fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "bell" => SoundStyle::Bell,
            "system" => SoundStyle::System,
            "both" => SoundStyle::Both,
            _ => SoundStyle::Auto,
        }
    }

    /// Collapse `Auto` to a concrete style. Windows prefers the system sound:
    /// BEL has to survive ConPTY plus whatever multiplexer is in the way, and
    /// Windows Terminal's `bellStyle` can silence it outright. Elsewhere BEL is
    /// the portable choice and needs no extra process or API.
    fn resolved(self) -> Self {
        match self {
            SoundStyle::Auto if cfg!(windows) => SoundStyle::System,
            SoundStyle::Auto => SoundStyle::Bell,
            other => other,
        }
    }

    fn wants_bell(self) -> bool {
        matches!(self.resolved(), SoundStyle::Bell | SoundStyle::Both)
    }

    fn wants_system(self) -> bool {
        matches!(self.resolved(), SoundStyle::System | SoundStyle::Both)
    }
}

/// Resolved notification preferences for one [`NotifyEvent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotifyConfig {
    pub enabled: bool,
    pub sound: bool,
    pub desktop: bool,
    pub status_bar: bool,
    pub sound_style: SoundStyle,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            sound: true,
            desktop: true,
            status_bar: true,
            sound_style: SoundStyle::Auto,
        }
    }
}

pub fn resolve_config(
    workspace: &Workspace,
    root: Option<&Path>,
    event: NotifyEvent,
) -> NotifyConfig {
    let [enabled_key, sound_key, desktop_key, status_bar_key] = event.setting_keys();
    let flag = |key: &str| {
        workspace
            .resolve_setting(key, root)
            .as_bool()
            .unwrap_or(true)
    };
    let style_raw = workspace.resolve_setting(settings::NOTIFICATIONS_SOUND_STYLE, root);
    NotifyConfig {
        enabled: flag(enabled_key),
        sound: flag(sound_key),
        desktop: flag(desktop_key),
        status_bar: flag(status_bar_key),
        sound_style: SoundStyle::parse(style_raw.as_str().unwrap_or("auto")),
    }
}

/// Play the notification sound and/or spawn a desktop toast. Never raises the
/// window. See the module docs for why only the toast honours
/// `terminal_focused`.
pub fn notify(
    config: &NotifyConfig,
    event: NotifyEvent,
    terminal_focused: bool,
    title: &str,
    body: &str,
) {
    if !config.enabled {
        return;
    }
    if config.sound {
        play_notification_sound(config.sound_style, event);
    }
    if config.desktop && terminal_focused {
        spawn_desktop_notification(title, body);
    }
}

pub fn play_notification_sound(style: SoundStyle, event: NotifyEvent) {
    let mut played = false;
    if style.wants_system() {
        played = play_system_sound(event);
    }
    // BEL also covers the "system sound requested on a platform that has none"
    // case, so an explicit `"system"` never leaves the user with silence.
    if style.wants_bell() || !played {
        play_terminal_bell();
    }
}

pub fn play_terminal_bell() {
    use crossterm::style::Print;
    let _ = crossterm::execute!(std::io::stdout(), Print("\x07"));
}

/// Play the platform system sound. Returns `false` when the platform has no
/// implementation (the caller then falls back to BEL).
///
/// Windows: `MessageBeep` is asynchronous — it queues the sound and returns
/// immediately — so it is safe to call from the event loop. Asterisk vs
/// Exclamation keeps "answer ready" and "needs you" apart by ear; both are
/// mapped in the stock Windows 11 sound scheme.
#[cfg(windows)]
fn play_system_sound(event: NotifyEvent) -> bool {
    // `MessageBeep` is filed under Diagnostics::Debug in win32metadata even
    // though it is a user32 sound call; its MB_* argument lives in
    // WindowsAndMessaging.
    use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONASTERISK, MB_ICONEXCLAMATION};
    let sound = match event {
        NotifyEvent::AgentFinished => MB_ICONASTERISK,
        NotifyEvent::AgentWaiting => MB_ICONEXCLAMATION,
    };
    unsafe { MessageBeep(sound) != 0 }
}

#[cfg(not(windows))]
fn play_system_sound(_event: NotifyEvent) -> bool {
    false
}

/// Fire-and-forget desktop notification. Best-effort: missing `notify-send`
/// / `osascript` is silently ignored.
///
/// Windows: intentionally a no-op in v1 (Tier W1 / PR-5, W-D6) — the
/// system sound and status-bar banner still fire. A WinRT toast needs
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
        assert!(c.enabled && c.sound && c.desktop && c.status_bar);
        assert_eq!(c.sound_style, SoundStyle::Auto);
    }

    #[test]
    fn disabled_config_is_silent() {
        let c = NotifyConfig {
            enabled: false,
            ..Default::default()
        };
        notify(&c, NotifyEvent::AgentWaiting, true, "title", "body");
    }

    #[test]
    fn sound_fires_when_terminal_unfocused() {
        // Regression guard for the focus gate: an unfocused terminal is
        // exactly when the alert matters most.
        let c = NotifyConfig::default();
        notify(&c, NotifyEvent::AgentFinished, false, "title", "body");
    }

    #[test]
    fn sound_style_parse_is_case_insensitive_and_defaults_to_auto() {
        assert_eq!(SoundStyle::parse("Bell"), SoundStyle::Bell);
        assert_eq!(SoundStyle::parse(" SYSTEM "), SoundStyle::System);
        assert_eq!(SoundStyle::parse("both"), SoundStyle::Both);
        assert_eq!(SoundStyle::parse("nonsense"), SoundStyle::Auto);
        assert_eq!(SoundStyle::parse(""), SoundStyle::Auto);
    }

    #[test]
    fn auto_resolves_per_platform() {
        let resolved = SoundStyle::Auto.resolved();
        if cfg!(windows) {
            assert_eq!(resolved, SoundStyle::System);
            assert!(SoundStyle::Auto.wants_system());
            assert!(!SoundStyle::Auto.wants_bell());
        } else {
            assert_eq!(resolved, SoundStyle::Bell);
            assert!(SoundStyle::Auto.wants_bell());
            assert!(!SoundStyle::Auto.wants_system());
        }
    }

    #[test]
    fn both_style_wants_every_channel() {
        assert!(SoundStyle::Both.wants_bell());
        assert!(SoundStyle::Both.wants_system());
    }

    #[test]
    fn finished_and_waiting_read_distinct_setting_groups() {
        let finished = NotifyEvent::AgentFinished.setting_keys();
        let waiting = NotifyEvent::AgentWaiting.setting_keys();
        for (f, w) in finished.iter().zip(waiting.iter()) {
            assert_ne!(f, w, "settings groups must not overlap");
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn no_system_sound_off_windows_so_bell_is_the_fallback() {
        assert!(!play_system_sound(NotifyEvent::AgentFinished));
    }

    /// Runtime check that the `MessageBeep` binding actually links and fires.
    /// Audible, so it is `#[ignore]`d like the other side-effecting tests —
    /// run with `cargo test -p gaviero-tui -- --ignored system_sound`. The
    /// sleep is only so a human can hear both sounds as distinct.
    #[cfg(windows)]
    #[test]
    #[ignore = "plays audible sounds"]
    fn system_sound_plays_on_windows() {
        assert!(play_system_sound(NotifyEvent::AgentFinished));
        std::thread::sleep(std::time::Duration::from_millis(900));
        assert!(play_system_sound(NotifyEvent::AgentWaiting));
    }

    /// End-to-end guard on the setting *names*: a typo in a key would silently
    /// fall back to `true` and be invisible in every other test here. Only
    /// keys written into the temp workspace are asserted, so the developer's
    /// own `~/.config/gaviero/settings.json` cannot influence the result.
    #[test]
    fn settings_file_overrides_reach_the_resolved_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".gaviero")).expect("mkdir .gaviero");
        std::fs::write(
            dir.path().join(".gaviero").join("settings.json"),
            r#"{
              "notifications": {
                "agentFinished": { "sound": true, "desktop": true },
                "agentWaiting": { "sound": false, "desktop": true },
                "sound": { "style": "both" }
              }
            }"#,
        )
        .expect("write settings.json");

        let ws = Workspace::single_folder(dir.path().to_path_buf());

        let waiting = resolve_config(&ws, None, NotifyEvent::AgentWaiting);
        assert!(!waiting.sound, "agentWaiting.sound override not applied");
        assert_eq!(waiting.sound_style, SoundStyle::Both);

        // Groups must be independent: silencing one must not silence the other.
        let finished = resolve_config(&ws, None, NotifyEvent::AgentFinished);
        assert!(finished.sound, "agentFinished.sound must stay independent");
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
