# Wallet Capability — Design Spec

## Overview

A wallet capability that enables peer-to-peer payments within the howm mesh.

Two primary use cases:

1. **Simple payments** — send crypto directly to any peer in your mesh. Open
   the wallet, pick a peer, send funds. No invoices, no access gates, just a
   transfer between two people.

2. **Pay-for-access** — a node operator sets a price on a resource (file,
   video, capability endpoint, etc.) and remote peers pay to unlock it. This
   is the invoice-based flow that other capabilities (files, future Tube, etc.)
   plug into via the wallet's RPC interface.

The wallet is chain-agnostic by design. The first implementation targets an
**EVM L2** (Base), but the architecture supports adding wallet backends for
other chains (Monero, Lightning, Solana, etc.) without changing the daemon
integration or UI contract.

---

## Architecture

```
Remote peer flow:

  Peer A (buyer)                    Peer B (seller)
  ─────────────                    ────────────────
  1. Browse files via P2PCD
     → catalogue includes prices
  2. Request paid file ─────────►  3. Files cap checks: price set?
                                   4. Yes → calls wallet cap RPC
                                      to create invoice
  5. ◄──────────────────────────   402 Payment Required + invoice
                                   { chain, address, amount, token,
                                     expiry, invoice_id }
  6. Wallet cap sends payment
  7. Re-request with invoice_id ►  8. Files cap calls wallet RPC
                                      to verify payment
                                   9. Confirmed → serve file
  10. ◄──────────────────────────   File data
```

Pricing lives in **the capability** (files, feed, etc.), NOT the daemon.
The daemon handles group-level access control (allow/deny). The capability
handles resource-level pricing. This means different capabilities can have
different pricing granularity — files prices per file, feed could price per
post, messaging stays free.

### Component Layout

```
┌─────────────────────────────────────────────────────────────┐
│                      howm daemon                             │
│                                                              │
│  access control (groups/CapabilityRule)                       │
│    "is this peer allowed to talk to this capability?"        │
│                                                              │
│  proxy.rs → forwards to capability if access = Allow         │
│             (no payment logic here)                           │
│                                                              │
└─────────────┬───────────────────────────────────────────────┘
              │ proxy
              ▼
┌─────────────────────┐      localhost RPC      ┌──────────────┐
│  files capability   │ ◄───────────────────► │ wallet cap    │
│                     │  create-invoice         │              │
│  per-offering price │  verify-payment         │ chain        │
│  402 flow           │                         │ backends     │
└─────────────────────┘                         └──────────────┘
```

---

## Components

### 1. Wallet Capability (standalone binary)

Location: `capabilities/wallet/`

A new capability binary following the same pattern as feed, files, messaging,
etc. It manages keypairs, tracks balances, and executes transactions.

#### Data Model (SQLite)

```sql
CREATE TABLE wallets (
    id          TEXT PRIMARY KEY,   -- uuid
    chain       TEXT NOT NULL,      -- "evm", "monero", "lightning", ...
    label       TEXT,               -- user-friendly name
    address     TEXT NOT NULL,      -- chain-native address
    created_at  INTEGER NOT NULL,
    is_default  INTEGER DEFAULT 0
);

-- Encrypted separately — never leaves the node
CREATE TABLE secrets (
    wallet_id   TEXT PRIMARY KEY REFERENCES wallets(id),
    encrypted   BLOB NOT NULL      -- ChaCha20-Poly1305 encrypted private key
);

-- All observed transactions (in + out)
CREATE TABLE transactions (
    id          TEXT PRIMARY KEY,   -- uuid
    wallet_id   TEXT NOT NULL REFERENCES wallets(id),
    direction   TEXT NOT NULL,      -- "in" | "out"
    chain_tx_id TEXT,               -- on-chain tx hash (null while pending)
    from_addr   TEXT,
    to_addr     TEXT,
    amount      TEXT NOT NULL,      -- decimal string, chain-native units
    token       TEXT NOT NULL,      -- "ETH", "USDC", "XMR", ...
    status      TEXT NOT NULL,      -- "pending" | "confirmed" | "failed"
    created_at  INTEGER NOT NULL,
    confirmed_at INTEGER,
    invoice_id  TEXT                -- links to a payment gate invoice
);

-- Payment invoices (seller side)
CREATE TABLE invoices (
    id          TEXT PRIMARY KEY,   -- uuid
    wallet_id   TEXT NOT NULL REFERENCES wallets(id),
    amount      TEXT NOT NULL,
    token       TEXT NOT NULL,
    status      TEXT NOT NULL,      -- "pending" | "paid" | "expired"
    peer_id     TEXT,               -- buyer's WG pubkey
    resource    TEXT,               -- e.g. "files::offering_id_here"
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL,
    paid_tx_id  TEXT                -- links to transactions.id once paid
);

-- Active subscriptions (buyer side -- tracks what I'm subscribed to)
CREATE TABLE subscriptions (
    id              TEXT PRIMARY KEY,    -- uuid
    peer_id         TEXT NOT NULL,       -- seller's WG pubkey
    peer_name       TEXT,                -- cached node name for display
    capability      TEXT NOT NULL,       -- "files", "tube", etc.
    amount          TEXT NOT NULL,       -- recurring amount
    token           TEXT NOT NULL,
    chain           TEXT NOT NULL,
    interval_secs   INTEGER NOT NULL,    -- subscription period in seconds
    status          TEXT NOT NULL,       -- "active" | "cancelled" | "expired"
    current_grant_expires INTEGER,       -- when current period ends
    last_payment_tx TEXT,                -- last transaction id
    created_at      INTEGER NOT NULL,
    cancelled_at    INTEGER              -- when user cancelled (NULL if active)
);

-- Payment receipts (both sides -- enriched view of transactions)
CREATE TABLE receipts (
    id              TEXT PRIMARY KEY,    -- uuid
    transaction_id  TEXT NOT NULL REFERENCES transactions(id),
    invoice_id      TEXT,                -- NULL for simple payments
    peer_id         TEXT NOT NULL,       -- the other party's WG pubkey
    peer_name       TEXT,                -- cached node name for display
    direction       TEXT NOT NULL,       -- "sent" | "received"
    amount          TEXT NOT NULL,
    token           TEXT NOT NULL,
    description     TEXT,                -- "File: photo-pack.zip", "Tube sub", "Direct payment"
    resource_type   TEXT,                -- "file" | "subscription" | "direct" | NULL
    resource_id     TEXT,                -- offering_id, capability name, etc.
    created_at      INTEGER NOT NULL
);
```

#### API Endpoints

All under `/cap/wallet/`.

| Method | Path                     | Description                           |
|--------|--------------------------|---------------------------------------|
| GET    | /wallets                 | List configured wallets               |
| POST   | /wallets                 | Create/import a wallet                |
| DELETE | /wallets/:id             | Remove a wallet                       |
| GET    | /wallets/:id/balance     | Current balance (may hit RPC)         |
| POST   | /wallets/:id/send        | Send payment                          |
| GET    | /transactions            | List transactions (filterable)        |
| GET    | /transactions/:id        | Transaction detail                    |
| POST   | /invoices                | Create a payment invoice              |
| GET    | /invoices/:id            | Invoice status                        |
| POST   | /invoices/:id/check      | Force-check if invoice is paid        |
| GET    | /invoices                    | List invoices                         |
| GET    | /subscriptions               | List my active/cancelled subscriptions |
| POST   | /subscriptions/:id/cancel    | Cancel a subscription                 |
| POST   | /subscriptions/:id/renew     | Manually renew an expired subscription |
| GET    | /receipts                    | Payment receipts (filter by peer, type, date) |
| GET    | /receipts/:id                | Single receipt detail                 |

#### Simple Peer-to-Peer Payments

The wallet works standalone for direct transfers — no invoices, no capabilities
involved. A user opens the wallet UI, enters a peer's address (or picks from
their peer list), and sends funds.

Flow:
1. Sender opens Wallet page, clicks "Send"
2. Picks recipient (peer list auto-populates addresses from peers who also
   have the wallet capability — discovered via P2PCD)
3. Enters amount, picks token
4. Wallet calls `POST /wallets/:id/send`
5. Transaction appears in both peers' transaction history

This is useful for tips, splitting costs, paying for services arranged
out-of-band, or anything that doesn't need the automated invoice flow.

The wallet UI also shows a **receive address** (QR code + copy button) so
peers can receive payments from outside the mesh too (e.g. from a regular
Ethereum wallet or exchange).

#### RPC Endpoints (capability-to-capability, internal)

| Method | Path                     | Description                           |
|--------|--------------------------|---------------------------------------|
| POST   | /rpc/create-invoice      | Files cap asks wallet to create invoice |
| POST   | /rpc/verify-payment      | Files cap asks wallet to verify payment |

These are called by other capabilities on localhost, not exposed to remote peers.

#### Chain Backend Trait

```rust
#[async_trait]
pub trait ChainBackend: Send + Sync {
    fn chain_name(&self) -> &str;

    async fn generate_keypair(&self, passphrase: &[u8])
        -> Result<(String, Vec<u8>)>;

    async fn import_keypair(&self, secret: &str, passphrase: &[u8])
        -> Result<(String, Vec<u8>)>;

    async fn get_balance(&self, address: &str)
        -> Result<Vec<TokenBalance>>;

    async fn send(
        &self,
        from_secret: &[u8],
        to_address: &str,
        amount: &str,
        token: &str,
    ) -> Result<String>;  // returns chain tx hash

    async fn check_payment(
        &self,
        address: &str,
        expected_amount: &str,
        token: &str,
        since_timestamp: u64,
    ) -> Result<Option<PaymentConfirmation>>;
}
```

First implementation: `EvmBackend` using the `alloy` crate, targeting Base
(chain ID 8453). RPC endpoint configurable, defaults to a public Base RPC.

Future backends:
- `MoneroBackend` — monero-wallet-rpc or native Rust (monero-rs)
- `LightningBackend` — LND/CLN gRPC
- `SolanaBackend` — solana-sdk

---

### 2. File-Level Pricing (files capability changes)

The existing `offerings` table gains optional price columns:

```sql
-- Schema migration v2
ALTER TABLE offerings ADD COLUMN price_amount      TEXT;     -- "0.001"
ALTER TABLE offerings ADD COLUMN price_token       TEXT;     -- "ETH"
ALTER TABLE offerings ADD COLUMN price_chain       TEXT;     -- "evm:8453"
ALTER TABLE offerings ADD COLUMN price_mode        TEXT;     -- "one_time" | "per_request" | "subscription"
ALTER TABLE offerings ADD COLUMN price_sub_seconds INTEGER;  -- for subscription mode
```

When all price columns are NULL, the file is free. When set, the file requires
payment. No separate pricing table — just fields on the offering.

The `Offering` struct gains:

```rust
pub struct Offering {
    // ... existing fields ...
    pub price: Option<OfferingPrice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfferingPrice {
    pub amount: String,
    pub token: String,
    pub chain: String,
    pub mode: PriceMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PriceMode {
    /// Pay once, access this specific resource forever.
    OneTime,
    /// Pay every time this resource is accessed.
    PerRequest,
    /// Pay once, access ALL subscription-tier resources in this capability
    /// for `seconds` duration. The grant is capability-wide, not per-resource.
    /// Example: a Tube capability charges 5 USDC/month — subscriber gets
    /// access to all subscriber-only videos for 30 days.
    Subscription { seconds: u64 },
}
```

#### The 402 Flow

When a remote peer requests a paid file:

```
GET /cap/files/offerings/:id/download
  → Files cap checks: offering.price is Some?
    → NO: serve file (existing flow, unchanged)
    → YES: check X-Invoice-Id header
      → Has valid paid invoice? → serve file
      → Missing or unpaid?
        → Call wallet cap: POST /rpc/create-invoice
        → Return 402 Payment Required + invoice JSON
```

The 402 response body:

```json
{
  "status": "payment_required",
  "invoice": {
    "invoice_id": "inv-uuid-here",
    "chain": "evm:8453",
    "address": "0xabc...",
    "amount": "0.001",
    "token": "ETH",
    "expiry": 1711660800
  },
  "offering": {
    "name": "photo-pack.zip",
    "size": 52428800,
    "mime_type": "application/zip"
  }
}
```

#### Payment Grant Cache (files-side)

The files capability caches payment grants locally to avoid re-verifying:

```rust
struct PaymentGrant {
    invoice_id: String,
    peer_id: String,
    /// For per-resource grants (OneTime, PerRequest): the specific offering_id.
    /// For subscription grants: None — the grant covers ALL subscription-tier
    /// resources in this capability.
    offering_id: Option<String>,
    capability_name: String,     // "files", "tube", etc.
    mode: PriceMode,
    granted_at: u64,
    expires_at: Option<u64>,     // None for OneTime
}
```

SQLite table in the capability's DB. For OneTime: scoped to a single resource,
no expiry. For PerRequest: scoped to a single resource, no caching. For
Subscription: scoped to the **entire capability** — one payment unlocks all
subscription-tier content for the duration. This is critical for use cases like
a Tube (video) capability where a subscriber should access all subscriber-only
videos, not pay per video.

#### Subscription Lifecycle

Subscriptions are tracked on the **buyer side** in the wallet capability's
`subscriptions` table. The seller side only knows about grants.

```
Subscribe:
  1. Buyer hits 402 on subscription content
  2. Payment dialog shows "Subscribe: 5 USDC / 30 days"
  3. Buyer pays, invoice marked paid
  4. Seller capability creates PaymentGrant (expires in 30 days)
  5. Buyer wallet creates subscription record (status: active)
  6. Receipt created for both sides

Auto-renew check (buyer-side, periodic):
  1. Wallet checks subscriptions nearing expiry (< 1 day remaining)
  2. For active (not cancelled) subscriptions:
     Notify user: "Subscription to X's Tube expires tomorrow"
  3. User can renew from subscriptions tab or let it lapse
  4. V1 is manual renewal only. Auto-pay is a future option.

Cancel:
  1. Buyer opens Subscriptions tab, clicks Cancel
  2. Confirmation: "You'll retain access until {date}"
  3. subscription.status = "cancelled", subscription.cancelled_at = now
  4. Access continues until current_grant_expires (already paid for)
  5. After expiry, seller capability sees no valid grant, returns 402
  6. No on-chain transaction needed -- cancellation is local bookkeeping

Renew (after expiry or cancellation):
  1. Buyer clicks "Renew" on expired/cancelled subscription
  2. Triggers fresh 402 flow: new invoice, pay, new grant period
  3. subscription.status = "active", new current_grant_expires
```

Note: V1 is manual renewal only. The buyer gets a notification before expiry
and chooses to renew or not. Automatic recurring payments can be added later --
the wallet would auto-create and pay an invoice when expiry approaches.

#### Catalogue Includes Prices

The RPC catalogue response (what remote peers see when browsing) already lists
offerings. It now includes the price:

```json
{
  "offerings": [
    {
      "offering_id": "abc-123",
      "name": "photo-pack.zip",
      "size": 52428800,
      "mime_type": "application/zip",
      "access": "public",
      "price": { "amount": "0.001", "token": "ETH", "chain": "evm:8453", "mode": "one_time" }
    },
    {
      "offering_id": "def-456",
      "name": "readme.txt",
      "size": 1024,
      "access": "public",
      "price": null
    }
  ]
}
```

#### Capability-to-Capability Communication

The files capability discovers the wallet capability's port via the daemon's
`GET /api/capabilities` endpoint (or an env var at startup), then makes
localhost HTTP calls:

```rust
async fn check_or_create_invoice(
    wallet_port: u16,
    offering: &Offering,
    peer_id: &str,
) -> Result<InvoiceResponse> {
    let client = reqwest::Client::new();
    client
        .post(format!("http://localhost:{}/rpc/create-invoice", wallet_port))
        .json(&CreateInvoiceRequest {
            amount: offering.price.as_ref().unwrap().amount.clone(),
            token: offering.price.as_ref().unwrap().token.clone(),
            chain: offering.price.as_ref().unwrap().chain.clone(),
            peer_id: peer_id.to_string(),
            resource: format!("files::{}", offering.offering_id),
        })
        .send()
        .await?
        .json()
        .await
}
```

---

### 3. UI Components

#### Wallet Page (`pages/WalletPage.tsx`)

Tabs: **Balance** | **Receipts** | **Subscriptions** | **Send**

**Balance tab:**
- List wallets with balances
- Create/import wallet flow
- Receive address with QR code + copy button

**Receipts tab (`components/ReceiptList.tsx`):**
- Chronological list of all payments sent and received
- Each receipt shows:
  - Peer name + avatar (from profile sync)
  - Direction indicator (sent/received)
  - Amount + token
  - Description ("File: photo-pack.zip", "Tube subscription", "Direct payment")
  - Timestamp
  - Chain tx link (opens block explorer)
- Filter by: peer, direction (sent/received), type (file/subscription/direct), date range
- Tap a receipt for full detail (tx hash, invoice ID, confirmations)
- Summary view: total sent/received per peer

**Subscriptions tab (`components/SubscriptionList.tsx`):**
- List of all subscriptions (active, cancelled, expired)
- Each entry shows:
  - Peer name (who you're subscribed to)
  - Capability name + icon ("Tube", "Files", etc.)
  - Amount + token + interval ("5 USDC / 30 days")
  - Status badge: active (green), expires soon (yellow), expired (red), cancelled (grey)
  - Time remaining on current period
- **Cancel button** on active subscriptions
  - Confirmation dialog: "Cancel subscription to {peer}'s {capability}? Access continues until {expiry_date}."
  - Sets cancelled_at, keeps grant until current_grant_expires
  - No refund -- access continues through paid period, just won't renew
- **Renew button** on expired/cancelled subscriptions
  - Re-triggers payment flow, creates new grant period

**Send tab:**
- Pick recipient from peer list (or enter address manually)
- Amount input, token picker
- Optional memo/description
- Confirm + send

#### Payment Dialog (`components/PaymentDialog.tsx`)
- Triggered automatically when any API call returns 402
- Displays: resource name, amount, token, chain, seller node name
- For subscriptions: shows interval ("5 USDC / 30 days") and what it unlocks
- "Pay" / "Subscribe" button, wallet sends and auto-retries the original request
- Receipt created automatically after payment confirms

#### Price Setting (on file upload/edit)
- Toggle "Set price" on any offering
- Amount input, token picker (ETH/USDC), mode selector
- Chain pre-filled from default wallet
- Price badge on file listings (amount + token)
- Subscription badge shows interval

#### Wallet FAB
- Manifest registers a FAB icon for quick access
- Balance badge overlay

---

### 4. Manifest

```json
{
  "name": "wallet",
  "version": "0.1.0",
  "description": "Crypto wallet and payment gate for peer-to-peer transactions",
  "binary": "howm-wallet",
  "port": null,
  "api": {
    "base_path": "/cap/wallet",
    "endpoints": [
      { "name": "list_wallets", "method": "GET", "path": "/wallets" },
      { "name": "create_wallet", "method": "POST", "path": "/wallets" },
      { "name": "get_balance", "method": "GET", "path": "/wallets/:id/balance" },
      { "name": "send_payment", "method": "POST", "path": "/wallets/:id/send" },
      { "name": "list_transactions", "method": "GET", "path": "/transactions" },
      { "name": "create_invoice", "method": "POST", "path": "/invoices" },
      { "name": "check_invoice", "method": "POST", "path": "/invoices/:id/check" }
    ]
  },
  "permissions": {
    "visibility": "private"
  },
  "ui": {
    "label": "Wallet",
    "icon": "wallet",
    "entry": "index.html",
    "style": "nav"
  }
}
```

---

## Security Considerations

### Private Key Storage
- Private keys encrypted at rest with ChaCha20-Poly1305
- Encryption key derived from user passphrase via Argon2id
- Keys decrypted only in memory, only for the duration of a transaction
- `secrets` table in a separate SQLite file with 0600 permissions

### Invoice Replay Protection
- Each invoice has a unique ID and expiry timestamp
- Invoices are single-use: once "paid", cannot be reused
- `peer_id` on invoice must match the requesting peer
- Payment verification checks on-chain state, not just the peer's claim

### Trust Model
- Wallet runs locally — keys never leave the node
- Payment verification done by seller checking on-chain state
- No escrow/arbitration (v1) — acceptable because:
  - Peers are in a WireGuard mesh (not anonymous)
  - Payment gate releases data automatically (no manual step)
  - Transaction history provides audit trail

### Denial of Service
- Invoice creation rate-limited per peer
- Expired invoices garbage-collected periodically
- Balance checks cached (don't hit RPC every request)

---

## Implementation Phases

### Phase 1: Wallet Capability (standalone)
- Scaffold capability binary (main.rs, db.rs, api.rs)
- `ChainBackend` trait + `EvmBackend` (Base L2, alloy crate)
- Key generation, encryption, storage
- Balance queries + send transactions
- Invoice creation and payment checking
- RPC endpoints (create-invoice, verify-payment)
- Unit tests for DB ops and EVM backend (mocked RPC)

### Phase 2: Files Capability Pricing
- Add price columns to offerings table (schema migration v2)
- `OfferingPrice` struct and serialization
- 402 flow in download endpoint
- Payment grant cache table
- Capability-to-capability RPC (files → wallet)
- Price fields in catalogue RPC response
- Integration tests: upload with price → attempt download → 402 → pay → download

### Phase 3: UI
- Wallet page (balances, history, send)
- Payment dialog (402 interception, auto-retry)
- Price editor on file upload/edit
- Price badges on file listings
- Wallet FAB with balance badge

### Phase 4: Additional Chains
- Monero backend (XMR — privacy-focused, no transparent ledger)
- Lightning backend (Bitcoin L2 — instant micropayments)
- Chain selector in wallet creation UI
- Multi-chain invoices (offer buyer choice of payment methods)

### Future: Tube Capability (video subscriptions)
A long-form video hosting capability that uses the wallet's subscription mode:
- Creator posts videos, marks some as "public" (free) and others as
  "subscriber-only" (priced with `Subscription { seconds: 2592000 }` = 30 days)
- Viewers browse the catalogue, see free and locked videos
- Clicking a locked video triggers the 402 flow → pay subscription → unlock
  ALL subscriber videos for 30 days (capability-wide grant)
- On expiry, viewer is prompted to renew
- This requires no wallet changes — just a new capability that calls the same
  `/rpc/create-invoice` and `/rpc/verify-payment` endpoints

---

## Dependencies (Rust crates)

### Wallet capability
- `alloy` — EVM interactions (transactions, ABI, RPC)
- `chacha20poly1305` — key encryption
- `argon2` — passphrase KDF
- `rusqlite` — database (same as other capabilities)
- `axum` + `tokio` — HTTP server (same as other capabilities)
- `serde` / `serde_json` — serialization

### Files capability additions
- `reqwest` — HTTP client for wallet RPC calls (already a dependency)

### Daemon additions
- None — daemon is not involved in payment flow

---

## Open Questions

1. **Confirmations threshold** — How many block confirmations before a payment
   is considered final? Base L2 has ~2s blocks. Suggest 3 confirmations (~6s)
   for the default, configurable per-wallet.

2. **Refunds** — V1 has no refund mechanism. Should we add a dispute flow in
   V2, or keep it simple and let peers resolve off-band?

3. **Gas estimation UI** — Should the wallet show estimated gas fees before
   sending, or just send and report the actual cost after?

4. **Multi-token support** — The EVM backend should support ETH + ERC-20
   tokens (USDC, DAI). Do we want a token allowlist or let users add any
   contract address?
