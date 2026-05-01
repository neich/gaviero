//! Sink implementation of `gaviero_core::observer::PromptObserver`.
//!
//! Tests wire `RecordingPromptObserver` into
//! `AgentOptions::prompt_observer` and read the captured events after
//! the subprocess exits. The observer fires synchronously inside
//! `AcpSession::spawn` (after the spill decision, before the spawn),
//! so events are guaranteed visible in the order spawns occur.

use std::sync::{Arc, Mutex};

use gaviero_core::observer::{PromptEvent, PromptObserver};

/// Thread-safe sink for `PromptEvent`s, keyed by `turn_id`.
///
/// The orchestrator that drives a turn is responsible for setting
/// `AgentOptions::turn_id` on the spawn. When a caller forgets, the
/// observer falls back to `current_turn` — a per-instance Mutex the
/// orchestrator updates around each spawn. Precedence: explicit
/// `turn_id` on `PromptEvent` wins; only the empty-string case is
/// rewritten to the fallback.
pub struct RecordingPromptObserver {
    events: Mutex<Vec<PromptEvent>>,
    current_turn: Mutex<String>,
}

impl RecordingPromptObserver {
    pub fn arc() -> Arc<Self> {
        Arc::new(Self {
            events: Mutex::new(Vec::new()),
            current_turn: Mutex::new(String::new()),
        })
    }

    pub fn set_current_turn(&self, id: impl Into<String>) {
        *self.current_turn.lock().unwrap() = id.into();
    }

    pub fn events(&self) -> Vec<PromptEvent> {
        self.events.lock().unwrap().clone()
    }

    pub fn events_for_turn(&self, id: &str) -> Vec<PromptEvent> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|ev| ev.turn_id == id)
            .cloned()
            .collect()
    }

    pub fn clear(&self) {
        self.events.lock().unwrap().clear();
    }
}

impl PromptObserver for RecordingPromptObserver {
    fn on_prompt(&self, mut ev: PromptEvent) {
        if ev.turn_id.is_empty() {
            ev.turn_id = self.current_turn.lock().unwrap().clone();
        }
        self.events.lock().unwrap().push(ev);
    }
}
