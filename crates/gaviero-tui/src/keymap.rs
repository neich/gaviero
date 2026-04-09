use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Quit,
    ToggleFileTree,
    ToggleSidePanel,
    ToggleTerminal,
    NewTab,
    CycleTabForward,
    CycleTabBack,
    CloseTab,

    // Panel focus (Alt+1/2/3/4)
    FocusLeftPanel,
    FocusEditor,
    FocusSidePanel,
    FocusTerminal,

    // Left panel modes (Alt+E/F/C)
    SetLeftModeExplorer,
    SetLeftModeFind,
    SetLeftModeChanges,

    // Side panel modes (Alt+A/W/G)
    SetSideModeChat,
    SetSideModeSwarm,
    SetSideModeGit,

    // Editor actions
    InsertChar(char),
    Backspace,
    Delete,
    Enter,
    Tab,
    CursorUp,
    CursorDown,
    CursorLeft,
    CursorRight,
    WordLeft,
    WordRight,
    SelectLeft,
    SelectRight,
    SelectUp,
    SelectDown,
    SelectWordLeft,
    SelectWordRight,
    PageUp,
    PageDown,
    Home,
    End,
    Undo,
    Redo,
    Save,
    TogglePreview,
    DeleteLine,
    DuplicateLine,
    MoveLineUp,
    MoveLineDown,
    GoToLineEnd,
    DeleteToLineEnd,
    DeleteWordBack,
    FormatBuffer,
    CycleFormatLevel,
    FindInBuffer,
    SearchInWorkspace,

    // Chat
    AltEnter,
    /// Toggle "auto-approve all permissions" for the next prompt only.
    ToggleAutoApprove,

    // Swarm (triggered by /swarm command, not a keybinding)
    #[allow(dead_code)]
    ToggleSwarmDashboard,

    // Layout
    ToggleFullscreen,
    SwitchLayout(u8), // 0-5 via Alt+5..9 and Alt+0
    Rename,

    // Clipboard
    Copy,
    Cut,
    Paste,
    SelectAll,

    // Terminal tabs
    #[allow(dead_code)]
    NewTerminal,
    CloseTerminal,

    // No mapped action
    None,
}

pub struct Keymap;

impl Keymap {
    /// Resolve a key event to an action based on the current focus.
    pub fn resolve(key: &KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);

        match key.code {
            // ── Global: Ctrl+letter ──────────────────────────────
            KeyCode::Char('q') if ctrl => Action::Quit,
            KeyCode::Char('b') if ctrl => Action::ToggleFileTree,
            KeyCode::Char('p') if ctrl => Action::ToggleSidePanel,
            KeyCode::Char('j') if ctrl => Action::ToggleTerminal,
            KeyCode::Char('t') if ctrl => Action::NewTab,
            KeyCode::Char('w') if ctrl => Action::CloseTab,
            KeyCode::Char('s') if ctrl => Action::Save,
            KeyCode::Char('f') if ctrl => Action::FindInBuffer,

            KeyCode::Char('z') if ctrl => Action::Undo,
            KeyCode::Char('y') if ctrl => Action::Redo,
            KeyCode::Char('c') if ctrl => Action::Copy,
            KeyCode::Char('x') if ctrl => Action::Cut,
            KeyCode::Char('v') if ctrl => Action::Paste,
            KeyCode::Char('a') if ctrl => Action::SelectAll,
            KeyCode::Char('k') if ctrl => Action::DeleteLine,
            KeyCode::Char('d') if ctrl => Action::DuplicateLine,
            KeyCode::Char('e') if ctrl => Action::GoToLineEnd,
            KeyCode::Char('h') if ctrl => Action::DeleteWordBack,

            // ── Panel focus: Alt+Number ──────────────────────────
            KeyCode::Char('1') if alt && !shift => Action::FocusLeftPanel,
            KeyCode::Char('2') if alt && !shift => Action::FocusEditor,
            KeyCode::Char('3') if alt && !shift => Action::FocusSidePanel,
            KeyCode::Char('4') if alt && !shift => Action::FocusTerminal,

            // ── Left panel modes: Alt+letter ─────────────────────
            KeyCode::Char('e') if alt => Action::SetLeftModeExplorer,
            KeyCode::Char('f') if alt => Action::SetLeftModeFind,
            KeyCode::Char('c') if alt => Action::SetLeftModeChanges,

            // ── Side panel modes: Alt+letter ─────────────────────
            KeyCode::Char('a') if alt => Action::SetSideModeChat,
            KeyCode::Char('w') if alt => Action::SetSideModeSwarm,
            KeyCode::Char('g') if alt => Action::SetSideModeGit,
            // Alt+Y: toggle auto-approve permissions for next prompt
            KeyCode::Char('y') if alt => Action::ToggleAutoApprove,

            // ── Layout presets: Alt+5..9 and Alt+0 ───────────────
            // Unshifted digits with Alt: keyboard-layout independent, not
            // grabbed by any common DE, consistent with Alt+1..4 panel focus.
            // Alt+5=layout1, Alt+6=layout2, ..., Alt+9=layout5, Alt+0=layout6
            KeyCode::Char('5') if alt && !ctrl => Action::SwitchLayout(0),
            KeyCode::Char('6') if alt && !ctrl => Action::SwitchLayout(1),
            KeyCode::Char('7') if alt && !ctrl => Action::SwitchLayout(2),
            KeyCode::Char('8') if alt && !ctrl => Action::SwitchLayout(3),
            KeyCode::Char('9') if alt && !ctrl => Action::SwitchLayout(4),
            KeyCode::Char('0') if alt && !ctrl => Action::SwitchLayout(5),

            // ── Tab cycling: Alt+O/I ─────────────────────────────
            KeyCode::Char('o') if alt => Action::CycleTabForward,
            KeyCode::Char('i') if alt => Action::CycleTabBack,

            // ── Preview toggle ───────────────────────────────────
            KeyCode::Char('p') if alt => Action::TogglePreview,

            // ── F-keys ───────────────────────────────────────────
            KeyCode::F(2) => Action::Rename,
            KeyCode::F(3) => Action::SearchInWorkspace,
            KeyCode::F(4) => Action::ToggleTerminal,
            KeyCode::F(5) => Action::FormatBuffer,
            KeyCode::F(6) => Action::CycleFormatLevel,
            KeyCode::F(11) => Action::ToggleFullscreen,

            // ── Tab character ────────────────────────────────────
            KeyCode::Tab => Action::Tab,

            // ── Word movement: Ctrl+Arrow ────────────────────────
            KeyCode::Left if ctrl && shift => Action::SelectWordLeft,
            KeyCode::Right if ctrl && shift => Action::SelectWordRight,
            KeyCode::Left if ctrl => Action::WordLeft,
            KeyCode::Right if ctrl => Action::WordRight,

            // ── Line movement: Alt+Up/Down ───────────────────────
            KeyCode::Up if alt => Action::MoveLineUp,
            KeyCode::Down if alt => Action::MoveLineDown,

            // ── Selection: Shift+Arrow ───────────────────────────
            KeyCode::Left if shift => Action::SelectLeft,
            KeyCode::Right if shift => Action::SelectRight,
            KeyCode::Up if shift => Action::SelectUp,
            KeyCode::Down if shift => Action::SelectDown,

            // ── Cursor movement ──────────────────────────────────
            KeyCode::Up => Action::CursorUp,
            KeyCode::Down => Action::CursorDown,
            KeyCode::Left => Action::CursorLeft,
            KeyCode::Right => Action::CursorRight,
            KeyCode::PageUp => Action::PageUp,
            KeyCode::PageDown => Action::PageDown,
            KeyCode::Home => Action::Home,
            KeyCode::End => Action::End,

            // ── Editing ──────────────────────────────────────────
            KeyCode::Backspace if ctrl => Action::DeleteWordBack,
            KeyCode::Delete if ctrl => Action::DeleteToLineEnd,
            KeyCode::Backspace => Action::Backspace,
            KeyCode::Delete => Action::Delete,
            KeyCode::Enter if alt => Action::AltEnter,
            KeyCode::Enter if shift => Action::AltEnter,
            KeyCode::Enter => Action::Enter,
            KeyCode::Char(c) if !ctrl && !alt => Action::InsertChar(c),

            _ => Action::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn test_ctrl_q_quits() {
        assert_eq!(
            Keymap::resolve(&key(KeyCode::Char('q'), KeyModifiers::CONTROL)),
            Action::Quit
        );
    }

    #[test]
    fn test_regular_char_inserts() {
        assert_eq!(
            Keymap::resolve(&key(KeyCode::Char('a'), KeyModifiers::NONE)),
            Action::InsertChar('a')
        );
    }

    #[test]
    fn test_ctrl_z_undoes() {
        assert_eq!(
            Keymap::resolve(&key(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            Action::Undo
        );
    }

    #[test]
    fn test_shift_arrow_selects() {
        assert_eq!(
            Keymap::resolve(&key(KeyCode::Up, KeyModifiers::SHIFT)),
            Action::SelectUp
        );
    }

    #[test]
    fn test_ctrl_arrow_word_movement() {
        assert_eq!(
            Keymap::resolve(&key(KeyCode::Left, KeyModifiers::CONTROL)),
            Action::WordLeft
        );
        assert_eq!(
            Keymap::resolve(&key(KeyCode::Right, KeyModifiers::CONTROL)),
            Action::WordRight
        );
    }

    #[test]
    fn test_shift_arrow_selection() {
        assert_eq!(
            Keymap::resolve(&key(KeyCode::Left, KeyModifiers::SHIFT)),
            Action::SelectLeft
        );
        assert_eq!(
            Keymap::resolve(&key(KeyCode::Right, KeyModifiers::SHIFT)),
            Action::SelectRight
        );
    }

    #[test]
    fn test_alt_number_focus() {
        assert_eq!(
            Keymap::resolve(&key(KeyCode::Char('1'), KeyModifiers::ALT)),
            Action::FocusLeftPanel
        );
        assert_eq!(
            Keymap::resolve(&key(KeyCode::Char('2'), KeyModifiers::ALT)),
            Action::FocusEditor
        );
        assert_eq!(
            Keymap::resolve(&key(KeyCode::Char('3'), KeyModifiers::ALT)),
            Action::FocusSidePanel
        );
        assert_eq!(
            Keymap::resolve(&key(KeyCode::Char('4'), KeyModifiers::ALT)),
            Action::FocusTerminal
        );
    }

    #[test]
    fn test_alt_letter_panel_modes() {
        assert_eq!(
            Keymap::resolve(&key(KeyCode::Char('e'), KeyModifiers::ALT)),
            Action::SetLeftModeExplorer
        );
        assert_eq!(
            Keymap::resolve(&key(KeyCode::Char('a'), KeyModifiers::ALT)),
            Action::SetSideModeChat
        );
    }

    #[test]
    fn test_ctrl_j_toggle_terminal() {
        assert_eq!(
            Keymap::resolve(&key(KeyCode::Char('j'), KeyModifiers::CONTROL)),
            Action::ToggleTerminal
        );
    }

    #[test]
    fn test_layout_preset_alt_digit() {
        assert_eq!(
            Keymap::resolve(&key(KeyCode::Char('5'), KeyModifiers::ALT)),
            Action::SwitchLayout(0)
        );
        assert_eq!(
            Keymap::resolve(&key(KeyCode::Char('9'), KeyModifiers::ALT)),
            Action::SwitchLayout(4)
        );
        assert_eq!(
            Keymap::resolve(&key(KeyCode::Char('0'), KeyModifiers::ALT)),
            Action::SwitchLayout(5)
        );
        // Must not fire without Alt
        assert_eq!(
            Keymap::resolve(&key(KeyCode::Char('5'), KeyModifiers::NONE)),
            Action::InsertChar('5')
        );
    }
}
