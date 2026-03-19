//! Agent chat panel — conversation history + input for Claude Code interaction.

use ratatui::{
    buffer::Buffer as RataBuf,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Widget},
};

use std::path::PathBuf;
use std::time::Instant;

use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use crate::app::collapse_file_blocks;
use crate::theme;
use crate::theme::Theme;

// ── Data types ──────────────────────────────────────────────────

/// Type of file attachment.
#[derive(Debug, Clone, PartialEq)]
pub enum AttachmentKind {
    /// Text/code file — contents included in prompt context.
    Text,
    /// Image file — passed via --file flag to Claude CLI.
    Image,
}

/// A file attached to the next chat message.
#[derive(Debug, Clone)]
pub struct Attachment {
    /// Absolute path to the file on disk.
    pub path: PathBuf,
    /// Display name (filename component).
    pub display_name: String,
    /// Type of attachment.
    pub kind: AttachmentKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChatRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
    pub tool_calls: Vec<String>,
}

// ── Chat state ──────────────────────────────────────────────────

/// Autocomplete state for @file references.
#[derive(Debug)]
pub struct FileAutocomplete {
    /// Whether the autocomplete popup is visible.
    pub active: bool,
    /// The partial text after @ being matched.
    pub query: String,
    /// Byte offset of the @ in the input string.
    pub at_pos: usize,
    /// Filtered file paths matching the query.
    pub matches: Vec<String>,
    /// Currently selected index in the matches list.
    pub selected: usize,
}

impl FileAutocomplete {
    fn new() -> Self {
        Self {
            active: false,
            query: String::new(),
            at_pos: 0,
            matches: Vec::new(),
            selected: 0,
        }
    }

    pub fn reset(&mut self) {
        self.active = false;
        self.query.clear();
        self.at_pos = 0;
        self.matches.clear();
        self.selected = 0;
    }
}

/// One conversation (tab) in the chat panel.
#[derive(Debug)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub messages: Vec<ChatMessage>,
    /// Per-conversation model override (None = use global default).
    pub model_override: Option<String>,
    /// Per-conversation effort level override (None = use global default).
    pub effort_override: Option<String>,
    /// Per-conversation memory namespace override (None = use workspace default).
    pub namespace_override: Option<String>,
    /// Whether this conversation is currently streaming a response.
    pub is_streaming: bool,
    /// Current activity description shown during streaming (e.g. "Thinking...", "Reading src/main.rs").
    pub streaming_status: String,
    /// When streaming started, for elapsed time display.
    pub streaming_started_at: Option<Instant>,
}

/// Global agent settings read from workspace settings.
#[derive(Debug, Clone)]
pub struct AgentSettings {
    pub model: String,
    pub effort: String,
    pub max_tokens: u32,
    /// The namespace to write memories to.
    pub write_namespace: String,
    /// Namespaces to search when reading (always includes write_namespace).
    pub read_namespaces: Vec<String>,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            model: "sonnet".to_string(),
            effort: "off".to_string(),
            max_tokens: 16384,
            write_namespace: "default".to_string(),
            read_namespaces: vec!["default".to_string()],
        }
    }
}

#[derive(Debug)]
pub struct AgentChatState {
    /// All conversations for this workspace.
    pub conversations: Vec<Conversation>,
    /// Index of the active conversation.
    pub active_conv: usize,

    pub input: String,
    pub input_cursor: usize,
    pub scroll_offset: usize,
    /// Current position in history (None = new input, Some(idx) = browsing user messages).
    pub history_index: Option<usize>,
    /// Stashed current input when browsing history.
    pub history_stash: String,
    /// When true, the input field is editing the conversation title instead of a chat message.
    pub renaming: bool,
    pub autocomplete: FileAutocomplete,
    /// Files attached to the next message.
    pub attachments: Vec<Attachment>,

    /// Global agent settings (from workspace config).
    pub agent_settings: AgentSettings,

    /// When true, user is browsing messages to copy content.
    pub browse_mode: bool,
    /// Index of the currently highlighted message (into active conversation's messages).
    pub browsed_msg: usize,
    /// Cached model options discovered from the `claude` CLI (lazily populated).
    cli_model_options: Option<Vec<String>>,
    /// Tick counter for spinner animation (incremented on each Event::Tick while streaming).
    pub tick_count: u64,

    /// Cached rendered line texts (set during render) for mouse text selection.
    pub rendered_lines_cache: Vec<String>,
    /// Cached conversation area rect (set during render).
    pub conv_area_cache: Option<Rect>,
    /// Text selection anchor: (rendered_line_index, char_index).
    pub text_sel_anchor: Option<(usize, usize)>,
    /// Text selection end: (rendered_line_index, char_index).
    pub text_sel_end: Option<(usize, usize)>,
    /// Whether mouse is currently dragging to select chat text.
    pub chat_dragging: bool,
}

impl AgentChatState {
    pub fn new() -> Self {
        let conv = Conversation {
            id: gaviero_core::session_state::new_conversation_id(),
            title: "New Chat".to_string(),
            messages: Vec::new(),
            model_override: None,
            effort_override: None,
            namespace_override: None,
            is_streaming: false,
            streaming_status: String::new(),
            streaming_started_at: None,
        };
        Self {
            conversations: vec![conv],
            active_conv: 0,
            input: String::new(),
            input_cursor: 0,
            scroll_offset: 0,
            history_index: None,
            history_stash: String::new(),
            autocomplete: FileAutocomplete::new(),
            attachments: Vec::new(),
            renaming: false,
            agent_settings: AgentSettings::default(),
            browse_mode: false,
            browsed_msg: 0,
            cli_model_options: None,
            tick_count: 0,
            rendered_lines_cache: Vec::new(),
            conv_area_cache: None,
            text_sel_anchor: None,
            text_sel_end: None,
            chat_dragging: false,
        }
    }

    /// Get model options from the `claude` CLI (cached after first call).
    fn model_options(&mut self) -> &[String] {
        if self.cli_model_options.is_none() {
            self.cli_model_options = Some(
                gaviero_core::acp::session::discover_model_options(),
            );
        }
        self.cli_model_options.as_deref().unwrap_or(&[])
    }

    /// Is the active conversation currently streaming?
    pub fn active_conv_streaming(&self) -> bool {
        self.conversations[self.active_conv].is_streaming
    }

    /// Get the effective model for the active conversation.
    pub fn effective_model(&self) -> &str {
        self.conversations[self.active_conv]
            .model_override
            .as_deref()
            .unwrap_or(&self.agent_settings.model)
    }

    /// Get the effective effort level for the active conversation.
    pub fn effective_effort(&self) -> &str {
        self.conversations[self.active_conv]
            .effort_override
            .as_deref()
            .unwrap_or(&self.agent_settings.effort)
    }

    /// Get the effective write namespace for the active conversation.
    pub fn effective_write_namespace(&self) -> &str {
        self.conversations[self.active_conv]
            .namespace_override
            .as_deref()
            .unwrap_or(&self.agent_settings.write_namespace)
    }

    /// Get all namespaces to search when reading memory.
    /// Always includes the write namespace.
    pub fn effective_read_namespaces(&self) -> Vec<String> {
        let write_ns = self.effective_write_namespace().to_string();
        let mut nss = vec![write_ns.clone()];
        for ns in &self.agent_settings.read_namespaces {
            if !nss.contains(ns) {
                nss.push(ns.clone());
            }
        }
        nss
    }

    /// Process slash commands in input. Returns true if a command was handled.
    pub fn process_slash_command(&mut self) -> bool {
        let input = self.input.trim().to_string();
        if !input.starts_with('/') {
            return false;
        }

        // Record the command in the conversation history
        self.add_user_message(&input);

        let parts: Vec<&str> = input.splitn(2, ' ').collect();
        let cmd = parts[0];
        let arg = parts.get(1).map(|s| s.trim()).unwrap_or("");

        match cmd {
            "/model" => {
                if arg.is_empty() {
                    let current = self.effective_model().to_string();
                    let options = self.model_options().to_vec();
                    let list = if options.is_empty() {
                        "sonnet, opus, haiku (or any full model name)".to_string()
                    } else {
                        options.join(", ")
                    };
                    self.add_system_message(&format!(
                        "Current model: {}\nAvailable: {}\nUsage: /model <name>",
                        current, list
                    ));
                } else {
                    let model = match arg {
                        "sonnet" | "claude-sonnet" => "sonnet",
                        "opus" | "claude-opus" => "opus",
                        "haiku" | "claude-haiku" => "haiku",
                        other => other, // Allow arbitrary model strings
                    };
                    self.conversations[self.active_conv].model_override =
                        Some(model.to_string());
                    self.add_system_message(&format!("Model set to: {}", model));
                }
                self.input.clear();
                self.input_cursor = 0;
                true
            }
            "/thinking" | "/effort" => {
                if arg.is_empty() {
                    let current = self.effective_effort();
                    self.add_system_message(&format!(
                        "Effort level: {}.\nUsage: /effort <off|low|medium|high|max>",
                        current
                    ));
                } else {
                    let level = match arg {
                        "off" | "0" | "none" => "off",
                        "low" | "l" => "low",
                        "medium" | "med" | "m" => "medium",
                        "high" | "h" => "high",
                        "max" => "max",
                        _ => {
                            self.add_system_message(
                                "Invalid effort level. Use: off, low, medium, high, max"
                            );
                            self.input.clear();
                            self.input_cursor = 0;
                            return true;
                        }
                    };
                    self.conversations[self.active_conv].effort_override =
                        Some(level.to_string());
                    self.add_system_message(&format!("Effort level set to: {}", level));
                }
                self.input.clear();
                self.input_cursor = 0;
                true
            }
            "/compact" => {
                let keep = if arg.is_empty() {
                    6 // default: keep last 6 messages (3 turns)
                } else {
                    arg.parse::<usize>().unwrap_or(6)
                };
                let conv = &mut self.conversations[self.active_conv];
                let total = conv.messages.len();
                if total <= keep {
                    self.add_system_message(&format!(
                        "Nothing to compact ({} messages, keeping {})", total, keep
                    ));
                } else {
                    let removed = total - keep;
                    // Summarize removed messages into a single system note
                    let summary = format!(
                        "[{} earlier messages compacted]",
                        removed
                    );
                    let kept: Vec<ChatMessage> = conv.messages.split_off(total - keep);
                    conv.messages.clear();
                    conv.messages.push(ChatMessage {
                        role: ChatRole::System,
                        content: summary,
                        tool_calls: Vec::new(),
                    });
                    conv.messages.extend(kept);

                    let (chars, pct) = self.estimate_context();
                    self.add_system_message(&format!(
                        "Compacted: removed {} messages, kept {}. Context: ~{}% of limit",
                        removed, keep, pct
                    ));
                    let _ = chars; // used via pct
                }
                self.input.clear();
                self.input_cursor = 0;
                true
            }
            "/context" => {
                let (chars, pct) = self.estimate_context();
                let tokens_est = chars / 4;
                let limit = self.context_limit_tokens();
                self.add_system_message(&format!(
                    "Context estimate: ~{} tokens (~{}% of {} limit)",
                    tokens_est, pct, limit
                ));
                self.input.clear();
                self.input_cursor = 0;
                true
            }
            "/namespace" | "/ns" => {
                if arg.is_empty() {
                    let write = self.effective_write_namespace().to_string();
                    let read = self.effective_read_namespaces();
                    let read_str = read.iter()
                        .map(|ns| if *ns == write { format!("{} (write)", ns) } else { ns.clone() })
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.add_system_message(&format!(
                        "Write namespace: {}\nRead namespaces: [{}]",
                        write, read_str
                    ));
                } else {
                    self.conversations[self.active_conv].namespace_override =
                        Some(arg.to_string());
                    self.add_system_message(&format!(
                        "Write namespace set to: {} (for this conversation)",
                        arg
                    ));
                }
                self.input.clear();
                self.input_cursor = 0;
                true
            }
            "/help" => {
                self.add_system_message(
                    "Available commands:\n\
                     /model <name>      — Set Claude model (sonnet, opus, haiku)\n\
                     /effort <level>    — Set effort level (off, low, medium, high, max)\n\
                     /namespace <name>  — Set memory namespace (or show current)\n\
                     /attach <path>     — Attach a file (text or image)\n\
                     /attach            — List current attachments\n\
                     /detach <name|all> — Remove attachment(s)\n\
                     /compact [N]       — Keep last N messages (default 6), discard older\n\
                     /context           — Show estimated context usage\n\
                     /help              — Show this help\n\n\
                     Keyboard shortcuts:\n\
                     F2                 — Rename active conversation tab\n\
                     Ctrl+T             — New conversation tab\n\
                     Ctrl+C             — Cancel streaming / enter browse mode\n\
                     Ctrl+V             — Paste text, or attach clipboard image\n\
                     Alt+Enter          — Insert newline in input\n\
                     PageUp / PageDown  — Scroll chat history\n\
                     Esc                — Clear input / return to editor\n\n\
                     Use @filename to reference workspace files in your prompt.\n\
                     Use /attach to attach files from outside the workspace."
                );
                self.input.clear();
                self.input_cursor = 0;
                true
            }
            _ => {
                self.add_system_message(&format!("Unknown command: {}. Type /help for available commands.", cmd));
                self.input.clear();
                self.input_cursor = 0;
                true
            }
        }
    }

    /// Estimate context size in (chars, percent of limit).
    pub fn estimate_context(&self) -> (usize, usize) {
        let conv = &self.conversations[self.active_conv];
        let mut chars: usize = 0;
        for msg in &conv.messages {
            chars += msg.content.len();
            for tc in &msg.tool_calls {
                chars += tc.len();
            }
        }
        // Add estimated system prompt overhead (~500 chars)
        chars += 500;
        let limit = self.context_limit_tokens();
        let tokens_est = chars / 4; // rough: ~4 chars per token
        let pct = if limit > 0 {
            (tokens_est * 100 / limit).min(100)
        } else {
            0
        };
        (chars, pct)
    }

    /// Context window size in tokens for the effective model.
    fn context_limit_tokens(&self) -> usize {
        let model = self.effective_model();
        match model {
            "opus" => 200_000,
            "sonnet" => 200_000,
            "haiku" => 200_000,
            _ => 200_000,
        }
    }

    pub fn add_system_message(&mut self, content: &str) {
        self.conversations[self.active_conv].messages.push(ChatMessage {
            role: ChatRole::System,
            content: content.to_string(),
            tool_calls: Vec::new(),
        });
        self.scroll_to_bottom();
    }

    // ── Active conversation helpers ─────────────────────────────

    fn messages(&self) -> &Vec<ChatMessage> {
        &self.conversations[self.active_conv].messages
    }

    fn messages_mut(&mut self) -> &mut Vec<ChatMessage> {
        &mut self.conversations[self.active_conv].messages
    }

    #[allow(dead_code)]
    pub fn active_conversation(&self) -> &Conversation {
        &self.conversations[self.active_conv]
    }

    /// Create a new conversation and switch to it.
    /// Start renaming the active conversation. Puts current title into the input field.
    pub fn start_rename(&mut self) {
        let title = self.conversations[self.active_conv].title.clone();
        self.input = title;
        self.input_cursor = self.input.len();
        self.renaming = true;
    }

    /// Confirm the rename — apply input as the new title.
    pub fn confirm_rename(&mut self) {
        let new_title = self.input.trim().to_string();
        if !new_title.is_empty() {
            self.conversations[self.active_conv].title = new_title;
        }
        self.input.clear();
        self.input_cursor = 0;
        self.renaming = false;
    }

    /// Cancel the rename — restore input field.
    pub fn cancel_rename(&mut self) {
        self.input.clear();
        self.input_cursor = 0;
        self.renaming = false;
    }

    pub fn new_conversation(&mut self) {
        let conv = Conversation {
            id: gaviero_core::session_state::new_conversation_id(),
            title: "New Chat".to_string(),
            messages: Vec::new(),
            model_override: None,
            effort_override: None,
            namespace_override: None,
            is_streaming: false,
            streaming_status: String::new(),
            streaming_started_at: None,
        };
        self.conversations.push(conv);
        self.active_conv = self.conversations.len() - 1;
        self.scroll_offset = 0;
        self.input.clear();
        self.input_cursor = 0;
    }

    /// Hit-test chat conversation tabs. Returns `Some(index)` for a tab click,
    /// or `Some(self.conversations.len())` for the "+" button.
    pub fn conv_tab_at_x(&self, click_x: u16, area_x: u16) -> Option<usize> {
        let mut x = area_x;
        for (i, conv) in self.conversations.iter().enumerate() {
            let is_active = i == self.active_conv;
            let title: String = conv.title.chars().take(15).collect();
            let label_len = if is_active {
                format!(" [{}] ", title).len() as u16
            } else {
                format!("  {}  ", title).len() as u16
            };
            let tab_width = label_len + 1; // +1 for separator │
            if click_x >= x && click_x < x + tab_width {
                return Some(i);
            }
            x += tab_width;
        }
        // Check "+" button (space + '+')
        if click_x >= x && click_x < x + 2 {
            return Some(self.conversations.len());
        }
        None
    }

    /// Switch to conversation by index.
    pub fn switch_conversation(&mut self, idx: usize) {
        if idx < self.conversations.len() {
            self.active_conv = idx;
            self.scroll_offset = usize::MAX; // scroll to bottom
        }
    }

    /// Cycle to the next conversation.
    pub fn next_conversation(&mut self) {
        if !self.conversations.is_empty() {
            self.active_conv = (self.active_conv + 1) % self.conversations.len();
            self.scroll_offset = usize::MAX;
            self.history_index = None;
            self.history_stash.clear();
        }
    }

    /// Cycle to the previous conversation.
    pub fn prev_conversation(&mut self) {
        if !self.conversations.is_empty() {
            self.active_conv = if self.active_conv == 0 {
                self.conversations.len() - 1
            } else {
                self.active_conv - 1
            };
            self.scroll_offset = usize::MAX;
            self.history_index = None;
            self.history_stash.clear();
        }
    }

    /// Get all messages for multi-turn context (to send to Claude).
    pub fn context_messages(&self) -> Vec<(&str, &str)> {
        self.messages()
            .iter()
            .filter(|m| m.role == ChatRole::User || m.role == ChatRole::Assistant)
            .map(|m| {
                let role = match m.role {
                    ChatRole::User => "user",
                    ChatRole::Assistant => "assistant",
                    _ => "system",
                };
                (role, m.content.as_str())
            })
            .collect()
    }

    // ── Input editing ──────────────────────────────────────────

    pub fn insert_char(&mut self, ch: char) {
        self.input.insert(self.input_cursor, ch);
        self.input_cursor += ch.len_utf8();
        self.update_autocomplete();
    }

    pub fn backspace(&mut self) {
        if self.input_cursor > 0 {
            let prev = self.input[..self.input_cursor]
                .chars()
                .last()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.input.drain(self.input_cursor - prev..self.input_cursor);
            self.input_cursor -= prev;
            self.update_autocomplete();
        }
    }

    pub fn delete(&mut self) {
        if self.input_cursor < self.input.len() {
            let next = self.input[self.input_cursor..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.input.drain(self.input_cursor..self.input_cursor + next);
        }
    }

    pub fn move_left(&mut self) {
        if self.input_cursor > 0 {
            let prev = self.input[..self.input_cursor]
                .chars()
                .last()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.input_cursor -= prev;
        }
    }

    pub fn move_right(&mut self) {
        if self.input_cursor < self.input.len() {
            let next = self.input[self.input_cursor..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.input_cursor += next;
        }
    }

    pub fn move_home(&mut self) {
        self.input_cursor = 0;
    }

    pub fn move_end(&mut self) {
        self.input_cursor = self.input.len();
    }

    /// Take the input text (for sending), clear the input field.
    pub fn take_input(&mut self) -> String {
        let text = self.input.clone();
        self.history_index = None;
        self.history_stash.clear();
        self.input.clear();
        self.input_cursor = 0;
        self.autocomplete.reset();
        text
    }

    /// Get user messages from the active conversation (chronological, owned).
    fn active_user_messages(&self) -> Vec<String> {
        self.conversations[self.active_conv]
            .messages
            .iter()
            .filter(|m| m.role == ChatRole::User)
            .map(|m| m.content.clone())
            .collect()
    }

    /// Navigate history upward (older). Called when Up is pressed with empty input.
    pub fn history_up(&mut self) {
        let user_msgs = self.active_user_messages();
        if user_msgs.is_empty() {
            return;
        }
        match self.history_index {
            None => {
                self.history_stash = self.input.clone();
                let idx = user_msgs.len() - 1;
                self.history_index = Some(idx);
                self.input = user_msgs[idx].clone();
            }
            Some(idx) if idx > 0 => {
                let new_idx = idx - 1;
                self.history_index = Some(new_idx);
                self.input = user_msgs[new_idx].clone();
            }
            _ => {}
        }
        self.input_cursor = self.input.len();
    }

    /// Navigate history downward (newer). Called when Down is pressed while browsing history.
    pub fn history_down(&mut self) {
        let Some(idx) = self.history_index else { return };
        let user_msgs = self.active_user_messages();
        if idx + 1 < user_msgs.len() {
            let new_idx = idx + 1;
            self.history_index = Some(new_idx);
            self.input = user_msgs[new_idx].clone();
        } else {
            self.history_index = None;
            self.input = std::mem::take(&mut self.history_stash);
        }
        self.input_cursor = self.input.len();
    }

    // ── Browse mode (copy from chat) ──────────────────────────

    /// Enter browse mode, selecting the last message.
    pub fn enter_browse_mode(&mut self) {
        let msg_count = self.conversations[self.active_conv].messages.len();
        if msg_count == 0 {
            return;
        }
        self.browse_mode = true;
        self.browsed_msg = msg_count - 1;
    }

    /// Exit browse mode.
    pub fn exit_browse_mode(&mut self) {
        self.browse_mode = false;
    }

    // ── Mouse text selection ─────────────────────────────────────

    /// Map screen coordinates to a position in the rendered lines cache.
    /// Returns (line_index, char_index) where char_index is the character offset.
    pub fn screen_to_text_pos(&self, col: u16, row: u16) -> Option<(usize, usize)> {
        let area = self.conv_area_cache?;
        if row < area.y || row >= area.y + area.height {
            return None;
        }
        let viewport_row = (row - area.y) as usize;
        let line_idx = self.scroll_offset + viewport_row;
        if line_idx >= self.rendered_lines_cache.len() {
            // Clamp to last line
            let last = self.rendered_lines_cache.len().saturating_sub(1);
            let char_count = self.rendered_lines_cache.get(last)
                .map(|l| l.chars().count()).unwrap_or(0);
            return Some((last, char_count));
        }
        let target_col = col.saturating_sub(area.x) as usize;
        let line = &self.rendered_lines_cache[line_idx];
        let mut current_col = 0usize;
        let mut char_idx = 0usize;
        for ch in line.chars() {
            let w = UnicodeWidthChar::width(ch).unwrap_or(1);
            if current_col + w > target_col {
                return Some((line_idx, char_idx));
            }
            current_col += w;
            char_idx += 1;
        }
        Some((line_idx, char_idx))
    }

    /// Start a text selection at the given position.
    pub fn start_text_selection(&mut self, line_idx: usize, char_idx: usize) {
        self.text_sel_anchor = Some((line_idx, char_idx));
        self.text_sel_end = Some((line_idx, char_idx));
        self.chat_dragging = true;
        // Exit browse mode if active
        self.browse_mode = false;
    }

    /// Extend the text selection to the given position.
    pub fn extend_text_selection(&mut self, line_idx: usize, char_idx: usize) {
        self.text_sel_end = Some((line_idx, char_idx));
    }

    /// Clear the text selection.
    pub fn clear_text_selection(&mut self) {
        self.text_sel_anchor = None;
        self.text_sel_end = None;
        self.chat_dragging = false;
    }

    /// Get the ordered selection range: (start_line, start_char, end_line, end_char).
    fn text_selection_range(&self) -> Option<(usize, usize, usize, usize)> {
        let (al, ac) = self.text_sel_anchor?;
        let (el, ec) = self.text_sel_end?;
        if al < el || (al == el && ac <= ec) {
            Some((al, ac, el, ec))
        } else {
            Some((el, ec, al, ac))
        }
    }

    /// Check if a character at (line_idx, char_idx) is within the selection.
    fn is_char_selected(&self, line_idx: usize, char_idx: usize) -> bool {
        let Some((sl, sc, el, ec)) = self.text_selection_range() else {
            return false;
        };
        if sl == el && sc == ec {
            return false;
        }
        if line_idx < sl || line_idx > el {
            return false;
        }
        if line_idx == sl && line_idx == el {
            return char_idx >= sc && char_idx < ec;
        }
        if line_idx == sl {
            return char_idx >= sc;
        }
        if line_idx == el {
            return char_idx < ec;
        }
        true
    }

    /// Extract the selected text from the cached rendered lines.
    pub fn selected_chat_text(&self) -> Option<String> {
        let (sl, sc, el, ec) = self.text_selection_range()?;
        if sl == el && sc == ec {
            return None;
        }
        let mut result = String::new();
        for line_idx in sl..=el {
            if line_idx >= self.rendered_lines_cache.len() {
                break;
            }
            let line = &self.rendered_lines_cache[line_idx];
            let chars: Vec<char> = line.chars().collect();
            let start_c = if line_idx == sl { sc.min(chars.len()) } else { 0 };
            let end_c = if line_idx == el { ec.min(chars.len()) } else { chars.len() };
            if line_idx > sl {
                result.push('\n');
            }
            let selected: String = chars[start_c..end_c].iter().collect();
            result.push_str(&selected);
        }
        if result.is_empty() { None } else { Some(result) }
    }

    /// Move to the previous message in browse mode.
    pub fn browse_up(&mut self) {
        if self.browsed_msg > 0 {
            self.browsed_msg -= 1;
        }
    }

    /// Move to the next message in browse mode.
    pub fn browse_down(&mut self) {
        let msg_count = self.conversations[self.active_conv].messages.len();
        if self.browsed_msg + 1 < msg_count {
            self.browsed_msg += 1;
        }
    }

    /// Get the content of the currently browsed message.
    pub fn browsed_message_content(&self) -> Option<String> {
        self.conversations[self.active_conv]
            .messages
            .get(self.browsed_msg)
            .map(|m| m.content.clone())
    }

    // ── @file autocomplete ─────────────────────────────────────

    /// Check if cursor is inside an @reference and update autocomplete state.
    fn update_autocomplete(&mut self) {
        let before_cursor = &self.input[..self.input_cursor];

        // Find the last '@' before cursor that isn't preceded by a non-whitespace char
        let at_pos = before_cursor.rfind('@');
        match at_pos {
            Some(pos) => {
                // Check that @ is at start or preceded by whitespace
                if pos > 0 {
                    let prev_byte = self.input.as_bytes()[pos - 1];
                    if prev_byte != b' ' && prev_byte != b'\n' && prev_byte != b'\t' {
                        self.autocomplete.reset();
                        return;
                    }
                }
                // Extract the query (text after @)
                let query = &before_cursor[pos + 1..];
                // Deactivate if query contains spaces (completed reference)
                if query.contains(' ') {
                    self.autocomplete.reset();
                    return;
                }
                self.autocomplete.active = true;
                self.autocomplete.at_pos = pos;
                self.autocomplete.query = query.to_string();
                self.autocomplete.selected = 0;
                // Matches will be updated by the caller (App) which has access to file list
            }
            None => {
                self.autocomplete.reset();
            }
        }
    }

    /// Update autocomplete matches from a list of workspace file paths.
    pub fn update_autocomplete_matches(&mut self, all_files: &[String]) {
        if !self.autocomplete.active {
            return;
        }
        let query_lower = self.autocomplete.query.to_lowercase();
        self.autocomplete.matches = all_files
            .iter()
            .filter(|f| {
                let f_lower = f.to_lowercase();
                if query_lower.is_empty() {
                    true // Show all when just '@'
                } else {
                    // Match anywhere in the path, or fuzzy on filename
                    f_lower.contains(&query_lower)
                }
            })
            .take(10) // Limit to 10 suggestions
            .cloned()
            .collect();
        if self.autocomplete.selected >= self.autocomplete.matches.len() {
            self.autocomplete.selected = 0;
        }
    }

    /// Accept the currently selected autocomplete match.
    pub fn accept_autocomplete(&mut self) {
        if !self.autocomplete.active || self.autocomplete.matches.is_empty() {
            return;
        }
        let selected = self.autocomplete.selected.min(self.autocomplete.matches.len() - 1);
        let path = self.autocomplete.matches[selected].clone();
        let at_pos = self.autocomplete.at_pos;

        // Replace @query with @path
        let after_cursor = self.input[self.input_cursor..].to_string();
        self.input.truncate(at_pos);
        self.input.push('@');
        self.input.push_str(&path);
        self.input.push(' ');
        self.input_cursor = self.input.len();
        self.input.push_str(&after_cursor);

        self.autocomplete.reset();
    }

    /// Move autocomplete selection up.
    pub fn autocomplete_up(&mut self) {
        if self.autocomplete.active && !self.autocomplete.matches.is_empty() {
            self.autocomplete.selected = self.autocomplete.selected.saturating_sub(1);
        }
    }

    /// Move autocomplete selection down.
    pub fn autocomplete_down(&mut self) {
        if self.autocomplete.active && !self.autocomplete.matches.is_empty() {
            self.autocomplete.selected =
                (self.autocomplete.selected + 1).min(self.autocomplete.matches.len() - 1);
        }
    }

    // ── Attachments ────────────────────────────────────────────

    /// Add a file attachment for the next message.
    pub fn add_attachment(&mut self, path: PathBuf, kind: AttachmentKind) {
        let display_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        // Avoid duplicates by path
        if !self.attachments.iter().any(|a| a.path == path) {
            self.attachments.push(Attachment {
                path,
                display_name,
                kind,
            });
        }
    }

    /// Remove an attachment by display name. Returns true if found.
    pub fn remove_attachment(&mut self, name: &str) -> bool {
        let before = self.attachments.len();
        self.attachments.retain(|a| a.display_name != name);
        self.attachments.len() < before
    }

    /// Take attachments for sending (clears the list).
    pub fn take_attachments(&mut self) -> Vec<Attachment> {
        std::mem::take(&mut self.attachments)
    }

    // ── Message management ─────────────────────────────────────

    pub fn add_user_message(&mut self, text: &str) {
        // Auto-title: set title from first user message (truncated)
        let conv = &mut self.conversations[self.active_conv];
        if conv.title == "New Chat" && conv.messages.is_empty() {
            let title: String = text.chars().take(30).collect();
            conv.title = if text.chars().count() > 30 {
                format!("{}...", title)
            } else {
                title
            };
        }

        self.messages_mut().push(ChatMessage {
            role: ChatRole::User,
            content: text.to_string(),
            tool_calls: Vec::new(),
        });
        self.scroll_to_bottom();
    }

    /// Finalize the current streaming message.
    pub fn finalize_message(&mut self, role: &str, content: &str) {
        let chat_role = match role {
            "user" => ChatRole::User,
            "assistant" => ChatRole::Assistant,
            _ => ChatRole::System,
        };

        // If the last message matches, just update its content
        if let Some(last) = self.messages_mut().last_mut() {
            if last.role == chat_role {
                if !content.is_empty() && last.content.is_empty() {
                    last.content = content.to_string();
                }
                return;
            }
        }

        // Add new message if needed (e.g. system error)
        if chat_role == ChatRole::System && !content.is_empty() {
            self.messages_mut().push(ChatMessage {
                role: chat_role,
                content: content.to_string(),
                tool_calls: Vec::new(),
            });
        }

        self.scroll_to_bottom();
    }

    fn scroll_to_bottom(&mut self) {
        // Will be recalculated during render
        self.scroll_offset = usize::MAX;
        // Exit browse mode so auto-scroll takes precedence during streaming
        self.browse_mode = false;
    }

    // ── Conversation-ID-targeted methods (for parallel streaming) ──

    fn find_conv_idx(&self, conv_id: &str) -> Option<usize> {
        self.conversations.iter().position(|c| c.id == conv_id)
    }

    pub fn append_stream_chunk_to(&mut self, conv_id: &str, text: &str) {
        let Some(idx) = self.find_conv_idx(conv_id) else { return };
        self.conversations[idx].streaming_status = "Writing...".to_string();
        let msgs = &mut self.conversations[idx].messages;
        if let Some(last) = msgs.last_mut() {
            if last.role == ChatRole::Assistant {
                last.content.push_str(text);
                if idx == self.active_conv { self.scroll_to_bottom(); }
                return;
            }
        }
        msgs.push(ChatMessage {
            role: ChatRole::Assistant,
            content: text.to_string(),
            tool_calls: Vec::new(),
        });
        if idx == self.active_conv { self.scroll_to_bottom(); }
    }

    pub fn add_tool_call_to(&mut self, conv_id: &str, tool_name: &str) {
        let Some(idx) = self.find_conv_idx(conv_id) else { return };
        self.conversations[idx].streaming_status = format!("Using {}...", tool_name);
        let msgs = &mut self.conversations[idx].messages;
        if let Some(last) = msgs.last_mut() {
            if last.role == ChatRole::Assistant {
                last.tool_calls.push(tool_name.to_string());
                return;
            }
        }
        msgs.push(ChatMessage {
            role: ChatRole::Assistant,
            content: String::new(),
            tool_calls: vec![tool_name.to_string()],
        });
    }

    /// Append a compact file proposal summary to the assistant's current streaming message.
    /// Displayed as `[wrote path/to/file.rs +N -M]` inline in the chat output.
    pub fn append_deferred_summary(&mut self, conv_id: &str, path: &std::path::Path, additions: usize, deletions: usize) {
        let Some(idx) = self.find_conv_idx(conv_id) else { return };
        let rel = path.file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        let summary = format!("\n[wrote {} +{} -{}]", rel, additions, deletions);
        let msgs = &mut self.conversations[idx].messages;
        if let Some(last) = msgs.last_mut() {
            if last.role == ChatRole::Assistant {
                last.content.push_str(&summary);
                if idx == self.active_conv { self.scroll_to_bottom(); }
                return;
            }
        }
        // No assistant message yet — create one with just the summary
        msgs.push(ChatMessage {
            role: ChatRole::Assistant,
            content: summary,
            tool_calls: Vec::new(),
        });
        if idx == self.active_conv { self.scroll_to_bottom(); }
    }

    pub fn finalize_message_to(&mut self, conv_id: &str, role: &str, content: &str) {
        let Some(idx) = self.find_conv_idx(conv_id) else { return };
        let chat_role = match role {
            "user" => ChatRole::User,
            "assistant" => ChatRole::Assistant,
            _ => ChatRole::System,
        };

        let msgs = &mut self.conversations[idx].messages;
        if let Some(last) = msgs.last_mut() {
            if last.role == chat_role {
                if !content.is_empty() && last.content.is_empty() {
                    last.content = content.to_string();
                }
                self.conversations[idx].is_streaming = false;
                self.conversations[idx].streaming_status.clear();
                self.conversations[idx].streaming_started_at = None;
                if idx == self.active_conv { self.scroll_to_bottom(); }
                return;
            }
        }

        if !content.is_empty() {
            self.conversations[idx].messages.push(ChatMessage {
                role: chat_role,
                content: content.to_string(),
                tool_calls: Vec::new(),
            });
        }

        self.conversations[idx].is_streaming = false;
        self.conversations[idx].streaming_status.clear();
        self.conversations[idx].streaming_started_at = None;
        if idx == self.active_conv { self.scroll_to_bottom(); }
    }

    /// Replace `<file>` blocks in the last assistant message with short summaries.
    pub fn collapse_file_blocks_in(&mut self, conv_id: &str) {
        let Some(idx) = self.find_conv_idx(conv_id) else { return };
        let msgs = &mut self.conversations[idx].messages;
        if let Some(last) = msgs.last_mut() {
            if last.role == ChatRole::Assistant && last.content.contains("<file path=\"") {
                last.content = collapse_file_blocks(&last.content);
            }
        }
    }

    // ── Persistence ─────────────────────────────────────────────

    /// Load all conversations for a workspace from disk.
    pub fn load_conversations(&mut self, workspace_key: &std::path::Path) {
        use gaviero_core::session_state as ss;

        let index = ss::load_conversation_index(workspace_key);

        self.conversations.clear();
        for summary in &index.conversations {
            if let Some(stored) = ss::load_conversation(workspace_key, &summary.id) {
                let messages = stored
                    .messages
                    .into_iter()
                    .map(|m| ChatMessage {
                        role: match m.role.as_str() {
                            "user" => ChatRole::User,
                            "assistant" => ChatRole::Assistant,
                            _ => ChatRole::System,
                        },
                        content: m.content,
                        tool_calls: m.tool_calls,
                    })
                    .collect();
                self.conversations.push(Conversation {
                    id: stored.id,
                    title: stored.title,
                    messages,
                    model_override: stored.model_override,
                    effort_override: stored.effort_override,
                    namespace_override: None,
                    is_streaming: false,
                    streaming_status: String::new(),
                    streaming_started_at: None,
                });
            }
        }

        // Set active conversation
        if let Some(ref active_id) = index.active_id {
            if let Some(idx) = self.conversations.iter().position(|c| c.id == *active_id) {
                self.active_conv = idx;
            }
        }

        // Ensure at least one conversation exists
        if self.conversations.is_empty() {
            self.new_conversation();
        }

        self.scroll_offset = usize::MAX;
        self.history_index = None;
        self.history_stash.clear();
    }

    /// Save all conversations for a workspace to disk.
    pub fn save_conversations(&self, workspace_key: &std::path::Path) {
        use gaviero_core::session_state as ss;

        let mut summaries = Vec::new();
        for conv in &self.conversations {
            let stored = ss::StoredConversation {
                id: conv.id.clone(),
                title: conv.title.clone(),
                messages: conv
                    .messages
                    .iter()
                    .map(|m| ss::StoredMessage {
                        role: match m.role {
                            ChatRole::User => "user".to_string(),
                            ChatRole::Assistant => "assistant".to_string(),
                            ChatRole::System => "system".to_string(),
                        },
                        content: m.content.clone(),
                        tool_calls: m.tool_calls.clone(),
                        timestamp: 0,
                    })
                    .collect(),
                created: 0,
                updated: ss::now_unix(),
                model_override: conv.model_override.clone(),
                effort_override: conv.effort_override.clone(),
            };

            summaries.push(ss::ConversationSummary {
                id: conv.id.clone(),
                title: conv.title.clone(),
                updated: stored.updated,
                message_count: conv.messages.len(),
            });

            if let Err(e) = ss::save_conversation(workspace_key, &stored) {
                tracing::warn!("Failed to save conversation {}: {}", conv.id, e);
            }
        }

        let active_id = self.conversations.get(self.active_conv).map(|c| c.id.clone());
        let index = ss::ConversationIndex {
            conversations: summaries,
            active_id,
        };
        if let Err(e) = ss::save_conversation_index(workspace_key, &index) {
            tracing::warn!("Failed to save conversation index: {}", e);
        }
    }

    // ── Rendering ──────────────────────────────────────────────

    pub fn render(&mut self, area: Rect, buf: &mut RataBuf, focused: bool, theme: &Theme) {
        let border_style = if focused {
            Style::default().fg(theme::ACCENT)
        } else {
            Style::default().fg(theme::TEXT_DIM)
        };

        let block = Block::default()
            .borders(Borders::LEFT)
            .border_style(border_style);
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width < 4 || inner.height < 5 {
            return;
        }

        // Conversation tabs (1 line at the top)
        let tab_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        };
        self.render_conv_tabs(tab_area, buf);

        // Split remaining: conversation area + [attachments] + input area (bottom, 3 lines)
        let input_height: u16 = 3;
        let attach_height: u16 = if self.attachments.is_empty() { 0 } else { 1 };
        let remaining_y = inner.y + 1;
        let remaining_h = inner.height.saturating_sub(1);
        let conv_height = remaining_h.saturating_sub(input_height + 1 + attach_height); // +1 for separator
        let conv_area = Rect {
            x: inner.x,
            y: remaining_y,
            width: inner.width,
            height: conv_height,
        };
        let sep_y = remaining_y + conv_height;
        let attach_y = sep_y + 1;
        let input_area = Rect {
            x: inner.x,
            y: attach_y + attach_height,
            width: inner.width,
            height: input_height,
        };

        // Render conversation
        self.render_conversation(conv_area, buf, theme);

        // Render separator
        let sep_style = Style::default().fg(theme::BORDER_DIM);
        for x in 0..inner.width {
            let cx = inner.x + x;
            if cx < buf.area().right() && sep_y < buf.area().bottom() {
                buf[(cx, sep_y)].set_char('─').set_style(sep_style);
            }
        }

        // Render attachment bar (if any)
        if !self.attachments.is_empty() {
            self.render_attachments(
                Rect {
                    x: inner.x,
                    y: attach_y,
                    width: inner.width,
                    height: attach_height,
                },
                buf,
            );
        }

        // Render input area
        self.render_input(input_area, buf, focused, theme);

        // Render autocomplete popup above the input area
        if self.autocomplete.active && !self.autocomplete.matches.is_empty() {
            let popup_height = self.autocomplete.matches.len().min(8) as u16;
            let popup_y = sep_y.saturating_sub(popup_height);
            let popup_area = Rect {
                x: inner.x,
                y: popup_y,
                width: inner.width.min(50),
                height: popup_height,
            };
            self.render_autocomplete(popup_area, buf);
        }
    }

    fn render_conv_tabs(&self, area: Rect, buf: &mut RataBuf) {
        let bg = theme::TAB_BG;
        let fg_active = theme::TEXT_BRIGHT;
        let fg_inactive = theme::TEXT_DIM;

        // Clear tab bar
        for x in 0..area.width {
            let cx = area.x + x;
            if cx < buf.area().right() && area.y < buf.area().bottom() {
                buf[(cx, area.y)].set_char(' ').set_style(Style::default().bg(bg));
            }
        }

        let mut x = area.x;
        for (i, conv) in self.conversations.iter().enumerate() {
            let is_active = i == self.active_conv;
            let style = if is_active {
                Style::default().fg(fg_active).bg(bg).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(fg_inactive).bg(bg)
            };

            // Truncate title to fit
            let title: String = conv.title.chars().take(15).collect();
            let label = if is_active {
                format!(" [{}] ", title)
            } else {
                format!("  {}  ", title)
            };

            for ch in label.chars() {
                if x < area.x + area.width && x < buf.area().right() {
                    buf[(x, area.y)].set_char(ch).set_style(style);
                    x += 1;
                }
            }

            // Separator
            if x < area.x + area.width && x < buf.area().right() {
                buf[(x, area.y)].set_char('│').set_style(
                    Style::default().fg(theme::BORDER_DIM).bg(bg),
                );
                x += 1;
            }
        }

        // "+" button for new conversation
        if x + 3 < area.x + area.width {
            let plus_style = Style::default().fg(theme::ACCENT).bg(bg);
            buf[(x, area.y)].set_char(' ').set_style(plus_style);
            x += 1;
            buf[(x, area.y)].set_char('+').set_style(plus_style);
        }
    }

    fn render_conversation(&mut self, area: Rect, buf: &mut RataBuf, theme: &Theme) {
        // Cache the conversation area for mouse hit-testing
        self.conv_area_cache = Some(area);

        if area.height == 0 {
            return;
        }

        let width = area.width as usize;
        let browse_bg = theme::BROWSE_BG; // highlight bg for browsed message

        // Build rendered lines from messages: (style, text, message_index)
        let mut lines: Vec<(Style, String, Option<usize>)> = Vec::new();

        for (msg_idx, msg) in self.messages().iter().enumerate() {
            let (prefix, base_style) = match msg.role {
                ChatRole::User => (
                    "You: ",
                    Style::default().fg(theme::ACCENT),
                ),
                ChatRole::Assistant => (
                    "Claude: ",
                    Style::default().fg(theme::TEXT_FG),
                ),
                ChatRole::System => (
                    "System: ",
                    Style::default().fg(theme::WARNING),
                ),
            };

            // Render tool calls
            for tc in &msg.tool_calls {
                let tool_style = Style::default().fg(theme::TOOL_DIM);
                lines.push((tool_style, format!("  [{}...]", tc), Some(msg_idx)));
            }

            // Filter <file> blocks from display (both complete and in-progress)
            let display_content = filter_file_blocks_for_display(&msg.content);

            if msg.role == ChatRole::Assistant && !display_content.is_empty() {
                // Render assistant messages with markdown formatting
                lines.push((base_style, prefix.to_string(), Some(msg_idx)));
                let md_lines = crate::panels::chat_markdown::format_chat_markdown(
                    &display_content,
                    width,
                    base_style,
                );
                for cl in md_lines {
                    lines.push((cl.style, cl.text, Some(msg_idx)));
                }
            } else {
                // User/System: simple word-wrap
                let full_text = format!("{}{}", prefix, display_content);
                for line in crate::widgets::render_utils::word_wrap(&full_text, width) {
                    lines.push((base_style, line, Some(msg_idx)));
                }
            }

            // Blank line between messages
            lines.push((Style::default(), String::new(), None));
        }

        // Streaming indicator with animated spinner
        if self.active_conv_streaming() {
            let spinner_frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            // Advance every ~6 ticks (~200ms at 33ms/tick)
            let frame = spinner_frames[(self.tick_count / 6) as usize % spinner_frames.len()];
            let conv = &self.conversations[self.active_conv];
            let status = &conv.streaming_status;
            let label = if status.is_empty() { "Thinking..." } else { status.as_str() };
            let elapsed_str = conv.streaming_started_at.map(|t| {
                let secs = t.elapsed().as_secs();
                if secs < 60 {
                    format!(" ({}s)", secs)
                } else {
                    format!(" ({}m{}s)", secs / 60, secs % 60)
                }
            }).unwrap_or_default();
            let stream_style = Style::default()
                .fg(theme::ACCENT);
            lines.push((stream_style, format!("{} {}{}", frame, label, elapsed_str), None));
        }

        // Cache rendered line texts for mouse text selection
        self.rendered_lines_cache = lines.iter().map(|(_, text, _)| text.clone()).collect();

        // In browse mode, scroll to keep the browsed message visible
        let total = lines.len();
        let viewport = area.height as usize;
        if self.browse_mode {
            // Find first and last line belonging to the browsed message
            let first_line = lines.iter().position(|(_, _, mi)| *mi == Some(self.browsed_msg));
            let last_line = lines.iter().rposition(|(_, _, mi)| *mi == Some(self.browsed_msg));
            if let (Some(first), Some(last)) = (first_line, last_line) {
                if first < self.scroll_offset {
                    self.scroll_offset = first;
                } else if last >= self.scroll_offset + viewport {
                    self.scroll_offset = last.saturating_sub(viewport - 1);
                }
            }
        } else if self.scroll_offset == usize::MAX || self.scroll_offset + viewport > total {
            // Auto-scroll to bottom
            self.scroll_offset = total.saturating_sub(viewport);
        }

        // Render visible lines
        let default_style = theme.default_style();
        for row in 0..viewport {
            let line_idx = self.scroll_offset + row;
            let y = area.y + row as u16;

            let is_browsed = self.browse_mode
                && line_idx < lines.len()
                && lines[line_idx].2 == Some(self.browsed_msg);

            let row_bg = if is_browsed { browse_bg } else { default_style.bg.unwrap_or(Color::Reset) };

            // Clear row
            let clear_style = if is_browsed {
                Style::default().bg(browse_bg)
            } else {
                default_style
            };
            for col in 0..area.width {
                let cx = area.x + col;
                if cx < buf.area().right() && y < buf.area().bottom() {
                    buf[(cx, y)].set_char(' ').set_style(clear_style);
                }
            }

            if line_idx < lines.len() {
                let (style, ref text, _) = lines[line_idx];
                let line_style = if is_browsed { style.bg(row_bg) } else { style };
                let sel_style = Style::default()
                    .fg(theme::TAB_BG)
                    .bg(theme::ACCENT);
                let mut cx = area.x;
                let mut char_idx = 0usize;
                for ch in text.chars() {
                    let display_ch = if ch == '\t' { ' ' } else { ch };
                    let ch_width = UnicodeWidthChar::width(display_ch).unwrap_or(1) as u16;
                    let final_style = if self.is_char_selected(line_idx, char_idx) {
                        sel_style
                    } else {
                        line_style
                    };
                    if cx + ch_width <= area.x + area.width && cx < buf.area().right() && y < buf.area().bottom() {
                        buf[(cx, y)].set_char(display_ch).set_style(final_style);
                    }
                    cx += ch_width;
                    char_idx += 1;
                }
            }
        }

        // Browse mode hint
        if self.browse_mode {
            let hint = " [BROWSE] ↑↓ nav  Ctrl+C copy  Esc exit ";
            let hint_style = Style::default()
                .fg(theme::TAB_BG)
                .bg(theme::ACCENT);
            let hint_y = area.y;
            let hint_display_w = UnicodeWidthStr::width(hint) as u16;
            let hint_x = area.x + area.width.saturating_sub(hint_display_w);
            let mut cx = hint_x;
            for ch in hint.chars() {
                let ch_w = UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
                if cx + ch_w <= area.x + area.width && cx < buf.area().right() && hint_y < buf.area().bottom() {
                    buf[(cx, hint_y)].set_char(ch).set_style(hint_style);
                }
                cx += ch_w;
            }
        }

        // Scrollbar
        crate::widgets::scrollbar::render_scrollbar(
            area,
            buf,
            total,
            viewport,
            self.scroll_offset,
        );
    }

    fn render_input(&self, area: Rect, buf: &mut RataBuf, focused: bool, _theme: &Theme) {
        let bg = theme::INPUT_BG;
        let fg = theme::TEXT_BRIGHT;
        let style = Style::default().fg(fg).bg(bg);

        // Clear input area
        for row in 0..area.height {
            for col in 0..area.width {
                let cx = area.x + col;
                let cy = area.y + row;
                if cx < buf.area().right() && cy < buf.area().bottom() {
                    buf[(cx, cy)].set_char(' ').set_style(style);
                }
            }
        }

        // Minimal prompt: only show context for special modes
        let prompt: &str = if self.renaming {
            "Rename: "
        } else if self.active_conv_streaming() {
            "Ctrl+C to cancel"
        } else {
            "> "
        };
        let prompt_style = Style::default()
            .fg(theme::ACCENT)
            .bg(bg);

        let mut x = area.x;
        for ch in prompt.chars() {
            if x < area.x + area.width && x < buf.area().right() {
                buf[(x, area.y)].set_char(ch).set_style(prompt_style);
                x += 1;
            }
        }

        // Input text — wraps across available rows
        let prompt_len = (x - area.x) as usize;
        let text_width = (area.width as usize).saturating_sub(prompt_len);
        let total_rows = area.height as usize;

        if self.input.is_empty() && !self.active_conv_streaming() {
            // Hint text
            let hint = "Type a message, Enter to send";
            let hint_style = Style::default()
                .fg(theme::TEXT_DIM)
                .bg(bg);
            for (i, ch) in hint.chars().enumerate() {
                let hx = x + i as u16;
                if hx < area.x + area.width && hx < buf.area().right() {
                    buf[(hx, area.y)].set_char(ch).set_style(hint_style);
                }
            }
            // Show cursor at prompt position even with empty input
            if focused {
                let cursor_style = Style::default().fg(bg).bg(theme::TEXT_FG);
                if x < area.x + area.width && x < buf.area().right() && area.y < buf.area().bottom() {
                    buf[(x, area.y)].set_style(cursor_style);
                }
            }
        } else if text_width > 0 {
            // Wrap input into lines: first line has `text_width` chars,
            // subsequent lines have full `area.width` chars.
            let input_chars: Vec<char> = self.input.chars().collect();
            let cursor_char_pos = self.input[..self.input_cursor].chars().count();

            // Build wrapped lines with (start_char_idx, line_width)
            let mut lines: Vec<(usize, usize)> = Vec::new();
            let mut pos = 0;
            // First line: starts after prompt
            let first_line_width = text_width;
            lines.push((0, first_line_width));
            pos += first_line_width.min(input_chars.len());
            // Subsequent lines: full width
            let full_width = area.width as usize;
            while pos < input_chars.len() {
                lines.push((pos, full_width));
                pos += full_width;
            }

            // Find which line the cursor is on
            let mut cursor_line = 0;
            let mut cursor_col = cursor_char_pos;
            for (i, &(start, width)) in lines.iter().enumerate() {
                if cursor_char_pos < start + width || i == lines.len() - 1 {
                    cursor_line = i;
                    cursor_col = cursor_char_pos.saturating_sub(start);
                    break;
                }
            }

            // Scroll so cursor line is visible
            let scroll = if cursor_line >= total_rows {
                cursor_line - total_rows + 1
            } else {
                0
            };

            // Render visible lines
            for row in 0..total_rows {
                let line_idx = scroll + row;
                if line_idx >= lines.len() {
                    break;
                }
                let (start, width) = lines[line_idx];
                let y = area.y + row as u16;
                let x_start = if line_idx == 0 { x } else { area.x };
                let end = (start + width).min(input_chars.len());

                for (i, &ch) in input_chars[start..end].iter().enumerate() {
                    let cx = x_start + i as u16;
                    if cx < area.x + area.width && cx < buf.area().right() && y < buf.area().bottom() {
                        buf[(cx, y)].set_char(ch).set_style(style);
                    }
                }
            }

            // Position cursor
            if focused && !self.active_conv_streaming() {
                let visible_cursor_line = cursor_line.saturating_sub(scroll);
                if visible_cursor_line < total_rows {
                    let y = area.y + visible_cursor_line as u16;
                    let x_start = if cursor_line == 0 { x } else { area.x };
                    let cursor_x = x_start + cursor_col as u16;
                    if cursor_x < area.x + area.width
                        && cursor_x < buf.area().right()
                        && y < buf.area().bottom()
                    {
                        let cursor_style = Style::default().fg(bg).bg(theme::TEXT_FG);
                        buf[(cursor_x, y)].set_style(cursor_style);
                    }
                }
            }
        }
    }

    fn render_autocomplete(&self, area: Rect, buf: &mut RataBuf) {
        let bg = theme::INPUT_BG;
        let fg = theme::TEXT_FG;
        let selected_bg = theme::SELECTION_BG;

        for (row, (i, path)) in self
            .autocomplete
            .matches
            .iter()
            .enumerate()
            .take(area.height as usize)
            .enumerate()
        {
            let y = area.y + row as u16;
            let is_selected = i == self.autocomplete.selected;
            let style = if is_selected {
                Style::default().fg(Color::White).bg(selected_bg)
            } else {
                Style::default().fg(fg).bg(bg)
            };

            // Clear row
            for col in 0..area.width {
                let cx = area.x + col;
                if cx < buf.area().right() && y < buf.area().bottom() {
                    buf[(cx, y)].set_char(' ').set_style(style);
                }
            }

            // Write path with @ prefix indicator
            let display = format!(" @{}", path);
            for (ci, ch) in display.chars().enumerate() {
                let cx = area.x + ci as u16;
                if cx < area.x + area.width && cx < buf.area().right() && y < buf.area().bottom() {
                    buf[(cx, y)].set_char(ch).set_style(style);
                }
            }
        }
    }

    /// Render attachment badges in a single-line bar.
    fn render_attachments(&self, area: Rect, buf: &mut RataBuf) {
        let bg = theme::INPUT_BG;
        let badge_fg = theme::TEXT_BRIGHT;
        let badge_bg = theme::BADGE_BG;
        let img_badge_bg = theme::IMAGE_BADGE_BG;
        let label_style = Style::default().fg(theme::TEXT_DIM).bg(bg);

        let y = area.y;
        if y >= buf.area().bottom() {
            return;
        }

        // Clear row
        for col in 0..area.width {
            let cx = area.x + col;
            if cx < buf.area().right() {
                buf[(cx, y)].set_char(' ').set_style(Style::default().bg(bg));
            }
        }

        let mut x = area.x;

        // Label
        let label = " Attached: ";
        for ch in label.chars() {
            if x < area.x + area.width && x < buf.area().right() {
                buf[(x, y)].set_char(ch).set_style(label_style);
                x += 1;
            }
        }

        // Badges
        for attach in &self.attachments {
            let this_bg = if attach.kind == AttachmentKind::Image {
                img_badge_bg
            } else {
                badge_bg
            };
            let style = Style::default().fg(badge_fg).bg(this_bg);

            // " name.ext " badge
            let badge = format!(" {} ", attach.display_name);
            for ch in badge.chars() {
                if x < area.x + area.width && x < buf.area().right() {
                    buf[(x, y)].set_char(ch).set_style(style);
                    x += 1;
                }
            }

            // Gap between badges
            if x < area.x + area.width && x < buf.area().right() {
                buf[(x, y)].set_char(' ').set_style(Style::default().bg(bg));
                x += 1;
            }
        }
    }
}

/// Parse `@path/to/file` references from input text.
/// Returns a list of relative file paths referenced.
pub fn parse_file_references(text: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'@' {
            // Check that @ is at start or preceded by whitespace
            if i > 0 && bytes[i - 1] != b' ' && bytes[i - 1] != b'\n' && bytes[i - 1] != b'\t' {
                i += 1;
                continue;
            }
            // Collect the reference (non-whitespace chars after @)
            let start = i + 1;
            let mut end = start;
            while end < len && bytes[end] != b' ' && bytes[end] != b'\n' && bytes[end] != b'\t' {
                end += 1;
            }
            if end > start {
                let path = &text[start..end];
                refs.push(path.to_string());
            }
            i = end;
        } else {
            i += 1;
        }
    }

    refs
}

/// Filter `<file>` blocks for display: collapse complete blocks and hide in-progress ones.
fn filter_file_blocks_for_display(text: &str) -> String {
    use crate::app::collapse_file_blocks;

    // First collapse any complete <file ...>...</file> blocks
    let collapsed = collapse_file_blocks(text);

    // Then handle any in-progress (unclosed) <file block from streaming
    if let Some(tag_start) = collapsed.rfind("<file path=\"") {
        // Check if there's a closing </file> after this opening tag
        if collapsed[tag_start..].find("</file>").is_none() {
            // In-progress block — extract path if available and truncate
            let after_attr = tag_start + "<file path=\"".len();
            let label = if let Some(quote_end) = collapsed[after_attr..].find('"') {
                let path = &collapsed[after_attr..after_attr + quote_end];
                format!("[writing {}...]", path)
            } else {
                "[writing file...]".to_string()
            };
            let mut result = collapsed[..tag_start].to_string();
            result.push_str(&label);
            return result;
        }
    }

    collapsed
}

