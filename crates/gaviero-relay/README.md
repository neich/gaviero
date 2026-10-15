# gaviero-relay

A small rendezvous relay for **Gaviero Remote**. It replaces Tailscale, the `ssh -R` tunnel and ntfy with **one subdomain on port 443**.

- **gaviero dials out from anywhere** (a desktop behind NAT, a VPS) to `wss://gaviero.<domain>/v1/host` and registers a route.
- **Live control:** the phone opens TLS to `gaviero.<domain>:443` and names the instance in SNI (`<workspace-id>-<machine>.gaviero.<domain>`). nginx passes it, still encrypted, to the relay, which splices it to that host. **TLS is end to end** between the phone and the host's sidecar, using a per-machine CA pinned at pairing, so the relay never sees the token, prompts, diffs or shell bytes.
- **Notifications:** hosts send `{machine, workspace, conv, kind}` and the phone's background service reads them from `/v1/phone/stream`. This replaces ntfy.

Wire protocol: [PROTOCOL.md](PROTOCOL.md). The types are `gaviero_remote::relay_proto`, the same module the host-side client ([`relay_client`](../gaviero-remote/src/relay_client.rs)) uses.

## Build

The relay is a crate of the gaviero workspace, but it is a server binary, **not part of a gaviero install**. Build it on its own:

```bash
git clone <gaviero repo> gaviero && cd gaviero
cargo build --release --locked -p gaviero-relay    # → target/release/gaviero-relay
```

- **Toolchain:** Rust ≥ 1.85 (edition 2024). On the VPS, install it with [rustup](https://rustup.rs); a distro `cargo` is usually too old.
- **What gets compiled:** only the relay and its own dependencies (tokio, axum, rustls's ClientHello parser, and gaviero-remote's plain serde wire types). The sidecar, the TUI and the C/cmake-built crates are not compiled.

Tests, from the workspace root:

```bash
cargo test -p gaviero-relay
```

`tests/e2e.rs` runs the real relay against the real host side (`gaviero_remote::relay_client` plus the sidecar with a pinned-CA leaf), with a rustls client acting as the phone.

## Deploy on the VPS (user-level service behind nginx)

Run these commands from the workspace root. The deploy files live in `crates/gaviero-relay/deploy/`.


1. **DNS:** one A/AAAA record `gaviero.wuilder.com` → VPS. Route names are never resolved, so no wildcard is needed.
2. **Binary and secret:**
   ```bash
   install -Dm755 target/release/gaviero-relay ~/.local/bin/gaviero-relay
   install -d -m700 ~/.config/gaviero-relay
   ~/.local/bin/gaviero-relay gen-secret > ~/.config/gaviero-relay/secret
   chmod 600 ~/.config/gaviero-relay/secret
   ```
3. **Service:**
   ```bash
   install -Dm644 crates/gaviero-relay/deploy/gaviero-relay.service ~/.config/systemd/user/gaviero-relay.service
   sudo loginctl enable-linger "$USER"
   systemctl --user daemon-reload && systemctl --user enable --now gaviero-relay
   journalctl --user -u gaviero-relay -f
   ```
4. **Certificate** (port 80 stays open for renewals):
   ```bash
   sudo certbot certonly --nginx -d gaviero.wuilder.com --deploy-hook "systemctl reload nginx"
   ```
5. **nginx:** `crates/gaviero-relay/deploy/nginx-gaviero-relay.conf` holds the top-level `stream` block (SNI routing on 443) plus the `http` servers.
   - **Check the build first:** `nginx -V 2>&1 | grep -o 'with-stream\|stream_ssl_preread\|http_realip'`.
   - **Then validate and reload:** `sudo nginx -t && sudo systemctl reload nginx`.
   - **Nothing else may listen on 443** on that IP: once the `stream` block owns 443, every other HTTPS site needs its own `map` entry.
6. **Firewall:** 443 and 80 only. The relay's ports (7080, 7443) are loopback-only.
7. **Check:**
   ```bash
   curl -s https://gaviero.wuilder.com/healthz            # ok
   curl -si https://gaviero.wuilder.com/v1/phone/stream | head -1   # HTTP/1.1 401
   ```

## Configure gaviero (on every machine, desktop or VPS)

`~/.gaviero/settings.json`:

```json
{ "remote": { "relay": {
    "host": "gaviero.wuilder.com",
    "secret": "<contents of ~/.config/gaviero-relay/secret>"
} } }
```

- **`machine`** is optional and defaults to the sanitised hostname.
- **`url`** is optional and overrides `wss://<host>`.
- **What relay mode changes:** binding is loopback only, the pinned CA is used, and there is no directory port. Run `/remote` and look for `Relay: … — registered`.

Pair the phone by scanning `/remote`'s QR once per machine. The QR carries the route URL, the machine CA (`tls_ca`), the token, and `relay_host` / `relay_secret`.

## Phone

When a relay machine is paired, Gaviero Remote starts a `specialUse` foreground service that holds `/v1/phone/stream` and posts alerts. It asks once for the battery-optimisation exemption, reconnects on network/VPN changes, and restarts after a reboot. Tapping an alert opens that conversation.

**The ntfy app is no longer needed.** Retire ntfy only after alerts work on your phone:
1. stop and disable the ntfy user service;
2. remove the `ntfy.wuilder.com` nginx blocks and DNS record;
3. run `sudo certbot delete --cert-name ntfy.wuilder.com`;
4. set `notifications.ntfy.enabled: false` in gaviero.

## Operational notes

- **Config:**
  - `--public-host` restricts routes to `<label>.<public-host>`;
  - `--proxy-protocol` must match nginx's `proxy_protocol on`;
  - `--note-ttl-hours` (default 6) sets how long notifications wait for an offline phone.
- **Logs** never contain the secret or any content: just route names, client IPs and byte counts.
- **Restarts:** restarting the relay drops live sessions. Hosts re-register within seconds (jittered backoff, ≤ 30 s) and phones reconnect. Queued notifications are lost on restart, because they're kept in memory only.
