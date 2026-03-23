# P2P-CD Core Capability Catalog v1

**Author:** Ivy Darling
**Spec Reference:** P2P-CD-01 v0.3
**Status:** Baseline Capability Proposal — Draft
**Date:** 2026-03-21

---

## Design Principle

Every capability in the `core.*` namespace exists because it solves a problem
that **most P2P applications encounter regardless of domain**. If only
file-sharing apps need it, it belongs in `com.example.*`. If every node in
every mesh eventually reinvents it, it belongs here.

The catalog is organized into three tiers based on what layer of the network
stack the capability addresses:

1. **`core.session.*`** — keeping the connection alive, honest, and verified
2. **`core.network.*`** — understanding the topology you're operating in
3. **`core.data.*`** — getting bytes between peers

### Namespace Alignment

The spec's namespace grammar (§4.4) is `org.scope.capability.version`. In
this catalog:

- **org** = `core` (reserved per §4.4 for capabilities defined by the spec
  and its normative extensions)
- **scope** = one of `session`, `network`, `data` — aligned 1:1 with the
  tier structure so the namespace itself communicates the functional layer
- **capability** = a single intuitive word identifying the specific function
- **version** = `1` for all capabilities in this initial catalog

This produces names like `core.session.heartbeat.1` and
`core.data.blob.1` — readable, sortable, and self-documenting. The scope
segment serves as a natural grouping key: `ls core.session.*` gives you
everything about connection health; `ls core.data.*` gives you every
data movement primitive.

Application developers extending the protocol use the same pattern under
their own org prefix: `com.example.game.state.1`,
`org.freedesktop.desktop.share.1`.

---

## A Note on Trust

This protocol provides mechanisms for peers to exchange identity,
capability, and build information. It does not — and cannot — guarantee
that a remote peer is behaving honestly. A peer that controls its own
runtime can misrepresent any self-reported value, including build hashes,
capability compliance, and session state.

This is not a limitation unique to P2P-CD. It is a fundamental property
of distributed systems without hardware trust roots. The protocol's
trust model is the same as the spec's approach to authentication (§10.1):
**the protocol provides the mechanism; the trust decision is local.**

Peer operators are responsible for evaluating the trustworthiness of the
peers they connect to. Capabilities such as build attestation
(`core.session.attest.1`) improve visibility into what a peer claims to
be running, but visibility is not enforcement. Only connect to peers you
have reason to trust. Only relay traffic for peers whose build identity
you have verified through an out-of-band channel.

The capabilities in this catalog raise the cost of undetected misbehavior.
They do not eliminate it.

---

## `core.session.*` — Session Health

These capabilities are about the session itself. They answer: *is my peer
alive, is our timing consistent, and can I verify what they're running?*

---

### `core.session.heartbeat.1`

**Already normative in the spec (Appendix B.1). Included here under the
new naming convention.**

| Field | Value |
|---|---|
| Name | `core.session.heartbeat.1` |
| Role | `BOTH` (mutual: true) |
| Purpose | Session liveness detection |
| Params | `interval_ms: uint`, `timeout_ms: uint` |
| Scope | `ttl` applies; `rate_limit` does not |
| Message types | PING (4), PONG (5) |
| Conformance | **Mandatory** (§D.1) |

Both peers send PING at `interval_ms`. Failure to receive PONG within
`timeout_ms` triggers CAP_EXCHANGE re-entry. This is the foundation that
every other capability relies on — without liveness, you can't reason about
any other guarantee.

---

### `core.session.attest.1`

| Field | Value |
|---|---|
| Name | `core.session.attest.1` |
| Role | `BOTH` (mutual: true) |
| Purpose | Mutual exchange of build identity at session activation |
| Params | see `attest-params` below |
| Scope | `ttl` applies; `rate_limit` does not |
| Message types | BUILD_ATTEST (6) |
| Conformance | **Recommended** |

```
build-attest = { 1 : 6,
    2 : attest-params
}

attest-params = {
    1 : uint,          ; spec_version — protocol spec version this
                       ;   build targets (e.g. 1 for P2P-CD-01 v0.3)
    2 : tstr,          ; lib_name — implementation identifier
                       ;   (e.g. "p2pcd-core", "p2pcd-go", "my-fork")
    3 : tstr,          ; lib_version — semantic version of the library
                       ;   (e.g. "0.1.0")
    4 : tstr,          ; source_repo — canonical source URL
                       ;   (e.g. "https://github.com/org/p2pcd-core")
    5 : bstr,          ; source_hash — commit hash the build was
                       ;   produced from (20 bytes for SHA-1 git,
                       ;   32 bytes for SHA-256 git)
    6 : bstr,          ; binary_hash — hash of the running binary
                       ;   image, self-computed at startup
    7 : tstr,          ; hash_algorithm — IANA name for binary_hash
                       ;   (e.g. "sha-256")
    ? 8 : tstr,        ; build_target — compiler target triple
                       ;   (e.g. "x86_64-unknown-linux-gnu")
    ? 9 : tstr,        ; build_profile — "release" | "debug" | custom
    ? 10 : bstr,       ; signature — detached signature over
                       ;   canonical CBOR of fields 1–9
    ? 11 : bstr,       ; signer_id — public key of the signer
    ? 12 : [* patch-decl]  ; declared modifications to source
}

patch-decl = {
    1 : tstr,          ; patch_name — human-readable label
    2 : bstr           ; patch_hash — hash of the patch content
}
```

#### Behavior

When `core.session.attest.1` enters the active set after CONFIRM
reconciliation, both peers immediately send a single `build-attest`
message. That is the entire exchange. There is no challenge, no
ongoing re-verification, no response expected.

```
A -> B : build-attest { attest-params_A }
B -> A : build-attest { attest-params_B }
```

Each peer evaluates the received attestation against local policy
and updates the peer's classification accordingly. The evaluation
is entirely local — the protocol does not define pass/fail semantics.

#### What This Verifies

**Spec compatibility.** The `spec_version` field tells you immediately
whether the peer is targeting the same protocol revision. A peer
reporting `spec_version: 1` and your node running spec version 2 is
a clear signal of potential incompatibility that the manifest's
`protocol_version` alone may not capture (since `protocol_version` is
about wire format, not behavioral conformance to a spec revision).

**Implementation identity.** The `lib_name` and `lib_version` fields
tell you which implementation the peer is running. In a heterogeneous
ecosystem with Rust, Go, and Python implementations, knowing that
your peer is running `p2pcd-go v0.3.1` vs `p2pcd-core v0.1.0`
provides context for compatibility expectations and debugging.

**Source provenance.** The `source_repo` and `source_hash` fields
identify exactly which code the binary was built from. If the peer
claims to be built from a known upstream commit, you can cross-reference
against published releases. If it points to an unknown repo, it's a
fork — visible, not hidden.

**Binary integrity.** The `binary_hash` is computed by the process at
startup over its own executable image. For reproducible builds, the
expected binary hash for a given `(source_hash, build_target,
build_profile)` tuple is deterministic and can be published alongside
releases. A match means the peer is running the expected binary for
that commit. A mismatch means something is different — a patched
build, a different compiler version, a non-reproducible build
environment, or tampering.

**Build authority.** When `signature` and `signer_id` are present,
the attestation is bound to a specific build authority (release
maintainer, CI pipeline, reproducible build farm). The receiving peer
verifies the signature over fields 1–9 and checks `signer_id` against
a local trust list.

**Declared modifications.** The `patch-decl` array lets forks and
patched builds honestly declare what they've changed. A peer running
upstream with a single logging patch can declare it by name and hash.
The remote peer sees the modification and decides whether to trust it.

#### What This Does NOT Verify

**Runtime honesty.** A peer that controls its own process can make
`self_hash()` return any value. The `binary_hash` is self-reported.
A malicious peer can report the correct hash while running modified
code. This capability does not detect a determined adversary with
full control over their runtime. No software-only mechanism can.

**Behavioral conformance.** Knowing that a peer is running the
correct binary does not prove it will behave correctly. A conforming
binary running on a compromised OS, or with interposition via
LD_PRELOAD, or with a debugger attached, may produce conforming
hashes while behaving non-conformantly. Build attestation verifies
the static artifact, not the dynamic execution.

**Shared library integrity.** If the binary is dynamically linked,
`binary_hash` covers the main executable only. Compromised shared
libraries are not detected. Implementations that require stronger
coverage should statically link or extend the attestation with a
library manifest in application-defined scope parameters.

#### Threat Model Summary

| Threat | Detected? | Mechanism |
|---|---|---|
| Trojanized binary from compromised mirror | **Yes** | `binary_hash` mismatch against published release |
| Accidental dirty build on production mesh | **Yes** | `source_hash` doesn't match a release commit |
| Unknown fork joining the mesh | **Yes** | `source_repo` points to unrecognized repository |
| Patched build with undeclared modifications | **Partially** | `binary_hash` mismatch, but no `patch-decl` |
| Determined adversary forging all fields | **No** | Self-reported values; no hardware trust root |
| Runtime code injection (LD_PRELOAD, ptrace) | **No** | Binary on disk is correct; runtime is not |
| Compromised shared libraries | **No** | Only main executable is hashed |

Build attestation catches supply chain attacks, accidental divergence,
and undeclared forks. It does not catch a sophisticated adversary who
controls the runtime. It is a meaningful first line of defense, not
a guarantee. Trust trustworthy peers.

#### Interaction With Other Capabilities

The `ClassificationResolver` can use attestation data to inform trust
gate evaluation for all other capabilities in the session:

- A `core.network.relay.1` provider might restrict relay service to
  peers whose `binary_hash` matches a known release.

- A `core.network.peerexchange.1` participant might weight PEX responses
  lower from peers running unrecognized forks.

These are policy decisions made by the local `ClassificationResolver`,
not protocol mandates.

---

### `core.session.timesync.1`

| Field | Value |
|---|---|
| Name | `core.session.timesync.1` |
| Role | `BOTH` (mutual: true) |
| Purpose | Clock offset estimation between peers |
| Params | `precision_ms: uint` — desired precision floor |
| Scope | `rate_limit` applies (sync probes/sec); `ttl` applies |
| Message types | TIME_REQ (7), TIME_RESP (8) |
| Conformance | Recommended |

```
time-req  = { 1 : 7,  2 : uint }   ; message_type=7, t1 (sender timestamp ms)
time-resp = { 1 : 8,  2 : uint,    ; message_type=8, t1 (echo)
                       3 : uint,    ; t2 (receiver timestamp at receipt)
                       4 : uint }   ; t3 (receiver timestamp at send)
```

NTP-style four-timestamp exchange. The initiator records t4 on receipt.
Offset = ((t2 - t1) + (t3 - t4)) / 2. RTT = (t4 - t1) - (t3 - t2).

**Why this belongs in core:** Coordinated timestamps show up in event
ordering (CRDTs, event logs), TTL enforcement, certificate validation,
log correlation, and replay detection. Every application that cares about
*when* something happened across peers needs a shared time reference.
Without it, `ttl` in scope params is only enforceable against local
clocks, which may drift arbitrarily between embedded/mobile peers.

**Negotiation:** `precision_ms` uses most-restrictive-wins (higher value =
less precise = less traffic). Peers that can't meet the requested precision
exclude this from their active set.

---

### `core.session.latency.1`

| Field | Value |
|---|---|
| Name | `core.session.latency.1` |
| Role | `BOTH` (mutual: true) |
| Purpose | One-way and round-trip latency measurement |
| Params | `sample_interval_ms: uint`, `window_size: uint` |
| Scope | `rate_limit` applies; `ttl` applies |
| Message types | LAT_PING (9), LAT_PONG (10) |
| Conformance | Optional |

```
lat-ping = { 1 : 9,  2 : uint,   ; message_type=9, sequence
                      3 : uint }  ; t_send (sender timestamp µs)
lat-pong = { 1 : 10, 2 : uint,   ; message_type=10, sequence (echo)
                      3 : uint,   ; t_send (echo)
                      4 : uint }  ; t_recv (responder timestamp µs)
```

Distinct from heartbeat. Heartbeat answers "are you alive?" with a binary
yes/no. Latency ping answers "how far away are you?" with a measurement.
The `window_size` param controls how many samples to retain for rolling
statistics (mean, p50, p99).

**Why this belongs in core:** Latency-aware peer selection is fundamental
to relay routing, stream quality adaptation, and any application that needs
to choose the "best" peer from a set. Pushing this to application-land
means every relay, every CDN mesh, and every game netcode reimplements it
with slightly different semantics.

---

## `core.network.*` — Network Awareness

These capabilities answer: *where am I, who else is out there, and how do
I reach peers I can't see directly?*

---

### `core.network.endpoint.1`

| Field | Value |
|---|---|
| Name | `core.network.endpoint.1` |
| Role | `PROVIDE` \| `CONSUME` |
| Purpose | Peer reports the requesting peer's externally observed network address |
| Params | `include_geo: bool` — whether to include approximate geolocation |
| Scope | `rate_limit` applies; `ttl` applies |
| Message types | WHOAMI_REQ (11), WHOAMI_RESP (12) |
| Conformance | Recommended |

```
whoami-req  = { 1 : 11 }
whoami-resp = { 1 : 12,
    2 : tstr,          ; observed_addr (e.g. "203.0.113.42:41312")
    3 : uint,          ; addr_family (4 = IPv4, 6 = IPv6)
    ? 4 : tstr,        ; observed_hostname (reverse DNS, if available)
    ? 5 : [float, float]  ; approx_geo [lat, lon] (if include_geo)
}
```

This is the P2P-CD equivalent of STUN or libp2p's `identify/observed-addr`.
The PROVIDE peer looks at the transport-level source address of the
authenticated session and reports it back. The CONSUME peer learns what
its public-facing address looks like from the perspective of this specific
peer.

**Why this belongs in core:** NAT traversal, multi-homing, and peer
advertisement all require a node to know its own externally visible address.
This is the single most common bootstrapping problem in P2P. Without it,
peers behind NAT can't advertise reachable endpoints to third parties.
Scoping it as a capability (rather than baking it into the handshake) means
peers can *choose* not to reveal this information and nodes behind privacy
layers can decline to provide it.

**Role semantics:** PROVIDE peers act as reflectors. CONSUME peers are
asking "what do I look like to you?" A public relay node would PROVIDE
this to all peers. A private node might CONSUME from multiple peers to
triangulate its NAT situation.

---

### `core.network.relay.1`

| Field | Value |
|---|---|
| Name | `core.network.relay.1` |
| Role | `PROVIDE` \| `CONSUME` |
| Purpose | Third-party relay — forwarding framed messages between two peers that cannot directly connect |
| Params | `max_circuits: uint`, `max_bandwidth_kbps: uint`, `relay_ttl: uint` |
| Scope | `rate_limit` applies (circuit creation rate); `ttl` applies (circuit lifetime) |
| Message types | CIRCUIT_OPEN (13), CIRCUIT_DATA (14), CIRCUIT_CLOSE (15) |
| Conformance | Recommended |

```
circuit-open = { 1 : 13,
    2 : bstr,          ; target_peer_id
    3 : uint,          ; requested_bandwidth_kbps
    ? 4 : uint         ; requested_ttl (seconds)
}
circuit-data = { 1 : 14,
    2 : uint,          ; circuit_id (assigned by relay)
    3 : bstr           ; payload (opaque, end-to-end encrypted by peers)
}
circuit-close = { 1 : 15,
    2 : uint,          ; circuit_id
    ? 3 : uint         ; reason (0=normal, 1=target_unreachable,
                       ;         2=bandwidth_exceeded, 3=ttl_expired)
}
```

A PROVIDE peer acts as a public relay. A CONSUME peer requests a circuit
to a target peer that it cannot reach directly. The relay forwards opaque
frames; it never inspects payload content. The relay enforces
`max_bandwidth_kbps` and `relay_ttl` per circuit.

**Why this belongs in core:** Relaying is the escape hatch for
symmetric NAT, CGNAT, restrictive firewalls, and every other situation
where two peers that want to talk cannot establish a direct path. Libp2p
has circuit relay v2, Tor has its onion circuits, TURN exists for
WebRTC — this is the P2P-CD equivalent. Without a standard relay
capability, every application that hits a NAT wall has to invent its own
relay protocol or give up.

**Critical design constraint:** The relay MUST NOT have access to
plaintext content. All `circuit-data` payloads are end-to-end encrypted
between the two endpoint peers using their own session keys. The relay
only sees ciphertext. This is a MUST-level requirement in the capability
definition, not a SHOULD.

**Negotiation:** `max_bandwidth_kbps` uses most-restrictive-wins.
`max_circuits` is provider-takes-precedence (the relay decides its own
capacity). `relay_ttl` uses most-restrictive-wins.

---

### `core.network.peerexchange.1`

| Field | Value |
|---|---|
| Name | `core.network.peerexchange.1` |
| Role | `BOTH` (mutual: true) |
| Purpose | Gossip-style exchange of known peer addresses and capability hashes |
| Params | `max_peers: uint`, `include_capabilities: bool` |
| Scope | `rate_limit` applies (exchanges/sec); `ttl` applies |
| Message types | PEX_REQ (16), PEX_RESP (17) |
| Conformance | Recommended |

```
pex-req = { 1 : 16,
    ? 2 : tstr,        ; filter_capability (only return peers offering this)
    ? 3 : uint         ; max_results
}
pex-resp = { 1 : 17,
    2 : [* peer-info]
}
peer-info = {
    1 : bstr,          ; peer_id
    2 : bstr,          ; personal_hash
    3 : tstr,          ; reachable_addr
    4 : uint,          ; last_seen (seconds since epoch)
    ? 5 : [* tstr]     ; capability_names (if include_capabilities)
}
```

Peer Exchange is how sparse networks bootstrap into connected meshes
without relying on a central rendezvous server. Peers share their
knowledge of other reachable peers, optionally filtered by capability.

**Why this belongs in core:** The spec explicitly puts peer *discovery
transport* out of scope (§1.1). But once you have even two connected
peers, the most natural way to find more peers is to ask the ones you
already know. PEX is the connective tissue between "I found one peer
via mDNS" and "I'm connected to a mesh." Every BitTorrent client,
every IPFS node, and every blockchain full node implements some variant
of this. Standardizing the exchange format in core means heterogeneous
applications on the same P2P-CD mesh can share peer knowledge even if
they have no capabilities in common.

**Security consideration:** A peer MUST NOT include peers in a PEX
response that have not consented to peer exchange (i.e., that are not
themselves advertising `core.network.peerexchange.1`). The `personal_hash`
is included so the receiver can apply auto-deny checks before attempting
connection.

---

## `core.data.*` — Data Movement

These capabilities answer: *how do bytes flow between peers once the
session is active?*

---

### `core.data.stream.1`

**Already defined in the spec (Appendix B.2). Included here under the
new naming convention.**

| Field | Value |
|---|---|
| Name | `core.data.stream.1` |
| Role | `PROVIDE` \| `CONSUME` |
| Purpose | Unidirectional continuous data stream |
| Params | `bitrate_kbps: uint`, `codec: tstr` |
| Scope | `ttl` applies; `rate_limit` does not |
| Message types | (defined in spec Appendix B.2, no new types allocated here) |
| Conformance | Optional |

**Negotiation:** `bitrate_kbps` uses most-restrictive-wins. `codec` uses
provider-takes-precedence per §7.3 fallback for non-numeric params.

---

### `core.data.blob.1`

| Field | Value |
|---|---|
| Name | `core.data.blob.1` |
| Role | `PROVIDE` \| `CONSUME` |
| Purpose | Reliable transfer of a finite, integrity-verified blob |
| Params | `max_blob_bytes: uint`, `chunk_size: uint`, `hash_algorithm: tstr` |
| Scope | `rate_limit` applies (bytes/sec); `ttl` applies |
| Message types | BLOB_REQ (18), BLOB_OFFER (19), BLOB_CHUNK (20), BLOB_ACK (21) |
| Conformance | Recommended |

```
blob-req = { 1 : 18,
    2 : bstr,          ; content_hash (identifies the blob)
    3 : tstr,          ; hash_algorithm
    ? 4 : uint         ; offset (resume from byte N)
}
blob-offer = { 1 : 19,
    2 : bstr,          ; content_hash (echo)
    3 : uint,          ; total_size (bytes)
    4 : uint,          ; chunk_size (bytes)
    ? 5 : uint         ; status (0=available, 1=not_found, 2=too_large)
}
blob-chunk = { 1 : 20,
    2 : bstr,          ; content_hash
    3 : uint,          ; offset
    4 : bstr           ; data
}
blob-ack = { 1 : 21,
    2 : bstr,          ; content_hash
    3 : uint,          ; bytes_received
    ? 4 : uint         ; status (0=continue, 1=complete, 2=hash_mismatch, 3=abort)
}
```

Content-addressed blob transfer with chunking, resume, and integrity
verification. The blob is identified by its hash, not by a filename or
path. The consumer requests a blob by hash; the provider streams chunks;
the consumer verifies the hash over the complete reassembled blob.

**Why this belongs in core:** `core.data.stream.1` is for continuous
data (audio, video, telemetry). Neither it nor generic RPC covers the
fundamental case of "I have 47 MB of data identified by its hash and I
want you to have it too." This is the primitive underneath file sync,
software distribution, block exchange, and content-addressable storage.
Making it content-addressed at the protocol level means deduplication and
integrity verification are free — two peers requesting the same blob from
different providers get identical bytes, verified by hash.

**Resume semantics:** The `offset` field in `blob-req` enables resume
after transport interruption. The provider starts sending from that
offset. This is critical for large blobs over unreliable links.

**Negotiation:** `max_blob_bytes` is provider-takes-precedence (the
provider knows its storage). `chunk_size` uses most-restrictive-wins.

---

### `core.data.rpc.1`

| Field | Value |
|---|---|
| Name | `core.data.rpc.1` |
| Role | `PROVIDE` \| `CONSUME` |
| Purpose | Generic request/response RPC over the P2P-CD session |
| Params | `max_request_bytes: uint`, `max_response_bytes: uint`, `methods: [tstr]` |
| Scope | `rate_limit` applies; `ttl` applies |
| Message types | RPC_REQ (22), RPC_RESP (23) |
| Conformance | Optional |

```
rpc-req = { 1 : 22,
    2 : uint,          ; request_id
    3 : tstr,          ; method
    4 : bstr           ; payload (CBOR-encoded, method-specific)
}
rpc-resp = { 1 : 23,
    2 : uint,          ; request_id (echo)
    3 : uint,          ; status (0=ok, 1=not_found, 2=error, 3=timeout)
    ? 4 : bstr         ; payload (CBOR-encoded, method-specific)
    ? 5 : tstr         ; error_message
}
```

Generic request/response semantics over an established P2P-CD session.
The `methods` param in the capability declaration advertises which RPC
methods the provider supports. Method semantics are defined by the
application; the core capability only standardizes the envelope.

**Why this belongs in core:** Streams and blobs cover continuous and bulk
data. But an enormous amount of peer interaction is simple
request/response: "give me this record," "update this state," "what's
your current status." Without a standard RPC envelope, every application
invents its own request ID tracking, error codes, and timeout semantics.
This is the boring, reliable request/response primitive that everything
else can build on.

**The `methods` param is the key design choice.** By listing supported
methods in the capability declaration, peers can inspect compatibility
*before* activation. A consumer that needs `getBlock` and `putBlock`
can see at OFFER time whether the provider supports those methods,
rather than discovering incompatibility after activation.

---

### `core.data.event.1`

| Field | Value |
|---|---|
| Name | `core.data.event.1` |
| Role | `PROVIDE` \| `CONSUME` \| `BOTH` |
| Purpose | Pub/sub event notification with topic filtering |
| Params | `topics: [tstr]`, `max_payload_bytes: uint` |
| Scope | `rate_limit` applies (events/sec); `ttl` applies |
| Message types | EVENT_SUB (24), EVENT_UNSUB (25), EVENT_MSG (26) |
| Conformance | Optional |

```
event-sub = { 1 : 24,
    2 : [* tstr]       ; topic_filters (prefix match)
}
event-unsub = { 1 : 25,
    2 : [* tstr]       ; topic_filters
}
event-msg = { 1 : 26,
    2 : tstr,          ; topic
    3 : bstr,          ; payload (opaque, application-defined)
    4 : uint,          ; timestamp_ms
    ? 5 : bstr         ; source_peer_id (originator, if relayed)
}
```

Topic-based pub/sub over an active session. A PROVIDE peer accepts
subscriptions and pushes matching events. A CONSUME peer subscribes
to topics and receives events. `BOTH` with `mutual: true` enables
bidirectional event exchange.

**Why this belongs in core:** Push-based event delivery is the complement
to pull-based data retrieval. Chat messages, sensor readings, state
change notifications, presence updates — these are all events. The RPC
capability handles request/response, blob transfer handles bulk data,
stream relay handles continuous data. Events handle real-time push.
Together these four data-movement primitives cover the fundamental access
patterns that every P2P application composes from.

**Topic filtering:** `topic_filters` use prefix matching.
Subscribing to `["chat."]` receives `chat.general`, `chat.random`,
etc. Subscribing to `[""]` (empty string prefix) receives everything.
The `topics` param in the capability declaration advertises available
topic prefixes so consumers can evaluate compatibility at OFFER time.

---

## Message Type Allocation Summary

| Type | Message | Capability | Status |
|---|---|---|---|
| 1 | OFFER | (protocol) | Normative §5.3.3 |
| 2 | CONFIRM | (protocol) | Normative §5.3.4 |
| 3 | CLOSE | (protocol) | Normative §5.3.5 |
| 4 | PING | `core.session.heartbeat.1` | Normative B.1 |
| 5 | PONG | `core.session.heartbeat.1` | Normative B.1 |
| 6 | BUILD_ATTEST | `core.session.attest.1` | Proposed |
| 7 | TIME_REQ | `core.session.timesync.1` | Proposed |
| 8 | TIME_RESP | `core.session.timesync.1` | Proposed |
| 9 | LAT_PING | `core.session.latency.1` | Proposed |
| 10 | LAT_PONG | `core.session.latency.1` | Proposed |
| 11 | WHOAMI_REQ | `core.network.endpoint.1` | Proposed |
| 12 | WHOAMI_RESP | `core.network.endpoint.1` | Proposed |
| 13 | CIRCUIT_OPEN | `core.network.relay.1` | Proposed |
| 14 | CIRCUIT_DATA | `core.network.relay.1` | Proposed |
| 15 | CIRCUIT_CLOSE | `core.network.relay.1` | Proposed |
| 16 | PEX_REQ | `core.network.peerexchange.1` | Proposed |
| 17 | PEX_RESP | `core.network.peerexchange.1` | Proposed |
| 18 | BLOB_REQ | `core.data.blob.1` | Proposed |
| 19 | BLOB_OFFER | `core.data.blob.1` | Proposed |
| 20 | BLOB_CHUNK | `core.data.blob.1` | Proposed |
| 21 | BLOB_ACK | `core.data.blob.1` | Proposed |
| 22 | RPC_REQ | `core.data.rpc.1` | Proposed |
| 23 | RPC_RESP | `core.data.rpc.1` | Proposed |
| 24 | EVENT_SUB | `core.data.event.1` | Proposed |
| 25 | EVENT_UNSUB | `core.data.event.1` | Proposed |
| 26 | EVENT_MSG | `core.data.event.1` | Proposed |
| 27–31 | (reserved) | — | Reserved for v2 |
| 32+ | (application) | (application) | Open |

Eleven capabilities, types 6–26, with five reserved slots (27–31) for
v2 additions such as protocol bridging (`core.bridge.*`). The
application-defined range starts at 32.

---

## Capability Dependency Graph

Some capabilities are more useful (or only meaningful) when other
capabilities are also active. These are **soft dependencies** — they
enhance behavior but don't prevent activation.

```
core.session.heartbeat.1          (no dependencies — foundational)
  └─ core.session.attest.1        (benefits from liveness guarantee)
  └─ core.session.timesync.1      (benefits from liveness guarantee)
  └─ core.session.latency.1       (benefits from liveness guarantee)

core.session.attest.1             (no hard dependencies)
  └─ (all other capabilities)     (trust gates informed by attestation)

core.network.endpoint.1           (no dependencies)
  └─ core.network.relay.1         (relay benefits from knowing peer endpoints)
  └─ core.network.peerexchange.1  (PEX benefits from knowing peer endpoints)

core.data.blob.1                  (no dependencies)
core.data.rpc.1                   (no dependencies)
core.data.stream.1                (no dependencies)
core.data.event.1                 (no dependencies)
```

No hard dependencies. Every capability can activate independently.
The dependency graph only indicates "works better when combined with."

---

## Catalog Summary

| # | Capability | Scope | Conformance | Msg Types |
|---|---|---|---|---|
| 1 | `core.session.heartbeat.1` | session | **Mandatory** | 4–5 |
| 2 | `core.session.attest.1` | session | **Recommended** | 6 |
| 3 | `core.session.timesync.1` | session | Recommended | 7–8 |
| 4 | `core.session.latency.1` | session | Optional | 9–10 |
| 5 | `core.network.endpoint.1` | network | Recommended | 11–12 |
| 6 | `core.network.relay.1` | network | Recommended | 13–15 |
| 7 | `core.network.peerexchange.1` | network | Recommended | 16–17 |
| 8 | `core.data.stream.1` | data | Optional | (spec B.2) |
| 9 | `core.data.blob.1` | data | Recommended | 18–21 |
| 10 | `core.data.rpc.1` | data | Optional | 22–23 |
| 11 | `core.data.event.1` | data | Optional | 24–26 |

One mandatory. Five recommended. Five optional.

---

## Extension Points for Developers

The catalog above is the v1 baseline. Here's how application developers
extend it:

**Building on `core.data.rpc.1`:**
Define your methods. The RPC capability handles the envelope; you
define `methods: ["getBlock", "putBlock", "getState"]` in your
capability declaration and handle the CBOR payloads in your
`CapabilityHandler`.

**Building on `core.data.blob.1`:**
Your application defines what content hashes mean and how blobs are
stored. The core capability handles chunking, resume, and integrity.
You decide what to serve and what to request.

**Building on `core.data.event.1`:**
Define your topic namespace. `chat.general`, `sensor.temperature.room3`,
`state.player.position` — the core capability handles subscription
management and delivery; your application defines the topic grammar
and payload schema.

**Building on `core.network.relay.1`:**
Your application can build overlay routing by chaining circuits through
multiple relay hops. The core capability handles single-hop relay;
multi-hop is an application concern composed from multiple circuit
instances.

**Building on `core.session.attest.1`:**
Define trust policies in your `ClassificationResolver` that use build
attestation data. Pin to signed releases for a production mesh. Allow
any known fork for a development mesh. Require matching `binary_hash`
for high-value relay nodes. The attestation data is just input to your
policy — the protocol doesn't prescribe what you do with it.

**Fully custom capabilities** (`com.yourorg.game.state.1`) that
don't build on any core capability use their own message types (33+)
and define their own wire format within the CBOR payload envelope.
The core library routes messages by type to the registered handler;
beyond that, the bytes are yours.

---

## Reserved for v2

The following capabilities were considered for v1 but deferred to reduce
implementation surface:

**`core.bridge.http.1`** — HTTP request proxying through a peer with
internet access. Critical for mesh and IoT deployments where not every
node can reach the public internet. Deferred because secure HTTP proxying
is a significant implementation lift with a large abuse surface that
requires careful design.

**`core.bridge.dns.1`** — DNS resolution through a peer with internet
access. Natural companion to HTTP bridging. Deferred alongside it.

Message type slots 27–31 are reserved for these and other v2 additions
under a `core.bridge.*` scope.

---

## What Was Deliberately Excluded

**`core.storage.*` (distributed storage, DHT):** Too opinionated.
DHT key-space partitioning, replication strategies, and consistency
models vary enormously across applications. Blob transfer gives you
content-addressed data movement; how you index and replicate is your
problem.

**`core.auth.*` (credential exchange, trust delegation):** The spec
puts identity and credential issuance out of scope (§1.1). The
`classification` field in capability declarations and the
`ClassificationResolver` trait are the right extension points.
Baking a specific credential exchange protocol into core would
prematurely constrain the trust model.

**`core.nat.*` (STUN/TURN/ICE negotiation):** NAT traversal
signaling is tightly coupled to the transport layer. The spec is
transport-agnostic (§1.1). `core.network.endpoint.1` gives you
the address reflection primitive; `core.network.relay.1` gives you
the relay fallback. The ICE-style candidate exchange that ties them
together is best defined per-transport-binding in an Appendix C
extension rather than in the capability namespace.

**Capability directory query:** The OFFER exchange already transmits
the full manifest. A post-activation query against a peer's capability
list is redundant with the data the protocol already provides. If a
peer's capabilities change, the rebroadcast mechanism (§8) triggers a
full re-exchange. No additional query mechanism is needed.

**Bidirectional stream:** Use `core.data.stream.1` with role `BOTH`
and `mutual: true`, or compose two unidirectional streams. A separate
primitive creates ambiguity about when to use which.

**Ongoing build challenge-response:** An earlier draft of build
attestation included periodic re-hashing challenges to detect runtime
binary replacement. This was removed because a peer that controls its
own runtime can intercept challenge-response as easily as it can forge
the initial hash. The mechanism added complexity and overhead for honest
peers while providing no additional security against dishonest ones.

---

*This catalog is a proposal. It should be reviewed against real
implementation experience from the reference build before any
capability name is registered in the P2P-CAPDISC Well-Known
Capabilities Registry (§11).*
