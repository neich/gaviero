# gaviero-remote

Wire protocol (DTOs + JSON Schema) and, behind the `server` feature, the WSS
sidecar server for Gaviero Remote. No dependency on `gaviero-core` or
`gaviero-tui` — DTOs are remote-owned by design.

- Contract: [PROTOCOL.md](PROTOCOL.md) (prose) + `protocol.schema.json`
  (generated; Rust is the source of truth).
- Fixtures: one example per frame under [`fixtures/`](fixtures/).

```bash
cargo test -p gaviero-remote                                    # full
cargo test -p gaviero-remote --no-default-features --features dto
# Regenerate protocol.schema.json after a DTO change (a normal test run
# never rewrites the working tree):
cargo test -p gaviero-remote regenerate_schema -- --ignored
```
