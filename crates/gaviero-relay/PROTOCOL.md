# gaviero-relay protocol v1

The prose for the types in [`gaviero_remote::relay_proto`](../gaviero-remote/src/relay_proto.rs). That module is the single definition: the relay (`proto`) and the host-side client ([`relay_client`](../gaviero-remote/src/relay_client.rs)) both use it. [`tests/e2e.rs`](tests/e2e.rs) drives the client against this relay.

## Topology

```
phone ──TLS (SNI = route)──▶ nginx stream :443 ──▶ splice port ──▶ (data WS) ──▶ host sidecar 127.0.0.1
host  ──wss──▶ nginx https (gaviero.<domain>) ──▶ http port: /v1/host, /v1/data/{nonce}, /v1/phone/stream
```

- **TLS is end to end.** The phone's TLS session terminates at the host's sidecar, which uses a pinned per-machine CA. The relay sees the SNI and ciphertext only.
- **Auth:** every HTTP/WebSocket endpoint requires `Authorization: Bearer <secret>`, compared in constant time; otherwise `401`. The relay is not the security boundary: someone holding the secret can at most squat on routes, which causes denial of service, because the phone's pinned CA refuses a foreign leaf.

## Routes

- A route is the full TLS server name the phone uses: `<label>.<public-host>`. gaviero picks `<workspace-id16>-<machine>.<relay host>`.
- It must be lowercase DNS labels of `[a-z0-9-]` (no leading or trailing `-`), each label 1–63 characters, 253 characters in total.
- With `--public-host`, the route must be a single label directly under it, and never the public host itself.
- **The newest registration of a route wins**, so a restarted TUI replaces its stale entry at once.
- When a host's control socket closes, all of its routes are dropped.

## Host control — `GET /v1/host` (WebSocket, JSON text frames ≤ 64 KiB)

**Host → relay:**

```json
{"type":"register","route":"0b7d…-desk.gaviero.example.com","machine":"desk.gaviero.example.com","workspace_id":"0b7d…","display_name":"gaviero"}
{"type":"unregister","route":"…"}
{"type":"notify","machine":"desk.gaviero.example.com","workspace_id":"0b7d…","conv":"c1","kind":"agent_finished"}
```

- `kind` is `agent_finished` or `agent_waiting`.
- Notifications are rate-limited to 20 per second per connection.
- There are at most 32 routes per connection.

**Relay → host:**

```json
{"type":"registered","route":"…"}
{"type":"rejected","route":"…","reason":"…"}
{"type":"dial","route":"…","nonce":"<32 hex>"}
{"type":"error","message":"…"}
```

- **Keepalive:** the relay pings every 25 s and drops a connection that has been silent for 75 s. The host pings at the same rate.

## Host data — `GET /v1/data/{nonce}` (WebSocket, binary frames)

This endpoint is opened by the host after a `dial`. The nonce is single-use, and an unknown or expired nonce returns `404`.

1. The relay first sends the phone's buffered ClientHello as one binary frame.
2. After that, both sides exchange raw TLS bytes until either side ends.
3. **Teardown is always full:** if either direction ends, or nothing moves for 180 s, both the phone TCP connection and the WebSocket are closed.

## Phone live control — splice port (raw TCP, routed by SNI)

1. If the relay runs with `--proxy-protocol`, one PROXY v1 line (nginx `proxy_protocol on`) precedes the TLS. It is required and yields the client IP.
2. The relay reads the ClientHello, at most 16 KiB within 10 s. It routes on SNI.
   - An unknown route or a missing SNI closes the connection, and the phone's handshake fails.
3. The relay sends `dial` to the route's host and waits up to 10 s for `/v1/data/{nonce}`, then splices.
4. Each client IP can hold at most 32 concurrent sessions.

## Phone notifications — `GET /v1/phone/stream?since=<id>&epoch=<epoch>` (NDJSON)

`application/x-ndjson`, with one JSON object per line and nginx buffering disabled (`X-Accel-Buffering: no`). The stream sends, in order:

1. `{"type":"presence","epoch":"<16 hex>","routes":[{route, machine, workspace_id, display_name}, …]}` on connect and on every change. The phone can use it as its instance directory.
2. Every stored notification with `id > since`, oldest first. Notifications are kept for 6 h (`--note-ttl-hours`), up to 200.
3. New notifications as they arrive:
   `{"type":"notify","id":7,"ts":1760000000,"machine":"…","workspace_id":"…","conv":"…","kind":"agent_waiting"}`
4. `{"type":"keepalive"}` every 25 s. Clients should treat 75 s of silence as a dead connection.

`id`s are monotonic within one relay **epoch**: a random value chosen at every process start and sent in each `presence`. The phone persists `(epoch, last id)` and reconnects with `since=<id>&epoch=<epoch>`.
- **A different epoch** means the relay restarted and ids began again at 1. The relay then ignores `since` and replays everything it has stored.
- **On the phone**, when a `presence` carries a new epoch, the phone resets its stored id before the replay that follows.

This was found by the emulator end-to-end run: without the epoch, a restarted relay's `id 1` was silently skipped.

Notifications carry identifiers only. The phone renders the text from names it already knows, so the relay never sees prompts, answers or file names.
