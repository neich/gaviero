# gaviero-tui — Architecture

Full-screen terminal editor. Rendering + event routing only. All logic delegates to `gaviero-core`.

---

## 1. Module Layout

```
gaviero-tui/src/
├─ main.rs                   Entry point, terminal setup, panic handler, main event loop
├─ app.rs                    App struct (state + layout + focus), ~5000 lines
├─ event.rs                  Event enum (43+ variants), EventLoop sources
├─ keymap.rs                 Action enum, keybinding definitions, chord system
├─ theme.rs                  One Dark colors (~80 constants), timing constants
├─ editor/
│  ├─ mod.rs                 Editor (buffer + view + state)
│  ├─ buffer.rs              Ropey text buffer, cursor, selection, undo/redo, find
│  ├─ view.rs                EditorView (viewport, gutter, syntax highlight, cursor)
│  ├─ diff_overlay.rs        DiffSource, DiffReviewState, hunk accept/reject
│  ├─ highlight.rs           Tree-sitter query runner → Vec<StyledSpan>
│  └─ markdown.rs            Markdown rendering in editor
├─ panels/
│  ├─ mod.rs                 Panel enum, layout calculations
│  ├─ file_tree.rs           Multi-root file browser, git decorations, proposals
│  ├─ agent_chat.rs          AgentChatState, conversation history, @file autocomplete
│  ├─ chat_markdown.rs       ChatLine: markdown rendering for chat
│  ├─ swarm_dashboard.rs     Agent status table, tier/phase labels
│  ├─ git_panel.rs           GitPanelState, staging, commit, branch picker
│  ├─ terminal.rs            Terminal rendering (tui-term), TerminalSelectionState
│  ├─ status_bar.rs          Mode, file, branch, agent status indicators
│  └─ search.rs              SearchPanelState, input + live results
├─ widgets/
│  ├─ mod.rs                 Widget trait, layout calculations
│  ├─ tabs.rs                TabBar, tab close indicators
│  ├─ scrollbar.rs           Custom scrollbar widget
│  ├─ scroll_state.rs        ScrollState: offset + selection with viewport caching
│  ├─ text_input.rs          TextInput: buffer with cursor, selection, undo/redo
│  └─ render_utils.rs        Shared rendering helpers
└─ app/
   ├─ controller.rs          Top-level event handling + action dispatch
   ├─ layout.rs              Layout computation (5-area split)
   ├─ render.rs              Draw orchestration
   ├─ left_panel.rs          Left panel (file tree, search, changes, review) state
   ├─ review.rs              Review UI, diff acceptance flows
   ├─ side_panel.rs          Side panel (chat, swarm, git) behavior
   ├─ commands.rs            Slash-command handlers (/run, /swarm, /cswarm, /remember)
   ├─ editing.rs             Editor + find-bar interactions
   ├─ session.rs             Session restore/save integration
   ├─ state.rs               Enums + structs shared by controllers
   └─ observers.rs           Observer trait implementations (bridges to Event)
```

---

## 2. Core Architecture

### Single Event Channel

All external events flow through one `mpsc::UnboundedChannel<Event>`:

```
Crossterm reader ──┐
File watcher ──────┤
Tick timer ────────┤
Terminal bridge ───┼─→ mpsc::unbounded_channel ──→ App::handle_event()
WriteGateObserver ─┤                                     │
AcpObserver ───────┤                                     ▼
SwarmObserver ─────┘                             App::render() & redraw
```

**Golden rule:** No background task mutates `App` directly.

### Main Loop

```rust
loop {
    // Draw current state
    terminal.draw(|frame| {
        app.render(frame);
    })?;
    
    // Receive ONE event
    match event_rx.recv().await {
        Some(event) => app.handle_event(event),
        None => break,  // Channel closed
    }
    
    // Drain up to 64 pending events before redraw
    for _ in 0..64 {
        match event_rx.try_recv() {
            Ok(event) => app.handle_event(event),
            Err(e) if e.is_empty() => break,  // No more pending
            Err(_) => break,  // Channel closed
        }
    }
    
    if app.should_quit { break; }
}
```

### Focus Model

```rust
pub enum Focus {
    Editor,
    FileTree,
    SidePanel,
    Terminal,
}
```

Switching:
- `Alt+1` → Editor
- `Alt+2` → FileTree
- `Alt+3` → SidePanel
- `Alt+4` → Terminal

---

## 3. Layout (5-area split)

```
┌──────────────────────────────────────────────┐
│            Tab Bar                           │
├──────┬───────────────────────┬───────────────┤
│      │                       │               │
│ Left │      Editor           │ Side Panel    │
│Panel │     (center,          │ (Chat/Swarm/ │
│      │     largest)          │  Git)        │
│      │                       │               │
│      │                       │               │
├──────┴───────────────────────┴───────────────┤
│           Terminal (embedded)                 │
├────────────────────────────────────────────────┤
│          Status Bar                            │
└────────────────────────────────────────────────┘
```

**Computed in `app/layout.rs`:**

```rust
pub fn compute_layout(
    full_area: Rect,
    left_visible: bool,
    side_visible: bool,
    terminal_visible: bool,
    split_ratios: (u16, u16, u16),  // [left, editor, side]
) -> LayoutAreas {
    // Returns: { tab_bar, left, editor, side, terminal, status_bar }
}
```

**Fullscreen mode:** single focused area, hides others.

---

## 4. Panel State Management

### Left Panel Modes

```rust
pub enum LeftPanelMode {
    FileTree { state: FileTreeState },
    Search { state: SearchState },
    Changes { state: ChangesState },
    Review { state: ReviewState },
}
```

Transitions:
- Default: FileTree
- `Ctrl+/` → Search
- Agent writes pending → Changes (auto-switch)
- Proposal created → Review (auto-switch)

### Side Panel Modes

```rust
pub enum SidePanelMode {
    AgentChat { state: AgentChatState },
    SwarmDashboard { state: SwarmDashboardState },
    GitPanel { state: GitPanelState },
}
```

Switching:
- `Alt+A` → AgentChat
- `Alt+W` → SwarmDashboard
- `Alt+G` → GitPanel

---

## 5. Event System

### Event Enum (43+ variants)

```rust
pub enum Event {
    // Input
    Key(KeyEvent),
    Mouse(MouseEvent),
    Paste(String),
    Resize(u16, u16),
    
    // File watcher
    FileChanged(PathBuf),
    FileTreeChanged,
    
    // Tick (30 fps)
    Tick,
    
    // Terminal bridge
    Terminal(TerminalEvent),
    TerminalExited(u32),
    
    // WriteGateObserver
    ProposalCreated(WriteProposal),
    ProposalUpdated(u64, ProposalStatus),
    ProposalFinalized(u64, ProposalStatus),
    
    // AcpObserver
    StreamChunk(String),
    ToolCallStarted(String, String),  // tool_name, tool_use_id
    StreamingStatus(String),
    MessageComplete(MessageStats),
    FileProposalDeferred(String),  // file_path
    AcpTaskCompleted(Result<()>),
    
    // SwarmObserver
    SwarmPhaseChanged(String),
    AgentStateChanged(String, AgentStatus),  // unit_id, status
    TierStarted(usize, Vec<WorkUnit>),
    MergeConflict(String, MergeConflict),
    SwarmCompleted(SwarmResult),
    TierDispatch(usize, Vec<String>),  // tier, unit_ids
    CoordinationStarted,
    CoordinationComplete(CompiledPlan),
    DslPlanReady(CompiledPlan),
    CostUpdate(f64),
    
    // Memory
    MemoryReady,
}
```

### Event Sources & Threading

| Source | Thread | Producer |
|---|---|---|
| Crossterm | Dedicated thread | `crossterm::terminal::enable_raw_mode()` reader |
| File watcher | tokio task | `notify` crate callback |
| Tick | tokio task | `tokio::time::interval(33ms)` |
| Terminal bridge | dedicated | TerminalManager internal |
| Observers | Core tasks | Observer trait implementations |

All → `mpsc::unbounded_channel`.

---

## 6. Observer Bridge

Core callbacks become Event variants via trait implementations.

### WriteGateObserver Implementation

```rust
pub struct TuiWriteGateObserver {
    event_tx: mpsc::UnboundedSender<Event>,
}

impl WriteGateObserver for TuiWriteGateObserver {
    fn on_proposal_created(&self, proposal: &WriteProposal) {
        let _ = self.event_tx.send(Event::ProposalCreated(proposal.clone()));
    }
    // ... on_proposal_updated, on_proposal_finalized
}
```

Same pattern for `AcpObserver` and `SwarmObserver`.

---

## 7. Editor: Buffer & View

### Buffer (editor/buffer.rs)

```rust
pub struct EditorBuffer {
    rope: ropey::Rope,           // Efficient text structure
    cursor: Cursor,              // (line, col)
    selection: Option<Range>,    // Visual selection
    undo_stack: Vec<Transaction>,
    redo_stack: Vec<Transaction>,
}

impl EditorBuffer {
    pub fn insert_char(&mut self, ch: char);
    pub fn delete_char(&mut self);
    pub fn find_next_match(&self, pattern: &str) -> Option<Range>;
    pub fn find_prev_match(&self, pattern: &str) -> Option<Range>;
}
```

### View (editor/view.rs)

```rust
pub struct EditorView {
    viewport: Rect,              // Visible area
    scroll_row: u16,             // Top visible row
    scroll_col: u16,             // Left visible col
    syntax_cache: HashMap<u64, Vec<StyledSpan>>, // Line hash → highlights
}

impl EditorView {
    pub fn render(&self, frame: &mut Frame, buffer: &EditorBuffer);
    // Renders: gutter, syntax highlights, cursor, diff overlay
}
```

### Syntax Highlighting (editor/highlight.rs)

Tree-sitter query runner (from `queries/{lang}/highlights.scm`):

```rust
pub fn highlight_line(
    tree: &Tree,
    language: &Language,
    line_range: Range,
) -> Vec<StyledSpan>
```

**Query processing:**
1. Run tree-sitter highlight query against visible buffer range
2. Cache results by line hash
3. Update on edit
4. Only visible viewport highlighted (performance)

---

## 8. Keybinding System

### Action Enum (keymap.rs)

```rust
pub enum Action {
    // Editor
    CharInsert(char),
    Delete,
    Undo,
    Redo,
    Find,
    FindNext,
    FindPrev,
    
    // Navigation
    MoveCursorUp,
    MoveCursorDown,
    MoveCursorLeft,
    MoveCursorRight,
    GoToLine(u32),
    GoToEof,
    PageUp,
    PageDown,
    
    // Selection
    SelectAll,
    SelectLine,
    SelectWord,
    
    // File/Workspace
    NewFile,
    OpenFile,
    SaveFile,
    CloseTab,
    NextTab,
    PrevTab,
    
    // Chat/Swarm
    SendMessage,
    RunTask,
    RunSwarm,
    RunCswarm,
    RememberToMemory,
    AttachFile,
    
    // Review
    AcceptProposal,
    RejectProposal,
    AcceptNode,
    FinalizeReview,
    
    // Focus/UI
    FocusEditor,
    FocusFileTree,
    FocusSidePanel,
    FocusTerminal,
    ToggleSidePanel,
    ToggleTerminal,
    Fullscreen,
    
    // System
    Quit,
}
```

### Keybinding Rules (keymap.rs)

```
Ctrl = Editor (text operations)
  Ctrl+C → Copy
  Ctrl+V → Paste
  Ctrl+Z → Undo
  Ctrl+Y → Redo
  Ctrl+A → SelectAll
  Ctrl+/ → Find
  Ctrl+S → SaveFile

Alt = Workspace (navigation + commands)
  Alt+1 → FocusEditor
  Alt+2 → FocusFileTree
  Alt+3 → FocusSidePanel
  Alt+4 → FocusTerminal
  Alt+N → NewFile
  Alt+O → OpenFile
  Alt+W → SwarmDashboard
  Alt+A → AgentChat
  Alt+G → GitPanel
  Alt+Q → Quit

Shift = Selection (extends with Alt or Ctrl)
  Shift+Up → SelectUp
  Shift+Down → SelectDown
  Shift+Left → SelectLeft
  Shift+Right → SelectRight

Special: Diff review overlay
  ]h / [h → navigate hunks
  a / r → accept/reject hunk
  A / R → accept/reject all hunks
  f → finalize review
  q → exit review
```

---

## 9. Diff Review Flow

### DiffOverlay (editor/diff_overlay.rs)

When proposal created:

```
1. on_proposal_created(proposal)
   ├─ Event::ProposalCreated → event channel
   
2. App::handle_event(ProposalCreated)
   ├─ Switch LeftPanelMode → Review
   ├─ Store proposal in ReviewState
   ├─ DiffOverlay initialized
   
3. Render loop shows diff hunks
   ├─ Original (left) vs. Proposed (right)
   ├─ Color: ─ (removed), + (added), unchanged
   ├─ Cursor highlights current hunk
   
4. User presses 'a' (accept) on hunk
   ├─ App::handle_event(Key('a'))
   ├─ write_gate.accept_hunk(proposal_id, hunk_index)
   ├─ Redraw shows update
   
5. All hunks reviewed
   ├─ User presses 'f' (finalize)
   ├─ write_gate.finalize_proposal(proposal_id)
   ├─ Disk write happens in core
   ├─ Event::ProposalFinalized → UI cleared
```

---

## 10. Commands

Slash commands in agent chat input trigger operations in `app/commands.rs`.

### Supported Commands

| Command | Handler | Effect |
|---|---|---|
| `/run <file.gaviero>` | `handle_run()` | Compile DSL, execute swarm pipeline |
| `/swarm <text>` | `handle_swarm()` | Natural-language swarm planning |
| `/cswarm <text>` | `handle_cswarm()` | Coordinated planning, generates .gaviero |
| `/remember <text>` | `handle_remember()` | Store text in memory |
| `/attach <path>` | `handle_attach()` | Attach file to chat |
| `/detach <path>` | `handle_detach()` | Remove attached file |
| `/set-model <spec>` | `handle_set_model()` | Override model for next message |
| `/clear` | `handle_clear()` | Clear conversation history |

### Execution Pattern

```rust
pub async fn handle_run(
    app: &mut App,
    file_path: &str,
) {
    // 1. Compile via gaviero_dsl::compile()
    match gaviero_dsl::compile(&source, &file_path, None, None) {
        Ok(plan) => {
            // 2. Execute via swarm::pipeline::execute()
            // 3. SwarmObserver events → Event → app.handle_event()
            // 4. Swarm dashboard updates in side panel
        }
        Err(e) => {
            // 5. Display error in chat panel
        }
    }
}
```

---

## 11. Session Persistence

`app/session.rs` integrates with `gaviero-core::session_state`.

### SessionState

```rust
pub struct SessionState {
    pub workspace_path: PathBuf,
    pub open_tabs: Vec<TabState>,      // file_path, cursor, selection
    pub current_tab_index: usize,
    pub left_panel_mode: LeftPanelMode,
    pub side_panel_mode: SidePanelMode,
    pub terminal_visible: bool,
    pub conversations: Vec<StoredConversation>,  // chat history
}

pub struct TabState {
    pub file_path: PathBuf,
    pub cursor: (u32, u32),
    pub scroll_row: u16,
}
```

### Lifecycle

```
TUI startup:
  ├─ SessionState::load_or_default()
  ├─ Restore tabs, cursor positions, panel modes
  └─ Populate app state

TUI runtime:
  ├─ ON_FILE_CHANGE or ON_TIMER
  └─ SessionState::save(app) → serialized to disk

TUI shutdown:
  └─ SessionState::save(app) → finalize
```

---

## 12. Concurrency Model

Single-threaded UI + background async tasks.

### Channels

```
Event Channel
├─ Receiver: main loop (single)
├─ Senders: crossterm reader, file watcher, tick timer, observer impls
└─ Rule: all mutations happen in main loop, after event received

Observer Arc
├─ ClaudeCodeBackend spawns sessions
├─ Sessions send events to observer
└─ Observer sends to event channel
```

**No Mutex in TUI.**

---

## 13. Error Handling

All errors displayed in a status/alert area:

```
User action fails
  ├─ Log to stderr
  ├─ Create Alert event
  └─ Render in status bar or pop-up

Chat/Swarm error
  ├─ Caught by observer
  ├─ Event::AcpTaskCompleted(Err) or SwarmCompleted(failed_result)
  └─ Display in side panel
```

No panics in production (recovery attempt).

---

## 14. Dependencies

- **ratatui 0.30:** terminal rendering
- **crossterm 0.29:** terminal I/O
- **ropey:** rope-based text buffer
- **notify:** filesystem watcher
- **portable-pty + vt100 + tui-term:** embedded terminal
- **arboard:** clipboard
- **gaviero-core, gaviero-dsl:** runtime + compilation

---

See [CLAUDE.md](CLAUDE.md) for build, test, conventions, keybinding details.
