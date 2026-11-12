//! Pure byte encoders for the session host: Windows key records as
//! win32-input-mode sequences, mouse records as SGR reports, and answers to
//! the terminal queries a child sends.
//!
//! **std only.** The integration tests (`tests/session_host_*.rs`) include
//! this file with `#[path]`, so it must not import `crate::` items or any
//! external crate.

/// One Windows `KEY_EVENT_RECORD`, field for field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyRecord {
    pub vk: u16,
    pub scan: u16,
    pub uchar: u16,
    pub key_down: bool,
    pub ctrl_state: u32,
    pub repeat: u16,
}

/// The win32-input-mode sequence for one key record
/// (`ESC [ Vk ; Sc ; Uc ; Kd ; Cs ; Rc _`). ConPTY always requests this mode
/// (`ESC[?9001h`) and turns the sequence back into the same `KEY_EVENT`
/// record for the child, so AltGr, dead keys and key-up events survive.
pub fn win32_key_seq(r: &KeyRecord) -> String {
    format!(
        "\x1b[{};{};{};{};{};{}_",
        r.vk,
        r.scan,
        r.uchar,
        r.key_down as u8,
        r.ctrl_state,
        r.repeat.max(1)
    )
}

/// Turns the console's key records into the bytes a ConPTY child is sent.
///
/// Text keys go as plain UTF-8, everything else as [`win32_key_seq`]. ConPTY
/// reads its input pipe in chunks, and the inbox conhost drops the start of
/// a sequence that a chunk boundary splits and hands the rest to the child
/// as typed text (microsoft/terminal#17738). A paste is a burst of records
/// that fills the pipe, so the boundaries land inside sequences and the
/// pasted text comes out as `;0;0;1_` garbage. Text has no sequence to
/// split: conhost turns each character back into a key event, and it joins
/// UTF-8 split across reads. For the same reason the key-ups of text keys
/// and bare modifier records (a paste wraps every capital in Shift down/up)
/// are not sent: no child reads them.
#[derive(Debug, Default)]
pub struct KeyEncoder {
    /// The first half of a surrogate pair, until the second arrives.
    high_surrogate: Option<u16>,
}

impl KeyEncoder {
    pub fn encode(&mut self, r: &KeyRecord) -> Vec<u8> {
        if r.uchar == 0 && is_modifier_vk(r.vk) {
            return Vec::new();
        }
        let Some(unit) = text_unit(r) else {
            self.high_surrogate = None;
            return win32_key_seq(r).into_bytes();
        };
        if !r.key_down {
            return Vec::new();
        }
        let c = match unit {
            0xD800..=0xDBFF => {
                self.high_surrogate = Some(unit);
                return Vec::new();
            }
            0xDC00..=0xDFFF => {
                let Some(high) = self.high_surrogate.take() else {
                    return Vec::new();
                };
                char::decode_utf16([high, unit]).next().and_then(Result::ok)
            }
            _ => {
                self.high_surrogate = None;
                char::from_u32(unit as u32)
            }
        };
        let Some(c) = c else {
            return Vec::new();
        };
        let mut utf8 = [0u8; 4];
        c.encode_utf8(&mut utf8).repeat(r.repeat.max(1) as usize).into_bytes()
    }
}

/// The UTF-16 unit a key record types, when it can be sent as text: a
/// printable character with no Ctrl/Alt, or with AltGr (`Ctrl + Alt`) when it
/// is not a letter or digit — `Ctrl+Alt+letter` stays a chord. Enter and Tab
/// only bare, so Shift+Enter and Shift+Tab keep their modifier.
fn text_unit(r: &KeyRecord) -> Option<u16> {
    let u = r.uchar;
    let alt = r.ctrl_state & ANY_ALT != 0;
    let ctrl = r.ctrl_state & ANY_CTRL != 0;
    if u >= 0x20 && u != 0x7F {
        let alnum = u < 0x80 && (u as u8).is_ascii_alphanumeric();
        return ((!alt && !ctrl) || (alt && ctrl && !alnum)).then_some(u);
    }
    let bare = r.ctrl_state & (ANY_ALT | ANY_CTRL | SHIFT) == 0;
    (bare && (u == b'\r' as u16 || u == b'\t' as u16)).then_some(u)
}

/// Virtual-key codes of modifier keys (Shift, Ctrl, Alt, Caps Lock, Win).
pub fn is_modifier_vk(vk: u16) -> bool {
    matches!(vk, 0x10..=0x12 | 0x14 | 0x5B | 0x5C | 0xA0..=0xA5)
}

/// `dwControlKeyState` bits (mirrors of the Windows constants).
pub const RIGHT_ALT: u32 = 1;
pub const LEFT_ALT: u32 = 2;
pub const RIGHT_CTRL: u32 = 4;
pub const LEFT_CTRL: u32 = 8;
pub const SHIFT: u32 = 16;

const ANY_ALT: u32 = RIGHT_ALT | LEFT_ALT;
const ANY_CTRL: u32 = RIGHT_CTRL | LEFT_CTRL;

/// Some Alt key is held and no Ctrl key. AltGr reports as
/// `RIGHT_ALT | LEFT_CTRL`, so it never counts as Alt here.
pub fn alt_only(ctrl_state: u32) -> bool {
    ctrl_state & ANY_ALT != 0 && ctrl_state & ANY_CTRL == 0
}

/// Some Ctrl key is held and no Alt key, so AltGr never counts here either.
pub fn ctrl_only(ctrl_state: u32) -> bool {
    ctrl_state & ANY_CTRL != 0 && ctrl_state & ANY_ALT == 0
}

/// One Windows `MOUSE_EVENT_RECORD` (0-based col/row, raw fields).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseRecord {
    pub col: u16,
    pub row: u16,
    pub buttons: u32,
    pub ctrl_state: u32,
    pub flags: u32,
}

/// An SGR mouse report (`ESC[<code;x;yM` press, `...m` release), 1-based.
pub fn sgr_mouse(code: u8, col0: u16, row0: u16, press: bool) -> String {
    format!(
        "\x1b[<{};{};{}{}",
        code,
        col0 + 1,
        row0 + 1,
        if press { 'M' } else { 'm' }
    )
}

/// `dwEventFlags` bits.
const MOUSE_MOVED: u32 = 1;
const MOUSE_WHEELED: u32 = 4;

/// Whether a raw console record is the outer terminal's font-zoom gesture
/// (Ctrl+wheel, vertical wheel only). The record form of
/// `platform::is_terminal_zoom_gesture`; see `Host::begin_zoom`.
/// `ctrl_held` asks the keyboard for its own Ctrl state
/// (`platform::ctrl_key_held`), only for wheel records: Windows Terminal's
/// wheel record can arrive without Ctrl in `ctrl_state`.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn is_zoom_gesture(rec: &MouseRecord, ctrl_held: impl FnOnce() -> bool) -> bool {
    rec.flags & MOUSE_WHEELED != 0 && (rec.ctrl_state & ANY_CTRL != 0 || ctrl_held())
}

/// Button bits of `dwButtonState` and their SGR button codes, in the order
/// the reports are emitted: left (0), middle (1), right (2).
const BUTTONS: [(u32, u8); 3] = [(1, 0), (4, 1), (2, 2)];

/// Turns raw mouse records into SGR reports. Windows reports button *state*,
/// SGR reports *transitions*, so the previous state is kept.
#[derive(Default)]
pub struct MouseTranslator {
    prev_buttons: u32,
}

impl MouseTranslator {
    /// The SGR reports for one record, with rows above `row_offset` (the
    /// host's tab bar) dropped and the rest shifted up by it.
    pub fn translate(&mut self, rec: &MouseRecord, row_offset: u16) -> Vec<String> {
        if rec.row < row_offset {
            return Vec::new();
        }
        let row0 = rec.row - row_offset;
        let mut mods: u8 = 0;
        if rec.ctrl_state & SHIFT != 0 {
            mods += 4;
        }
        if rec.ctrl_state & ANY_ALT != 0 {
            mods += 8;
        }
        if rec.ctrl_state & ANY_CTRL != 0 {
            mods += 16;
        }

        if rec.flags & MOUSE_WHEELED != 0 {
            let delta = (rec.buttons >> 16) as u16 as i16;
            let code = if delta > 0 { 64 } else { 65 };
            return vec![sgr_mouse(code + mods, rec.col, row0, true)];
        }

        let now = rec.buttons & 0x7;
        let mut out = Vec::new();
        for (bit, code) in BUTTONS {
            let was = self.prev_buttons & bit != 0;
            let is = now & bit != 0;
            if is && !was {
                out.push(sgr_mouse(code + mods, rec.col, row0, true));
            } else if was && !is {
                out.push(sgr_mouse(code + mods, rec.col, row0, false));
            }
        }
        let held = BUTTONS.iter().find(|(bit, _)| now & bit != 0);
        if out.is_empty()
            && rec.flags & MOUSE_MOVED != 0
            && let Some((_, code)) = held
        {
            out.push(sgr_mouse(code + 32 + mods, rec.col, row0, true));
        }
        self.prev_buttons = now;
        out
    }
}

/// How many times `needle` occurs in `hay` (overlaps counted).
pub fn count_occurrences(hay: &[u8], needle: &[u8]) -> usize {
    if needle.is_empty() || hay.len() < needle.len() {
        return 0;
    }
    hay.windows(needle.len()).filter(|w| *w == needle).count()
}

/// Answers to the terminal queries in a child's output: DSR-CPR (`ESC[6n`)
/// gets the cursor position, DA1 (`ESC[c` / `ESC[0c`) a VT101-class id.
/// The host *is* the child's terminal, so it must answer (pwsh blocks on
/// DSR otherwise). `cursor` is the 0-based (row, col).
pub fn query_replies(output: &[u8], cursor: (u16, u16)) -> Vec<u8> {
    let mut out = Vec::new();
    let report = format!("\x1b[{};{}R", cursor.0 + 1, cursor.1 + 1);
    for _ in 0..count_occurrences(output, b"\x1b[6n") {
        out.extend_from_slice(report.as_bytes());
    }
    let da1 = count_occurrences(output, b"\x1b[c") + count_occurrences(output, b"\x1b[0c");
    for _ in 0..da1 {
        out.extend_from_slice(b"\x1b[?1;2c");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(vk: u16, uchar: u16, down: bool, ctrl: u32) -> KeyRecord {
        KeyRecord {
            vk,
            scan: 0,
            uchar,
            key_down: down,
            ctrl_state: ctrl,
            repeat: 1,
        }
    }

    fn mouse(col: u16, row: u16, buttons: u32, ctrl: u32, flags: u32) -> MouseRecord {
        MouseRecord {
            col,
            row,
            buttons,
            ctrl_state: ctrl,
            flags,
        }
    }

    #[test]
    fn key_down_and_up_records_encode_exactly() {
        let mut r = key(0x41, b'a' as u16, true, 0);
        r.scan = 30;
        assert_eq!(win32_key_seq(&r), "\x1b[65;30;97;1;0;1_");
        r.key_down = false;
        assert_eq!(win32_key_seq(&r), "\x1b[65;30;97;0;0;1_");
        // A zero repeat count is sent as 1.
        r.repeat = 0;
        assert_eq!(win32_key_seq(&r), "\x1b[65;30;97;0;0;1_");
    }

    fn encode_all(records: &[KeyRecord]) -> Vec<u8> {
        let mut enc = KeyEncoder::default();
        records.iter().flat_map(|r| enc.encode(r)).collect()
    }

    fn tap(vk: u16, uchar: u16, ctrl: u32) -> [KeyRecord; 2] {
        [key(vk, uchar, true, ctrl), key(vk, uchar, false, ctrl)]
    }

    #[test]
    fn text_keys_go_as_utf8_without_their_key_ups() {
        let mut records = Vec::new();
        records.extend(tap(0x41, 'a' as u16, 0));
        records.push(key(0xA0, 0, true, SHIFT));
        records.extend(tap(0x4C, 'L' as u16, SHIFT));
        records.push(key(0xA0, 0, false, 0));
        records.extend(tap(0, 'ó' as u16, 0));
        records.extend(tap(0x0D, '\r' as u16, 0));
        records.extend(tap(0x09, '\t' as u16, 0));
        assert_eq!(encode_all(&records), "aLó\r\t".as_bytes());
    }

    #[test]
    fn altgr_symbols_are_text_and_ctrl_alt_letters_stay_chords() {
        let at = key(0x32, '@' as u16, true, RIGHT_ALT | LEFT_CTRL);
        assert_eq!(encode_all(&[at]), b"@");
        let chord = key(0x51, 'q' as u16, true, RIGHT_ALT | LEFT_CTRL);
        assert_eq!(encode_all(&[chord]), win32_key_seq(&chord).into_bytes());
    }

    #[test]
    fn keys_that_are_not_text_keep_their_sequence() {
        for r in [
            key(0x41, 0x01, true, LEFT_CTRL),         // Ctrl+A
            key(0x41, 'a' as u16, true, LEFT_ALT),    // Alt+A
            key(0x0D, '\r' as u16, true, SHIFT),      // Shift+Enter
            key(0x09, '\t' as u16, true, SHIFT),      // Shift+Tab
            key(0x08, 0x08, true, 0),                 // Backspace
            key(0x1B, 0x1B, true, 0),                 // Esc
            key(0x25, 0, true, 0),                    // Left
            key(0x25, 0, false, 0),                   // Left up
        ] {
            assert_eq!(encode_all(&[r]), win32_key_seq(&r).into_bytes(), "{r:?}");
        }
    }

    #[test]
    fn surrogate_pairs_join_into_one_character() {
        let mut records = Vec::new();
        records.extend(tap(0, 0xD83D, 0));
        records.extend(tap(0, 0xDE00, 0));
        assert_eq!(encode_all(&records), "😀".as_bytes());
        // A lone low half is dropped.
        assert_eq!(encode_all(&tap(0, 0xDE00, 0)), b"");
    }

    #[test]
    fn a_repeat_count_repeats_the_text() {
        let mut r = key(0x41, 'a' as u16, true, 0);
        r.repeat = 3;
        assert_eq!(encode_all(&[r]), b"aaa");
    }

    #[test]
    fn ctrl_alt_left_encodes_both_modifier_bits() {
        let r = key(0x25, 0, true, LEFT_CTRL | LEFT_ALT);
        assert_eq!(win32_key_seq(&r), "\x1b[37;0;0;1;10;1_");
    }

    #[test]
    fn alt_only_excludes_altgr() {
        assert!(alt_only(LEFT_ALT));
        assert!(alt_only(RIGHT_ALT | SHIFT));
        assert!(!alt_only(RIGHT_ALT | LEFT_CTRL));
        assert!(!alt_only(LEFT_ALT | RIGHT_CTRL));
        assert!(!alt_only(0));
        assert!(!alt_only(SHIFT));
    }

    #[test]
    fn left_press_and_release() {
        let mut t = MouseTranslator::default();
        assert_eq!(t.translate(&mouse(9, 4, 1, 0, 0), 0), vec!["\x1b[<0;10;5M"]);
        assert_eq!(t.translate(&mouse(9, 4, 0, 0, 0), 0), vec!["\x1b[<0;10;5m"]);
    }

    #[test]
    fn right_and_middle_buttons() {
        let mut t = MouseTranslator::default();
        assert_eq!(t.translate(&mouse(0, 0, 2, 0, 0), 0), vec!["\x1b[<2;1;1M"]);
        assert_eq!(t.translate(&mouse(0, 0, 0, 0, 0), 0), vec!["\x1b[<2;1;1m"]);
        assert_eq!(t.translate(&mouse(0, 0, 4, 0, 0), 0), vec!["\x1b[<1;1;1M"]);
    }

    #[test]
    fn drag_reports_motion_with_a_held_button_only() {
        let mut t = MouseTranslator::default();
        t.translate(&mouse(1, 1, 1, 0, 0), 0);
        assert_eq!(
            t.translate(&mouse(5, 1, 1, 0, MOUSE_MOVED), 0),
            vec!["\x1b[<32;6;2M"]
        );
        t.translate(&mouse(5, 1, 0, 0, 0), 0);
        // Bare motion, no button held: nothing.
        assert!(t.translate(&mouse(6, 1, 0, 0, MOUSE_MOVED), 0).is_empty());
    }

    #[test]
    fn wheel_up_and_down() {
        let mut t = MouseTranslator::default();
        let up = 120u32 << 16;
        let down = ((-120i16) as u16 as u32) << 16;
        assert_eq!(
            t.translate(&mouse(3, 3, up, 0, MOUSE_WHEELED), 0),
            vec!["\x1b[<64;4;4M"]
        );
        assert_eq!(
            t.translate(&mouse(3, 3, down, 0, MOUSE_WHEELED), 0),
            vec!["\x1b[<65;4;4M"]
        );
    }

    #[test]
    fn zoom_gesture_is_ctrl_wheel_only() {
        let up = 120u32 << 16;
        let down = ((-120i16) as u16 as u32) << 16;
        assert!(is_zoom_gesture(&mouse(3, 3, up, LEFT_CTRL, MOUSE_WHEELED), || false));
        assert!(is_zoom_gesture(&mouse(3, 0, down, RIGHT_CTRL, MOUSE_WHEELED), || false));
        // Ctrl missing from the record but held on the keyboard: still zoom.
        assert!(is_zoom_gesture(&mouse(3, 3, up, 0, MOUSE_WHEELED), || true));
        // Plain wheel, Ctrl+click, Ctrl+drag: not the gesture.
        assert!(!is_zoom_gesture(&mouse(3, 3, up, 0, MOUSE_WHEELED), || false));
        assert!(!is_zoom_gesture(&mouse(3, 3, 1, LEFT_CTRL, 0), || true));
        assert!(!is_zoom_gesture(&mouse(3, 3, 1, LEFT_CTRL, MOUSE_MOVED), || true));
        // Horizontal wheel (MOUSE_HWHEELED = 8) is not zoom.
        assert!(!is_zoom_gesture(&mouse(3, 3, up, LEFT_CTRL, 8), || true));
    }

    #[test]
    fn row_offset_drops_the_bar_and_shifts_the_rest() {
        let mut t = MouseTranslator::default();
        assert!(t.translate(&mouse(2, 0, 1, 0, 0), 1).is_empty());
        let mut t = MouseTranslator::default();
        assert_eq!(t.translate(&mouse(9, 5, 1, 0, 0), 1), vec!["\x1b[<0;10;5M"]);
    }

    #[test]
    fn modifier_bits_are_added_to_the_code() {
        let mut t = MouseTranslator::default();
        assert_eq!(
            t.translate(&mouse(0, 0, 1, SHIFT | LEFT_ALT | RIGHT_CTRL, 0), 0),
            vec!["\x1b[<28;1;1M"]
        );
    }

    #[test]
    fn dsr_and_da1_get_answers() {
        assert_eq!(query_replies(b"x\x1b[6ny", (4, 9)), b"\x1b[5;10R".to_vec());
        assert_eq!(query_replies(b"\x1b[c", (0, 0)), b"\x1b[?1;2c".to_vec());
        assert_eq!(query_replies(b"\x1b[0c", (0, 0)), b"\x1b[?1;2c".to_vec());
        assert!(query_replies(b"plain", (0, 0)).is_empty());
        assert_eq!(count_occurrences(b"aaa", b"aa"), 2);
    }
}
