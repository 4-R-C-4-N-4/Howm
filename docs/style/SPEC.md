# Howm UI — Style Implementation Spec

> Companion to `ASSESSMENT.md`. Describes exactly what changes, in what
> order, and how to verify each step.

---

## Phase 1 — Colour Scheme: Black / White / Blue

**Goal:** Update the design tokens to the target palette. Every
component that references `--howm-*` vars picks up the change
automatically — zero code edits outside `theme.css` and `index.css`.

### New Token Values

```css
:root {
  /* ── Backgrounds ─────────────────────────────────────── */
  --howm-bg-primary:   #000000;
  --howm-bg-secondary: #0a0a0a;
  --howm-bg-surface:   #111111;
  --howm-bg-elevated:  #1a1a1a;

  /* ── Text ────────────────────────────────────────────── */
  --howm-text-primary:   #ffffff;
  --howm-text-secondary: #a0a0a0;
  --howm-text-muted:     #666666;

  /* ── Accent / brand ──────────────────────────────────── */
  --howm-accent:       #3b82f6;          /* blue-500 */
  --howm-accent-hover: #60a5fa;          /* blue-400 */
  --howm-accent-dim:   rgba(59,130,246,0.15);

  /* ── Semantic ────────────────────────────────────────── */
  --howm-success: #22c55e;
  --howm-warning: #eab308;
  --howm-error:   #ef4444;
  --howm-info:    #3b82f6;

  /* ── Borders ─────────────────────────────────────────── */
  --howm-border:        #222222;
  --howm-border-subtle: #181818;

  /* (spacing, typography, shadows, z-index unchanged) */
}
```

### Steps

1. Replace values in `ui/web/public/theme.css`.
2. Update hardcoded fallbacks in `ui/web/src/index.css` to match.
3. Grep the React source for any remaining hardcoded hex values that
   should reference tokens instead (e.g. `#0f1117` literals in inline
   styles). Replace with `var(--howm-*)`.
4. The inline toast colours in `App.tsx` (e.g. `#1e3a5f`, `#14532d`)
   should move to semantic token references or at minimum be updated to
   harmonise with the new blues/blacks.

### Verification

- `npm run build` — no regressions.
- Visual spot-check: dashboard, peer list, capability page, messaging
  iframe. Everything should read as black/white/blue with no leftover
  purple-grey artifacts.

---

## Phase 2 — Tailwind CSS Integration

**Goal:** Replace all inline `React.CSSProperties` objects with Tailwind
utility classes. Eliminate style duplication. Gain hover/focus/responsive
support.

### Why Tailwind

| Criteria | Tailwind | CSS Modules | Emotion/styled |
|----------|----------|-------------|----------------|
| Runtime overhead | None (compile-time) | None | JS runtime |
| Vite support | First-class plugin | Built-in | Needs config |
| Design-token integration | `@theme` directive | Manual | Manual |
| Hover/focus/responsive | ✅ | ✅ | ✅ |
| Bundle size | Tiny (purged) | Varies | Runtime + styles |
| Learning curve | Low (utility classes) | Low | Medium |

Tailwind wins on zero-runtime + design-token mapping + the smallest
migration surface.

### Setup

```bash
cd ui/web
npm install -D tailwindcss @tailwindcss/vite
```

**`vite.config.ts`** — add the plugin:

```ts
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  plugins: [react(), tailwindcss()],
});
```

**`src/index.css`** — add Tailwind import at the top:

```css
@import "tailwindcss";
```

**`src/index.css`** — map `--howm-*` tokens into Tailwind's theme so
classes like `bg-howm-surface` and `text-howm-accent` just work:

```css
@theme {
  --color-howm-bg-primary:   var(--howm-bg-primary);
  --color-howm-bg-secondary: var(--howm-bg-secondary);
  --color-howm-bg-surface:   var(--howm-bg-surface);
  --color-howm-bg-elevated:  var(--howm-bg-elevated);

  --color-howm-text-primary:   var(--howm-text-primary);
  --color-howm-text-secondary: var(--howm-text-secondary);
  --color-howm-text-muted:     var(--howm-text-muted);

  --color-howm-accent:       var(--howm-accent);
  --color-howm-accent-hover: var(--howm-accent-hover);
  --color-howm-accent-dim:   var(--howm-accent-dim);

  --color-howm-success: var(--howm-success);
  --color-howm-warning: var(--howm-warning);
  --color-howm-error:   var(--howm-error);
  --color-howm-info:    var(--howm-info);

  --color-howm-border:        var(--howm-border);
  --color-howm-border-subtle: var(--howm-border-subtle);
}
```

### Migration Strategy

Migrate one file at a time. For each component:

1. Delete the `const fooStyle: React.CSSProperties = { ... }` block.
2. Replace `style={fooStyle}` with equivalent Tailwind classes.
3. Run `npm run build` — must compile clean.
4. Visual spot-check the page.

**Migration order** (dependency-first):

| # | File | Notes |
|---|------|-------|
| 1 | `App.tsx` | NavBar, Shell, toast container |
| 2 | `Dashboard.tsx` | Heaviest inline styles, most duplication |
| 3 | `PeersPage.tsx` / `PeerList.tsx` / `PeerRow.tsx` | |
| 4 | `PeerDetail.tsx` | |
| 5 | `Connection.tsx` / `ConnectionInfo.tsx` | |
| 6 | `GroupsPage.tsx` / `GroupDetail.tsx` / `GroupChips.tsx` | |
| 7 | `CapabilityList.tsx` / `CapabilityPage.tsx` | |
| 8 | `Settings.tsx` | |
| 9 | Remaining small components (`InviteManager`, `DenyModal`, etc.) | |

### Example: Card Pattern

Before (repeated in every file):

```ts
const cardStyle: React.CSSProperties = {
  background: 'var(--howm-bg-surface, #232733)',
  border: '1px solid var(--howm-border, #2e3341)',
  borderRadius: 'var(--howm-radius-md, 8px)',
  padding: '20px',
  marginBottom: '16px',
};
```

After:

```tsx
<section className="bg-howm-bg-surface border border-howm-border rounded-lg p-5 mb-4">
```

One line, no separate const, same result — plus hover/focus available
for free.

### What Does NOT Change

- `theme.css` — untouched (already done in Phase 1).
- Capability UI `.css` files — they are plain HTML, not React.
  Tailwind is scoped to the React app only.
- `index.css` global resets — kept, Tailwind layers on top via
  `@import "tailwindcss"`.

---

## Phase 3 — Capability Placement System

**Goal:** Capabilities declare where they appear in the shell UI via
their `ui.style` field. The navbar stops being the only option.

### Placement Modes

| `ui.style` value | Behaviour |
|------------------|-----------|
| `"nav"` | Tab in the top navbar (current default). Full-page iframe on click. |
| `"fab"` | Floating action button, bottom-right corner. Click opens an overlay panel with the iframe. Badge count shown on the bubble. |
| `"dock"` | (Future) Icon in a left-side dock/sidebar. |
| `"hidden"` | No chrome. Accessible only via direct URL (`/app/<name>`). |

If `ui.style` is absent or unrecognised, default to `"nav"`.

### Rust Side — Capability Manifest

The `ui.style` field is already declared in the capability TOML
manifest type. No Rust struct changes needed — just ensure the daemon
serialises it through the `/capabilities` JSON endpoint (it already
does, as `style: String`).

Capability authors set it in their `howm-capability.toml`:

```toml
[ui]
label = "Messages"
icon  = "chat-bubble"    # icon identifier (see Icon Set below)
entry = "index.html"
style = "fab"
```

### React Side — Shell Changes

#### 3a. NavBar filtering

`NavBar` renders only capabilities where `cap.ui.style` is `"nav"` (or
absent):

```tsx
{capabilities
  ?.filter(c => c.ui && (!c.ui.style || c.ui.style === 'nav'))
  .map(cap => ( /* existing NavLink */ ))}
```

#### 3b. FAB layer

A new `<FabLayer />` component sits in the Shell, outside the
`<Routes>` block. It renders one floating button per `"fab"`
capability:

```
┌─────────────────────────────────────────────┐
│  NavBar  [Dashboard] [Peers] [Feed] ...     │
├─────────────────────────────────────────────┤
│                                             │
│                 <Routes />                  │
│                                             │
│                                         💬 │ ← FAB (bottom-right)
│                                             │
└─────────────────────────────────────────────┘
```

Clicking the FAB toggles an overlay panel (fixed-position, e.g.
400×600px, anchored bottom-right) containing the capability's iframe.
The panel has a small title bar with the capability label and a close
button.

Badge count renders as a red dot/number on the FAB icon, sourced from
the existing `badges` API.

#### 3c. Icon set

The `ui.icon` field maps to an inline SVG or a small icon component.
Start with a minimal built-in set:

| Identifier | Shape |
|------------|-------|
| `chat-bubble` | Speech bubble (messaging) |
| `folder` | Folder (files) |
| `feed` | RSS/list icon (feed) |
| `grid` | Grid squares (generic) |
| `globe` | Globe (web-facing caps) |

Icons are pure SVG components — no icon font dependency.

### FAB Panel Behaviour

- **Toggle:** Click FAB → open panel. Click FAB again or click outside
  → close.
- **Persist while navigating:** Panel stays open if the user clicks a
  navbar link. It's an overlay, not a route.
- **Deep links:** `?peer=<id>` search params on the FAB panel's iframe
  URL, same mechanism as the full-page `CapabilityPage`.
- **Mobile:** On narrow viewports (`< 640px`), FAB panel expands to
  full screen with a back/close button in the header.

### Verification

- Messaging capability with `style = "fab"` renders as a bottom-right
  chat bubble, not a nav tab.
- Feed capability with `style = "nav"` (or no style) still renders as a
  nav tab.
- Badge count appears on the FAB icon.
- Panel opens/closes cleanly, iframe loads with token handshake.
- Toasts from the FAB-hosted capability still appear in the shell's
  toast container.

---

## Phase Summary

| Phase | Scope | Files touched |
|-------|-------|---------------|
| 1 | Colour scheme | `theme.css`, `index.css`, inline fallbacks |
| 2 | Tailwind migration | `package.json`, `vite.config.ts`, `index.css`, all `.tsx` components |
| 3 | Capability placement | `App.tsx` (NavBar + new FabLayer), `capabilities.ts` type, capability TOMLs |

Phases are independent and can ship incrementally. Phase 1 is a
standalone visual refresh. Phase 2 is a refactor with no user-visible
change. Phase 3 adds new UX behaviour.
