# Dogfood QA Report

**Target:** http://localhost:7000
**Date:** 2026-03-25
**Scope:** Full application — dashboard, navigation, all capability pages, settings
**Tester:** Hermes Agent (automated exploratory QA)

---

## Executive Summary

| Severity | Count |
|----------|-------|
| 🔴 Critical | 1 |
| 🟠 High | 2 |
| 🟡 Medium | 1 |
| 🔵 Low | 1 |
| **Total** | **5** |

**Overall Assessment:** Core navigation and daemon pages (Dashboard, Peers, Messages, Connection, Groups, Settings) are solid with zero console errors and clean layouts. The capability pages (Feed, Files) are broken due to stale embedded assets and an un-rebuilt UI — both are build/deploy issues rather than code logic bugs.

---

## Issues

### Issue #1: Feed page stuck on "Loading feed…" — JS never executes ✅ COMPLETED

| Field | Value |
|-------|-------|
| **Severity** | 🔴 Critical |
| **Category** | Functional |
| **URL** | http://localhost:7000/app/social.feed (iframe → /cap/feed/ui/) |

**Description:**
The Feed capability embeds its UI via `include_dir!("$CARGO_MANIFEST_DIR/ui")` at compile time. The running binary is serving a stale version of `feed.js` (410 lines) that differs from the file on disk (408 lines). The stale version contains `var tokenParam=params...n');` (corrupted from an earlier sed fix of `***` literals), which is a syntax error (`...` is the spread operator, invalid in this context). This prevents the entire script from parsing, so `loadFeed()`, `submitPost()`, `setFilter()`, and all other functions are undefined.

**Steps to Reproduce:**
1. Navigate to the Feed page via the nav bar
2. Observe "Loading feed…" message that never resolves
3. Click the "All" filter button → console error: `setFilter is not defined`

**Expected Behavior:**
Feed loads and shows "No posts yet. Be the first!" (since there are 0 posts).

**Actual Behavior:**
Permanently stuck on "Loading feed…". All interactive elements (Post button, filter buttons) throw `ReferenceError: X is not defined`.

**Screenshot:**
MEDIA:/home/ivy/.hermes/browser_screenshots/browser_screenshot_9b52f652724a4f688700d205b1f09276.png

**Console Errors:**
```
Uncaught ReferenceError: setFilter is not defined
    at HTMLButtonElement.onclick
```

**Fix:** Rebuild the feed capability: `cd capabilities/feed && cargo build` to re-embed the corrected `feed.js`.

---

### Issue #2: Files page renders completely blank ✅ COMPLETED

| Field | Value |
|-------|-------|
| **Severity** | 🟠 High |
| **Category** | Functional |
| **URL** | http://localhost:7000/app/social.files |

**Description:**
The Files capability has `ui.style: "route"` and `ui.entry: "/files"`. The CapabilityPage.tsx computes the proxy prefix as `cap.name.split('.')[0]` → `"social"`, so the iframe src becomes `/cap/social/files`. The daemon route `/cap/:name/*rest` captures `name=social` and `rest=files`, but no capability is named `social` — result is a 404. The fix (using last segment instead of first) is applied on disk but the web UI hasn't been rebuilt yet.

**Steps to Reproduce:**
1. Click "Files" in the nav bar
2. Observe blank page with no content, no loading indicator, no error

**Expected Behavior:**
Files capability UI loads showing offerings list.

**Actual Behavior:**
Completely blank content area. No feedback to the user.

**Screenshot:**
MEDIA:/home/ivy/.hermes/browser_screenshots/browser_screenshot_548c53428e144195bba8474ab46653ee.png

**Fix:** Rebuild the web UI (`cd ui/web && npm run build`) to pick up the CapabilityPage.tsx proxy prefix fix. Same fix also affects Feed when loaded via iframe.

---

### Issue #3: Capability proxy lookup ambiguity

| Field | Value |
|-------|-------|
| **Severity** | 🟠 High |
| **Category** | Functional |
| **URL** | N/A (daemon code — proxy.rs) |

**Description:**
The proxy capability lookup in `proxy.rs` matches on first segment, last segment, or full name. This creates collision risk: if two capabilities share a segment name (e.g. `social.feed` and `files.feed`), the first match wins nondeterministically. Similarly in `proxy_routes.rs`, `resolve_p2pcd_cap_name()` matches `"social"` to the first `howm.social.*.1` it finds. As more capabilities are added, this will cause routing bugs.

**Expected Behavior:**
Each capability has a unique, deterministic URL prefix derived from its manifest `base_path`.

**Actual Behavior:**
Fuzzy segment matching with first-match-wins semantics.

**Fix:** Store `base_path` from the manifest in `CapabilityEntry` and match on it exactly, rather than heuristic segment matching.

---

### Issue #4: Files page shows no empty state or error on blank render

| Field | Value |
|-------|-------|
| **Severity** | 🟡 Medium |
| **Category** | UX |
| **URL** | http://localhost:7000/app/social.files |

**Description:**
When the Files iframe fails to load (404), the CapabilityPage shows a completely blank area with zero feedback. No "Failed to load" message, no spinner, no error state. The user has no idea what happened.

**Expected Behavior:**
Either a loading spinner with a timeout error, or an error message like "Capability UI failed to load".

**Actual Behavior:**
Silent blank page.

**Fix:** Add iframe `onerror`/`onload` handling in CapabilityPage.tsx to detect load failures and show an error state.

---

### Issue #5: `howm:ready` postMessage reports stale capability name

| Field | Value |
|-------|-------|
| **Severity** | 🔵 Low |
| **Category** | Content |
| **URL** | /cap/feed/ui/ |

**Description:**
In `feed/ui/feed.js` line 40, the `howm:ready` postMessage sends `{ name: 'feed' }` instead of `'social.feed'`. While this doesn't break functionality (the shell doesn't seem to use the name for routing), it's inconsistent with the standardized naming convention.

**Fix:** Update to `{ name: 'social.feed' }`.

---

## Issues Summary Table

| # | Title | Severity | Category | URL |
|---|-------|----------|----------|-----|
| 1 | Feed stuck on "Loading feed…" | 🔴 Critical | Functional | /app/social.feed |
| 2 | Files page renders blank | 🟠 High | Functional | /app/social.files |
| 3 | Proxy lookup ambiguity | 🟠 High | Functional | proxy.rs |
| 4 | No error state for failed capability iframe | 🟡 Medium | UX | /app/social.files |
| 5 | Stale capability name in postMessage | 🔵 Low | Content | /cap/feed/ui/ |

## Testing Coverage

### Pages Tested
- Dashboard (/)
- Peers (/peers)
- Messages (/messages)
- Connection (/connection)
- Groups (/access/groups)
- Files (/app/social.files)
- Feed (/app/social.feed)
- Feed direct (/cap/feed/ui/)
- Settings (/settings)

### Features Tested
- Navigation between all pages
- Console error checking on every page
- Visual layout inspection
- Empty state rendering (Peers, Messages)
- Feed post composition UI
- Feed filter buttons
- Capability iframe loading
- API endpoint availability (curl)
- Static asset serving through proxy

### Not Tested / Out of Scope
- Actual peer connections (0 peers available)
- Messaging with peers
- File upload/download flows
- Invite creation and redemption
- Group creation and editing
- Settings JSON editing and save
- Mobile/responsive layout
- Performance under load

### Blockers
- Feed and Files capability UIs non-functional due to stale builds — could not test any interactive features within those capabilities.

---

## Notes

The root cause for issues #1 and #2 is that the current running binaries haven't been rebuilt after today's code fixes. A full `./howm.sh` restart (which rebuilds both the web UI and capabilities) should resolve both. Issue #3 is a design concern that should be addressed before adding more capabilities to avoid routing collisions.

Pages that don't depend on capability iframes (Dashboard, Peers, Messages, Connection, Groups, Settings) are all clean — zero console errors, proper empty states, good visual design.
