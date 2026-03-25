# Howm UI — Holistic Review

Reviewed: 2026-03-24
Scope: All source under `ui/web/`, capability UIs, daemon proxy layer, bundled dist.

---

## 1. Architecture Overview

React 19 + TypeScript SPA. Vite dev server, TanStack Query for data fetching, Axios HTTP client, React Router v7, Zustand imported but unused. 12 page components, 14 shared components, 7 API modules, 2 lib modules. ~3,200 lines of application code total (excluding dist bundle).

Good decisions:
- Clean separation: pages → components → api → lib
- TanStack Query used consistently with refetchInterval for live data
- CSS custom properties in theme.css with fallbacks everywhere
- postMessage contract between shell and capability iframes is well-documented
- Optimistic updates in messaging (ConversationView)

---

## 2. Critical Issues (Must Fix)

### 2.1 Capability name mismatch — `howm.feed.1` should be `howm.social.feed.1`

Three places in the UI hardcode `howm.feed.1` but the p2pcd capability name is now `howm.social.feed.1`:

- `src/api/access.ts` lines 52, 64 (TIER_CAPABILITIES for Friends and Trusted)
- `src/lib/access.ts` line 57 (ALL_CAPABILITIES array)

This means the access group UI presets create rules for a capability name that doesn't exist on the wire. Groups created from the UI will have a dead `howm.feed.1` rule instead of `howm.social.feed.1`.

The bundled `dist/assets/index-*.js` has the same stale values baked in.

### 2.2 Dist bundle is stale

`ui/web/dist/` is checked into git and contains the previous build. After any source change (including the feed rename), the dist must be rebuilt or it serves stale code. If the daemon serves from dist, users get the old broken UI.

### 2.3 Vite proxy missing `/access` routes

`vite.config.ts` proxies `/node`, `/cap`, `/capabilities`, `/network`, `/settings` to localhost:7000, but `/access/*` routes (used by the access API module) are not proxied. This means dev mode (`npm run dev`) can't reach access group endpoints.

---

## 3. Security Observations

### 3.1 Token handling
- API token stored in `localStorage` (dev mode) or injected via `<meta>` tag (production). The localStorage path means tokens survive across sessions, which is appropriate for a local-network admin panel.
- Token is passed as a URL query param to capability iframes (`?token=...`). This leaks the token into browser history, server logs, and the iframe's `document.referrer`. Consider using the postMessage handshake exclusively.

### 3.2 iframe sandbox
- `CapabilityPage.tsx` uses `sandbox="allow-scripts allow-same-origin allow-forms"`. The `allow-same-origin` combined with `allow-scripts` effectively negates the sandbox — the iframe can reach into the parent's cookies/storage and call `parent.postMessage` freely. This is probably intentional (capabilities are trusted code from the same daemon), but worth documenting that sandbox provides no real isolation here.

### 3.3 postMessage origin check
- `postMessage.ts` line 59: checks `e.origin !== window.location.origin`. Good — blocks cross-origin messages.

### 3.4 No input sanitization on peer names
- Peer names come from the network and are rendered directly into the DOM. React's JSX escaping handles XSS for text content, so this is safe. But the `peer.name` is used in template strings for toast messages and clipboard operations without concern.

---

## 4. Code Quality Issues

### 4.1 Massive style duplication

Every page file defines its own `pageStyle`, `cardStyle`, `mutedStyle`, `h1Style`, `h3Style`, `dlStyle`, `dtStyle`, `ddStyle` etc. with near-identical values. Counted 8 separate definitions of `mutedStyle` and 7 of `cardStyle` across pages.

**Fix:** Extract shared style objects to a `styles.ts` module or use CSS modules. The existing `theme.css` has all the tokens but nobody consumes them systematically.

### 4.2 Toast system is copy-pasted everywhere

The toast pattern (useState + useRef + useCallback + setTimeout + render block) is duplicated in:
- App.tsx (Shell)
- PeersPage.tsx
- PeerDetail.tsx
- GroupsPage.tsx
- GroupDetail.tsx

Each has its own `toasts` state, `toastId` ref, `showToast` callback, and identical render block. This is ~25 lines copy-pasted 5 times.

**Fix:** Extract to a `useToast()` hook + `<ToastContainer>` component. The App-level one already exists but the page-level ones don't use it.

### 4.3 Zustand imported but unused

`package.json` lists `zustand@^5.0.11` as a dependency, but no store files exist. Either use it (e.g., for toast state, peer cache) or remove the dep.

### 4.4 Duplicate constant definitions

`GROUP_DEFAULT`, `GROUP_FRIENDS`, `GROUP_TRUSTED` are defined in both:
- `src/api/access.ts` (lines 32-34)
- `src/lib/access.ts` (lines 1-3)

Tier capability arrays are defined in both:
- `src/api/access.ts` (TIER_CAPABILITIES)
- `src/lib/access.ts` (ALL_CAPABILITIES, CORE_CAPABILITIES)

Single source of truth should be `lib/access.ts`, with the API module importing from there.

### 4.5 GroupDetail Members section is broken

`GroupDetail.tsx` line 154: `const memberPeerIds: string[] = [];` is hardcoded empty with a comment "Will be populated from group detail endpoint." This means the Members section always shows ALL peers (not just group members), and every peer has a "Remove from group" button regardless of whether they're actually in the group.

### 4.6 PeersPage custom useQueries hook fires N+1 requests

`PeersPage.tsx` lines 153-182: Fetches group memberships for EVERY peer on initial render via `Promise.all(peers.map(...))`. With 50 peers, that's 50 simultaneous HTTP requests. This should be a single batch endpoint or at minimum debounced/paginated.

---

## 5. UX / Design Issues

### 5.1 No loading skeletons
Every page shows a text "Loading…" string. Skeleton loaders or spinners would feel more polished and prevent layout shift.

### 5.2 No error boundaries
No React error boundary anywhere. A failed component render crashes the entire app to a white screen.

### 5.3 No 404 / catch-all route
Unknown routes render a blank page. Need a `<Route path="*" element={<NotFound />} />`.

### 5.4 NavBar overflow on mobile
`navStyle` has `overflow: 'hidden'` — on narrow screens, nav items just disappear. No hamburger menu, no scroll indicator. The entire app is desktop-only.

### 5.5 No keyboard navigation for modals
`DenyModal`, `CreateGroupModal`, `DemotionWarning` — none trap focus or respond to Escape. The add-peer dropdown in GroupDetail has mousedown-outside-to-close but no keyboard equivalent.

### 5.6 Conversation polling is aggressive
`ConversationView` polls every 3 seconds, `MessagesPage` polls every 5 seconds, `PeersPage` every 30 seconds. When multiple tabs are open, this multiplies. Consider using `refetchOnWindowFocus` instead of constant polling, or WebSocket/SSE for real-time updates.

### 5.7 Settings P2P-CD editor has no validation
Raw JSON textarea with no syntax highlighting, no validation on type, no schema awareness. The "Failed — check JSON" error on save is the only feedback. A JSON editor component (or at minimum, real-time parse error display) would prevent config corruption.

---

## 6. Consistency with Existing Enhancement Doc

Cross-referencing `UI-ENHANCEMENTS.md`:

| Enhancement | Status | Notes |
|---|---|---|
| 1. Inline style → style system | Not started | Still fully inline. Highest ROI fix. |
| 2. Nav improvements | Partial | Active route underline exists. No icon grouping for capabilities. |
| 3. Peer identicons | Not started | Still text-only rows. |
| 4. Messaging polish | Partial | Unread badge in navbar IS implemented. Delivery icons still emoji. |
| 6. Network topology view | Not started | Still text cards. |
| 7. Dashboard data viz | Not started | Still flat text counts. |
| 8. Accessibility | Not started | No aria-*, no focus management, no responsive breakpoints. |
| 9. Notification drawer | Not started | Still auto-dismiss toasts only. |
| 10. Settings JSON validation | Not started | Still raw textarea. |

Note: Item 4 (unread count in navbar) was listed as missing in the enhancement doc but is actually implemented in App.tsx lines 105-116. The doc is out of date there.

---

## 7. Rename Residue

Beyond the `howm.feed.1` → `howm.social.feed.1` issue in section 2.1:

- `capability.yaml` description still says "Distributed social feed"
- `posts.rs` line 65: "Configurable media limits for the social feed"
- Several daemon comments reference "social-feed" (bridge.rs, engine.rs, bridge_client.rs)

---

## 8. Priority Recommendations

**P0 (blocking):**
1. Fix `howm.feed.1` → `howm.social.feed.1` in UI source (3 files)
2. Rebuild dist bundle
3. Fix GroupDetail member list (currently shows all peers)

**P1 (important):**
4. Add `/access` to Vite proxy config
5. Extract shared styles to a module
6. Extract toast hook
7. Add error boundary
8. Add 404 route
9. Remove or use Zustand
10. Deduplicate GROUP_* constants

**P2 (quality):**
11. Batch peer group queries (N+1 problem)
12. Replace polling with SSE/WebSocket where available
13. Add loading skeletons
14. Responsive navbar
15. Modal focus trapping + Escape key
16. Stop passing token in iframe URL params
