//! SWARM side panel: pick a `.gaviero` script, fill in its inputs, and follow
//! the run agent by agent.
//!
//! Holds state and rendering only. Launching and input live in
//! [`crate::app::swarm_run`]; every change arrives through the event loop as a
//! [`SwarmUpdate`].

use std::path::PathBuf;
use std::time::{Duration, Instant};

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};

use crate::theme;
use crate::widgets::prompt_input::{self, FileAutocomplete, PromptInput};
use crate::widgets::render_utils::{strip_ansi, word_wrap, write_text};
use crate::widgets::scroll_state::ScrollState;
use crate::widgets::scrollbar::render_scrollbar;
use crate::widgets::text_input::TextInput;
use gaviero_core::swarm::models::{AgentStatus, SwarmResult};
use gaviero_core::swarm::plan::ExecutionMode;
use gaviero_core::types::ModelTier;
use gaviero_dsl::{ParamKindInfo, ScriptInspection, VarKind};

/// Activity kept per agent; the oldest output goes first.
const MAX_ACTIVITY_LINES: usize = 500;
const MAX_ACTIVITY_BYTES: usize = 256 * 1024;

// ── Updates ─────────────────────────────────────────────────────

/// One change to the current run, from the pipeline's observers.
#[derive(Debug)]
pub enum SwarmUpdate {
    Phase(String),
    Agent {
        id: String,
        status: AgentStatus,
        detail: String,
    },
    Tier {
        current: usize,
        total: usize,
    },
    Dispatch {
        id: String,
        tier: ModelTier,
        backend: String,
    },
    Text {
        id: String,
        text: String,
    },
    Tool {
        id: String,
        summary: String,
    },
    Status {
        id: String,
        status: String,
    },
    File {
        id: String,
        path: String,
        additions: usize,
        deletions: usize,
    },
    /// An agent's whole answer; shown only if it never streamed any text.
    Message {
        id: String,
        content: String,
    },
    Notice(String),
    Cost(f64),
    Completed(Box<SwarmResult>),
    Failed(String),
    Stopped,
    Reverted(Result<(), String>),
}

// ── Activity ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum ActivityKind {
    Text,
    ToolCall,
    Status,
    FileChange,
}

#[derive(Debug, Clone)]
pub struct ActivityLine {
    pub kind: ActivityKind,
    pub text: String,
}

// ── Picker ──────────────────────────────────────────────────────

/// One script the picker can offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptRow {
    pub name: String,
    /// `global`, `workspace`, a folder name, or `path`.
    pub source: String,
    pub description: Option<String>,
    pub path: PathBuf,
    /// The workspace folder that owns a repo-scoped script.
    pub owner: Option<PathBuf>,
}

pub struct ScriptPicker {
    pub rows: Vec<ScriptRow>,
    pub filter: TextInput,
    pub scroll: ScrollState,
    /// Where scripts are looked for, for the empty state.
    pub searched: Vec<String>,
    /// Script folders that could not be listed.
    pub warnings: Vec<String>,
    pub error: Option<String>,
    list_rect: Rect,
}

impl ScriptPicker {
    fn new() -> Self {
        Self {
            rows: Vec::new(),
            filter: TextInput::new(),
            scroll: ScrollState::new(),
            searched: Vec::new(),
            warnings: Vec::new(),
            error: None,
            list_rect: Rect::default(),
        }
    }

    /// Rows matching the filter, in catalog order.
    pub fn visible(&self) -> Vec<&ScriptRow> {
        let needle = self.filter.text.to_lowercase();
        self.rows
            .iter()
            .filter(|r| needle.is_empty() || r.name.to_lowercase().contains(&needle))
            .collect()
    }

    pub fn selected(&self) -> Option<ScriptRow> {
        self.visible().get(self.scroll.selected).map(|r| (*r).clone())
    }

    /// Keep the selection on the list after the filter narrowed it.
    pub fn filter_changed(&mut self) {
        self.scroll.reset();
        self.error = None;
    }

    /// The list row at a screen position, if any.
    pub fn index_at(&self, column: u16, row: u16) -> Option<usize> {
        if !self.list_rect.contains((column, row).into()) {
            return None;
        }
        let index = self.scroll.offset + (row - self.list_rect.y) as usize;
        (index < self.visible().len()).then_some(index)
    }
}

// ── Form ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldRole {
    Folder,
    Workflow,
    Profile,
    Prompt,
    Param(ParamKindInfo),
    Var,
    /// A var declared `file`.
    File,
    /// A var declared `dir`.
    Dir,
}

#[derive(Debug, Clone)]
pub enum FieldValue {
    Choice { options: Vec<String>, selected: usize },
    Text(TextInput),
    /// The script's prompt, in the same box the chat uses.
    Prompt(Box<PromptInput>),
}

#[derive(Debug, Clone)]
pub struct FormField {
    pub name: String,
    pub role: FieldRole,
    pub value: FieldValue,
    /// What an untouched field holds; `None` when the script gives no value.
    pub default: Option<String>,
    pub required: bool,
}

impl FormField {
    fn text(&self) -> Option<&str> {
        match &self.value {
            FieldValue::Text(input) => Some(&input.text),
            FieldValue::Prompt(prompt) => Some(&prompt.input.text),
            FieldValue::Choice { .. } => None,
        }
    }

    /// True once the user has changed a text field from its default.
    fn is_edited(&self) -> bool {
        self.text()
            .is_some_and(|t| t != self.default.as_deref().unwrap_or(""))
    }

    fn label(&self) -> String {
        let required = if self.required { " *" } else { "" };
        let kind = match self.role {
            FieldRole::Param(ParamKindInfo::Roster) => " (roster)",
            FieldRole::Param(ParamKindInfo::Client) => " (model)",
            FieldRole::File => " (file)",
            FieldRole::Dir => " (folder)",
            _ => "",
        };
        format!("{}{kind}{required}", self.name)
    }

    fn hint(&self) -> &'static str {
        match self.role {
            FieldRole::Folder => "Where the script runs. ←/→ to change",
            FieldRole::Workflow => "Which workflow to run. ←/→ to change",
            FieldRole::Profile => "Which models the script's tiers use. ←/→ to change",
            FieldRole::Prompt => "Alt+Enter new line  Alt+↑↓ resize  $skill  @file",
            FieldRole::Param(ParamKindInfo::Roster) => "id=provider:model[@effort],… or id=tier:<name>,…",
            FieldRole::Param(ParamKindInfo::Client) => "provider:model[@effort] or tier:<name>",
            FieldRole::Var => "Script variable. Empty leaves it unset",
            FieldRole::File => "An existing file. Type to search, as with @ in chat",
            FieldRole::Dir => "A folder. Type to search the workspace",
        }
    }

    fn prompt_field(required: bool) -> Self {
        Self {
            name: "Prompt".to_string(),
            role: FieldRole::Prompt,
            value: FieldValue::Prompt(Box::default()),
            default: None,
            required,
        }
    }

    fn text_field(name: &str, role: FieldRole, default: Option<String>, required: bool) -> Self {
        let mut input = TextInput::new();
        if let Some(value) = &default {
            input.insert_str(value);
        }
        Self {
            name: name.to_string(),
            role,
            value: FieldValue::Text(input),
            default,
            required,
        }
    }
}

/// The inputs of one run, ready to compile with. Values the user left at the
/// script's default are omitted, so the script's own richer default (a
/// client's privacy or extra settings) still applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormValues {
    pub root: PathBuf,
    pub workflow: Option<String>,
    /// `None` leaves the choice to the `default` profile, if there is one.
    pub profile: Option<String>,
    pub prompt: Option<String>,
    pub vars: Vec<(String, String)>,
    pub params: Vec<(String, String)>,
}

/// First entry of the Profile selector: no explicit choice.
const PROFILE_AUTO: &str = "auto";

/// Shown in place of the prompt box, so its absence does not look like a fault.
const NO_PROMPT_NOTE: &str = "This script takes no prompt: its agents have fixed instructions.";

pub struct SwarmForm {
    pub script: ScriptRow,
    pub inspection: ScriptInspection,
    pub folders: Vec<(String, PathBuf)>,
    /// Profile names on offer; the selector is hidden when empty.
    pub profiles: Vec<String>,
    pub fields: Vec<FormField>,
    /// `fields.len()` is the Run button.
    pub focus: usize,
    pub error: Option<String>,
    pub warnings: Vec<String>,
    /// Popup of the focused file or param field.
    pub completion: FileAutocomplete,
}

impl SwarmForm {
    /// `folders` must not be empty.
    pub fn new(
        script: ScriptRow,
        inspection: ScriptInspection,
        folders: Vec<(String, PathBuf)>,
        selected_folder: usize,
    ) -> Self {
        let mut form = Self {
            script,
            inspection,
            folders,
            profiles: Vec::new(),
            fields: Vec::new(),
            focus: 0,
            error: None,
            warnings: Vec::new(),
            completion: FileAutocomplete::new(),
        };
        form.fields = form.build_fields(selected_folder, 0, 0);
        form
    }

    /// Offer these profiles in a selector.
    pub fn with_profiles(mut self, profiles: Vec<String>) -> Self {
        self.profiles = profiles;
        self.fields = self.build_fields(self.selected_folder(), self.selected_workflow(), 0);
        self
    }

    fn build_fields(&self, folder: usize, workflow: usize, profile: usize) -> Vec<FormField> {
        let mut fields = Vec::new();
        if self.folders.len() > 1 {
            fields.push(FormField {
                name: "Folder".into(),
                role: FieldRole::Folder,
                value: FieldValue::Choice {
                    options: self.folders.iter().map(|(name, _)| name.clone()).collect(),
                    selected: folder,
                },
                default: None,
                required: false,
            });
        }
        if self.inspection.workflows.len() > 1 {
            fields.push(FormField {
                name: "Workflow".into(),
                role: FieldRole::Workflow,
                value: FieldValue::Choice {
                    options: self
                        .inspection
                        .workflows
                        .iter()
                        .map(|w| w.name.clone())
                        .collect(),
                    selected: workflow,
                },
                default: None,
                required: false,
            });
        }
        if !self.profiles.is_empty() {
            let mut options = vec![PROFILE_AUTO.to_string()];
            options.extend(self.profiles.iter().cloned());
            fields.push(FormField {
                name: "Profile".into(),
                role: FieldRole::Profile,
                value: FieldValue::Choice {
                    options,
                    selected: profile,
                },
                default: None,
                required: false,
            });
        }
        if self.inspection.accepts_prompt {
            fields.push(FormField::prompt_field(self.inspection.requires_prompt));
        }
        if let Some(wf) = self.inspection.workflows.get(workflow) {
            for param in &wf.params {
                fields.push(FormField::text_field(
                    &param.name,
                    FieldRole::Param(param.kind),
                    param.default.clone(),
                    param.default.is_none(),
                ));
            }
        }
        for var in &self.inspection.vars {
            let (role, required) = match var.kind {
                VarKind::File => (FieldRole::File, var.default.is_none()),
                VarKind::Dir => (FieldRole::Dir, var.default.is_none()),
                VarKind::String => (FieldRole::Var, false),
            };
            fields.push(FormField::text_field(
                &var.name,
                role,
                var.default.clone(),
                required,
            ));
        }
        fields
    }

    fn choice_of(&self, role: FieldRole) -> usize {
        self.fields
            .iter()
            .find(|f| f.role == role)
            .and_then(|f| match f.value {
                FieldValue::Choice { selected, .. } => Some(selected),
                _ => None,
            })
            .unwrap_or(0)
    }

    pub fn selected_folder(&self) -> usize {
        self.choice_of(FieldRole::Folder)
    }

    fn selected_workflow(&self) -> usize {
        self.choice_of(FieldRole::Workflow)
    }

    /// The profile picked by name; entry 0 of the selector is [`PROFILE_AUTO`].
    pub fn selected_profile(&self) -> Option<String> {
        let index = self.choice_of(FieldRole::Profile).checked_sub(1)?;
        self.profiles.get(index).cloned()
    }

    pub fn execution_mode(&self) -> ExecutionMode {
        self.inspection
            .workflows
            .get(self.selected_workflow())
            .map_or(self.inspection.default_execution, |w| w.execution)
    }

    /// Move a choice field by `delta`. The params on show follow the workflow;
    /// anything the user typed survives under the same name.
    pub fn cycle_focused_choice(&mut self, delta: isize) {
        let Some(field) = self.fields.get_mut(self.focus) else {
            return;
        };
        let FieldValue::Choice { options, selected } = &mut field.value else {
            return;
        };
        let len = options.len() as isize;
        *selected = (*selected as isize + delta).rem_euclid(len) as usize;
        if field.role != FieldRole::Workflow {
            return;
        }

        let focused = self.focus;
        let folder = self.selected_folder();
        let workflow = self.selected_workflow();
        let profile = self.choice_of(FieldRole::Profile);
        let old = std::mem::take(&mut self.fields);
        let mut fresh = self.build_fields(folder, workflow, profile);
        for field in &mut fresh {
            if let Some(previous) = old
                .iter()
                .find(|o| o.name == field.name && o.role == field.role && o.is_edited())
            {
                field.value = previous.value.clone();
            }
        }
        self.fields = fresh;
        self.focus = focused.min(self.fields.len());
    }

    pub fn focused_input(&mut self) -> Option<&mut TextInput> {
        match &mut self.fields.get_mut(self.focus)?.value {
            FieldValue::Text(input) => Some(input),
            FieldValue::Prompt(prompt) => Some(&mut prompt.input),
            FieldValue::Choice { .. } => None,
        }
    }

    /// The prompt box, when it has the keyboard.
    pub fn focused_prompt(&mut self) -> Option<&mut PromptInput> {
        match &mut self.fields.get_mut(self.focus)?.value {
            FieldValue::Prompt(prompt) => Some(prompt),
            _ => None,
        }
    }

    /// The prompt box, if this script takes a prompt.
    pub fn prompt_mut(&mut self) -> Option<&mut PromptInput> {
        self.fields.iter_mut().find_map(|f| match &mut f.value {
            FieldValue::Prompt(prompt) => Some(prompt.as_mut()),
            _ => None,
        })
    }

    /// A press on the prompt box focuses it and places the cursor.
    pub fn press_prompt(&mut self, col: u16, row: u16) {
        let index = self.fields.iter().position(|f| match &f.value {
            FieldValue::Prompt(prompt) => {
                prompt.area.is_some_and(|a| a.contains((col, row).into()))
            }
            _ => false,
        });
        if let Some(index) = index {
            self.focus = index;
            if let Some(prompt) = self.prompt_mut() {
                prompt.start_drag(col, row);
            }
        }
    }

    pub fn focused_role(&self) -> Option<FieldRole> {
        self.fields.get(self.focus).map(|f| f.role)
    }

    /// Focus the field called `name`, if there is one.
    pub fn focus_field(&mut self, name: &str) {
        if let Some(index) = self.fields.iter().position(|f| f.name == name) {
            self.focus = index;
        }
    }

    /// Open or close the popup for what the focused field now holds.
    pub fn update_completion(&mut self) {
        let Some(field) = self.fields.get(self.focus) else {
            return;
        };
        match (field.role, &field.value) {
            (FieldRole::File | FieldRole::Dir, FieldValue::Text(input)) => {
                prompt_input::update_path_autocomplete(input, &mut self.completion);
            }
            (FieldRole::Param(kind), FieldValue::Text(input)) => {
                prompt_input::update_tier_autocomplete(
                    input,
                    &mut self.completion,
                    kind == ParamKindInfo::Roster,
                );
            }
            _ => self.completion.reset(),
        }
    }

    pub fn accept_completion(&mut self) {
        if let Some(FormField {
            value: FieldValue::Text(input),
            ..
        }) = self.fields.get_mut(self.focus)
        {
            prompt_input::accept_autocomplete(input, &mut self.completion);
        }
    }

    pub fn on_run_button(&self) -> bool {
        self.focus >= self.fields.len()
    }

    pub fn focus_next(&mut self) {
        self.completion.reset();
        self.focus = (self.focus + 1).min(self.fields.len());
    }

    pub fn focus_previous(&mut self) {
        self.completion.reset();
        self.focus = self.focus.saturating_sub(1);
    }

    /// Validate and gather the inputs. The message names the first missing
    /// field and focus moves there.
    pub fn collect(&mut self) -> Result<FormValues, String> {
        if let Some(index) = self
            .fields
            .iter()
            .position(|f| f.required && f.text().is_some_and(|t| t.trim().is_empty()))
        {
            self.focus = index;
            return Err(format!("{} is required", self.fields[index].name));
        }

        let workflow = self
            .inspection
            .workflows
            .get(self.selected_workflow())
            .map(|w| w.name.clone());
        let mut values = FormValues {
            root: self.folders[self.selected_folder()].1.clone(),
            workflow,
            profile: self.selected_profile(),
            prompt: None,
            vars: Vec::new(),
            params: Vec::new(),
        };
        for field in &self.fields {
            let Some(text) = field.text() else { continue };
            match field.role {
                FieldRole::Prompt if !text.trim().is_empty() => {
                    values.prompt = Some(text.to_string());
                }
                FieldRole::Param(_) if field.is_edited() => {
                    values.params.push((field.name.clone(), text.to_string()));
                }
                // `dir` only chooses the completion; otherwise it is a string.
                FieldRole::Var | FieldRole::Dir if field.is_edited() => {
                    values.vars.push((field.name.clone(), text.to_string()));
                }
                // Even at its default: run preparation must see the path.
                FieldRole::File if !text.trim().is_empty() => {
                    values.vars.push((field.name.clone(), text.trim().to_string()));
                }
                _ => {}
            }
        }
        Ok(values)
    }
}

// ── Agents ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AgentEntry {
    pub id: String,
    pub status: AgentStatus,
    pub branch: Option<String>,
    /// Latest one-liner for the table row.
    pub detail: String,
    pub modified_files: Vec<String>,
    pub model_tier: Option<ModelTier>,
    pub backend: Option<String>,
    pub activity: Vec<ActivityLine>,
    /// Bumped on every activity change; keys the wrapped-output cache.
    revision: u64,
    saw_text: bool,
    started_at: Option<Instant>,
    /// Frozen run time once the agent has finished.
    finished_after: Option<Duration>,
}

/// An agent known from the compiled plan before anything has run.
#[derive(Debug, Clone)]
pub struct PlannedAgent {
    pub id: String,
    pub tier: ModelTier,
    pub model: Option<String>,
}

impl AgentEntry {
    fn new(id: String, status: AgentStatus, detail: String) -> Self {
        let started_at = matches!(status, AgentStatus::Running).then(Instant::now);
        Self {
            branch: None,
            modified_files: Vec::new(),
            model_tier: None,
            backend: None,
            activity: Vec::new(),
            revision: 0,
            saw_text: false,
            finished_after: None,
            id,
            status,
            detail,
            started_at,
        }
    }

    fn push_activity(&mut self, kind: ActivityKind, text: String) {
        self.activity.push(ActivityLine { kind, text });
        self.trim_activity();
    }

    fn trim_activity(&mut self) {
        self.revision += 1;
        if self.activity.len() > MAX_ACTIVITY_LINES {
            let excess = self.activity.len() - MAX_ACTIVITY_LINES;
            self.activity.drain(..excess);
        }
        let mut total: usize = self.activity.iter().map(|l| l.text.len()).sum();
        while total > MAX_ACTIVITY_BYTES {
            let over = total - MAX_ACTIVITY_BYTES;
            if self.activity.len() > 1 && self.activity[0].text.len() <= over {
                total -= self.activity.remove(0).text.len();
            } else {
                let first = &mut self.activity[0].text;
                let mut cut = over.min(first.len());
                while !first.is_char_boundary(cut) {
                    cut += 1;
                }
                first.drain(..cut);
                total -= cut;
            }
        }
    }

    fn elapsed(&self) -> Option<Duration> {
        self.finished_after
            .or_else(|| self.started_at.map(|s| s.elapsed()))
    }

    fn is_finished(&self) -> bool {
        matches!(self.status, AgentStatus::Completed | AgentStatus::Failed(_))
    }
}

// ── Dashboard ───────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwarmView {
    Picker,
    Form,
    Run,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DashboardFocus {
    Table,
    Detail,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiffLineKind {
    Added,
    Removed,
    Context,
    Header,
}

#[derive(Debug, Clone)]
pub struct SwarmAgentDiffState {
    pub agent_id: String,
    pub lines: Vec<(DiffLineKind, String)>,
    pub scroll: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AgentCounts {
    pub running: usize,
    pub queued: usize,
    pub done: usize,
    pub failed: usize,
}

#[derive(Default)]
struct WrappedOutput {
    agent: usize,
    width: usize,
    revision: u64,
    lines: Vec<(ActivityKind, String)>,
}

pub struct SwarmDashboardState {
    pub view: SwarmView,
    pub picker: ScriptPicker,
    pub form: Option<SwarmForm>,
    /// Height the user gave the prompt box with `Alt+Up/Down`; 0 fits the text.
    pub prompt_rows: u16,

    /// Identifies the current run; updates from an older one are dropped.
    pub run_id: u64,
    /// A task is executing the current run.
    pub running: bool,
    pub abort: Option<tokio::task::AbortHandle>,
    pub script_label: String,
    pub run_root: Option<PathBuf>,
    pub agents: Vec<AgentEntry>,
    pub scroll: ScrollState,
    pub phase: String,
    pub tier_current: usize,
    pub tier_total: usize,
    pub result: Option<SwarmResult>,
    pub estimated_cost_usd: f64,
    /// Latest run-wide message (preparation notes, failures, gate results).
    pub status_message: String,
    pub detail_scroll: usize,
    pub detail_auto_scroll: bool,
    pub focus: DashboardFocus,
    /// The selected agent's output fills the panel.
    pub output_maximized: bool,
    /// Selection jumps to whichever agent starts, until the user picks one.
    pub follow_running: bool,
    pub diff_agent: Option<SwarmAgentDiffState>,
    pub pending_undo_confirm: bool,
    pub pending_stop_confirm: bool,
    pub table_rect: Rect,
    pub detail_rect: Rect,
    wrapped: WrappedOutput,
}

impl SwarmDashboardState {
    pub fn new() -> Self {
        Self {
            view: SwarmView::Picker,
            picker: ScriptPicker::new(),
            form: None,
            prompt_rows: 0,
            run_id: 0,
            running: false,
            abort: None,
            script_label: String::new(),
            run_root: None,
            agents: Vec::new(),
            scroll: ScrollState::new(),
            phase: "idle".to_string(),
            tier_current: 0,
            tier_total: 0,
            result: None,
            estimated_cost_usd: 0.0,
            status_message: String::new(),
            detail_scroll: 0,
            detail_auto_scroll: true,
            focus: DashboardFocus::Table,
            output_maximized: false,
            follow_running: true,
            diff_agent: None,
            pending_undo_confirm: false,
            pending_stop_confirm: false,
            table_rect: Rect::default(),
            detail_rect: Rect::default(),
            wrapped: WrappedOutput::default(),
        }
    }

    /// True once any run has been started in this session.
    pub fn has_run(&self) -> bool {
        self.run_id > 0
    }

    /// Show a new run, with every planned agent listed as queued.
    pub fn begin_run(
        &mut self,
        script_label: String,
        run_root: PathBuf,
        planned: Vec<PlannedAgent>,
        notes: Vec<String>,
    ) -> u64 {
        self.run_id += 1;
        self.running = true;
        self.abort = None;
        self.view = SwarmView::Run;
        self.form = None;
        self.script_label = script_label;
        self.run_root = Some(run_root);
        self.agents = planned
            .into_iter()
            .map(|p| {
                let mut entry = AgentEntry::new(p.id, AgentStatus::Pending, "queued".to_string());
                entry.model_tier = Some(p.tier);
                entry.backend = p.model;
                entry
            })
            .collect();
        self.scroll.reset();
        self.phase = "starting".to_string();
        self.tier_current = 0;
        self.tier_total = 0;
        self.result = None;
        self.estimated_cost_usd = 0.0;
        self.status_message = notes.join("\n");
        self.detail_scroll = 0;
        self.detail_auto_scroll = true;
        self.focus = DashboardFocus::Table;
        self.output_maximized = false;
        self.follow_running = true;
        self.diff_agent = None;
        self.pending_undo_confirm = false;
        self.pending_stop_confirm = false;
        self.wrapped = WrappedOutput::default();
        self.run_id
    }

    pub fn counts(&self) -> AgentCounts {
        let mut counts = AgentCounts::default();
        for agent in &self.agents {
            match agent.status {
                AgentStatus::Running => counts.running += 1,
                AgentStatus::Pending => counts.queued += 1,
                AgentStatus::Completed => counts.done += 1,
                AgentStatus::Failed(_) => counts.failed += 1,
            }
        }
        counts
    }

    /// Apply an update. Those from a run that is no longer current are ignored.
    pub fn apply(&mut self, run: u64, update: SwarmUpdate) {
        if run != self.run_id {
            return;
        }
        match update {
            SwarmUpdate::Phase(phase) => self.phase = phase,
            SwarmUpdate::Agent { id, status, detail } => self.update_agent(&id, status, detail),
            SwarmUpdate::Tier { current, total } => {
                self.tier_current = current;
                self.tier_total = total;
            }
            SwarmUpdate::Dispatch { id, tier, backend } => {
                if let Some(entry) = self.agent_mut(&id) {
                    entry.model_tier = Some(tier);
                    entry.backend = Some(backend);
                }
            }
            SwarmUpdate::Text { id, text } => self.append_text(&id, &text),
            SwarmUpdate::Tool { id, summary } => {
                let text = if summary.starts_with('[') {
                    summary
                } else {
                    format!("[{summary}]")
                };
                let entry = self.agent_or_insert(&id);
                entry.detail = text.clone();
                entry.push_activity(ActivityKind::ToolCall, text);
            }
            SwarmUpdate::Status { id, status } => self.agent_or_insert(&id).detail = status,
            SwarmUpdate::File {
                id,
                path,
                additions,
                deletions,
            } => {
                let line = format!("[wrote {path} +{additions} -{deletions}]");
                let entry = self.agent_or_insert(&id);
                entry.detail = line.clone();
                entry.push_activity(ActivityKind::FileChange, line);
            }
            SwarmUpdate::Message { id, content } => {
                let entry = self.agent_or_insert(&id);
                if !entry.saw_text && !content.trim().is_empty() {
                    entry.saw_text = true;
                    entry.push_activity(ActivityKind::Text, strip_ansi(&content));
                }
            }
            SwarmUpdate::Notice(message) => self.status_message = message,
            SwarmUpdate::Cost(usd) => self.estimated_cost_usd = usd,
            SwarmUpdate::Completed(result) => self.finish_with_result(*result),
            SwarmUpdate::Failed(message) => {
                self.end_run("failed");
                self.status_message = message;
            }
            SwarmUpdate::Stopped => {
                self.end_run("stopped");
                self.status_message = "Run stopped.".to_string();
            }
            SwarmUpdate::Reverted(outcome) => match outcome {
                Ok(()) => {
                    self.result = None;
                    self.diff_agent = None;
                    self.phase = "reverted".to_string();
                    self.status_message = "Reverted to the state before the run.".to_string();
                }
                Err(e) => self.status_message = format!("Undo failed: {e}"),
            },
        }
    }

    /// Close out a run that ended without a result: anything unfinished
    /// did not finish.
    fn end_run(&mut self, phase: &str) {
        self.running = false;
        self.abort = None;
        self.pending_stop_confirm = false;
        self.phase = phase.to_string();
        for agent in &mut self.agents {
            if !agent.is_finished() {
                agent.finished_after = agent.started_at.map(|s| s.elapsed());
                agent.status = AgentStatus::Failed(phase.to_string());
                agent.detail = phase.to_string();
            }
        }
    }

    fn finish_with_result(&mut self, result: SwarmResult) {
        self.running = false;
        self.abort = None;
        self.pending_stop_confirm = false;
        self.phase = if result.success {
            "completed".to_string()
        } else {
            "completed with failures".to_string()
        };
        for manifest in &result.manifests {
            let Some(entry) = self
                .agents
                .iter_mut()
                .find(|a| a.id == manifest.work_unit_id)
            else {
                continue;
            };
            entry.status = manifest.status.clone();
            if entry.finished_after.is_none() {
                entry.finished_after = entry.started_at.map(|s| s.elapsed());
            }
            entry.branch = manifest.branch.clone();
            entry.modified_files = manifest
                .modified_files
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            let count = entry.modified_files.len();
            entry.detail = if count > 0 {
                format!("Modified {count} file{}", if count == 1 { "" } else { "s" })
            } else {
                manifest
                    .summary
                    .clone()
                    .unwrap_or_else(|| entry.detail.clone())
            };
        }
        // An agent the run never reached has no manifest and stays queued.
        for agent in &mut self.agents {
            if matches!(agent.status, AgentStatus::Pending | AgentStatus::Running) {
                agent.status = AgentStatus::Failed("not run".to_string());
                agent.detail = "not run".to_string();
            }
        }
        self.result = Some(result);
    }

    fn agent_mut(&mut self, id: &str) -> Option<&mut AgentEntry> {
        self.agents.iter_mut().find(|a| a.id == id)
    }

    /// Agents the plan did not name up front (a coordinator, an expanded
    /// clone) still get a row.
    fn agent_or_insert(&mut self, id: &str) -> &mut AgentEntry {
        let index = match self.agents.iter().position(|a| a.id == id) {
            Some(index) => index,
            None => {
                self.agents.push(AgentEntry::new(
                    id.to_string(),
                    AgentStatus::Running,
                    String::new(),
                ));
                self.agents.len() - 1
            }
        };
        &mut self.agents[index]
    }

    fn update_agent(&mut self, id: &str, status: AgentStatus, detail: String) {
        let entry = self.agent_or_insert(id);
        let changed = std::mem::discriminant(&entry.status) != std::mem::discriminant(&status);
        let now_running = matches!(status, AgentStatus::Running);
        if now_running && (changed || entry.started_at.is_none()) {
            // A loop runs the same agent again: the clock restarts each pass.
            entry.started_at = Some(Instant::now());
            entry.finished_after = None;
            entry.saw_text = false;
        }
        if matches!(status, AgentStatus::Completed | AgentStatus::Failed(_))
            && entry.finished_after.is_none()
        {
            entry.finished_after = entry.started_at.map(|s| s.elapsed());
        }
        if changed {
            let line = match &status {
                AgentStatus::Pending => format!("Queued: {detail}"),
                AgentStatus::Running => format!("Running: {detail}"),
                AgentStatus::Completed => format!("Completed: {detail}"),
                AgentStatus::Failed(e) => format!("Failed: {e}"),
            };
            entry.push_activity(ActivityKind::Status, line);
        }
        entry.status = status;
        entry.detail = detail;

        if now_running && changed {
            self.follow(id);
        }
    }

    /// Move the selection to a newly started agent, unless the agent already
    /// selected is itself running or the user has taken over.
    fn follow(&mut self, id: &str) {
        if !self.follow_running {
            return;
        }
        let selected_running = self
            .agents
            .get(self.scroll.selected)
            .is_some_and(|a| a.id != id && matches!(a.status, AgentStatus::Running));
        if selected_running {
            return;
        }
        if let Some(index) = self.agents.iter().position(|a| a.id == id) {
            self.select(index);
        }
    }

    fn select(&mut self, index: usize) {
        if self.scroll.selected != index {
            self.detail_scroll = 0;
            self.detail_auto_scroll = true;
        }
        self.scroll.selected = index;
        self.scroll.ensure_visible();
    }

    /// The user picked an agent; following stops until the next run.
    pub fn select_by_user(&mut self, index: usize) {
        self.follow_running = false;
        self.select(index.min(self.agents.len().saturating_sub(1)));
    }

    fn append_text(&mut self, id: &str, text: &str) {
        // Raw escape codes written into the cell buffer corrupt other panels.
        let clean = strip_ansi(text);
        let entry = self.agent_or_insert(id);
        entry.saw_text = true;
        match entry.activity.last_mut() {
            Some(last) if last.kind == ActivityKind::Text => {
                last.text.push_str(&clean);
                entry.trim_activity();
            }
            _ => entry.push_activity(ActivityKind::Text, clean),
        }
    }

    pub fn show_diff(&mut self, agent_id: String, diff_text: String) {
        self.diff_agent = Some(SwarmAgentDiffState {
            agent_id,
            lines: parse_diff(&diff_text),
            scroll: 0,
        });
    }

    /// Wrapped output of the selected agent. Rewrapped only when it changed.
    fn wrapped_output(&mut self, width: usize) -> &[(ActivityKind, String)] {
        let selected = self.scroll.selected;
        let Some(agent) = self.agents.get(selected) else {
            self.wrapped.lines.clear();
            return &self.wrapped.lines;
        };
        let current = selected == self.wrapped.agent
            && width == self.wrapped.width
            && agent.revision == self.wrapped.revision;
        if !current {
            self.wrapped = WrappedOutput {
                agent: selected,
                width,
                revision: agent.revision,
                lines: wrap_activity(&agent.activity, width),
            };
        }
        &self.wrapped.lines
    }

    /// Rendered rows of the selected agent's output at the last drawn width.
    pub fn output_row_count(&mut self) -> usize {
        let width = self.detail_rect.width.saturating_sub(2) as usize;
        self.wrapped_output(width.max(1)).len()
    }

    // ── Rendering ───────────────────────────────────────────────

    pub fn render(&mut self, area: Rect, buf: &mut Buffer, focused: bool) {
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                buf[(x, y)]
                    .set_char(' ')
                    .set_style(Style::default().bg(theme::PANEL_BG));
            }
        }
        if area.height == 0 || area.width == 0 {
            return;
        }
        match self.view {
            SwarmView::Picker => self.render_picker(area, buf, focused),
            SwarmView::Form => self.render_form(area, buf, focused),
            SwarmView::Run => self.render_run(area, buf, focused),
        }
    }

    fn render_picker(&mut self, area: Rect, buf: &mut Buffer, focused: bool) {
        let bg = theme::PANEL_BG;
        let right = area.right();
        put(buf, area.x, area.y, right, " SWARM  choose a script", bold(theme::WARNING));

        let filter_y = area.y + 1;
        let label_end = put(buf, area.x, filter_y, right, " Filter: ", fg(theme::TEXT_DIM));
        let input = &self.picker.filter;
        let end = put(buf, label_end, filter_y, right, &input.text, fg(theme::TEXT_BRIGHT));
        if focused && end < right {
            buf[(end, filter_y)]
                .set_char(' ')
                .set_style(Style::default().add_modifier(Modifier::REVERSED));
        }

        let footer_rows: u16 = 2;
        let list_y = area.y + 2;
        let list_height = area.height.saturating_sub(2 + footer_rows);
        let list = Rect {
            x: area.x,
            y: list_y,
            width: area.width,
            height: list_height,
        };
        self.picker.list_rect = list;
        self.picker.scroll.set_viewport(list.height as usize);
        self.picker.scroll.ensure_visible_on_render();

        let rows: Vec<ScriptRow> = self.picker.visible().into_iter().cloned().collect();
        if rows.is_empty() {
            self.render_picker_empty(list, buf);
        }
        let selected_bg = if focused {
            theme::FOCUSED_SELECTION_BG
        } else {
            theme::DARK_BG
        };
        for (row_index, row) in rows
            .iter()
            .enumerate()
            .skip(self.picker.scroll.offset)
            .take(list.height as usize)
        {
            let y = list.y + (row_index - self.picker.scroll.offset) as u16;
            let row_bg = if row_index == self.picker.scroll.selected {
                selected_bg
            } else {
                bg
            };
            for x in area.x..right {
                buf[(x, y)].set_style(Style::default().bg(row_bg));
            }
            let x = put(buf, area.x + 1, y, right, &row.name, bold(theme::TEXT_FG).bg(row_bg));
            let source = format!(" [{}]", row.source);
            let x = put(buf, x, y, right, &source, fg(theme::TEXT_DIM).bg(row_bg));
            if let Some(description) = &row.description {
                let text = format!("  {description}");
                put(buf, x, y, right, &text, fg(theme::TEXT_DIM).bg(row_bg));
            }
        }

        let footer_y = area.bottom().saturating_sub(footer_rows);
        if let Some(error) = &self.picker.error {
            put(buf, area.x + 1, footer_y, right, error, fg(theme::ERROR));
        } else if let Some(warning) = self.picker.warnings.first() {
            put(buf, area.x + 1, footer_y, right, warning, fg(theme::WARNING));
        }
        let hint = if self.has_run() {
            " ↑↓ select  Enter open  Esc back to run"
        } else {
            " ↑↓ select  Enter open  type to filter or a path"
        };
        put(buf, area.x, area.bottom() - 1, right, hint, fg(theme::TEXT_DIM));
    }

    fn render_picker_empty(&self, list: Rect, buf: &mut Buffer) {
        let right = list.right();
        let mut y = list.y;
        let filtered = !self.picker.filter.text.is_empty();
        let message = if self.picker.filter.text.ends_with(".gaviero") {
            " Enter opens this path."
        } else if filtered {
            " No script matches."
        } else {
            " No scripts yet. Add <name>/<name>.gaviero under:"
        };
        put(buf, list.x, y, right, message, fg(theme::TEXT_DIM));
        y += 1;
        if filtered {
            return;
        }
        for dir in &self.picker.searched {
            if y >= list.bottom() {
                break;
            }
            put(buf, list.x + 1, y, right, dir, fg(theme::TEXT_DIM));
            y += 1;
        }
    }

    fn render_form(&mut self, area: Rect, buf: &mut Buffer, focused: bool) {
        let Some(form) = self.form.as_ref() else {
            return;
        };
        let right = area.right();
        let title = format!(" {}  [{}]", form.script.name, form.script.source);
        put(buf, area.x, area.y, right, &title, bold(theme::WARNING));

        let width = area.width.saturating_sub(4).max(1) as usize;
        let error_rows: Vec<String> = form
            .error
            .as_deref()
            .map(|e| word_wrap(e, area.width.saturating_sub(2).max(1) as usize))
            .unwrap_or_default();
        let error_height = (error_rows.len() as u16).min(area.height / 2);

        let box_width = area.width.saturating_sub(2);
        let mut rows: Vec<FormRow> = Vec::new();
        for warning in &form.warnings {
            for line in word_wrap(warning, width + 2) {
                rows.push(FormRow::plain(line, fg(theme::WARNING)));
            }
        }
        if !form.inspection.accepts_prompt {
            for line in word_wrap(NO_PROMPT_NOTE, width + 2) {
                rows.push(FormRow::plain(line, fg(theme::TEXT_DIM)));
            }
        }
        for (index, field) in form.fields.iter().enumerate() {
            let is_focused = index == form.focus;
            let label_style = if is_focused {
                bold(theme::ACCENT)
            } else {
                fg(theme::TEXT_FG)
            };
            rows.push(FormRow::field(field.label(), label_style, index, None));
            match &field.value {
                FieldValue::Choice { options, selected } => {
                    let value = format!("‹ {} ›", options[*selected]);
                    rows.push(FormRow::field(value, fg(theme::TEXT_BRIGHT), index, None));
                }
                FieldValue::Text(input) => {
                    let cursor = is_focused.then_some(input.cursor);
                    for (text, col) in field_rows(&input.text, cursor, width, 2) {
                        let style = Style::default().fg(theme::TEXT_BRIGHT).bg(theme::INPUT_BG);
                        rows.push(FormRow {
                            text,
                            style,
                            cursor: col,
                            field: Some(index),
                            fill: Some(theme::INPUT_BG),
                            prompt_box: false,
                        });
                    }
                }
                FieldValue::Prompt(prompt) => {
                    let height = prompt_input::box_height(
                        self.prompt_rows,
                        prompt.visual_line_count(box_width),
                        (area.height / 2).max(3),
                    );
                    for _ in 0..height {
                        rows.push(FormRow::prompt_box(index));
                    }
                }
            }
        }
        let on_button = form.on_run_button();
        let button_style = if on_button {
            bold(Color::Black).bg(theme::SUCCESS)
        } else {
            fg(theme::SUCCESS)
        };
        rows.push(FormRow::field(" Run ".to_string(), button_style, form.fields.len(), None));

        let hint = form
            .fields
            .get(form.focus)
            .map(|f| f.hint())
            .unwrap_or("Enter starts the run");
        let footer_y = area.bottom() - 1;
        put(buf, area.x + 1, footer_y, right, hint, fg(theme::TEXT_DIM));

        let body_y = area.y + 1;
        let body_height = area.height.saturating_sub(2 + error_height) as usize;
        let first_focused = rows.iter().position(|r| r.field == Some(form.focus)).unwrap_or(0);
        let last_focused = rows
            .iter()
            .rposition(|r| r.field == Some(form.focus))
            .unwrap_or(0);
        let offset = form_offset(first_focused, last_focused, body_height, rows.len());
        let mut box_rows: Option<(u16, u16)> = None;
        let mut value_rows: Option<(u16, u16)> = None;
        for (i, row) in rows.iter().skip(offset).take(body_height).enumerate() {
            let y = body_y + i as u16;
            if row.field == Some(form.focus) && row.fill.is_some() && !row.prompt_box {
                let (top, height) = value_rows.unwrap_or((y, 0));
                value_rows = Some((top, height + 1));
            }
            if row.prompt_box {
                let (top, height) = box_rows.unwrap_or((y, 0));
                box_rows = Some((top, height + 1));
            }
            if let Some(fill) = row.fill {
                for x in (area.x + 1)..right.saturating_sub(1) {
                    buf[(x, y)].set_style(Style::default().bg(fill));
                }
            }
            put(buf, area.x + 2, y, right, &row.text, row.style);
            if focused && let Some(col) = row.cursor {
                let x = area.x + 2 + col as u16;
                if x < right {
                    let cell = &mut buf[(x, y)];
                    cell.set_style(cell.style().add_modifier(Modifier::REVERSED));
                }
            }
        }
        for (i, line) in error_rows.iter().take(error_height as usize).enumerate() {
            let y = footer_y - error_height + i as u16;
            put(buf, area.x + 1, y, right, line, fg(theme::ERROR));
        }

        let box_area = box_rows.map(|(y, height)| Rect {
            x: area.x + 1,
            y,
            width: box_width,
            height,
        });
        let bounds = Rect {
            y: area.y + 1,
            height: area.height.saturating_sub(2),
            ..area
        };
        self.render_prompt_box(box_area, bounds, buf, focused);

        if let (Some(form), Some((y, height))) = (self.form.as_ref(), value_rows)
            && form.completion.is_open()
        {
            let value = Rect {
                x: area.x + 1,
                y,
                width: box_width,
                height,
            };
            if let Some(popup) =
                prompt_input::popup_area(value, bounds, form.completion.matches.len())
            {
                prompt_input::render_autocomplete(&form.completion, popup, buf);
            }
        }
    }

    /// Draw the prompt box into the rows reserved for it, then its popup,
    /// which may cover other fields.
    fn render_prompt_box(
        &mut self,
        box_area: Option<Rect>,
        bounds: Rect,
        buf: &mut Buffer,
        focused: bool,
    ) {
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let prompt_focused = form.focused_prompt().is_some();
        let Some(prompt) = form.prompt_mut() else {
            return;
        };
        prompt.area = box_area;
        let Some(area) = box_area else {
            return;
        };
        prompt_input::render_input(
            area,
            buf,
            &prompt_input::InputView {
                input: &prompt.input,
                label: PromptInput::LABEL,
                hint: Some("Type the prompt"),
                show_caret: focused && prompt_focused,
            },
        );
        if prompt_focused
            && prompt.autocomplete.is_open()
            && let Some(popup) =
                prompt_input::popup_area(area, bounds, prompt.autocomplete.matches.len())
        {
            prompt_input::render_autocomplete(&prompt.autocomplete, popup, buf);
        }
    }

    fn render_run(&mut self, area: Rect, buf: &mut Buffer, focused: bool) {
        let bg = theme::PANEL_BG;
        let right = area.right();
        self.render_run_header(area, buf);

        let summary_y = area.y + 1;
        let counts = self.counts();
        let mut x = area.x + 1;
        for (icon, count, label, color) in [
            ("●", counts.running, "running", theme::FOCUS_BORDER),
            ("◯", counts.queued, "queued", theme::TEXT_DIM),
            ("✓", counts.done, "done", theme::SUCCESS),
            ("✗", counts.failed, "failed", theme::ERROR),
        ] {
            let text = format!("{icon} {count} {label}  ");
            let style = if count > 0 { bold(color) } else { fg(theme::TEXT_DIM) };
            x = put(buf, x, summary_y, right, &text, style);
        }

        let footer_y = area.bottom().saturating_sub(1);
        let message_rows: Vec<String> = if self.status_message.is_empty() {
            Vec::new()
        } else {
            word_wrap(&self.status_message, area.width.saturating_sub(2).max(1) as usize)
        };
        let message_height = (message_rows.len() as u16).min(area.height / 4).min(4);

        let content_y = area.y + 2 + message_height;
        for (i, line) in message_rows.iter().take(message_height as usize).enumerate() {
            let color = if self.phase == "failed" {
                theme::ERROR
            } else {
                theme::TEXT_DIM
            };
            put(buf, area.x + 1, area.y + 2 + i as u16, right, line, fg(color));
        }
        let content_height = footer_y.saturating_sub(content_y);

        let table_height = if self.output_maximized {
            0
        } else {
            (self.agents.len() as u16)
                .min(content_height / 2)
                .min(10)
                .max(1)
        };
        let table_area = Rect {
            x: area.x,
            y: content_y,
            width: area.width,
            height: table_height,
        };
        let separator_y = content_y + table_height;
        let detail_area = Rect {
            x: area.x,
            y: separator_y + 1,
            width: area.width,
            height: footer_y.saturating_sub(separator_y + 1),
        };
        self.table_rect = table_area;
        self.detail_rect = detail_area;

        if table_height > 0 {
            let selection_bg = if focused && self.focus == DashboardFocus::Table {
                theme::FOCUSED_SELECTION_BG
            } else {
                theme::DARK_BG
            };
            self.scroll.set_viewport(table_area.height as usize);
            self.scroll.ensure_visible_on_render();
            self.render_agent_table(table_area, buf, bg, selection_bg);
        }
        if separator_y < footer_y {
            self.render_separator(area.x, separator_y, area.width, buf, bg);
        }
        if detail_area.height > 0 {
            self.render_detail(detail_area, buf, bg);
        }

        let hint = self.run_hint();
        let style = if self.pending_stop_confirm || self.pending_undo_confirm {
            bold(theme::WARNING)
        } else {
            fg(theme::TEXT_DIM)
        };
        put(buf, area.x + 1, footer_y, right, hint, style);
    }

    fn run_hint(&self) -> &'static str {
        if self.pending_stop_confirm {
            "Press x again to stop every agent. Esc cancels."
        } else if self.pending_undo_confirm {
            "Press u again to undo the whole run. Esc cancels."
        } else if self.diff_agent.is_some() {
            "↑↓ scroll  Esc back to output"
        } else if self.running {
            "↑↓ agent  Enter expand  Tab scroll output  x stop"
        } else {
            "Esc scripts  ↑↓ agent  Enter expand  d diff  u undo"
        }
    }

    fn render_run_header(&self, area: Rect, buf: &mut Buffer) {
        let cost = if self.estimated_cost_usd > 0.0 {
            format!("  ~${:.3}", self.estimated_cost_usd)
        } else {
            String::new()
        };
        let tier = if self.tier_total > 0 {
            format!("  Tier {}/{}", self.tier_current, self.tier_total)
        } else {
            String::new()
        };
        let header = format!(" {}  {}{tier}{cost}", self.script_label, self.phase);
        put(buf, area.x, area.y, area.right(), &header, bold(theme::WARNING));
    }

    fn render_agent_table(&self, area: Rect, buf: &mut Buffer, bg: Color, selection_bg: Color) {
        let id_width = self
            .agents
            .iter()
            .map(|a| a.id.chars().count())
            .max()
            .unwrap_or(0)
            .min(24)
            .min((area.width as usize).saturating_sub(10) / 2)
            .max(6) as u16;

        for row in 0..area.height as usize {
            let index = self.scroll.offset + row;
            let Some(agent) = self.agents.get(index) else {
                break;
            };
            let y = area.y + row as u16;
            let row_bg = if index == self.scroll.selected {
                selection_bg
            } else {
                bg
            };
            for x in area.x..area.right() {
                buf[(x, y)].set_style(Style::default().bg(row_bg));
            }

            let running = matches!(agent.status, AgentStatus::Running);
            let (icon, icon_color) = match &agent.status {
                AgentStatus::Pending => ("◯", theme::TEXT_DIM),
                AgentStatus::Running => (spinner(), theme::FOCUS_BORDER),
                AgentStatus::Completed => ("✓", theme::SUCCESS),
                AgentStatus::Failed(_) => ("✗", theme::ERROR),
            };
            put(buf, area.x + 1, y, area.right(), icon, bold(icon_color).bg(row_bg));

            let mut col = area.x + 3;
            if let Some(tier) = &agent.model_tier {
                let (badge, color) = match tier {
                    ModelTier::Cheap => ("C", theme::TEXT_DIM),
                    ModelTier::Expensive => ("E", theme::WARNING),
                };
                put(buf, col, y, area.right(), badge, bold(color).bg(row_bg));
                col += 2;
            }

            let id_style = if running {
                bold(theme::TEXT_BRIGHT).bg(row_bg)
            } else {
                fg(theme::TEXT_FG).bg(row_bg)
            };
            let id: String = agent.id.chars().take(id_width as usize).collect();
            put(buf, col, y, area.right(), &id, id_style);
            col += id_width + 1;

            let elapsed = format_elapsed(agent.elapsed());
            let elapsed_x = area.right().saturating_sub(elapsed.chars().count() as u16 + 1);
            if !elapsed.is_empty() && elapsed_x > col {
                put(buf, elapsed_x, y, area.right(), &elapsed, fg(theme::TEXT_DIM).bg(row_bg));
            }
            let detail_end = if elapsed.is_empty() {
                area.right()
            } else {
                elapsed_x.saturating_sub(1)
            };
            if detail_end > col {
                let detail = table_detail(agent);
                put(buf, col, y, detail_end, &detail, fg(theme::TEXT_DIM).bg(row_bg));
            }
        }
    }

    fn render_separator(&self, x: u16, y: u16, width: u16, buf: &mut Buffer, bg: Color) {
        let right = x + width;
        let active = self.focus == DashboardFocus::Detail;
        let line_style = Style::default()
            .fg(if active { theme::FOCUS_BORDER } else { theme::TEXT_DIM })
            .bg(bg);
        for cx in x..right {
            buf[(cx, y)].set_char('─').set_style(line_style);
        }
        let label = match self.agents.get(self.scroll.selected) {
            Some(agent) => match &agent.backend {
                Some(backend) => format!(" {} · {backend} ", agent.id),
                None => format!(" {} ", agent.id),
            },
            None => " no agent ".to_string(),
        };
        put(buf, x + 2, y, right, &label, bold(theme::FOCUS_BORDER).bg(bg));
    }

    fn render_detail(&mut self, area: Rect, buf: &mut Buffer, bg: Color) {
        if self.diff_agent.is_some() {
            self.render_diff(area, buf, bg);
            return;
        }
        let width = area.width.saturating_sub(2).max(1) as usize;
        let auto_scroll = self.detail_auto_scroll;
        let requested = self.detail_scroll;
        let empty_message = match self.agents.get(self.scroll.selected) {
            Some(agent) if agent.activity.is_empty() => Some(match agent.status {
                AgentStatus::Pending => " Queued. Output appears when it starts.",
                _ => " Waiting for output...",
            }),
            _ => None,
        };
        if let Some(message) = empty_message {
            put(buf, area.x, area.y, area.right(), message, fg(theme::TEXT_DIM));
            return;
        }
        let lines = self.wrapped_output(width);
        let total = lines.len();
        let viewport = area.height as usize;
        let scroll = if auto_scroll {
            total.saturating_sub(viewport)
        } else {
            requested.min(total.saturating_sub(viewport))
        };
        for (row, (kind, text)) in lines.iter().skip(scroll).take(viewport).enumerate() {
            let style = match kind {
                ActivityKind::ToolCall => bold(theme::TOOL_DIM),
                ActivityKind::FileChange => fg(theme::SUCCESS),
                ActivityKind::Status => fg(theme::INFO_CYAN),
                ActivityKind::Text => fg(theme::TEXT_FG),
            };
            put(
                buf,
                area.x + 1,
                area.y + row as u16,
                area.x + 1 + width as u16,
                text,
                style.bg(bg),
            );
        }
        if total > viewport {
            render_scrollbar(area, buf, total, viewport, scroll);
        }
    }

    fn render_diff(&self, area: Rect, buf: &mut Buffer, bg: Color) {
        let Some(diff) = &self.diff_agent else {
            return;
        };
        let right = area.right();
        let title = format!(" [diff] {}", diff.agent_id);
        put(buf, area.x, area.y, right, &title, bold(theme::ACCENT));
        if area.height <= 1 {
            return;
        }
        let content = Rect {
            y: area.y + 1,
            height: area.height - 1,
            ..area
        };
        if diff.lines.is_empty() {
            put(buf, content.x + 1, content.y, right, "(no diff)", fg(theme::TEXT_DIM));
            return;
        }
        let width = content.width.saturating_sub(2).max(1) as usize;
        let wrapped: Vec<(&DiffLineKind, String)> = diff
            .lines
            .iter()
            .flat_map(|(kind, text)| word_wrap(text, width).into_iter().map(move |l| (kind, l)))
            .collect();
        let viewport = content.height as usize;
        let scroll = diff.scroll.min(wrapped.len().saturating_sub(viewport));
        for (row, (kind, text)) in wrapped.iter().skip(scroll).take(viewport).enumerate() {
            let style = match kind {
                DiffLineKind::Added => fg(Color::Green),
                DiffLineKind::Removed => fg(Color::Red),
                DiffLineKind::Header => bold(Color::Yellow),
                DiffLineKind::Context => fg(theme::TEXT_DIM),
            };
            put(
                buf,
                content.x + 1,
                content.y + row as u16,
                content.x + 1 + width as u16,
                text,
                style.bg(bg),
            );
        }
        if wrapped.len() > viewport {
            render_scrollbar(content, buf, wrapped.len(), viewport, scroll);
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────

struct FormRow {
    text: String,
    style: Style,
    cursor: Option<usize>,
    field: Option<usize>,
    fill: Option<Color>,
    /// Blank row the prompt box is drawn over.
    prompt_box: bool,
}

impl FormRow {
    fn plain(text: String, style: Style) -> Self {
        Self {
            text,
            style,
            cursor: None,
            field: None,
            fill: None,
            prompt_box: false,
        }
    }

    fn prompt_box(field: usize) -> Self {
        Self {
            text: String::new(),
            style: Style::default(),
            cursor: None,
            field: Some(field),
            fill: Some(theme::INPUT_BG),
            prompt_box: true,
        }
    }

    fn field(text: String, style: Style, field: usize, cursor: Option<usize>) -> Self {
        Self {
            text,
            style,
            cursor,
            field: Some(field),
            fill: None,
            prompt_box: false,
        }
    }
}

fn fg(color: Color) -> Style {
    Style::default().fg(color)
}

fn bold(color: Color) -> Style {
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

fn put(buf: &mut Buffer, x: u16, y: u16, x_max: u16, text: &str, style: Style) -> u16 {
    write_text(buf, x, y, x_max, text, style)
}

fn spinner() -> &'static str {
    let tick = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        / 200;
    ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"][(tick % 10) as usize]
}

/// First row to draw so the focused field is on screen, preferring to show
/// its label over its tail.
fn form_offset(first: usize, last: usize, viewport: usize, total: usize) -> usize {
    if total <= viewport || viewport == 0 {
        return 0;
    }
    let lowest_start = (last + 1).saturating_sub(viewport);
    lowest_start.min(first).min(total - viewport)
}

/// Wrap a field's value by characters into at most `max_rows` rows around the
/// cursor. Returns each row and, on the cursor's row, its column.
fn field_rows(
    text: &str,
    cursor: Option<usize>,
    width: usize,
    max_rows: usize,
) -> Vec<(String, Option<usize>)> {
    let width = width.max(1);
    let chars: Vec<char> = text
        .chars()
        .map(|c| if c == '\n' { '↵' } else { c })
        .collect();
    let row_count = chars.len().div_ceil(width).max(1);
    let cursor_row = cursor.map(|c| c.min(chars.len()) / width);

    let last_row = (row_count - 1).max(cursor_row.unwrap_or(0));
    let first = match cursor_row {
        Some(row) => (row + 1).saturating_sub(max_rows),
        None => 0,
    };
    (first..=last_row.min(first + max_rows.saturating_sub(1)))
        .map(|row| {
            let start = row * width;
            let end = ((row + 1) * width).min(chars.len());
            let line: String = chars.get(start..end).unwrap_or(&[]).iter().collect();
            let col = match (cursor, cursor_row) {
                (Some(c), Some(r)) if r == row => Some(c.min(chars.len()) - start),
                _ => None,
            };
            (line, col)
        })
        .collect()
}

fn wrap_activity(activity: &[ActivityLine], width: usize) -> Vec<(ActivityKind, String)> {
    let mut lines = Vec::new();
    for entry in activity {
        for line in entry.text.split('\n') {
            if line.is_empty() {
                continue;
            }
            for wrapped in word_wrap(line, width) {
                lines.push((entry.kind.clone(), wrapped));
            }
        }
    }
    lines
}

fn table_detail(agent: &AgentEntry) -> String {
    let text = if agent.detail.is_empty() {
        match &agent.status {
            AgentStatus::Failed(e) => e.as_str(),
            _ => "",
        }
    } else {
        agent.detail.as_str()
    };
    text.replace(['\n', '\r'], " ")
}

fn format_elapsed(elapsed: Option<Duration>) -> String {
    let Some(elapsed) = elapsed else {
        return String::new();
    };
    let secs = elapsed.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Classify the lines of a unified diff.
pub fn parse_diff(text: &str) -> Vec<(DiffLineKind, String)> {
    text.lines()
        .map(|line| {
            let kind = if line.starts_with("+++")
                || line.starts_with("---")
                || line.starts_with("diff ")
                || line.starts_with("index ")
                || line.starts_with("@@")
                || line.starts_with("new file")
                || line.starts_with("deleted file")
            {
                DiffLineKind::Header
            } else if line.starts_with('+') {
                DiffLineKind::Added
            } else if line.starts_with('-') {
                DiffLineKind::Removed
            } else {
                DiffLineKind::Context
            };
            (kind, line.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gaviero_dsl::{ParamInspection, VarInspection, WorkflowInspection};

    fn row() -> ScriptRow {
        ScriptRow {
            name: "audit".into(),
            source: "repo".into(),
            description: None,
            path: PathBuf::from("/x/audit.gaviero"),
            owner: None,
        }
    }

    fn workflow(name: &str, params: Vec<ParamInspection>) -> WorkflowInspection {
        WorkflowInspection {
            name: name.into(),
            execution: ExecutionMode::Repo,
            params,
        }
    }

    fn param(name: &str, default: Option<&str>) -> ParamInspection {
        ParamInspection {
            name: name.into(),
            kind: ParamKindInfo::Client,
            default: default.map(str::to_string),
        }
    }

    fn inspection(workflows: Vec<WorkflowInspection>) -> ScriptInspection {
        ScriptInspection {
            workflows,
            vars: vec![
                VarInspection {
                    name: "OUT_DIR".into(),
                    default: Some("out".into()),
                    kind: VarKind::String,
                },
                VarInspection {
                    name: "TARGET".into(),
                    default: None,
                    kind: VarKind::String,
                },
            ],
            requires_prompt: true,
            accepts_prompt: true,
            default_execution: ExecutionMode::Repo,
            tiers: Vec::new(),
        }
    }

    fn folders() -> Vec<(String, PathBuf)> {
        vec![
            ("a".to_string(), PathBuf::from("/a")),
            ("b".to_string(), PathBuf::from("/b")),
        ]
    }

    fn type_into(form: &mut SwarmForm, name: &str, text: &str) {
        form.focus = form.fields.iter().position(|f| f.name == name).unwrap();
        let input = form.focused_input().unwrap();
        input.clear();
        input.insert_str(text);
    }

    #[test]
    fn untouched_defaults_are_not_passed_as_overrides() {
        let mut form = SwarmForm::new(
            row(),
            inspection(vec![workflow("w", vec![param("judge", Some("claude:opus"))])]),
            folders(),
            1,
        );
        type_into(&mut form, "Prompt", "fix it");

        let values = form.collect().unwrap();
        assert_eq!(values.root, PathBuf::from("/b"));
        assert_eq!(values.workflow.as_deref(), Some("w"));
        assert_eq!(values.prompt.as_deref(), Some("fix it"));
        assert!(values.params.is_empty(), "default keeps the script's own settings");
        assert!(values.vars.is_empty());
    }

    #[test]
    fn edited_values_are_passed_and_empty_undeclared_vars_are_not() {
        let mut form = SwarmForm::new(
            row(),
            inspection(vec![workflow("w", vec![param("judge", Some("claude:opus"))])]),
            folders(),
            0,
        );
        type_into(&mut form, "Prompt", "p");
        type_into(&mut form, "judge", "codex:x@low");
        type_into(&mut form, "OUT_DIR", "elsewhere");

        let values = form.collect().unwrap();
        assert_eq!(values.params, vec![("judge".to_string(), "codex:x@low".to_string())]);
        assert_eq!(values.vars, vec![("OUT_DIR".to_string(), "elsewhere".to_string())]);
    }

    #[test]
    fn missing_required_input_is_reported_and_focused() {
        let mut form = SwarmForm::new(
            row(),
            inspection(vec![workflow("w", vec![param("gate", None)])]),
            folders(),
            0,
        );
        type_into(&mut form, "Prompt", "p");

        assert_eq!(form.collect().unwrap_err(), "gate is required");
        assert_eq!(form.fields[form.focus].name, "gate");
    }

    #[test]
    fn changing_workflow_swaps_params_and_keeps_typed_values() {
        let mut form = SwarmForm::new(
            row(),
            inspection(vec![
                workflow("one", vec![param("only_one", Some("a:b"))]),
                workflow("two", vec![param("only_two", Some("c:d"))]),
            ]),
            folders(),
            0,
        );
        type_into(&mut form, "Prompt", "keep me");
        form.focus = form.fields.iter().position(|f| f.name == "Workflow").unwrap();

        form.cycle_focused_choice(1);

        let names: Vec<&str> = form.fields.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"only_two") && !names.contains(&"only_one"), "{names:?}");
        let values = form.collect().unwrap();
        assert_eq!(values.workflow.as_deref(), Some("two"));
        assert_eq!(values.prompt.as_deref(), Some("keep me"));
    }

    fn focus_field(form: &mut SwarmForm, name: &str) {
        form.focus = form.fields.iter().position(|f| f.name == name).unwrap();
    }

    #[test]
    fn profile_selector_starts_on_auto_and_names_the_choice() {
        let mut form = SwarmForm::new(row(), inspection(vec![workflow("w", vec![])]), folders(), 0)
            .with_profiles(vec!["claude".into(), "mixed".into()]);
        type_into(&mut form, "Prompt", "p");

        assert_eq!(form.collect().unwrap().profile, None, "auto chooses nothing");

        focus_field(&mut form, "Profile");
        form.cycle_focused_choice(1);
        assert_eq!(form.collect().unwrap().profile.as_deref(), Some("claude"));

        form.cycle_focused_choice(1);
        assert_eq!(form.collect().unwrap().profile.as_deref(), Some("mixed"));
    }

    #[test]
    fn profile_choice_survives_changing_the_workflow() {
        let mut form = SwarmForm::new(
            row(),
            inspection(vec![workflow("one", vec![]), workflow("two", vec![])]),
            folders(),
            0,
        )
        .with_profiles(vec!["claude".into()]);
        type_into(&mut form, "Prompt", "p");
        focus_field(&mut form, "Profile");
        form.cycle_focused_choice(1);

        focus_field(&mut form, "Workflow");
        form.cycle_focused_choice(1);

        let values = form.collect().unwrap();
        assert_eq!(values.workflow.as_deref(), Some("two"));
        assert_eq!(values.profile.as_deref(), Some("claude"));
    }

    #[test]
    fn no_profiles_means_no_selector() {
        let form = SwarmForm::new(row(), inspection(vec![workflow("w", vec![])]), folders(), 0)
            .with_profiles(Vec::new());
        assert!(form.fields.iter().all(|f| f.role != FieldRole::Profile));
    }

    #[test]
    fn single_folder_and_workflow_need_no_selector() {
        let form = SwarmForm::new(
            row(),
            inspection(vec![workflow("w", vec![])]),
            vec![("a".to_string(), PathBuf::from("/a"))],
            0,
        );
        assert!(form.fields.iter().all(|f| !matches!(f.value, FieldValue::Choice { .. })));
    }

    #[test]
    fn enter_reaches_the_run_button_only_after_the_last_field() {
        let mut form = SwarmForm::new(
            row(),
            inspection(vec![workflow("w", vec![])]),
            vec![("a".to_string(), PathBuf::from("/a"))],
            0,
        );
        for _ in 0..form.fields.len() {
            assert!(!form.on_run_button());
            form.focus_next();
        }
        assert!(form.on_run_button());
        form.focus_next();
        assert!(form.on_run_button(), "focus stops on the button");
    }

    fn planned(id: &str) -> PlannedAgent {
        PlannedAgent {
            id: id.into(),
            tier: ModelTier::Cheap,
            model: Some("claude:sonnet".into()),
        }
    }

    fn running_dashboard(ids: &[&str]) -> (SwarmDashboardState, u64) {
        let mut dash = SwarmDashboardState::new();
        let run = dash.begin_run(
            "audit".into(),
            PathBuf::from("/a"),
            ids.iter().map(|i| planned(i)).collect(),
            vec![],
        );
        (dash, run)
    }

    fn set_status(dash: &mut SwarmDashboardState, run: u64, id: &str, status: AgentStatus) {
        dash.apply(
            run,
            SwarmUpdate::Agent {
                id: id.into(),
                status,
                detail: "d".into(),
            },
        );
    }

    #[test]
    fn every_planned_agent_is_listed_before_it_runs() {
        let (dash, _) = running_dashboard(&["a", "b", "c"]);
        assert_eq!(
            dash.counts(),
            AgentCounts {
                queued: 3,
                ..Default::default()
            }
        );
    }

    #[test]
    fn counts_follow_agent_states() {
        let (mut dash, run) = running_dashboard(&["a", "b", "c", "d"]);
        set_status(&mut dash, run, "a", AgentStatus::Running);
        set_status(&mut dash, run, "b", AgentStatus::Completed);
        set_status(&mut dash, run, "c", AgentStatus::Failed("boom".into()));
        assert_eq!(
            dash.counts(),
            AgentCounts {
                running: 1,
                queued: 1,
                done: 1,
                failed: 1
            }
        );
    }

    #[test]
    fn output_is_kept_per_agent_and_streamed_chunks_coalesce() {
        let (mut dash, run) = running_dashboard(&["a", "b"]);
        for (id, text) in [("a", "hel"), ("b", "other"), ("a", "lo")] {
            dash.apply(
                run,
                SwarmUpdate::Text {
                    id: id.into(),
                    text: text.into(),
                },
            );
        }
        assert_eq!(dash.agents[0].activity.len(), 1);
        assert_eq!(dash.agents[0].activity[0].text, "hello");
        assert_eq!(dash.agents[1].activity[0].text, "other");
    }

    #[test]
    fn final_message_is_shown_only_when_nothing_streamed() {
        let (mut dash, run) = running_dashboard(&["streamed", "silent"]);
        dash.apply(
            run,
            SwarmUpdate::Text {
                id: "streamed".into(),
                text: "chunk".into(),
            },
        );
        for id in ["streamed", "silent"] {
            dash.apply(
                run,
                SwarmUpdate::Message {
                    id: id.into(),
                    content: "whole answer".into(),
                },
            );
        }
        assert_eq!(dash.agents[0].activity.len(), 1, "no duplicate of the stream");
        assert_eq!(dash.agents[0].activity[0].text, "chunk");
        assert_eq!(dash.agents[1].activity[0].text, "whole answer");
    }

    #[test]
    fn updates_from_a_previous_run_are_ignored() {
        let (mut dash, old) = running_dashboard(&["a"]);
        let new = dash.begin_run(
            "next".into(),
            PathBuf::from("/a"),
            vec![planned("a")],
            vec![],
        );
        assert_ne!(old, new);

        dash.apply(old, SwarmUpdate::Failed("late".into()));

        assert!(dash.running);
        assert_eq!(dash.phase, "starting");
    }

    #[test]
    fn selection_follows_the_running_agent_until_the_user_picks_one() {
        let (mut dash, run) = running_dashboard(&["a", "b", "c"]);
        set_status(&mut dash, run, "b", AgentStatus::Running);
        assert_eq!(dash.scroll.selected, 1);

        dash.select_by_user(0);
        set_status(&mut dash, run, "c", AgentStatus::Running);
        assert_eq!(dash.scroll.selected, 0, "the user's choice sticks");
    }

    #[test]
    fn following_does_not_leave_an_agent_that_is_still_running() {
        let (mut dash, run) = running_dashboard(&["a", "b"]);
        set_status(&mut dash, run, "a", AgentStatus::Running);
        set_status(&mut dash, run, "b", AgentStatus::Running);
        assert_eq!(dash.scroll.selected, 0);
    }

    #[test]
    fn stopping_marks_unfinished_agents_and_keeps_finished_ones() {
        let (mut dash, run) = running_dashboard(&["done", "live", "queued"]);
        set_status(&mut dash, run, "done", AgentStatus::Completed);
        set_status(&mut dash, run, "live", AgentStatus::Running);

        dash.apply(run, SwarmUpdate::Stopped);

        assert!(!dash.running);
        assert_eq!(dash.phase, "stopped");
        assert!(matches!(dash.agents[0].status, AgentStatus::Completed));
        assert!(matches!(dash.agents[1].status, AgentStatus::Failed(_)));
        assert!(matches!(dash.agents[2].status, AgentStatus::Failed(_)));
    }

    #[test]
    fn a_rerun_agent_restarts_its_clock() {
        let (mut dash, run) = running_dashboard(&["a"]);
        set_status(&mut dash, run, "a", AgentStatus::Running);
        set_status(&mut dash, run, "a", AgentStatus::Completed);
        assert!(dash.agents[0].finished_after.is_some());

        set_status(&mut dash, run, "a", AgentStatus::Running);
        assert!(dash.agents[0].finished_after.is_none());
    }

    #[test]
    fn activity_is_bounded_in_bytes_and_stays_valid_utf8() {
        let (mut dash, run) = running_dashboard(&["a"]);
        for _ in 0..40 {
            dash.apply(
                run,
                SwarmUpdate::Text {
                    id: "a".into(),
                    text: "é".repeat(5_000),
                },
            );
        }
        let total: usize = dash.agents[0].activity.iter().map(|l| l.text.len()).sum();
        assert!(total <= MAX_ACTIVITY_BYTES, "{total}");
        assert!(dash.agents[0].activity[0].text.ends_with('é'));
    }

    #[test]
    fn ansi_is_stripped_from_output() {
        let (mut dash, run) = running_dashboard(&["a"]);
        dash.apply(
            run,
            SwarmUpdate::Text {
                id: "a".into(),
                text: "\u{1b}[31mred\u{1b}[0m".into(),
            },
        );
        assert_eq!(dash.agents[0].activity[0].text, "red");
    }

    #[test]
    fn field_rows_wrap_and_place_the_cursor() {
        assert_eq!(field_rows("abcdef", Some(4), 3, 4), vec![
            ("abc".to_string(), None),
            ("def".to_string(), Some(1)),
        ]);
        assert_eq!(field_rows("", Some(0), 3, 2), vec![(String::new(), Some(0))]);
        // The cursor past a full last row gets a fresh row.
        let rows = field_rows("abc", Some(3), 3, 2);
        assert_eq!(rows.last().unwrap(), &(String::new(), Some(0)));
    }

    #[test]
    fn field_rows_show_the_part_around_the_cursor() {
        let rows = field_rows("aaabbbcccddd", Some(11), 3, 2);
        assert_eq!(rows[0].0, "ccc");
        assert_eq!(rows[1], ("ddd".to_string(), Some(2)));
    }

    #[test]
    fn form_offset_keeps_the_focused_field_visible() {
        assert_eq!(form_offset(0, 1, 10, 5), 0);
        assert_eq!(form_offset(12, 13, 5, 20), 9);
        assert_eq!(form_offset(2, 3, 5, 20), 0);
        assert_eq!(form_offset(19, 19, 5, 20), 15);
    }

    fn render_rows(dash: &mut SwarmDashboardState, width: u16, height: u16) -> Vec<String> {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        dash.render(area, &mut buf, true);
        (0..height)
            .map(|y| (0..width).map(|x| buf[(x, y)].symbol().to_string()).collect())
            .collect()
    }

    #[test]
    fn run_view_shows_who_is_running_and_the_selected_output() {
        let (mut dash, run) = running_dashboard(&["planner", "builder"]);
        set_status(&mut dash, run, "planner", AgentStatus::Running);
        dash.apply(
            run,
            SwarmUpdate::Text {
                id: "planner".into(),
                text: "thinking about it".into(),
            },
        );

        let screen = render_rows(&mut dash, 70, 16).join("\n");

        assert!(screen.contains("1 running"), "{screen}");
        assert!(screen.contains("1 queued"), "{screen}");
        assert!(screen.contains("planner"), "{screen}");
        assert!(screen.contains("builder"), "{screen}");
        assert!(screen.contains("thinking about it"), "{screen}");
        assert!(screen.contains("x stop"), "{screen}");
    }

    #[test]
    fn selecting_another_agent_shows_its_output_instead() {
        let (mut dash, run) = running_dashboard(&["a", "b"]);
        for (id, text) in [("a", "from-a"), ("b", "from-b")] {
            set_status(&mut dash, run, id, AgentStatus::Running);
            dash.apply(
                run,
                SwarmUpdate::Text {
                    id: id.into(),
                    text: text.into(),
                },
            );
        }
        dash.select_by_user(1);

        let screen = render_rows(&mut dash, 70, 16).join("\n");
        assert!(screen.contains("from-b"), "{screen}");
        assert!(!screen.contains("from-a"), "{screen}");
    }

    #[test]
    fn picker_lists_scripts_and_says_where_to_add_them() {
        let mut dash = SwarmDashboardState::new();
        dash.picker.searched = vec!["/w/.gaviero/dsl".to_string()];
        let empty = render_rows(&mut dash, 60, 10).join("\n");
        assert!(empty.contains("<name>/<name>.gaviero"), "{empty}");
        assert!(empty.contains("/w/.gaviero/dsl"), "{empty}");

        dash.picker.rows = vec![row()];
        let listed = render_rows(&mut dash, 60, 10).join("\n");
        assert!(listed.contains("audit"), "{listed}");
        assert!(listed.contains("[repo]"), "{listed}");
    }

    #[test]
    fn picker_filter_narrows_the_list() {
        let mut dash = SwarmDashboardState::new();
        let mut other = row();
        other.name = "review".into();
        dash.picker.rows = vec![row(), other];
        dash.picker.filter.insert_str("REV");
        dash.picker.filter_changed();

        assert_eq!(dash.picker.visible().len(), 1);
        assert_eq!(dash.picker.selected().unwrap().name, "review");
    }

    #[test]
    fn form_view_renders_fields_and_the_error() {
        let mut dash = SwarmDashboardState::new();
        let mut form = SwarmForm::new(
            row(),
            inspection(vec![workflow("w", vec![param("gate", None)])]),
            folders(),
            0,
        );
        form.error = Some("gate is required".to_string());
        dash.form = Some(form);
        dash.view = SwarmView::Form;

        let screen = render_rows(&mut dash, 60, 24).join("\n");
        assert!(screen.contains("audit"), "{screen}");
        assert!(screen.contains("Prompt *"), "{screen}");
        assert!(screen.contains("gate (model) *"), "{screen}");
        assert!(screen.contains("OUT_DIR"), "{screen}");
        assert!(screen.contains("Run"), "{screen}");
        assert!(screen.contains("gate is required"), "{screen}");
    }

    fn prompt_form() -> SwarmDashboardState {
        let mut dash = SwarmDashboardState::new();
        dash.form = Some(SwarmForm::new(
            row(),
            inspection(vec![workflow("w", vec![])]),
            vec![("a".to_string(), PathBuf::from("/a"))],
            0,
        ));
        dash.view = SwarmView::Form;
        dash
    }

    fn prompt_of(dash: &mut SwarmDashboardState) -> &mut PromptInput {
        dash.form.as_mut().unwrap().prompt_mut().unwrap()
    }

    #[test]
    fn the_prompt_is_a_box_with_a_row_per_line() {
        let mut dash = prompt_form();
        prompt_of(&mut dash).insert_str("first line\nsecond line");

        let screen = render_rows(&mut dash, 60, 24);

        let first = screen.iter().position(|r| r.contains("> first line")).unwrap();
        assert!(screen[first + 1].contains("second line"), "{screen:#?}");
        assert!(!screen.join("").contains('↵'), "newlines are real rows");
        assert_eq!(prompt_of(&mut dash).area.unwrap().height, 3);
    }

    #[test]
    fn the_user_sized_prompt_box_overrides_fitting_within_the_limit() {
        let mut dash = prompt_form();
        dash.prompt_rows = 7;
        render_rows(&mut dash, 60, 24);
        assert_eq!(prompt_of(&mut dash).area.unwrap().height, 7);

        dash.prompt_rows = 30;
        render_rows(&mut dash, 60, 24);
        assert_eq!(prompt_of(&mut dash).area.unwrap().height, 12, "half the panel");
    }

    #[test]
    fn the_completion_popup_is_drawn_with_the_prompt_focused() {
        let mut dash = prompt_form();
        let prompt = prompt_of(&mut dash);
        prompt.insert_str("$li");
        prompt.autocomplete.matches = vec!["$lint".to_string(), "$links".to_string()];

        let screen = render_rows(&mut dash, 60, 24).join("\n");

        assert!(screen.contains("$lint"), "{screen}");
        assert!(screen.contains("$links"), "{screen}");
    }

    #[test]
    fn a_script_without_a_prompt_says_so_instead_of_showing_a_box() {
        let mut dash = prompt_form();
        assert!(!render_rows(&mut dash, 80, 24).join("\n").contains("takes no prompt"));

        let mut info = inspection(vec![workflow("w", vec![])]);
        info.accepts_prompt = false;
        info.requires_prompt = false;
        dash.form = Some(SwarmForm::new(row(), info, folders(), 0));

        let screen = render_rows(&mut dash, 80, 24).join("\n");
        assert!(screen.contains("takes no prompt"), "{screen}");
        assert!(!screen.contains("Prompt"), "{screen}");
    }

    fn form_with_file_vars() -> SwarmForm {
        let mut info = inspection(vec![workflow("w", vec![])]);
        info.vars = vec![
            VarInspection {
                name: "BRIEF".into(),
                default: Some("docs/brief.md".into()),
                kind: VarKind::File,
            },
            VarInspection {
                name: "NEEDED".into(),
                default: None,
                kind: VarKind::File,
            },
        ];
        SwarmForm::new(row(), info, folders(), 0)
    }

    #[test]
    fn file_vars_are_always_passed_and_one_without_a_default_is_required() {
        let mut form = form_with_file_vars();
        type_into(&mut form, "Prompt", "p");

        assert_eq!(form.collect().unwrap_err(), "NEEDED is required");

        type_into(&mut form, "NEEDED", " a.md ");
        let values = form.collect().unwrap();
        assert_eq!(
            values.vars,
            vec![
                ("BRIEF".to_string(), "docs/brief.md".to_string()),
                ("NEEDED".to_string(), "a.md".to_string()),
            ],
            "a file stays passed at its default, trimmed"
        );
    }

    #[test]
    fn a_file_field_is_labelled_and_draws_its_popup() {
        let mut dash = SwarmDashboardState::new();
        let mut form = form_with_file_vars();
        focus_field(&mut form, "NEEDED");
        form.focused_input().unwrap().insert_str("mai");
        form.update_completion();
        dash.form = Some(form);
        dash.view = SwarmView::Form;

        let closed = render_rows(&mut dash, 60, 24).join("\n");
        assert!(closed.contains("NEEDED (file) *"), "{closed}");
        assert!(closed.contains("BRIEF (file)"), "{closed}");
        assert!(!closed.contains("src/main.rs"), "no matches, no popup: {closed}");

        dash.form.as_mut().unwrap().completion.matches = vec!["src/main.rs".to_string()];
        let open = render_rows(&mut dash, 60, 24).join("\n");
        assert!(open.contains(" src/main.rs"), "{open}");
    }

    #[test]
    fn a_dir_var_completes_and_is_otherwise_a_string() {
        let mut info = inspection(vec![workflow("w", vec![])]);
        info.vars = vec![VarInspection {
            name: "SRC".into(),
            default: Some("src".into()),
            kind: VarKind::Dir,
        }];
        let mut form = SwarmForm::new(row(), info, folders(), 0);
        type_into(&mut form, "Prompt", "p");
        assert!(form.collect().unwrap().vars.is_empty(), "the default stays the script's");

        type_into(&mut form, "SRC", "lib");
        assert_eq!(
            form.collect().unwrap().vars,
            vec![("SRC".to_string(), "lib".to_string())]
        );

        focus_field(&mut form, "SRC");
        form.update_completion();
        assert!(form.completion.active, "completes like a file field");

        let mut dash = SwarmDashboardState::new();
        form.completion.reset();
        dash.form = Some(form);
        dash.view = SwarmView::Form;
        let screen = render_rows(&mut dash, 60, 24).join("\n");
        assert!(screen.contains("SRC (folder)"), "{screen}");
    }

    #[test]
    fn a_long_completion_shows_eight_rows_and_a_bar() {
        let mut dash = SwarmDashboardState::new();
        let mut form = form_with_file_vars();
        focus_field(&mut form, "NEEDED");
        form.update_completion();
        form.completion.matches = (0..12).map(|i| format!("path{i:02}.md")).collect();
        form.completion.move_selection(11);
        dash.form = Some(form);
        dash.view = SwarmView::Form;

        let screen = render_rows(&mut dash, 60, 30);
        let shown = screen.iter().filter(|r| r.contains("path")).count();

        assert_eq!(shown, 8, "{screen:#?}");
        assert!(screen.join("\n").contains("path11.md"), "{screen:#?}");
        assert!(!screen.join("\n").contains("path03.md"), "{screen:#?}");
        assert!(screen.iter().any(|r| r.contains('█')), "{screen:#?}");
    }

    #[test]
    fn moving_off_a_field_closes_its_popup() {
        let mut form = form_with_file_vars();
        focus_field(&mut form, "BRIEF");
        form.update_completion();
        assert!(form.completion.active);

        form.focus_next();

        assert!(!form.completion.active);
    }

    #[test]
    fn parse_diff_classifies_lines() {
        let lines = parse_diff("--- a\n+++ b\n@@ -1 +1 @@\n-old\n+new\n same");
        assert_eq!(lines[3].0, DiffLineKind::Removed);
        assert_eq!(lines[4].0, DiffLineKind::Added);
        assert_eq!(lines[5].0, DiffLineKind::Context);
        assert_eq!(lines[0].0, DiffLineKind::Header);
    }
}
