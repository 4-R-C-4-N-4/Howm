# NAT Punch Integration Tests

## COMPLETED

Design doc for the mock-based NAT hole punch test suite.

## Overview

The NAT hole punch (`node/daemon/src/punch.rs`) rotates WireGuard endpoints
until a handshake succeeds. The three WG operations it depends on — add peer,
set endpoint, check handshake — all shell out to `wg`. This makes the punch
loop untestable without a real WireGuard interface... until now.

## Architecture: WgControl Trait

```
┌─────────────────┐        ┌──────────────┐
│   run_punch()   │───────▶│  WgControl   │  (trait)
│  (punch loop)   │        │  - add_peer  │
│                 │        │  - set_ep    │
│  candidates     │        │  - check_hs  │
│  timing logic   │        └──────┬───────┘
│  retry/timeout  │               │
└─────────────────┘        ┌──────┴───────┐
                           │              │
                    ┌──────▼──┐    ┌──────▼───────┐
                    │ System  │    │  Mock        │
                    │ WgCtrl  │    │  WgCtrl      │
                    │ (prod)  │    │  (tests)     │
                    └─────────┘    └──────────────┘
```

**`WgControl` trait** (in `punch.rs`):
```rust
#[async_trait]
pub trait WgControl: Send + Sync {
    async fn add_peer(&self, config: &PunchConfig, wg_iface: &str) -> Result<()>;
    async fn set_endpoint(&self, wg_iface: &str, pubkey: &str, endpoint: &str) -> Result<()>;
    async fn check_handshake(&self, wg_iface: &str, pubkey: &str) -> Result<bool>;
}
```

**`SystemWgControl`** — production implementation, shells out to `wg` CLI.
**`MockWgControl`** — test implementation with configurable behaviors.

## Backwards Compatibility

Callers that used `run_punch(config, data_dir, iface, timeout)` now use
`run_punch_system(config, data_dir, iface, timeout)` — a thin wrapper that
creates a `SystemWgControl` and calls `run_punch(config, &wg, iface, timeout)`.

The `check_handshake_by_status()` public API is unchanged.

## Mock Configuration

The `MockWgControl` in `tests/punch_integration.rs` supports:

| Builder method | Behavior |
|---|---|
| `.succeed_on("ip:port")` | `check_handshake` returns true when this endpoint was last set |
| `.succeed_after(N)` | `check_handshake` returns true after N calls |
| `.with_add_peer_failure()` | `add_peer` always returns an error |
| `.with_failing_endpoints(&[..])` | `set_endpoint` fails for specific endpoints |

Observable state: `attempted_endpoints()`, `was_peer_added()`, check count.

## Test Scenarios (18 tests)

### NAT Topology Scenarios

| Test | Our NAT | Peer NAT | Expected | What it validates |
|---|---|---|---|---|
| `open_to_open_immediate_success` | Open | Open | Success on STUN port | Direct connectivity, first candidate wins |
| `cone_to_cone_stun_port` | Cone | Cone | Success on STUN port | Both probe simultaneously |
| `cone_vs_symmetric_stride_offset` | Cone | Symmetric (stride=4) | Success on +4 offset | Initiator role, stride prediction |
| `symmetric_to_symmetric_timeout` | Symmetric | Symmetric | Timeout | Neither can punch, proper timeout |
| `port_preserving_nat` | Cone | Cone | Success on base port | Zero stride, STUN == WG port |
| `negative_stride` | Cone | Symmetric (stride=-2) | Success on -2 offset | Decreasing port allocation |

### Punch Loop Mechanics

| Test | What it validates |
|---|---|
| `success_after_multiple_rotations` | Handshake after N attempts, rotation works |
| `candidate_cycling` | Endpoints wrap around when exhausted |
| `initiator_vs_responder_timing` | 1s vs 200ms interval based on `we_initiate` |
| `different_wg_and_stun_ports` | STUN port tried first, then WG port |

### Error Handling

| Test | What it validates |
|---|---|
| `add_peer_failure` | Returns `PunchResult::Error`, no endpoints attempted |
| `set_endpoint_partial_failure` | Some endpoints fail, punch recovers on others |

### Edge Cases

| Test | What it validates |
|---|---|
| `candidates_near_port_max` | No overflow at u16::MAX |
| `candidates_near_port_min` | No underflow near port 0 |
| `candidates_include_stride_predictions` | Stride offsets present in candidate list |
| `psk_passed_to_add_peer` | PSK flows through to WG peer config |
| `no_psk` | Punch works without PSK |

### End-to-End Pipeline

| Test | What it validates |
|---|---|
| `full_invite_accept_punch_pipeline` | accept::generate → accept::decode → PunchConfig → build_candidates → run_punch. Full data flow from invite exchange through successful punch. |

## Running

```bash
# Just the punch integration tests
cargo test --test punch_integration

# All tests (274 total)
cargo test
```

## Lib Crate Extraction

To support integration tests accessing internal modules, the daemon now has
both `lib.rs` (public module re-exports) and `main.rs` (binary entry point
that imports from the library). This is the standard Rust pattern for
bin+lib crates.

## Files Changed

- `node/daemon/Cargo.toml` — added `[lib]` target, `async-trait` dependency
- `node/daemon/src/lib.rs` — NEW: public module re-exports
- `node/daemon/src/main.rs` — uses `howm::*` imports instead of `mod` declarations
- `node/daemon/src/punch.rs` — extracted `WgControl` trait, `SystemWgControl`, `run_punch_system`
- `node/daemon/src/matchmake.rs` — `run_punch` → `run_punch_system`
- `node/daemon/src/api/node_routes.rs` — `run_punch` → `run_punch_system`
- `node/daemon/tests/punch_integration.rs` — NEW: 18 mock-based integration tests
