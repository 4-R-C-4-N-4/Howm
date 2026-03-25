# SPEC: Isolate Messaging UI & Daemon Notification API

**Author:** IV
**Project:** Howm
**Status:** Draft
**Date:** 2026-03-25
**Related:** BRD-002-peer-messaging.md, TASKS-002-peer-messaging.md

---

## 1. Problem

The messaging capability follows the correct backend pattern — it runs as a
separate process, the daemon proxies API calls to it via `/cap/messaging/*`,
and it receives P2P-CD lifecycle events through `cap_notify`. However, its UI
is special-cased: the daemon's embedded React shell (`ui/web/`) hardcodes
`MessagesPage.tsx`, `ConversationView.tsx`, a messaging API client, and a nav
bar unread badge directly into the shell SPA.

Every other capability with a UI (e.g. `social.feed`) declares a `ui` block in
its `manifest.json` and the shell loads it dynamically via an iframe at
`/app/:name`. Messaging bypasses this entirely.

This creates several problems:

1. **Pattern inconsistency.** New capability developers see two different UI
   integration paths and must determine which to follow. The correct one (iframe
   via manifest) has a working example in feed. The incorrect one (hardcoded in
   shell) has a working example in messaging.

2. **Shell bloat.** ~600 lines of messaging-specific React code are compiled
   into the daemon binary via `include_dir!`. The shell should be thin chrome:
   nav, iframe routing, settings, and platform services.

3. **Coupled deploy.** Changing a bubble color in the messaging UI requires
   rebuilding the entire `howm` binary.

4. **No uninstall story.** The messaging nav tab and routes exist even if the
   messaging capability isn't installed. The shell would show errors instead of
   gracefully hiding the tab.

5. **No badge mechanism for other capabilities.** The nav badge is implemented
   by having the shell directly poll the messaging API. No other capability can
   push a badge count to the nav bar. Future features (feed notification counts,
   "notify me when X posts", LANSPEC game invites) would each need to be
   hardcoded into the shell the same way.

---

## 2. Goals

- **G1.** Messaging UI runs entirely within the messaging capability process,
  served via its own embedded assets, loaded by the shell as an iframe —
  identical to how `social.feed` works today.

- **G2.** The daemon exposes a Notification API that any capability can use to
  push badge counts and transient notifications. The shell consumes this API to
  render nav badges and toast notifications.

- **G3.** The shell has zero messaging-specific code. Messaging is discovered
  and displayed dynamically through the capability manifest and Notification
  API, same as any other capability.

- **G4.** Deep linking from the shell into a capability iframe is supported via
  a new postMessage contract (`howm:navigate:to`) and URL query parameters, so
  shell pages (e.g. PeerDetail) can link into the messaging UI for a specific
  peer.

---

## 3. Non-Goals

- Real-time push (WebSocket/SSE) for notifications. Polling is consistent with
  the existing UI patterns and sufficient for the current scale. SSE can be
  layered on later without changing the API shape.
- OS-level / push notifications (deferred per BRD-002).
- Notification persistence or history (daemon holds notifications in-memory
  only; capability is the source of truth for counts).
- Changing the messaging backend API or P2P-CD wire protocol — this spec is
  purely about UI isolation and the notification surface.

---

## 4. Design

### 4.1 Daemon Notification API

A new set of daemon HTTP endpoints under `/notifications/`. These are
**localhost-only** (same middleware as `/access/*`) because only local
capability processes call them.

#### 4.1.1 Badge State

Capabilities push badge counts to the daemon. The daemon holds them in-memory
(a `HashMap<String, BadgeState>` behind an `RwLock` on `AppState`). The shell
polls a single endpoint to get all badges.

**Set badge — called by capability processes:**

```
POST /notifications/badge
Content-Type: application/json

{
  "capability": "social.messaging",   // installed capability name
  "count": 3                          // 0 clears the badge
}
```

Response: `204 No Content`

The daemon validates that the capability name matches an installed capability.
Unknown names → 404.

**Get all badges — called by shell:**

```
GET /notifications/badges
```

Response:
```json
{
  "badges": {
    "social.messaging": 3,
    "social.feed": 0
  }
}
```

Only capabilities with count > 0 are included.

#### 4.1.2 Transient Notifications (Toast)

Capabilities can push transient notifications that the shell displays as
toasts. These are held in a bounded in-memory ring buffer (max 50) and expire
after 60 seconds. The shell polls and drains them.

**Push notification — called by capability processes:**

```
POST /notifications/push
Content-Type: application/json

{
  "capability": "social.messaging",
  "level": "info",                    // info | success | warning | error
  "title": "New message",
  "message": "Alice: Hey, are you free?",
  "action": "/app/social.messaging?peer=<base64_peer_id>"  // optional deep link
}
```

Response: `204 No Content`

**Poll notifications — called by shell:**

```
GET /notifications/poll?since={timestamp_ms}
```

Response:
```json
{
  "notifications": [
    {
      "id": "notif-001",
      "capability": "social.messaging",
      "level": "info",
      "title": "New message",
      "message": "Alice: Hey, are you free?",
      "action": "/app/social.messaging?peer=...",
      "created_at": 1711360000000
    }
  ],
  "timestamp": 1711360001000
}
```

The shell passes back the `timestamp` as `since` on the next poll to avoid
re-fetching seen notifications.

#### 4.1.3 Route Summary

| Method | Path                    | Auth        | Source       |
|--------|-------------------------|-------------|--------------|
| POST   | /notifications/badge    | localhost   | capabilities |
| GET    | /notifications/badges   | local+wg    | shell        |
| POST   | /notifications/push     | localhost   | capabilities |
| GET    | /notifications/poll     | local+wg    | shell        |

Badge GET and poll GET use `local_or_wg` middleware (same as other shell-facing
read endpoints). Badge POST and push POST use `localhost_only` middleware (only
capability processes on the same machine).

#### 4.1.4 Daemon State

Add to `AppState`:

```rust
/// Badge counts pushed by capabilities. Key: installed capability name.
pub badges: Arc<RwLock<HashMap<String, u32>>>,

/// Transient notification ring buffer.
pub notifications: Arc<RwLock<NotificationBuffer>>,
```

`NotificationBuffer` is a `VecDeque<Notification>` capped at 50 entries, with
entries auto-expired on read (older than 60s are dropped).

---

### 4.2 Messaging Capability UI Migration

#### 4.2.1 Manifest Update

Add the `ui` block to `capabilities/messaging/manifest.json`:

```json
{
  "name": "social.messaging",
  "version": "0.1.0",
  "description": "Private peer-to-peer direct messaging",
  "binary": "./messaging",
  "port": 7002,
  "ui": {
    "label": "Messages",
    "icon": "message-square",
    "entry": "/ui/",
    "style": "iframe"
  },
  ...
}
```

This causes the shell's dynamic nav loop (`capabilities?.filter(c => c.ui)`)
to automatically add a "Messages" tab that loads the messaging UI in an iframe.

#### 4.2.2 Messaging UI Build

The existing `capabilities/messaging/ui/` directory contains bare-bones
`index.html`, `messaging.js`, and `messaging.css`. These need to be replaced
with a full messaging UI that replicates (and improves on) the functionality
currently in the shell's `MessagesPage.tsx` and `ConversationView.tsx`.

The messaging UI is **plain JS/CSS/HTML** with no build step — consistent with
the feed and files capability UIs. The raw files in `capabilities/messaging/ui/`
are embedded directly in the messaging binary via `include_dir!`. This keeps
capability UIs lightweight and dependency-free (only the daemon shell uses
React/Vite).

The UI must:

1. Request the API token via `postMessage('howm:token:request')` on load.
2. Use the token to call `/cap/messaging/*` endpoints (proxied by daemon).
3. Call `GET /node/peers` (via daemon) to resolve peer pubkeys to names.
4. Push badge counts to the daemon via `POST /notifications/badge` whenever
   the unread count changes (on poll, on mark-read, on new message received).
5. Push transient notifications via `POST /notifications/push` on inbound
   message receipt (from the capability backend, not the UI — see 4.2.3).
6. Accept deep-link parameters: `?peer=<base64_peer_id>` opens directly to
   that conversation.

#### 4.2.3 Badge and Notification Push from Backend

The messaging capability's Rust backend (not the UI) is responsible for
pushing badge and notification updates to the daemon. This happens at two
points:

1. **On inbound message receipt** (`POST /p2pcd/inbound` handler):
   - Query current total unread count from SQLite.
   - `POST http://127.0.0.1:<daemon_port>/notifications/badge` with the new
     count.
   - `POST http://127.0.0.1:<daemon_port>/notifications/push` with a toast
     notification (sender name, body preview).

2. **On mark-read** (`POST /conversations/{peer_id}/read` handler):
   - Recompute total unread count.
   - Push updated badge count.

This keeps badge state accurate even when the UI iframe is not open.

#### 4.2.4 Shell Changes

Remove from `ui/web/`:

- `src/pages/MessagesPage.tsx`
- `src/pages/ConversationView.tsx`
- `src/api/messaging.ts`
- All messaging imports and routes from `App.tsx`
- The hardcoded "Messages" NavLink and unread badge polling from `NavBar`

Add to shell:

- **Badge polling:** `NavBar` polls `GET /notifications/badges` every 5s.
  For each capability with a non-zero badge, render a count badge on its
  nav tab. This is generic — works for messaging, feed, or any future
  capability.

- **Toast polling:** The `Shell` component polls `GET /notifications/poll`
  every 5s (same interval as current messaging poll). New notifications are
  displayed as toasts using the existing toast system.

- **Deep link support:** When a shell page needs to link into a capability
  (e.g. PeerDetail → messaging), it navigates to
  `/app/social.messaging?peer=<base64_peer_id>`. The iframe loads with that
  query param and the messaging UI reads it to open the correct conversation.

---

### 4.3 postMessage Bridge Extensions

Add one new message type to the existing postMessage contract:

#### Shell → Capability

```
howm:navigate:to  { params: Record<string, string> }
```

Sent by the shell to the iframe when the user navigates to the capability page
with query parameters (e.g. `?peer=abc`). The capability uses this to deep-link
to a specific view. This is needed because the iframe's `src` URL doesn't
change when the shell's query params change (SPA routing).

The shell sends this message:
1. On initial iframe load (after `howm:ready` from the capability).
2. When the shell route's query params change while the iframe is already open.

#### Capability → Shell (existing, no changes)

The existing `howm:notify` message type is retained but becomes redundant for
most uses once the Notification API exists. Capabilities that want toasts can
use either mechanism. The postMessage path is kept for backward compatibility
and for cases where a capability wants to notify only while its iframe is in
the foreground.

---

### 4.4 Badge Lifecycle

```
Inbound DM arrives
  → messaging backend persists message
  → messaging backend queries total unread count (3)
  → messaging backend POST /notifications/badge { "capability": "social.messaging", "count": 3 }
  → daemon stores badges["social.messaging"] = 3

Shell polls GET /notifications/badges every 5s
  → response: { "badges": { "social.messaging": 3 } }
  → NavBar renders "Messages (3)" badge on the messaging tab

User opens messaging iframe, views conversation
  → messaging UI calls POST /cap/messaging/conversations/{peer}/read
  → messaging backend marks read, queries total unread count (0)
  → messaging backend POST /notifications/badge { "capability": "social.messaging", "count": 0 }
  → daemon stores badges["social.messaging"] = 0

Shell's next poll
  → badge disappears from nav
```

---

## 5. Migration Path

The migration can be done incrementally:

1. **Phase 1: Notification API** — Add the daemon endpoints and shell polling.
   Messaging continues to work as-is (hardcoded in shell). This phase is
   independently useful.

2. **Phase 2: Messaging UI in capability** — Build the messaging UI inside the
   capability, add the manifest `ui` block, wire up badge/notification pushes
   from the backend. At this point both UIs exist (shell hardcoded + iframe).

3. **Phase 3: Shell cleanup** — Remove hardcoded messaging code from the shell.
   The dynamic capability tab takes over. Verify PeerDetail deep links work.

Each phase is independently shippable and testable.

---

## 6. Open Questions

| #    | Question | Status |
|------|----------|--------|
| OQ-1 | Should `GET /notifications/badges` return all installed capabilities with ui blocks (count=0 included) or only non-zero? | **Closed — non-zero only.** Simpler response, shell doesn't need zero entries. |
| OQ-2 | Should the notification push endpoint support an `action` URL that opens a specific capability view, or is query-param deep linking sufficient? | **Closed — include action URL.** No harm in the design and useful for future capability notifications (game invites, feed mentions, etc). |
| OQ-3 | Should the messaging UI be plain JS or get its own React/Vite build? | **Closed — plain JS/CSS.** Consistent with feed and files capability UIs (all plain JS, no build step). Only the daemon shell uses React/Vite. |
| OQ-4 | Rate limiting on notification push — should the daemon cap the rate per capability (e.g. 10/sec) to prevent a misbehaving capability from flooding toasts? | **Closed — yes.** 10 per capability per 10s window. |
| OQ-5 | Should badge counts be persisted to disk (survive daemon restart) or is in-memory sufficient since capabilities will re-push on startup? | **Closed — in-memory.** Capabilities re-push on startup; no disk overhead needed. |

---

## 7. Success Criteria

- The daemon shell has zero messaging-specific imports, components, or routes.
- Messaging appears in the nav bar dynamically (same as Feed) when the
  capability is installed.
- Messaging does NOT appear in the nav bar when the capability is not installed.
- Nav badge shows correct unread count within 5s of message arrival.
- Badge clears within 5s of marking a conversation as read.
- Deep link from PeerDetail opens the messaging iframe to the correct
  conversation.
- The Notification API is generic: a test with a fake capability pushing a
  badge count works identically to messaging.
- Toast notifications appear for inbound messages when the messaging iframe is
  not in the foreground.
