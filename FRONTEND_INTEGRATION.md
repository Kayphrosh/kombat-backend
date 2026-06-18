# Kombat Frontend Integration Guide

Last updated: 2026-06-16

This backend is now Sui-first. The frontend should use Dynamic embedded wallets for auth, Sui wallet access, transaction signing, and Dynamic Native Onramp. There is no Solana RPC/auth flow left in the product path.

## 1. Base Contract

All API responses use the same envelope:

```ts
type ApiResponse<T> = {
  success: boolean;
  data: T | null;
  error: string | null;
};
```

Recommended FE helpers:

```ts
const API_BASE_URL = process.env.NEXT_PUBLIC_API_BASE_URL!;

async function api<T>(path: string, init: RequestInit = {}) {
  const res = await fetch(`${API_BASE_URL}${path}`, {
    ...init,
    headers: {
      "Content-Type": "application/json",
      ...(init.headers ?? {}),
    },
  });
  const json = (await res.json()) as ApiResponse<T>;
  if (!res.ok || !json.success) throw new Error(json.error ?? "Request failed");
  return json.data as T;
}

const usdcToMicro = (amount: number) => Math.round(amount * 1_000_000);
const microToUsdc = (amount: number | string) => Number(amount) / 1_000_000;
```

Conventions:

- `amount_usdc`, `stake_usdc`, `ask_amount_usdc`, balances, and shortfalls are integer micro-USDC.
- Timestamps are ISO strings unless a field is named `*_ts` or `*_ms`; `expiry_ts` in P2P wagers is Unix milliseconds.
- Wallets are Sui addresses. Normalize and store lowercase `0x...` addresses from the backend response.
- Network values are `testnet` and `mainnet`.
- User-protected routes require `Authorization: Bearer <appAccessToken>`.
- Admin, agent, and webhook secrets must never be shipped to the public/mobile frontend.

## 2. Environment Needed By FE

```env
NEXT_PUBLIC_API_BASE_URL=https://your-backend.example.com
NEXT_PUBLIC_DYNAMIC_ENVIRONMENT_ID=...
NEXT_PUBLIC_DEFAULT_SUI_NETWORK=testnet
```

The FE does not need Sui package IDs in env. Fetch them from:

```http
GET /api/sui/config
GET /api/sui/networks/testnet/config
GET /api/sui/networks/mainnet/config
```

Current testnet package metadata is also committed in `sui/Published.toml`. Mainnet can stay disabled/missing until deployed; the FE must respect `can_build: false` responses.

## 3. Auth With Dynamic

Flow:

1. User logs in through Dynamic embedded wallet.
2. FE gets Dynamic auth token/JWT from the Dynamic SDK.
3. FE exchanges it for Kombat's app JWT.
4. Use the returned `accessToken` on protected routes.

```http
POST /api/auth/verify
Content-Type: application/json

{
  "dynamicToken": "<dynamic sdk token>"
}
```

Response:

```ts
type DynamicAuthResponse = {
  user: {
    id: string;
    wallet_address: string;
    email?: string | null;
    display_name?: string | null;
    avatar_url?: string | null;
    wins: number;
    losses: number;
  };
  accessToken: string;
};
```

Store the app JWT securely in memory/secure storage. Refresh it by rerunning `/api/auth/verify` after Dynamic session restore.

## 4. App Bootstrap

On app launch after auth:

```http
GET /api/sui/config
GET /api/walrus/config
GET /api/ramps/providers?country=NG
GET /api/users/:wallet
GET /api/sui/wallets/:wallet/dashboard?limit=20
GET /notifications/:wallet
GET /api/tournaments?status=upcoming&limit=20
```

Use `/api/home/:wallet` if the FE wants a compact home/profile payload. Use `/health` for uptime and `/metrics` only for ops.

Profile and search endpoints:

```http
GET /api/users/search?q=ola&limit=20
GET /api/users/:wallet
POST /api/users/:wallet
Authorization: Bearer <appAccessToken>
{
  "email": "user@example.com",
  "display_name": "Kay",
  "avatar_url": "https://..."
}

DELETE /api/users/:wallet
Authorization: Bearer <appAccessToken>

GET /api/users/:wallet/stats
GET /api/home/:wallet
```

Only allow profile mutation/delete in the UI when the connected Dynamic wallet matches `:wallet`.

## 5. Wallet Screen

The wallet screen in the mockup maps directly to:

```http
GET /api/sui/wallets/:wallet/dashboard?limit=20
GET /api/sui/networks/:network/wallets/:wallet/dashboard?limit=20
```

Key response fields:

```ts
type WalletDashboardResponse = {
  network: string;
  wallet: string;
  usdc_coin_type?: string | null;
  available_balance_usdc: number;
  locked_in_kombats_usdc: number;
  total_balance_usdc: number;
  transaction_history: Array<{
    id: string;
    kind: string;
    title: string;
    subtitle?: string | null;
    amount_usdc: number;
    direction: "in" | "out";
    created_at: string;
    metadata?: unknown;
  }>;
  actions: {
    fund_wallet: {
      enabled: boolean;
      provider: string;
      requires_frontend_wallet: boolean;
    };
    withdraw: {
      enabled: boolean;
      provider: string;
      requires_frontend_wallet: boolean;
    };
  };
};
```

Display:

- Available balance: `available_balance_usdc`.
- Locked in Kombats: `locked_in_kombats_usdc`.
- Live tournament stakes: `/api/home/:wallet` includes `active_stakes` alongside `live_kombats`.
- History rows: `transaction_history`.
- Fund button: call Dynamic Native Onramp through `/api/ramps/session`.
- Withdraw button: currently should be hidden/disabled unless backend returns `actions.withdraw.enabled`.

Raw balances are also available:

```http
GET /api/sui/wallets/:wallet/balances
GET /api/sui/wallets/:wallet/usdc-balance
GET /api/sui/networks/:network/wallets/:wallet/balances
GET /api/sui/networks/:network/wallets/:wallet/usdc-balance
```

## 6. Funding / On-Ramp

We are using Dynamic Native Onramp from custom UI. The backend only tells the FE what to launch; Dynamic handles provider availability in the Dynamic dashboard.

```http
GET /api/ramps/providers?country=US
```

```http
POST /api/ramps/session
Authorization: Bearer <appAccessToken>

{
  "wallet_address": "0x...",
  "product": "BUY",
  "fiat_currency": "USD",
  "fiat_amount": 50,
  "crypto_currency_code": "USDC",
  "network": "sui"
}
```

Response:

```ts
type RampSessionResponse = {
  provider: "dynamic_native";
  product: "BUY";
  wallet_address: string;
  launch_method: "dynamic_sdk";
  client_action: "open_dynamic_onramp";
  network: string;
  crypto_currency_code: string;
  fiat_currency: string;
  fiat_amount?: number;
  crypto_amount?: number;
  note: string;
};
```

FE behavior:

1. Render your own Fund Wallet modal.
2. Call `/api/ramps/session`.
3. If `client_action === "open_dynamic_onramp"`, open Dynamic's native funding UI from the SDK with wallet, network, token, and fiat amount.
4. Poll `/api/sui/wallets/:wallet/dashboard` after the onramp closes.

Legacy Transak endpoints still exist as fallback/config surfaces, but do not use them for the main flow:

```http
GET /api/transak/config
POST /api/transak/widget-url
POST /api/transak/quote
```

## 7. Tournament Feed And Detail

List and filter tournaments:

```http
GET /api/tournaments?status=upcoming&videogame=codm&pool_configured=true&search=final&limit=20&offset=0
GET /api/tournaments/:id
```

The `MatchWithOdds` response contains:

- `match_info`: tournament/match metadata, status, timing, Sui pool object ID.
- `pool_configured`: `true` only when `match_info.sui_pool_object_id` is present.
- `pool_object_id` / `sui_pool_object_id`: top-level aliases for the configured pool object.
- `opponents`: exactly two sides for stakeable binary markets.
- pool/odds data used for display and payout estimates.

Only show on-chain stake entry points for matches where `pool_configured === true`.

PandaScore source metadata:

```http
GET /api/tournaments/source/pandascore
```

Do not call PandaScore directly from the FE. The API key lives only on the backend.

## 8. Smart Pay + Stake Flow

Use payment intents for the cleanest "fund if needed, then stake" flow.

Create intent:

```http
POST /api/payments/intents
Authorization: Bearer <appAccessToken>

{
  "wallet_address": "0x...",
  "match_id": "<uuid>",
  "opponent_id": "<uuid>",
  "amount_usdc": 25000000,
  "network": "testnet",
  "reserve_balance_usdc": 1000000,
  "settlement_rule": "return_to_wallet"
}
```

Response:

```ts
type PaymentIntentResponse = {
  intent: {
    id: string;
    user_wallet: string;
    network: string;
    match_id: string;
    opponent_id: string;
    amount_usdc: number;
    reserve_balance_usdc: number;
    funding_shortfall_usdc: number;
    status: "requires_funding" | "ready_to_stake" | string;
  };
  funding: {
    current_balance_usdc: number;
    required_balance_usdc: number;
    funding_shortfall_usdc: number;
    onramp_required: boolean;
  };
  rules: Array<{
    rule_type: string;
    amount_usdc: number;
    description: string;
  }>;
  match_name: string;
  opponent_name: string;
  pool_configured: boolean;
  pool_object_id?: string;
};
```

If funding is required:

```http
POST /api/payments/intents/:id/onramp-session
Authorization: Bearer <appAccessToken>
```

Then open Dynamic Native Onramp and poll:

```http
GET /api/payments/intents/:id
Authorization: Bearer <appAccessToken>
```

When `funding.onramp_required === false`, fetch the PTB plan:

```http
GET /api/payments/intents/:id/ptb
Authorization: Bearer <appAccessToken>
```

The response is not a serialized transaction. It is a build plan:

```ts
type PaymentIntentPtbResponse = {
  intent_id: string;
  network: "testnet" | "mainnet";
  can_build: boolean;
  reason?: string | null;
  coin_type?: string | null;
  amount_usdc: number;
  reserve_balance_usdc: number;
  expected_receipt_type: "StakeReceipt";
  steps: Array<{ kind: string; description: string }>;
  move_call?: {
    target: string;
    package_id: string;
    module: string;
    function: string;
    type_arguments: string[];
    arguments: Array<{
      name: string;
      kind: "shared_object" | "owned_object" | "coin" | "u8" | "u64" | "address" | "string" | string;
      value?: unknown;
      source: string;
    }>;
  } | null;
};
```

FE transaction signing responsibilities:

1. Use Dynamic's embedded Sui wallet.
2. Build a Sui `Transaction`.
3. Select and split the user's USDC coin for `amount_usdc`.
4. Pass shared objects such as pool and `0x6` Clock as objects.
5. Call the returned `move_call.target`.
6. Sign and execute through Dynamic/Sui wallet.
7. Refresh wallet dashboard and tournament detail.

Minimal pseudo-code:

```ts
import { Transaction } from "@mysten/sui/transactions";

async function executeMovePlan(plan: PaymentIntentPtbResponse, signer: any) {
  if (!plan.can_build || !plan.move_call) throw new Error(plan.reason ?? "PTB unavailable");

  const tx = new Transaction();
  const call = plan.move_call;

  // FE must implement coin selection. Merge/split USDC coins if needed.
  const paymentCoin = await splitUsdcCoin(tx, {
    owner: await signer.getAddress(),
    coinType: plan.coin_type!,
    amount: plan.amount_usdc,
  });

  tx.moveCall({
    target: call.target,
    typeArguments: call.type_arguments,
    arguments: [
      tx.object(String(call.arguments.find((a) => a.name === "pool")?.value)),
      tx.pure.u8(Number(call.arguments.find((a) => a.name === "outcome")?.value)),
      paymentCoin,
      tx.object("0x6"),
    ],
  });

  return signer.signAndExecuteTransaction({ transaction: tx });
}
```

## 9. Direct Stake Endpoints

These are simpler DB/payment calculation endpoints. Prefer Smart Pay for user-facing staking because it handles funding shortfalls.

```http
POST /api/tournaments/:id/calculate
{
  "opponent_id": "<uuid>",
  "amount_usdc": 25000000
}
```

```http
POST /api/tournaments/:id/stake
Authorization: Bearer <appAccessToken>
{
  "user_wallet": "0x...",
  "opponent_id": "<uuid>",
  "amount_usdc": 25000000
}
```

User portfolio:

```http
GET /api/users/:wallet/stakes?status=active&limit=20&offset=0
GET /api/users/:wallet/stake-stats
GET /api/tournaments/:id/stakes
```

Stake rows include `stake_receipt_id` when the backend knows the minted `StakeReceipt` object ID. Use that value as `receipt_id` when creating a receipt-market listing. If it is null for an older stake, the FE may need to recover the object ID from the original stake transaction's `objectChanges`.

## 10. Trading In/Out Before Settlement

Users can sell a `StakeReceipt` through the receipt market. This is the "trade out before settlement" feature.

Create a draft listing:

```http
POST /api/receipt-market/listings
Authorization: Bearer <appAccessToken>

{
  "wallet_address": "0xSeller",
  "receipt_id": "0xStakeReceiptObject",
  "match_id": "<uuid>",
  "opponent_id": "<uuid>",
  "ask_amount_usdc": 18000000,
  "network": "testnet",
  "expires_at": "2026-06-16T22:00:00Z"
}
```

Escrow/list receipt on-chain:

```http
GET /api/receipt-market/listings/:id/list-ptb
Authorization: Bearer <appAccessToken>
```

The response uses the same generic PTB plan shape as wagers and payment intents: `can_build`, optional `reason`, `coin_type`, `package_id`, `steps`, and `move_call`. `move_call.function` is `list_receipt`.

Receipt listing responses include the canonical nested shape `{ listing, match_name, opponent_name }` and top-level aliases for common listing fields such as `id`, `wallet_address`, `seller_wallet`, `receipt_id`, `match_id`, `opponent_id`, `ask_amount_usdc`, and `status`.

After successful on-chain listing, activate the backend record:

```http
POST /api/receipt-market/listings/:id/activate
Authorization: Bearer <appAccessToken>

{
  "wallet_address": "0xSeller",
  "listing_object_id": "0xReceiptListingObject",
  "listing_tx_hash": "<txDigest>"
}
```

Buy flow:

```http
GET /api/receipt-market/listings?match_id=<uuid>&status=active
GET /api/receipt-market/listings/:id/buy-ptb
Authorization: Bearer <appAccessToken>
```

The buy PTB response uses the same generic PTB plan shape. `move_call.function` is `buy_receipt`; `can_build=false` with `reason="listing_not_active"` when the listing cannot be purchased.

After buyer signs the PTB:

```http
POST /api/receipt-market/listings/:id/mark-sold
Authorization: Bearer <appAccessToken>

{
  "buyer_wallet": "0xBuyer",
  "sale_tx_hash": "<txDigest>"
}
```

## 11. P2P Wagers

P2P wagers use the Sui wager package for escrow and the backend for social indexing, disputes, notifications, and Walrus evidence.

Create on-chain PTB plan:

```http
POST /api/wagers/create-ptb
{
  "initiator": "0x...",
  "stake_usdc": 10000000,
  "description": "Fnatic beats FaZe",
  "expiry_ts": 1780000000000,
  "resolver": "0xResolver",
  "network": "testnet",
  "challenger_address": "0xOptionalSpecificChallenger",
  "initiator_option": "Fnatic"
}
```

After the user signs and the FE extracts the created `Wager` object ID:

```http
POST /api/wagers
{
  "on_chain_address": "0xWagerObject",
  "wager_id": 123,
  "initiator": "0xInitiator",
  "stake_usdc": 10000000,
  "description": "Fnatic beats FaZe",
  "expiry_ts": 1780000000000,
  "resolution_source": "mutual_consent",
  "resolver": "0xResolver",
  "challenger_address": null,
  "initiator_option": "Fnatic",
  "terms": {
    "game": "CODM",
    "rules": "Best of 5"
  }
}
```

List and detail:

```http
GET /api/wagers?status=open&limit=20&offset=0
GET /api/wagers/mine?wallet=0x...&limit=20&offset=0
GET /api/wagers/:address
```

Wager lifecycle status values currently returned by the backend are:

- `open`: created on-chain/indexed, not accepted yet.
- `active`: accepted by a challenger.
- `resolved`: both parties agreed on a winner or an admin/status path marked it resolved.
- `disputed`: at least one dispute submission exists.
- `cancelled`: open wager was cancelled by the initiator after on-chain refund.
- `declined`: named challenger declined the social invite; initiator still needs the cancel flow to reclaim escrow.
- `expired`: legacy/manual status for open wagers past expiry; use the cancel flow for refund if the on-chain object is still `STATUS_OPEN`.

New Sui rows use Sui `0x...` object/wallet addresses and Unix milliseconds in `expiry_ts`. Legacy rows may still contain Solana/base58 addresses and Unix seconds. Every wager detail/list row includes derived compatibility fields:

```ts
{
  expiry_ts: number;       // original stored value
  expiry_ms: number;       // normalized Unix milliseconds for display
  expiry_unit: "seconds" | "milliseconds";
  address_format: "sui" | "legacy";
  is_legacy: boolean;
}
```

The FE should use `expiry_ms` for display and branch on `is_legacy`/`address_format` before attempting Sui address validation or Sui PTB construction.

Accept:

```http
GET /api/wagers/:address/accept-ptb
POST /api/wagers/:address/accept
{ "challenger": "0xChallenger" }
```

Cancel/refund an unaccepted wager:

```http
GET /api/wagers/:address/cancel-ptb
POST /api/wagers/:address/cancel
{ "initiator": "0xInitiator" }
```

`cancel-ptb` calls `wager::cancel_wager`; the signer must be the initiator and the on-chain wager must still be open. `POST /cancel` only updates the backend record after the FE has submitted the signed transaction.

Decline a named social invite:

```http
POST /api/wagers/:address/decline
{ "challenger": "0xChallenger" }
```

Decline does not refund escrow. It only records the named challenger's rejection; the initiator must still use `cancel-ptb` and `/cancel`.

Mutual result declaration:

```http
POST /api/wagers/:address/declare-winner
{
  "participant": "0xParticipant",
  "declared_winner": "0xWinner"
}
```

When both participants declare the same winner, backend resolves the social record and attempts on-chain resolution if the configured resolver signer can do it. The response includes `onchain_resolve_tx` on success or `onchain_resolve_error` on failure. Wager detail/list rows persist the latest `resolution_error` and `resolution_attempted_at` so polling clients can surface resolver/RPC failure and direct the user to admin/manual escalation or `resolve-ptb`.

Disputes and evidence:

```http
POST /api/wagers/:address/disputes
{
  "submitter": "0x...",
  "description": "Opponent did not follow rules",
  "declared_winner": "0x...",
  "evidence": {
    "screenshots": ["..."],
    "notes": "..."
  }
}

GET /api/wagers/:address/disputes
GET /api/wagers/:address/artifacts
```

## 12. Organizer Dashboard

The organizer dashboard unlocks community/CODM tournaments that are not available on PandaScore.

Apply:

```http
POST /api/organizers/apply
{
  "wallet_address": "0x...",
  "organization_name": "Lagos Mobile League",
  "contact_email": "ops@example.com",
  "website_url": "https://example.com",
  "country": "NG",
  "description": "CODM community organizer",
  "metadata": {
    "discord": "...",
    "twitter": "..."
  }
}
```

Create KYC session:

```http
POST /api/organizers/kyc-session
Authorization: Bearer <appAccessToken>
{
  "wallet_address": "0x...",
  "provider": "manual",
  "return_url": "https://app.example.com/organizer"
}
```

Fetch organizer profile:

```http
GET /api/organizers/:wallet
Authorization: Bearer <appAccessToken>
```

Create organizer tournament:

```http
POST /api/organizer/tournaments
Authorization: Bearer <appAccessToken>
{
  "organizer_wallet": "0x...",
  "name": "CODM Friday Clash",
  "videogame_name": "Call of Duty Mobile",
  "videogame_slug": "codm",
  "description": "Community tournament",
  "rules_blob_id": "walrusBlobId",
  "bracket_blob_id": "walrusBlobId",
  "starts_at": "2026-06-20T18:00:00Z",
  "ends_at": "2026-06-20T22:00:00Z",
  "metadata": {
    "region": "Africa",
    "format": "single_elimination"
  }
}
```

List organizer tournaments:

```http
GET /api/organizer/tournaments?organizer_wallet=0x...&status=upcoming&videogame=codm
```

Create match under organizer tournament:

```http
POST /api/organizer/tournaments/:id/matches
Authorization: Bearer <appAccessToken>
{
  "organizer_wallet": "0x...",
  "name": "Team Alpha vs Team Beta",
  "scheduled_at": "2026-06-20T19:00:00Z",
  "match_type": "best_of",
  "number_of_games": 5,
  "rules_blob_id": "walrusBlobId",
  "bracket_blob_id": "walrusBlobId",
  "streams_list": [{ "raw_url": "https://youtube.com/..." }],
  "opponents": [
    {
      "name": "Team Alpha",
      "opponent_type": "team",
      "image_url": "https://..."
    },
    {
      "name": "Team Beta",
      "opponent_type": "team",
      "image_url": "https://..."
    }
  ],
  "metadata": {
    "round": "semi-final"
  }
}
```

Required organizer details in UI:

- Organization name.
- Wallet address from Dynamic.
- Contact email.
- Country/region.
- Tournament name.
- Game name and slug.
- Tournament start/end time.
- Match schedule.
- Two opponents per stakeable match.
- Team names and optional team logos.
- Rules, bracket, and evidence files when available.
- Stream links.
- Outcome source policy: organizer, agent, PandaScore, or admin review.

Upload team logos/files first with `/api/uploads`, then pass the returned URL as `image_url` or store larger artifacts on Walrus.

## 13. Uploads

For normal images/files that should be served by this backend:

```http
POST /api/uploads
Content-Type: multipart/form-data

file=<binary>
category=team_logo
```

The response includes a public URL under `/uploads/...` when static uploads are enabled.

Use this for team logos and lightweight public images. Use Walrus for durable rules, brackets, evidence, and agent artifacts.

## 14. Walrus Artifacts

Config:

```http
GET /api/walrus/config
```

Create artifact:

```http
POST /api/walrus/artifacts
Authorization: Bearer <appAccessToken>

{
  "artifact_type": "tournament_rules",
  "owner_wallet": "0x...",
  "match_id": "<uuid>",
  "content_type": "application/json",
  "manifest": {
    "title": "CODM Friday Clash Rules",
    "rules": ["Best of 5", "No emulator"]
  },
  "metadata": {
    "source": "organizer_dashboard"
  }
}
```

Admin and agent services may also upload artifacts with their own server-only tokens.

Fetch artifact and blob URL:

```http
GET /api/walrus/artifacts/:id
GET /api/walrus/blobs/:blob_id/url
```

Privacy note: do not put private KYC documents, private user PII, or secret organizer credentials on public Walrus. Store public/verifiable artifacts there: rules, brackets, match evidence, agent reports, and dispute evidence intended for review.

## 15. Outcome Proposals And Agents

Community/organizer outcome proposal:

```http
POST /api/tournaments/:id/outcome-proposals
Authorization: Bearer <appAccessToken>
{
  "proposer_wallet": "0x...",
  "proposed_winner_opponent_id": "<uuid>",
  "proposed_winner_name": "Team Alpha",
  "source": "organizer",
  "confidence": 0.95,
  "evidence_blob_id": "walrusBlobId",
  "evidence_url": "https://...",
  "evidence_summary": "Bracket screenshot and stream VOD confirm winner",
  "raw_data": {}
}
```

List proposals for a tournament:

```http
GET /api/tournaments/:id/outcome-proposals
```

Agent ingestion is server-to-server only:

```http
POST /api/agents/outcome-proposals
X-Agent-Token: <server-only>
```

Admin agent run visibility:

```http
GET /api/admin/agent-runs
GET /api/admin/agent-runs/:id
```

The public FE should display approved/visible proposal status but must not include `AGENT_API_TOKEN`.

## 16. Notifications

List notifications:

```http
GET /notifications/:wallet?limit=50&offset=0
Authorization: Bearer <appAccessToken>
```

Mark read:

```http
POST /notifications/:id/read
Authorization: Bearer <appAccessToken>
```

SSE stream:

```http
GET /notifications/stream/:wallet
Authorization: Bearer <appAccessToken>
```

Use a fetch-based SSE client or an EventSource polyfill that supports headers. Native browser `EventSource` cannot attach `Authorization`.

WebSocket alias:

```http
GET /ws/notifications/:wallet
Authorization: Bearer <appAccessToken>
```

Push token:

```http
POST /users/:wallet/push-token
Authorization: Bearer <appAccessToken>
{
  "expo_token": "ExponentPushToken[...]"
}
```

Settings:

```http
GET /users/:wallet/notification-settings
PUT /users/:wallet/notification-settings
Authorization: Bearer <appAccessToken>
{
  "challenges": true,
  "funds": true,
  "disputes": true,
  "marketing": false
}
```

Notification payloads may include an action:

```ts
type NotificationAction = {
  label: string;
  action: string;
  method: "GET" | "POST";
  endpoint: string;
  params?: Record<string, unknown>;
};
```

Known actions:

- `open_onramp`: call the endpoint, then open Dynamic onramp.
- `open_stake_confirmation`: fetch PTB and show confirmation.
- `open_receipt_listing`: navigate to listing detail.
- `open_tournament`: navigate to tournament detail.
- `open_list_receipt`: fetch list PTB and show listing confirmation.

## 17. Admin Dashboard

Admin routes require `X-Admin-Token` or `Authorization: Bearer <admin token>`. Do not put this token in a public/mobile FE. Use a protected internal admin app only.

Organizer review:

```http
GET /api/admin/organizers?status=pending&kyc_status=pending&country=NG&search=league
GET /api/admin/organizers/:wallet

POST /api/organizers/:wallet/review
{
  "status": "approved",
  "kyc_status": "approved",
  "reviewed_by": "admin@example.com"
}
```

Outcome review:

```http
GET /api/admin/outcome-proposals?status=pending&source=organizer
GET /api/admin/outcome-proposals/:id

POST /api/outcome-proposals/:id/review
{
  "reviewer_wallet": "0xAdmin",
  "decision": "approve"
}
```

PandaScore sync:

```http
POST /api/tournaments/source/pandascore/sync
X-Admin-Token: <server-only>
{
  "statuses": ["not_started", "running", "finished"],
  "videogame_slugs": ["valorant", "codm"],
  "max_pages": 2,
  "per_page": 50
}
```

Resolve/cancel/sync:

```http
POST /api/tournaments/:id/resolve
POST /api/tournaments/:id/cancel
POST /api/tournaments/:id/sync
```

## 18. Webhooks

These are backend/server-only. The frontend should not call them.

```http
POST /api/webhooks/match-result
X-Webhook-Signature: hex(HMAC-SHA256(WEBHOOK_SECRET, raw_body))

POST /api/webhooks/pandascore
X-PandaScore-Token: <server-only>
```

## 19. Transaction Builder Notes For Sui

Backend PTB endpoints return `move_call` metadata, not prebuilt bytes. This is intentional because the user's Dynamic embedded wallet owns signing.

The FE must implement:

- Sui network selection matching `plan.network`.
- USDC coin lookup by `plan.coin_type`.
- Merge/split coins when the user has multiple USDC coin objects.
- Shared object arguments from `move_call.arguments`.
- Dynamic wallet signing/execution.
- Post-transaction refresh and backend activation/indexing calls where required.

Important object kinds:

- `coin`: split exact micro-USDC from user's USDC coins.
- `shared_object`: pass `tx.object(objectId)`.
- `owned_object`: pass a user-owned object, such as `StakeReceipt`.
- `u8`, `u64`, `address`, `string`: pass with `tx.pure.*`.
- `clock`: always Sui Clock object `0x6`.

If `can_build === false`, do not show a sign button. Show the `reason` and fallback action:

- `intent_requires_funding`: open onramp.
- `staking_package_not_configured`: network is not deployed/configured yet.
- `usdc_coin_type_not_configured`: token config missing.
- `pool_object_not_configured`: market has no Sui pool object yet.
- `listing_not_active` or `listing_expired`: refresh listing/feed.

After the platform creates a `tournament_staking::create_pool` object on-chain, the backend/indexer must register the shared pool object before users can stake:

```http
POST /api/admin/tournaments/:id/pool
X-Admin-Token: <AUTH_ADMIN_TOKEN>

{
  "sui_network": "testnet",
  "sui_pool_object_id": "0x..."
}
```

The backend can also create and register missing pools in one admin backfill call when these env vars are configured:

- `PLATFORM_SIGNER_KEYPAIR`: funded Sui signer that owns the staking `AdminCap`.
- `SUI_PACKAGE_ID` or `SUI_TESTNET_PACKAGE_ID`: package containing `tournament_staking`.
- `SUI_ADMIN_CAP_OBJECT_ID` or `SUI_TESTNET_ADMIN_CAP_OBJECT_ID`: staking `AdminCap` object.
- `SUI_USDC_COIN_TYPE` or `SUI_TESTNET_USDC_COIN_TYPE`: coin type used by pools.

```http
POST /api/admin/tournaments/pools/backfill
X-Admin-Token: <AUTH_ADMIN_TOKEN>

{
  "sui_network": "testnet",
  "limit": 25,
  "match_ids": ["<optional-match-uuid>"],
  "default_stake_window_hours": 72
}
```

The response lists each attempted match, the created `pool_object_id`, transaction digest, or the per-match failure reason.

PandaScore sync/backfill payloads may also include `sui_network` and `sui_pool_object_id`; ordinary syncs preserve any existing pool object when those fields are omitted.

## 20. Security Checklist For FE

- Never expose `AUTH_ADMIN_TOKEN`, `AGENT_API_TOKEN`, `WEBHOOK_SECRET`, `PANDASCORE_API_KEY`, private Sui keys, or database URLs.
- Always compare authenticated wallet with the connected Dynamic wallet before rendering protected user actions.
- Only call user-protected routes with the app JWT from `/api/auth/verify`.
- Do not let users submit arbitrary `owner_wallet` on Walrus uploads unless it matches the connected wallet.
- Do not show admin/agent endpoints in the consumer app.
- Treat all uploaded URLs and metadata as untrusted display content.
- Do not assume onramp completion; always refetch Sui balance.
- Do not assume transaction success from wallet UI; wait for executed digest and refresh backend state.

## 21. Suggested FE Route Map

- `/login`: Dynamic login, exchange token.
- `/home`: tournaments, active stakes, notifications.
- `/wallet`: dashboard, fund wallet, history.
- `/tournaments`: filterable feed.
- `/tournaments/:id`: odds, calculate, Smart Pay stake, receipt listings.
- `/market`: receipt marketplace.
- `/wagers`: P2P open/mine.
- `/wagers/:address`: accept, declare winner, dispute.
- `/organizer/apply`: organizer/KYC onboarding.
- `/organizer`: organizer tournament dashboard.
- `/organizer/tournaments/:id`: create matches, upload rules/brackets/evidence.
- `/notifications`: notification inbox with action buttons.
- `/admin`: separate protected admin app only.

## 22. Current Smoke-Test Data

These are non-production test values from the current Railway/local smoke test:

- Organizer wallet: `0x51b3bd25fe883ffd5cc3b215a419b85714733a1abf2ea8c23302f6c9cc9bf3c6`
- Organizer tournament: `a2975914-4925-4dfa-a08f-9b09ed2ed39b`
- Organizer match: `cb4a429a-6268-4a77-a886-a3960f2cf25b`
- Walrus blob: `Fybpg_JvgRQB4hXSJFgsHehvCqNtydESYEwG3MRkI3s`
- Testnet Sui package: `0xeb9b807853b4a3e440c9dc6e988bfee06886b5177bbe297083e79afb97edc160`

Use these only to validate screen wiring while the real FE data flows are being built.
