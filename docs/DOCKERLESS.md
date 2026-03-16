
# HOWM Runtime Refactor Specification

### Version

`v0.2 architecture proposal`

### Goals

1. **Remove Docker dependency**
2. Support **capabilities as WASM modules or native binaries**
3. Embed **userspace WireGuard networking**
4. Maintain **cross-platform support**
5. Avoid conflicts with existing VPN tools like Tailscale or WireGuard
6. Simplify deployment to **single binary per platform**

---

# 1. High Level Architecture

## Current Architecture (simplified)

```
host
├── docker: wireguard
├── docker: capability-A
├── docker: capability-B
└── docker: capability-C
```

Problems:

* Docker networking complexity
* slow capability startup
* heavy dependency
* difficult cross-platform UX

---

## Proposed Architecture

```
howm-node
│
├── network
│ └── embedded wireguard (boringtun)
│
├── runtime
│ ├── wasm executor
│ └── native executor
│
├── capability registry
│
├── capability manager
│
└── service discovery
```

Capabilities run as:

```
WASM modules
or
native binaries
```

Containers become **deprecated**.

---

# 2. Capability Execution Model

Capabilities are packaged applications executed by the Howm runtime.

## Supported Types

### WASM Capability

```
capabilities/
social-feed/
manifest.json
module.wasm
```

Executed using a WASI runtime.

Recommended runtime:

* Wasmtime

Advantages:

* sandboxed
* portable
* fast startup
* safe execution

---

### Native Capability

```
capabilities/
indexer/
manifest.json
indexer
```

Native executables launched by the runtime.

Isolation implemented via:

Linux:

```
namespaces
seccomp
cgroups
```

Mac / Windows:

```
process isolation
filesystem permissions
runtime policy
```

---

# 3. Capability Manifest Specification

File:

```
manifest.json
```

Example:

```json
{
"name": "social-feed",
"version": "0.1.0",
"type": "wasm",
"entry": "module.wasm",
"permissions": {
"network": true,
"filesystem": ["./data"]
},
"port": 8080
}
```

Supported fields:

| field | description |
| ----------- | ------------------------- |
| name | capability identifier |
| version | semantic version |
| type | wasm or native |
| entry | executable or wasm module |
| permissions | sandbox rules |
| port | optional service port |

---

# 4. Capability Runtime

## Runtime Responsibilities

```
load capability
validate manifest
create sandbox
launch process or wasm
monitor lifecycle
expose service endpoint
```

---

## WASM Execution

Example:

```
wasmtime module.wasm
```

Runtime configuration:

```
WASI enabled
filesystem sandbox
memory limits
network permissions
```

---

## Native Execution

Process launched by runtime:

```
./capability_binary
```

With:

```
restricted environment
limited filesystem access
optional network access
```

---

# 5. Embedded WireGuard Networking

Replace containerized WireGuard with embedded implementation.

Recommended engine:

boringtun

Advantages:

* Rust native
* high performance
* userspace
* cross-platform

---

## Networking Architecture

```
howm-node
│
├── boringtun engine
│
├── tun interface
│
└── peer routing
```

Packet flow:

```
capability
↓
OS network stack
↓
howm0 interface
↓
boringtun
↓
UDP
↓
remote peer
```

---

# 6. Network Interface Strategy

To prevent conflicts with existing VPNs:

Default interface name:

```
howm0
```

Never use:

```
wg0
```

If collision detected:

```
howm1
howm2
```

Runtime should dynamically allocate.

---

# 7. Network Subnet Allocation

Avoid overlap with common VPN ranges.

Recommended subnet:

```
100.120.0.0/16
```

Example node address:

```
100.120.0.10
```

Routing rule:

```
only route howm subnet via howm interface
```

Never modify default route.

---

# 8. UDP Port Allocation

Default port:

```
43192
```

Startup logic:

```
if port available:
bind port
else:
choose random high port
```

Range:

```
40000–60000
```

---

# 9. Firewall Policy

Howm should **not modify global firewall rules**.

Instead rely on:

```
userspace routing
existing OS firewall
```

If firewall rules are required:

```
scoped rules only
cleanup on shutdown
```

---

# 10. DNS Policy

Howm **must not modify system DNS**.

Avoid:

```
/etc/resolv.conf
system DNS configuration
```

Peer resolution handled via:

```
howm registry
```

Example:

```
resolve capability → peer IP
```

---

# 11. VPN Conflict Avoidance

Runtime startup checks for interfaces:

```
tailscale0
wg0
wg1
```

If detected:

```
log warning
continue startup
```

Example log:

```
existing VPN detected: tailscale0
Howm will use interface howm0
```

This ensures compatibility with:

* Tailscale
* WireGuard
* Netbird

---

# 12. Cross Platform Implementation

## Linux

TUN interface:

```
/dev/net/tun
```

Network stack:

```
boringtun + tun
```

Isolation:

```
namespaces
seccomp
```

---

## macOS

TUN interface:

```
utunX
```

Networking:

```
boringtun
```

---

## Windows

Driver:

Wintun

Package includes:

```
howm.exe
wintun.dll
```

---

# 13. Capability Discovery

Capabilities register with runtime:

```
register capability
expose metadata
announce endpoint
```

Example registry entry:

```
social-feed
version: 0.1
node: 100.120.0.10
port: 8080
```

---

# 14. CLI Behavior

Start node:

```
howm node start
```

Output:

```
wireguard interface: howm0
address: 100.120.0.10
listening port: 43192
```

Install capability:

```
howm capability install ./social-feed
```

Run capability:

```
howm capability start social-feed
```

---

# 15. Capability Filesystem Layout

```
~/.howm/

capabilities/
social-feed/
manifest.json
module.wasm

data/

config/

network/
```

---

# 16. Deployment Model

Release artifacts:

```
Linux:
howm

macOS:
howm

Windows:
howm.exe
wintun.dll
```

Single runtime binary.

No container runtime required.

---

# 17. Migration Plan

Step 1

Introduce capability manifest system.

Step 2

Add WASM runtime.

Step 3

Add native capability execution.

Step 4

Embed boringtun networking.

Step 5

Remove container capability requirement.

Step 6

Deprecate Docker support.

---

# 18. Expected Improvements

| Metric | Improvement |
| ---------------------- | --------------- |
| startup time | 10-100x faster |
| memory usage | lower |
| deployment complexity | greatly reduced |
| cross-platform support | improved |
| network stability | improved |

---

# 19. Future Extensions

Possible enhancements:

```
WASM capability marketplace
capability signing
distributed capability discovery
sandbox policy enforcement
```

---

✅ **Summary**

This refactor:

* removes Docker
* replaces containers with **WASM and native apps**
* embeds **WireGuard networking via boringtun**
* ensures compatibility with VPN tools like Tailscale
* produces a **single binary runtime**

Result:

```
howm node start
```

is sufficient to launch the full system.
