# Howm UI — Style Architecture Assessment

## Current State

The UI is split across three styling layers, each serving a different
audience.

### 1. `theme.css` — Shared design tokens (good)

Served by the daemon at `/theme.css`. Defines `--howm-*` CSS custom
properties for colours, spacing, typography, radii, shadows, and
z-indices. Capability UIs link to it directly:

```html
<link rel="stylesheet" href="/theme.css" />
```

This is the strongest part of the current architecture. It gives every
capability a consistent palette without coupling them to the React app's
build toolchain.

### 2. `index.css` — Global resets (fine)

Minimal box-sizing reset, body defaults, anchor/button/input normalisations,
and the `howm-spin` keyframe. References `--howm-*` vars so it stays in
sync with the theme.

### 3. Inline `React.CSSProperties` objects (problematic)

Every React component defines large `const` style objects at the bottom
of the file — `pageStyle`, `cardStyle`, `h1Style`, `mutedStyle`,
`btnStyle`, `accentBtnStyle`, `inputStyle`, etc. Nearly identical
declarations are copy-pasted across 15+ files.

**Pain points:**

| Problem | Impact |
|---------|--------|
| No `:hover` / `:focus` / `:active` states | Buttons feel dead; accessibility suffers |
| No media queries | Layout can't adapt to narrow viewports |
| Massive duplication | Each file re-declares the same card/button/input patterns |
| No shared component primitives | A "card" means something slightly different in every page |
| Hard to audit | Style changes require touching every file |
| Verbose JSX | Style objects bloat component files by ~40% |

### Capability UIs (plain HTML + CSS)

The three capability UIs (feed, files, messaging) are vanilla HTML pages
with their own `.css` files. They link to `/theme.css` for tokens and
define local aliases:

```css
:root {
  --bg: var(--howm-bg-primary, #0f1117);
  --surface: var(--howm-bg-surface, #232733);
  /* … */
}
```

This pattern is clean and should stay as-is. Capabilities are
independently deployed — they must not depend on the React app's
bundler.

---

## Navbar Crowding

The current navbar renders every capability with a UI as a `<NavLink>`
tab. With 3 capabilities this is fine; with 10+ it overflows.

The `CapabilityUi` type already has a `style` field that is defined but
**never read**. This is the natural hook for placement control.

Not every capability needs a full-page tab. Messaging, for instance, is
more natural as a floating chat bubble (bottom-right FAB) than a nav
tab.

---

## Colour Palette

The current palette (`--howm-bg-primary: #0f1117`, accent `#6c8cff`) is
a muted dark-grey with soft lavender-blue. The desired direction is
**black, white, blue** — higher contrast, cleaner feel.

---

## Summary

| Layer | Verdict | Action |
|-------|---------|--------|
| `theme.css` tokens | ✅ Keep | Update palette values |
| `index.css` resets | ✅ Keep | Minor tweaks |
| Inline style objects | ❌ Replace | Migrate to utility-class system |
| Capability `.css` files | ✅ Keep | No changes (consume tokens) |
| Navbar placement | ❌ Rethink | Drive layout from `ui.style` field |
