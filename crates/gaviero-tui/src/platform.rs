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

/// Enable/disable host-terminal bracketed paste where crossterm can actually
/// deliver `Event::Paste` — Unix only.
///
/// On Windows this is deliberately a no-op, for three verified reasons
/// (crossterm 0.29 source):
/// - the Windows event source builds only Key/Mouse/Resize/Focus events from
///   console input records and can never surface `Event::Paste`; pastes are
///   reconstructed from the key burst by `event::read_crossterm_batch`;
/// - on a VT-capable host, `EnableBracketedPaste` *would* write `?2004h`
///   through ConPTY, inviting `ESC[200~` marker bytes to arrive as ordinary
///   key events for the coalescer to mangle;
/// - on a legacy console (`supports_ansi()` = false), the command's WinAPI
///   fallback returns `Err(Unsupported)`, which would abort startup.
pub fn set_bracketed_paste(w: &mut impl Write, enable: bool) -> std::io::Result<()> {
    if cfg!(windows) {
        return Ok(());
    }
    use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
    if enable {
        crossterm::execute!(w, EnableBracketedPaste)
    } else {
        crossterm::execute!(w, DisableBracketedPaste)
    }
}

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
