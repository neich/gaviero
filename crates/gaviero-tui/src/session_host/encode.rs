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
