//! Workspace-scoped memory services bootstrap.
//!
//! `MemoryServices` is the single bundle of long-lived memory-subsystem
//! resources opened once at workspace-bootstrap time:
//!
//! - the multi-DB [`MemoryStores`] registry (global / workspace / folder),
//! - the [`WriterHandle`] backed by a single-consumer writer task,
//! - an optional [`ConsolidationLlm`] used by S3 extraction and B5
//!   session consolidation.
//!
//! Why a sibling struct, not a field on `Workspace`: `Workspace` is a
//! `Clone + Debug` config value that callers re-instantiate freely
//! (every CLI subcommand, every test). Embedding async services on it
//! would silently spawn duplicate writer tasks and break `Clone`. The
//! plan (Tier S / S2) explicitly contemplated either approach; the
//! deep-dive on [`crate::workspace::Workspace`] settled it as a
//! sibling struct.
//!
//! Tier S / S2 wiring: this is the one canonical place that calls
//! [`spawn_writer_task`]. TUI bootstrap, CLI subcommands, and tests
//! all go through `MemoryServices::open` — no other call site of
//! `spawn_writer_task` is permitted.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use super::consolidation_llm::ConsolidationLlm;
use super::observer::{ManifestObserver, MemoryObserver};
use super::stores::MemoryStores;
use super::writer::{WriterConfig, WriterHandle, spawn_writer_task};
use crate::workspace::Workspace;

/// Caller-supplied wiring for [`MemoryServices::open`].
///
/// Defaults (`ServicesOpts::default()`) give a headless, no-LLM,
/// no-observer setup suitable for the CLI and tests. The TUI fills
/// `llm` (so S3 extraction works) and both observers (so the memory
/// panel refreshes live).
#[derive(Default)]
pub struct ServicesOpts {
    /// Embedder name resolution. Empty / `None` lets the writer fall
    /// back to `GAVIERO_EMBEDDER_MODEL` env var, then the legacy default.
    pub embedder_name: Option<String>,
    /// LLM used by the writer task for `TurnComplete` extraction (S3)
    /// and `SessionConsolidate` (B5). `None` keeps the writer alive
    /// but routes those messages through the documented fallbacks.
    pub llm: Option<Arc<dyn ConsolidationLlm>>,
    /// Memory write lifecycle observer (enqueue / commit / fail).
    pub observer: Option<Arc<dyn MemoryObserver>>,
    /// Manifest persistence observer for the A4 panel.
    pub manifest_observer: Option<Arc<dyn ManifestObserver>>,
}

/// Bundled long-lived services for the memory subsystem.
///
/// Cheap to clone via `Arc`. Construct once at workspace-bootstrap time
/// and share the `Arc<MemoryServices>` across the TUI controller, CLI
/// subcommands, and any future headless callers.
pub struct MemoryServices {
    pub stores: Arc<MemoryStores>,
    pub writer: WriterHandle,
    pub llm: Option<Arc<dyn ConsolidationLlm>>,
}

impl MemoryServices {
    /// Open the multi-DB registry against `workspace_root` + the
    /// resolved `Workspace` settings, then spawn the writer task.
    ///
    /// **Blocking**: builds the embedder ONNX session inside
    /// `MemoryStores::open`. Wrap the call in
    /// `tokio::task::spawn_blocking` from async contexts when the
    /// startup latency matters.
    pub fn open(
        workspace_root: &Path,
        workspace: &Workspace,
        opts: ServicesOpts,
    ) -> Result<Arc<Self>> {
        let embedder_name = opts
            .embedder_name
            .unwrap_or_else(|| std::env::var("GAVIERO_EMBEDDER_MODEL").unwrap_or_default());
        let stores = MemoryStores::open(workspace_root, workspace, &embedder_name)?;

        let writer = spawn_writer_task(WriterConfig {
            stores: stores.clone(),
            llm: opts.llm.clone(),
            observer: opts.observer,
            manifest_observer: opts.manifest_observer,
        });

        Ok(Arc::new(Self {
            stores,
            writer,
            llm: opts.llm,
        }))
    }

    /// Spawn the writer task against an already-opened `MemoryStores`
    /// registry. Used by the TUI bootstrap, which opens the stores
    /// off-thread and signals readiness via `Event::MemoryReady` —
    /// the controller then wraps them through this helper rather than
    /// re-opening.
    ///
    /// New code that hasn't yet opened the stores should prefer
    /// [`MemoryServices::open`].
    pub fn from_stores(stores: Arc<MemoryStores>, opts: ServicesOpts) -> Arc<Self> {
        let writer = spawn_writer_task(WriterConfig {
            stores: stores.clone(),
            llm: opts.llm.clone(),
            observer: opts.observer,
            manifest_observer: opts.manifest_observer,
        });
        Arc::new(Self {
            stores,
            writer,
            llm: opts.llm,
        })
    }

    /// Test helper — wires an in-memory `MemoryStores` registry to a
    /// writer task with no LLM and no observers. Returns the live
    /// services bundle so tests can enqueue messages and inspect the
    /// stores directly.
    /// Test helper. Not feature-gated because integration tests in
    /// other crates need it; it depends only on `NullEmbedder` and
    /// `MemoryStores::for_tests_in_memory`, both already public.
    pub fn for_tests_in_memory() -> Result<Arc<Self>> {
        use super::embedder::{Embedder, NullEmbedder};
        let embedder: Arc<dyn Embedder> = Arc::new(NullEmbedder::new(16));
        let stores = MemoryStores::for_tests_in_memory(embedder)?;
        let writer = spawn_writer_task(WriterConfig {
            stores: stores.clone(),
            llm: None,
            observer: None,
            manifest_observer: None,
        });
        Ok(Arc::new(Self {
            stores,
            writer,
            llm: None,
        }))
    }
}
