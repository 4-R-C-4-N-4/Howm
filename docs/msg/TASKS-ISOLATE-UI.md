# Tasks: Isolate Messaging UI & Daemon Notification API

Linked spec: `ISOLATE_UI_NOTIFICATION_SPEC.md`
Linked BRD: `BRD-002-peer-messaging.md`

---

## Phase 1: Daemon Notification API

### TASK-N1: Notification State and Types

Add notification infrastructure to the daemon.

**Scope:**
- Define types in a new `node/daemon/src/notifications.rs`:
  - `BadgeUpdate { capability: String, count: u32 }`
  - `PushNotification { id: String, capability: String, level: NotifyLevel, title: String, message: String, action: Option<String>, created_at: u64 }`
  - `NotifyLevel` enum: `Info`, `Success`, `Warning`, `Error`
  - `NotificationBuffer` — `VecDeque<PushNotification>` capped at 50 entries, with an `expired()` method that drops entries older than 60s.
- Add to `AppState` in `state.rs`:
  - `badges: Arc<RwLock<HashMap<String, u32>>>`
  - `notifications: Arc<RwLock<NotificationBuffer>>`
- Initialize both in `AppState::new()`.
- Add `mod notifications;` to `main.rs`.

**Acceptance criteria:**
- Types compile and are accessible from route handlers.
- `NotificationBuffer` correctly caps at 50 and expires entries > 60s old.
- Unit tests for buffer cap, expiry, and drain-since behavior.

---

### TASK-N2: Notification HTTP Routes

Implement the four notification endpoints.

**Scope:**
- Create `node/daemon/src/api/notification_routes.rs` with four handlers:
  - `POST /notifications/badge` — parse `BadgeUpdate`, validate capability name against installed capabilities list, update `state.badges`. Return 204 on success, 404 for unknown capability.
  - `GET /notifications/badges` — return `{ badges: { cap_name: count, ... } }` for all entries with count > 0.
  - `POST /notifications/push` — parse push notification, assign auto-incrementing ID, validate capability name, push to `NotificationBuffer`. Return 204. Rate limit: max 10 pushes per capability per 10s window (simple token bucket per capability name, 429 on excess).
  - `GET /notifications/poll?since={timestamp_ms}` — return notifications created after `since`, plus a `timestamp` cursor. Expired entries are pruned on read.
- Register routes in `api/mod.rs`:
  - Badge POST and push POST: `localhost_only` middleware (same as `/access/*`).
  - Badge GET and poll GET: `local_or_wg` middleware (same as read-only routes).
  - Both sets: bearer token auth on write endpoints (capabilities have the token via env), no auth on read endpoints (they're already subnet-restricted).
- Add `pub mod notification_routes;` to `api/mod.rs`.

**Reference files:**
- `node/daemon/src/api/access_routes.rs` — handler pattern with AppState extraction
- `node/daemon/src/api/mod.rs:174-210` — how localhost_only routes are registered
- `node/daemon/src/api/settings_routes.rs` — simple GET/POST pattern

**Acceptance criteria:**
- `POST /notifications/badge` with valid capability name → 204, reflected in GET.
- `POST /notifications/badge` with unknown capability → 404.
- `POST /notifications/badge` with count=0 → clears the badge (GET no longer includes it).
- `POST /notifications/push` → notification appears in `GET /notifications/poll`.
- Poll with `since` only returns newer notifications.
- Push from non-localhost IP → 403.
- Rate limit: 11th push within 10s from same capability → 429.

---

### TASK-N3: Shell Badge Polling

Add generic badge rendering to the shell's NavBar.

**Scope:**
- Add `ui/web/src/api/notifications.ts`:
  - `getBadges(): Promise<Record<string, number>>` — calls `GET /notifications/badges`.
  - `pollNotifications(since: number): Promise<{ notifications: Notification[], timestamp: number }>` — calls `GET /notifications/poll?since=`.
- Modify `NavBar` in `App.tsx`:
  - Add a react-query hook polling `getBadges()` every 5s.
  - In the dynamic capability tab loop (`capabilities?.filter(c => c.ui).map(...)`), look up the badge count by capability name. If > 0, render the red badge (same style as the current hardcoded messaging badge).
  - Keep the existing hardcoded Messages tab and badge for now (removed in Phase 3). The dynamic badges will coexist during transition.
- Modify `Shell` component in `App.tsx`:
  - Add polling for `pollNotifications()` every 5s.
  - On new notifications, feed them into the existing `addToast()` system.
  - If a notification has an `action` URL, make the toast clickable and navigate to it.
  - Track the `since` cursor in component state.

**Acceptance criteria:**
- When a capability pushes a badge via the daemon API, the shell nav shows the count within 5s.
- Badge disappears within 5s of being cleared (count=0).
- Toast notifications from capabilities appear in the existing toast system.
- Clicking a toast with an action URL navigates to that route.

---

## Phase 2: Messaging UI in Capability

### TASK-M1: Messaging Manifest UI Block

Add the `ui` declaration to the messaging capability's manifest.

**Scope:**
- Update `capabilities/messaging/manifest.json` to add:
  ```json
  "ui": {
    "label": "Messages",
    "icon": "message-square",
    "entry": "/ui/",
    "style": "iframe"
  }
  ```
- Verify that when the messaging capability is installed, the shell's dynamic
  nav loop picks up the "Messages" tab and renders it with an iframe via
  `CapabilityPage`.

**Acceptance criteria:**
- With messaging installed, a "Messages" tab appears in the nav bar via the
  dynamic capability loop (not the hardcoded one, which still exists in
  parallel during this phase).

---

### TASK-M2: Messaging UI — Conversation List View

Replace the bare-bones messaging UI with a full conversation list view.
Plain JS/CSS/HTML — no build step, consistent with feed and files capability
UIs.

**Scope:**
- Rewrite `capabilities/messaging/ui/index.html`:
  - Howm iframe integration: on load, `postMessage({ type: 'howm:token:request' })`.
    Listen for `howm:token:reply` and store the token for API calls.
  - Listen for `howm:navigate:to` with `{ params: { peer: "..." } }` for deep
    linking (consumed in TASK-M3).
  - Post `howm:ready` after initialization.
  - Client-side hash routing: `#/` for conversation list, `#/chat/<peer_id>`
    for conversation view. Simple `hashchange` listener that swaps visible
    sections.
- Rewrite `capabilities/messaging/ui/messaging.js`:
  - Conversation list view:
    - Fetch `GET /cap/messaging/conversations` (via daemon proxy, using token).
    - Fetch `GET /node/peers` to resolve peer names.
    - Render list sorted by most recent activity. Each row: peer name, preview,
      timestamp, unread badge.
    - Click navigates to `#/chat/<peer_id>`.
    - Poll every 5s via `setInterval` + `fetch`.
    - On each poll, compute total unread count. Call
      `POST /notifications/badge { capability: "social.messaging", count: N }`
      on the daemon whenever the count changes.
  - Token and peer data cached in module-level variables.
- Style in `capabilities/messaging/ui/messaging.css` to match the howm dark
  theme (use CSS custom properties: `--howm-bg-primary`, `--howm-bg-surface`,
  `--howm-text-primary`, `--howm-accent`, etc.). Follow the same patterns as
  `capabilities/feed/ui/feed.css`.

**Reference files:**
- `ui/web/src/pages/MessagesPage.tsx` — the current implementation to replicate
- `capabilities/feed/ui/` — plain JS capability UI pattern (feed.js, feed.css,
  index.html)
- `capabilities/files/ui/` — another plain JS capability UI example

**Acceptance criteria:**
- No build step required — raw files embedded via `include_dir!`.
- Conversation list renders with correct peer names and unread counts.
- Token handshake works: UI can call authenticated daemon API endpoints.
- Badge count pushed to daemon matches the actual total unread count.
- UI updates within 5s of a new inbound message arriving.
- Matches the visual style of the existing shell MessagesPage.

---

### TASK-M3: Messaging UI — Conversation Detail View

Build the conversation detail / chat view in plain JS within the existing
messaging capability UI files.

**Scope:**
- Hash route `#/chat/<peer_id>` (routed via the `hashchange` listener from
  TASK-M2):
  - Fetch `GET /cap/messaging/conversations/{peer_id}` with pagination.
    Poll every 3s via `setInterval` + `fetch`.
  - Render messages in ascending time order. Sent right-aligned (accent bg),
    received left-aligned (surface bg).
  - Delivery status indicators: ⏳ pending, ✓ delivered, ⚠ failed.
  - Date dividers between days.
  - Auto-scroll to bottom on load and new messages.
- Composer:
  - Textarea with Enter to send, Shift+Enter for newline.
  - Byte counter (N / 4096), red at > 4000, send blocked at > 4096.
  - Disabled with banner when peer is offline (check peers list for
    `last_seen` recency).
  - Optimistic message insertion on send (append to DOM immediately with
    pending status, update on next poll).
- On open: call `POST /cap/messaging/conversations/{peer_id}/read`.
  After mark-read, recompute total unread and push badge update.
- Deep linking: consume `howm:navigate:to { params: { peer: "<id>" } }` from
  the postMessage listener (TASK-M2). On receipt, set `location.hash` to
  `#/chat/<peer_id>`. Also check URL query params on initial load.
- Back navigation: link/button to return to `#/`.

**Reference files:**
- `ui/web/src/pages/ConversationView.tsx` — the current implementation to
  replicate in plain JS
- `capabilities/feed/ui/feed.js` — plain JS DOM manipulation patterns

**Acceptance criteria:**
- Messages render correctly with delivery status indicators.
- Sending a message shows optimistic insert, updates to delivered on poll.
- Composer disabled with offline banner when peer is not reachable.
- Deep link via `howm:navigate:to` or `?peer=<id>` opens the correct
  conversation.
- Mark-read triggers badge count update on daemon.

---

### TASK-M4: Backend Badge and Toast Push

Wire the messaging capability's Rust backend to push badges and notifications
to the daemon.

**Scope:**
- Add a `DaemonNotifier` struct to the messaging capability:
  - `push_badge(count: u32)` — POST to daemon's `/notifications/badge`.
  - `push_toast(title, message, action)` — POST to daemon's `/notifications/push`.
  - Calls are fire-and-forget (spawn a tokio task, log on error, don't block
    the handler).
- Wire into the inbound message handler (`POST /p2pcd/inbound` in `api.rs`):
  - After persisting the received message, query total unread count.
  - `push_badge(total_unread)`.
  - `push_toast("New message", "{sender_name}: {body_preview}", "/app/social.messaging?peer={sender_id}")`.
  - Sender name: resolve from the active peers cache (populated by
    `peer-active` notifications) or fall back to truncated peer ID.
- Wire into mark-read handler (`POST /conversations/{peer_id}/read`):
  - After updating read marker, query total unread count.
  - `push_badge(total_unread)`.
- Wire into capability startup:
  - On startup (after `init_peers_from_daemon`), query total unread count
    from SQLite and push initial badge. This ensures badge state is correct
    after daemon restart.

**Reference files:**
- `capabilities/messaging/src/api.rs` — existing handlers
- `node/daemon/src/p2pcd/bridge.rs` — pattern for localhost HTTP calls

**Acceptance criteria:**
- Receiving a DM when messaging iframe is not open → toast appears in shell
  within 5s.
- Badge count on nav matches actual unread count at all times.
- After daemon restart, badge count is re-pushed within seconds of messaging
  capability starting.
- Notification push failures don't block message delivery or ACK.

---

### TASK-M5: postMessage Deep Link Extension

Add the `howm:navigate:to` postMessage type to the shell-capability bridge.

**Scope:**
- Update `ui/web/src/lib/postMessage.ts`:
  - Add `howm:navigate:to` to the Shell → Capability direction documentation.
  - Add `sendNavigateTo(iframe: HTMLIFrameElement, params: Record<string, string>)` helper.
- Update `CapabilityPage.tsx`:
  - On initial load, after receiving `howm:ready` from the iframe, send
    `howm:navigate:to` with the current URL search params parsed into a
    record (e.g. `?peer=abc` → `{ peer: "abc" }`).
  - When the shell route's search params change (useSearchParams), re-send
    `howm:navigate:to` to the iframe.
- Update `PeerDetail.tsx`:
  - Change the "Message" button to link to `/app/social.messaging?peer=<wg_pubkey>`
    instead of the current `/messages/<wg_pubkey>`.

**Reference files:**
- `ui/web/src/lib/postMessage.ts` — existing bridge contract
- `ui/web/src/pages/CapabilityPage.tsx` — iframe management
- `ui/web/src/pages/PeerDetail.tsx` — current "Message" link

**Acceptance criteria:**
- Clicking "Message" on PeerDetail opens the messaging iframe at the correct
  conversation.
- Navigating directly to `/app/social.messaging?peer=<id>` opens the correct
  conversation.
- The iframe receives `howm:navigate:to` after the token handshake completes.

---

## Phase 3: Shell Cleanup

### TASK-C1: Remove Hardcoded Messaging from Shell

Remove all messaging-specific code from the daemon's embedded shell.

**Scope:**
- Delete:
  - `ui/web/src/pages/MessagesPage.tsx`
  - `ui/web/src/pages/ConversationView.tsx`
  - `ui/web/src/api/messaging.ts`
- Remove from `App.tsx`:
  - Imports for MessagesPage, ConversationView, getConversations.
  - The hardcoded "Messages" NavLink with its unread badge polling.
  - The `/messages` and `/messages/:peerId` Route entries.
- Verify the dynamic capability tab loop in NavBar now handles messaging
  entirely through the manifest `ui` block and the notification badge API.
- Verify PeerDetail links to `/app/social.messaging?peer=...`.
- Rebuild the shell (`cd ui/web && npm run build`) and confirm the daemon
  binary no longer includes messaging UI assets.

**Acceptance criteria:**
- `grep -r "MessagesPage\|ConversationView\|messaging" ui/web/src/` returns
  zero hits (except possibly the notification API client which is generic).
- The messaging tab appears via dynamic capability discovery.
- Badge appears via the Notification API, not direct messaging API polling.
- All existing messaging UX works identically from the user's perspective.
- The daemon binary size decreases (messaging UI assets removed from
  include_dir).

---

### TASK-C2: Verify Messaging Capability UI End-to-End

Final verification that the messaging capability's plain JS UI is fully
functional as the sole messaging interface.

**Scope:**
- Verify `include_dir!("$CARGO_MANIFEST_DIR/ui")` in the messaging
  capability's `main.rs` correctly embeds the rewritten UI files from
  TASK-M2 and TASK-M3.
- Verify the `/ui` and `/ui/{*path}` route handlers serve the UI correctly
  when accessed via the shell iframe.
- Run a clean `cargo build --release` of the messaging capability and confirm
  the embedded UI works end-to-end: token handshake, conversation list, chat
  view, badge updates, deep linking.
- Confirm no dead code remains from the old bare-bones UI (the files were
  rewritten in-place by TASK-M2/M3, so this should be a no-op).

**Acceptance criteria:**
- Messaging capability builds cleanly with no warnings.
- UI is accessible via the shell iframe at `/app/social.messaging`.
- All messaging UX works: list, chat, send, mark-read, badge, deep link.

---

## Task Dependency Order

```
Phase 1 (Notification API):
  TASK-N1 (state + types)
    └── TASK-N2 (HTTP routes)
          └── TASK-N3 (shell badge + toast polling)

Phase 2 (Messaging UI in capability):
  TASK-M1 (manifest update)  ── can start immediately
  TASK-M2 (conversation list)  ── needs TASK-N2 for badge push
  TASK-M3 (conversation detail)  ── needs TASK-M2
  TASK-M4 (backend push)  ── needs TASK-N2
  TASK-M5 (deep link)  ── needs TASK-M2

Phase 3 (Shell cleanup):
  TASK-C1 (remove hardcoded)  ── needs TASK-M2, M3, M4, M5, N3 all complete
  TASK-C2 (cleanup old UI)  ── needs TASK-C1
```

**Parallelism:**
- TASK-N1 + TASK-M1 can start in parallel.
- TASK-M2 + TASK-M4 can run in parallel after TASK-N2.
- TASK-M3 is sequential after TASK-M2.
- TASK-M5 can run any time after TASK-M2.
- Phase 3 is strictly after all of Phase 1 and Phase 2.

**Estimated effort:**
- Phase 1: ~2 sessions (N1+N2 together, N3 separate)
- Phase 2: ~3-4 sessions (M1 trivial, M2+M3 are the bulk, M4+M5 lighter)
- Phase 3: ~1 session (mostly deletion + verification)
