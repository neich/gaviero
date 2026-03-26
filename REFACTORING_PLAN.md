# Refactoring Plan — Gaviero Stabilization

> Generated: 2026-03-25 | Status: **Phase 1 complete (audit)**

---

## 1. Panel Unification (PRIMARY FOCUS)

### 1a. Feature Matrix

| Feature               | FileTree       | AgentChat          | GitPanel           | SwarmDashboard     | Terminal        | Search         | StatusBar   |
|-----------------------|----------------|--------------------|--------------------|--------------------|-----------------|----------------|-------------|
| Scroll offset         | `scroll_offset: usize` | `scroll_offset: usize` + sentinel `MAX` | ✗ (fixed layout) | `scroll_offset` + `detail_scroll` | ✗ (vt100)    | `scroll_offset: usize` | ✗         |
| Selection model       | Single `selected: usize` | Multi: input cursor + browse msg + text sel | Per-region: `unstaged_selected`, `staged_selected` | Single `selected: usize` | ✗ | Single `selected: usize` | ✗ |
| Keyboard nav (↑/↓)   | ✓ saturating   | ✓ multi-line input + browse | ✓ per-region dispatch | ✗ (no nav)       | ✗ (passthrough) | ✓ saturating   | ✗           |
| Mouse handling        | Click→row      | Text selection + tab click | ✗                  | ✗                  | Text selection  | ✗              | ✗           |
| Focus enter/leave     | Border color   | Border color + cursor visibility | Border color     | Border color       | Border color    | Border color   | N/A         |
| Resize behavior       | Via `Rect` arg | Via `Rect` arg     | Via `Rect` arg     | Via `Rect` arg     | Via `Rect` arg  | Via `Rect` arg | Via `Rect`  |
| Render boilerplate    | Right border   | Full border        | No border          | No border          | Top border line | No border      | No border   |
| Content filtering     | Glob exclude + git allowlist | `<file>` block filter | Branch filter | ✗              | ✗               | Query match    | ✗           |
| Scrollbar             | ✓ delegated    | ✓ delegated        | ✗                  | ✓ delegated        | ✗               | ✓ delegated    | ✗           |
| Empty state           | ✓              | ✓                  | ✓                  | ✓ (spinner)        | ✗               | ✓              | ✗           |

### 1b. Shared Behavior — Duplicated Code

**Pattern 1: Scroll-into-view logic** (HIGH duplication)
- `file_tree.rs:270-280` — `ensure_selected_visible(viewport)`
- `search.rs:130-134` — `ensure_visible()` (near-identical)
- `agent_chat.rs:1903-1916` — complex variant with browse mode + auto-scroll
- `swarm_dashboard.rs:542-547` — detail pane auto-scroll

All implement the same core algorithm:
```
if selected < scroll_offset → scroll_offset = selected
if selected >= scroll_offset + viewport → scroll_offset = selected - viewport + 1
```

**Pattern 2: Move up/down selection** (HIGH duplication)
- `file_tree.rs:179-188` — `move_up/down` with saturating bounds
- `search.rs:108-120` — identical + ensure_visible call
- `git_panel.rs:117-144` — per-region dispatch variant
- `swarm_dashboard.rs` — implicit via app.rs handler

**Pattern 3: Viewport iteration** (MEDIUM duplication)
- `file_tree.rs:334-336` — `.skip(scroll_offset).take(viewport)`
- `search.rs:188-196` — `for row in 0..viewport { idx = scroll_offset + row }`
- `swarm_dashboard.rs:417-425` — same loop pattern
- `agent_chat.rs:1920-1922` — same pattern

**Pattern 4: Line clearing / background fill** (MEDIUM duplication)
- 50+ instances across panels of `buf[(x, y)].set_char(' ').set_style(bg)`
- Already partially addressed by `render_utils::fill_row()` but panels don't use it

**Pattern 5: Text input editing** (MEDIUM duplication)
- `agent_chat.rs:666-999` — full input: char-indexed cursor, undo/redo, word movement, selection
- `git_panel.rs:148-183` — simplified input: byte-indexed cursor, no undo, no selection
- Two independent implementations of the same concept at different fidelity

### 1c. Render Signature Inconsistency

| Panel            | Signature                                                              |
|------------------|------------------------------------------------------------------------|
| `agent_chat`     | `fn render(&mut self, area: Rect, buf: &mut RataBuf, focused: bool, theme: &Theme)` |
| `git_panel`      | `fn render(&self, area: Rect, buf: &mut RataBuf, focused: bool, _theme: &Theme)` |
| `swarm_dashboard`| `fn render(&mut self, area: Rect, buf: &mut Buffer, focused: bool)`    |
| `file_tree`      | `fn render(&mut self, area: Rect, buf: &mut Buffer, focused: bool)`    |
| `search`         | `fn render(&self, area: Rect, buf: &mut Buffer, focused: bool)`        |
| `status_bar`     | `fn render(&self, area: Rect, buf: &mut Buffer)`                       |

Issues: mix of `Buffer`/`RataBuf`, inconsistent `theme` parameter, mix of `&self`/`&mut self`.

### 1d. Proposed Unification Strategy

**Recommended approach: Shared structs (composition) + utility functions.**

A full `PanelTrait` is premature — panels are too heterogeneous (terminal is pure render, agent_chat has multi-region input, status_bar is stateless). Instead:

**Step 1: Extract `ScrollState` struct** into `widgets/scroll_state.rs`:
```rust
pub struct ScrollState {
    pub offset: usize,
    pub selected: usize,
}

impl ScrollState {
    pub fn move_up(&mut self) { self.selected = self.selected.saturating_sub(1); }
    pub fn move_down(&mut self, item_count: usize) {
        if self.selected < item_count.saturating_sub(1) { self.selected += 1; }
    }
    pub fn ensure_visible(&mut self, viewport: usize) { /* clamp logic */ }
    pub fn page_up(&mut self, viewport: usize) { /* ... */ }
    pub fn page_down(&mut self, item_count: usize, viewport: usize) { /* ... */ }
    pub fn visible_range(&self, viewport: usize) -> Range<usize> { /* ... */ }
}
```

**Step 2: Extract `TextInput` struct** into `widgets/text_input.rs`:
```rust
pub struct TextInput {
    pub text: String,
    cursor: usize,          // char index
    sel_anchor: Option<usize>,
    undo_stack: Vec<(String, usize)>,
    redo_stack: Vec<(String, usize)>,
}
// Consolidates agent_chat input + git_panel commit input
```

**Step 3: Standardize render signatures** — all panels get:
```rust
fn render(&mut self, area: Rect, buf: &mut Buffer, focused: bool)
```
Theme accessed via global/static rather than parameter (it never changes at runtime).

**Step 4: Ensure panels use `render_utils`** — replace inline buffer manipulation with existing `write_text()`, `fill_row()` calls.

### 1e. Migration Path

| Order | Panel            | Complexity | Rationale |
|-------|------------------|------------|-----------|
| 1     | **search**       | Low        | Simplest list panel. Clean scroll + selection. Ideal proof of concept for `ScrollState`. |
| 2     | **file_tree**    | Low-Med    | Tree model adds expand/collapse, but scroll/selection is standard. |
| 3     | **swarm_dashboard** | Medium  | Two scroll regions (table + detail). Good test of composing two `ScrollState` instances. |
| 4     | **git_panel**    | Medium     | Multi-region selection + text input. Good test of `TextInput` extraction. |
| 5     | **agent_chat**   | High       | Most complex: multi-line input, text selection, browse mode, auto-scroll. Migrate last. |
| —     | **terminal**     | Skip       | Pure rendering module, no scroll/selection state to unify. |
| —     | **status_bar**   | Skip       | Stateless renderer, no shared patterns applicable. |

---

## 2. God Modules

### 2a. `app.rs` — 5406 lines

**Current structure:** Single file with 1 massive `App` impl block (~100 methods) mixing:
- Layout calculation and rendering dispatch
- Event routing and action handling (19 `handle_*` methods)
- Business logic (swarm commands, memory, agent attachment)
- State management (focus cycling, panel toggling, session save/restore)
- Two full "inline panels" (BatchReview, Changes) defined and rendered entirely in app.rs

**Panel state that belongs in panel structs (currently in App):**
- `BatchReviewState` (lines 62-82) with proposals, selection, scroll, cached diff
- `ChangesState` (lines 134-152) with entries, selection, scroll, cached diff
- `TreeDialog` (lines 327-424) with file/folder creation dialog state
- Layout state: `PanelVisibility`, `LayoutAreas`, widths, presets (lines 163-197)
- Mouse state: `mouse_dragging`, `scrollbar_dragging`, `last_click` (lines 449-460)

**Exact duplicates within app.rs:**
- `render_review_file_list()` (2662-2753) ≈ `render_changes_file_list()` (2981-3089) — ~90% identical
- `render_batch_review_diff()` (2756-2860) ≈ `render_changes_diff()` (3096-3202) — ~90% identical
- `build_simple_diff()` (lines 92-128) — LCS algorithm used by both

**Mutation during render (anti-pattern):**
- `render_review_file_list()` mutates `scroll_offset` during rendering (line 2687-2693)
- `render_changes_file_list()` same pattern (line 3021-3027)

**Proposed decomposition:**

| New module | Responsibility | Est. lines | Source lines |
|------------|---------------|------------|--------------|
| `app.rs` (retained) | App struct, new(), session restore/save, top-level dispatch | ~500 | — |
| `app_input.rs` | `handle_event`, `handle_action` dispatch, focus routing | ~350 | 626-1157 |
| `app_render.rs` | `render()` orchestration, layout calculation, render dispatch | ~250 | 4417-4568 |
| `layout.rs` | `PanelVisibility`, `LayoutPreset`, `LayoutAreas`, width calculation | ~200 | 163-312 |
| `panels/review.rs` | `BatchReviewState`, render, input handling | ~350 | 62-128, 2314-2860 |
| `panels/changes.rs` | `ChangesEntry`, `ChangesState`, render, input handling | ~300 | 134-152, 2926-3202 |
| `ui/clipboard.rs` | Clipboard abstraction with system + internal fallback | ~80 | 459-460, 3283-3312 |
| `ui/diff_utils.rs` | `build_simple_diff()`, diff rendering shared by review + changes | ~150 | 85-128 |
| `ui/tree_dialog.rs` | TreeDialog struct, input handling, file/folder creation | ~250 | 327-551 |
| `ui/mouse.rs` | `handle_mouse`, scrollbar drag, hit-testing | ~450 | 3573-4005 |

**Risk: MEDIUM-HIGH.** Requires careful re-wiring of `&mut self` access patterns. Each module needs access to specific App fields.

---

## 3. Duplicated Logic (Single Source of Truth Violations)

### 3a. Language extension mapping — LOW risk
- **Files:** `gaviero-core/src/tree_sitter.rs:4-23` (`language_for_extension`) and `:26-46` (`language_name_for_extension`)
- **Problem:** Two parallel match statements mapping extensions to languages. Markdown appears only in `language_name_for_extension`.
- **Fix:** Single `LANGUAGE_REGISTRY` array of `(ext, name, Option<Language>)` tuples. Both functions query it.
- **Called from:** 8 files across core and TUI crates.

### 3b. Inline file list rendering — LOW risk
- **Files:** `app.rs:2662-2753` vs `app.rs:2981-3089`
- **Problem:** `render_review_file_list` and `render_changes_file_list` are ~90% identical.
- **Fix:** Extract `render_file_list_panel()` generic over entry type.

### 3c. Inline diff rendering — LOW risk
- **Files:** `app.rs:2756-2860` vs `app.rs:3096-3202`
- **Problem:** `render_batch_review_diff` and `render_changes_diff` are ~90% identical.
- **Fix:** Extract `render_diff_panel()` generic over data source.

### 3d. Hardcoded colors outside theme — LOW risk
- **Files:** `swarm_dashboard.rs` (9 hardcoded RGB values at lines 279, 449-452, 564, 571), `search.rs:140`, `terminal.rs:225`, `file_tree.rs:350`
- **Problem:** Colors not centralized in `theme.rs`.
- **Fix:** Add constants to `theme.rs` (e.g., `TIER_COORDINATOR`, `TIER_REASONING`, `ACTIVITY_TOOL_CALL`, etc.).

### 3e. Buffer manipulation not using render_utils — LOW risk
- **Files:** All panels except status_bar
- **Problem:** `render_utils.rs` provides `write_text()`, `fill_row()`, `word_wrap()` but panels inline equivalent logic.
- **Fix:** Replace inline buffer writes with `render_utils` calls.

---

## 4. Pattern Inconsistencies

### 4a. Dangerous `.unwrap()` calls — MEDIUM risk
- **`app.rs`** — 11 unwraps, including `terminal_manager.active_instance_mut().unwrap()` (line 641) and `take().unwrap()` on optional review state (lines 2332, 2368)
- **`editor/highlight.rs`** — 6 unwraps on `parser.set_language()` and `parse()` results (lines 131-198)
- **`editor/buffer.rs`** — 3 unwraps on newline searches
- **Fix:** Replace with `anyhow::Result` propagation or `.ok()?` / `if let` patterns per ARCHITECTURE.md constraint #10.

### 4b. Char vs. byte cursor indexing — LOW risk
- **`agent_chat.rs`** — correctly uses char indices (line 652-657)
- **`git_panel.rs`** — uses byte indices for commit input cursor (line 148-183)
- **Fix:** Standardize on char indices (or extract `TextInput` struct that handles it correctly).

### 4c. `usize::MAX` scroll sentinel — LOW risk
- **`agent_chat.rs:1430`** — sets `scroll_offset = usize::MAX` to mean "scroll to bottom"
- **All other panels** — use explicit calculation
- **Fix:** Use `Option<usize>` or a `ScrollAnchor::Bottom` enum variant in `ScrollState`.

### 4d. Render signature divergence — LOW risk
- See section 1c above. Mix of `Buffer`/`RataBuf`, optional `theme` param, `&self`/`&mut self`.
- **Fix:** Standardize all to `fn render(&mut self, area: Rect, buf: &mut Buffer, focused: bool)`.

---

## 5. Type-Level Improvements

### 5a. Action enum — 84 variants — MEDIUM risk
- **File:** `keymap.rs`
- **Problem:** Flat enum with 84 variants mixing editor, navigation, panel, clipboard, terminal, and layout actions. `SelectLeft`/`SelectRight` are `#[allow(dead_code)]`.
- **Proposed grouping:** `Action::Editor(EditorAction)`, `Action::Panel(PanelAction)`, `Action::Navigation(NavAction)`, `Action::Tab(TabAction)`, `Action::Clipboard(ClipboardAction)`, `Action::Terminal(TerminalAction)`, `Action::Layout(LayoutAction)`.
- **Benefits:** Type-safe dispatch per handler, dead code detection, reduced cognitive load.
- **Risk:** Touches every `match` on `Action` across app.rs. MEDIUM effort, MEDIUM risk.

### 5b. Dead Action variants — LOW risk
- `SelectLeft`, `SelectRight` — marked `#[allow(dead_code)]`, never dispatched.
- **Fix:** Remove them.

### 5c. Duplicate keybindings — LOW risk
- `AltEnter` + `Shift+Enter` resolve to same action.
- `Ctrl+H` and `Ctrl+Backspace` both map to `DeleteWordBack`.
- **Fix:** Document as intentional or remove redundancy.

---

## 6. Performance

### 6a. Settings resolution re-reads disk on every call — HIGH impact
- **File:** `workspace.rs:259-291`
- **Problem:** `resolve_setting()` calls `fs::read_to_string()` + `serde_json::from_str()` for both folder-level and user-level settings files on every invocation. No caching.
- **Fix:** Cache parsed settings JSON in `Workspace` struct. Invalidate on file-watcher event.
- **Risk:** LOW — straightforward caching addition.

### 6b. Memory search brute-force — LOW impact (current scale)
- **File:** `memory/store.rs:159-168`
- **Problem:** Fetches ALL embeddings from SQLite, computes cosine similarity for each, sorts all, then truncates to limit.
- **Current scale:** Likely < 1000 entries — acceptable.
- **Future fix:** Binary heap for top-K, or SQL LIMIT on initial query.
- **Risk:** DEFERRED — no action needed now.

### 6c. O(n²) scope validation — LOW impact
- **File:** `swarm/validation.rs:37-62`
- **Problem:** Pairwise comparison of all work unit scopes.
- **Current scale:** n < 10 agents — O(100) comparisons.
- **Risk:** DEFERRED.

### 6d. Render-time state mutation — MEDIUM impact
- **File:** `app.rs:2687-2693, 3021-3027`
- **Problem:** Scroll offset adjusted during render calls, coupling layout to state.
- **Fix:** Move scroll adjustment to `handle_*` methods or dedicated `update()` pass.

---

## 7. Testability Gaps

### 7a. app.rs — untestable monolith
- **Problem:** All state transitions happen via methods on `App`, which requires a full ratatui `Terminal`, event channel, filesystem access, and optional TerminalManager.
- **Fix:** Extract pure state transition logic (focus cycling, panel mode switching, scroll calculations) into standalone functions or small structs that can be unit tested without UI dependencies.

### 7b. Panel rendering — coupled to ratatui Buffer
- **Problem:** Panel render methods write directly to `Buffer`. Testing requires constructing a `Buffer` of correct size and inspecting cells.
- **Fix:** Acceptable for now — ratatui's `TestBackend` makes this testable. Not a blocker.

### 7c. highlight.rs — unwrap-heavy tree-sitter setup
- **Problem:** Parser initialization unwraps (lines 131-198). If a language grammar fails to load, the whole editor panics.
- **Fix:** Return `Option<HighlightState>` and degrade gracefully to plain text.

### 7d. No integration tests for panel event handling
- **Problem:** Panel action handlers live in app.rs and are tested only implicitly.
- **Fix:** After decomposing app.rs, each handler module can have focused unit tests.

---

## Phase 2: Triage

### Quick Wins (low risk, high impact, < 30 min each)

| # | Finding | Section | Files | Risk |
|---|---------|---------|-------|------|
| QW-1 | Extract hardcoded colors to `theme.rs` constants | 3d | swarm_dashboard.rs, search.rs, terminal.rs, file_tree.rs, theme.rs | LOW |
| QW-2 | Remove dead `SelectLeft`/`SelectRight` Action variants | 5b | keymap.rs | LOW |
| QW-3 | Unify language extension mapping into single registry | 3a | tree_sitter.rs | LOW |
| QW-4 | Replace panel inline buffer writes with `render_utils` calls | 3e | All panels | LOW |
| QW-5 | Cache settings JSON in Workspace struct | 6a | workspace.rs | LOW |
| QW-6 | Fix `usize::MAX` scroll sentinel with explicit enum/option | 4c | agent_chat.rs | LOW |

### Structural — Panel Unification

| # | Step | Files | Risk | Status |
|---|------|-------|------|--------|
| PU-1 | Create `widgets/scroll_state.rs` with `ScrollState` struct | New file | LOW | ✅ Done |
| PU-2 | Migrate `search.rs` to use `ScrollState` (proof of concept) | search.rs, app.rs | LOW | ✅ Done |
| PU-3 | Migrate `file_tree.rs` to use `ScrollState` | file_tree.rs, app.rs | LOW | ✅ Done |
| PU-4 | Migrate `swarm_dashboard.rs` (table scroll) | swarm_dashboard.rs, app.rs | MEDIUM | ✅ Done |
| PU-5 | Create `widgets/text_input.rs` with `TextInput` struct | New file | MEDIUM |
| PU-6 | Migrate `git_panel.rs` to use `ScrollState` + `TextInput` | git_panel.rs | MEDIUM |
| PU-7 | Migrate `agent_chat.rs` to use `ScrollState` + `TextInput` | agent_chat.rs | HIGH |
| PU-8 | Standardize render signatures across all panels | All panels | LOW |

### Structural — Other

| # | Step | Files | Risk |
|---|------|-------|------|
| SO-1 | Extract `BatchReviewState` + render/input into `panels/review.rs` | app.rs → new file | MEDIUM |
| SO-2 | Extract `ChangesState` + render/input into `panels/changes.rs` | app.rs → new file | MEDIUM |
| SO-3 | Extract `build_simple_diff` + shared diff rendering into `ui/diff_utils.rs` | app.rs → new file | LOW |
| SO-4 | Extract `TreeDialog` into `ui/tree_dialog.rs` | app.rs → new file | LOW |
| SO-5 | Extract clipboard abstraction into `ui/clipboard.rs` | app.rs → new file | LOW |
| SO-6 | Extract mouse handling into `ui/mouse.rs` | app.rs → new file | MEDIUM |
| SO-7 | Split remaining app.rs into `app_input.rs` + `app_render.rs` | app.rs → new files | MEDIUM-HIGH |
| SO-8 | Group Action enum into sub-enums | keymap.rs + all handlers | MEDIUM |
| SO-9 | Replace dangerous `.unwrap()` calls with proper error handling | app.rs, highlight.rs, buffer.rs | MEDIUM |

### Deferred (high risk or low impact)

| # | Finding | Reason to defer |
|---|---------|-----------------|
| D-1 | Memory store brute-force search | Current scale < 1000 entries, acceptable |
| D-2 | O(n²) scope validation | n < 10, O(100) total |
| D-3 | Action enum sub-enum grouping | Large blast radius, every handler touched |
| D-4 | Full app.rs decomposition (SO-7) | Requires all other structural changes first |

---

## Phase 3: Quick Win Execution Log

| # | Status | Notes |
|---|--------|-------|
| QW-1 | ✅ Done | Added 12 constants to theme.rs; updated swarm_dashboard.rs (5 colors), search.rs (1), file_tree.rs (1), terminal.rs (1), app.rs (2). Removed unused `Color` imports from search.rs and file_tree.rs. |
| QW-2 | ✅ Done | Removed `SelectLeft`/`SelectRight` from Action enum and their match arms in app.rs. Added `#[allow(dead_code)]` to buffer methods (kept for future use; tested). |
| QW-3 | ✅ Done | Unified `language_for_extension` and `language_name_for_extension` into single `LANGUAGE_REGISTRY` table in tree_sitter.rs. Both functions now delegate to `lookup_extension()`. |
| QW-4 | ☐ Deferred | Panels already partially use render_utils. Full migration better done during panel unification (PU steps). |
| QW-5 | ✅ Done | Added `folder_settings_cache` (HashMap) and `user_settings_cache` (Option) to Workspace struct. Populated at construction and via `reload_settings_cache()`. `resolve_setting()` no longer reads disk. |
| QW-6 | ✅ Done | Replaced `usize::MAX` sentinel in agent_chat.rs with explicit `scroll_pinned_to_bottom: bool` field. Updated 5 set-sites and 1 check-site. Also fixed 1 occurrence in app.rs. |
