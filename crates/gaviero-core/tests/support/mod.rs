//! Shared test support for E2E and testbed integration tests.
//!
//! Cargo treats every file under `tests/` as a separate test binary,
//! so harness types cannot be private to a single file and re-imported.
//! This module is `mod support;` from each integration test file that
//! needs the harness; cargo wires it as a sibling module per binary.
//!
//! Submodule layout:
//!
//! - [`env`]          — workspace bootstrap (`E2eEnv`), write-counter
//!                      observer, capturing ACP observer, single-turn
//!                      driver `run_one_claude_turn`, diagnostic
//!                      `TestReport` / `ReportGuard`, and shared
//!                      timing constants.
//! - [`prompt_capture`] — `RecordingPromptObserver` sink for T1.1's
//!                      `PromptObserver` trait, keyed by turn id with a
//!                      "current turn" fallback for callers that don't
//!                      thread `AgentOptions::turn_id`.
//! - [`classifier`]   — heuristic prompt-section splitter that maps a
//!                      `&str` blob to a `PromptDigest` keyed by
//!                      `SectionKind` (`UserMessage`, `MemorySelections`,
//!                      `GraphSelections`, `FileRefs`, `ReplayHistory`,
//!                      `Wrapper`, `Other`).
//! - [`orchestrator`] — `run_turn` single-turn driver that wires a
//!                      `RecordingPromptObserver` and returns a
//!                      `TurnOutcome`. T1.5 will extend this with the
//!                      `Step` enum and `run_parallel` driver.

#![allow(dead_code)]

pub mod classifier;
pub mod env;
pub mod orchestrator;
pub mod prompt_capture;
