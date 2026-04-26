# Tier C Part 2 — Typed Storage and Forgetting

## Your role

You are a senior Rust systems architect producing a **detailed implementation plan** for a specific phase of improvements to **Gaviero**, a local-first AI coding-agent system. You are writing this plan for the sole developer of Gaviero to review and execute.

Your output is a plan, not code. Be concrete about crates, modules, schemas, migrations, task ordering, and acceptance criteria. Flag decisions that require human judgment. Do not produce full Rust code listings — high-level signatures, type skeletons, and data-flow descriptions are acceptable.

Assume the reader knows Rust, tokio, SQLite, sqlite-vec, ONNX Runtime, tree-sitter, petgraph, zstd compression, and the general design patterns of audit trails and append-only logs. Do not explain these.

**This document covers Phase 2 of Tier C**, containing two items (C1, C2) focused on making the memory schema reflect the History/Memory/Scratchpad lifecycle and introducing first-class forgetting with audit. Phase 1 of Tier C (C5 pluggable traits, C3 node-specificity in PageRank, C4 typed graph edges) is covered in a separate document (**Tier C Part 1 — Graph and Pluggable Traits**) and is a **prerequisite** — specifically, C5's trait formalization is easier to build on when the schema migration (C1) isn't also in flight. Treat Phase 2 as self-contained for planning, but do not start implementation until Phase 1 has landed.

---

## Project context: Gaviero

### What Gaviero is

Gaviero is a Rust 2024 AI-powered coding agent with two binaries: `gaviero` (ratatui TUI editor) and `gaviero-cli` (headless runner). It orchestrates Claude Code (ACP), Codex, Ollama, and mock backends through a provider-agnostic `AgentBackend`. Agent writes flow through a `WriteGatePipeline`. Swarm orchestration runs a 6-phase pipeline.

Local-first, single-developer-oriented.

### Stack

- Rust 2024, tokio, petgraph, ropey, git2, ratatui 0.30
- SQLite (WAL mode) + sqlite-vec
- ONNX Runtime (`ort`) with `gte-modernbert-base` embedder + `gte-reranker-modernbert-base` reranker (Tier B1/B2)
- tree-sitter 0.25, 16-language registry
- zstd via the `zstd` crate for History compression (added in this phase)
- logos + chumsky + miette for DSL

### Crate layout

- `gaviero-core` (lib) — all runtime, no UI
- `gaviero-tui`, `gaviero-cli`, `gaviero-dsl`, `tree-sitter-gaviero`

### Memory subsystem state (assuming Tier S, A, B, and Tier C Phase 1 are in place)

- 5-level scope: `Global → Workspace → Repo → Module → Run`
- sqlite-vec partitioned by `scope_level`; FTS5 for lexical search
- **Merged multi-scope retrieval** (Tier B3) — no early-exit; scope bias via multipliers
- **Cross-encoder reranker** in retrieval pipeline (Tier B2)
- **`gte-modernbert-base` embedder** (Tier B1)
- Composite score: `(sim*0.5 + importance*0.2 + recency*0.15 + 0.15) * scope_multiplier * trust`, blended with reranker
- **Decay floor at 0.35; type-based exemptions** for decision/convention/invariant/preference (Tier B4)
- SHA-256 + semantic near-dup dedup on writes
- **Writer task owns all writes** (Tier S2) — this phase extends the dispatch to include `memory_kind`
- **Retrieval manifest persistence** — every injection produces an `injection_manifests` row (Tier S4); manifests reference turn transcripts, which after C1 live as immutable history rows
- **Three-cadence consolidation**: per-turn extractor (S3), per-session consolidator (B5), sleeptime pass (B5) — sleeptime extends in this phase to compress old history
- **Retrieval-use telemetry** feeding sleeptime trust re-scoring (Tier B6) — writes to its own `retrieval_use` table, not part of `memory_kind` system
- **Annotations sidecar** parsed from LLM output (Tier A1)
- **`/remember` scope-corrected and visible** (Tier A2)
- **`source` and `trust` attributes** populate on all writes (Tier A3)
- **TUI memory panel** for inspection and curation with bulk delete, pin, scope-change, manifest inspection (Tier A4) — extended in this phase with per-kind tabs
- **Gaviero-as-MCP-server** with three read-only tools: `memory_search`, `blast_radius`, `node_doc` (Tier A5) — `memory_search` gains a `kind` parameter in this phase
- **`Embedder` and `Reranker` traits formalized** (Tier C5)
- **Specificity-weighted PageRank** on the code graph (Tier C3)
- **Typed graph edges** with intent-based weighting (Tier C4)

### Conceptual framing

Gaviero's memory follows the **History / Memory / Scratchpad** lifecycle — and Phase 2 of Tier C is the tier that **makes this framing explicit in the schema**. C1 promotes the distinction to first-class by introducing three memory kinds (record, history, summary) with different mutability, retention, and injection policies. History becomes immutable append-only (the source of truth for every derived record and manifest); Records and Summaries are the derived, mutable Memory layer. Scratchpad remains outside the memory system proper (handled by swarm's discovery board for multi-agent work; not needed for chat). C2 then provides the audited forgetting surface that the now-asymmetric mutability landscape requires.

### Hard constraints (non-negotiable)

1. All agent writes flow through `WriteGatePipeline`.
2. `git2` only.
3. Tree-sitter for syntax work.
4. `MemoryStore` behind `tokio::sync::Mutex`; embedding outside lock.
5. **No Mutex across `await`, parse, or `fs` I/O.**
6. `gaviero-core` has no UI/CLI types.
7. Single TUI `mpsc::UnboundedReceiver<Event>` main loop.
8. Provider-agnostic `AgentBackend`.
9. Config via `settings.json` cascade.
10. Memory writes require explicit `WriteScope`.
11. **All memory writes go through the Tier S2 writer task.**
12. **MCP surface is read-only.**

### Decisions already taken (do not re-litigate)

- Writer task architecture; read-only MCP; annotations sidecar; three-cadence consolidation.
- No LLM writes; no graph-RAG / Neo4j / A-MEM memory evolution.
- `gte-modernbert-base` is the embedder; cross-encoder reranker enabled.
- Pluggability for embedder/reranker formalized in C5; MCP `blast_radius` gains `mode` parameter in C4.
- **History is immutable append-only** — decision made during the paper-review revision of the original "90-day-retention Episodes" design; compression (not pruning) handles storage. This design decision is final; the plan executes it, does not revisit it.
- **Telemetry (Tier B6) stays in its own purpose-specific table**, not as a `memory_kind` value.

---

## Phase 2 context and rationale

Phase 2 of Tier C is **the storage layer's coming-of-age moment**. It finally makes the conceptual History/Memory/Scratchpad distinction structurally enforced rather than a documentation convention, and it introduces the auditable forgetting operations that a mature memory system requires.

- **C1 (typed memory stores split).** Separate memory into Records (structured facts), History (immutable append-only transcripts, compressed-not-pruned), Summaries (consolidator output). Different retention, injection, mutability, and dedup policies per kind. The largest item in Phase 2 — a schema migration affecting every write path. Introduces SQL-enforced immutability on History rows. Adds zstd compression for old History.
- **C2 (`/forget` with audit trail).** First-class forgetting with N-day audit retention. Table stakes for mature memory systems; also a prerequisite for safe automated near-dup merge at scale (Tier B5's merges become reversible through the audit table). Includes the special-case `/forget-history` tombstone path for the rare legitimate redaction need on otherwise-immutable History rows.

**Dependencies and ordering within Phase 2:**

- **C1 ships first.** C2 needs the `memory_kind` discriminator to provide kind-scoped forgetting semantics; C2's special-cased `/forget-history` only makes sense after History is distinguishable from other kinds.
- **C2 depends on C1** for type-scoped deletion semantics and for the immutability invariant that `/forget-history` must explicitly override.

**Recommended sequence:** C1 → C2.

**All Phase 2 items must continue to satisfy the evaluation gates from Tier B.** Tier 1 retrieval smoke test runs pre/post; no regression allowed. C1 specifically must run Tier 2 eval before and after the schema migration to confirm no retrieval-quality regression from the `kind = record` default injection filter.

---

## Items in this phase

### C1. Typed memory stores (Records / History / Summaries split)

**Problem.** The current `memory` table conflates three distinct content classes with different retention, injection, and consolidation semantics. The split matters because each class has a different *role in the system*, not just different content:

- **Records.** Structured high-value facts (decisions, lessons, errors, conventions, gotchas, invariants, preferences) derived from interactions. Long retention. Injected into chat context. Aggressive dedup. The workhorse.
- **History.** Raw interaction log — full turn transcripts as they occurred. **Immutable append-only; the source of truth.** Never injected. Never deleted. Compressed (not pruned) after retention window to bound storage.
- **Summaries.** Session-level consolidator outputs (produced in Tier B5). AI-oriented narrative text describing what happened in a session. Injected as "session context" when topically relevant. Semantic dedup (summaries of similar sessions may merge).

**Why immutable History (and not 90-day-retention Episodes as originally planned).** A design reflection from the `arxiv.org/abs/2512.05470` file-system-abstraction paper: derived Memory (Records, Summaries) must trace back to a source. If we prune transcripts after 90 days, every extractor output older than 90 days becomes unauditable — you can't replay extraction with a new prompt, can't evaluate extractor drift, can't reconstruct why a memory exists. For a single-dev local-first tool, the storage pragmatism that originally justified pruning doesn't hold: ~50 turns/day × 5 KB/turn × 365 days ≈ 90 MB/year/workspace, trivial on modern disks. Compression (zstd on old sessions) handles the tail; true pruning is not necessary. This decision is final; the plan executes it.

**Architecture.**

- **Logical separation, not necessarily physical.** Option A: three tables. Option B: one table with a strong `memory_kind` discriminator (`record | history | summary`) and per-kind config. Option A is cleaner long-term; Option B is a much smaller migration. Plan should analyze and recommend.
- Recommended: **Option B with views**, migrating to Option A if cross-type concerns prove rare. Specifically:
  - Add `memory_kind ENUM NOT NULL` column to the memory table (migration: all existing rows default to `record`).
  - Create SQL views `v_records`, `v_history`, `v_summaries` for convenient per-kind queries.
  - Per-kind config lives in settings under `memory.kinds.record.*`, `memory.kinds.history.*`, `memory.kinds.summary.*`.
- **Per-kind policies:**
  - **Record:** `retention: unlimited`, `injectIntoChat: true`, `dedup: sha256+cosine0.95`, `decayExempt: types from B4`, `mutable: true` (can be edited, merged, superseded, deleted).
  - **History:** `retention: unlimited`, `injectIntoChat: false`, `dedup: none`, `decayExempt: true` (not applicable — not injected), `mutable: false`. **Append-only.** The audit trail is load-bearing: if a History row is ever modified after write, the audit is compromised. Enforce with: **SQL trigger rejecting UPDATE/DELETE on any row where `memory_kind = 'history'`** (the trigger fires on the base table, not the view — views alone are not sufficient enforcement); writer task never enqueues `PanelEdit { op: Delete }` for history-kind rows; TUI panel's History tab is read-only (no `d`/`e`/`p` keys).
  - **Summary:** `retention: 365d` (then soft-delete per C2 audit, not hard-delete), `injectIntoChat: conditional` (retrieval considers summaries only for queries matching `session_thread` topics), `dedup: cosine0.90`, `decayExempt: false`, `mutable: true`.
- **Compression for History.** History rows older than `compressAfterDays` (default 90) are opportunistically compressed by the sleeptime pass (Tier B5):
  - Full transcript text column moves from plain TEXT to a zstd-compressed BLOB with a `compressed: bool` flag.
  - Metadata fields (session_id, turn_id, timestamp, refs, scope) stay uncompressed for indexability.
  - On read, decompress transparently. FTS5 index over compressed rows is rebuilt at compression time from decompressed text; FTS entries themselves are not compressed (needed for search).
  - Compression is opportunistic: if disk pressure is low and user hasn't asked for it, History can stay uncompressed indefinitely without harm.
  - **Compression preserves SHA-256 hash of original text for verification.** On decompress, recompute the SHA and compare; mismatch triggers a data-integrity alarm. Never delete the uncompressed row until the compressed row is verified readable and the SHA matches.
- Writer task already dispatches by source (Tier A3) — now also dispatches by `memory_kind`. Producers specify kind:
  - `UserRemember` → `record`
  - `TurnComplete` (extractor output) → `record`; but the *raw transcript* is written as `history` (immutable)
  - `SessionConsolidate` output → summary (kind=`summary`); the per-item operations still produce `record`s
  - `SwarmConsolidate` → `record`s and optionally a summary
  - Panel edits → whatever kind is being edited, but **panel cannot edit history rows**
- Injection pipeline (chat auto-inject in Tier S1, MCP `memory_search` in Tier A5) by default retrieves `record`s only. Summaries are retrieved when a `session_thread` topic matches. History is **never** injected by default (it's audit data, not agent context). Add a `memory_kind` filter param to `memory_search` MCP tool; default: `record`; explicit `any` allowed but discouraged.
- **History content.** Every `TurnComplete` (from Tier S3) now *also* writes a history row containing the full transcript + system prompt snapshot + injected-memory IDs (cross-ref to S4 manifest). This is the complete provenance bundle for "what actually happened in turn X." Carries `session_id`, `turn_id`, and `manifest_id` for navigation.
- **Provenance links.** Each record's `refs` field can now point at history rows by `history_id`, not just `session_id/turn_id`. This gives records a durable pointer back to their source transcript. The Tier S3 extractor and Tier A1 annotations populate `refs: [history_id]` on every record they produce.

**Benefits.**

- **Provenance is unbreakable.** Every derived record traces back to an immutable raw transcript. If an extracted record looks wrong in 6 months, you can find the exact turn that produced it and re-run the extractor.
- **Policy per class is explicit.** Records get aggressive dedup and long retention; history gets append-only discipline; summaries get semantic merge.
- **Conceptual alignment with established memory lifecycle.** History is the log; Memory is the index. Matches the LLM-as-OS paradigm cleanly.
- **Storage bounded without sacrificing audit.** Compression handles the storage-size concern that originally motivated pruning.
- **Enables retrospective re-extraction.** Swap extractor prompt, re-run on last 90 days of history, compare output. Today impossible.
- **Unblocks safer sleeptime operations.** Once merges are reversible through audit (C2) and immutable transcripts ground every derived row (C1), sleeptime can afford to be more aggressive about near-dup cleanup without user anxiety.

**Risks.**

- **Schema migration on active `memory.db`.** All existing rows default to `record`. Test migration against populated fixture. Because no existing rows are history-kind, the immutability enforcement only constrains new writes — no legacy concern.
- **History storage growth (before compression).** ~250 KB/day of History for a moderately-active session. At 90 days uncompressed + compressed beyond that, total is roughly 50–200 MB/workspace after 1 year. Acceptable but monitor.
- **Compression correctness risk.** A bug in the compress/decompress path silently corrupts historical transcripts. Mitigate: SHA-256 verification on decompress; never delete the uncompressed row until the compressed row is verified readable.
- **Immutability feels heavy for dev use.** Tempting to "just delete that embarrassing debug session." Resist: audit requires discipline. Provide `/forget-history <turn_id>` as an **explicit, confirm-twice** operation (implemented in C2) that writes a tombstone (not a true deletion — a "user-requested redaction" row replacing the transcript with a hash + timestamp) for the rare legitimate case.
- **Over-engineering if users don't need per-type policy.** Mitigate by keeping Option B (discriminator + views), avoiding Option A (separate tables).
- **Retrieval complexity.** If by-default filter is `kind=record`, users/LLMs must know to override for history/summary queries. Document in MCP tool schemas.
- **SQL trigger complexity.** The UPDATE/DELETE rejection trigger must fire before any legitimate-looking edit path (including sqlite-vec virtual table operations that might cascade). Test the trigger fires on: `UPDATE memory SET text = ... WHERE memory_kind = 'history'`, `DELETE FROM memory WHERE memory_kind = 'history'`, cascades from related tables. Test it does *not* fire on: legitimate writes to other kinds, reads, vec-index updates that don't touch the history text itself.

**Alternatives considered and rejected.**

- *Keep single table with type column (current after A3), retain Episodes with 90-day pruning.* The original plan. Loses audit discipline; makes the Tier S4 manifest system only partially useful because old manifests can point at deleted transcripts.
- *Three physical tables (Option A).* More invasive migration, harder cross-type queries. Revisit if Option B becomes unwieldy.
- *Separate databases per kind.* Excessive isolation; complicates transactions.
- *Full event-sourcing with CRDT-ish derived views.* Cleaner theoretically, massively overbuilt for a single-dev tool.
- *Application-layer immutability enforcement only (no SQL trigger).* A bug or future-contributor shortcut in the writer task silently breaks the invariant. Belt-and-braces: SQL trigger + writer-task discipline + TUI read-only.

**Integration notes.**

- The TUI memory panel (Tier A4) gains tabs or filters for Records / History / Summaries. Default tab: Records. History tab is chronological and read-only (view-only, with an "export turn" action). Summaries tab is grouped by `session_id`.
- MCP `memory_search` gains an optional `kind` parameter; default `"record"`; values `"record" | "history" | "summary" | "any"`. Schema change to the tool contract — document clearly for subprocess-agent users.
- Writer task's `TurnComplete` handler now produces multiple writes per turn: up to 5 records (from extractor + annotations), plus 1 history row (the transcript with provenance), plus 1 manifest row (from S4). The history write is independent of record writes — isolate the failure mode so a history-write failure doesn't lose records and vice versa.
- Sleeptime pass (Tier B5) gains per-kind sweep: compress old history, semantic-merge summaries, do record hygiene as today. **Sleeptime never deletes history rows.**
- The B6 retrieval-use telemetry stays in its own `retrieval_use` table, not as a `memory_kind` value.
- Settings:
  - `memory.kinds.record.retentionDays: Option<u32>` (default null = unlimited)
  - `memory.kinds.record.injectIntoChat: bool` (default true)
  - `memory.kinds.record.dedupCosineThreshold: f32` (default 0.95)
  - `memory.kinds.history.injectIntoChat: bool` (default false; should almost never be true)
  - `memory.kinds.history.compressAfterDays: u32` (default 90)
  - `memory.kinds.history.compressionAlgorithm: "zstd" | "none"` (default `"zstd"`)
  - `memory.kinds.summary.retentionDays: u32` (default 365)
  - `memory.kinds.summary.injectIntoChat: bool` (default true, topic-matched)

---

### C2. `/forget` command with audit trail

**Problem.** Gaviero has no first-class deletion surface. The TUI panel (Tier A4) supports per-row delete, but there's no bulk, no filter-based, and no audit. When LLM-extracted memory accumulates something wrong — an invariant that no longer holds, a gotcha that was misstated — users need a surgical way to clean up without opening SQLite by hand. Also, the sleeptime pass (B5) does near-dup merges that are destructive; without audit, these are irreversible. And after C1, History is immutable by default — which is correct — but occasional legitimate redaction needs a guarded escape hatch.

**Architecture.**

- **Command surface (operates on records and summaries by default; never on history):**
  - TUI slash command: `/forget <query>` — fuzzy-matches against memory text; shows candidate list; user selects with space, confirms with Enter.
  - TUI command: `/forget-scope <scope>` — forgets all memories at given scope (with confirmation count).
  - TUI command: `/forget-type <type>` — forgets all memories of given type.
  - TUI command: `/forget-source <source>` — forgets all memories from given source (e.g., `/forget-source llm_extracted` for a factory reset of LLM extractions).
  - Panel: per-row `d` (already exists in Tier A4); additionally `Shift+d` for bulk-select + delete.
  - CLI: `gaviero-cli memory forget --filter '...'` for scripted use.
- **Audit table:** new table `deletions` with rows for every deletion:
  ```
  (id, memory_id, memory_content_hash, memory_kind, memory_source, memory_trust,
   deleted_at, deleted_by (user_command | panel | sleeptime_merge | sleeptime_prune | user_redaction),
   reason (optional), original_row_json)
  ```
  `original_row_json` is a full serialization of the deleted row, enabling restore.
- **Soft-delete vs hard-delete:**
  - User deletions (records, summaries): soft-delete (move to `deletions` table) with `retention_days = 30` (configurable). Hard-delete after retention expires.
  - Sleeptime merges: both soft-delete the merged-away row and preserve the merge reference in `deletions.original_row_json.merged_into = <new_id>`.
  - Sleeptime summary retention-expiry (>365d): soft-delete with `retention_days = 14` (shorter window).
  - **History rows are not subject to regular `/forget`.** See C1: History is immutable and append-only — the SQL trigger from C1 enforces this at the database layer. The only path to remove content from a history row is the explicit `/forget-history <turn_id>` operation, described below.

- **The `/forget-history` tombstone path (explicit two-step-confirm redaction):**
  - **Guarded two-step confirmation:** user types `/forget-history <turn_id>`, then is prompted to type the turn_id again, then type `REDACT` literally. Three chances to back out.
  - **Does not delete the history row.** Writes a tombstone: replaces the transcript body with a marker `[REDACTED: sha=<original-sha> redacted_at=<timestamp> reason=<user-provided>]`. Metadata fields (session_id, turn_id, timestamp, manifest_id refs) are preserved. The row still exists; its content is redacted.
  - **Implementation path through the SQL trigger.** The C1 trigger rejects UPDATE/DELETE on history rows unconditionally. `/forget-history` works by executing a single privileged write operation via a **dedicated writer task variant** `WriterMessage::RedactHistory { turn_id, reason }`. The writer task handler temporarily disables the trigger within a transaction, performs the redaction UPDATE, logs to `deletions` audit, and re-enables the trigger before committing. No other code path disables the trigger; grep-enforceable.
  - **Logged in the `deletions` audit table with `deleted_by = "user_redaction"`**, including the original SHA. The audit row itself is subject to the standard 30-day retention — after 30 days, the audit row is hard-deleted and the redaction becomes permanent with no trace of the redactor or reason (only the tombstone marker survives on the history row).
  - **Is not restorable.** The `original_row_json` in the audit row deliberately stores **the redacted tombstone, not the original transcript**. Redaction is intentionally one-way — the point is that the data is gone. If a user wants reversible deletion, they shouldn't use `/forget-history`; they should use `/forget` on the derived records.
  - **Settings:** `memory.forget.allowHistoryRedaction: bool` (default `true`; workspaces under strict audit can set `false` to disable the escape hatch entirely, making History genuinely immutable).
- **Restore command (records and summaries only; never history):**
  - `/restore <id>` — restores a soft-deleted row from the audit table.
  - `/restore --since <duration>` — restores all soft-deletions since time T.
  - `gaviero-cli memory restore ...`
  - Restore runs the row through the normal dedup pipeline (cosine check against current memories); if it conflicts with a newer row, merge semantics apply.
- **Dry-run:** every `/forget` command accepts `--dry-run` to list what would be deleted without executing. `/forget-history` does *not* offer dry-run — the two-step confirmation is the dry-run.

**Benefits.**

- Makes destructive operations on records and summaries auditable and reversible within a window.
- Gives users explicit control over LLM-authored pollution.
- Unblocks aggressive sleeptime merging (Tier B5's near-dup merge becomes safer because reversible).
- Addresses a common complaint: "the AI remembered something wrong and I can't undo it."
- Provides a guarded, explicit, auditable path for the rare legitimate History redaction need without compromising the default immutability invariant.

**Risks.**

- **Audit table grows unboundedly.** Auto-prune after retention expires. Track size as a metric.
- **Restore races.** If a restored row's content_hash conflicts with a newer row, merge semantics must apply (use the same dedup pipeline as any other write).
- **Scope- or type-bulk deletions are dangerous.** Confirmation UI shows count; `--dry-run` available for scripted use. Never silently bulk-delete.
- **Trigger-disable window during `/forget-history`.** The brief window where the immutability trigger is disabled is a foot-gun. Keep the window inside a single transaction, held for the minimum code path. Never hold it across an await or a userspace call. Audit test: write a fuzzing harness that tries to smuggle non-redaction UPDATEs through the `RedactHistory` code path; assert all rejected.
- **User thinks `/forget-history` is reversible and is surprised.** Copy in the two-step prompt must be unambiguous: "This redaction cannot be undone. The transcript will be permanently replaced with a tombstone. Type REDACT to proceed." Test with a colleague before shipping.
- **Redaction undermines audit but is necessary.** The whole point of immutable History is audit — and `/forget-history` breaks that. Accept the trade-off: the tombstone preserves the fact that a redaction occurred (timestamp, SHA of original) even after the audit row is pruned. Users can turn off `/forget-history` entirely via `memory.forget.allowHistoryRedaction = false` for strict-audit workspaces.

**Alternatives considered and rejected.**

- *Soft-delete via `is_deleted` flag only, no audit table.* Simpler but loses the "what was it before" for merges and for restore.
- *No deletion, only decay.* Users with bad LLM writes will resort to `rm memory.db`; much worse.
- *External backup/restore.* Doesn't solve in-workflow corrections.
- *No `/forget-history` at all; History is genuinely immutable always.* Correct in spirit; impractical in practice. A pragmatic escape hatch is better than users learning to `sqlite3 memory.db` and bypassing the whole system. The escape hatch is audited and two-step-guarded, not hidden.
- *Tombstone that retains the original transcript in `original_row_json`.* Defeats the purpose of redaction — the data is recoverable. Store the tombstone itself.

**Integration notes.**

- `WriterMessage::PanelEdit { op: Delete(id) }` from Tier A4 is extended to variants: `Delete(id) | BulkDelete(filter) | Restore(id) | RedactHistory { turn_id, reason }`.
- The `RedactHistory` handler is the **only** code path authorized to disable the C1 immutability trigger. Code search for the trigger-disable statement should return exactly one callsite. Enforce with a CI grep check.
- Sleeptime pass (Tier B5) merge operations now go through the audit table, tagged `sleeptime_merge` as `deleted_by`.
- TUI panel gains a "Deletions" tab or section showing recent deletions with a `u` key to undo. Redactions appear in the panel's Deletions tab but have no `u` key — they're marked "permanent".
- Settings:
  - `memory.forget.userDeletionRetentionDays: u32` (default 30)
  - `memory.forget.sleeptimePruneRetentionDays: u32` (default 14)
  - `memory.forget.requireConfirmForBulk: bool` (default true)
  - `memory.forget.allowHistoryRedaction: bool` (default true)

---

## Expected output format from the implementation plan

For each of C1, C2 the plan should contain:

**1. Summary.** 3–5 sentences: goal, user-visible effect, non-goals.

**2. Dependencies and ordering.** What must be in place (from S, A, B, and Phase 1 of Tier C); sub-task order within the item. C1 specifically must call out that it runs before C2.

**3. Affected crates and modules.** Tree-style.

**4. New types, traits, public API.** For C1: `MemoryKind` enum, per-kind policy struct, writer dispatch by kind, zstd compress/decompress helpers with SHA verification. For C2: `WriterMessage` variants `BulkDelete`, `Restore`, `RedactHistory`; `deletions` table schema; restore pipeline entry point.

**5. Data-flow description.** For C1: writer task dispatch by `memory_kind`; compression pass inside sleeptime with SHA verification before old-row removal; retrieval default `kind=record` filter. For C2: per-command filter resolution → dry-run preview → confirmation → transactional soft-delete with audit row → background hard-delete after retention. Dedicated `RedactHistory` transaction showing trigger-disable-scope explicitly.

**6. Schema / settings / UI changes.** SQL migrations: add `memory_kind` column, create views, install immutability trigger, add `deletions` table. Settings.json additions (listed per-item above). TUI changes: per-kind tabs, Deletions tab, redaction two-step prompt flow.

**7. Task breakdown.** Ordered sub-tasks with IDs, size (S/M/L), dependencies. C1 breaks into: schema migration, writer dispatch by kind, immutability trigger, zstd compression pass, TUI tabs, MCP `kind` param. C2 breaks into: audit table + basic soft-delete, restore, bulk/filter commands, `/forget-history` with dedicated writer variant, sleeptime audit integration, Deletions panel tab.

**8. Test strategy.** Unit + integration + regression + **audit invariant tests**. Specifically:
- C1: SQL trigger rejects UPDATE/DELETE on history rows in all smuggling attempts; compression round-trip preserves SHA; writer task fails history-edit `PanelEdit` messages loudly; retrieval default filter correctly excludes history.
- C2: soft-delete + restore round-trip preserves all fields; bulk delete confirmation cannot be bypassed; `/forget-history` two-step confirmation cannot be bypassed; `RedactHistory` is the only caller that disables the immutability trigger (grep-enforceable CI test); after retention expires, audit rows are hard-deleted and redaction tombstones remain.
- Both: Tier 1 retrieval smoke test non-regresses after C1's `kind=record` default filter is active.

**9. Observability.** Tracing spans for per-kind writer dispatch; audit-table-size metric; compression-ratio metric; redaction counter; restore-success-rate metric. Observer event `DeletionsObserver::on_deletion(kind, count, reason)` for the panel.

**10. Risks and rollback.** Per sub-task: failure modes and reversal. For C1 migration: backup `memory.db` before adding the trigger; trigger can be dropped and re-added without data loss. For C1 compression: never remove the uncompressed row until SHA verifies on the compressed row. For C2 `/forget-history`: if the trigger-disable window leaks, all history writes are compromised — treat the implementation as a security-sensitive code review.

**11. Acceptance criteria.** Measurable, human-testable.

**12. Open questions.** Decisions requiring developer judgment. Examples: whether to ship Option A (three tables) straight away for a populated dogfooding `memory.db` or commit to Option B (one table with discriminator); what the first-run UX looks like on a pre-C1 `memory.db` (migrate silently vs prompt); whether `/forget-history`'s second-step word should be `REDACT` or something stronger; whether workspaces should be able to set `allowHistoryRedaction = false` permanently via a sealed config flag (useful for shared-team scenarios even though Gaviero is single-dev).

---

## Evaluation gates

- **Tier 1 retrieval smoke test** runs pre/post-C1 migration; no regression allowed (specifically: the default `kind = record` injection filter must not drop retrieval recall — verified by a before/after A/B on the pinned query set).
- **Tier 2 code-specific eval** runs pre/post-C1 migration; Summary retrieval when topically matched should produce equal or better Tier 2 scores on scenarios designed to exercise session-level context recall.
- **C2 audit invariant tests** must pass in CI (no bypass paths; bulk confirmations cannot be scripted through; redaction logs audit rows).
- **C1 immutability invariant test suite** must pass in CI (every attempt to UPDATE/DELETE a history row outside `RedactHistory` fails; every attempted code path that bypasses the writer task fails).
- **Compression round-trip test** on 1000 fixture transcripts: all decompress to SHA-matching originals.

---

## Explicit non-goals for Phase 2

Do **not** plan the following here:

- Retrieval pipeline changes — Tier B Phase 1
- Consolidation cadences — Tier B Phase 2
- Retrieval-use telemetry — Tier B Phase 2 (telemetry stays in its own table, not a `memory_kind`)
- Pluggable embedder/reranker traits — Phase 1 C5
- PageRank node-specificity — Phase 1 C3
- Typed graph edges — Phase 1 C4
- KG node-doc schema — Tier D1
- Contextual Retrieval — Tier D2
- AGENTS.md / CLAUDE.md compat — Tier D3
- Three-table physical split (Option A) — defer unless Option B proves unwieldy
- Full event-sourcing / CRDT — explicitly rejected
- Hiding the `/forget-history` escape hatch entirely — default remains `allowHistoryRedaction = true`

---

## Acceptance criteria for Phase 2

Phase 2 is complete when all of the following are true:

1. Every memory row has a non-null `memory_kind ∈ {record, history, summary}`. Legacy rows migrated to `record`. Per-kind policy is enforced by settings + writer-task dispatch + SQL trigger (for history).
2. Every `TurnComplete` produces: up to 5 `record` rows (from extractor + annotations), 1 `history` row (the transcript + provenance), and 1 manifest row (S4). History-write failure does not lose records and vice versa.
3. The C1 SQL trigger rejects every tested attempt to UPDATE or DELETE a history row. Verified by a CI test suite with at least 10 smuggling attempts including direct UPDATE, cascade-triggered UPDATE, and DELETE.
4. History rows older than `compressAfterDays` (default 90) are opportunistically zstd-compressed by sleeptime. SHA-256 verification passes on 100% of round-trip test fixtures. Uncompressed rows are removed only after the compressed row verifies.
5. MCP `memory_search` tool defaults to `kind = record`; accepts `history | summary | any` explicitly. Documented in the tool schema.
6. TUI memory panel (Tier A4) has per-kind tabs; History tab is read-only (no `d`/`e`/`p` keys active); Summaries tab is grouped by `session_id`.
7. `/forget`, `/forget-scope`, `/forget-type`, `/forget-source` all work, go through the audit table as soft-deletes with 30-day retention (default), support `--dry-run`, and require bulk-confirmation.
8. `/restore <id>` and `/restore --since <duration>` work, running restored rows through dedup.
9. `/forget-history <turn_id>` is the **only** code path that disables the immutability trigger, does so within a single transaction, writes a redaction tombstone preserving metadata + original SHA, logs to audit with `deleted_by = "user_redaction"`, and is non-restorable. Verified by CI grep check.
10. Sleeptime merges and prunes go through the audit table with correct `deleted_by` tags; restorable within retention window.
11. Audit table is auto-pruned after `userDeletionRetentionDays` / `sleeptimePruneRetentionDays` expires. Track audit-table size as a metric.
12. No regression in Tier S, A, B, or Tier C Phase 1 acceptance criteria.

---

## Anti-patterns to avoid

- **Mutating history rows anywhere outside the `RedactHistory` writer variant.** Application-layer discipline plus the SQL trigger is the defense — don't weaken either.
- **Pruning history rows for storage reasons.** Compress, don't prune. The 90-MB/year/workspace storage cost is the price of audit; pay it.
- **Hard-deleting from `/forget` without going through the audit table.** Every deletion is recorded first. No silent drops.
- **Exposing history content in default retrieval.** Default `kind = record` in all injection and MCP paths. Users/LLMs must explicitly opt into history retrieval.
- **Silent migration on first run.** The C1 migration is load-bearing; prompt the user on first post-upgrade start: "Gaviero's memory schema is upgrading to typed stores. Backup taken at `<path>`. Continue?"
- **Making `/forget-history` easier than it needs to be.** The two-step confirmation is a feature, not a friction. Don't add a `--force` flag.
- **Storing the original transcript in the redaction audit row.** Defeats the redaction. Tombstone only.
- **Disabling the immutability trigger "temporarily for debugging" outside the `RedactHistory` path.** Once you add a second caller, the invariant is gone. If debugging requires trigger bypass, it's a signal that the schema is wrong — stop and reconsider.
- **Auto-deleting anything in sleeptime without audit.** Every destructive sleeptime op goes through the `deletions` table.

---

## Final instruction

Produce the implementation plan per **Expected output format** above, covering C1 and C2 in the recommended order (C1 → C2). C1 is the largest item in the entire Tier C — break it into shippable sub-tasks (schema migration first, then writer dispatch by kind, then immutability trigger, then zstd compression, then TUI tabs, then MCP `kind` param). C2 should be similarly broken (audit table + soft-delete, then restore, then bulk/filter, then `/forget-history`). Call out the CI grep check for `RedactHistory` as the single trigger-disabler. Flag every decision the developer must make. If ambiguous, add to open questions rather than deciding.
