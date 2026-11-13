# Contributing to Gaviero

Thanks for your interest! Bug reports, feature ideas and pull requests are all welcome.

## Reporting a bug

Open an issue with:

- your OS and terminal (e.g. Windows Terminal + PowerShell 7, macOS + iTerm2, Linux + kitty),
- the agent provider and model, if the bug involves an agent (`claude:sonnet`, `codex:…`, `ollama:…`),
- what you did, what you expected, and what happened,
- the relevant lines of `gaviero.log`: `~/.cache/gaviero/` on Linux, `~/Library/Caches/gaviero/` on macOS, `%LOCALAPPDATA%\gaviero\` on Windows.

Leave out API keys, tokens, and `.gaviero/settings.json` values such as `remote.relay.secret`.

## Building and testing

```bash
cargo build              # every crate
cargo test               # whole workspace
cargo test -p gaviero-core
cargo run -p gaviero-tui -- path/to/repo
```

Rust 2024 edition, stable toolchain (pinned in `rust-toolchain.toml`). Tests that need the network, a model download, or an installed agent CLI are `#[ignore]`d.

On Windows, if tests fail with "Access denied" on temporary files, point `TMP`/`TEMP` at a folder you own (e.g. `target/tmp`).

## Where things live

Read [ARCHITECTURE.md](ARCHITECTURE.md) first. Each crate has a `CLAUDE.md` with its own conventions and rules. Those files are written for coding agents, but they are the most accurate map of the code for people too:

| Crate | What it holds |
|---|---|
| [`gaviero-core`](crates/gaviero-core/CLAUDE.md) | All runtime logic; no UI or DSL dependencies |
| [`gaviero-tui`](crates/gaviero-tui/CLAUDE.md) | The terminal editor (ratatui + crossterm) |
| [`gaviero-cli`](crates/gaviero-cli/CLAUDE.md) | Headless runner |
| [`gaviero-dsl`](crates/gaviero-dsl/CLAUDE.md) | `.gaviero` workflow compiler |
| [`gaviero-mcp-shim`](crates/gaviero-mcp-shim/CLAUDE.md) | stdio ↔ socket / named-pipe MCP bridge |
| [`tree-sitter-gaviero`](crates/tree-sitter-gaviero/CLAUDE.md) | `.gaviero` grammar |

## Ground rules

These are the rules that most often trip people up. The full list is in [CLAUDE.md](CLAUDE.md).

- **Don't run `rustfmt`** on the codebase. Keep diffs to the lines you change.
- **Model specs are always `provider:model`** (`claude:sonnet`, never `sonnet`).
- **No file change skips review.** Agent edits go through turn capture (TUI chat) or the Write Gate (swarm/CLI).
- **Never hold a `Mutex` across `.await`, I/O or embedding.** All SQLite writes go through the memory writer task.
- **`gaviero-core` has no UI or DSL dependencies.**
- Never edit `tree-sitter-gaviero/src/parser.c` or `grammar.json` by hand; regenerate them from `grammar.js`.

## Pull requests

- Keep each PR focused on one change, and describe what changed and why.
- Add or update tests for behaviour changes. Update the README or the crate docs when a user-visible flag, key or setting changes.
- Make sure `cargo check --workspace --all-targets` passes. CI also checks Linux and macOS, which matters if you develop on one platform only.
- PRs are squash-merged.

Using an AI agent to write the change is fine, gaviero included. You are responsible for reviewing what it produced.

## License

By contributing you agree that your contributions are licensed under the [Apache License 2.0](LICENSE).
