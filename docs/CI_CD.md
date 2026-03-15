# Howm CI/CD & Release Guide

## Overview

Howm uses two GitHub Actions workflows:

| Workflow              | File                          | Trigger                         |
|-----------------------|-------------------------------|---------------------------------|
| **CI**                | `.github/workflows/ci.yml`    | Push to `main`/`develop`, PRs   |
| **Release**           | `.github/workflows/release.yml` | Git tags matching `v*.*.*`    |

---

## CI Workflow

Runs automatically on every push and PR. Four parallel jobs:

### 1. Lint & Format
- Checks `cargo fmt` compliance
- Runs `clippy` with `-D warnings` (warnings = failure)

### 2. Build & Test (matrix)
Builds and tests across three platforms:
- `x86_64-unknown-linux-gnu` (ubuntu-latest)
- `aarch64-apple-darwin` (macos-latest)
- `x86_64-pc-windows-msvc` (windows-latest)

Binary artifacts are uploaded and retained for 7 days.

### 3. Docker
- Builds the `social-feed` capability image (no push, validation only)

### 4. Web UI
- Installs Node.js 20, runs `npm ci`, lint, and `npm run build`

---

## Release Workflow

Triggered by pushing a semver tag. Creates cross-platform binaries,
pushes Docker images to GHCR, and publishes a GitHub Release.

### Supported Targets

| Archive                      | Platform           | Architecture |
|------------------------------|--------------------|--------------|
| `howm-linux-amd64.tar.gz`    | Linux              | x86_64       |
| `howm-linux-arm64.tar.gz`    | Linux              | aarch64      |
| `howm-macos-arm64.tar.gz`    | macOS (Apple Silicon) | aarch64   |
| `howm-macos-amd64.tar.gz`    | macOS (Intel)      | x86_64       |
| `howm-windows-amd64.zip`     | Windows            | x86_64       |

### Docker Images

Published to GitHub Container Registry:
```
ghcr.io/<owner>/howm/cap-social-feed:<version>
ghcr.io/<owner>/howm/cap-social-feed:latest
```

### GitHub Release Contents
- Platform archives (tar.gz / zip)
- SHA256SUMS.txt with checksums for all archives
- Auto-generated changelog from commits since last tag

---

## How to Cut a Release

### 1. Update the version in Cargo.toml

```bash
# Edit node/daemon/Cargo.toml -> version = "0.2.0"
```

### 2. Commit and tag

```bash
git add -A
git commit -m "release: v0.2.0"
git tag v0.2.0
git push origin main --tags
```

### 3. Watch the workflow

Go to **Actions** tab in GitHub. The Release workflow will:
1. Build binaries for all 5 targets
2. Push Docker images to GHCR
3. Create a GitHub Release with archives + checksums

### Pre-releases

Use semver pre-release suffixes and the release will be marked as prerelease:

```bash
git tag v0.2.0-rc.1    # release candidate
git tag v0.2.0-beta.1  # beta
git tag v0.2.0-alpha.1 # alpha
```

---

## Branch Strategy

```
main        ← stable, releases cut from here
  └─ develop  ← integration branch
       └─ feature/*  ← PRs target develop
```

- PRs to `main` trigger CI
- Pushes to `main` and `develop` trigger CI
- Only tags on `main` should be used for releases

---

## Caching

Both workflows cache:
- `~/.cargo/registry` and `~/.cargo/git` (dependency downloads)
- `node/target` (build artifacts)

Cache keys are based on `Cargo.lock` hash, so updating dependencies
automatically invalidates caches.

---

## Required Repository Settings

### Branch Protection (recommended)

For `main`:
- Require PR reviews before merging
- Require status checks to pass (select: `lint`, `build`, `docker`, `web-ui`)
- Require branches to be up to date before merging

### Secrets

No additional secrets needed. The workflows use:
- `GITHUB_TOKEN` (auto-provided) for GHCR login and release creation

### Permissions

The release workflow needs `contents: write` and `packages: write`.
These are declared in the workflow files. If your repo uses restrictive
default permissions, ensure these are allowed in:
**Settings → Actions → General → Workflow permissions**

---

## Troubleshooting

### CI fails on format check
```bash
cd node && cargo fmt --all    # auto-fix locally, then commit
```

### CI fails on clippy
```bash
cd node && cargo clippy --all-targets --all-features
# Fix warnings, they are treated as errors
```

### Cross-compilation fails for linux-arm64
The workflow installs `gcc-aarch64-linux-gnu`. If linking fails,
check that your dependencies don't require platform-specific C libs
that aren't available in the cross toolchain.

### Release not created
- Ensure the tag matches `v*.*.*` exactly (e.g. `v0.1.0`, not `0.1.0`)
- Check that workflow permissions allow creating releases
