//! The session host's one key decision.
//!
//! Both input paths become a [`HostKey`] first — Windows console records
//! ([`HostKey::from_record`]) and crossterm events on Unix and in
//! `gaviero --pick` ([`HostKey::from_event`]) — and [`route`] alone decides
//! what a key does: answer the close guard, drive the picker, run a host
//! chord, reconnect a dropped SSH tab, paste the local clipboard into an SSH
//! tab, or go on to the active session. Only turning a forwarded key into
//! bytes stays per OS (`Host::on_key` / `Host::on_term_key`), because the
//! child reads key records on Windows and VT everywhere else.
//!
//! The precedence, for a key-down:
//! 1. close guard open → it takes every key but a bare modifier (`y`/`Y`
//!    without Ctrl/Alt is yes, anything else no);
//! 2. picker open → it takes the keys it understands, drops the rest;
//! 3. host chord ([`chord`]) → the host runs it, the session never sees it;
//! 4. active tab disconnected → plain Enter reconnects, the rest is dropped;
//! 5. Ctrl+V in an SSH tab → the local clipboard is pasted;
//! 6. everything else → the active session.
//!
//! A key-up whose key-down the host took is swallowed; any other key-up is
//! dropped while a dialog is open or the tab is disconnected, and forwarded
//! otherwise (Windows children read key-ups; Unix never reports them).

use super::Chord;
use super::encode::{self, KeyRecord};
use super::picker::PickerKey;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// One key as the host decides on it, whichever input path it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HostKey {
    /// The key. A letter or digit pressed with Ctrl or Alt (not both) is the
    /// lowercase `Char` of the key itself, whatever the layout types, with
    /// `SHIFT` in `mods` when it was uppercase — so chords match one way on
    /// both input paths.
    pub code: KeyCode,
    /// `CONTROL`, `ALT` and `SHIFT` only. AltGr keeps both Ctrl and Alt here
    /// and is told apart by `text`.
    pub mods: KeyModifiers,
    /// The character this key types, when it types one: printable, and with
    /// no Ctrl/Alt — or with both (AltGr) when it is not an ASCII letter or
    /// digit, the same rule as `encode::KeyEncoder`.
    pub text: Option<char>,
    pub down: bool,
    /// A bare modifier key (Shift, Ctrl, Alt, Win, Caps Lock).
    pub modifier_only: bool,
    /// The Windows virtual-key code, for key-up bookkeeping; 0 off Windows.
    pub vk: u16,
}

impl HostKey {
    /// A Windows console key record.
    pub(crate) fn from_record(r: &KeyRecord) -> HostKey {
        let ctrl = r.ctrl_state & encode::ANY_CTRL != 0;
        let alt = r.ctrl_state & encode::ANY_ALT != 0;
        let mut mods = KeyModifiers::NONE;
        mods.set(KeyModifiers::CONTROL, ctrl);
        mods.set(KeyModifiers::ALT, alt);
        mods.set(KeyModifiers::SHIFT, r.ctrl_state & encode::SHIFT != 0);
        let printable = printable(r.uchar);
        let code = match r.vk {
            0x08 => KeyCode::Backspace,
            0x09 => KeyCode::Tab,
            0x0D => KeyCode::Enter,
            0x1B => KeyCode::Esc,
            0x21 => KeyCode::PageUp,
            0x22 => KeyCode::PageDown,
            0x23 => KeyCode::End,
            0x24 => KeyCode::Home,
            0x25 => KeyCode::Left,
            0x26 => KeyCode::Up,
            0x27 => KeyCode::Right,
            0x28 => KeyCode::Down,
            0x2D => KeyCode::Insert,
            0x2E => KeyCode::Delete,
            0x70..=0x87 => KeyCode::F((r.vk - 0x6F) as u8),
            0x30..=0x39 | 0x41..=0x5A if ctrl != alt => {
                KeyCode::Char((r.vk as u8 as char).to_ascii_lowercase())
            }
            _ => printable.map_or(KeyCode::Null, KeyCode::Char),
        };
        let text = printable.filter(|c| {
            (!ctrl && !alt) || (ctrl && alt && !c.is_ascii_alphanumeric())
        });
        HostKey {
            code,
            mods,
            text,
            down: r.key_down,
            modifier_only: encode::is_modifier_vk(r.vk),
            vk: r.vk,
        }
    }

    /// A crossterm key event (Unix input path, `gaviero --pick`).
    pub(crate) fn from_event(k: &KeyEvent) -> HostKey {
        let mut mods =
            k.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        let ctrl = mods.contains(KeyModifiers::CONTROL);
        let alt = mods.contains(KeyModifiers::ALT);
        let code = match k.code {
            KeyCode::BackTab => {
                mods |= KeyModifiers::SHIFT;
                KeyCode::Tab
            }
            KeyCode::Char(c) if ctrl || alt => {
                if c.is_uppercase() {
                    mods |= KeyModifiers::SHIFT;
                }
                KeyCode::Char(c.to_ascii_lowercase())
            }
            other => other,
        };
        let text = match k.code {
            KeyCode::Char(c) if !ctrl && !alt && !c.is_control() => Some(c),
            _ => None,
        };
        HostKey {
            code,
            mods,
            text,
            down: k.kind != KeyEventKind::Release,
            modifier_only: matches!(k.code, KeyCode::Modifier(_)),
            vk: 0,
        }
    }

    fn ctrl(&self) -> bool {
        self.mods.contains(KeyModifiers::CONTROL)
    }

    fn alt(&self) -> bool {
        self.mods.contains(KeyModifiers::ALT)
    }

    fn shift(&self) -> bool {
        self.mods.contains(KeyModifiers::SHIFT)
    }
}

/// A UTF-16 unit that is a printable character on its own.
fn printable(u: u16) -> Option<char> {
    if u < 0x20 || u == 0x7F {
        return None;
    }
    char::from_u32(u32::from(u)).filter(|c| !c.is_control())
}

/// What the host state looks like to the key decision.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HostView {
    pub confirm_open: bool,
    pub picker_open: bool,
    pub active_disconnected: bool,
    pub active_ssh: bool,
    /// This key-up belongs to a key-down the host took.
    pub keyup_swallowed: bool,
}

impl HostView {
    fn modal_open(&self) -> bool {
        self.confirm_open || self.picker_open
    }
}

/// What one key does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostRoute {
    /// Nothing.
    Drop,
    /// The key-up of a key-down the host took.
    SwallowKeyUp,
    /// The close guard's answer: yes or no.
    Confirm(bool),
    Picker(PickerKey),
    Chord(Chord),
    Reconnect,
    PasteLocalClipboard,
    /// On to the active session.
    Forward,
}

impl HostRoute {
    /// The host acted on the key-down, so its key-up must not reach the
    /// session either.
    pub(crate) fn takes_key(&self) -> bool {
        !matches!(
            self,
            HostRoute::Drop | HostRoute::SwallowKeyUp | HostRoute::Forward
        )
    }
}

/// The one key decision of the session host (precedence in the module docs).
pub(crate) fn route(view: &HostView, key: &HostKey) -> HostRoute {
    if !key.down {
        return if view.keyup_swallowed {
            HostRoute::SwallowKeyUp
        } else if view.modal_open() || view.active_disconnected {
            HostRoute::Drop
        } else {
            HostRoute::Forward
        };
    }
    if view.confirm_open {
        if key.modifier_only {
            return HostRoute::Drop;
        }
        let yes = matches!(key.text, Some('y' | 'Y'));
        return HostRoute::Confirm(yes);
    }
    if view.picker_open {
        return picker_key(key).map_or(HostRoute::Drop, HostRoute::Picker);
    }
    if let Some(chord) = chord(key) {
        return HostRoute::Chord(chord);
    }
    if view.active_disconnected {
        let plain = !key.ctrl() && !key.alt() && !key.shift();
        return if key.code == KeyCode::Enter && plain {
            HostRoute::Reconnect
        } else {
            HostRoute::Drop
        };
    }
    if view.active_ssh
        && key.code == KeyCode::Char('v')
        && key.ctrl()
        && !key.alt()
        && !key.shift()
    {
        return HostRoute::PasteLocalClipboard;
    }
    HostRoute::Forward
}

/// Whether a paste (Unix: a bracketed-paste event) goes to the active
/// session. Never past a dialog or into a disconnected tab.
pub(crate) fn paste_goes_to_session(view: &HostView) -> bool {
    !view.modal_open() && !view.active_disconnected
}

/// The host chord a key-down is, if any. Ctrl+Q is Ctrl without Alt and the
/// rest Alt without Ctrl, so AltGr chords stay text. Alt+X is never a chord:
/// Windows keeps it for itself.
pub(crate) fn chord(key: &HostKey) -> Option<Chord> {
    if !key.down {
        return None;
    }
    let (ctrl, alt, shift) = (key.ctrl(), key.alt(), key.shift());
    if ctrl && !alt {
        return (key.code == KeyCode::Char('q')).then_some(Chord::QuitAll);
    }
    if !alt || ctrl {
        return None;
    }
    match key.code {
        KeyCode::Left if shift => Some(Chord::MoveLeft),
        KeyCode::Right if shift => Some(Chord::MoveRight),
        KeyCode::Left => Some(Chord::Prev),
        KeyCode::Right => Some(Chord::Next),
        KeyCode::Char('n') if !shift => Some(Chord::New),
        KeyCode::Char('q') => Some(Chord::Close),
        _ => None,
    }
}

/// The picker key a key-down is, if the picker understands it.
pub(crate) fn picker_key(key: &HostKey) -> Option<PickerKey> {
    if !key.down {
        return None;
    }
    Some(match key.code {
        KeyCode::Enter => PickerKey::Enter,
        KeyCode::Esc => PickerKey::Esc,
        KeyCode::Backspace => PickerKey::Backspace,
        KeyCode::Tab => PickerKey::Tab,
        KeyCode::Up => PickerKey::Up,
        KeyCode::Down => PickerKey::Down,
        KeyCode::Left => PickerKey::Left,
        KeyCode::Right => PickerKey::Right,
        KeyCode::PageUp => PickerKey::PageUp,
        KeyCode::PageDown => PickerKey::PageDown,
        KeyCode::Char('t') if key.ctrl() && !key.alt() => PickerKey::ToggleMode,
        _ => PickerKey::Char(key.text?),
    })
}

/// The bytes a forwarded key goes as when the child reads VT (every Unix
/// tab, and SSH tabs on Windows through `encode::ssh_key_record`), where
/// they differ from the plain encoding: Shift+Enter has no VT form of its own
/// and would arrive as plain Enter, so it goes as Alt+Enter (`ESC \r`),
/// which gaviero binds to the same newline; Alt+Enter itself keeps its ESC.
pub(crate) fn vt_override(k: &KeyEvent) -> Option<&'static [u8]> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let shift_or_alt = k.modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::ALT);
    (k.code == KeyCode::Enter && shift_or_alt && !ctrl).then_some(b"\x1b\r".as_slice())
}

#[cfg(test)]
mod tests {
    use super::*;
    use encode::{LEFT_ALT, LEFT_CTRL, RIGHT_ALT, RIGHT_CTRL, SHIFT};

    fn rec(vk: u16, uchar: u16, ctrl_state: u32) -> HostKey {
        HostKey::from_record(&KeyRecord {
            vk,
            scan: 0,
            uchar,
            key_down: true,
            ctrl_state,
            repeat: 1,
        })
    }

    fn up(mut k: HostKey) -> HostKey {
        k.down = false;
        k
    }

    fn ev(code: KeyCode, mods: KeyModifiers) -> HostKey {
        HostKey::from_event(&KeyEvent::new(code, mods))
    }

    const IDLE: HostView = HostView {
        confirm_open: false,
        picker_open: false,
        active_disconnected: false,
        active_ssh: false,
        keyup_swallowed: false,
    };

    /// The same physical key, from both input paths, routes the same way.
    fn same_route(view: HostView, win: HostKey, unix: HostKey) -> HostRoute {
        let a = route(&view, &win);
        assert_eq!(a, route(&view, &unix), "{win:?} vs {unix:?} in {view:?}");
        a
    }

    #[test]
    fn host_chords_are_alt_keys_and_ctrl_q() {
        assert_eq!(chord(&rec(0x25, 0, LEFT_ALT)), Some(Chord::Prev));
        assert_eq!(chord(&rec(0x27, 0, RIGHT_ALT)), Some(Chord::Next));
        assert_eq!(chord(&rec(0x4E, 'n' as u16, LEFT_ALT)), Some(Chord::New));
        assert_eq!(chord(&rec(0x51, 'q' as u16, LEFT_ALT)), Some(Chord::Close));
        assert_eq!(chord(&rec(0x51, 'Q' as u16, LEFT_ALT | SHIFT)), Some(Chord::Close));
        assert_eq!(chord(&rec(0x51, 17, LEFT_CTRL)), Some(Chord::QuitAll));
        assert_eq!(chord(&rec(0x51, 17, RIGHT_CTRL)), Some(Chord::QuitAll));
        assert_eq!(chord(&rec(0x25, 0, LEFT_ALT | SHIFT)), Some(Chord::MoveLeft));
        assert_eq!(chord(&rec(0x27, 0, RIGHT_ALT | SHIFT)), Some(Chord::MoveRight));
        // Alt+X belongs to Windows; other Ctrl keys belong to the session.
        assert_eq!(chord(&rec(0x58, 'x' as u16, LEFT_ALT)), None);
        assert_eq!(chord(&rec(0x53, 19, LEFT_CTRL)), None);
        // Alt+Shift+N is not New; Shift or Ctrl+Alt arrows are the session's.
        assert_eq!(chord(&rec(0x4E, 'N' as u16, LEFT_ALT | SHIFT)), None);
        assert_eq!(chord(&rec(0x25, 0, SHIFT)), None);
        assert_eq!(chord(&rec(0x25, 0, LEFT_ALT | LEFT_CTRL | SHIFT)), None);
        // Key-ups never trigger a chord.
        assert_eq!(chord(&up(rec(0x25, 0, LEFT_ALT))), None);
        assert_eq!(chord(&up(rec(0x51, 17, LEFT_CTRL))), None);
    }

    #[test]
    fn altgr_and_plain_keys_are_not_chords() {
        assert_eq!(chord(&rec(0x4E, 'n' as u16, RIGHT_ALT | LEFT_CTRL)), None);
        // AltGr+Q is '@' on German layouts: text, not quit.
        let at = rec(0x51, '@' as u16, RIGHT_ALT | LEFT_CTRL);
        assert_eq!(chord(&at), None);
        assert_eq!(at.text, Some('@'));
        assert_eq!(route(&IDLE, &at), HostRoute::Forward);
        assert_eq!(chord(&rec(0x4E, 'n' as u16, 0)), None);
        assert_eq!(chord(&rec(0x51, 'q' as u16, 0)), None);
        assert_eq!(chord(&rec(0x25, 0, 0)), None);
    }

    #[test]
    fn both_input_paths_agree_on_every_decision() {
        let (alt, ctrl, shift) = (KeyModifiers::ALT, KeyModifiers::CONTROL, KeyModifiers::SHIFT);
        // Chords.
        let cases = [
            (rec(0x25, 0, LEFT_ALT), ev(KeyCode::Left, alt)),
            (rec(0x27, 0, LEFT_ALT | SHIFT), ev(KeyCode::Right, alt | shift)),
            (rec(0x4E, 'n' as u16, LEFT_ALT), ev(KeyCode::Char('n'), alt)),
            (rec(0x4E, 'N' as u16, LEFT_ALT | SHIFT), ev(KeyCode::Char('N'), alt)),
            (rec(0x51, 'Q' as u16, LEFT_ALT | SHIFT), ev(KeyCode::Char('Q'), alt)),
            (rec(0x51, 17, LEFT_CTRL), ev(KeyCode::Char('q'), ctrl)),
            (rec(0x58, 'x' as u16, LEFT_ALT), ev(KeyCode::Char('x'), alt)),
            (rec(0x41, 'a' as u16, 0), ev(KeyCode::Char('a'), KeyModifiers::NONE)),
        ];
        for (win, unix) in cases {
            same_route(IDLE, win, unix);
        }

        // The close guard: y or Y is yes, Ctrl/Alt+y is no.
        let guard = HostView { confirm_open: true, ..IDLE };
        let yes = same_route(guard, rec(0x59, 'y' as u16, 0), ev(KeyCode::Char('y'), KeyModifiers::NONE));
        assert_eq!(yes, HostRoute::Confirm(true));
        let shift_y = same_route(guard, rec(0x59, 'Y' as u16, SHIFT), ev(KeyCode::Char('Y'), shift));
        assert_eq!(shift_y, HostRoute::Confirm(true));
        let ctrl_y = same_route(guard, rec(0x59, 25, LEFT_CTRL), ev(KeyCode::Char('y'), ctrl));
        assert_eq!(ctrl_y, HostRoute::Confirm(false));
        let esc = same_route(guard, rec(0x1B, 0x1B, 0), ev(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(esc, HostRoute::Confirm(false));
        // A bare modifier neither answers nor is swallowed.
        assert_eq!(route(&guard, &rec(0x10, 0, SHIFT)), HostRoute::Drop);

        // The picker: Ctrl+T (with or without Shift) switches mode; Alt+letter
        // types nothing; AltGr symbols are text.
        let picker = HostView { picker_open: true, ..IDLE };
        let toggle = HostRoute::Picker(PickerKey::ToggleMode);
        assert_eq!(same_route(picker, rec(0x54, 0x14, LEFT_CTRL), ev(KeyCode::Char('t'), ctrl)), toggle);
        assert_eq!(
            same_route(picker, rec(0x54, 0x14, LEFT_CTRL | SHIFT), ev(KeyCode::Char('T'), ctrl | shift)),
            toggle
        );
        assert_eq!(
            same_route(picker, rec(0x41, 'a' as u16, LEFT_ALT), ev(KeyCode::Char('a'), alt)),
            HostRoute::Drop
        );
        assert_eq!(
            route(&picker, &rec(0x32, '@' as u16, RIGHT_ALT | LEFT_CTRL)),
            HostRoute::Picker(PickerKey::Char('@'))
        );
        assert_eq!(
            same_route(picker, rec(0x09, '\t' as u16, SHIFT), ev(KeyCode::BackTab, shift)),
            HostRoute::Picker(PickerKey::Tab)
        );

        // A disconnected tab: plain Enter reconnects, Shift+Enter does not.
        let gone = HostView { active_disconnected: true, ..IDLE };
        let enter = same_route(gone, rec(0x0D, 13, 0), ev(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(enter, HostRoute::Reconnect);
        let shift_enter = same_route(gone, rec(0x0D, 13, SHIFT), ev(KeyCode::Enter, shift));
        assert_eq!(shift_enter, HostRoute::Drop);

        // Ctrl+V pastes the local clipboard into an SSH tab only.
        let ssh = HostView { active_ssh: true, ..IDLE };
        let ctrl_v = same_route(ssh, rec(0x56, 0x16, LEFT_CTRL), ev(KeyCode::Char('v'), ctrl));
        assert_eq!(ctrl_v, HostRoute::PasteLocalClipboard);
        let ctrl_shift_v = same_route(ssh, rec(0x56, 0x16, LEFT_CTRL | SHIFT), ev(KeyCode::Char('V'), ctrl | shift));
        assert_eq!(ctrl_shift_v, HostRoute::Forward);
        assert_eq!(route(&IDLE, &rec(0x56, 0x16, LEFT_CTRL)), HostRoute::Forward);
    }

    #[test]
    fn precedence_is_guard_picker_chord_disconnected_paste_forward() {
        let alt_q = rec(0x51, 'q' as u16, LEFT_ALT);
        // The guard wins over a chord, the picker over a chord.
        let guard = HostView { confirm_open: true, picker_open: true, ..IDLE };
        assert_eq!(route(&guard, &alt_q), HostRoute::Confirm(false));
        let picker = HostView { picker_open: true, ..IDLE };
        assert_eq!(route(&picker, &alt_q), HostRoute::Drop);
        // A chord works in a disconnected tab.
        let gone = HostView { active_disconnected: true, active_ssh: true, ..IDLE };
        assert_eq!(route(&gone, &alt_q), HostRoute::Chord(Chord::Close));
        // A disconnected SSH tab takes no clipboard paste.
        assert_eq!(route(&gone, &rec(0x56, 0x16, LEFT_CTRL)), HostRoute::Drop);
    }

    #[test]
    fn key_ups_follow_their_key_down() {
        let enter = up(rec(0x0D, 13, 0));
        let swallowed = HostView { keyup_swallowed: true, ..IDLE };
        assert_eq!(route(&swallowed, &enter), HostRoute::SwallowKeyUp);
        assert_eq!(route(&HostView { picker_open: true, ..IDLE }, &enter), HostRoute::Drop);
        assert_eq!(route(&HostView { active_disconnected: true, ..IDLE }, &enter), HostRoute::Drop);
        assert_eq!(route(&IDLE, &enter), HostRoute::Forward);
        // What the host acts on swallows its key-up; forwarding does not.
        assert!(HostRoute::Chord(Chord::New).takes_key());
        assert!(HostRoute::Confirm(false).takes_key());
        assert!(!HostRoute::Forward.takes_key());
        assert!(!HostRoute::Drop.takes_key());
    }

    #[test]
    fn pastes_never_pass_a_dialog() {
        assert!(paste_goes_to_session(&IDLE));
        assert!(!paste_goes_to_session(&HostView { picker_open: true, ..IDLE }));
        assert!(!paste_goes_to_session(&HostView { confirm_open: true, ..IDLE }));
        assert!(!paste_goes_to_session(&HostView { active_disconnected: true, ..IDLE }));
    }

    #[test]
    fn shift_and_alt_enter_keep_their_escape_over_vt() {
        let key = |mods| KeyEvent::new(KeyCode::Enter, mods);
        assert_eq!(vt_override(&key(KeyModifiers::SHIFT)), Some(b"\x1b\r".as_slice()));
        assert_eq!(vt_override(&key(KeyModifiers::ALT)), Some(b"\x1b\r".as_slice()));
        assert_eq!(vt_override(&key(KeyModifiers::NONE)), None);
        assert_eq!(vt_override(&key(KeyModifiers::CONTROL | KeyModifiers::SHIFT)), None);
        assert_eq!(vt_override(&KeyEvent::new(KeyCode::Char('a'), KeyModifiers::SHIFT)), None);
    }
}
