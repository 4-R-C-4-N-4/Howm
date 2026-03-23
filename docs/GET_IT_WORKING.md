# Get It Working

Status audit and fix list for getting Howm into a usable two-peer demo state.

---

## 1. Invite System Is Broken Without `--wg-endpoint`

**Problem:** Both invite types (one-time and open) embed the host's WireGuard
endpoint into the token. The fallback chain is:

```
endpoint_override → identity.wg_endpoint → "0.0.0.0:51820"
```

If `--wg-endpoint` is never passed, the token contains `0.0.0.0:51820`. When the
joiner redeems, it extracts the host IP from that endpoint and makes an HTTP call
to `http://0.0.0.0:{daemon_port}/node/complete-invite` (or `/node/open-join`),
which is not routable and fails silently.

**Root cause:** `invite.rs:50-52`, `open_invite.rs:38-40` — no validation that
the endpoint is actually reachable before encoding it into the token.

**Fixes needed:**

- [ ] **Refuse to generate invites when endpoint is `0.0.0.0`.**
  Return an error like: `"Cannot create invite: WireGuard endpoint not configured.
  Restart with --wg-endpoint <public-ip:port> or set HOWM_WG_ENDPOINT."`
  Files: `invite.rs:generate()`, `open_invite.rs:create()`

- [ ] **Surface this in the UI.** When invite generation fails, show the actual
  error message (not just "Failed — is the API token set?").
  Files: `PeerList.tsx:77`, `OpenInviteSection.tsx:58`

- [ ] **Auto-detect public IP (stretch).** Use a STUN query or
  `https://api.ipify.org` at startup to populate `identity.wg_endpoint` when
  `--wg-endpoint` is not provided. Log the detected IP so the user can verify.
  File: `wireguard.rs` or new `net_detect.rs`

- [ ] **Show endpoint status on Dashboard.** The WireGuard card already shows
  the endpoint field but it will be empty/null when unconfigured. Add a warning
  banner: "WireGuard endpoint not set — invites will not work."
  File: `Dashboard.tsx` WireGuard section

### Invite Redemption Flow (for reference)

```
Inviter                              Joiner
────────                             ──────
POST /node/invite
  → token with pubkey, endpoint,
    wg_addr, psk, assigned_ip,
    daemon_port, expiry
                          ──token──→ POST /node/redeem-invite {code}
                                       decode token
                                       add inviter as WG peer
                                       HTTP POST to inviter's
                                         /node/complete-invite
                                         (uses endpoint IP from token!)
  ← complete-invite ←──────────────    send own pubkey, endpoint, addr
    validate PSK
    add joiner as WG peer
    save to peers.json
                                     wait 2s for WG handshake
                                     verify via GET /node/info over WG
                                     save to peers.json (trust: Friend)
```

The same pattern applies to open invites (`/node/open-join`).

---

## 2. Both Peers Need Public Reachability

The invite system requires both sides to reach each other:

- **UDP 51820** (WireGuard) — the tunnel itself
- **TCP {daemon_port}** (HTTP) — the `/node/complete-invite` or `/node/open-join`
  call happens over the public internet, not the WG tunnel (it can't — the tunnel
  doesn't exist yet at that point)
- **TCP {p2pcd_port}** (default 7654) — P2P-CD session negotiation after WG
  handshake, runs over the WG tunnel

**For two machines on the same LAN:** Use private IPs as the endpoint.
```
./howm.sh --wg-endpoint 192.168.1.10:51820
```

**For two machines on the internet:** Both need public IPs or port-forwarded ports.
```
./howm.sh --wg-endpoint myhost.example.com:51820
```

**Fix needed:**

- [ ] **Document this requirement clearly** in the startup banner and Dashboard.
  Right now there's no indication that `--wg-endpoint` is effectively required for
  any multi-peer use.

- [ ] **The joiner also needs reachability.** During open-join, the joiner sends
  `my_endpoint` to the host. If the joiner's endpoint is also `0.0.0.0`, the host
  adds an unreachable WG peer. The joiner's endpoint comes from
  `identity.wg_endpoint` (same fallback chain).
  File: `node_routes.rs` — `redeem_open_invite()` around line 630

---

## 3. Dashboard Uses Light-Theme Hardcoded Colors

The shell (`App.tsx`, `index.css`, `theme.css`) was updated to a dark theme using
`--howm-*` CSS variables. But the Dashboard and all its sub-components still use
hardcoded light-theme colors:

| Component | Examples of stale colors |
|-----------|------------------------|
| `Dashboard.tsx` | `background: '#fff'`, `border: '1px solid #e5e7eb'`, `color: '#374151'`, `color: '#888'` |
| `PeerList.tsx` | `background: '#f3f4f6'`, `border: '1px solid #ddd'`, `background: '#f0f9ff'`, trust badges with light backgrounds |
| `OpenInviteSection.tsx` | `background: '#f3f4f6'`, `background: '#f0fdf4'`, `background: '#fff'` |
| `CapabilityList.tsx` | `border: '1px solid #eee'`, `color: '#888'` |

**Fix needed:**

- [ ] **Retheme all Dashboard components** to use `var(--howm-*)` properties.
  Every inline style with a hardcoded hex color needs to be replaced. The
  `Settings.tsx` page was already written with the dark theme and can serve as
  the reference pattern. Key mappings:

  | Light value | Replace with |
  |------------|-------------|
  | `#fff` (card bg) | `var(--howm-bg-surface, #232733)` |
  | `#e5e7eb` (border) | `var(--howm-border, #2e3341)` |
  | `#f3f4f6` (button bg) | `var(--howm-bg-elevated, #2a2e3d)` |
  | `#f9fafb` (form bg) | `var(--howm-bg-secondary, #1a1d27)` |
  | `#4f46e5` (accent) | `var(--howm-accent, #6c8cff)` |
  | `#888`, `#6b7280` (muted text) | `var(--howm-text-muted, #5c6170)` |
  | `#374151` (text) | `var(--howm-text-primary, #e1e4eb)` |

  Files: `Dashboard.tsx`, `PeerList.tsx`, `OpenInviteSection.tsx`,
  `CapabilityList.tsx`

---

## 4. No Clear Path to Social Feed from Dashboard

**Problem:** The social feed capability shows up in the NavBar *if* the capability
is installed and has a `ui` field in its manifest. But:

1. The `CapabilityList` component on the Dashboard shows capabilities as a
   status-only list (name, version, port, status badge). There are no links to
   the capability's UI page (`/cap/social.feed`).

2. If the NavBar's capability query hasn't loaded yet or the capability isn't
   running, there's no indication that a feed page exists.

3. The old `Feed` page was deleted (it was a React page that called the feed API
   directly). The replacement is the iframe-based `CapabilityPage` that loads the
   capability's own UI. This only works if the social-feed process is running and
   serving its `/ui/` route.

**Fixes needed:**

- [ ] **Add UI links to CapabilityList.** When a capability has a `ui` field,
  render its label as a link to `/cap/{name}`.
  File: `CapabilityList.tsx`

- [ ] **Show a "Feed" quick-link on Dashboard** or at minimum make the
  CapabilityList entry for social.feed visually prominent/clickable.

- [ ] **Verify the social-feed process actually starts** and serves `/ui/`.
  The daemon spawns capabilities as child processes. Confirm the manifest is
  loaded and the capability's UI route is proxied through at `/cap/social/ui/`.
  Files: `capabilities.rs` (spawn logic), `api/mod.rs` (proxy routing)

---

## 5. API Token UX Issues

**Current state:** The token is generated on first daemon run and written to
`{data_dir}/api_token`. The user must find this file, copy its contents, and
paste it into the Dashboard's token input. The Dashboard reloads the entire page
on set/clear.

**What's stale or awkward:**

- [ ] **Show the token in the startup banner.** `howm.sh` already does this
  (reads `api_token` file and prints it in the box). But if someone runs the
  binary directly, they see nothing. The daemon should log the token path or value
  on first run.
  File: `main.rs` (startup logging)

- [ ] **Replace page reload with query invalidation.** Setting the token should
  call `queryClient.invalidateQueries()` instead of `window.location.reload()`.
  File: `Dashboard.tsx:27`

- [ ] **Persist token in the URL for easy sharing during dev.** The
  `?token=...` pattern already works for capability iframes. Consider supporting
  it on the shell too for dev/testing convenience.

---

## 6. Stale / Dead Code to Clean Up

| Item | Location | Issue |
|------|----------|-------|
| Zustand sidebar store | `store/index.ts` | `sidebarOpen` state is never used anywhere |
| Docker CI job | `ci.yml` had docker job | Removed in recent commit but verify no references remain |
| `howm:navigate` handler | `postMessage.ts` | Defined in contract but not wired in `App.tsx` Shell |
| Vite SVG assets | `assets/react.svg`, `assets/vite.svg` | Default Vite scaffold files, not used |
| `assets/hero.png` | `src/assets/hero.png` | Likely unused leftover |

---

## 7. Minimal Demo Checklist

To get a working two-peer demo with social feed visible:

1. **Start node A:**
   ```
   ./howm.sh --wg-endpoint <A_PUBLIC_IP>:51820 --name node-a
   ```

2. **Start node B:**
   ```
   ./howm.sh --wg-endpoint <B_PUBLIC_IP>:51820 --name node-b --port 7002
   ```

3. **Set API tokens** on both Dashboards (copy from terminal output).

4. **Generate invite on A** → copy the `howm://invite/...` string.

5. **Redeem on B** → paste into the "Redeem Invite" input on B's Dashboard.

6. **Verify peering:**
   - Both Dashboards should show the other in the Peer List
   - WireGuard card should show an active tunnel
   - P2P-CD should negotiate and activate the social.feed capability

7. **Post on A's feed** → should appear on B's feed (via P2P-CD peer exchange).

**Currently blocked by:** Items 1-2 above (endpoint validation, error surfacing).

---

## Priority Order

| # | Task | Impact | Effort |
|---|------|--------|--------|
| 1 | Reject invites when endpoint is 0.0.0.0 | Unblocks all peering | Small |
| 2 | Surface real error messages in PeerList/OpenInvite UI | Users can diagnose | Small |
| 3 | Retheme Dashboard components to dark theme | Visual coherence | Medium |
| 4 | Add capability UI links to CapabilityList | Discoverability | Small |
| 5 | Fix token set/clear to not reload page | UX polish | Small |
| 6 | Show endpoint warning on Dashboard | Prevents confusion | Small |
| 7 | Auto-detect public IP | Nice to have | Medium |
| 8 | Clean up dead code | Hygiene | Small |
