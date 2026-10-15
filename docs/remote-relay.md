# Gaviero Remote through gaviero-relay (no Tailscale, no ntfy)

Run gaviero anywhere — a desktop behind NAT or a VPS — and reach it from the phone through **one subdomain on port 443** of a VPS you control. The relay is the workspace crate [`crates/gaviero-relay`](../crates/gaviero-relay/README.md), an optional server binary that a gaviero install never includes. Build it on the VPS with `cargo build --release --locked -p gaviero-relay`. Its README covers deployment (a user-level systemd service behind nginx), and its PROTOCOL.md covers the wire protocol.

```
phone ──TLS (SNI = <workspace-id>-<machine>.gaviero.<domain>)──▶ nginx :443 (stream, ssl_preread) ──▶ gaviero-relay ──▶ gaviero sidecar (127.0.0.1)
gaviero ──wss://gaviero.<domain>/v1/host (dials out)──────────────────────────────▶ gaviero-relay
phone background service ◀── /v1/phone/stream (notifications, replaces ntfy) ── gaviero-relay
```

- **TLS is end to end, phone ⇄ sidecar.** It uses the machine's private CA, which the app pins from the pairing QR (`tls_ca`). The relay sees only the route name and ciphertext.
- **No per-machine setup on the VPS:** no DNS records, no ports, no SSH keys. Machines only need the relay secret.
- **Notifications** are `{machine, workspace, conv, kind}` and nothing else. The app renders the text.

## Settings (`~/.gaviero/settings.json`, same on every machine)

```json
{ "remote": { "relay": {
    "host": "gaviero.wuilder.com",
    "secret": "<the relay's --secret-file contents>"
} } }
```

| Key | Default | Meaning |
|---|---|---|
| `remote.relay.host` | `""` | Non-empty turns relay mode on |
| `remote.relay.secret` | `""` | Required in relay mode (`RelaySecretMissing` otherwise) |
| `remote.relay.machine` | sanitised hostname | Machine label in route names (`[a-z0-9-]`, ≤ 40) |
| `remote.relay.url` | `wss://<host>` | Override the relay base URL (tests, non-443 setups) |

**Relay mode implies these settings:**
- `bindMode = "loopback"`;
- `tlsMode = "pinnedCa"`, with the leaf also covering `*.<relay host>`;
- no machine directory port;
- machine id `<machine>.<relay host>`;
- route `<workspace-id>-<machine>.<relay host>`;
- QR URL `wss://<route>/v1/ws` (no port).

The QR also carries `relay_host` and `relay_secret`.

**`/remote` shows** `Relay: wss://… — route … — registered`. Anything other than `registered` (a 401, a timeout) explains why the phone can't connect.

## Phone

1. Scan `/remote` once per machine.
2. Gaviero Remote starts its own alert service (`specialUse` foreground service) and asks once for the battery-optimisation exemption.
3. Tapping an alert opens the conversation.

**The ntfy app is not needed.** ntfy publishing from the TUI still works while `notifications.ntfy.enabled` is on. Turn it off once relay alerts work on your phone.

## Lower-level building blocks

`remote.bindMode = "loopback"` and `remote.tlsMode = "pinnedCa"` can still be set on their own, for example behind a hand-built TLS-passthrough proxy. The earlier `ssh -R` + per-machine nginx port design that used them directly is superseded by gaviero-relay. It is kept in git history only.
