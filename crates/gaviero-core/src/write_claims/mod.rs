//! Write claims — a machine-local journal of which gaviero writer wrote
//! which file, so concurrent writers in one tree (two TUI chat turns, a
//! `gaviero-cli` run, the TUI SWARM panel, the editor) each review only
//! their own changes.
//!
//! **Layout.** One directory per machine: `$GAVIERO_CLAIMS_DIR`, else
//! `<runtime dir>/gaviero/claims` on Unix (beside the MCP sockets, `0700`)
//! or `%LOCALAPPDATA%\gaviero\claims` on Windows — never inside a workspace
//! (sync clients). Each writer owns two files:
//! - `<id>.writer.json` — [`WriterInfo`]: kind, label, pid, roots, start and
//!   (once finished) end;
//! - `<id>.claims.ndjson` — one [`Claim`] per line, appended: path key,
//!   before-state, after-hash, time.
//!
//! A writer that died without finishing ends at its files' last mtime;
//! writers past [`CLAIM_TTL_NS`] are collected on the next registration.
//!
//! **Decision rule** ([`JournalSnapshot::classify`]). A turn's change is left
//! out of its review only when another writer `W` claimed the path within
//! the turn's window, `W`'s latest claim's after-hash equals the file's
//! current content, this turn made no claim on the path, and the drop is
//! proven: `W` is the editor, or `W` started no later than the turn, or the
//! claim records a before-hash different from its after-hash (so a writer
//! started mid-turn cannot hide a file by claiming a no-op write). Both
//! claimed → kept, overlap-marked. Unclaimed while other agent writers ran
//! over the path → kept, tagged with their labels.
//!
//! **Known gaps** (the files stay in reviews, tagged where possible): Bash,
//! formatter and other shell-command writes; swarm `codex exec` direct
//! writes in MCP-bypass mode; swarm tool-agent claims that arrive only at the
//! end of the agent's turn (`PathsModified`); swarm `agy:`/`dsh:` native
//! edits, which have no before-hash; `revert_swarm`'s `git reset --hard`;
//! verification and loop-gate shell commands.

mod attribution;
mod journal;
mod liveness;
mod run;

pub use attribution::{JournalSnapshot, Verdict, WriterRecord, proves_change};
pub use journal::{
    Before, CLAIM_TTL_NS, CLAIMS_DIR_ENV, Claim, ClaimHandle, ClaimJournal, ClaimKeyCache,
    ClaimWriter, FileWrite, WriterInfo, WriterKind, claim_key,
};
pub use run::{RunClaims, SwarmClaimConfig};

#[cfg(test)]
mod tests;
