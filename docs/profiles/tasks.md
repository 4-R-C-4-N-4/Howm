# Profiles — Task List

## Phase 1: Data Model & Storage

- [ ] Create `profile.rs` module — Profile struct, load/save to `~/.local/howm/profile/`
- [ ] Add profile fields to config/identity as needed (or keep fully separate)
- [ ] Avatar file handling — validate format (png/jpg/webp), enforce size limit
- [ ] Homepage directory handling — resolve path, validate index.html exists

## Phase 2: API Endpoints

- [ ] `GET /profile` — return own profile (name, bio, avatar url, homepage status)
- [ ] `PUT /profile` — update name/bio
- [ ] `PUT /profile/avatar` — upload avatar image, save to profile dir
- [ ] `PUT /profile/homepage` — set homepage path (file or directory)
- [ ] `GET /profile/avatar` — serve avatar image (public, peers fetch this)
- [ ] `GET /profile/home` — serve homepage HTML (public)
- [ ] `GET /profile/home/*` — serve homepage assets (css, images, fonts)
- [ ] `GET /peer/{id}/profile` — proxy fetch a peer's profile over WG
- [ ] CSP headers on homepage routes — sandbox iframe, block access to howm API

## Phase 3: Profile Sync

- [ ] Push profile metadata (name, bio, avatar hash) to peers on connect
- [ ] Push profile updates to connected peers when profile changes
- [ ] Peers cache metadata locally (name/bio/avatar hash) for offline display
- [ ] Avatar fetch-on-demand from peer node (don't replicate the image)

## Phase 4: UI — Profile Settings

- [ ] Profile settings page — edit name, bio
- [ ] Avatar upload with preview + crop (optional)
- [ ] Homepage file picker — select .html file or directory
- [ ] Homepage live preview (iframe of your own page)
- [ ] Wire into Settings or new top-level nav item

## Phase 5: UI — Viewing Peer Profiles

- [ ] Peer list — show avatars next to names
- [ ] Peer detail page — profile card (name, bio, avatar)
- [ ] Peer detail page — homepage iframe (sandboxed, loaded from their node)
- [ ] Loading/error states for when peer node is unreachable
- [ ] Visual indicator for peers who have a homepage vs those who don't

## Notes

- Homepage is always served live from the source node, never cached/replicated
- Avatar is the only binary blob that gets transferred — keep it small
- Phase 1-2 can ship without sync (profile works locally first)
- Phase 3 can piggyback on existing peer messaging/notification infrastructure
