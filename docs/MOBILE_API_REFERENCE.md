# Kombat Backend — Mobile API Reference

Complete endpoint reference so the mobile frontend stays in sync with the
backend. For the automated outcome-resolution / agent flow, see the companion
doc [AGENT_OUTCOME_FE_GUIDE.md](./AGENT_OUTCOME_FE_GUIDE.md).

> All amounts are **micro-USDC** (6 decimals). `1_000_000` = 1 USDC.
> All timestamps are ISO-8601 UTC strings. All `id` fields are UUID strings
> unless noted as on-chain object ids.

---

## 0. Conventions

### Response envelope
Every endpoint wraps its payload:
```ts
interface ApiResponse<T> {
  success: boolean;
  data: T | null;
  error: string | null;
}
```
Always check `success` first. On error, `data` is null and `error` holds the
message. HTTP status mirrors the failure type (400 validation, 401 auth, 404
missing, 503 feature not configured, 500 server).

### Routes are mounted twice
Most user-facing routes exist both at the bare path and under `/api/...`
(e.g. `/home/:wallet` **and** `/api/home/:wallet`). **Prefer the `/api/`
prefix** everywhere for consistency.

### Auth
| Audience | Header | Notes |
|---|---|---|
| Authenticated user | `Authorization: Bearer <jwt>` | JWT obtained from `/api/auth/verify` |
| Admin | `x-admin-token: <token>` | Internal only |
| Agent (server-to-server) | `x-agent-token: <token>` | Not used by mobile |

Most read endpoints are keyed by a `:wallet` path param and don't strictly
require the JWT today, but send the `Authorization` header on all
authenticated-user calls so behavior stays correct as auth tightens.

---

## 1. Auth

### Verify Dynamic token → app JWT
```
POST /api/auth/verify
Body: { "dynamicToken": "<token from Dynamic SDK>" }
```
```ts
// data:
interface DynamicAuthResponse {
  user: UserRecord;
  accessToken: string;   // store and send as Bearer on subsequent calls
}
```
Call this right after the Dynamic SDK login. Persist `accessToken`.

---

## 2. User Profile

```ts
interface UserRecord {
  id: string;
  wallet_address: string;
  email: string | null;
  display_name: string | null;
  avatar_url: string | null;
  wins: number;
  losses: number;
  created_at: string;
  updated_at: string;
}
```

| Method | Path | Body / Query | Returns |
|---|---|---|---|
| GET | `/api/users/:wallet` | — | `UserRecord` |
| POST | `/api/users/:wallet` | `UpdateProfileRequest` | `UserRecord` |
| DELETE | `/api/users/:wallet` | — | `()` |
| GET | `/api/users/search` | `?q=` or `?username=` or `?display_name=` `&limit=` | `UserRecord[]` |
| GET | `/api/users/:wallet/stats` | — | `UserStats` |

```ts
interface UpdateProfileRequest {
  email?: string;
  display_name?: string;
  avatar_url?: string;
}
interface UserStats {
  live_count: number;
  completed_count: number;
  total_stake: number;   // micro-USDC
  total_won: number;     // micro-USDC
}
```

### Home summary (dashboard landing)
```
GET /api/home/:wallet
```
```ts
interface HomeSummaryResponse {
  // flattened UserStats:
  live_count: number;
  completed_count: number;
  total_stake: number;
  total_won: number;
  live_kombats: WagerDetailResponse[];
  history_kombats: WagerDetailResponse[];
}
```

---

## 3. Notifications

```ts
interface NotificationRecord {
  id: string;
  user_wallet: string;
  kind: string;
  payload: unknown | null;
  is_read: boolean;
  created_at: string;
}
```

| Method | Path | Body / Query | Returns |
|---|---|---|---|
| GET | `/api/notifications/:wallet` | `?limit=&offset=` | `NotificationRecord[]` |
| POST | `/api/notifications/:id/read` | — | marks read |
| GET | `/notifications/stream/:wallet` | — | **SSE** stream |
| GET | `/ws/notifications/:wallet` | — | **WebSocket** stream |
| POST | `/api/users/:wallet/push-token` | `{ "expo_token": "..." }` | registers Expo push |

### Notification settings
```
GET /api/users/:wallet/notification-settings   → NotificationSettings
PUT /api/users/:wallet/notification-settings   → NotificationSettings
```
```ts
interface NotificationSettings {
  user_wallet: string;
  challenges: boolean;
  funds: boolean;
  disputes: boolean;
  marketing: boolean;
}
// PUT body: any subset of the booleans (UpdateNotificationSettings)
```

### Real-time options
- **WebSocket:** `wss://<host>/ws/notifications/:wallet` — preferred for mobile.
- **SSE:** `GET /notifications/stream/:wallet` — fallback.

Both push a `NotificationRecord` on each new notification. Use one, not both.

---

## 4. Sui / Wallet

### Config & health
| Method | Path | Returns |
|---|---|---|
| GET | `/api/sui/config` | `SuiAppConfigResponse` |
| GET | `/api/sui/health` | `SuiHealthResponse` |
| GET | `/api/sui/networks/:network/config` | `SuiConfigResponse` |
| GET | `/api/sui/networks/:network/health` | `SuiHealthResponse` |

```ts
interface SuiConfigResponse {
  network: string;
  rpc_url: string;
  package_id: string | null;       // staking package
  wager_package_id: string | null; // separate wager package
  usdc_coin_type: string | null;
  staking_module: string;
  wager_module: string;
}
interface SuiAppConfigResponse {
  active_network: string;
  networks: SuiConfigResponse[];
}
interface SuiHealthResponse extends SuiConfigResponse {
  chain_identifier: string;
  reference_gas_price: unknown;
}
```

### Balances & dashboard
| Method | Path | Returns |
|---|---|---|
| GET | `/api/sui/wallets/:wallet/balances` | `SuiBalance[]` |
| GET | `/api/sui/wallets/:wallet/usdc-balance` | `SuiCoinBalance` |
| GET | `/api/sui/wallets/:wallet/dashboard` | `WalletDashboardResponse` |
| GET | `/api/sui/networks/:network/wallets/:wallet/balances` | `SuiBalance[]` |
| GET | `/api/sui/networks/:network/wallets/:wallet/usdc-balance` | `SuiCoinBalance` |
| GET | `/api/sui/networks/:network/wallets/:wallet/dashboard` | `WalletDashboardResponse` |

```ts
interface SuiCoinBalance {       // serialized camelCase
  coinType: string;
  coinObjectCount: number;
  totalBalance: string;          // raw base units as string
  lockedBalance: unknown;
}
interface WalletDashboardResponse {
  network: string;
  wallet: string;
  usdc_coin_type: string | null;
  available_balance_usdc: number;
  locked_in_kombats_usdc: number;
  total_balance_usdc: number;
  transaction_history: WalletTransactionItem[];
  actions: {
    fund_wallet: WalletAction;
    withdraw: WalletAction;
  };
}
interface WalletAction { enabled: boolean; provider: string; requires_frontend_wallet: boolean; }
interface WalletTransactionItem {
  id: string;
  kind: string;
  title: string;
  subtitle: string | null;
  amount_usdc: number;
  direction: string;   // "in" | "out"
  status: string;
  tx_hash: string | null;
  occurred_at: string;
}
```
The **dashboard** is the one-stop screen for the wallet tab: balance breakdown,
fund/withdraw availability, and recent transactions. Dashboard accepts
`?limit=&offset=` for transaction paging.

---

## 5. Funding / On-ramp

### Generic ramp layer (preferred)
```
GET  /api/ramps/providers          ?country=US     → RampProvidersResponse
POST /api/ramps/session            RampSessionRequest → RampSessionResponse
```
```ts
interface RampSessionRequest {
  wallet_address: string;
  product?: string;            // "BUY"
  fiat_currency?: string;      // default USD
  fiat_amount?: string;        // decimal as string
  crypto_currency_code?: string; // default USDC
  crypto_amount?: string;
  network?: string;            // default sui
}
interface RampSessionResponse {
  provider: string;
  product: string;
  wallet_address: string;
  launch_method: string;       // how the client should launch the ramp
  client_action: string;
  network: string;
  crypto_currency_code: string;
  fiat_currency: string;
  fiat_amount: string | null;
  crypto_amount: string | null;
  note: string;
}
```

### Transak fallback
```
GET  /api/transak/config       → provider config
POST /api/transak/widget-url   TransakWidgetRequest → { provider, product, wallet_address, widget_url }
POST /api/transak/quote        TransakQuoteRequest  → { raw: <transak quote JSON> }
```
Open `widget_url` in an in-app browser/WebView for the Transak flow.

---

## 6. Tournaments & Matches (Pool Staking)

A "tournament" route here is the match-betting surface. A match has 2 opponents;
users stake USDC on an opponent; the pool determines odds.

```ts
interface MatchOpponent {
  id: string;
  match_id: string;
  pandascore_id: number;
  opponent_type: string;     // "Team" | "Player"
  name: string;
  acronym: string | null;
  image_url: string | null;
  location: string | null;
  position: number;
  is_winner: boolean | null;
  created_at: string;
}
interface OpponentWithPool extends MatchOpponent {
  pool_usdc: number;
  pool_percentage: number;
  odds: number;
  staker_count: number;
}
interface MatchWithOdds {
  // flattened MatchRecord (see below) plus:
  opponents: OpponentWithPool[];
  total_pool_usdc: number;
  total_stakers: number;
}
```

`MatchRecord` key fields the mobile UI uses: `id`, `name`, `videogame_name`,
`league_name`, `scheduled_at`, `begin_at`, `end_at`, `status`
(`upcoming|live|completed|cancelled`), `pandascore_status`, `winner_id`,
`result_status`, `verification_status`, `streams_list`, `sui_pool_object_id`.

### List & detail
| Method | Path | Query | Returns |
|---|---|---|---|
| GET | `/api/tournaments` | `?status=&videogame=&league_id=&search=&limit=&offset=` | `MatchWithOdds[]` |
| GET | `/api/tournaments/:id` | — | `MatchWithOdds` |
| GET | `/api/tournaments/source/pandascore` | — | provider source config |

`status` filter values: `upcoming`, `live`, `completed`, `cancelled`.

### Staking
```
POST /api/tournaments/:id/stake
Body (PlaceStakeRequest):
{
  "user_wallet": "0x...",
  "opponent_id": "<opponent UUID>",
  "amount_usdc": 1000000
}
```
Returns the created stake. **Note:** placing a stake involves an on-chain USDC
transaction — most mobile flows use the **payment intent** flow (§7) instead of
calling this directly, because that builds the Sui transaction for you.

### Payout preview (before staking)
```
POST /api/tournaments/:id/calculate
Body: { "opponent_id": "<uuid>", "amount_usdc": 1000000 }
```
```ts
interface PayoutCalculation {
  stake_amount_usdc: number;
  current_odds: number;
  min_payout_usdc: number;
  min_profit_usdc: number;
  profit_percentage: number;
  warning: string | null;
}
```

### Stakes on a match
```
GET /api/tournaments/:id/stakes      → PoolStakeRecord[]
```

### Admin-only (mobile typically doesn't call)
- `POST /api/tournaments/:id/resolve` — settle a match
- `POST /api/tournaments/:id/cancel` — cancel & refund
- `POST /api/tournaments/:id/sync` — re-sync from PandaScore

---

## 7. Payment Intents (the recommended staking flow)

This is how the mobile app should place a stake: create an intent, check funding,
optionally on-ramp, then fetch the PTB to sign.

```ts
interface PaymentIntentRecord {
  id: string;
  user_wallet: string;
  kind: string;
  status: string;
  network: string;
  match_id: string;
  opponent_id: string;
  amount_usdc: number;
  reserve_balance_usdc: number;
  settlement_rule: string;          // currently "return_to_wallet"
  current_balance_usdc: number | null;
  funding_shortfall_usdc: number;
  stake_tx_hash: string | null;
  stake_receipt_id: string | null;
  metadata: unknown;
  expires_at: string;
  created_at: string;
  updated_at: string;
}
```

### 1) Create intent
```
POST /api/payments/intents
Body (CreatePaymentIntentRequest):
{
  "wallet_address": "0x...",
  "match_id": "<uuid>",
  "opponent_id": "<uuid>",
  "amount_usdc": 1000000,
  "network": "sui",                 // optional
  "reserve_balance_usdc": 0,        // optional
  "settlement_rule": "return_to_wallet" // optional, only supported value
}
```
```ts
interface PaymentIntentResponse {
  intent: PaymentIntentRecord;
  funding: {
    current_balance_usdc: number;
    required_balance_usdc: number;
    funding_shortfall_usdc: number;
    onramp_required: boolean;
  };
  rules: { rule_type: string; amount_usdc: number; description: string }[];
  match_name: string;
  opponent_name: string;
}
```

### 2) Get intent (poll status / refresh funding)
```
GET /api/payments/intents/:id      → PaymentIntentResponse
```

### 3) If funding short, start an on-ramp session
```
POST /api/payments/intents/:id/onramp-session
```
```ts
interface PaymentIntentOnrampResponse {
  intent: PaymentIntentResponse;
  onramp_required: boolean;
  ramp_session: RampSessionResponse | null;
}
```

### 4) Get the Sui transaction to sign (PTB)
```
GET /api/payments/intents/:id/ptb
```
```ts
interface PaymentIntentPtbResponse {
  intent_id: string;
  network: string;
  can_build: boolean;               // false → read `reason`
  reason: string | null;
  coin_type: string | null;
  amount_usdc: number;
  reserve_balance_usdc: number;
  expected_receipt_type: string;
  steps: { kind: string; description: string }[];
  move_call: PaymentMoveCall | null;
}
interface PaymentMoveCall {
  target: string;
  package_id: string;
  module: string;
  function: string;
  type_arguments: string[];
  arguments: { name: string; kind: string; value: unknown | null; source: string }[];
}
```
Build the Sui transaction from `move_call`, have the user sign it via the Dynamic
wallet, then submit on-chain. The backend reconciles the stake from the receipt.

**Flow summary:** create → (poll funding / on-ramp) → ptb → sign → done.

---

## 8. Stake Receipt Secondary Market

Users can list their stake receipt (an on-chain object) for resale.

```ts
interface ReceiptMarketListing {
  id: string;
  network: string;
  seller_wallet: string;
  buyer_wallet: string | null;
  receipt_id: string;
  listing_object_id: string | null;
  match_id: string;
  opponent_id: string;
  ask_amount_usdc: number;
  status: string;          // draft | active | sold | ...
  listing_tx_hash: string | null;
  sale_tx_hash: string | null;
  metadata: unknown;
  expires_at: string;
  created_at: string;
  updated_at: string;
}
interface ReceiptListingResponse {
  listing: ReceiptMarketListing;
  match_name: string;
  opponent_name: string;
}
```

| Method | Path | Body / Query | Returns |
|---|---|---|---|
| GET | `/api/receipt-market/listings` | `?match_id=&seller_wallet=&status=&limit=&offset=` | `ReceiptListingResponse[]` |
| POST | `/api/receipt-market/listings` | `CreateReceiptListingRequest` | `ReceiptListingResponse` |
| GET | `/api/receipt-market/listings/:id` | — | `ReceiptListingResponse` |
| GET | `/api/receipt-market/listings/:id/list-ptb` | — | `ReceiptMarketPtbResponse` (build tx to list) |
| POST | `/api/receipt-market/listings/:id/activate` | `ActivateReceiptListingRequest` | confirms on-chain listing |
| GET | `/api/receipt-market/listings/:id/buy-ptb` | — | `ReceiptMarketPtbResponse` (build tx to buy) |
| POST | `/api/receipt-market/listings/:id/mark-sold` | `MarkReceiptListingSoldRequest` | confirms sale |

```ts
interface CreateReceiptListingRequest {
  wallet_address: string;
  receipt_id: string;
  match_id: string;
  opponent_id: string;
  ask_amount_usdc: number;
  network?: string;
  expires_at?: string;
}
interface ActivateReceiptListingRequest { wallet_address: string; listing_object_id: string; listing_tx_hash?: string; }
interface MarkReceiptListingSoldRequest { buyer_wallet: string; sale_tx_hash?: string; }
```
List/buy follow the same **get-PTB → sign → confirm** pattern as payment intents.

---

## 9. User Stakes (portfolio)

```
GET /api/users/:wallet/stakes        ?status=&match_id=&limit=&offset=  → StakeWithMatch[]
GET /api/users/:wallet/stake-stats   → UserStakeStats
```
```ts
interface StakeWithMatch {
  // flattened PoolStakeRecord:
  id: string;
  match_id: string;
  opponent_id: string;
  user_wallet: string;
  amount_usdc: number;
  odds_at_stake: string | null;
  status: string;           // active | won | lost | refunded
  payout_usdc: number | null;
  stake_tx_hash: string | null;
  payout_tx_hash: string | null;
  stake_receipt_id: string | null;
  created_at: string;
  resolved_at: string | null;
  // joined match info:
  match_name: string;
  match_status: string;
  opponent_name: string;
  opponent_image_url: string | null;
  videogame_name: string | null;
  scheduled_at: string | null;
}
interface UserStakeStats {
  active_stakes: number;
  total_staked_usdc: number;
  total_won_usdc: number;
  total_lost_usdc: number;
  win_count: number;
  loss_count: number;
}
```
`status` filter for the stakes list: `active`, `won`, `lost`, `refunded`.

---

## 9b. P2P Wagers (1-v-1)

A direct head-to-head bet between two wallets, separate from the pool-staking
tournament flow. The backend **builds the on-chain transactions** (PTBs) for
create/accept/resolve, **indexes** the wager, and owns the off-chain social layer
(accept, declared winners, disputes, win/loss stats). It can also store the wager
**terms** and dispute **evidence** durably on Walrus.

> The PTB endpoints require `SUI_<NET>_PACKAGE_ID`, `usdc_coin_type`, and
> `SUI_WAGER_MODULE` to be configured. If they aren't, the PTB response returns
> `can_build: false` with a `reason` (e.g. `wager_package_not_configured`), and
> `move_call` is null — surface the reason and disable the action.

```ts
interface WagerRecord {
  id: string;
  on_chain_address: string;     // the wager object's address — the path key
  wager_id: number;
  initiator: string;
  challenger: string | null;
  stake_usdc: number;
  description: string;
  status: string;               // open | active | resolved | cancelled | disputed | declined | expired
  resolution_source: string;
  resolver: string;
  expiry_ts: number;            // unix seconds
  created_at: string;
  resolved_at: string | null;
  winner: string | null;
  protocol_fee_bps: number;
  oracle_feed: string | null;
  oracle_target: number | null;
  dispute_opened_at: string | null;
  dispute_opener: string | null;
  initiator_option: string | null;
  creator_declared_winner: string | null;
  challenger_declared_winner: string | null;
}
interface WagerDetailResponse extends WagerRecord {
  initiator_name: string | null;
  initiator_avatar: string | null;
  challenger_name: string | null;
  challenger_avatar: string | null;
  challenger_option: string | null;
  opponent_wallet: string | null;
  opponent_name: string | null;
  opponent_avatar: string | null;
}
```

### Endpoints
| Method | Path | Body / Query | Returns |
|---|---|---|---|
| POST | `/api/wagers` | `CreateWagerRequest` | `WagerDetailResponse` |
| GET | `/api/wagers` | `?initiator=&challenger=&status=&limit=&offset=` | `WagerDetailResponse[]` |
| GET | `/api/wagers/mine` | `?wallet=` (required) `&limit=&offset=` | `WagerDetailResponse[]` |
| GET | `/api/wagers/:address` | — | `WagerDetailResponse` |
| POST | `/api/wagers/:address/accept` | `{ "challenger": "0x..." }` | `WagerDetailResponse` |
| POST | `/api/wagers/:address/status` | `{ "status": "cancelled" }` | `WagerDetailResponse` |
| POST | `/api/wagers/:address/declare-winner` | `ConsentRequest` | `DeclareWinnerResponse` |
| GET | `/api/wagers/:address/disputes` | — | `DisputeSubmission[]` |
| POST | `/api/wagers/:address/disputes` | `DisputeSubmissionRequest` | `DisputeSubmission` |
| POST | `/api/wagers/create-ptb` | `WagerCreatePtbRequest` | `WagerPtbResponse` |
| GET | `/api/wagers/:address/accept-ptb` | — | `WagerPtbResponse` |
| GET | `/api/wagers/:address/resolve-ptb` | `?winner=0x..` | `WagerPtbResponse` |
| GET | `/api/wagers/:address/artifacts` | — | `WalrusArtifact[]` (terms + evidence) |

`:address` is the wager's `on_chain_address`.

### Transaction building (PTBs)
The PTB endpoints describe the Sui Move call to run; the mobile app builds and
signs the transaction from this with the Dynamic wallet.
```ts
interface WagerCreatePtbRequest {
  initiator: string;
  stake_usdc: number;
  description: string;
  expiry_ts: number;          // unix seconds
  resolver: string;
  network?: string;           // defaults to active network
  challenger_address?: string;
  initiator_option?: string;
}
interface WagerPtbResponse {
  wager_address: string | null;   // null for create (object doesn't exist yet)
  network: string;
  can_build: boolean;             // false → read reason, move_call is null
  reason: string | null;
  coin_type: string | null;
  package_id: string | null;
  expected_object_type: string;   // "Wager"
  steps: { kind: string; description: string }[];
  move_call: {
    target: string;               // e.g. "<pkg>::wager::create_wager"
    package_id: string;
    module: string;
    function: string;
    type_arguments: string[];
    arguments: { name: string; kind: string; value: unknown | null; source: string }[];
  } | null;
}
```
**On-chain flow:**
1. `POST /api/wagers/create-ptb` → build/sign/submit → get the new object address.
2. `POST /api/wagers` to index it (§ above).
3. Challenger: `GET /:address/accept-ptb` → sign → then `POST /:address/accept`.
4. Resolve: `GET /:address/resolve-ptb?winner=` → sign → backend reflects result.

### Walrus terms & evidence
- **Terms:** pass an optional `terms` JSON object in `CreateWagerRequest`. It's
  uploaded to Walrus and indexed as a `wager_terms` artifact.
- **Evidence:** pass an optional `evidence` JSON object in
  `DisputeSubmissionRequest`. It's uploaded to Walrus, indexed as a
  `wager_evidence` artifact, and the resulting aggregator URL is saved as the
  dispute's `evidence_url`.
- **Fetch:** `GET /api/wagers/:address/artifacts` returns all `WalrusArtifact`
  rows for the wager (see §11 for the shape). Read each artifact's
  `aggregator_url` to render the stored JSON.

Both are best-effort: if Walrus isn't configured the wager/dispute still saves,
just without a stored blob.

```ts
interface CreateWagerRequest {
  on_chain_address: string;     // from the on-chain create tx
  wager_id: number;
  initiator: string;
  stake_usdc: number;
  description: string;
  expiry_ts: number;            // unix seconds
  resolution_source: string;
  resolver: string;
  challenger_address?: string;  // if targeting a specific opponent
  initiator_option?: string;
  protocol_fee_bps?: number;
  oracle_feed?: string;
  oracle_target?: number;
  terms?: object;               // optional — stored durably on Walrus
}
// status POST — only: open | active | cancelled | declined | expired
interface ConsentRequest { participant: string; declared_winner: string; }
interface DeclareWinnerResponse {
  resolved_winner: string | null;     // set when both sides agreed → off-chain resolved
  onchain_resolve_tx: string | null;  // tx digest if the backend also paid out on-chain
  wager: WagerDetailResponse;
}
interface DisputeSubmissionRequest {
  submitter: string;
  description: string;
  evidence_url?: string;        // used as-is if `evidence` not provided
  declared_winner?: string;
  evidence?: object;            // optional — uploaded to Walrus; its URL becomes evidence_url
}
interface DisputeSubmission {
  id: string;
  wager_address: string;
  submitter: string;
  description: string;
  evidence_url: string | null;
  declared_winner: string | null;
  created_at: string;
  updated_at: string;
}
```

### Lifecycle
1. **Create on-chain** (client) → **POST `/api/wagers`** to index it (`open`, or
   `active` if a challenger is named).
2. Opponent **accepts** → `POST /accept` (sets challenger + `active`).
3. After the real-world result, **each side declares the winner** via
   `/declare-winner`. When both declare the **same** wallet, the wager
   auto-resolves (`status = resolved`, `winner` set, win/loss stats recorded) and
   `resolved_winner` comes back non-null. If the wager's on-chain `resolver` is
   the platform signer (and it holds gas), the backend **also pays out on-chain**
   automatically and returns `onchain_resolve_tx`. Otherwise settle on-chain with
   `GET /:address/resolve-ptb` signed by the resolver.
4. If they **disagree**, either side files a dispute via `POST /disputes`
   (marks the wager `disputed`); resolve out-of-band, then settle on-chain.

> Win/loss on `UserRecord` (`wins`/`losses`) is incremented automatically on
> auto-resolution — the profile and `/stats` reflect wager outcomes.

---

## 10. File Upload

```
POST /api/uploads
Content-Type: multipart/form-data   (file field)
→ { "url": "<public file url>" }
```
Use for avatars and any image the user provides. Returns a URL you can then save
to the profile via `POST /api/users/:wallet`.

---

## 11. Walrus (durable evidence storage)

Mobile typically only reads evidence; see the agent guide. Quick reference:

| Method | Path | Returns |
|---|---|---|
| GET | `/api/walrus/config` | `{ enabled, configured, network, aggregator_url, max_upload_bytes }` |
| GET | `/api/walrus/artifacts/:id` | `WalrusArtifactRecord` |
| GET | `/api/walrus/blobs/:blob_id/url` | `{ blob_id, url }` |

---

## 12. Health

```
GET /health    → { status, service, version, sui, transak, ramps, walrus }
```
Use for a connectivity/config check on app boot.

---

## Appendix — common status enums

| Field | Values |
|---|---|
| match `status` | `upcoming`, `live`, `completed`, `cancelled` |
| match `result_status` | `pending`, `proposed`, `approved`, `rejected`, `disputed` |
| stake `status` | `active`, `won`, `lost`, `refunded` |
| wager `status` | `open`, `active`, `resolved`, `disputed`, `cancelled`, `declined`, `expired` |
| proposal `status` | `pending`, `pending_review`, `auto_verified`, `approved`, `rejected`, `disputed` |
| listing `status` | `draft`, `active`, `sold` |
| payment intent `status` | `pending`, funded/settled states (poll via GET) |

---

## Appendix — recommended screen → endpoint map

| Screen | Calls |
|---|---|
| Login | `POST /api/auth/verify` |
| Home | `GET /api/home/:wallet` |
| Wallet | `GET /api/sui/wallets/:wallet/dashboard`, ramp/transak for funding |
| Matches list | `GET /api/tournaments?status=` |
| Match detail | `GET /api/tournaments/:id`, `POST /api/tournaments/:id/calculate` |
| Place stake | payment-intent flow (§7) |
| My stakes | `GET /api/users/:wallet/stakes`, `/stake-stats` |
| Resell receipt | receipt-market flow (§8) |
| P2P wager (create/accept/resolve) | wager flow (§9b) |
| Notifications | `GET /api/notifications/:wallet` + WS stream |
| Profile | `GET/POST /api/users/:wallet`, `POST /api/uploads` |
