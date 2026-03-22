# DOCKERLESS Implementation Spec

Based on `DOCKERLESS.md` (v0.2 architecture proposal) and the current codebase.

---

## 0. Scope

This spec turns the high-level proposal into a phased, file-level implementation
plan for removing Docker, adding WASM + native capability execution, and
embedding userspace WireGuard via boringtun. Each phase is independently
shippable and testable.

---

## 1. Current State Summary

Key modules that will be touched or replaced:

```
node/daemon/src/docker.rs          -- bollard-based Docker lifecycle (312 lines)
node/daemon/src/wireguard.rs       -- WG via Docker container (830 lines)
node/daemon/src/capabilities.rs    -- CapabilityEntry struct, load/save (48 lines)
node/daemon/src/api/capability_routes.rs -- install/start/stop/restart via Docker (266 lines)
node/daemon/src/config.rs          -- CLI flags (62 lines)
node/daemon/src/state.rs           -- AppState with wg_container_id (51 lines)
node/daemon/src/main.rs            -- startup: Docker WG init, container restart loop
node/daemon/src/discovery.rs       -- peer capability crawling
node/daemon/Cargo.toml             -- depends on bollard (Docker client)
```

Current data model (capabilities.json):
```json
{
  "name": "social.feed",
  "version": "0.1.0",
  "port": 7001,
  "container_id": "44a1b62c...",
  "image": "cap-social-feed:0.1",
  "status": "Running",
  "visibility": "friends"
}
```

Current WG config:
- Subnet: 10.47.0.0/16
- Port: 51820
- Interface: wg0 (inside Docker container)
- Runs via linuxserver/wireguard Docker image

---

## 2. Phase 1 — Capability Manifest System

### Goal
Decouple capability metadata from Docker. Introduce manifest.json as the
source of truth for capability config, replacing the image-embedded
capability.yaml.

### Changes

**New file: `node/daemon/src/manifest.rs`**
- Define `CapabilityManifest` struct:
  ```
  name: String
  version: String
  type: CapType           // enum { Wasm, Native }
  entry: String           // "module.wasm" or "social-feed" (binary name)
  permissions: Permissions
  port: Option<u16>       // service port the capability listens on
  resources: Option<ResourceLimits>
  visibility: String      // "private" | "friends" | "public"
  ```
- `Permissions` struct:
  ```
  network: bool
  filesystem: Vec<String>   // allowed paths relative to capability data dir
  ```
- `ResourceLimits` struct:
  ```
  memory: Option<String>    // "256m", "1g"
  cpu: Option<String>       // "0.5", "2"
  ```
- Validation function: `validate_manifest(path) -> Result<CapabilityManifest>`
- Load from: `~/.howm/capabilities/<name>/manifest.json`

**Modify: `node/daemon/src/capabilities.rs`**
- Add `cap_type: CapType` field to `CapabilityEntry` (default: `Native` for
  backward compat, but `Docker` as legacy variant during transition)
- Replace `container_id: String` with `runtime_id: String` (generic handle —
  could be a container ID, PID, or WASM instance ID)
- Replace `image: String` with `source: CapSource` enum:
  ```
  enum CapSource {
      Docker { image: String },
      Wasm { module_path: PathBuf },
      Native { binary_path: PathBuf },
  }
  ```
- Keep `load()` / `save()` backward-compatible with existing capabilities.json
  via serde defaults

**New directory layout** (~/.howm/):
```
~/.howm/
  capabilities/
    social-feed/
      manifest.json
      module.wasm          (if WASM)
      social-feed          (if native binary)
      data/                (capability-scoped data)
  config/
  network/
```

### Questions

> Q1: Should we support BOTH the old Docker-based capabilities and new
> manifest-based capabilities simultaneously during migration? Or do we cut
> over all at once?
>
> The codebase currently has two social.feed entries in capabilities.json
> pointing at Docker images. We need a migration story.

> Q2: The current `capability.yaml` is read from INSIDE the running Docker
> container (docker exec cat). The new manifest.json lives on the host
> filesystem. During migration, should `howm capability install` accept both
> a Docker image AND a local directory path?

> Q3: The proposal says capabilities install to `~/.howm/capabilities/`.
> The current data_dir is `./data` (relative, configurable via --data-dir).
> Should we:
>   (a) keep using the configurable data_dir and put capabilities under it
>   (b) hardcode ~/.howm/ as the proposal suggests
>   (c) default to ~/.howm/ but allow override

---

## 3. Phase 2 — Native Capability Executor

### Goal
Run capabilities as native child processes managed by the daemon, no Docker.

### Changes

**New file: `node/daemon/src/executor/mod.rs`**
- Trait: `CapabilityExecutor`
  ```rust
  #[async_trait]
  trait CapabilityExecutor {
      async fn start(&self, manifest: &CapabilityManifest, data_dir: &Path) -> Result<RuntimeHandle>;
      async fn stop(&self, handle: &RuntimeHandle) -> Result<()>;
      async fn status(&self, handle: &RuntimeHandle) -> Result<CapStatus>;
      async fn logs(&self, handle: &RuntimeHandle, lines: usize) -> Result<Vec<String>>;
  }
  ```
- `RuntimeHandle`:
  ```
  id: String          // PID as string, or container ID, or WASM instance ID
  pid: Option<u32>
  started_at: u64
  ```

**New file: `node/daemon/src/executor/native.rs`**
- Implements `CapabilityExecutor` for native binaries
- Spawns child process via `tokio::process::Command`
- Sets environment:
  ```
  PORT=<assigned_port>
  HOWM_DATA_DIR=<capability_data_dir>
  HOWM_CAP_NAME=<name>
  ```
- Sandbox (Linux): use `Command::pre_exec` to:
  - Set up mount namespace (unshare)
  - Apply seccomp filter (if available)
  - Set cgroup limits
- Sandbox (macOS/Windows): process isolation + filesystem permissions only
- Stdout/stderr captured to log files under `~/.howm/capabilities/<name>/logs/`
- Health check: periodic HTTP GET to `http://127.0.0.1:<port>/health`
- Process monitoring: tokio task watches child, updates CapStatus on exit

**Modify: `node/daemon/src/api/capability_routes.rs`**
- `install_capability`: accept `InstallRequest` with either:
  ```json
  { "path": "./social-feed" }       // local directory with manifest.json
  ```
  or (later, for WASM registry):
  ```json
  { "url": "https://registry.howm.network/social-feed/0.1.0" }
  ```
- `start_capability`: dispatch to native executor based on manifest type
- `stop_capability`: dispatch to native executor
- Keep Docker path behind a feature flag or `cap_type` match during migration

**Modify: `node/daemon/src/main.rs`**
- Capability restart loop: check PID alive instead of Docker container health
- Add process reaper / monitor task

### Linux Isolation Detail

For native capabilities on Linux, the executor should:
1. Create a new mount namespace
2. Bind-mount only the allowed filesystem paths from manifest.permissions
3. Drop all capabilities except what's needed
4. Apply seccomp filter to block dangerous syscalls
5. Set memory/CPU cgroup limits from manifest.resources

For macOS, we rely on:
1. Process-level sandboxing (sandbox-exec or just rlimit)
2. Filesystem permissions (chroot is not practical without root)
3. Resource limits via setrlimit

### Questions

> Q4: How much Linux sandboxing do we want in v1? Full namespace isolation
> (requires root or CAP_SYS_ADMIN) vs. simple process isolation (no root
> needed)?
>
> The proposal lists namespaces + seccomp + cgroups but these all need elevated
> privileges. If howm runs as a regular user, we can't use real namespaces
> without user namespaces (which some distros disable). Options:
>   (a) Require root / sudo for full isolation
>   (b) Use user namespaces where available, degrade gracefully
>   (c) Skip kernel isolation in v1, rely on filesystem permissions + seccomp

> Q5: Should capabilities communicate with the daemon via HTTP only (current
> model) or should we add a Unix socket / IPC channel for richer interaction
> (lifecycle signals, log streaming, etc.)?

> Q6: Log management — should we:
>   (a) capture stdout/stderr to rotating log files
>   (b) pipe through the daemon's tracing subscriber
>   (c) both — files for persistence, tracing for real-time

---

## 4. Phase 3 — WASM Capability Executor

### Goal
Run WASM capabilities via wasmtime, sandboxed and portable.

### Changes

**New dependency in Cargo.toml:**
```toml
wasmtime = "27"
wasmtime-wasi = "27"
```

**New file: `node/daemon/src/executor/wasm.rs`**
- Implements `CapabilityExecutor` for WASM modules
- Uses wasmtime with WASI:
  ```rust
  let engine = Engine::default();
  let module = Module::from_file(&engine, &manifest.entry)?;
  let linker = Linker::new(&engine);
  wasmtime_wasi::add_to_linker(&linker, |s| s)?;
  ```
- WASI configuration:
  - Preopened dirs: only `./data` (scoped to capability)
  - Env vars: PORT, HOWM_DATA_DIR
  - Network: if permissions.network == true, allow outbound TCP
  - Memory limits from manifest.resources
  - Inherit stdout/stderr for logging
- Runs in a separate tokio blocking task (WASM execution is CPU-bound)
- Health check same as native: HTTP GET to localhost:<port>

### WASM Networking

This is the tricky part. WASI preview 1 does NOT have socket support.
WASI preview 2 has `wasi:sockets` but wasmtime support is still evolving.

Options:
1. **WASI preview 2 sockets** — if wasmtime supports it by the time we build
2. **Host-provided proxy** — daemon opens the port and proxies to the WASM
   module via shared memory or function calls
3. **WASI-http** — capability exposes an HTTP handler, daemon calls it

### Questions

> Q7: Which WASM networking model should we target? This fundamentally shapes
> how WASM capabilities expose HTTP services. Options:
>   (a) WASI preview 2 sockets (most natural, but wasmtime support TBD)
>   (b) wasi-http component model (daemon proxies HTTP to WASM handler)
>   (c) Skip WASM networking in v1, WASM caps are compute-only (no ports)

> Q8: Should WASM capabilities be compiled to the component model
> (wasm32-wasip2) or classic WASI (wasm32-wasip1)? Component model is the
> future but toolchain support is still maturing.

> Q9: The wasmtime dependency is large (~30MB added to binary). Is that
> acceptable for the "single binary" goal? Alternative: make wasmtime an
> optional feature flag so users who don't need WASM get a smaller binary.

---

## 5. Phase 4 — Embedded WireGuard via boringtun

### Goal
Replace the Docker-based WireGuard with in-process userspace WireGuard.

### Changes

**New dependencies in Cargo.toml:**
```toml
boringtun = "0.6"
tun = "0.7"             # TUN interface creation (cross-platform)
```

**Replace: `node/daemon/src/wireguard.rs`** (complete rewrite)

New module structure:
```
node/daemon/src/network/
  mod.rs            -- public API (init, add_peer, remove_peer, status)
  tunnel.rs         -- boringtun tunnel management
  interface.rs      -- TUN interface creation (platform-specific)
  routing.rs        -- subnet routing rules
  conflict.rs       -- VPN conflict detection
```

**`network/interface.rs`**
- Create TUN interface named `howm0` (fallback: howm1, howm2...)
- Linux: open /dev/net/tun, set IFF_TUN | IFF_NO_PI
- macOS: use utunX
- Windows: use wintun.dll
- Assign address from 100.120.0.0/16 subnet
- Set MTU (1420 typical for WG)

**`network/tunnel.rs`**
- Initialize boringtun Tunn per peer
- Packet loop:
  ```
  loop {
      select! {
          // Read from TUN -> encrypt via boringtun -> send UDP
          packet = tun.read() => {
              tunn.encapsulate(packet, &mut buf);
              udp_socket.send_to(&buf, peer_endpoint);
          }
          // Read from UDP -> decrypt via boringtun -> write to TUN
          packet = udp_socket.recv() => {
              tunn.decapsulate(packet, &mut buf);
              tun.write(&buf);
          }
          // Timer tick for keepalive
          _ = timer.tick() => {
              tunn.update_timers(&mut buf);
          }
      }
  }
  ```

**`network/routing.rs`**
- Add route: 100.120.0.0/16 via howm0
- Linux: `ip route add 100.120.0.0/16 dev howm0`
- macOS: `route add -net 100.120.0.0/16 -interface utunX`
- NEVER touch default route
- Cleanup routes on shutdown

**`network/conflict.rs`**
- On startup, enumerate interfaces
- Check for: tailscale0, wg0, wg1, utun*, tun*
- Check for subnet overlap with 100.120.0.0/16
- Log warnings but continue

**Modify: `node/daemon/src/config.rs`**
- Change default WG port from 51820 to 43192
- Add: `--wg-interface` (default: "howm0")
- Add: `--wg-subnet` (default: "100.120.0.0/16")
- Keep `--no-wg` flag

**Modify: `node/daemon/src/state.rs`**
- Replace `wg_container_id: Arc<RwLock<Option<String>>>` with:
  ```rust
  wg_state: Arc<RwLock<Option<NetworkState>>>
  ```
  where NetworkState holds the tunnel handles, TUN fd, UDP socket, etc.

**Modify: `node/daemon/src/main.rs`**
- Replace `wireguard::init()` (which starts Docker) with
  `network::init()` (which starts boringtun)
- Spawn packet processing task
- Graceful shutdown: close TUN, cleanup routes

### Subnet Migration

Current subnet: 10.47.0.0/16
Proposed subnet: 100.120.0.0/16

This is a BREAKING CHANGE for existing peers.

### Questions

> Q10: The subnet change from 10.47.0.0/16 to 100.120.0.0/16 breaks all
> existing peer connections. Options:
>   (a) Hard cutover — all nodes must upgrade simultaneously
>   (b) Support both subnets during migration (complex)
>   (c) Keep 10.47.0.0/16 as default, use 100.120 only for new deployments
>   (d) Make subnet configurable, default to 100.120 for new installs

> Q11: boringtun operates in userspace but TUN interface creation typically
> needs root/sudo (Linux) or admin privileges (macOS). How should we handle
> this?
>   (a) Require sudo / elevated privileges at startup
>   (b) Use a small privileged helper binary for TUN setup, then drop privs
>   (c) Support a "no-TUN" mode that uses UDP proxying (no kernel interface)

> Q12: The current WG setup generates keys via `wg genkey` inside the Docker
> container. With boringtun embedded, we need to generate X25519 keys ourselves.
> boringtun provides this, but should we:
>   (a) Use boringtun's key generation directly
>   (b) Use the `x25519-dalek` crate for key generation (more control)
>   (c) Keep compatibility with existing keys in data/wireguard/

---

## 6. Phase 5 — Remove Docker Dependency

### Goal
Remove bollard and all Docker code paths.

### Changes

**Delete:**
- `node/daemon/src/docker.rs` (entire file)

**Remove from Cargo.toml:**
```
bollard = { ... }
```

**Modify: `node/daemon/src/main.rs`**
- Remove `mod docker;`
- Remove Docker container health check loop
- All capability lifecycle goes through executor trait

**Modify: `node/daemon/src/api/capability_routes.rs`**
- Remove all `docker::*` calls
- Install flow becomes:
  1. Copy/download capability to ~/.howm/capabilities/<name>/
  2. Validate manifest.json
  3. Launch via appropriate executor (native or wasm)

**Modify: `node/daemon/src/wireguard.rs`**
- File is already replaced in Phase 4, can be fully deleted
- All networking is in `node/daemon/src/network/`

**Update release CI (.github/workflows/release.yml):**
- No longer need Docker for integration tests
- Cross-compile for Linux, macOS, Windows
- Windows release includes wintun.dll

### Questions

> Q13: Should we keep a Docker executor as an optional/plugin capability type
> for users who want to run heavier workloads in containers? Or is Docker
> fully dead after this?

---

## 7. Phase 6 — CLI Overhaul

### Goal
Update CLI to match the proposal's UX.

### Changes

**Modify: `node/daemon/src/config.rs` + main.rs**

Restructure CLI with subcommands:
```
howm node start          -- start the daemon (replaces running daemon directly)
howm node stop           -- graceful shutdown
howm node status         -- show node info, WG state, peer count

howm capability install <path|url>  -- install from local dir or registry
howm capability start <name>        -- start a capability
howm capability stop <name>         -- stop a capability
howm capability list                -- list installed capabilities
howm capability logs <name>         -- tail capability logs
howm capability remove <name>       -- uninstall

howm peer list           -- list peers
howm peer add <invite>   -- join via invite link
howm peer remove <id>    -- remove peer

howm invite create       -- create invite link
howm invite open         -- create open invite
```

This requires restructuring from a single daemon binary to a CLI that can
either run as a daemon OR send commands to a running daemon via the HTTP API.

### Questions

> Q14: The proposal shows `howm node start` as the entry point. Currently
> the binary is called `daemon` and runs directly. Should we:
>   (a) Rename the binary to `howm` and add subcommand routing
>   (b) Keep `daemon` as the long-running process, add a separate `howm` CLI
>       that talks to the daemon API
>   (c) Single binary that forks into background when `howm node start` is run

> Q15: The current daemon API requires an auth token. The CLI subcommands
> would need to know this token. Where should it be stored?
>   (a) Written to ~/.howm/config/api_token on daemon start
>   (b) Printed to stdout on start, user passes via env var
>   (c) Unix socket auth (no token needed for local CLI)

---

## 8. Migration Checklist

For existing Howm users upgrading:

1. [ ] Existing capabilities.json format is backward-compatible (serde defaults)
2. [ ] Existing WG keys (data/wireguard/private_key) are reusable
3. [ ] Existing peer connections survive the subnet change (or migration plan)
4. [ ] Docker-based capabilities can be exported to manifest.json format
5. [ ] Data volumes (cap-data/) are preserved
6. [ ] node.json identity is preserved

---

## 9. Dependency Changes

### Added
```
wasmtime + wasmtime-wasi    -- WASM runtime (Phase 3)
boringtun                   -- userspace WireGuard (Phase 4)
tun / wintun                -- TUN interface (Phase 4)
x25519-dalek (maybe)        -- key generation (Phase 4)
```

### Removed
```
bollard                     -- Docker client (Phase 5)
```

### Kept
```
axum, tokio, serde, reqwest, clap, tracing  -- all stay
```

---

## 10. Testing Strategy

### Unit Tests
- Manifest parsing and validation
- Executor trait mock implementations
- Network subnet/interface allocation logic
- VPN conflict detection

### Integration Tests
- Native executor: start/stop/health-check a test capability
- WASM executor: load and run a simple WASI module
- Network: create TUN, send/receive packets through boringtun
- Full flow: install capability from directory, start it, query its API

### Platform Tests
- Linux: full test suite including namespace isolation
- macOS: TUN creation, basic networking
- Windows: wintun interface, basic networking (CI may need special setup)

---

## 11. Open Design Questions Summary

| #   | Topic                           | Options                          |
| --- | ------------------------------- | -------------------------------- |
| Q1  | Migration: dual Docker+native?  | simultaneous / cutover           |
| Q2  | Install source: image vs dir?   | both / dir-only                  |
| Q3  | Capability base path            | data_dir / ~/.howm / configurable|
| Q4  | Linux sandbox depth in v1       | root / user-ns / minimal         |
| Q5  | Cap-to-daemon IPC               | HTTP only / Unix socket / both   |
| Q6  | Log management                  | files / tracing / both           |
| Q7  | WASM networking model            | wasi-sockets / wasi-http / none  |
| Q8  | WASM target                     | wasip1 / wasip2 component model  |
| Q9  | wasmtime binary size            | always / feature flag             |
| Q10 | Subnet migration                | hard cut / dual / configurable   |
| Q11 | TUN privileges                  | sudo / helper binary / no-TUN    |
| Q12 | Key generation                  | boringtun / x25519-dalek / compat|
| Q13 | Keep Docker as option?          | yes / no                         |
| Q14 | Binary structure                | single howm / daemon+cli / fork  |
| Q15 | CLI auth to daemon              | token file / env var / unix sock |

---

## 12. Suggested Implementation Order

```
Phase 1  [~1 week]   Manifest system + CapabilityEntry refactor
Phase 2  [~2 weeks]  Native executor (biggest change to capability lifecycle)
Phase 3  [~1-2 weeks] WASM executor (can parallel with Phase 4)
Phase 4  [~2 weeks]  Embedded WireGuard (boringtun + TUN)
Phase 5  [~1 week]   Remove Docker, clean up
Phase 6  [~1 week]   CLI restructure
```

Total estimate: ~6-8 weeks for a single developer, assuming answers to
the open questions don't require significant design pivots.

---

## 13. Design Decisions

Answers to the open questions above, decided March 2026.

**Q1 & Q2 — Migration strategy: Clean break, no Docker.**
A new branch has been cut for this transition. No backwards compatibility with
the Docker-based capability system. Rip Docker out completely — no dual-mode,
no migration path. The old `capability.yaml`-in-container approach is gone;
capabilities are native processes with host-side manifests.

**Q3 — Capability/data path: Always configurable, default `~/.local/howm/`.**
Both config and data default to `~/.local/howm/` (following XDG-ish conventions).
Fully overridable via `--data-dir` / `HOWM_DATA_DIR`. No hardcoded paths.

**Q4 — Sandboxing: Installer with root, saved namespace permissions.**
Use an installer step that requires root to set up the necessary permissions
(e.g. granting `CAP_SYS_ADMIN` or configuring user namespaces via
`/proc/sys/kernel/unprivileged_userns_clone` or a setuid helper). After
installation, the daemon itself runs unprivileged but retains the ability to
create namespaces for capability isolation. No root needed at runtime.

**Q5 — IPC: Unix socket + UDP.**
Both Unix socket and UDP communication channels should be available for
capability-to-daemon interaction. HTTP alone is insufficient for the richer
lifecycle management needed (signals, log streaming, peer events). Unix socket
for local IPC, UDP for network-facing capability traffic.

**Q6 — Logging: Log files only, no stdout unless debug mode.**
Move to file-based logging by default. stdout/stderr output only when running
in debug mode (`RUST_LOG=debug` or `--debug` flag). Log files should rotate
and live under the data directory.

**Q7, Q8, Q9 — WASM: Scaffold but don't build yet.**
WASM/WASI support will be required for more intensive future capabilities and
should be scaffolded in the architecture (the `CapabilityExecutor` trait, manifest
`runtime` field, etc.). However, for the current social feed capability this is
overkill. Don't pull in wasmtime yet — leave the WASM executor as a stub/TODO
behind a feature flag. Build it when a real use case demands it.

**Q10 — Subnet: Hard cutover to `100.222.0.0/16`.**
Clean break from the old `10.47.0.0/16` subnet. New subnet is `100.222.0.0/16`.
All existing peer connections will need to be re-established. This aligns with
the "no backwards compatibility" decision from Q1/Q2.

**Q11 — TUN privileges: Installer sets up root permission, sudo for TUN step only.**
The installer runs with root to configure the system (e.g. setcap on the binary
or a small helper). At runtime, only the TUN interface creation step requires
elevated privileges (via sudo or the privileged helper). The rest of the daemon
runs unprivileged. This keeps the security surface minimal.

**Q12 — Key generation: Scrap existing keys, use whatever is easiest and cross-platform.**
No compatibility with the old Docker-based WG keys. Fresh key generation using
whichever crate is simplest and works across Linux, macOS, and Windows. boringtun's
built-in key generation or `x25519-dalek` — whichever has fewer dependencies and
better cross-platform support.

**Q13 — Docker: No.**
Docker is fully removed. No optional executor, no fallback. The entire point of
this transition is eliminating the Docker dependency. Capabilities are native
processes or (eventually) WASM modules.

**Q14 — Binary: Single `howm` binary including UI.**
One binary called `howm` with subcommand routing (`howm node start`, `howm cap install`,
etc.). The web UI is embedded as static assets served by the daemon. No separate
CLI binary, no forking — the user runs `howm` and gets everything.

**Q15 — CLI auth: Token file (option A).**
The daemon writes the API token to `~/.local/howm/config/api_token` on startup.
CLI subcommands read from this file automatically. Simple, no env vars to manage,
works across shell sessions.
