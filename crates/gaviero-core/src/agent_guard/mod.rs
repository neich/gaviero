//! `.gaviero/settings.json` → per-provider read policy.
//!
//! Gaviero owns the decision of which files an agent may read; each provider
//! only receives a compiled form of it:
//!
//! * [`hook`] — the decision logic behind `gaviero-cli --hook`, the command
//!   Claude (`PostToolUse` Bash output redaction) and Cursor
//!   (`beforeReadFile` / `beforeShellExecution`) call per tool use.
//! * [`claude`] — the `--settings` layer (Read denies, outside-read block,
//!   the Bash hook) every Claude spawn carries.
//! * [`cursor`] — `.cursor/hooks.json`, written before every Cursor spawn.
//! * [`codex`] — the `thread/start` config (deny-read permission profile,
//!   MXC sandbox on Windows) for the Codex app-server.

pub mod claude;
pub mod codex;
pub mod cursor;
pub mod hook;
