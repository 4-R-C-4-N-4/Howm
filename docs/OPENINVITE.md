# Open Invitations

## Problem

Current invites are one-time-use and expire after 15 minutes. To connect with someone, you generate an invite, send it privately, and they redeem it before it expires. This works for intentional 1:1 connections but doesn't support the social discovery case: "Here's my Howm, anyone can connect."

An open invitation is a stable link you can post on your website, social bio, forum signature, or QR code. Anyone who has it can connect to your node without you being online to generate a fresh invite each time.

## Design

### The Open Invite Link

```
howm://open/<base64url(node_id|wg_pubkey|endpoint|daemon_port|sig)>
```

An open invite contains your node's **public** connection info — everything a peer needs to initiate a handshake — signed by your node's identity key so it can't be tampered with.

Unlike a regular invite, an open invite:
- Has **no expiry** (or an optional long expiry set by the owner)
- Has **no pre-shared key baked in** (PSK is negotiated per-connection)
- Has **no pre-assigned IP** (IPs are assigned on demand when someone connects)
- Can be **redeemed by multiple peers** (not consumed on use)
- Can be **revoked** by the owner at any time

### Connection Flow

```
   Joiner (has the open invite link)          Host (published the link)
   ─────────────────────────────────          ────────────────────────

   1. Decode link → get host's
      pubkey, endpoint, daemon port

   2. POST /node/open-join ──────────────────▶ 3. Validate signature on the
      {                                            open invite token
         open_token: "...",
         my_pubkey: "abc...",                  4. Check rate limit + max peers
         my_endpoint: "1.2.3.4:51820",
         my_daemon_port: 7000                  5. Assign WG IP for joiner
      }
                                               6. Generate PSK for this pair

                                               7. Add joiner as WG peer

   8. Receive response ◀─────────────────────  8. Return:
      {                                            {
        assigned_ip: "10.47.0.5",                    assigned_ip, psk,
        psk: "...",                                  host_wg_address
        host_wg_address: "10.47.0.1"               }
      }

   9. Add host as WG peer with
      received PSK + known pubkey

  10. Configure WG with assigned IP

  11. Verify tunnel via
      GET /node/info over WG ────────────────▶ 12. Discovery loop picks up
                                                    new peer, updates name/id
```

Key differences from the current invite flow:
- **No complete-invite callback needed.** The host adds the WG peer server-side during the `/open-join` request and returns the PSK directly. The current flow needs a callback because both sides need to exchange keys; here the joiner sends their pubkey in the initial request.
- **The host must be reachable.** Current invites encode all connection info so the redeemer can initiate. Open invites still require the host to be online when someone joins (they need to assign an IP and add the WG peer). This is fine — you can't connect to an offline node anyway.

### Security Analysis

**What's in the open invite (public info):**
- Node ID — already public via `/node/info`
- WireGuard public key — must be public for WG to work
- Endpoint (IP:port) — must be known to connect
- Daemon port — must be known to call the API
- Signature — proves the invite was created by this node

**What's NOT in the open invite:**
- No PSK (negotiated per-connection, unique per peer pair)
- No WG private key (never leaves the host)
- No pre-assigned IPs (assigned dynamically)

**Threat model:**

| Threat | Mitigation |
|--------|------------|
| Spam connections | Rate limiting on `/open-join` (configurable, e.g. 10/hour) |
| IP exhaustion | Max open-invite peers limit (configurable, e.g. 256) |
| DDoS via WG | WG itself is DDoS-resistant (silent to unauthenticated packets) |
| Impersonation (fake invite) | Invite is signed by node's identity key; joiner verifies |
| Unwanted peer | Host can remove any peer via `DELETE /node/peers/:id` |
| Stale endpoint | Owner regenerates open invite if their IP changes |

**You're right that this isn't a huge security concern** for a social app. The WireGuard tunnel itself provides strong encryption and authentication — once peered, traffic is as secure as any WG connection. The open invite just lowers the barrier to becoming peers. The main risk is resource exhaustion (too many peers), which is handled by rate limiting and max peer caps.

### Trust Levels (Foundation)

Open invite peers should eventually have different default permissions than peers added via private invite. This spec doesn't implement a full permission system, but establishes the **trust level** field on peers as a foundation:

```rust
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum TrustLevel {
    /// Added via private invite or manual peer add. Full access.
    Friend,
    /// Connected via open invite. Limited access by default.
    Public,
    /// Manually restricted. Minimal access.
    Restricted,
}
```

For now, `Public` and `Friend` peers have identical access. But the field exists so future capabilities can gate behavior:

| Capability | Friend | Public | Restricted |
|------------|--------|--------|------------|
| View public files | yes | yes | no |
| View feed | yes | yes | no |
| Post to feed | yes | no | no |
| Install capabilities | yes | no | no |
| Visit howm.world | yes | yes | read-only |
| gaming.portal | yes | yes | yes |
| See peer list | yes | limited | no |

These are suggestions for future work — not enforced by this spec.

---

## Data Model

### Open Invite Configuration

Stored at `{data-dir}/open_invite.json`:

```json
{
  "enabled": true,
  "token": "base64url(...)",
  "created_at": 1710000000,
  "expires_at": null,
  "max_peers": 256,
  "rate_limit_per_hour": 10,
  "current_peer_count": 12,
  "label": "ivy's howm — connect freely"
}
```

### Peer (updated)

```rust
pub struct Peer {
    pub node_id: String,
    pub name: String,
    pub wg_pubkey: String,
    pub wg_address: String,
    pub wg_endpoint: String,
    pub port: u16,
    pub last_seen: u64,
    pub trust: TrustLevel,  // NEW — Friend | Public | Restricted
}
```

Existing peers default to `Friend`. Peers added via open invite default to `Public`.

### Signing

The open invite token is signed using the node's WireGuard private key (x25519 → used as an Ed25519-compatible signing key via the standard birational map, or simpler: HMAC-SHA256 with the private key as the secret).

Simpler approach (recommended for MVP): just HMAC-SHA256.

```
payload = node_id | wg_pubkey | endpoint | daemon_port
signature = HMAC-SHA256(wg_private_key, payload)
token = base64url(payload | signature)
```

The joiner can't verify the signature without the private key — but that's fine. The signature's purpose is to prevent third parties from modifying the invite (e.g. changing the endpoint to redirect connections). The joiner trusts the invite because they got it from a source they trust (the host's website, social profile, etc.). The host verifies incoming `/open-join` requests by checking that the token's signature matches.

---

## API Changes

### New Endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/node/open-invite` | Bearer | Create or regenerate open invite |
| GET | `/node/open-invite` | Bearer | Get current open invite status |
| DELETE | `/node/open-invite` | Bearer | Revoke open invite |
| POST | `/node/open-join` | None | Join via open invite (called by joiner) |

### POST `/node/open-invite` — Create Open Invite

Request:
```json
{
  "label": "ivy's howm",
  "max_peers": 256,
  "rate_limit_per_hour": 10,
  "expires_at": null
}
```

Response:
```json
{
  "invite_link": "howm://open/abc123...",
  "created_at": 1710000000,
  "label": "ivy's howm"
}
```

Creating a new open invite revokes any previous one (only one active at a time).

### GET `/node/open-invite` — Status

Response:
```json
{
  "enabled": true,
  "invite_link": "howm://open/abc123...",
  "label": "ivy's howm",
  "created_at": 1710000000,
  "expires_at": null,
  "max_peers": 256,
  "current_peer_count": 12,
  "rate_limit_per_hour": 10
}
```

### DELETE `/node/open-invite` — Revoke

Disables the open invite. Existing peers connected via open invite are NOT disconnected — they remain as peers. Only new connections via the old link are rejected.

### POST `/node/open-join` — Join (unauthenticated)

Request:
```json
{
  "open_token": "abc123...",
  "my_pubkey": "base64...",
  "my_endpoint": "1.2.3.4:51820",
  "my_daemon_port": 7000,
  "my_node_id": "uuid",
  "my_name": "bob-laptop"
}
```

Response (success):
```json
{
  "assigned_ip": "10.47.0.5",
  "psk": "base64...",
  "host_wg_address": "10.47.0.1",
  "host_daemon_port": 7000,
  "host_name": "ivy-desktop",
  "host_node_id": "uuid"
}
```

Error responses:
- `410 Gone` — open invite revoked or expired
- `429 Too Many Requests` — rate limited
- `507 Insufficient Storage` — max peers reached
- `409 Conflict` — peer with this pubkey already connected

---

## CLI / Config

### New daemon flags

| Flag | Env | Default | Description |
|------|-----|---------|-------------|
| `--open-invite-max-peers` | `HOWM_OPEN_MAX_PEERS` | `256` | Max peers via open invite |
| `--open-invite-rate-limit` | `HOWM_OPEN_RATE_LIMIT` | `10` | Max joins per hour |

### Generating via CLI

```bash
# Create open invite
curl -X POST localhost:7000/node/open-invite \
  -H 'Authorization: Bearer <token>' \
  -H 'Content-Type: application/json' \
  -d '{"label": "ivy howm — connect freely"}'

# Get the link
curl localhost:7000/node/open-invite \
  -H 'Authorization: Bearer <token>'

# Revoke
curl -X DELETE localhost:7000/node/open-invite \
  -H 'Authorization: Bearer <token>'
```

### Joining via CLI

```bash
# Join someone's open howm
daemon join howm://open/abc123...
# or via API:
curl -X POST <host-endpoint>/node/open-join \
  -H 'Content-Type: application/json' \
  -d '{"open_token": "abc123...", "my_pubkey": "...", ...}'
```

---

## Implementation Plan

### Step 1: Trust level on peers
- Add `TrustLevel` enum to `peers.rs`
- Add `trust` field to `Peer` struct (default `Friend` for existing peers)
- Serde: deserialize missing field as `Friend` for backwards compat

### Step 2: Open invite CRUD
- New file: `open_invite.rs`
- Create/read/revoke open invite config
- HMAC-SHA256 signing with WG private key
- Token encoding/decoding
- Persistence to `{data-dir}/open_invite.json`

### Step 3: Open join endpoint
- `/node/open-join` handler in `node_routes.rs`
- Token validation (verify HMAC)
- Rate limiting (reuse existing `RateLimiter`)
- Max peer check
- Assign WG IP, generate PSK, add WG peer
- Add to peers list with `TrustLevel::Public`
- Return connection info

### Step 4: Joiner-side support
- Decode `howm://open/` links
- POST to host's `/node/open-join` with own WG info
- Configure local WG peer with received PSK + host info
- Verify connectivity

### Step 5: UI integration
- Open invite toggle in web UI settings
- Display invite link + QR code
- Show open-invite peer count
- Allow promoting `Public` → `Friend` or demoting to `Restricted`

---

## Open Questions

1. **One open invite or many?** This spec allows only one active open invite at a time. Could there be value in multiple open invites with different labels/limits (e.g. one for your blog, one for a conference)? Probably not for MVP.

2. **Peer discovery via open invite peers.** If Alice connects to Bob via open invite, and Carol also connects to Bob, should Alice and Carol discover each other? Currently the discovery loop would expose them to each other through Bob's peer list. This might be undesirable — open-invite peers might not want to be visible to each other. The trust level system could gate this: `Public` peers only see `Friend` peers in the peer list.

3. **Auto-cleanup of stale open-invite peers.** If someone joins via open invite and then goes offline permanently, they consume a WG IP. Should there be an auto-prune for `Public` peers not seen in X days? Probably yes, with a configurable threshold.

4. **Mutual peering.** In the current flow, both sides become peers of each other. With open invite, should it be asymmetric? The joiner peers with the host, but does the host automatically become a peer of the joiner? For social feed aggregation to work both ways, yes. But the joiner might not want to run a publicly reachable node. Worth considering a "follow" vs "peer" distinction in the future.
