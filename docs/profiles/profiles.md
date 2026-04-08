# Profiles Spec

## Overview

User profiles for howm nodes. Simple identity + a fully customizable HTML homepage served from your node — MySpace energy, your node your page.

## Profile Fields

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| name | string | yes | Display name, already in identity |
| bio | string | no | Short text, ~280 chars |
| avatar | image file | no | Profile picture, stored locally |
| homepage | HTML file | no | Local .html file served as your personal page |

That's it. No bloat.

## Homepage

The killer feature. You pick a local HTML file and it becomes your homepage on the mesh. Full creative control — CSS, JS, images, whatever you want. Your peers visit it through the howm UI or directly via your node's address.

- User selects an .html file (or a directory with index.html + assets)
- Howm serves it at a profile route (e.g. `/profile/home` or `/~username`)
- Peers can browse to it — rendered in an iframe or navigated directly
- No restrictions on content — it's YOUR page
- Could reference local assets (images, css, fonts) bundled alongside
- Static only — no server-side execution, just served files

### Inspiration
- MySpace custom profiles
- Neocities
- Personal homepages / geocities
- ~user directories on unix servers

## Data Model

```
~/.local/howm/profile/
├── profile.json        # name, bio, avatar path, homepage path
├── avatar.{png,jpg,webp}
└── homepage/           # user's HTML page + assets
    ├── index.html
    ├── style.css
    └── ...
```

```json
{
  "name": "IV",
  "bio": "building the mesh",
  "avatar": "avatar.png",
  "homepage": "homepage/index.html"
}
```

## API Endpoints

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/profile` | local+wg | Get own profile |
| PUT | `/profile` | local | Update name/bio |
| PUT | `/profile/avatar` | local | Upload avatar image |
| PUT | `/profile/homepage` | local | Set homepage HTML path |
| GET | `/profile/avatar` | public | Serve avatar (peers fetch this) |
| GET | `/profile/home` | public | Serve homepage HTML (peers visit this) |
| GET | `/profile/home/*` | public | Serve homepage assets (css, images, etc) |
| GET | `/peer/{id}/profile` | wg | Fetch a peer's profile over the mesh |

## Sharing

- Profile metadata (name, bio, avatar hash) pushed to peers on connect and on update
- Avatar fetched on demand from the peer's node (GET /profile/avatar)
- Homepage is always live from the source node — not cached/replicated
- When you visit a peer's profile, their homepage loads from their node in real-time

## UI

- **Settings/Profile page**: edit name, bio, upload avatar, select homepage file/folder
- **Peer detail page**: shows their profile card + iframe to their homepage
- **Peer list**: avatars shown next to peer names
- **Profile preview**: see your own homepage before publishing

## Open Questions

- Max avatar size? (256KB? 1MB?)
- Homepage size limit? (probably don't need one — it's their storage)
- Should homepage assets be restricted to the homepage directory? (yes, for security)
- Sandboxing the homepage iframe — CSP headers to prevent it from accessing the howm API?
- Support for homepage "themes" or starter templates?
