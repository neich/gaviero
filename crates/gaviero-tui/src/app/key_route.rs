//! The TUI's one key decision.
//!
//! Every key event — read by `event.rs` (already normalized for the
//! terminal gaviero runs in, see `platform::normalize_key`) or injected by the
//! Windows Ctrl+C forwarder — is decided here, once, by [`route_key`]: a pure
//! function of a [`KeyContext`] snapshot of the app and the key.
//! `controller::handle_event` only carries out the [`KeyRoute`] it returns.
//! When the route is [`KeyRoute::Action`], `controller::handle_action`
//! dispatches the action by focus — the same path mouse, remote and tests use.
//!
//! Precedence, first match wins:
//! 1. an open dialog ([`Modal`], in its own order) takes every key;
//! 2. a focused, running terminal takes every key it is not given up by
//!    (`panels::terminal::escapes_terminal`), unless a review is pending;
//!    Ctrl+C there copies the selection when there is one;
//! 3. move mode / bulk-move destination picking in the explorer;
//! 4. Esc, unless a review is pending (it goes to the review): closes the
//!    find bar, else steps back in HISTORY / SWARM;
//! 5. text keys right after a Windows paste are its ConPTY residue: dropped;
//! 6. everything else becomes `Keymap::resolve(key)`, resolved exactly once.
//!
//! Pastes and mouse events pass the same dialog gate ([`KeyContext::modal`]).

use super::*;
use crate::keymap::Keymap;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A dialog that owns the keyboard while it is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Modal {
    SettingsError,
    SyntaxError,
    RestartPopup,
    CodexTrust,
    QuitConfirm,
    BulkConfirm,
    TreeDialog,
    DefinitionPicker,
}

/// The side panels that take Esc themselves (it steps back a level).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PanelEscape {
    History,
    Swarm,
}

/// What the key decision looks at: a snapshot of the app.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct KeyContext {
    pub settings_error: bool,
    pub syntax_error: bool,
    pub restart_popup: bool,
    pub codex_trust: bool,
    pub quit_confirm: bool,
    pub bulk_confirm: bool,
    pub tree_dialog: bool,
    pub definition_picker: bool,
    /// A diff or batch review is pending.
    pub review: bool,
    /// The terminal has focus and its active tab runs.
    pub terminal: bool,
    pub terminal_selection: bool,
    /// The explorer is picking a file to move / a folder to move into.
    pub move_mode: bool,
    pub bulk_move_dest: bool,
    pub find_bar: bool,
    /// The focused side panel, when it takes Esc itself.
    pub panel_escape: Option<PanelEscape>,
    /// A Windows text paste just landed (`editing::paste_settling`).
    pub paste_settling: bool,
}

impl KeyContext {
    pub(crate) fn of(app: &App) -> KeyContext {
        let explorer = app.focus == Focus::FileTree && app.left_panel == LeftPanelMode::FileTree;
        KeyContext {
            settings_error: app.settings_error_dialog.is_some(),
            syntax_error: app.syntax_error_dialog.is_some(),
            restart_popup: app.restart.popup_open,
            codex_trust: app.codex_trust_dialog.is_some(),
            quit_confirm: app.quit_confirm,
            bulk_confirm: matches!(
                app.bulk_op_state,
                Some(
                    BulkOpState::ConfirmDelete { .. }
                        | BulkOpState::ConfirmMove { .. }
                        | BulkOpState::ConfirmCopy { .. }
                )
            ),
            tree_dialog: app.tree_dialog.is_some(),
            definition_picker: app.definition_picker.is_some(),
            review: app.has_active_review(),
            terminal: app.focus == Focus::Terminal
                && app
                    .terminal_manager
                    .active_instance()
                    .is_some_and(|inst| inst.spawned),
            terminal_selection: app.terminal_selection.has_selection(),
            move_mode: explorer && app.move_state.is_some(),
            bulk_move_dest: explorer
                && matches!(app.bulk_op_state, Some(BulkOpState::SelectingDest { .. })),
            find_bar: app.find_bar_active,
            panel_escape: match (app.focus, app.side_panel) {
                (Focus::SidePanel, SidePanelMode::HistoryPanel) => Some(PanelEscape::History),
                (Focus::SidePanel, SidePanelMode::SwarmDashboard) => Some(PanelEscape::Swarm),
                _ => None,
            },
            paste_settling: super::editing::paste_settling(app),
        }
    }

    /// The open dialog that owns the keyboard, in precedence order. Keys,
    /// pastes and mouse events all stop here.
    pub(crate) fn modal(&self) -> Option<Modal> {
        [
            (self.settings_error, Modal::SettingsError),
            (self.syntax_error, Modal::SyntaxError),
            (self.restart_popup, Modal::RestartPopup),
            (self.codex_trust, Modal::CodexTrust),
            (self.quit_confirm, Modal::QuitConfirm),
            (self.bulk_confirm, Modal::BulkConfirm),
            (self.tree_dialog, Modal::TreeDialog),
            (self.definition_picker, Modal::DefinitionPicker),
        ]
        .into_iter()
        .find_map(|(open, modal)| open.then_some(modal))
    }
}

/// What one key does.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum KeyRoute {
    /// The open dialog's own key handler.
    Modal(Modal),
    /// Ctrl+C in the terminal with a selection: copy it.
    TerminalCopy,
    /// To the terminal's shell, as the bytes it expects.
    TerminalBytes,
    MoveMode,
    BulkMoveDest,
    CloseFindBar,
    PanelEscape(PanelEscape),
    /// ConPTY residue of a paste that just landed.
    DropPasteStraggler,
    Action(Action),
}

/// The one key decision of the TUI (precedence in the module docs).
pub(crate) fn route_key(ctx: &KeyContext, key: &KeyEvent) -> KeyRoute {
    if let Some(modal) = ctx.modal() {
        return KeyRoute::Modal(modal);
    }
    let action = Keymap::resolve(key);
    if ctx.terminal && !ctx.review {
        let ctrl_c = key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl_c && ctx.terminal_selection {
            return KeyRoute::TerminalCopy;
        }
        return if crate::panels::terminal::escapes_terminal(key, &action) {
            KeyRoute::Action(action)
        } else {
            KeyRoute::TerminalBytes
        };
    }
    if ctx.move_mode {
        return KeyRoute::MoveMode;
    }
    if ctx.bulk_move_dest {
        return KeyRoute::BulkMoveDest;
    }
    // A pending review owns Esc (it asks to discard a batch review).
    if action == Action::Escape && !ctx.review {
        if ctx.find_bar {
            return KeyRoute::CloseFindBar;
        }
        if let Some(panel) = ctx.panel_escape {
            return KeyRoute::PanelEscape(panel);
        }
    }
    if ctx.paste_settling && matches!(action, Action::InsertChar(_) | Action::Enter | Action::Tab) {
        return KeyRoute::DropPasteStraggler;
    }
    KeyRoute::Action(action)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    const NONE: KeyContext = KeyContext {
        settings_error: false,
        syntax_error: false,
        restart_popup: false,
        codex_trust: false,
        quit_confirm: false,
        bulk_confirm: false,
        tree_dialog: false,
        definition_picker: false,
        review: false,
        terminal: false,
        terminal_selection: false,
        move_mode: false,
        bulk_move_dest: false,
        find_bar: false,
        panel_escape: None,
        paste_settling: false,
    };

    #[test]
    fn a_plain_key_resolves_once_through_the_keymap() {
        assert_eq!(route_key(&NONE, &key(KeyCode::Char('a'))), KeyRoute::Action(Action::InsertChar('a')));
        assert_eq!(route_key(&NONE, &ctrl('q')), KeyRoute::Action(Action::Quit));
        assert_eq!(route_key(&NONE, &key(KeyCode::Esc)), KeyRoute::Action(Action::Escape));
    }

    #[test]
    fn dialogs_come_first_in_their_own_order() {
        let all = KeyContext {
            settings_error: true,
            syntax_error: true,
            restart_popup: true,
            codex_trust: true,
            quit_confirm: true,
            bulk_confirm: true,
            tree_dialog: true,
            definition_picker: true,
            terminal: true,
            review: true,
            ..NONE
        };
        assert_eq!(all.modal(), Some(Modal::SettingsError));
        let order = [
            Modal::SettingsError,
            Modal::SyntaxError,
            Modal::RestartPopup,
            Modal::CodexTrust,
            Modal::QuitConfirm,
            Modal::BulkConfirm,
            Modal::TreeDialog,
            Modal::DefinitionPicker,
        ];
        let mut ctx = all;
        for modal in order {
            assert_eq!(route_key(&ctx, &key(KeyCode::Char('y'))), KeyRoute::Modal(modal));
            match modal {
                Modal::SettingsError => ctx.settings_error = false,
                Modal::SyntaxError => ctx.syntax_error = false,
                Modal::RestartPopup => ctx.restart_popup = false,
                Modal::CodexTrust => ctx.codex_trust = false,
                Modal::QuitConfirm => ctx.quit_confirm = false,
                Modal::BulkConfirm => ctx.bulk_confirm = false,
                Modal::TreeDialog => ctx.tree_dialog = false,
                Modal::DefinitionPicker => ctx.definition_picker = false,
            }
        }
        assert_eq!(ctx.modal(), None);
    }

    /// Ctrl+Q from the terminal opens the quit question; its `y` must answer
    /// the question, not reach the shell.
    #[test]
    fn a_dialog_opened_from_the_terminal_gets_the_next_key() {
        let ctx = KeyContext { terminal: true, quit_confirm: true, ..NONE };
        assert_eq!(route_key(&ctx, &key(KeyCode::Char('y'))), KeyRoute::Modal(Modal::QuitConfirm));
        let ctx = KeyContext { terminal: true, codex_trust: true, ..NONE };
        assert_eq!(route_key(&ctx, &key(KeyCode::Char('n'))), KeyRoute::Modal(Modal::CodexTrust));
    }

    #[test]
    fn the_terminal_takes_keys_except_app_chords() {
        let term = KeyContext { terminal: true, ..NONE };
        for k in [key(KeyCode::Char('a')), key(KeyCode::Esc), key(KeyCode::Enter), ctrl('c'), ctrl('d')] {
            assert_eq!(route_key(&term, &k), KeyRoute::TerminalBytes, "{k:?}");
        }
        assert_eq!(route_key(&term, &ctrl('q')), KeyRoute::Action(Action::Quit));
        let shift_pgup = KeyEvent::new(KeyCode::PageUp, KeyModifiers::SHIFT);
        assert_eq!(route_key(&term, &shift_pgup), KeyRoute::Action(Action::PageUp));
        // Ctrl+C copies a selection instead of interrupting.
        let selected = KeyContext { terminal_selection: true, ..term };
        assert_eq!(route_key(&selected, &ctrl('c')), KeyRoute::TerminalCopy);
        // A pending review locks the terminal: keys become (swallowed) actions.
        let review = KeyContext { review: true, ..term };
        assert_eq!(route_key(&review, &key(KeyCode::Char('a'))), KeyRoute::Action(Action::InsertChar('a')));
    }

    #[test]
    fn explorer_modes_then_escape_targets() {
        let moving = KeyContext { move_mode: true, find_bar: true, ..NONE };
        assert_eq!(route_key(&moving, &key(KeyCode::Esc)), KeyRoute::MoveMode);
        let dest = KeyContext { bulk_move_dest: true, ..NONE };
        assert_eq!(route_key(&dest, &key(KeyCode::Down)), KeyRoute::BulkMoveDest);
        // The find bar closes before HISTORY / SWARM step back.
        let find = KeyContext { find_bar: true, panel_escape: Some(PanelEscape::History), ..NONE };
        assert_eq!(route_key(&find, &key(KeyCode::Esc)), KeyRoute::CloseFindBar);
        let history = KeyContext { panel_escape: Some(PanelEscape::History), ..NONE };
        assert_eq!(route_key(&history, &key(KeyCode::Esc)), KeyRoute::PanelEscape(PanelEscape::History));
        let swarm = KeyContext { panel_escape: Some(PanelEscape::Swarm), ..NONE };
        assert_eq!(route_key(&swarm, &key(KeyCode::Esc)), KeyRoute::PanelEscape(PanelEscape::Swarm));
        // Other keys in those panels are ordinary actions.
        assert_eq!(route_key(&swarm, &key(KeyCode::Char('j'))), KeyRoute::Action(Action::InsertChar('j')));
        // A pending review owns Esc.
        let review = KeyContext { review: true, ..find };
        assert_eq!(route_key(&review, &key(KeyCode::Esc)), KeyRoute::Action(Action::Escape));
    }

    #[test]
    fn text_keys_right_after_a_windows_paste_are_dropped() {
        let settling = KeyContext { paste_settling: true, ..NONE };
        for k in [key(KeyCode::Char('\\')), key(KeyCode::Enter), key(KeyCode::Tab)] {
            assert_eq!(route_key(&settling, &k), KeyRoute::DropPasteStraggler, "{k:?}");
        }
        assert_eq!(route_key(&settling, &ctrl('s')), KeyRoute::Action(Action::Save));
        assert_eq!(route_key(&settling, &key(KeyCode::Esc)), KeyRoute::Action(Action::Escape));
    }
}
