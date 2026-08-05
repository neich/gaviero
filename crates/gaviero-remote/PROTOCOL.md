# Gaviero Remote Protocol — v1.0 (frozen)

Normative wire contract between the gaviero TUI sidecar (server) and the mobile client.
Frozen by Plan A V3 unit A0 on 2026-08-05. The machine-readable form is
`protocol.schema.json` (generated from the Rust DTOs in this crate — Rust is the source of
truth); one example fixture per frame lives in [`fixtures/`](fixtures/). Where prose and
schema disagree, the schema wins.

```
PROTOCOL_VERSION  = { major: 1, minor: 0 }
WebSocket path    = /v1/ws
Subprotocol       = gaviero.v1
```

The document version of the plan that produced this (V3) is unrelated to the wire version.

- **major** bumps on incompatible envelope or semantic changes. The server rejects a
  different major.
- **minor** bumps on backward-compatible additions (new optional fields, new ignorable
  server events, new capability strings). Clients MUST ignore unknown frame types and
  unknown fields.

## Transport and handshake

- TLS is terminated in-process (rustls). The QR URL always uses the Tailscale MagicDNS
  hostname, never an IP or `localhost`.
- The bearer token is accepted **only** in the HTTP upgrade header
  `Authorization: Bearer <token>`. Never in the URL, query string, subprotocol, or any
  frame. The server rejects the upgrade otherwise.
- The client MUST request subprotocol `gaviero.v1`.
- After upgrade, the client MUST send `client_hello` before any other frame. The server
  replies with `hello`. A newly authenticated **and version-compatible** client evicts any
  previous client with close code 4005; a failed handshake never evicts.
- All frames are text frames containing one JSON object. Binary frames, malformed JSON,
  and frames over `limits.max_frame_bytes` are protocol errors (close 4003 / 4004).
- The server pings every 20 s and closes after 60 s without pong or traffic. A
  backgrounded phone hitting this is normal; reconnect and resnapshot.

## Envelopes

Client → server:

```json
{ "version": {"major":1,"minor":0}, "instance_id": "…", "command_id": "…",
  "type": "…", "payload": { } }
```

Server → client:

```json
{ "version": {"major":1,"minor":0}, "instance_id": "…", "seq": 42, "revision": 7,
  "type": "…", "payload": { } }
```

- `instance_id` is random per TUI launch, delivered in `hello`. The client echoes it on
  every frame. **Exception:** on `client_hello` (the only frame sent before `hello`) it is
  `null`. An incoming server frame with a different `instance_id` means the TUI restarted:
  drop local state and resnapshot.
- `command_id` is client-generated and unique per command (monotonic counter + session
  nonce). The server deduplicates repeats within a bounded recent-ID cache, so retrying a
  flaky send is safe.
- `seq` is monotonic per `instance_id`, assigned only when a frame is emitted, and
  continues across socket generations. A gap means missed frames → send
  `request_snapshot`.
- `revision` is the global snapshot generation. It orders snapshots and signals staleness.
  It is **never** sent back as a command precondition — freshness is per entity (below).
- Optional fields are **omitted** when absent (not `null`); the single exception is
  `client_hello`'s envelope `instance_id`, which is literally `null`.

## Command correlation

Every command receives exactly one terminal response: `command_result { command_id,
status, result? }` or `command_error { command_id, code, message }`.
`status: "accepted"` **is terminal** — it means validated-and-started; the outcome then
arrives as ordinary lifecycle events keyed by the `turn_id` / `proposal_id` carried in
`result`. A command never gets both an `accepted` and a later `completed`.

`command_error.code` (frozen; additions are minor bumps):

```
invalid_payload      unknown_type          unknown_conversation  unknown_request
unknown_proposal     invalid_hunk          stale_request         stale_proposal
stale_conversation   conversation_streaming  slash_not_allowed   confirm_required
too_large            rate_limited          duplicate_command     internal_error
```

## Per-entity freshness — first-writer-wins

The desktop and the phone are both live; losing a race is normal, not an error. Never
retry a stale command blindly — re-read the entity and let the user decide.

| Command | Token sent | On mismatch |
|---|---|---|
| `permission_decision` | `request_id` | `stale_request`. First valid answer (either side) takes the oneshot sender and wins; the loser's command fails without mutation; **both** sides then see `permission_closed` with `answered_by`. |
| `review_action` (all actions incl. `finalize`) | `proposal_revision` | `stale_proposal`, no mutation. Re-read from the last `proposal_updated`. Actions are absolute sets, not toggles. |
| `rename_conversation`, `reset_conversation` | `conv_revision` | `stale_conversation`, no mutation. Refresh from `conversation_state_changed`. |
| `send_prompt`, `slash`, `switch_conversation`, `new_conversation`, `interrupt`, `request_*` | none (id validity only) | `send_prompt` additionally fails with `conversation_streaming` on a streaming target. |

Every reducer that mutates an entity bumps that entity's revision in the same transition
that emits the outbound event, so a client that applied the last event it received always
holds the current token.

## Close codes

```
4001 unauthorized          4005 replaced
4002 unsupported_version   4006 token_rotated
4003 protocol_error        4007 server_shutdown
4004 frame_too_large       4008 slow_client
```

## Client frames

| type | payload |
|---|---|
| `client_hello` | `{ protocol_version, client_name, client_version }` |
| `send_prompt` | `{ conv_id, text }` |
| `slash` | `{ conv_id, line, confirmed }` — `confirmed: true` required for entries in `hello.confirm_required` |
| `permission_decision` | `{ request_id, allow, answers?, message? }` — see Permission safety |
| `review_action` | `{ proposal_id, proposal_revision, action, hunk_index? }` — `action ∈ accept_hunk \| reject_hunk \| accept_all \| reject_all \| finalize`; `hunk_index` required for the per-hunk actions |
| `new_conversation` | `{ }` |
| `switch_conversation` | `{ conv_id }` |
| `rename_conversation` | `{ conv_id, conv_revision, title }` |
| `reset_conversation` | `{ conv_id, conv_revision }` |
| `interrupt` | `{ conv_id, turn_id? }` |
| `request_snapshot` | `{ }` |
| `request_messages` | `{ conv_id, before_seq, limit }` — `limit` clamped to 1–200 |
| `request_proposal` | `{ proposal_id }` |

There is **no** `rotate_token` command. Rotation is desktop-only and reaches the client as
close 4006.

## Server frames

| type | payload |
|---|---|
| `hello` | `{ protocol_version, instance_id, tui_version, workspace: { id, display_name }, capabilities, confirm_required, allowed_slash_commands, limits }` |
| `snapshot` | `{ revision, conversations: [ConversationSummary], active_id, active_conversation: ConversationState, open_permissions: [PermissionRequest], open_proposals: [ProposalSummary], settings: RemoteSettings }` |
| `conversation_state_changed` | `{ conversation: ConversationSummary, active_id }` — upsert by `conv_id` |
| `conversation_removed` | `{ conv_id, active_id }` |
| `message_page` | `{ conv_id, messages: [Message], oldest_seq, has_older_messages }` |
| `stream_chunk` | `{ conv_id, turn_id, text }` — coalesced ~50 ms; append-only within a turn |
| `streaming_status` | `{ conv_id, turn_id, status }` — `status` is a free-form display string |
| `streaming_ended` | `{ conv_id, turn_id, cancelled, error?, proposal_count }` |
| `tool_call_started` | `{ conv_id, turn_id, tool_name }` |
| `message_complete` | `{ conv_id, message: Message }` |
| `permission_request` | `{ conv_id, request_id, tool_name, description, input, ask? }` — `input` is display-only |
| `permission_closed` | `{ conv_id, request_id, outcome, answered_by }` — `outcome ∈ allowed \| denied \| superseded \| cancelled`; `answered_by ∈ desktop \| remote \| system` |
| `proposal_created` | `{ proposal: Proposal }` |
| `proposal_updated` | `{ proposal: Proposal }` |
| `proposal_detail` | `{ proposal: Proposal }` — response to `request_proposal` |
| `proposal_finalized` | `{ proposal_id, path, outcome }` — `outcome ∈ accepted \| partially_accepted \| rejected` |
| `token_usage` | `{ conv_id, turn_id, usage }` |
| `cost_update` | `{ conv_id, turn_id, usd }` |
| `command_result` | `{ command_id, status, result? }` — `status ∈ accepted \| completed` |
| `command_error` | `{ command_id, code, message }` |

`hello.capabilities` is an array of strings, **empty in 1.0**; the shape is frozen so
minor versions can advertise features. Clients ignore unknown entries.

## DTOs

```text
ProtocolVersion      { major: u16, minor: u16 }

ConversationSummary  { conv_id, conv_revision, title, model?, effort?, namespace?,
                       is_streaming, pending_turn_id?, context_pressure?, auto_approve,
                       last_message_preview? }
  model is a full `provider:model` spec. context_pressure = { used_tokens, max_tokens }.

ConversationState    { summary: ConversationSummary, messages: [Message],
                       oldest_seq, has_older_messages }
  At most the latest 100 messages and 512 KiB encoded.

Message              { seq, role, content, truncated, full_bytes?, tool_calls: [string],
                       code_blocks: [CodeBlock] }
  role ∈ user | assistant | system. seq is monotonic per conversation, survives
  /compact and /reset, and is NOT the envelope seq. Over 128 KiB the head is sent with
  truncated: true and full_bytes set.

CodeBlock            { start_byte, end_byte, language?, truncated, spans: [Span] }
Span                 { start_byte, end_byte, class }

PermissionRequest    { conv_id, request_id, tool_name, description, input, ask? }
Ask                  { questions: [AskQuestion] }
AskQuestion          { question, header, multi_select, options: [AskOption] }
AskOption            { label, description }

Proposal             { proposal_id, proposal_revision, conv_id?, source, path, status,
                       is_deletion, conflicts_with: [u64], hunks: [Hunk] }
  status ∈ pending | partially_accepted | accepted | rejected | superseded
  path is workspace-relative. source is a free-form producer label.

Hunk                 { index, original_range, proposed_range, original_text,
                       proposed_text, truncated, hunk_type, description, status }
  hunk_type ∈ added | removed | modified;  status ∈ pending | accepted | rejected
  original_range / proposed_range = { start_line, end_line } — 0-indexed, display-only.
  index is the position in the gate's structural_hunks at creation, stable for the
  proposal lifetime, and is what review_action sends back.

ProposalSummary      { proposal_id, proposal_revision, conv_id?, source, path, status,
                       is_deletion, conflicts_with, hunk_count, added_lines,
                       removed_lines, hunks: [HunkSummary] }
HunkSummary          { index, hunk_type, description, status }
  Never carries hunk text.

RemoteSettings       { default_model?, default_effort? }
  Explicit allow-list; never raw workspace settings. Additions are minor bumps.

TokenUsage           { input_tokens, cache_creation_input_tokens,
                       cache_read_input_tokens, output_tokens }

Limits               { max_frame_bytes, max_prompt_bytes, command_rate_per_second }
```

### Highlighting offsets (normative)

**All `CodeBlock` and `Span` offsets are UTF-8 byte offsets into `Message.content`, with
exclusive ends.** A `CodeBlock` range covers the entire fenced block **including** the
opening and closing fences; `Span` offsets are absolute into `content` (not relative to
the block). Dart strings are UTF-16 — the client must slice `utf8.encode(content)` (or
build an offset table), never `String.substring`. `class` is a semantic tree-sitter
capture name (`keyword`, `string`, `function.method`, …); unknown classes render as plain
text; the server never sends colors. `truncated: true` on a block (over 256 KiB) means no
spans — render plain. Span fields exist from 1.0 and may be empty until highlighting
ships (A7).

### Permission safety (normative)

A permission decision cannot change *what* executes. The wire carries `answers: [[u32]]`
— per question, the selected option indices — never a tool-input document. The server
rejects `answers` on a non-`AskUserQuestion` permission, validates
`answers.len == questions.len`, every index in range, exactly one selection when
`multi_select` is false, then rebuilds the tool input server-side through the same code
path the desktop uses. A plain allow carries no `answers` and echoes the original input
unchanged. `permission_request.input` is display-only. `message?` on a deny is free
text, size-limited, control-character-filtered, and never reaches tool input.

## Caps and limits (frozen)

| What | Cap |
|---|---|
| Frame | `remote.maxFrameBytes`, default 256 KiB |
| Prompt | `remote.maxPromptBytes`, default 128 KiB |
| Command rate | `remote.commandRatePerSecond`, default 10 |
| Snapshot active tail | 100 messages / 512 KiB |
| `request_messages` page | limit clamp 1–200 / 512 KiB |
| Single message | 128 KiB, then `truncated` + `full_bytes` |
| Hunk side | 64 KiB, then hunk `truncated` |
| Proposal | 512 KiB, then affected hunks `truncated` |
| Highlighted block | 256 KiB, then block `truncated`, no spans |

A truncated hunk is still fully reviewable: the client sends
`{ proposal_id, hunk_index, action }` and the server assembles the file from its own
copy. The client never sends file content.

## Slash command policy

The server owns the allow-list; the client builds its entire command surface from
`hello.allowed_slash_commands` and confirm-gates `hello.confirm_required`. Initial
allow-list:

```
/model /thinking /effort /compact /context /inject /no-inject /reset /clear
/rename /namespace /ns /autoapprove /yolo /workspace /ws /lite /minimal /help /skills
```

Initial confirm-required: `/autoapprove /yolo /reset /clear`. Denied remotely (returned
as `slash_not_allowed`, never forwarded to the agent): `/attach`, `/detach`, `/run`,
swarm/coordinator commands, memory write/delete/restore/reembed/sleeptime commands, and
any command not explicitly allow-listed. Matching preserves the desktop parser's token
boundary — `/runaway` does not match `/run`.

## QR pairing payload

```json
{ "kind": "gaviero-remote", "url": "wss://host.tailnet.ts.net:PORT/v1/ws",
  "token": "SECRET", "workspace": "display-name", "protocol_major": 1 }
```

The QR is the only intentional display of the token.

## Fixtures

One example per frame type under `fixtures/client/` (13) and `fixtures/server/` (20),
named `<type>.json`, each a complete envelope. `fixtures/server/message_complete.json`
deliberately contains non-ASCII content (accent + emoji) with hand-verified UTF-8 byte
offsets — the A1 test suite asserts those offsets land on UTF-8 boundaries and match the
declared fence positions. Regenerate the schema with the ignored update test documented
in the crate README; a normal `cargo test` never rewrites the working tree.
