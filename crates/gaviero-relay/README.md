# gaviero-relay

A small rendezvous relay for **Gaviero Remote**: the only way the phone reaches gaviero, through **one subdomain on port 443** of a VPS you control.

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

`gaviero.example.com` stands for your relay's name throughout this README and in the `deploy/` templates. You copy each template into place and then edit the copy: each step shows the exact lines, before and after. The templates in the repo never change.

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
- **Set your relay's name once per shell.** The commands below use this variable:
  ```bash
  RELAY_HOST=gaviero.yourdomain.tld          # the subdomain you'll point at this VPS
  ```

### 1. DNS

Create one A/AAAA record, `$RELAY_HOST` → the VPS. Route names are never resolved, so no wildcard record is needed.

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

**Copy the unit:**
```bash
install -d ~/.config/systemd/user
cp crates/gaviero-relay/deploy/gaviero-relay.service ~/.config/systemd/user/
nano ~/.config/systemd/user/gaviero-relay.service
```

**Edit one line** in the copy, the relay's name:

Before:
```ini
    --public-host gaviero.example.com \
```
After (with your name):
```ini
    --public-host gaviero.yourdomain.tld \
```

`grep -n example ~/.config/systemd/user/gaviero-relay.service` must print nothing.

**Start it:**
```bash
sudo loginctl enable-linger "$USER"          # start at boot, keep running after logout
systemctl --user daemon-reload && systemctl --user enable --now gaviero-relay
systemctl --user status gaviero-relay        # "active (running)"
journalctl --user -u gaviero-relay -f        # "gaviero-relay listening"
```

**What the unit runs:** [`deploy/gaviero-relay.service`](deploy/gaviero-relay.service) starts the relay with:
- `--http 127.0.0.1:7080 --splice 127.0.0.1:7443`;
- `--secret-file ~/.config/gaviero-relay/secret`;
- `--public-host <your relay's name>` (the line you edited);
- `--proxy-protocol`.

All flags are listed under [Relay options](#relay-options).

**Troubleshooting:**
- **`Failed to enable unit: Unit …/default.target.wants/gaviero-relay.service does not exist`:** something under `~/.config/systemd` is owned by root (usually because it was created with sudo), so systemd can't create the `.wants` link.
  - Check with `ls -ld ~/.config ~/.config/systemd ~/.config/systemd/user`.
  - Fix with `sudo chown -R "$USER": ~/.config ~/.local`, then run `daemon-reload` and `enable --now` again.
- **`cannot read …/secret`** in the journal: the secret file is missing or not readable by the service user (see step 2).

### 4. nginx and the certificate

The nginx config comes in two files. They follow the `*-available` / `*-enabled` layout: each file lives in an `-available` directory and is switched on with a symlink in the matching `-enabled` directory.

| File | Goes to (`-available`, linked from `-enabled`) | Contains |
|---|---|---|
| [`nginx-site.conf`](deploy/nginx-site.conf) | `/etc/nginx/sites-available/gaviero-relay` | Port 80 (certbot renewals, redirect to HTTPS) and the relay's HTTPS on `127.0.0.1:8443` (Let's Encrypt cert) → relay `:7080` |
| [`nginx-stream.conf`](deploy/nginx-stream.conf) | `/etc/nginx/streams-available/gaviero-relay` | `:443` SNI router: route names (`*.$RELAY_HOST`) → relay splice `:7443`; every other name, `$RELAY_HOST` included → `:8443` |

**Why the HTTPS server listens on `127.0.0.1:8443` and not on 443:** port 443 carries two different kinds of traffic.

| Traffic | TLS ends at | Who must handle it |
|---|---|---|
| `https://$RELAY_HOST/…`: gaviero's control WebSocket, the phone's notification stream, `/healthz` | **nginx**, with the Let's Encrypt certificate | An ordinary HTTPS server that proxies to the relay's `:7080` |
| TLS to a route name (`<workspace-id>-<machine>.$RELAY_HOST`): the phone's live session | **The gaviero machine**, with its private CA | Nobody on the VPS may decrypt it. The relay forwards the raw bytes (`:7443`) |

An `http {}` server on 443 would try to terminate every connection, which would break the end-to-end TLS of the second kind. So 443 belongs to the `stream {}` block in `nginx-stream.conf`.
- That block reads only the server name from the ClientHello (`ssl_preread`), without decrypting anything.
- It then forwards the still-encrypted connection.
- Only route names (`*.$RELAY_HOST`) go to the relay's splice port.
- Everything else, `$RELAY_HOST` and **all your other HTTPS sites**, goes to one shared internal port, `127.0.0.1:8443`. There nginx terminates TLS and picks the site by `server_name`, exactly as it did on 443.
- That port is reachable only through the stream block, never from the internet directly.
- `proxy_protocol` on that listener recovers the client's IP address, which would otherwise always be 127.0.0.1.

**Why there are separate `streams-*` directories:**
- `sites-enabled/*` is included **inside** `http {}`, but a `stream {}` block must sit at the **top level** of nginx.conf. It can't go in `sites-*`.
- Step 4d therefore creates `streams-available` / `streams-enabled` with the same layout, plus one top-level `include`.

**4a. Packages and modules:**
```bash
sudo apt install nginx libnginx-mod-stream certbot python3-certbot-nginx
nginx -V 2>&1 | grep -o 'with-stream[^ ]*\|stream_ssl_preread\|http_realip'
ls /etc/nginx/modules-enabled/ | grep stream             # e.g. 50-mod-stream.conf
```
- **Built:** all three names must appear in `nginx -V`.
- **Loaded:** `with-stream=dynamic` (Debian/Ubuntu) means the module is built but loaded only by a `load_module` line. `libnginx-mod-stream` installs that line as `modules-enabled/50-mod-stream.conf`, and `nginx.conf` must keep `include /etc/nginx/modules-enabled/*.conf;` at its top.
- **Symptom when it isn't loaded:** `nginx -t` fails with `unknown directive "stream"`.
- **Fix:**
  - **Package missing:** install `libnginx-mod-stream`.
  - **Package installed but no loader file:** create it with:
    ```bash
    echo 'load_module modules/ngx_stream_module.so;' | sudo tee /etc/nginx/modules-enabled/50-mod-stream.conf
    ```

**4b. The certificate, before the site is enabled:**
```bash
sudo certbot certonly --nginx -d "$RELAY_HOST" --deploy-hook "systemctl reload nginx"
```
- **Why first:** `nginx-site.conf` points at this certificate, and nginx refuses to load a site whose `ssl_certificate` doesn't exist yet.
- **How certbot answers the challenge without our site:** nginx must be running with port 80 reachable. Certbot's nginx mode then serves the challenge from the `listen 80 default_server` block (Debian's `default` site), or adds a temporary block of its own, and removes it afterwards.
- **`certonly`** obtains the certificate without editing your nginx files.

**4c. The relay's site (port 80 + HTTPS):**

**Copy the site:**
```bash
sudo cp crates/gaviero-relay/deploy/nginx-site.conf /etc/nginx/sites-available/gaviero-relay
sudo nano /etc/nginx/sites-available/gaviero-relay
```

**Edit four lines** in the copy. Only the name changes, and the `listen` lines stay as they are.

Before:
```nginx
server {
    listen 80;
    listen [::]:80;
    server_name gaviero.example.com;
    …
}

server {
    listen 127.0.0.1:8443 ssl proxy_protocol;
    server_name gaviero.example.com;
    …
    ssl_certificate     /etc/letsencrypt/live/gaviero.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/gaviero.example.com/privkey.pem;
```

After (with your name):
```nginx
server {
    listen 80;
    listen [::]:80;
    server_name gaviero.yourdomain.tld;
    …
}

server {
    listen 127.0.0.1:8443 ssl proxy_protocol;
    server_name gaviero.yourdomain.tld;
    …
    ssl_certificate     /etc/letsencrypt/live/gaviero.yourdomain.tld/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/gaviero.yourdomain.tld/privkey.pem;
```

**Check and enable it:**
- The two certificate paths must match what `sudo certbot certificates` lists for this name (step 4b). A repeated issue can create `…/live/<name>-0001/`.
- `grep -n example /etc/nginx/sites-available/gaviero-relay` must print nothing.
- Then link it:
  ```bash
  sudo ln -s ../sites-available/gaviero-relay /etc/nginx/sites-enabled/gaviero-relay
  ```

**Notes:**
- **Renewals** are automatic (certbot's systemd timer) and go through this site's port-80 block, so keep port 80 open.
- **Check renewal** with `sudo certbot renew --dry-run` once step 4e is done.

**4d. The `:443` SNI router (`streams-available` / `streams-enabled`, top level):**
**Copy the router:**
```bash
sudo install -d /etc/nginx/streams-available /etc/nginx/streams-enabled
sudo cp crates/gaviero-relay/deploy/nginx-stream.conf /etc/nginx/streams-available/gaviero-relay
sudo nano /etc/nginx/streams-available/gaviero-relay
```

**Edit two map entries** in the copy. Keep the leading `*.` on the second one.

Before:
```nginx
    map $ssl_preread_server_name $gaviero_upstream {
        hostnames;
        gaviero.example.com    127.0.0.1:8443;   # the relay's own HTTPS (nginx-site.conf)
        *.gaviero.example.com  127.0.0.1:7443;   # route names: phone TLS → relay splice port (--splice)
        default                127.0.0.1:8443;   # every other name: all your HTTPS sites
    }
```

After (with your name):
```nginx
    map $ssl_preread_server_name $gaviero_upstream {
        hostnames;
        gaviero.yourdomain.tld    127.0.0.1:8443;   # the relay's own HTTPS (nginx-site.conf)
        *.gaviero.yourdomain.tld  127.0.0.1:7443;   # route names: phone TLS → relay splice port (--splice)
        default                   127.0.0.1:8443;   # every other name: all your HTTPS sites
    }
```

`grep -n example /etc/nginx/streams-available/gaviero-relay` must print nothing.

**Enable it:**
```bash
sudo ln -s ../streams-available/gaviero-relay /etc/nginx/streams-enabled/gaviero-relay
```

**Edit `/etc/nginx/nginx.conf` once.** This is the only change to that file. Add one `include` line at the very end, after the closing `}` of `http`.

Before:
```nginx
http {
    …
    include /etc/nginx/sites-enabled/*;
}
```

After:
```nginx
http {
    …
    include /etc/nginx/sites-enabled/*;
}

include /etc/nginx/streams-enabled/*;
```

- **Why at the very end:** that puts the `include` at top level, outside `http {}`, which is where `stream {}` must be.
- **Inside `http {}` it fails:** if it lands there, `nginx -t` reports `"stream" directive is not allowed here`.

Before reloading, move your other HTTPS sites first; see [Other HTTPS sites on the same nginx](#other-https-sites-on-the-same-nginx). If any site in `sites-enabled` still has `listen 443`, nginx can't bind the port.

**4e. Validate and reload:**
```bash
sudo nginx -t && sudo systemctl reload nginx
```

#### Other HTTPS sites on the same nginx

Once the `stream` block owns `:443`, **nothing else may `listen 443`** on this host. That includes ntfy while you still run it.

**What stays the same:**
- Your sites keep their certificates, their `location`/`proxy_pass` blocks and their port-80 redirect blocks.
- TLS still ends at nginx with each site's Let's Encrypt certificate.
- The stream map needs no change: every name that isn't a relay route already goes to `127.0.0.1:8443`.

**The only change is each site's `listen 443` lines.**

To find the sites, run `grep -ln 'listen.*443' /etc/nginx/sites-enabled/*`.

A typical certbot-managed site, before:

```nginx
server {
    server_name app.example.com;
    location / { proxy_pass http://127.0.0.1:3000; … }

    listen [::]:443 ssl ipv6only=on; # managed by Certbot
    listen 443 ssl; # managed by Certbot
    ssl_certificate /etc/letsencrypt/live/app.example.com/fullchain.pem; # managed by Certbot
    ssl_certificate_key /etc/letsencrypt/live/app.example.com/privkey.pem; # managed by Certbot
    include /etc/letsencrypt/options-ssl-nginx.conf; # managed by Certbot
    ssl_dhparam /etc/letsencrypt/ssl-dhparams.pem; # managed by Certbot
}
```

After (only the two `listen` lines change, plus two new lines):

```nginx
server {
    server_name app.example.com;
    location / { proxy_pass http://127.0.0.1:3000; … }

    listen 127.0.0.1:8443 ssl proxy_protocol;   # was: listen 443 ssl + listen [::]:443 ssl
    set_real_ip_from 127.0.0.1;                 # take the client IP from the PROXY line,
    real_ip_header proxy_protocol;              #   so $remote_addr / logs stay correct
    ssl_certificate /etc/letsencrypt/live/app.example.com/fullchain.pem; # managed by Certbot
    ssl_certificate_key /etc/letsencrypt/live/app.example.com/privkey.pem; # managed by Certbot
    include /etc/letsencrypt/options-ssl-nginx.conf; # managed by Certbot
    ssl_dhparam /etc/letsencrypt/ssl-dhparams.pem; # managed by Certbot
}
```

**Rules for the shared port:**
- **Every** site uses the same `listen 127.0.0.1:8443 ssl proxy_protocol;`. Same port, same options; no per-site ports.
- **Drop the IPv6 `listen [::]:443` line.** The stream block already listens on `[::]:443` and forwards over IPv4 loopback.
- **Drop any `quic` / `listen 443 quic` lines.** HTTP/3 would need UDP 443, which this setup doesn't carry.
- **HTTP/2:** keep `http2 on;` if a site has it, since it's per site. But replace the old `listen 443 ssl http2;` flag with the plain line above: on a shared port, that flag would switch HTTP/2 on for every site, including the relay's, which must stay HTTP/1.1 for its WebSockets.
- **`default_server`:** if one site had `listen 443 ssl default_server`, keep the flag on exactly one site: `listen 127.0.0.1:8443 ssl proxy_protocol default_server;`.
- **Don't change the port-80 blocks** (the `return 301 https://…` redirects certbot wrote). Renewals use them.

**Then:**
1. Validate and reload: `sudo nginx -t && sudo systemctl reload nginx`.
2. Check each site: `curl -sI https://app.example.com | head -1`.
3. Check renewals still work: `sudo certbot renew --dry-run`.

**Adding a site later:** give it the same `listen 127.0.0.1:8443 ssl proxy_protocol;` and the two `real_ip` lines. If you run `certbot --nginx` (install mode), it may add `listen 443 ssl; # managed by Certbot` again; replace that line, then run `nginx -t`.

### 5. Firewall

Open 80 and 443 only. The relay's own ports (7080, 7443) and nginx's shared HTTPS listener (8443) are bound to 127.0.0.1. With ufw:

```bash
sudo ufw allow OpenSSH && sudo ufw allow 80,443/tcp && sudo ufw enable
```

### 6. Check

On the VPS:
```bash
curl -sS "https://$RELAY_HOST/healthz"                             # ok
curl -sSi "https://$RELAY_HOST/v1/phone/stream" | head -1         # HTTP/1.1 401 (no secret)
curl -sSN --max-time 5 -H "Authorization: Bearer $(head -1 ~/.config/gaviero-relay/secret)" \
  "https://$RELAY_HOST/v1/phone/stream"                            # {"type":"presence","epoch":"…","routes":[]}
```

- **`-S` matters.** `-s` on its own hides every curl error, so a TLS failure or a timeout prints nothing at all.
- The stream never ends by itself, so the last command finishing with `curl: (28) Operation timed out` **after** the presence line is expected.
- **Nothing printed, or only the timeout:** run the last command again with `-v` instead of `-sS`, then ask the relay directly, bypassing nginx:
  ```bash
  curl -sSN --max-time 5 -H "Authorization: Bearer $(head -1 ~/.config/gaviero-relay/secret)" \
    http://127.0.0.1:7080/v1/phone/stream
  ```
  If that prints the presence line, the relay is fine and the fault is in nginx (the table below). If it doesn't, check `journalctl --user -u gaviero-relay`.

| Symptom | Likely cause |
|---|---|
| `curl: (28)` with no line before it | Something between nginx and the relay buffers the stream: the request reached a `server` other than `nginx-site.conf`'s (check with `-v` which certificate and headers come back), or the installed site lacks `proxy_buffering off` |
| `HTTP/1.1 404` with an empty body | The running relay binary predates `/v1/phone/stream`: rebuild and reinstall it ([Updating](#updating)) |
| Connection refused or timeout | DNS, firewall, or the stream block isn't loaded (step 4d: the `streams-enabled` link or the top-level `include`) |
| Certificate error, e.g. `curl: (60) no alternative certificate subject name matches` | No `server` on `127.0.0.1:8443` has `server_name $RELAY_HOST`, so nginx answers with another site's certificate. To see which one, run `openssl s_client -connect $RELAY_HOST:443 -servername $RELAY_HOST </dev/null 2>/dev/null \| openssl x509 -noout -subject`. Then check step 4c: the site is linked in `sites-enabled`, its `listen` is `127.0.0.1:8443` (an older template used 8444), and both `server_name` lines carry your name. Also check that the map sends `$RELAY_HOST` to `:8443` (step 4d) |
| One of your other sites shows the wrong certificate or a connection error | That site still has `listen 443`, or it lacks `proxy_protocol` on its `127.0.0.1:8443` listen line |
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
    "host": "gaviero.example.com",
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

gaviero no longer publishes to ntfy. If this VPS still runs an ntfy server from an earlier setup, retire it once relay alerts work on your phone:
1. stop and disable the ntfy user service;
2. remove its link from `sites-enabled`, then reload nginx and delete the DNS record;
3. run `sudo certbot delete --cert-name ntfy.example.com`.

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

### Changing the relay's name

1. **On the VPS:**
   - point the new DNS record at the VPS;
   - set `RELAY_HOST` to the new name;
   - change the name in the installed copies, using the same lines as in steps 3, 4c and 4d;
   - issue a certificate for the new name (step 4b);
   - reload nginx and restart the relay.
2. **On every machine,** update `remote.relay.host` and restart gaviero.
3. **On the phone,** re-pair each machine. Route names include the relay's name.
