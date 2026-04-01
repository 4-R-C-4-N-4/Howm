### Online peers cannot chat
### Online peers do not share feed content when "Friends"

## I think this is a systemic issue and needs exploration, and testing

Goal 1. When I connect to a peer who is a friend, or promote a peer to fiend group, I should be able to share all the custom capabilities that we've designed here in the Howm caps.

Goal 2. We need legit integration tests for each cap. There is a legacy integration test under Howm/scripts/local-two-peer.sh, assess if this is still valid. We don't want to take shortcuts here, ideally the integration tests run two real Howm nodes on separate ports. NOTE this requires sudo to run because of the wireguard setup, so you won't be able to run it, but I can. Caps to verify would be:
- message
- feed
- files
- voice
- presence

Use the API routes that the UI leverages.

---

## Discovery — Goal 1: Capability sharing between friends

### Architecture Trace

The pipeline from "two peers connected" to "capability works between them":

```
1. Daemon loads p2pcd-peer.toml → PeerConfig with capabilities list
2. PeerConfig.to_manifest() → DiscoveryManifest with CapabilityDeclaration[]
3. Session OFFER exchange: both peers send manifests
4. compute_intersection(): matches caps by name + role + trust_gate
5. Trust gate: access_db.resolve_permission(peer_id, cap_name).is_allowed()
6. Final active_set = intersection of both peers' CONFIRM sets
7. post_session_setup(): cap_router.activate_capabilities(peer_id, active_set)
8. cap_notify: POST /p2pcd/peer-active to each capability's local port
9. Capability adds peer to its active_peers map
10. Capability can now send/receive messages via p2pcd bridge
```

### Default p2pcd-peer.toml Capabilities

The default config (`PeerConfig::generate_default`) advertises:
- `core.session.heartbeat.1` (always passes trust gate)
- `howm.feed.1` (Both/mutual, classification=Public)
- `howm.social.messaging.1` (Both/mutual, no classification)
- `howm.social.files.1` (Both/mutual, no classification)

**Missing from default config:**
- `howm.social.presence.1` — not in the default TOML at all
- `howm.voice.1` — not in the default TOML
- `howm.world.room.1` — not in the default TOML

If these capabilities are installed but not in p2pcd-peer.toml, they won't be advertised in the manifest, won't appear in compute_intersection, and won't be in the active_set. **The peer won't know the other peer supports them.**

### Potential Issue 1: p2pcd-peer.toml not synced with installed capabilities

When a capability is installed via `POST /capabilities/install`, the daemon:
1. Reads manifest.json
2. Creates a CapabilityEntry with `p2pcd_name` (e.g. "howm.social.messaging.1")
3. Saves to capabilities.json
4. Starts the binary

But it does **NOT** update p2pcd-peer.toml to add the capability to the advertised set. The p2pcd-peer.toml is a static file generated once at first boot. If the user installs new capabilities after first boot, they won't be advertised to peers.

**This is likely the systemic issue.** The default TOML has 4 caps. If the user installs presence or voice, those won't be in the TOML, won't be advertised, won't negotiate.

### Potential Issue 2: Trust gate for non-default group

The trust gate calls `access_db.resolve_permission(peer_id, cap_name)`. For a "friends" group capability:
- The peer must be in `howm.friends` group
- The capability's p2pcd_name must be in that group's allowed capabilities

The default access schema puts these in `howm.friends`:
- `howm.social.feed.1`
- `howm.social.messaging.1`
- `howm.social.files.1`
- `howm.world.room.1`

So if the peer IS in `howm.friends`, feed/messaging/files should pass the trust gate. But if `promote to friend` only adds them to the access group without triggering a p2pcd rebroadcast, the active session's intersection won't update.

### Potential Issue 3: Rebroadcast on group change

When a peer is promoted to friend (`add_friend`), the daemon calls:
1. `access_db.add_member(GROUP_FRIENDS, peer_id)`
2. `engine.rebroadcast()` — re-runs OFFER/CONFIRM with new trust gate

This SHOULD work — the rebroadcast re-evaluates compute_intersection with the updated access_db. But if rebroadcast fails silently (e.g. transport error during re-exchange), the session keeps its old active_set.

### Potential Issue 4: Capability notification delivery

When active_set is established, `cap_notify.notify_peer_active()` POSTs to each capability's local port. But:
- The capability must be RUNNING on that port
- The endpoint must be `/p2pcd/peer-active`
- If the capability started after the session was established, it missed the notification

The messaging cap handles this with `init_peers_from_daemon()` on startup — polls the bridge for active peers. But not all capabilities may do this.

### Recommended Investigation Order

1. **Check p2pcd-peer.toml on both machines.** Does it list messaging/feed/files? If it only has the defaults and was never regenerated after capability install, that's the root cause.

2. **Check active_set.** Hit `GET /p2pcd/sessions` on both daemons. Look at each session's `active_set`. If it's empty or only has `core.session.heartbeat.1`, the capabilities aren't negotiating.

3. **Check access_db.** Is the remote peer in `howm.friends`? Hit `GET /access/groups/howm.friends/members`.

4. **Check capability status.** Hit `GET /capabilities`. Are messaging/feed/files running? What ports?

5. **Test cap notification.** Manually POST to `http://127.0.0.1:<cap_port>/p2pcd/peer-active` with a test payload. Does the capability respond?

### Fix Hypothesis

The most likely fix: when a capability is installed, the daemon should automatically add it to the p2pcd config and trigger a rebroadcast. This ensures installed capabilities are advertised to all active peers.

```rust
// In install_capability handler, after saving to capabilities.json:
// 1. Add cap to p2pcd config
// 2. Trigger engine.rebroadcast()
```

Alternatively: the daemon should build the manifest dynamically from installed capabilities + p2pcd-peer.toml, rather than relying solely on the static TOML file.
