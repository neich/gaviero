# gaviero-remote

Wire protocol (DTOs + JSON Schema) and, behind the `server` feature, the WSS
sidecar server for Gaviero Remote. No dependency on `gaviero-core` or
`gaviero-tui` — DTOs are remote-owned by design.

- Contract: [PROTOCOL.md](PROTOCOL.md) (prose) + `protocol.schema.json`
  (generated; Rust is the source of truth). Wire 1.1 is additive over the
  frozen 1.0 (`hello.machine`, optional `request_messages.before_seq`,
  `GET /v1/instances`).
- Fixtures: one example per frame under [`fixtures/`](fixtures/), plus
  `fixtures/http/instances.json` for the directory body.

```bash
cargo test -p gaviero-remote                                    # full
cargo test -p gaviero-remote --no-default-features --features dto
# Regenerate protocol.schema.json after a DTO change (a normal test run
# never rewrites the working tree):
cargo test -p gaviero-remote regenerate_schema -- --ignored
```
