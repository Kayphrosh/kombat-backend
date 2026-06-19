# Kombat Frontend Integration Guide

This backend is the Kombat API for a Sui-first app using Dynamic embedded wallets. The frontend should treat Dynamic as the only authentication source, use the user's Dynamic-created Sui embedded wallet as the Kombat wallet, and use backend-provided PTB descriptors for Sui transactions.

Production API:

```text
https://kombat-backend-production.up.railway.app
```

Local API defaults to:

```text
http://localhost:3000
```

## Integration Rules

- Only Dynamic auth is supported. Do not use native wallet nonce signing or Solana wallet-adapter auth.
- Wallet addresses are Sui addresses from Dynamic embedded wallets.
- Protected user routes require `Authorization: Bearer <accessToken>`.
- Do not put JWTs in query strings, including notification streams.
- Amounts are always micro-USDC. `1 USDC = 1_000_000`.
- Most API responses are wrapped as `{ "success": true, "data": ..., "error": null }`.
- `POST /api/auth/verify` is the exception. It returns `{ "user": ..., "accessToken": "..." }` directly.
- Treat `401` as expired or invalid Kombat JWT. Clear the app token and re-run Dynamic verify.
- The primary tournament staking path is payment intents, not direct `POST /api/tournaments/:id/stake`.
- Frontend reads tournament data from Kombat endpoints. Do not call PandaScore directly from the app.
- Transak is optional fallback only. Dynamic native funding is the primary on-ramp path.

## Dynamic Setup

Dynamic must be configured in the dashboard before new accounts can receive Sui wallets.

1. Enable Sui under Dynamic embedded wallet chains, not only under general networks.
2. Open the Embedded Wallets settings gear and enable one Sui network for wallet creation.
3. Enable only one Sui network at a time in Dynamic for this app environment.
4. In the client, create the wallet with the Dynamic SDK chain value `Sui`.
5. Do not retry with `SUI`; the backend and Dynamic SDK expect Sui, and `SUI` is invalid for wallet creation.

If Dynamic logs `No enabled embedded wallet chains`, the problem is dashboard configuration under Embedded Wallets. Enabling Sui in the regular Networks page is not enough.

## API Client Shape

Use one helper for wrapped Kombat responses:

```ts
type ApiEnvelope<T> = {
  success: boolean;
  data: T | null;
  error: string | null;
};

async function apiFetch<T>(path: string, init: RequestInit = {}, token?: string): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    ...init,
    headers: {
      "Content-Type": "application/json",
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
      ...(init.headers ?? {}),
    },
  });

  const body = await res.json().catch(() => null);
  if (!res.ok) {
    throw new Error(body?.error ?? body?.message ?? `HTTP ${res.status}`);
  }

  if (body && typeof body === "object" && "success" in body) {
    if (!body.success) throw new Error(body.error ?? "Request failed");
    return body.data as T;
  }

  return body as T;
}
```

Keep auth verify separate because it is not envelope-wrapped:

```ts
type DynamicAuthResponse = {
  user: UserRecord;
  accessToken: string;
};

async function verifyDynamic(dynamicToken: string): Promise<DynamicAuthResponse> {
  const res = await fetch(`${API_BASE}/api/auth/verify`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ dynamic_token: dynamicToken }),
  });

  const body = await res.json().catch(() => null);
  if (!res.ok) throw new Error(body?.error ?? body?.message ?? `Auth verify failed: ${res.status}`);
  return body;
}
```

## Authentication

### Verify Dynamic Token

```http
POST /api/auth/verify
Content-Type: application/json

{
  "dynamic_token": "<jwt-from-dynamic-sdk>"
}
```

`dynamicToken` is also accepted as an alias, but new code should send `dynamic_token`.

Response:

```json
{
  "user": {
    "id": "uuid",
    "wallet_address": "0x...",
    "email": null,
    "display_name": null,
    "avatar_url": null,
    "wins": 0,
    "losses": 0,
    "created_at": "2026-06-16T00:00:00Z",
    "updated_at": "2026-06-16T00:00:00Z"
  },
  "accessToken": "<kombat-app-jwt>"
}
```

The backend extracts the wallet from the Dynamic token, upserts the user profile, and returns a short-lived Kombat app JWT. If the Dynamic token has no Sui wallet address, the endpoint returns `400`.

### Auth Flow

1. User signs in with Dynamic.
2. Ensure a Sui embedded wallet exists.
3. Get the Dynamic JWT from the SDK.
4. Call `POST /api/auth/verify`.
5. Store `accessToken` and `user.wallet_address`.
6. Send `Authorization: Bearer <accessToken>` on protected Kombat routes.
7. On `401`, clear the Kombat token and call verify again with a fresh Dynamic token.

## Sui Config And Wallet Data

### App Config

```http
GET /api/sui/config
GET /api/sui/networks/:network/config
```

Returns:

```ts
type SuiAppConfigResponse = {
  active_network: string;
  networks: SuiConfigResponse[];
};

type SuiConfigResponse = {
  network: string;
  rpc_url: string;
  package_id: string | null;
  wager_package_id: string | null;
  usdc_coin_type: string | null;
  staking_module: string;
  wager_module: string;
};
```

### Health

```http
GET /api/sui/health
GET /api/sui/networks/:network/health
```

Use this for environment diagnostics. It includes RPC reachability and chain metadata.

### Balances

```http
GET /api/sui/wallets/:wallet/balances
GET /api/sui/wallets/:wallet/usdc-balance
GET /api/sui/networks/:network/wallets/:wallet/balances
GET /api/sui/networks/:network/wallets/:wallet/usdc-balance
```

Wallet addresses are validated and normalized as Sui addresses.

### Wallet Screen

Use this as the single wallet screen fetch:

```http
GET /api/sui/networks/testnet/wallets/:wallet/dashboard?limit=20&offset=0
```

Map fields directly:

- Segmented control: `network`
- Available Balance: `available_balance_usdc`
- Locked in Kombats: `locked_in_kombats_usdc`
- Transaction History: `transaction_history`
- Fund Wallet button: `actions.fund_wallet`
- Withdraw button: `actions.withdraw`

`actions.withdraw.enabled` is currently false because Kombat supports on-ramp only.

## Funding

### Discover Providers

```http
GET /api/ramps/providers?country=NG
```

Response data includes:

```ts
type RampProvidersResponse = {
  primary_provider: "dynamic_native";
  default_network: "sui";
  default_crypto_currency: "USDC";
  default_fiat_currency: "USD";
  partner_fee_bps: number;
  country?: string;
  providers: unknown[];
};
```

### Create Dynamic Funding Session

```http
POST /api/ramps/session
Authorization: Bearer <accessToken>
Content-Type: application/json

{
  "wallet_address": "0x...",
  "product": "BUY",
  "fiat_currency": "USD",
  "fiat_amount": 50,
  "crypto_currency_code": "USDC",
  "network": "sui"
}
```

Response data:

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
  fiat_amount?: string;
  crypto_amount?: string;
  note: string;
};
```

When `client_action` is `open_dynamic_onramp`, launch Dynamic's native funding UI with the wallet, network, asset, and amount from the response. `SELL` is rejected.

## Tournament Data

Frontend reads tournaments from Kombat:

```http
GET /api/tournaments
GET /api/tournaments?status=upcoming&videogame=codm&limit=20&offset=0
GET /api/tournaments?tournament_id=12345&status=all&limit=100
GET /api/tournaments/:id
```

Query params:

- `status`: defaults to current feed (`upcoming` + `live`); accepts `active` for current feed and `completed`/`finished`/`past` for result history
- `videogame`: videogame slug
- `league_id`: PandaScore league id
- `tournament_id`, `tournament_slug`: PandaScore tournament filters for full tournament/bracket views
- `search`: match name search
- `limit`, `offset`

Use `status=all` with `tournament_id`/`tournament_slug` when a screen needs every synced match in a real tournament. Incomplete matches are stored and returned for schedule/bracket context, but they are not stakeable until two opponents and a Sui pool are configured.

Response data is `MatchWithOdds[]` for list and `MatchWithOdds` for detail:

```ts
type MatchWithOdds = MatchRecord & {
  opponents: OpponentWithPool[];
  total_pool_usdc: number;
  total_stakers: number;
};

type OpponentWithPool = MatchOpponentRecord & {
  pool_usdc: number;
  pool_percentage: number;
  odds: number;
  staker_count: number;
};
```

The backend owns GRID access and normalizes provider data. The mobile app should not call GRID.

Admin-only GRID sync:

```http
GET /api/tournaments/source/grid
POST /api/tournaments/source/grid/sync
X-Admin-Token: <admin-token>
```

Optional sync body:

```json
{
  "statuses": ["upcoming", "running"],
  "videogame_slugs": ["csgo", "dota2", "lol", "valorant"],
  "tournament_id": "optional-grid-tournament-id",
  "tournament_slug": "optional-grid-tournament-slug",
  "max_pages": 3,
  "per_page": 100
}
```

Sync stores matches even when both opponents are not known yet; those are counted in `synced_incomplete`.

`POST /api/tournaments` accepts PandaScore-shaped data for admin backfills and local development only. It requires `X-Admin-Token` and should not be normal app flow.

## Smart Pay Staking

Use payment intents for the primary stake flow. The backend checks Sui USDC balance, calculates funding shortfall, and returns a PTB descriptor for the frontend to sign with Dynamic.

### Stake Flow

1. User signs in with Dynamic and verifies with Kombat.
2. User selects match, opponent, and amount.
3. Client creates a payment intent.
4. Backend returns current balance, required balance, and shortfall.
5. If `funding.onramp_required` is true, create/open the intent on-ramp session.
6. Poll or refetch the intent until `onramp_required` is false.
7. Fetch the PTB descriptor.
8. Build and sign the Sui transaction with Dynamic.
9. The Move contract locks USDC and mints a `StakeReceipt`.

### Create Intent

```http
POST /api/payments/intents
Authorization: Bearer <accessToken>
Content-Type: application/json

{
  "wallet_address": "0x...",
  "match_id": "match-uuid",
  "opponent_id": "opponent-uuid",
  "amount_usdc": 25000000,
  "reserve_balance_usdc": 5000000,
  "network": "testnet"
}
```

Rules:

- `wallet_address` must match the wallet in the JWT.
- Minimum amount is `1_000_000` micro-USDC.
- `reserve_balance_usdc` is optional and defaults to `0`.
- `settlement_rule` is optional and currently must be `return_to_wallet`.

Response data:

```ts
type PaymentIntentResponse = {
  intent: PaymentIntentRecord;
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
};
```

### Refresh Intent

```http
GET /api/payments/intents/:id
Authorization: Bearer <accessToken>
```

Use this after funding to re-check the wallet balance and intent status.

### Intent On-Ramp

```http
POST /api/payments/intents/:id/onramp-session
Authorization: Bearer <accessToken>
```

Call this only when the intent says `onramp_required: true`. If funding is required, response data includes `ramp_session` with `client_action: "open_dynamic_onramp"`.

### Intent PTB

```http
GET /api/payments/intents/:id/ptb
Authorization: Bearer <accessToken>
```

Response data:

```ts
type PaymentIntentPtbResponse = {
  intent_id: string;
  network: string;
  can_build: boolean;
  reason: string | null;
  coin_type: string | null;
  amount_usdc: number;
  reserve_balance_usdc: number;
  expected_receipt_type: "StakeReceipt";
  steps: PaymentPtbStep[];
  move_call: PaymentMoveCall | null;
};
```

If `can_build` is false, show a blocking state using `reason`. Common reasons are:

- `intent_requires_funding`
- `staking_package_not_configured`
- `usdc_coin_type_not_configured`
- `pool_object_not_configured`

The Move call target is:

```text
<package_id>::<staking_module>::stake
```

The frontend must provide the user's USDC coin input, shared pool object, outcome id, amount, and Sui clock exactly as described by `move_call.arguments`.

## PTB Descriptor Handling

Kombat returns descriptors, not serialized transactions. The frontend builds the transaction with the Sui SDK and signs it using Dynamic.

Descriptor fields:

```ts
type PaymentMoveCall = {
  target: string;
  package_id: string;
  module: string;
  function: string;
  type_arguments: string[];
  arguments: Array<{
    name: string;
    kind: string;
    value?: unknown;
    source: string;
  }>;
};
```

Implementation pattern:

1. Read `move_call.target` and `type_arguments`.
2. Resolve object arguments from `value`.
3. For `source: "frontend_wallet"`, select/split the required USDC coin from the user's wallet.
4. Use shared objects as shared object inputs.
5. Use `0x6` as the Sui clock when specified.
6. Sign and execute through the Dynamic Sui wallet.
7. Save returned tx hashes/object ids through the matching backend endpoint when the flow requires activation or indexing.

## Receipt Market

Users can sell `StakeReceipt` objects before settlement. The pool is not unwound; ownership of the receipt changes.

### Seller Flow

1. Create listing draft.
2. Fetch list PTB.
3. Sign `list_receipt<USDC>` with Dynamic.
4. Activate listing with the created shared listing object id.

Create listing:

```http
POST /api/receipt-market/listings
Authorization: Bearer <accessToken>
Content-Type: application/json

{
  "wallet_address": "0x...",
  "receipt_id": "0x...",
  "match_id": "match-uuid",
  "opponent_id": "opponent-uuid",
  "ask_amount_usdc": 20000000,
  "network": "testnet"
}
```

List PTB:

```http
GET /api/receipt-market/listings/:id/list-ptb
Authorization: Bearer <accessToken>
```

Activate after successful Sui transaction:

```http
POST /api/receipt-market/listings/:id/activate
Authorization: Bearer <accessToken>
Content-Type: application/json

{
  "wallet_address": "0x...",
  "listing_object_id": "0x...",
  "listing_tx_hash": "..."
}
```

### Buyer Flow

Discover listings:

```http
GET /api/receipt-market/listings?match_id=match-uuid&status=active&limit=20&offset=0
GET /api/receipt-market/listings/:id
```

Buy PTB:

```http
GET /api/receipt-market/listings/:id/buy-ptb
Authorization: Bearer <accessToken>
```

Mark sold after successful Sui transaction:

```http
POST /api/receipt-market/listings/:id/mark-sold
Authorization: Bearer <accessToken>
Content-Type: application/json

{
  "buyer_wallet": "0x...",
  "sale_tx_hash": "..."
}
```

The buyer cannot be the seller. Draft, sold, cancelled, and expired listings should not be shown as buyable.

## P2P Wagers

P2P wagers use on-chain Sui objects plus backend indexing/social state. In the current implementation these endpoints validate request data and Sui addresses, but they do not enforce the Kombat app JWT. The user still signs all on-chain PTBs with their Dynamic Sui wallet.

### Create Wager

First request a create PTB:

```http
POST /api/wagers/create-ptb
Content-Type: application/json

{
  "initiator": "0x...",
  "stake_usdc": 25000000,
  "description": "Chelsea beats Arsenal",
  "expiry_ts": 1782932400000,
  "resolver": "0x...",
  "network": "testnet",
  "challenger_address": "0x...",
  "initiator_option": "Chelsea"
}
```

After the user signs and executes the Sui create transaction, index it:

```http
POST /api/wagers
Content-Type: application/json

{
  "on_chain_address": "0x...",
  "wager_id": 1,
  "initiator": "0x...",
  "challenger_address": "0x...",
  "stake_usdc": 25000000,
  "description": "Chelsea beats Arsenal",
  "expiry_ts": 1782932400000,
  "resolution_source": "manual",
  "resolver": "0x...",
  "initiator_option": "Chelsea",
  "terms": {
    "title": "Chelsea vs Arsenal wager",
    "rules": ["Regulation time result"]
  }
}
```

If `terms` is present, the backend stores it on Walrus best-effort.

### Discover Wagers

```http
GET /api/wagers?status=open&limit=20&offset=0
GET /api/wagers/mine?wallet=0x...&limit=20&offset=0
GET /api/wagers/:address
```

Filters:

- `initiator`
- `challenger`
- `status`
- `limit`
- `offset`

### Accept Wager

1. Fetch accept PTB.
2. Sign the accept transaction with Dynamic.
3. Update backend status/social state.

```http
GET /api/wagers/:address/accept-ptb
```

```http
POST /api/wagers/:address/accept
Content-Type: application/json

{
  "challenger": "0x..."
}
```

### Resolve Or Dispute

Mutual winner declaration:

```http
POST /api/wagers/:address/declare-winner
Content-Type: application/json

{
  "participant": "0x...",
  "declared_winner": "0x..."
}
```

If both participants declare the same winner, the backend resolves indexed state and attempts on-chain resolution with the configured resolver signer.

Resolve PTB for resolver-driven client signing:

```http
GET /api/wagers/:address/resolve-ptb?winner=0x...
```

Disputes:

```http
POST /api/wagers/:address/disputes
Content-Type: application/json

{
  "submitter": "0x...",
  "description": "The match was cancelled.",
  "declared_winner": "0x...",
  "evidence": {
    "sources": ["https://example.com/result"],
    "notes": "Organizer announcement"
  }
}
```

```http
GET /api/wagers/:address/disputes
GET /api/wagers/:address/artifacts
```

Statuses supported by `POST /api/wagers/:address/status` are `open`, `active`, `cancelled`, `declined`, and `expired`.

## Notifications

Notification endpoints require the Kombat app JWT and the requested wallet must match the JWT wallet.

```http
GET /api/notifications/:wallet?limit=20&offset=0
POST /api/notifications/:id/read
GET /notifications/stream/:wallet
GET /ws/notifications/:wallet
```

For SSE and websocket connections, send the JWT in the `Authorization` header. Do not use query-string tokens.

Notification records:

```ts
type NotificationRecord = {
  id: string;
  user_wallet: string;
  kind: string;
  payload: {
    title: string;
    body: string;
    action: {
      label: string;
      type: string;
      method: string;
      endpoint: string;
      params: Record<string, unknown>;
    };
    entities: Record<string, unknown>;
  } | null;
  is_read: boolean;
  created_at: string;
};
```

Render `payload.title` and `payload.body`, then use `payload.action` for the primary CTA. Example actions include:

- `open_onramp`
- `open_tournament`
- `open_payment_intent`
- `open_receipt_listing`

The websocket accepts JSON acknowledgements:

```json
{ "ack": "<notification-id>" }
```

## User Profiles

Public profile and search:

```http
GET /api/users/:wallet
POST /api/users/:wallet
DELETE /api/users/:wallet
GET /api/users/search?q=ola&limit=20
GET /api/users/:wallet/stats
GET /api/home/:wallet
GET /api/users/:wallet/stakes?status=active&limit=20&offset=0
GET /api/users/:wallet/stake-stats
```

Update profile:

```json
{
  "email": "user@example.com",
  "display_name": "Ola",
  "avatar_url": "https://..."
}
```

Notification preferences:

```http
GET /api/users/:wallet/notification-settings
PUT /api/users/:wallet/notification-settings
POST /api/users/:wallet/push-token
```

Push token body:

```json
{
  "expo_token": "ExponentPushToken[...]"
}
```

## Uploads

```http
POST /api/uploads
Content-Type: multipart/form-data
```

Fields:

- `file`: uploaded file
- `type`: optional category, defaults to `general`

Response data:

```json
{
  "url": "https://..."
}
```

Use uploads for public app media such as avatars or event images. Use Walrus artifacts for durable market evidence, rules, brackets, reports, and audit manifests.

## Organizer Tournaments

Organizer-created tournaments cover events PandaScore does not cover, such as CODM community tournaments.

### Organizer Onboarding

1. Organizer signs in with Dynamic.
2. Organizer applies.
3. Organizer starts KYC.
4. Admin/provider reviews.
5. Only `status = approved` and `kyc_status = verified` wallets can create organizer markets.

Apply:

```http
POST /api/organizers/apply
Authorization: Bearer <accessToken>
Content-Type: application/json

{
  "wallet_address": "0x...",
  "organization_name": "Lagos CODM League",
  "contact_email": "ops@example.com",
  "website_url": "https://example.com",
  "country": "NG",
  "description": "Community CODM tournament organizer"
}
```

Start KYC:

```http
POST /api/organizers/kyc-session
Authorization: Bearer <accessToken>
Content-Type: application/json

{
  "wallet_address": "0x...",
  "provider": "manual_review",
  "return_url": "https://example.com/organizer/kyc"
}
```

Get organizer profile:

```http
GET /api/organizers/:wallet
Authorization: Bearer <accessToken>
```

This endpoint requires either the organizer's own JWT or admin credentials.

### Create Organizer Market

Create tournament:

```http
POST /api/organizer/tournaments
Authorization: Bearer <accessToken>
Content-Type: application/json

{
  "organizer_wallet": "0x...",
  "name": "CODM Lagos Invitational",
  "videogame_name": "Call of Duty Mobile",
  "videogame_slug": "codm",
  "description": "Community bracket",
  "rules_blob_id": "walrus_blob_rules",
  "bracket_blob_id": "walrus_blob_bracket",
  "starts_at": "2026-07-01T18:00:00Z"
}
```

Create stakeable match:

```http
POST /api/organizer/tournaments/:id/matches
Authorization: Bearer <accessToken>
Content-Type: application/json

{
  "organizer_wallet": "0x...",
  "name": "Alpha Clan vs ZoneX",
  "scheduled_at": "2026-07-01T19:00:00Z",
  "match_type": "best_of",
  "number_of_games": 3,
  "opponents": [
    { "pandascore_id": 0, "opponent_type": "Team", "name": "Alpha Clan" },
    { "pandascore_id": 0, "opponent_type": "Team", "name": "ZoneX" }
  ]
}
```

Users discover organizer matches through normal `GET /api/tournaments`.

List organizer tournaments:

```http
GET /api/organizer/tournaments?organizer_wallet=0x...&status=active&videogame=codm
```

## Outcome Proposals

Organizer and agent results are submitted as proposals first. Approval can settle the match when a winner opponent id is present.

Create proposal:

```http
POST /api/tournaments/:id/outcome-proposals
Authorization: Bearer <accessToken>
Content-Type: application/json

{
  "source": "organizer",
  "proposer_wallet": "0x...",
  "proposed_winner_opponent_id": "opponent-uuid",
  "confidence": "0.9100",
  "evidence_blob_id": "walrus_blob_result_evidence",
  "evidence_url": "https://...",
  "evidence_summary": "Organizer bracket and stream result match."
}
```

List proposals for a match:

```http
GET /api/tournaments/:id/outcome-proposals
```

Admin review:

```http
POST /api/outcome-proposals/:id/review
X-Admin-Token: <admin-token>
Content-Type: application/json

{
  "decision": "approve",
  "reviewer_wallet": "0x..."
}
```

`approve` settles when the proposal includes `proposed_winner_opponent_id`. `reject` and `dispute` update proposal or verification status without settlement.

## Walrus Artifacts

Walrus stores public durable evidence. Use it for rules, brackets, screenshots, match reports, and audit manifests. Do not upload private KYC documents or sensitive user data unless a privacy layer is added.

Config:

```http
GET /api/walrus/config
```

Store artifact:

```http
POST /api/walrus/artifacts
Authorization: Bearer <accessToken>
Content-Type: application/json

{
  "artifact_type": "bracket",
  "owner_wallet": "0x...",
  "match_id": "match-uuid",
  "content_type": "application/json",
  "manifest": {
    "title": "CODM Lagos Invitational Bracket",
    "sources": ["https://organizer.example/bracket"],
    "notes": "Round one bracket published by organizer."
  },
  "metadata": {
    "tournament_name": "CODM Lagos Invitational"
  }
}
```

Response data includes:

- `blob_id`
- `aggregator_url`
- `blob_url`
- indexed artifact fields

Save `blob_id` into `rules_blob_id`, `bracket_blob_id`, or `evidence_blob_id` when creating tournaments, matches, or proposals.

Other artifact routes:

```http
GET /api/walrus/artifacts/:id
GET /api/walrus/blobs/:blob_id/url
```

Walrus artifact creation accepts one of:

- User `Authorization: Bearer <accessToken>` with matching `owner_wallet`
- `X-Admin-Token`
- `X-Agent-Token`
- `Authorization: Bearer <admin-or-agent-token>`

## Agents

Agents submit evidence-backed outcome proposals. This is mostly admin/operator surface, but the frontend admin dashboard can read the review data.

Agent submission:

```http
POST /api/agents/outcome-proposals
X-Agent-Token: <agent-token>
Content-Type: application/json

{
  "match_id": "match-uuid",
  "agent_name": "kombat-outcome-agent",
  "watch_sources": ["organizer_site", "youtube_stream", "discord_announcement"],
  "proposed_winner_opponent_id": "opponent-uuid",
  "confidence": "0.9100",
  "evidence_summary": "Agent found matching bracket update and stream result.",
  "raw_output": {
    "match_id": "match-uuid",
    "winner": "Alpha Clan",
    "source_data": [{ "url": "https://..." }],
    "checks": ["bracket winner matched", "stream title matched"]
  }
}
```

`raw_output` is required and must contain enough schema data for backend validation.

Admin agent routes:

```http
GET /api/admin/agent-runs?status=completed&limit=50
GET /api/admin/agent-runs/:id
```

## Admin Dashboard

Admin endpoints accept either `X-Admin-Token: <admin-token>` or `Authorization: Bearer <admin-token>`.

```http
GET /api/admin/organizers?status=pending&kyc_status=pending&limit=50
GET /api/admin/organizers/:wallet
POST /api/organizers/:wallet/review
GET /api/admin/outcome-proposals?status=pending&source=agent&limit=50
GET /api/admin/outcome-proposals/:id
GET /api/admin/agent-runs?status=completed&limit=50
GET /api/admin/agent-runs/:id
POST /api/outcome-proposals/:id/review
```

Organizer review body:

```json
{
  "status": "approved",
  "kyc_status": "verified",
  "kyc_provider": "manual_review",
  "reviewed_by": "admin"
}
```

## Transak Fallback

Transak remains available only when configured.

```http
GET /api/transak/config
POST /api/transak/quote
POST /api/transak/widget-url
```

Use Dynamic native funding first. Only show Transak fallback when config says it is enabled and product requirements need it.

## Legacy Pool Routes

These routes still exist for admin tooling, backfills, and legacy flows:

```http
POST /api/tournaments/:id/calculate
POST /api/tournaments/:id/stake
GET /api/tournaments/:id/stakes
POST /api/tournaments/:id/resolve
POST /api/tournaments/:id/cancel
POST /api/tournaments/:id/sync
GET /api/users/:wallet/stakes
GET /api/users/:wallet/stake-stats
```

For app staking, prefer payment intents and PTB signing. Do not build the primary mobile stake CTA on `POST /api/tournaments/:id/stake`.

## Error Handling

Wrapped error response:

```json
{
  "success": false,
  "data": null,
  "error": "wallet in token does not match request"
}
```

Common HTTP statuses:

- `400`: validation failure, missing wallet in Dynamic token, bad Sui address, unsupported network, insufficient request data
- `401`: missing/expired/invalid Kombat token, wallet mismatch, invalid admin or agent token
- `404`: resource not found
- `500`: backend configuration or provider error

Recommended frontend behavior:

- Show validation messages from `error`.
- On `401`, clear the Kombat JWT and re-run Dynamic verify.
- Do not retry protected calls without a token.
- Treat PTB `can_build: false` as a UI-blocking state, not a crash.
- For funding, refetch intent/dashboard after the Dynamic on-ramp flow completes.

## Route Reference

### Auth

```text
POST /api/auth/verify
```

### Sui

```text
GET /api/sui/config
GET /api/sui/health
GET /api/sui/wallets/:wallet/balances
GET /api/sui/wallets/:wallet/usdc-balance
GET /api/sui/wallets/:wallet/dashboard
GET /api/sui/networks/:network/config
GET /api/sui/networks/:network/health
GET /api/sui/networks/:network/wallets/:wallet/balances
GET /api/sui/networks/:network/wallets/:wallet/usdc-balance
GET /api/sui/networks/:network/wallets/:wallet/dashboard
```

### Users

```text
GET /api/users/search
GET /api/users/:wallet
POST /api/users/:wallet
DELETE /api/users/:wallet
GET /api/home/:wallet
GET /api/users/:wallet/stats
GET /api/users/:wallet/stakes
GET /api/users/:wallet/stake-stats
GET /api/users/:wallet/notification-settings
PUT /api/users/:wallet/notification-settings
POST /api/users/:wallet/push-token
```

### Tournaments

```text
GET /api/tournaments
POST /api/tournaments
GET /api/tournaments/source/pandascore
POST /api/tournaments/source/pandascore/sync
GET /api/tournaments/:id
GET /api/tournaments/:id/outcome-proposals
POST /api/tournaments/:id/outcome-proposals
POST /api/outcome-proposals/:id/review
POST /api/tournaments/:id/calculate
POST /api/tournaments/:id/stake
GET /api/tournaments/:id/stakes
POST /api/tournaments/:id/resolve
POST /api/tournaments/:id/cancel
POST /api/tournaments/:id/sync
```

### Organizer And Admin

```text
GET /api/organizer/tournaments
POST /api/organizer/tournaments
POST /api/organizer/tournaments/:id/matches
POST /api/organizers/apply
POST /api/organizers/kyc-session
GET /api/organizers/:wallet
POST /api/organizers/:wallet/review
GET /api/admin/organizers
GET /api/admin/organizers/:wallet
GET /api/admin/outcome-proposals
GET /api/admin/outcome-proposals/:id
GET /api/admin/agent-runs
GET /api/admin/agent-runs/:id
```

### Payments And Funding

```text
GET /api/ramps/providers
POST /api/ramps/session
POST /api/payments/intents
GET /api/payments/intents/:id
POST /api/payments/intents/:id/onramp-session
GET /api/payments/intents/:id/ptb
GET /api/transak/config
POST /api/transak/quote
POST /api/transak/widget-url
```

### Receipt Market

```text
POST /api/receipt-market/listings
GET /api/receipt-market/listings
GET /api/receipt-market/listings/:id
POST /api/receipt-market/listings/:id/activate
GET /api/receipt-market/listings/:id/list-ptb
GET /api/receipt-market/listings/:id/buy-ptb
POST /api/receipt-market/listings/:id/mark-sold
```

### P2P Wagers

```text
GET /api/wagers
POST /api/wagers
GET /api/wagers/mine
POST /api/wagers/create-ptb
GET /api/wagers/:address
POST /api/wagers/:address/accept
GET /api/wagers/:address/accept-ptb
GET /api/wagers/:address/resolve-ptb
POST /api/wagers/:address/status
POST /api/wagers/:address/declare-winner
GET /api/wagers/:address/artifacts
GET /api/wagers/:address/disputes
POST /api/wagers/:address/disputes
```

### Walrus, Agents, Notifications, Uploads

```text
GET /api/walrus/config
POST /api/walrus/artifacts
GET /api/walrus/artifacts/:id
GET /api/walrus/blobs/:blob_id/url
POST /api/agents/outcome-proposals
GET /api/notifications/:wallet
POST /api/notifications/:id/read
GET /notifications/stream/:wallet
GET /ws/notifications/:wallet
POST /api/uploads
```

### Health And Metrics

```text
GET /health
GET /metrics
```

## Frontend Checklist

- Dynamic embedded wallets are enabled for Sui under Embedded Wallets settings.
- Client creates Sui embedded wallet with chain `Sui`.
- Dynamic JWT is exchanged through `POST /api/auth/verify`.
- App stores and sends Kombat `accessToken`.
- Protected wallet-bearing requests use the same wallet as the JWT.
- Notification requests use headers, not query-string tokens.
- Tournaments come from `/api/tournaments`.
- Stake CTA uses payment intents.
- Funding CTA opens Dynamic native on-ramp from ramp session response.
- PTB screens handle `can_build: false` gracefully.
- Micro-USDC is formatted for display and never sent as decimal dollars.
- Organizer-created files and result evidence use Walrus artifacts.
- Receipt market flows activate/mark-sold after successful Sui transactions.
- `401` clears app token and re-verifies with Dynamic.
