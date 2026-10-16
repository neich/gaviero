# gaviero-relay

A small rendezvous relay for **Gaviero Remote**. It replaces Tailscale, the `ssh -R` tunnel and ntfy with **one subdomain on port 443** of a VPS you control.

- **gaviero dials out from anywhere** (a desktop behind NAT, a VPS) to `wss://gaviero.<domain>/v1/host` and registers a route.
- **Live control:** the phone opens TLS to `gaviero.<domain>:443` and names the instance in SNI (`<workspace-id>-<machine>.gaviero.<domain>`). nginx passes it, still encrypted, to the relay, which splices it to that host.
  - **TLS is end to end** between the phone and the host's sidecar, using a per-machine CA pinned at pairing.
  - The relay never sees the token, prompts, diffs or shell bytes.
- **Notifications:** hosts send `{machine, workspace, conv, kind}` and the phone's background service reads them from `/v1/phone/stream`. This replaces ntfy.

Wire protocol: [PROTOCOL.md](PROTOCOL.md). The types are `gaviero_remote::relay_proto`, the same module the host-side client ([`relay_client`](../gaviero-remote/src/relay_client.rs)) uses.

**Setup at a glance:**
1. [Build](#build) the relay on the VPS.
2. [Deploy](#deploy-on-the-vps) it: DNS, binary, **the shared secret**, service, certificate, nginx, firewall.
3. [Configure gaviero](#configure-gaviero-on-every-machine) on every machine with the relay host and that secret.
4. [Pair the phone](#phone) by scanning `/remote` once per machine.

The examples use `gaviero.wuilder.com`. For another name, replace it everywhere it appears (see [Using another domain](#using-another-domain)).

## Build

The relay is a crate of the gaviero workspace, but it is a server binary, **not part of a gaviero install**.

1. **Prerequisites on the VPS:** git, a C linker, and Rust ≥ 1.85 (edition 2024). A distro `cargo` is usually too old, so use [rustup](https://rustup.rs):
   ```bash
   sudo apt install git build-essential                              # Debian/Ubuntu: git + cc linker
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh    # then open a new shell
   ```
2. **Build** as the user that will run the service:
   ```bash
   git clone <gaviero repo> ~/gaviero && cd ~/gaviero
   cargo build --release --locked -p gaviero-relay    # → target/release/gaviero-relay
   ```

**What gets compiled:** only the relay and its own dependencies (tokio, axum, rustls's ClientHello parser, and gaviero-remote's plain serde wire types). The sidecar, the TUI and the C/cmake-built crates are not compiled.

**Tests**, from the workspace root:

```bash
cargo test -p gaviero-relay
```

`tests/e2e.rs` runs the real relay against the real host side (`gaviero_remote::relay_client` plus the sidecar with a pinned-CA leaf), with a rustls client acting as the phone.

## Deploy on the VPS

**Where to run things:**
- Run every command from the workspace root (`~/gaviero`). The deploy files live in `crates/gaviero-relay/deploy/`.
- Run steps 2–3 **as the service user, without `sudo`**: log in over SSH as that user, or use `sudo machinectl shell <user>@`. Under `su`, `systemctl --user` has no session bus. The one exception is `loginctl enable-linger`, which needs sudo.
- Steps 4–6 need sudo.

### 1. DNS

Create one A/AAAA record, `gaviero.wuilder.com` → the VPS. Route names are never resolved, so no wildcard record is needed.

### 2. Binary and the shared secret

```bash
install -Dm755 target/release/gaviero-relay ~/.local/bin/gaviero-relay
install -d -m700 ~/.config/gaviero-relay
~/.local/bin/gaviero-relay gen-secret > ~/.config/gaviero-relay/secret
chmod 600 ~/.config/gaviero-relay/secret
```

**The secret file:**
- `gen-secret` prints a fresh random 256-bit secret (64 hex characters).
- The relay reads only the file's first line, and it refuses to start if that line is shorter than 16 characters.

**Who needs this secret:**
- **Every gaviero machine.** You copy it into their settings in [Configure gaviero](#configure-gaviero-on-every-machine).
- **The phone.** It receives it inside the pairing QR.

It decides who may use the relay at all; it is not what protects your sessions (end-to-end TLS does). Still, treat it like a password. Rotating it is covered under [Rotating the secret](#rotating-the-secret).

### 3. Service (systemd user unit)

```bash
install -Dm644 crates/gaviero-relay/deploy/gaviero-relay.service ~/.config/systemd/user/gaviero-relay.service
sudo loginctl enable-linger "$USER"          # start at boot, keep running after logout
systemctl --user daemon-reload && systemctl --user enable --now gaviero-relay
systemctl --user status gaviero-relay        # "active (running)"
journalctl --user -u gaviero-relay -f        # "gaviero-relay listening"
```

**What the unit runs:** [`deploy/gaviero-relay.service`](deploy/gaviero-relay.service) starts the relay with:
- `--http 127.0.0.1:7080 --splice 127.0.0.1:7443`;
- `--secret-file ~/.config/gaviero-relay/secret`;
- `--public-host gaviero.wuilder.com`;
- `--proxy-protocol`.

All flags are listed under [Relay options](#relay-options).

**Troubleshooting:**
- **`Failed to enable unit: Unit …/default.target.wants/gaviero-relay.service does not exist`:** something under `~/.config/systemd` is owned by root (usually because it was created with sudo), so systemd can't create the `.wants` link.
  - Check with `ls -ld ~/.config ~/.config/systemd ~/.config/systemd/user`.
  - Fix with `sudo chown -R "$USER": ~/.config ~/.local`, then run `daemon-reload` and `enable --now` again.
- **`cannot read …/secret`** in the journal: the secret file is missing or not readable by the service user (see step 2).

### 4. nginx and the certificate

The nginx config comes in three files. They follow the `*-available` / `*-enabled` layout: each file lives in an `-available` directory and is switched on with a symlink in the matching `-enabled` directory. Install them in this order:

| File | Goes to (`-available`, linked from `-enabled`) | Contains |
|---|---|---|
| [`nginx-http.conf`](deploy/nginx-http.conf) | `/etc/nginx/sites-available/gaviero-relay-http` | Port 80: certbot challenges, redirect to HTTPS |
| [`nginx-https.conf`](deploy/nginx-https.conf) | `/etc/nginx/sites-available/gaviero-relay-https` | The relay's HTTPS on `127.0.0.1:8444` (Let's Encrypt cert) → relay `:7080` |
| [`nginx-stream.conf`](deploy/nginx-stream.conf) | `/etc/nginx/streams-available/gaviero-relay` | `:443` SNI router: `gaviero.wuilder.com` → `:8444`, any other name → relay splice `:7443` |

**Why there are separate `streams-*` directories:**
- `sites-enabled/*` is included **inside** `http {}`, but a `stream {}` block must sit at the **top level** of nginx.conf. It can't go in `sites-*`.
- Step 4d therefore creates `streams-available` / `streams-enabled` with the same layout, plus one top-level `include`.

**4a. Packages and modules:**
```bash
sudo apt install nginx libnginx-mod-stream certbot python3-certbot-nginx
nginx -V 2>&1 | grep -o 'with-stream[^ ]*\|stream_ssl_preread\|http_realip'
```
All three names must appear. On Debian/Ubuntu, `with-stream=dynamic` is fine once `libnginx-mod-stream` is installed.

**4b. Port 80, then the certificate:**
```bash
sudo install -m644 crates/gaviero-relay/deploy/nginx-http.conf /etc/nginx/sites-available/gaviero-relay-http
sudo ln -s ../sites-available/gaviero-relay-http /etc/nginx/sites-enabled/gaviero-relay-http
sudo nginx -t && sudo systemctl reload nginx
sudo certbot certonly --nginx -d gaviero.wuilder.com --deploy-hook "systemctl reload nginx"
```
- `certonly` obtains the certificate without editing your nginx files.
- Renewals are automatic (certbot's systemd timer) and go through port 80, so keep it open.
- Check renewal with `sudo certbot renew --dry-run`.

**4c. The relay's HTTPS server:**
```bash
sudo install -m644 crates/gaviero-relay/deploy/nginx-https.conf /etc/nginx/sites-available/gaviero-relay-https
sudo ln -s ../sites-available/gaviero-relay-https /etc/nginx/sites-enabled/gaviero-relay-https
```

**4d. The `:443` SNI router (`streams-available` / `streams-enabled`, top level):**
```bash
sudo install -d /etc/nginx/streams-available /etc/nginx/streams-enabled
sudo install -m644 crates/gaviero-relay/deploy/nginx-stream.conf /etc/nginx/streams-available/gaviero-relay
sudo ln -s ../streams-available/gaviero-relay /etc/nginx/streams-enabled/gaviero-relay
grep -q 'streams-enabled/' /etc/nginx/nginx.conf || echo 'include /etc/nginx/streams-enabled/*;' | sudo tee -a /etc/nginx/nginx.conf
```
- **The `include` is the only edit to nginx.conf.** Appending it to the end of the file puts it at top level, outside `http {}`, which is where `stream {}` must be.
- **Check with `tail -3 /etc/nginx/nginx.conf`.** The `include` must come after the closing `}` of `http`, not inside it.

Before reloading, read [Other HTTPS sites on the same nginx](#other-https-sites-on-the-same-nginx). If any other site in `sites-enabled` still has `listen 443`, nginx can't bind the port.

**4e. Validate and reload:**
```bash
sudo nginx -t && sudo systemctl reload nginx
```

#### Other HTTPS sites on the same nginx

Once the `stream` block owns `:443`, **nothing else may `listen 443`** on this host. That includes ntfy while you still run it. Move each other HTTPS site to its own loopback port and add it to the map.

To find them, run `grep -l 'listen.*443' /etc/nginx/sites-enabled/*`.

1. **Change its `listen` lines.** In that site's file in `/etc/nginx/sites-available/`, find the `server { … }` with `listen 443 ssl` (and `listen [::]:443 ssl`, plus any `quic`/`http2` listen flags). Replace those lines with:
   ```nginx
   listen 127.0.0.1:8445 ssl proxy_protocol;   # one free loopback port per site
   set_real_ip_from 127.0.0.1;
   real_ip_header proxy_protocol;
   ```
   Keep its `ssl_certificate` lines. `http2 on;` may stay for ordinary sites.
2. **Add the site to the map** in `/etc/nginx/streams-available/gaviero-relay`, above `default`:
   ```nginx
   ntfy.wuilder.com  127.0.0.1:8445;
   ```
3. **Validate and reload:** `sudo nginx -t && sudo systemctl reload nginx`.

Certbot renewals for those sites keep working, because they use port 80.

### 5. Firewall

Open 80 and 443 only. The relay's own ports (7080, 7443) and nginx's loopback listeners (8444, …) are bound to 127.0.0.1. With ufw:

```bash
sudo ufw allow OpenSSH && sudo ufw allow 80,443/tcp && sudo ufw enable
```

### 6. Check

On the VPS:
```bash
curl -s https://gaviero.wuilder.com/healthz                         # ok
curl -si https://gaviero.wuilder.com/v1/phone/stream | head -1     # HTTP/1.1 401 (no secret)
curl -sN --max-time 5 -H "Authorization: Bearer $(head -1 ~/.config/gaviero-relay/secret)" \
  https://gaviero.wuilder.com/v1/phone/stream                       # {"type":"presence","epoch":"…","routes":[]}
```

| Symptom | Likely cause |
|---|---|
| Connection refused or timeout | DNS, firewall, or the stream block isn't loaded (step 4d: the `streams-enabled` link or the top-level `include`) |
| Certificate error | `gaviero-relay-https` isn't linked in `sites-enabled`, or the map doesn't send `gaviero.wuilder.com` to `:8444` |
| `502 Bad Gateway` | The relay isn't running (step 3) |
| These checks pass and `/remote` says `registered`, but the phone can't open a session | The PROXY setting doesn't match: the relay's `--proxy-protocol` and `proxy_protocol on` in `nginx-stream.conf` must both be on. With `RUST_LOG=gaviero_relay=debug`, the journal shows `missing or bad PROXY header` or `no ClientHello` |
| Journal: `no host registered for route` | The phone is paired with an instance that isn't running or isn't registered (check `/remote` on that machine) |

## Configure gaviero (on every machine)

**1. Get the secret.** Use the one generated in [step 2](#2-binary-and-the-shared-secret) of the deploy (`~/.config/gaviero-relay/secret` on the VPS). Print it there, or fetch it over SSH from the machine you're configuring:

```bash
ssh <user>@<vps> head -1 .config/gaviero-relay/secret
```

**2. Add it to that machine's user settings.** Edit `~/.gaviero/settings.json` (Windows: `%USERPROFILE%\.gaviero\settings.json`), merging into any existing `"remote"` object:

```json
{ "remote": { "relay": {
    "host": "gaviero.wuilder.com",
    "secret": "<the 64-hex-character secret>"
} } }
```

| Key | Default | Meaning |
|---|---|---|
| `remote.relay.host` | `""` | Relay name; non-empty turns relay mode on. Must equal the relay's `--public-host`, or the route is rejected |
| `remote.relay.secret` | `""` | Required in relay mode |
| `remote.relay.machine` | sanitised hostname | This machine's label in route names: lowercase `[a-z0-9-]`, ≤ 40 characters. Must be unique per machine |
| `remote.relay.url` | `wss://<host>` | Override the relay base URL (tests, non-443 setups) |

- **User level, not the workspace:** a workspace `.gaviero/settings.json` can override these too (cascade in the [TUI README](../gaviero-tui/README.md#configuration)). Prefer the user-level file so the secret stays out of repositories.
- **What relay mode changes:**
  - the sidecar binds to loopback only;
  - it uses a per-machine private CA (`~/.gaviero/remote/tls/pinned/`);
  - there is no directory port.
- **Full reference:** [docs/remote-relay.md](../../docs/remote-relay.md).

**3. Restart gaviero** (the TUI reads these settings at startup). Then run `/remote` and look at the `Relay:` line:

| `Relay:` line ends with | Meaning |
|---|---|
| `registered` | Working; phones can reach this instance |
| `disconnected — the relay rejected the secret (401) — check remote.relay.secret` | Wrong or stale secret |
| `route rejected — route must be <label>.<host>` | `remote.relay.host` differs from the relay's `--public-host` |
| `disconnected — timed out connecting to the relay` / `relay answered HTTP 502` | The relay or nginx isn't reachable (see [Check](#6-check)) |

If `/remote` reports that `remote.relay.secret` is empty, step 2 didn't take effect: check the file is valid JSON and the key is spelled correctly.

## Phone

1. Install a Gaviero Remote build with relay support.
2. Scan `/remote`'s QR **once per machine**. The QR carries:
   - the route URL;
   - the machine CA (`tls_ca`);
   - the pairing token;
   - `relay_host` / `relay_secret`.

   Because it holds both secrets, don't share screenshots of it.
3. Allow notifications, and accept the one-time battery-optimisation exemption prompt.

**The alert service:** when a relay machine is paired, Gaviero Remote starts a `specialUse` foreground service that:
- holds `/v1/phone/stream` and posts alerts;
- reconnects on network/VPN changes and restarts after a reboot.

Tapping an alert opens that conversation.

**The ntfy app is no longer needed.** Retire ntfy only after alerts work on your phone:
1. stop and disable the ntfy user service;
2. remove its line from the map in `streams-available/gaviero-relay`, remove its link from `sites-enabled`, then reload nginx and delete the DNS record;
3. run `sudo certbot delete --cert-name ntfy.wuilder.com`;
4. set `notifications.ntfy.enabled: false` in gaviero.

## Operations

### Relay options

| Flag | Default | Meaning |
|---|---|---|
| `--http <addr>` | `127.0.0.1:7080` | Control/data WebSockets and the phone stream (behind nginx HTTPS) |
| `--splice <addr>` | `127.0.0.1:7443` | Raw TLS from phones, routed by SNI (behind nginx `stream`) |
| `--secret-file <path>` | required | Shared secret, first line |
| `--public-host <name>` | none | Accept only routes `<label>.<name>` |
| `--proxy-protocol` | off | Expect a PROXY v1 line on the splice port; must match nginx's `proxy_protocol on` |
| `--note-ttl-hours <n>` | 6 | How long notifications wait for an offline phone |

`RUST_LOG` sets the log level (the unit uses `gaviero_relay=info`). Logs never contain the secret or any content: just route names, client IPs and byte counts.

To change a flag, edit `~/.config/systemd/user/gaviero-relay.service`, then:

```bash
systemctl --user daemon-reload && systemctl --user restart gaviero-relay
```

### Updating

```bash
cd ~/gaviero && git pull
cargo build --release --locked -p gaviero-relay
install -m755 target/release/gaviero-relay ~/.local/bin/gaviero-relay
systemctl --user restart gaviero-relay
```

**What a restart affects:**
- **Live sessions** drop.
- **Hosts** re-register within seconds (jittered backoff, ≤ 30 s), and phones reconnect.
- **Queued notifications** are lost, because they're kept in memory only.

### Rotating the secret

1. **On the VPS**, write a new secret and restart:
   ```bash
   ~/.local/bin/gaviero-relay gen-secret > ~/.config/gaviero-relay/secret
   systemctl --user restart gaviero-relay
   ```
2. **On every machine**, put it in `remote.relay.secret` and restart gaviero.
3. **On the phone**, re-pair each machine. The old QR carries the old secret.

### Using another domain

Replace `gaviero.wuilder.com` in:
- the DNS record;
- `deploy/gaviero-relay.service` (`--public-host`);
- the three `deploy/nginx-*.conf` files (`server_name`, the map, the certificate paths), or their copies in `sites-available` / `streams-available` if they're already installed;
- the certbot command;
- `remote.relay.host` on every machine.

For example:

```bash
sed -i 's/gaviero\.wuilder\.com/gaviero.example.org/g' crates/gaviero-relay/deploy/*
```
