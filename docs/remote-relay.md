# Gaviero Remote through gaviero-relay

gaviero-relay is the only way Gaviero Remote reaches a machine. Run gaviero anywhere — a desktop behind NAT or a VPS — and reach it from the phone through **one subdomain on port 443** of a VPS you control. The relay is the workspace crate [`crates/gaviero-relay`](../crates/gaviero-relay/README.md), an optional server binary that a gaviero install never includes. Build it on the VPS with `cargo build --release --locked -p gaviero-relay`. Its README covers deployment (a user-level systemd service behind nginx), and its PROTOCOL.md covers the wire protocol.

```
phone ──TLS (SNI = <workspace-id>-<machine>.gaviero.<domain>)──▶ nginx :443 (stream, ssl_preread) ──▶ gaviero-relay ──▶ gaviero sidecar (127.0.0.1)
gaviero ──wss://gaviero.<domain>/v1/host (dials out)──────────────────────────────▶ gaviero-relay
phone background service ◀── /v1/phone/stream (notifications) ── gaviero-relay
```

- **TLS is end to end, phone ⇄ sidecar.** It uses the machine's private CA, which the app pins from the pairing QR (`tls_ca`). The relay sees only the route name and ciphertext.
- **No per-machine setup on the VPS:** no DNS records, no ports, no SSH keys. Machines only need the relay secret.
- **Notifications** are `{machine, workspace, conv, kind}` and nothing else. The app renders the text.

## Settings (`~/.gaviero/settings.json`, same on every machine)

```json
{ "remote": { "relay": {
    "host": "gaviero.example.com",
    "secret": "<the relay's --secret-file contents>"
} } }
```

| Key | Default | Meaning |
|---|---|---|
| `remote.relay.host` | `""` | Required: without it the sidecar does not start |
| `remote.relay.secret` | `""` | Required (`RelaySecretMissing` otherwise) |
| `remote.relay.machine` | sanitised hostname | Machine label in route names (`[a-z0-9-]`, ≤ 40) |
| `remote.relay.url` | `wss://<host>` | Override the relay base URL (tests, non-443 setups) |

**What follows from these settings:**
- the listener binds loopback only (`127.0.0.1`, `::1`);
- TLS uses a per-machine private CA under `~/.gaviero/remote/tls/pinned/`, with the leaf also covering `*.<relay host>`;
- every instance's route serves the machine's instance list (`/v1/instances`);
- machine id `<machine>.<relay host>`;
- route `<workspace-id>-<machine>.<relay host>`;
- QR URL `wss://<route>/v1/ws` (no port).

The QR also carries `relay_host` and `relay_secret`.

**`/remote` shows** `Relay: wss://… — route … — registered`. Anything other than `registered` (a 401, a timeout) explains why the phone can't connect.

## Phone

1. Scan `/remote` once per machine.
2. Gaviero Remote starts its own alert service (`specialUse` foreground service) and asks once for the battery-optimisation exemption.
3. Tapping an alert opens the conversation.
