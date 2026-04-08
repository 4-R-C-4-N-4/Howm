# Wallet Capability — Development Tasks

Detailed task breakdown for each implementation phase. Tasks are ordered
by dependency — complete them top to bottom within each phase.

Reference: [DESIGN.md](./DESIGN.md) for architecture and data models.

**Environment:** Base Sepolia testnet (chain ID 84532) for all development.
Switch to mainnet Base (8453) for production via config.

---

## Phase 1: Wallet Capability (standalone binary)

The wallet runs as an independent capability process, same pattern as
`capabilities/files/`, `capabilities/messaging/`, etc.

### 1.1 Scaffold the crate

- [ ] Create `capabilities/wallet/` directory structure:
  ```
  capabilities/wallet/
  ├── Cargo.toml
  ├── manifest.json
  ├── src/
  │   ├── main.rs
  │   ├── db.rs
  │   ├── api.rs
  │   ├── chain/
  │   │   ├── mod.rs        (ChainBackend trait)
  │   │   └── evm.rs        (EvmBackend implementation)
  │   └── crypto.rs         (key encryption/decryption helpers)
  └── ui/                   (embedded UI, Phase 3)
  ```
- [ ] `Cargo.toml` with dependencies:
  - `axum`, `tokio`, `serde`, `serde_json`, `clap`, `tracing`,
    `tracing-subscriber`, `anyhow` (same as other caps)
  - `rusqlite` with `bundled` feature
  - `alloy` (EVM provider, signer, primitives)
  - `chacha20poly1305` for key encryption
  - `argon2` for passphrase KDF
  - `uuid` with `v7` + `serde` features
  - `reqwest` with `json` + `rustls-tls` features
  - `include_dir` for embedded UI
  - `hex` for address encoding
- [ ] `manifest.json` following the files capability pattern:
  - name: `"wallet"`
  - port: 7005 (or next available)
  - base_path: `/cap/wallet`
  - visibility: `"private"` (wallet is local-only, not exposed to peers)
  - UI entry: `/ui/`
- [ ] `main.rs`: clap Config struct, tracing init, DB open, axum Router,
  bind to `127.0.0.1:{port}`. Follow `capabilities/files/src/main.rs` pattern.
  Config fields: `port`, `data_dir`, `daemon_port`, `chain_rpc_url`
  (default: `https://sepolia.base.org`), `chain_id` (default: 84532)

### 1.2 Database layer (`db.rs`)

- [ ] Create `wallets` table (id, chain, label, address, created_at, is_default)
- [ ] Create `secrets` table (wallet_id, encrypted blob) — **separate SQLite
  file** (`secrets.db`) with 0600 permissions on Unix
- [ ] Create `transactions` table (id, wallet_id, direction, chain_tx_id,
  from_addr, to_addr, amount, token, status, created_at, confirmed_at,
  invoice_id)
- [ ] Create `invoices` table (id, wallet_id, amount, token, status, peer_id,
  resource, created_at, expires_at, paid_tx_id)
- [ ] Create `subscriptions` table (id, peer_id, peer_name, capability,
  amount, token, chain, interval_secs, status, current_grant_expires,
  last_payment_tx, created_at, cancelled_at)
- [ ] Create `receipts` table (id, transaction_id, invoice_id, peer_id,
  peer_name, direction, amount, token, description, resource_type,
  resource_id, created_at)
- [ ] CRUD functions for each table:
  - `insert_wallet()`, `list_wallets()`, `get_wallet()`, `delete_wallet()`,
    `set_default_wallet()`
  - `insert_secret()`, `get_secret()`, `delete_secret()`
  - `insert_transaction()`, `list_transactions()`, `get_transaction()`,
    `update_transaction_status()`
  - `insert_invoice()`, `get_invoice()`, `list_invoices()`,
    `update_invoice_status()`
  - `insert_subscription()`, `list_subscriptions()`, `get_subscription()`,
    `update_subscription_status()`, `cancel_subscription()`
  - `insert_receipt()`, `list_receipts()`, `get_receipt()`
- [ ] Schema versioning (same pattern as files: `schema_version` table)
- [ ] **Tests:** Unit tests for all CRUD operations (in-memory SQLite)

### 1.3 Key encryption (`crypto.rs`)

- [ ] `derive_key(passphrase: &[u8], salt: &[u8]) -> [u8; 32]`
  — Argon2id with sensible params (t=3, m=64MB, p=1)
- [ ] `encrypt_secret(plaintext: &[u8], passphrase: &[u8]) -> Vec<u8>`
  — generates random salt + nonce, prepends to ciphertext
  — output format: `[salt:16][nonce:12][ciphertext+tag]`
- [ ] `decrypt_secret(encrypted: &[u8], passphrase: &[u8]) -> Result<Vec<u8>>`
  — extracts salt + nonce, decrypts
- [ ] **Tests:** round-trip encrypt/decrypt, wrong passphrase returns error,
  corrupted data returns error

### 1.4 Chain backend trait (`chain/mod.rs`)

- [ ] Define `ChainBackend` trait:
  ```rust
  #[async_trait]
  pub trait ChainBackend: Send + Sync {
      fn chain_name(&self) -> &str;
      async fn generate_keypair(&self, passphrase: &[u8])
          -> Result<(String, Vec<u8>)>;  // (address, encrypted_secret)
      async fn import_keypair(&self, secret: &str, passphrase: &[u8])
          -> Result<(String, Vec<u8>)>;
      async fn get_balance(&self, address: &str)
          -> Result<Vec<TokenBalance>>;
      async fn send(&self, from_secret: &[u8], to_address: &str,
                    amount: &str, token: &str) -> Result<String>;
      async fn check_payment(&self, address: &str, expected_amount: &str,
                             token: &str, since_timestamp: u64)
          -> Result<Option<PaymentConfirmation>>;
  }
  ```
- [ ] Define shared types: `TokenBalance { token, amount, decimals }`,
  `PaymentConfirmation { tx_hash, amount, confirmed_at, block_number }`
- [ ] Backend registry: `fn get_backend(chain: &str) -> Option<Arc<dyn ChainBackend>>`
  — V1 only returns `EvmBackend`, but structured for future additions

### 1.5 EVM backend (`chain/evm.rs`)

- [ ] `EvmBackend` struct: holds `rpc_url`, `chain_id`, alloy provider
- [ ] `generate_keypair()`:
  - Generate random 32-byte private key using `rand`
  - Derive Ethereum address from public key
  - Encrypt private key with passphrase via `crypto.rs`
  - Return (address as `0x...` hex string, encrypted bytes)
- [ ] `import_keypair()`:
  - Accept hex private key or mnemonic seed phrase
  - Derive address, encrypt, return
- [ ] `get_balance()`:
  - Query ETH balance via `eth_getBalance` RPC
  - Query ERC-20 balances (USDC at minimum) via `balanceOf` call
  - Return vec of `TokenBalance`
  - Cache balances for 30s to avoid RPC spam
- [ ] `send()`:
  - Decrypt private key from encrypted bytes
  - For ETH: build + sign + send transaction via alloy
  - For ERC-20: encode `transfer(address,uint256)` call, build + sign + send
  - Return tx hash
  - Zero the decrypted key in memory after use
- [ ] `check_payment()`:
  - For ETH: scan recent blocks for incoming tx to address with matching amount
  - For ERC-20: check `Transfer` event logs for matching recipient + amount
  - Return confirmation details if found
  - Configurable confirmations threshold (default: 3 blocks)
- [ ] **Tests:**
  - Mock RPC responses (use alloy's test utilities or a mock HTTP server)
  - Test keypair generation produces valid Ethereum addresses
  - Test balance parsing (ETH + ERC-20)
  - Test transaction building + signing (verify against known test vectors)
  - Test payment confirmation detection with mock block/log data

### 1.6 API routes (`api.rs`)

- [ ] **Wallet management:**
  - `POST /wallets` — create new or import existing wallet
    - Body: `{ chain: "evm", label?: string, import_key?: string, passphrase: string }`
    - Calls chain backend `generate_keypair()` or `import_keypair()`
    - Stores wallet + encrypted secret in DB
    - Returns wallet (without secret)
  - `GET /wallets` — list all wallets (no secrets)
  - `GET /wallets/:id/balance` — calls chain backend `get_balance()`
  - `DELETE /wallets/:id` — removes wallet + secret from DB
  - `POST /wallets/:id/default` — set as default wallet

- [ ] **Transactions:**
  - `POST /wallets/:id/send` — send a payment
    - Body: `{ to_address: string, amount: string, token: string, passphrase: string, memo?: string }`
    - Decrypts key, calls backend `send()`, records transaction + receipt
    - Returns transaction with tx hash
  - `GET /transactions` — list transactions
    - Query params: `?direction=in|out&status=pending|confirmed|failed&limit=50&offset=0`
  - `GET /transactions/:id` — single transaction detail

- [ ] **Invoices:**
  - `POST /invoices` — create invoice (used by UI for manual invoicing)
    - Body: `{ amount, token, chain, peer_id?, resource?, expires_in_secs? }`
    - Uses default wallet's address as pay-to
    - Returns invoice with payment address
  - `GET /invoices` — list invoices
    - Query params: `?status=pending|paid|expired&limit=50&offset=0`
  - `GET /invoices/:id` — invoice detail
  - `POST /invoices/:id/check` — force re-check payment status
    - Calls backend `check_payment()`, updates invoice if paid

- [ ] **RPC endpoints (internal, for other capabilities):**
  - `POST /rpc/create-invoice` — called by files cap (or any paying cap)
    - Body: `{ amount, token, chain, peer_id, resource }`
    - Returns: `{ invoice_id, address, amount, token, chain, expiry }`
  - `POST /rpc/verify-payment` — called by files cap to check if paid
    - Body: `{ invoice_id }`
    - Returns: `{ paid: bool, tx_hash?: string }`

- [ ] **Subscriptions:**
  - `GET /subscriptions` — list my subscriptions
    - Query params: `?status=active|cancelled|expired`
  - `POST /subscriptions/:id/cancel` — set status to cancelled
  - `POST /subscriptions/:id/renew` — trigger re-subscription flow

- [ ] **Receipts:**
  - `GET /receipts` — list receipts
    - Query params: `?peer_id=X&direction=sent|received&resource_type=file|subscription|direct&from=timestamp&to=timestamp&limit=50&offset=0`
  - `GET /receipts/:id` — single receipt detail

- [ ] **Housekeeping:**
  - `GET /health` — health check endpoint
  - Background task: expire pending invoices past their `expires_at`
  - Background task: poll pending outbound transactions for confirmation
  - Background task: check subscription expiry, update status to "expired"

- [ ] **Tests:**
  - Integration tests with mock chain backend
  - Test wallet CRUD lifecycle
  - Test send flow (mock backend, verify DB records created)
  - Test invoice creation + verification flow
  - Test subscription cancel + renew
  - Test receipt creation for each payment type (direct, file, subscription)
  - Test invoice expiry background task
  - Test rate limiting on invoice creation

### 1.7 Build and install verification

- [ ] `cargo build -p wallet` compiles cleanly
- [ ] `cargo clippy -p wallet` — zero warnings
- [ ] `cargo test -p wallet` — all tests pass
- [ ] Manual test: install capability via daemon, verify it starts on
  configured port, health endpoint responds
- [ ] Manual test: create wallet on Base Sepolia, check balance (should be 0)
- [ ] Manual test: fund wallet from faucet, verify balance updates

---

## Phase 2: Files Capability Pricing

Wire the 402 payment flow into the existing files capability.

### 2.1 Schema migration

- [ ] Add schema version 2 migration to `capabilities/files/src/db.rs`:
  ```sql
  ALTER TABLE offerings ADD COLUMN price_amount TEXT;
  ALTER TABLE offerings ADD COLUMN price_token TEXT;
  ALTER TABLE offerings ADD COLUMN price_chain TEXT;
  ALTER TABLE offerings ADD COLUMN price_mode TEXT;
  ALTER TABLE offerings ADD COLUMN price_sub_seconds INTEGER;
  ```
- [ ] Add `payment_grants` table to files.db:
  ```sql
  CREATE TABLE payment_grants (
      id              TEXT PRIMARY KEY,
      invoice_id      TEXT NOT NULL,
      peer_id         TEXT NOT NULL,
      offering_id     TEXT,           -- NULL for capability-wide subscriptions
      capability_name TEXT NOT NULL,
      mode            TEXT NOT NULL,   -- "one_time" | "per_request" | "subscription"
      granted_at      INTEGER NOT NULL,
      expires_at      INTEGER          -- NULL for one_time
  );
  CREATE INDEX idx_grants_peer_cap ON payment_grants(peer_id, capability_name);
  CREATE INDEX idx_grants_peer_offering ON payment_grants(peer_id, offering_id);
  ```
- [ ] Update `Offering` struct with `price: Option<OfferingPrice>` field
- [ ] Update `row_to_offering()` to read price columns
- [ ] Update offering insert/update queries to write price columns
- [ ] DB functions: `insert_grant()`, `check_grant()`, `expire_grants()`
- [ ] **Tests:** migration from v1 to v2, price round-trip in offerings,
  grant insert + lookup + expiry

### 2.2 Wallet capability discovery

- [ ] Add `wallet_port: Option<u16>` to files `AppState`
- [ ] On startup, query daemon `GET /api/capabilities` to find wallet
  capability port. Also accept `WALLET_PORT` env var override.
- [ ] Helper: `discover_wallet_port(daemon_port: u16) -> Option<u16>`
- [ ] If wallet capability not found, pricing features are disabled
  (priced files return 503 Service Unavailable instead of 402)

### 2.3 Offering price management

- [ ] Update `POST /offerings` (create) to accept optional price fields:
  `{ ..., price_amount?, price_token?, price_chain?, price_mode?, price_sub_seconds? }`
- [ ] Update `PATCH /offerings/:id` to allow setting/clearing price
- [ ] Price validation: amount must be positive decimal string, token must be
  non-empty, chain must be valid format, mode must be valid enum value
- [ ] Catalogue RPC response (`peer_catalogue`) includes price in offering list
- [ ] **Tests:** create offering with price, update price, clear price,
  catalogue includes price, invalid price rejected

### 2.4 The 402 payment flow

- [ ] In download endpoint, before serving file:
  1. Check `offering.price` — if None, serve file (existing flow unchanged)
  2. If priced, check `X-Invoice-Id` request header
  3. If header present, call wallet RPC `POST /rpc/verify-payment`
     — if paid, create grant in `payment_grants`, serve file
     — if not paid, return 402 with existing invoice
  4. If no header, check `payment_grants` for existing valid grant
     — OneTime: check by (peer_id, offering_id) — no expiry
     — Subscription: check by (peer_id, capability_name) — check expiry
     — if valid grant exists, serve file
  5. No grant, no invoice: call wallet RPC `POST /rpc/create-invoice`,
     return 402 with invoice JSON
- [ ] 402 response body format:
  ```json
  {
    "status": "payment_required",
    "invoice": { "invoice_id", "chain", "address", "amount", "token", "expiry" },
    "offering": { "name", "size", "mime_type" }
  }
  ```
- [ ] Handle wallet capability being unavailable (503 response)
- [ ] **Tests:**
  - Free file: download works without payment (no regression)
  - Priced file, no payment: returns 402 with invoice
  - Priced file, paid invoice: returns file data
  - Priced file, unpaid invoice: returns 402 again
  - OneTime grant: second download works without re-payment
  - Subscription grant: works across multiple offerings
  - Subscription grant: expired grant returns 402
  - Wallet unavailable: returns 503

### 2.5 Grant housekeeping

- [ ] Background task in files capability: periodically expire subscription
  grants past their `expires_at`
- [ ] Garbage collect old one-time grants? (probably keep forever, they're small)
- [ ] Log grant creation and expiry for audit trail

### 2.6 End-to-end integration test

- [ ] Two-node test scenario (can be simulated with two capability instances):
  1. Node A: upload file with price (0.001 ETH, one_time)
  2. Node B: browse Node A's catalogue, sees price
  3. Node B: attempt download, receive 402 + invoice
  4. Node B: pay invoice (via wallet RPC, mocked chain)
  5. Node B: retry download with invoice_id, receive file
  6. Node B: retry again, should work (one_time grant cached)

---

## Phase 3: UI

### 3.1 Wallet page scaffold

- [ ] Create `capabilities/wallet/ui/` directory with Vite + React + TypeScript
  (same toolchain as files capability UI)
- [ ] Create `pages/WalletPage.tsx` with tab navigation:
  Balance | Receipts | Subscriptions | Send
- [ ] API client module (`api.ts`) with typed fetch wrappers for all
  wallet endpoints
- [ ] Shared types (`types.ts`): Wallet, Transaction, Invoice, Subscription,
  Receipt, OfferingPrice, etc.

### 3.2 Balance tab

- [ ] Wallet list component showing each wallet:
  - Chain icon, label, address (truncated with copy button)
  - Balance per token (ETH, USDC, etc.)
  - Default wallet indicator (star icon)
- [ ] "Create Wallet" button:
  - Modal: choose chain (EVM only for now, greyed-out Monero/Lightning)
  - Passphrase input (with confirmation)
  - Option to import existing key
  - Shows generated address on success
- [ ] "Receive" section:
  - QR code of default wallet address (use `qrcode.react` or similar)
  - Address text with copy button
- [ ] Delete wallet (with confirmation + passphrase to verify ownership)
- [ ] Auto-refresh balances on tab focus (with 30s cache)

### 3.3 Send tab

- [ ] Recipient input:
  - Dropdown of mesh peers who have wallet capability (fetched from
    daemon peer list + P2PCD capability info)
  - Shows peer name + address
  - Manual address input fallback
- [ ] Amount input with token selector (ETH / USDC dropdown)
- [ ] Optional memo/description field
- [ ] Gas estimate display (call backend for estimate before sending)
- [ ] Passphrase input for transaction signing
- [ ] Send button with confirmation dialog:
  "Send {amount} {token} to {peer_name}? Gas: ~{estimate}"
- [ ] Progress states: signing, broadcasting, pending confirmation, confirmed
- [ ] Success: show tx hash with block explorer link
- [ ] Error handling: insufficient funds, network error, wrong passphrase

### 3.4 Receipts tab

- [ ] Receipt list component with infinite scroll / pagination
- [ ] Each receipt row:
  - Peer avatar + name (or truncated address if no name)
  - Direction indicator: green arrow in (received), red arrow out (sent)
  - Amount + token
  - Description text ("File: photo-pack.zip", "Tube subscription",
    "Direct payment", etc.)
  - Relative timestamp ("2 hours ago")
  - Tap to expand: full tx hash (linked to block explorer), invoice ID,
    block confirmations, exact timestamp
- [ ] Filter bar:
  - Peer selector (dropdown of peers you've transacted with)
  - Direction toggle: All / Sent / Received
  - Type filter: All / Files / Subscriptions / Direct
  - Date range picker
- [ ] Summary stats at top: total sent, total received (in default token)
- [ ] Empty state: "No payments yet" with link to Send tab

### 3.5 Subscriptions tab

- [ ] Subscription list component
- [ ] Each subscription card:
  - Peer name + avatar
  - Capability icon + name ("Tube", "Files")
  - Price: "5 USDC / 30 days"
  - Status badge:
    - Active (green) with "X days remaining"
    - Expiring soon (yellow, < 3 days) with "Expires in X hours"
    - Expired (red) with "Expired X days ago"
    - Cancelled (grey) with "Cancelled, access until {date}"
  - Cancel button (active subs only):
    - Confirmation modal: "Cancel subscription to {peer}'s {cap}?
      You'll retain access until {expiry_date}. No refund for remaining time."
    - On confirm: `POST /subscriptions/:id/cancel`
    - Card updates to cancelled state
  - Renew button (expired/cancelled subs):
    - Triggers payment flow (same as initial subscribe)
    - On success: card updates to active with new expiry
- [ ] Empty state: "No subscriptions" with explanation text

### 3.6 Payment dialog (402 interception)

- [ ] Global component mounted at app root (or in the daemon's main UI)
- [ ] Intercepts 402 responses from any capability API call
- [ ] Dialog shows:
  - What you're paying for (file name + size, or "Subscription to X's Tube")
  - Seller node name
  - Amount + token + chain
  - For subscriptions: "5 USDC / 30 days — unlocks all subscriber content"
  - Wallet selector (if multiple wallets)
  - Passphrase input
- [ ] "Pay" / "Subscribe" button:
  1. Calls `POST /wallets/:id/send` to wallet capability
  2. Shows progress (signing, broadcasting, confirming)
  3. On confirmation, auto-retries the original request with `X-Invoice-Id`
  4. On success, closes dialog and completes the original action
- [ ] Error states: insufficient funds, network error, invoice expired
- [ ] "Cancel" button to dismiss without paying

### 3.7 Price editor on files UI

- [ ] Add price controls to the files capability's offering create/edit forms
- [ ] Toggle: "Free" / "Paid"
- [ ] When paid:
  - Amount input
  - Token selector (ETH / USDC)
  - Mode selector: One-time / Per request / Subscription
  - Subscription: interval input (days)
  - Chain auto-filled from default wallet
- [ ] Price badge component for file listings:
  - Free files: no badge (or subtle "Free" text)
  - Paid files: "{amount} {token}" badge
  - Subscription files: "{amount} {token}/mo" badge with lock icon
- [ ] Remote peer file browser: show prices on peer's catalogue
  - "Download" button for free files
  - "Buy ({amount} {token})" button for paid files
  - "Subscribe ({amount} {token}/mo)" button for subscription files

### 3.8 Wallet FAB

- [ ] Register wallet FAB in manifest UI config
- [ ] FAB icon: wallet/payment icon
- [ ] Badge overlay: primary wallet balance (truncated)
- [ ] Tap opens wallet page

### 3.9 Build integration

- [ ] `npm run build` in `capabilities/wallet/ui/` produces `dist/`
- [ ] `include_dir!` in wallet `main.rs` embeds UI assets
- [ ] Fallback route serves SPA `index.html` for client-side routing
- [ ] Verify UI loads via `http://localhost:{port}/ui/`

---

## Phase 4: Additional Chain Backends

### 4.1 Monero backend (`chain/monero.rs`)

- [ ] Research: choose between `monero-wallet-rpc` (requires running monerod +
  monero-wallet-rpc daemon) vs pure Rust (`monero-rs` / `cuprate` libs)
- [ ] Implement `ChainBackend` for Monero:
  - `generate_keypair()`: Monero spend key + view key pair
  - `import_keypair()`: from seed (25-word mnemonic)
  - `get_balance()`: query wallet balance (XMR only, no tokens)
  - `send()`: build + sign + submit Monero transaction
  - `check_payment()`: check for incoming tx by payment ID or integrated address
- [ ] Monero-specific considerations:
  - Longer confirmation times (~2 min per block, suggest 10 confirmations)
  - Payment ID or subaddress-per-invoice for payment identification
  - View key can be shared for payment verification without spend access
- [ ] Add `"monero"` to backend registry
- [ ] Tests with Monero stagenet (testnet equivalent)
- [ ] Config: `monero_rpc_url`, `monero_network` (stagenet/mainnet)

### 4.2 Lightning backend (`chain/lightning.rs`)

- [ ] Research: LND vs CLN vs LDK (embedded, no external daemon)
- [ ] Implement `ChainBackend` for Lightning:
  - `generate_keypair()`: Lightning node identity (or LDK wallet)
  - `get_balance()`: channel balance (sats)
  - `send()`: pay a BOLT11 invoice
  - `check_payment()`: check invoice status (paid/unpaid)
  - Note: Lightning invoices are native — maps cleanly to our invoice model
- [ ] Lightning-specific considerations:
  - Instant settlement (no confirmation wait)
  - Requires channel liquidity (inbound for receiving)
  - Invoice-native protocol — our invoice abstraction maps 1:1
  - Amounts in satoshis
- [ ] Config: LND/CLN connection params or LDK data dir

### 4.3 UI updates for multi-chain

- [ ] Wallet creation: chain selector (EVM / Monero / Lightning)
  - Each chain shows relevant info (EVM: address, Monero: address + view key,
    Lightning: node pubkey)
- [ ] Token selector adapts to chain (EVM: ETH/USDC/DAI, Monero: XMR,
  Lightning: BTC/sats)
- [ ] Price setting: chain selector alongside token
- [ ] Payment dialog: if seller accepts multiple chains, show options
- [ ] Receipts: chain icon per transaction

### 4.4 Multi-chain invoices

- [ ] Invoice creation accepts multiple `accepted_chains`:
  ```json
  {
    "accepted": [
      { "chain": "evm:8453", "address": "0x...", "token": "USDC", "amount": "5.00" },
      { "chain": "monero", "address": "4...", "token": "XMR", "amount": "0.03" }
    ]
  }
  ```
- [ ] 402 response includes all accepted payment methods
- [ ] Buyer's payment dialog shows options, buyer picks preferred chain
- [ ] Verification checks the chain that was actually used

---

## Testing Strategy (all phases)

### Unit tests (per module)
- [ ] `db.rs`: all CRUD operations, schema migrations, edge cases
- [ ] `crypto.rs`: encrypt/decrypt round-trip, wrong passphrase, corrupted data
- [ ] `chain/evm.rs`: keypair generation, tx building, payment detection (mocked RPC)
- [ ] `api.rs`: each endpoint with mock DB + mock chain backend

### Integration tests
- [ ] Wallet lifecycle: create wallet, check balance, send, verify receipt
- [ ] Invoice lifecycle: create, check, expire
- [ ] Subscription lifecycle: subscribe, access, cancel, expire, renew
- [ ] Files + wallet: upload priced file, 402 flow, pay, download
- [ ] Wallet unavailable: files cap handles gracefully (503)

### Testnet tests (manual, Base Sepolia)
- [ ] Create wallet, fund from faucet
- [ ] Send ETH between two wallets
- [ ] Send USDC between two wallets (need testnet USDC contract)
- [ ] Create invoice, pay it, verify confirmation
- [ ] Full file purchase flow between two nodes on testnet

### Security tests
- [ ] Private key never appears in logs (grep for hex patterns)
- [ ] Private key never appears in API responses
- [ ] Secrets.db has correct file permissions
- [ ] Invoice replay: same invoice_id cannot grant access twice
- [ ] Peer ID mismatch: invoice for peer A cannot be used by peer B
- [ ] Expired invoice: returns appropriate error, not access
- [ ] Amount mismatch: underpayment detected and rejected
