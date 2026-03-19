use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Quit,
    ToggleFileTree,
    ToggleSidePanel,
    ToggleTerminal,
    CycleFocus,
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    NewTab,
    CycleTabForward,
    CycleTabBack,
    CloseTab,

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
    #[allow(dead_code)] SelectLeft,
    #[allow(dead_code)] SelectRight,
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
    SearchInWorkspace,
    CycleLeftPanel,
    ShiftLeft,
    ShiftRight,
    ShiftUp,
    ShiftDown,

    // Chat
    AltEnter,

    // Swarm (triggered by /swarm command, not a keybinding)
    #[allow(dead_code)]
    ToggleSwarmDashboard,

    // Side panel mode switching (Ctrl+1/2/3)
    SidePanelChat,
    SidePanelSwarm,
    SidePanelGit,

    // Layout
    ToggleFullscreen,
    SwitchLayout(u8), // 0-9
    Rename,

    // Clipboard
    Copy,
    Cut,
    Paste,
    SelectAll,

    // Terminal tabs
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
            // Global keybinds (Ctrl+)
            KeyCode::Char('q') if ctrl => Action::Quit,
            KeyCode::Char('b') if ctrl => Action::ToggleFileTree,
            KeyCode::Char('p') if ctrl => Action::ToggleSidePanel,
            KeyCode::F(4) => Action::ToggleTerminal,
            KeyCode::F(8) => Action::NewTerminal,
            KeyCode::Char('t') if ctrl => Action::NewTab,
            KeyCode::Char('w') if ctrl => Action::CloseTab,
            KeyCode::Char('s') if ctrl => Action::Save,
            KeyCode::Char('p') if alt => Action::TogglePreview,
            KeyCode::F(3) => Action::SearchInWorkspace,
            KeyCode::Char('f') if ctrl => Action::ToggleFullscreen,

            // Side panel mode switching (Ctrl+1/2/3)
            KeyCode::Char('1') if ctrl => Action::SidePanelChat,
            KeyCode::Char('2') if ctrl => Action::SidePanelSwarm,
            KeyCode::Char('3') if ctrl => Action::SidePanelGit,

            // Layout presets (Ctrl+0,4..9 or Alt+0..9 — terminals vary)
            KeyCode::Char(c @ '0'..='9') if ctrl || alt => Action::SwitchLayout((c as u8) - b'0'),

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

            // Tab cycling
            KeyCode::Char(']') if alt => Action::CycleTabForward,
            KeyCode::Char('[') if alt => Action::CycleTabBack,

            // Focus cycling (Ctrl+\)
            KeyCode::Char('\\') if ctrl => Action::CycleFocus,

            // Tab inserts a tab character in editor
            KeyCode::Tab => Action::Tab,

            // Focus navigation (Ctrl+Arrow)
            KeyCode::Left if ctrl => Action::FocusLeft,
            KeyCode::Right if ctrl => Action::FocusRight,
            KeyCode::Up if ctrl => Action::FocusUp,
            KeyCode::Down if ctrl => Action::FocusDown,

            // Context-sensitive Shift+Left/Right:
            // Editor: cycle tabs. FileTree/Search: cycle left panel mode.
            KeyCode::Left if shift && !ctrl => Action::ShiftLeft,
            KeyCode::Right if shift && !ctrl => Action::ShiftRight,

            // Move line (Alt+Up/Down)
            KeyCode::Up if alt => Action::MoveLineUp,
            KeyCode::Down if alt => Action::MoveLineDown,

            // Selection (Shift+Up/Down)
            KeyCode::Up if shift => Action::ShiftUp,
            KeyCode::Down if shift => Action::ShiftDown,
            KeyCode::Up => Action::CursorUp,
            KeyCode::Down => Action::CursorDown,
            KeyCode::Left => Action::CursorLeft,
            KeyCode::Right => Action::CursorRight,
            KeyCode::PageUp => Action::PageUp,
            KeyCode::PageDown => Action::PageDown,
            KeyCode::Home => Action::Home,
            KeyCode::End => Action::End,

            // Misc
            KeyCode::F(2) => Action::Rename,
            KeyCode::F(5) => Action::FormatBuffer,
            KeyCode::F(6) => Action::CycleFormatLevel,
            KeyCode::F(7) => Action::CycleLeftPanel,

            // Editing
            KeyCode::Backspace if ctrl => Action::DeleteWordBack,
            KeyCode::Delete if ctrl => Action::DeleteToLineEnd,
            KeyCode::Backspace => Action::Backspace,
            KeyCode::Delete => Action::Delete,
            KeyCode::Enter if alt => Action::AltEnter,
            KeyCode::Enter => Action::Enter,
            KeyCode::Char(c) if !ctrl && !alt => Action::InsertChar(c),

            _ => Action::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEventKind;

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
            Action::ShiftUp
        );
    }
}
