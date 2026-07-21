//! Platform-specific terminal workarounds, centralized.
//!
//! Every Windows/ConPTY quirk the TUI must work around lives here (or, for
//! the paste-burst coalescer, is documented here and implemented next to its
//! only consumer in [`crate::event`]). Keeping the workarounds in one module
//! makes the platform surface auditable: `main.rs`, `keymap.rs`, and the
//! panels call small named helpers instead of scattering `cfg` blocks.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::io::Write;

/// VT mouse-mode sequences (normal + button-drag tracking, SGR encoding).
/// crossterm's `EnableMouseCapture` on Windows only sets the WinAPI console
/// mode (`is_ansi_code_supported` is hardwired to `false`); under Windows
/// Terminal/ConPTY that never reaches the hosting terminal, which then keeps
/// its own native drag-selection — a full-window-width highlight that ignores
/// panel boundaries — instead of forwarding mouse events to the app. Writing
/// the sequences explicitly makes ConPTY pass the request through.
///
/// Under a multiplexer these sequences reach the mux, not the host terminal.
/// psmux forwards drags to alt-screen panes only when its own client-side
/// drag-selection is disabled: `set -g mouse-selection off` (default is on).
/// With it on, the psmux client swallows every left-drag after the initial
/// press and paints its own unclamped highlight — the same full-window
/// symptom — and no gaviero-side sequence can override that.
///
/// `?1003h` (any-motion tracking) and `?1015h` (urxvt encoding) that
/// crossterm's Unix path also emits are deliberately omitted: nothing in the
/// crate consumes `MouseEventKind::Moved`.
#[cfg(windows)]
const ENABLE_VT_MOUSE: &str = "\x1b[?1000h\x1b[?1002h\x1b[?1006h";
#[cfg(windows)]
const DISABLE_VT_MOUSE: &str = "\x1b[?1006l\x1b[?1002l\x1b[?1000l";

/// Ask the hosting terminal to forward mouse events (Windows-only VT write;
/// no-op elsewhere, where crossterm's `EnableMouseCapture` already emits the
/// sequences). Must run after `EnterAlternateScreen`/`EnableMouseCapture`.
///
/// Called once at startup AND re-asserted while running (throttled Tick +
/// host focus-gain — `App::maybe_reassert_vt_mouse`): a multiplexer keeps
/// its own per-pane record of these modes (psmux gates click forwarding on
/// it) and can lose it without any signal to gaviero, e.g. on a pane
/// respawn or vt100 state reset. Note the Jul 2026 "selection dead
/// app-wide" incident turned out to be one layer up — Windows Terminal
/// dropped the *psmux client's* mouse registration, which psmux only
/// re-arms in SSH mode — so this keep-alive could not have fixed it; it
/// remains as cheap insurance for the pane-level loss it does cover. The
/// sequences are idempotent, so re-writing is safe on every host.
#[cfg(windows)]
pub fn enable_vt_mouse_passthrough(w: &mut impl Write) -> std::io::Result<()> {
    w.write_all(ENABLE_VT_MOUSE.as_bytes())?;
    w.flush()
}

#[cfg(not(windows))]
pub fn enable_vt_mouse_passthrough(_w: &mut impl Write) -> std::io::Result<()> {
    Ok(())
}

/// Counterpart of [`enable_vt_mouse_passthrough`]; called from every
/// terminal-restore path.
#[cfg(windows)]
pub fn disable_vt_mouse_passthrough(w: &mut impl Write) -> std::io::Result<()> {
    w.write_all(DISABLE_VT_MOUSE.as_bytes())?;
    w.flush()
}

#[cfg(not(windows))]
pub fn disable_vt_mouse_passthrough(_w: &mut impl Write) -> std::io::Result<()> {
    Ok(())
}

/// Install a Windows console-control handler that turns Ctrl+C / Ctrl+Break
/// *signals* back into key events instead of letting them terminate the TUI.
///
/// With crossterm raw mode active (`ENABLE_PROCESSED_INPUT` cleared), Ctrl+C
/// arrives as an ordinary key event and the keymap gives it its in-app
/// meaning (cancel a streaming chat turn / copy). But that mode lives on the
/// *console*, which is shared with every attached process; if something
/// flips it back to processed mode behind our back, conhost instead
/// broadcasts a `CTRL_C_EVENT` — and a handler-less process is terminated on
/// the spot ("Ctrl+C killed gaviero mid-prompt", W1). Ctrl+Break is
/// delivered as a control event *regardless* of raw mode, so it was always
/// fatal. Agent subprocess trees are detached from the console at the spawn
/// layer (`CREATE_NO_WINDOW`, `gaviero_core::util::spawn`); this handler is
/// the safety net for anything that still stomps the mode (direct `git`
/// spawns, credential prompts).
///
/// The handler swallows both events and re-injects a synthetic Ctrl+C key
/// press into the unified event channel, so the gesture keeps its exact
/// keymap semantics on whichever path Windows chose. No dedup is needed:
/// with processed input off no control event is generated, and with it on
/// the keystroke never enters the input buffer — exactly one path fires.
/// Close / logoff / shutdown fall through to the default handler so closing
/// the window still exits.
///
/// Registration failure is logged and ignored — running without the net is
/// the status quo, not a startup-abort condition.
#[cfg(windows)]
pub fn install_console_ctrl_forwarder(tx: tokio::sync::mpsc::UnboundedSender<crate::event::Event>) {
    use windows_sys::Win32::Foundation::TRUE;
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;

    let _ = console_ctrl::TX.set(tx);
    let ok = unsafe { SetConsoleCtrlHandler(Some(console_ctrl::forward_ctrl_events), TRUE) };
    if ok == 0 {
        tracing::warn!(
            "SetConsoleCtrlHandler failed ({}) — Ctrl+C on a cooked console will kill the TUI",
            std::io::Error::last_os_error()
        );
    }
}

#[cfg(not(windows))]
pub fn install_console_ctrl_forwarder(
    _tx: tokio::sync::mpsc::UnboundedSender<crate::event::Event>,
) {
}

#[cfg(windows)]
mod console_ctrl {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{BOOL, FALSE, TRUE};
    use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT};

    pub(super) static TX: OnceLock<tokio::sync::mpsc::UnboundedSender<crate::event::Event>> =
        OnceLock::new();

    /// Runs on a console-spawned thread — must stay signal-handler-simple:
    /// one lock-free channel send, no I/O, no allocation-heavy work.
    pub(super) unsafe extern "system" fn forward_ctrl_events(ctrl_type: u32) -> BOOL {
        if ctrl_type == CTRL_C_EVENT || ctrl_type == CTRL_BREAK_EVENT {
            if let Some(tx) = TX.get() {
                let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
                let _ = tx.send(crate::event::Event::Key(key));
            }
            TRUE
        } else {
            FALSE
        }
    }
}

/// Re-assert the WinAPI console *input* modes the TUI depends on: raw mode
/// (Ctrl+C as key event, no line buffering/echo) and mouse reporting. Same
/// insurance pattern as the VT keep-alive above, one layer down — the input
/// mode is shared with every process attached to the console, and a child
/// that flips it (cooked `ReadConsole`, git terminal prompts) reverts it for
/// gaviero too, with no signal. Agent trees are already console-detached at
/// spawn; this heals stomps from the remaining attached helpers so Ctrl+C
/// returns to the key-event path within one reassert interval (the ctrl
/// handler covers the gap).
///
/// Both crossterm 0.29 calls are verified idempotent on Windows:
/// `enable_raw_mode` is a stateless read-modify-write, and
/// `EnableMouseCapture`'s WinAPI path snapshots the pre-TUI mode only once
/// (CAS-guarded), so repeated calls cannot corrupt the exit restore. No-op
/// off Windows, where the pty is not shared this way.
#[cfg(windows)]
pub fn reassert_console_input_modes(w: &mut impl Write) -> std::io::Result<()> {
    crossterm::terminal::enable_raw_mode()?;
    // WinAPI-only under the hood (`is_ansi_code_supported` hardwired false
    // for input modes): writes nothing to `w`, so no mux interference.
    crossterm::execute!(w, crossterm::event::EnableMouseCapture)
}

#[cfg(not(windows))]
pub fn reassert_console_input_modes(_w: &mut impl Write) -> std::io::Result<()> {
    Ok(())
}

/// Enable/disable host-terminal bracketed paste.
///
/// **Unix:** crossterm's `EnableBracketedPaste` both writes `?2004h` and makes
/// the event source surface real `Event::Paste` values.
///
/// **Windows:** crossterm's console event source can never emit `Event::Paste`
/// (Key/Mouse/Resize/Focus only), and its `EnableBracketedPaste` WinAPI
/// fallback returns `Unsupported` on legacy consoles — so we write the VT
/// sequences ourselves (same pattern as [`enable_vt_mouse_passthrough`]).
/// Windows Terminal then emits `ESC[200~…ESC[201~` on Ctrl+V; for an
/// image-only clipboard the payload is empty, and
/// `event::read_crossterm_batch` turns that into `Event::Paste("")` so the
/// chat path can attach the clipboard image (parity with Linux).
#[cfg(windows)]
const ENABLE_BRACKETED_PASTE: &str = "\x1b[?2004h";
#[cfg(windows)]
const DISABLE_BRACKETED_PASTE: &str = "\x1b[?2004l";

pub fn set_bracketed_paste(w: &mut impl Write, enable: bool) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        let seq = if enable {
            ENABLE_BRACKETED_PASTE
        } else {
            DISABLE_BRACKETED_PASTE
        };
        w.write_all(seq.as_bytes())?;
        return w.flush();
    }
    #[cfg(not(windows))]
    {
        use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
        if enable {
            crossterm::execute!(w, EnableBracketedPaste)
        } else {
            crossterm::execute!(w, DisableBracketedPaste)
        }
    }
}

/// Rising edge of physical Ctrl+V (Win32 `GetAsyncKeyState`), independent of
/// whether Windows Terminal consumed the chord for its own paste.
///
/// WT intercepts Ctrl+V before ConPTY: text clipboards become a key burst (or
/// bracketed paste), but an image-only clipboard often produces **no** input
/// events at all — so the keymap never sees `Action::Paste`. Polling the
/// hardware key state lets the chat tick attach the clipboard image after a
/// short grace period when nothing else arrived. Always `false` off Windows.
#[cfg(windows)]
pub fn take_ctrl_v_press_edge() -> bool {
    use std::sync::atomic::{AtomicBool, Ordering};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

    const VK_CONTROL: i32 = 0x11;
    const VK_V: i32 = 0x56;

    static WAS_DOWN: AtomicBool = AtomicBool::new(false);

    // High bit set ⇒ key currently down.
    let ctrl = (unsafe { GetAsyncKeyState(VK_CONTROL) } as u16) & 0x8000 != 0;
    let v = (unsafe { GetAsyncKeyState(VK_V) } as u16) & 0x8000 != 0;
    let down = ctrl && v;
    let was = WAS_DOWN.swap(down, Ordering::SeqCst);
    down && !was
}

#[cfg(not(windows))]
pub fn take_ctrl_v_press_edge() -> bool {
    false
}

/// How long to wait after a physical Ctrl+V for the terminal to deliver a
/// paste/key burst before falling back to a direct clipboard-image attach.
pub const CTRL_V_IMAGE_FALLBACK_MS: u64 = 50;

/// The character an AltGr chord resolves to, if this key event is one.
///
/// AltGr on Windows reports as CONTROL|ALT with the layout-resolved character
/// (Spanish AltGr+2 = '@'): crossterm's console parser sets both modifiers
/// from RIGHT_ALT+LEFT_CTRL and emits `KeyCode::Char` with them untouched.
/// Consumers must treat such events as text input, not Ctrl/Alt shortcuts —
/// and must check *after* their specific Ctrl/Alt bindings, because AltGr
/// chords with no layout mapping resolve to the base character (AltGr+q →
/// Char('q')+CONTROL|ALT) and are indistinguishable from shortcut chords.
///
/// Always `None` off Windows, where AltGr arrives as plain resolved text.
pub fn altgr_char(key: &KeyEvent) -> Option<char> {
    if !cfg!(windows) {
        return None;
    }
    let ctrl_alt = KeyModifiers::CONTROL | KeyModifiers::ALT;
    match key.code {
        KeyCode::Char(c) if key.modifiers.contains(ctrl_alt) => Some(c),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The re-assert path (`App::maybe_reassert_vt_mouse`) depends on this
    /// write being the full enable set and nothing else — a disable byte
    /// here would turn the keep-alive into a mouse kill switch.
    #[cfg(windows)]
    #[test]
    fn vt_mouse_passthrough_writes_only_enable_sequences() {
        let mut out: Vec<u8> = Vec::new();
        enable_vt_mouse_passthrough(&mut out).unwrap();
        assert_eq!(out, b"\x1b[?1000h\x1b[?1002h\x1b[?1006h".to_vec());
        assert!(!out.contains(&b'l'), "no DECRST (mode-off) bytes");
    }

    /// Windows image paste depends on us advertising `?2004h` so Windows
    /// Terminal emits empty `ESC[200~ESC[201~` for an image-only clipboard.
    #[cfg(windows)]
    #[test]
    fn bracketed_paste_writes_vt_enable_and_disable() {
        let mut on: Vec<u8> = Vec::new();
        set_bracketed_paste(&mut on, true).unwrap();
        assert_eq!(on, b"\x1b[?2004h".to_vec());
        let mut off: Vec<u8> = Vec::new();
        set_bracketed_paste(&mut off, false).unwrap();
        assert_eq!(off, b"\x1b[?2004l".to_vec());
    }

    /// The handler must swallow Ctrl+C/Ctrl+Break (return TRUE — anything
    /// else lets the OS default handler terminate the process) and re-inject
    /// them as a synthetic Ctrl+C key event; close/logoff/shutdown must fall
    /// through (FALSE) so closing the window still exits.
    #[cfg(windows)]
    #[test]
    fn ctrl_forwarder_swallows_ctrl_c_and_reinjects_key_event() {
        use windows_sys::Win32::System::Console::{CTRL_C_EVENT, CTRL_CLOSE_EVENT};

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let _ = super::console_ctrl::TX.set(tx);

        let handled = unsafe { super::console_ctrl::forward_ctrl_events(CTRL_C_EVENT) };
        assert_eq!(handled, 1, "Ctrl+C must be swallowed, not left fatal");
        match rx.try_recv() {
            Ok(crate::event::Event::Key(k)) => {
                assert_eq!(k.code, KeyCode::Char('c'));
                assert!(k.modifiers.contains(KeyModifiers::CONTROL));
            }
            other => panic!("expected synthetic Ctrl+C key event, got {other:?}"),
        }

        let close = unsafe { super::console_ctrl::forward_ctrl_events(CTRL_CLOSE_EVENT) };
        assert_eq!(close, 0, "window close must reach the default handler");
        assert!(rx.try_recv().is_err(), "close must not synthesize a key");
    }

    #[cfg(windows)]
    #[test]
    fn altgr_char_resolves_ctrl_alt_chords() {
        let key = KeyEvent::new(
            KeyCode::Char('@'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        );
        assert_eq!(altgr_char(&key), Some('@'));
        // Plain Ctrl or plain Alt chords are shortcuts, not AltGr.
        let ctrl = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(altgr_char(&ctrl), None);
        let alt = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT);
        assert_eq!(altgr_char(&alt), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn altgr_char_is_none_off_windows() {
        let key = KeyEvent::new(
            KeyCode::Char('@'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        );
        assert_eq!(altgr_char(&key), None);
    }
}
