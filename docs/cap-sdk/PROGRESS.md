# Capability SDK Refactor — Progress

Last updated: 2026-04-12

## Task Status

| # | Task | Status | Notes |
|---|---|---|---|
| 1 | R2 — Move RPC dispatch helpers into SDK | **Done** | `p2pcd::capability_sdk::rpc` module with `extract_method`, `extract_inner_payload`, `extract_request_id`. Core handler keys made `pub`, SDK re-exports. Three private copies deleted from files/messaging/voice. |
| 2 | R1a — Scaffold CapabilityApp builder + run loop | **Done** | `CapabilityApp<S>` generic builder with `with_routes`, `with_inbound_handler`, `with_ui`, `with_body_limit`, `spawn_task`, `run`. Plus `LocalPeerId::lazy`, `init_tracing`, SPA UI serving with MIME table + cache-control. |
| 3 | R1b — SQLite bootstrap helper in SDK | **Done** | `p2pcd::cap_db::open_sqlite(path)` behind `cap-db` feature. Applies WAL/busy_timeout/foreign_keys pragmas, creates parent dirs. |
| 4 | R1c — Migrate feed cap (pilot) | **Done** | 163 → 99 lines. Removed serve_ui/ui_mime (44 lines), health handler, manual bind/serve. |
| 5 | R1d — Migrate presence, voice, messaging, files | **Done** | All 5 caps migrated. Total: 943 → 636 lines (-307). messaging uses `LocalPeerId`. All 137 cap tests + 293 node tests pass. |
| 6 | Bootstrap wallet + world caps | Open | deffer |
| 7 | R4 — Split engine.rs into subdirectory | Open | |
| 8 | R5 — Split bridge.rs by domain | Open | |
| 9 | R6 — Group connectivity into subfolder | Open | |
| 10 | Split p2pcd-types/lib.rs into submodules | Open | |
| 11 | R7 — TrustGate / PeerCache traits (deferred) | Open | Do when pain is concrete. |
| 12 | R8 — EventBus daemon-internal subscribers (deferred) | Open | Do when first subscriber needs it. |

## Additional fixes shipped alongside the SDK work

These were discovered during testing and fixed in-session. They are NOT part of the original TASKS.md plan but are significant.

### Daemon / protocol fixes

- **Blob handler per-peer senders** — `BlobHandler::send_tx` was never set in production; every `BLOB_OFFER`/`BLOB_CHUNK` was silently dropped. Blob transfers have never worked until this fix. Converted to per-peer `peer_senders` map (same pattern as the RPC handler fix from peer-cap). Wired in engine session setup/teardown.
- **All 6 remaining core handlers** (stream, latency, timesync, attest, peerexchange, endpoint) had the same `send_tx` bug. All converted to per-peer senders and wired in engine.
- **RPC method→capability routing** — `forward_rpc_to_capability` iterated the active_set alphabetically and forwarded to the first registered endpoint (usually feed, port 7001). Added a `preferred_cap` routing table matching method prefix to capability name (`dm.*` → messaging, `catalogue.*` → files, `voice.*` → voice, etc.).
- **CBOR wire format body limit** — SDK's `CapabilityApp` now applies `DefaultBodyLimit::disable()` + `RequestBodyLimitLayer` so caps with large uploads (feed 50MB, files 500MB) aren't capped by axum's 2MB default.
- **SSE snapshot `wg_address`** — was hardcoded `null`; now resolved via `engine.peer_wg_ip()`. Bridge `/peers` endpoint also returns `wg_address`.
- **`howm.sh` stale `howm0` cleanup** — tears down the WG interface on startup so the port probe doesn't fall back to 41642.
- **`howm.sh` explicit `--data-dir`** — always passes `$HOME/.local/share/howm` to avoid `dirs::data_local_dir()` resolving differently under sudo.
- **Access schema v4 migration** — added `howm.social.presence.1` and `howm.social.voice.1` to the `howm.friends` capability rules. Both were missing, so the trust gate filtered them out during capability exchange.

### Presence fixes

- **Capability name `.0` → `.1`** — PeerStream subscribed to `howm.social.presence.0` but daemon registered `.1`. Fixed.
- **Type-3 PeerStream** — switched from Type-2 to Type-3 (pre-built tracker) so the on_active hook can look up `wg_address` from the tracker for gossip address resolution.
- **Gossip `peer_addresses` population** — on_active hook now resolves WG address from tracker (live events) or bridge fallback (snapshot), populating the address map so gossip sender/receiver can function.
- **Diagnostic logging** — added info-level logs to `set_status`, `send_immediate_broadcast`, gossip sender tick, and gossip receiver.

### Voice fixes

- **Removed presence dependency** — voice UI fetched peers from `/cap/presence/peers`. Now uses its own `/peers` endpoint backed by PeerStream tracker (same pattern as files/messaging).
- **Added `BASE` path detection** — voice.js lacked the `/cap/voice` prefix for API calls through the daemon proxy. All `fetch()` and WebSocket URLs now use `${BASE}`.
- **`create_room` sends invites** — previously only stored invite list as metadata without sending RPC invites. Now fires `voice.invite` RPC to each peer.
- **Placeholder rooms for invites** — incoming `voice.invite` RPC now creates a local placeholder room so `GET /rooms` returns it and the UI shows Join/Decline.
- **`is_invited` / `is_member` server-side** — `GET /rooms` response includes computed flags so the UI doesn't need to match peer IDs client-side (random localStorage ID never matched WG pubkeys).
- **`/me` endpoint** — returns the caller's identity as seen by the server (from proxy-injected `X-Node-Id`). Used by the WS handshake which bypasses the proxy.
- **WebSocket direct connection** — daemon proxy doesn't support WS upgrade; voice.js now connects directly to cap port 7005 for signaling.
- **`join_room` accepts placeholder invites** — bypass the invite-list check for placeholder rooms (local user's ID isn't in the list since the RPC handler doesn't know it).
- **Error handling** — `enterRoom` and `refreshRoom` now check `resp.ok` before parsing JSON.

### Files fixes

- **`escAttr` for Download button** — `escHtml` doesn't escape `"` (innerHTML limitation). Download button's `data-offering` attribute was truncated at the first `"` in the JSON, causing `JSON.parse` to fail with "Unexpected end of input". Added `escAttr()` that escapes `"`, `'`, `&`, `<`, `>`.
- **Download retry on stale records** — `initiate_download` now marks blobs already in the daemon store as complete (instead of returning CONFLICT), and clears stale "transferring" records on retry.

### iframe permissions

- **`allow="microphone; camera"`** — added to both `FabLayer.tsx` and `CapabilityPage.tsx` iframe elements so Brave/Chrome allow `getUserMedia` in capability iframes.
- **`allow-modals`** — added to CapabilityPage sandbox so `prompt()` works for room name input.
