# Frontend Integration

Kombat is moving to a Sui-first architecture with Dynamic embedded wallets.

## Authentication

Only Dynamic auth is supported.

```http
POST /api/auth/verify
Content-Type: application/json

{
  "dynamic_token": "<jwt-from-dynamic-sdk>"
}
```

The response includes the Kombat app JWT as `accessToken`. Send it on protected routes:

```http
Authorization: Bearer <accessToken>
```

Removed flows:

- Native wallet nonce signing
- Solana wallet-adapter auth
- Server-built Solana transaction responses

## Wallets

The wallet address returned by auth is the user's Sui embedded wallet address from Dynamic.

## Current Backend Surface

The backend still provides user profiles, notifications, uploads, and tournament metadata/pool endpoints while the Sui Move staking contract is being introduced.

Core routes:

- `POST /api/auth/verify`
- `GET /api/sui/config`
- `GET /api/sui/health`
- `GET /api/sui/wallets/:wallet/balances`
- `GET /api/sui/wallets/:wallet/usdc-balance`
- `GET /api/sui/wallets/:wallet/dashboard`
- `GET /api/sui/networks/:network/config`
- `GET /api/sui/networks/:network/health`
- `GET /api/sui/networks/:network/wallets/:wallet/balances`
- `GET /api/sui/networks/:network/wallets/:wallet/usdc-balance`
- `GET /api/sui/networks/:network/wallets/:wallet/dashboard`
- `GET /api/users/:wallet`
- `POST /api/users/:wallet`
- `GET /api/tournaments`
- `POST /api/tournaments`
- `GET /api/tournaments/source/pandascore`
- `POST /api/tournaments/source/pandascore/sync`
- `GET /api/tournaments/:id`
- `GET /api/tournaments/:id/outcome-proposals`
- `POST /api/tournaments/:id/outcome-proposals`
- `POST /api/outcome-proposals/:id/review`
- `GET /api/organizer/tournaments`
- `POST /api/organizer/tournaments`
- `POST /api/organizer/tournaments/:id/matches`
- `POST /api/organizers/apply`
- `POST /api/organizers/kyc-session`
- `GET /api/organizers/:wallet`
- `POST /api/organizers/:wallet/review`
- `GET /api/admin/organizers`
- `GET /api/admin/organizers/:wallet`
- `GET /api/admin/outcome-proposals`
- `GET /api/admin/outcome-proposals/:id`
- `POST /api/tournaments/:id/calculate`
- `POST /api/tournaments/:id/stake`
- `POST /api/tournaments/:id/resolve`
- `POST /api/tournaments/:id/cancel`
- `GET /api/ramps/providers`
- `POST /api/ramps/session`
- `POST /api/payments/intents`
- `GET /api/payments/intents/:id`
- `POST /api/payments/intents/:id/onramp-session`
- `GET /api/payments/intents/:id/ptb`
- `POST /api/receipt-market/listings`
- `GET /api/receipt-market/listings`
- `GET /api/receipt-market/listings/:id`
- `POST /api/receipt-market/listings/:id/activate`
- `GET /api/receipt-market/listings/:id/list-ptb`
- `GET /api/receipt-market/listings/:id/buy-ptb`
- `POST /api/receipt-market/listings/:id/mark-sold`
- `GET /api/transak/config`
- `POST /api/transak/quote`
- `POST /api/transak/widget-url`
- `GET /api/notifications/:wallet`
- `POST /api/notifications/:id/read`
- `GET /notifications/stream/:wallet`
- `GET /ws/notifications/:wallet`

Notification endpoints require `Authorization: Bearer <accessToken>`. Do not send JWTs in query strings.

Notification payloads are actionable. Render `payload.title` and `payload.body`, then use `payload.action` for the primary CTA:

```json
{
  "title": "Fund wallet to enter tournament",
  "body": "Chelsea vs Arsenal needs more USDC before staking on Chelsea.",
  "action": {
    "label": "Fund wallet",
    "type": "open_onramp",
    "method": "POST",
    "endpoint": "/api/payments/intents/<id>/onramp-session",
    "params": {
      "intent_id": "<id>",
      "shortfall_usdc": 25000000
    }
  },
  "entities": {
    "match_id": "<match id>",
    "amount_usdc": 25000000
  }
}
```

## Kombat Smart Pay

## Tournament Data

The frontend should read tournaments from Kombat, not PandaScore directly:

- `GET /api/tournaments`
- `GET /api/tournaments/:id`

The backend owns the PandaScore API key and normalizes provider data into Kombat's `matches` and `match_opponents` tables. An admin job can run `POST /api/tournaments/source/pandascore/sync` with `X-Admin-Token` to fetch upcoming, running, and recent past matches. Finished matches with a `winner_id` are resolved server-side so stake notifications and settlement state stay in sync.

Optional sync body:

```json
{
  "statuses": ["upcoming", "running", "past"],
  "videogame_slugs": ["csgo", "dota2", "lol"],
  "max_pages": 1,
  "per_page": 50
}
```

`POST /api/tournaments` still accepts PandaScore-shaped data for admin backfills and local development, but it should not be the normal mobile app path.

## Organizer Tournaments

Use organizer endpoints for games or events that PandaScore does not cover, such as CODM community tournaments.

Organizer onboarding:

1. Organizer signs in with Dynamic.
2. Organizer applies with `POST /api/organizers/apply`.
3. Organizer starts KYC with `POST /api/organizers/kyc-session`.
4. Admin/provider review updates the organizer with `POST /api/organizers/:wallet/review`.
5. Only wallets with `status = approved` and `kyc_status = verified` can create organizer markets.

`GET /api/organizers/:wallet` requires either that organizer's `Authorization: Bearer <accessToken>` or `X-Admin-Token`.

Organizer market creation:

1. Organizer creates a tournament with `POST /api/organizer/tournaments`.
2. Organizer stores rules/bracket files on Walrus and sends blob IDs as `rules_blob_id` and `bracket_blob_id`.
3. Organizer adds stakeable matches with `POST /api/organizer/tournaments/:id/matches`.
4. Users discover the matches through the normal `GET /api/tournaments` endpoint.

Apply as organizer:

```json
{
  "wallet_address": "0x...",
  "organization_name": "Lagos CODM League",
  "contact_email": "ops@example.com",
  "website_url": "https://example.com",
  "country": "NG",
  "description": "Community CODM tournament organizer"
}
```

Admin review:

```json
{
  "status": "approved",
  "kyc_status": "verified",
  "kyc_provider": "manual_review",
  "reviewed_by": "admin"
}
```

Admin dashboard endpoints:

- `GET /api/admin/organizers?status=pending&kyc_status=pending&limit=50`
- `GET /api/admin/organizers/:wallet`
- `POST /api/organizers/:wallet/review`
- `GET /api/admin/outcome-proposals?status=pending&source=agent&limit=50`
- `GET /api/admin/outcome-proposals/:id`
- `GET /api/admin/agent-runs?status=completed&limit=50`
- `GET /api/admin/agent-runs/:id`
- `POST /api/outcome-proposals/:id/review`

All admin endpoints require `X-Admin-Token` or `Authorization: Bearer <admin token>`.

Create tournament:

```json
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

Create match:

```json
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

## Walrus Artifacts And Agents

Walrus stores public, durable evidence for organizer markets and agent result reports. Use it for rules, brackets, screenshots, match reports, and audit manifests. Do not upload private KYC documents or sensitive user data unless a Seal-based privacy layer is added.

Frontend config:

```http
GET /api/walrus/config
```

Store a JSON artifact:

```http
POST /api/walrus/artifacts
Authorization: Bearer <accessToken>
Content-Type: application/json
```

```json
{
  "artifact_type": "bracket",
  "owner_wallet": "0x...",
  "match_id": "<optional match uuid>",
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

The response includes `blob_id`, `aggregator_url`, and `blob_url`. Save the blob ID into `rules_blob_id`, `bracket_blob_id`, or `evidence_blob_id` when creating tournaments, matches, or proposals.

Agent result flow:

1. Agent watches tournament sources, streams, scoreboards, and organizer pages.
2. Agent writes its report to Walrus with `POST /api/walrus/artifacts` using `X-Agent-Token`.
3. Agent submits `POST /api/agents/outcome-proposals` using the Walrus `blob_id`.
4. Admin reviews `GET /api/admin/agent-runs` and `GET /api/admin/outcome-proposals?source=agent`.
5. Admin approves, rejects, or disputes with `POST /api/outcome-proposals/:id/review`.

Agent submission:

```json
{
  "match_id": "<match uuid>",
  "agent_name": "kombat-outcome-agent",
  "watch_sources": ["organizer_site", "youtube_stream", "discord_announcement"],
  "proposed_winner_opponent_id": "<opponent uuid>",
  "confidence": "0.9100",
  "evidence_blob_id": "walrus_blob_result_evidence",
  "evidence_url": "https://aggregator.example/v1/blobs/walrus_blob_result_evidence",
  "evidence_summary": "Agent found matching bracket update and stream result.",
  "raw_output": {
    "checks": ["bracket winner matched", "stream title matched"]
  }
}
```

Admin agent endpoints:

- `GET /api/admin/agent-runs?status=completed&limit=50`
- `GET /api/admin/agent-runs/:id`

## Outcome Proposals

Organizer and agent results are submitted as proposals first. This keeps settlement safer for non-PandaScore events.

```json
{
  "source": "agent",
  "proposer_wallet": "0x...",
  "proposed_winner_opponent_id": "<opponent uuid>",
  "confidence": "0.9100",
  "evidence_blob_id": "walrus_blob_result_evidence",
  "evidence_url": "https://...",
  "evidence_summary": "Agent found matching bracket update and stream result."
}
```

Admin review:

```json
{
  "decision": "approve",
  "reviewer_wallet": "0x..."
}
```

`approve` settles the match when the proposal includes `proposed_winner_opponent_id`. `reject` and `dispute` update proposal/match verification status without settling.

Use payment intents for the primary staking flow. This lets the app keep a custom UI while turning a user action like "Stake $25 on Chelsea" into a programmable Sui payment.

1. User signs in with Dynamic.
2. User taps a stake CTA in the Kombat UI.
3. Client creates a payment intent with match, opponent, amount, and optional `reserve_balance_usdc`.
4. Backend checks the user's Sui USDC balance and returns the exact funding shortfall.
5. If funding is needed, client calls the intent on-ramp session endpoint and opens Dynamic native funding from the custom UI.
6. Client refetches the intent until `onramp_required` is false.
7. Client fetches the PTB plan and builds/signs the Sui transaction with Dynamic.
8. The Move contract locks USDC and mints a `StakeReceipt`.

Create an intent:

```http
POST /api/payments/intents
Authorization: Bearer <accessToken>
Content-Type: application/json
```

```json
{
  "wallet_address": "0x...",
  "match_id": "match-uuid",
  "opponent_id": "opponent-uuid",
  "amount_usdc": 25000000,
  "reserve_balance_usdc": 5000000,
  "network": "testnet"
}
```

The reserve rule means: stake the requested amount, but preserve at least that much USDC in the wallet after staking. Use `POST /api/payments/intents/:id/onramp-session` only when the intent says `onramp_required: true`.

## Trading In And Out

Users can exit before settlement by selling their `StakeReceipt`; buyers enter by buying that receipt. The pool is not unwound early, so pool economics stay intact.

Seller flow:

1. Create a receipt listing with `POST /api/receipt-market/listings`.
2. Fetch `GET /api/receipt-market/listings/:id/list-ptb`.
3. Use Dynamic to sign the returned `list_receipt<USDC>` Move call.
4. After the transaction, call `POST /api/receipt-market/listings/:id/activate` with the shared listing object ID.

Buyer flow:

1. Discover listings with `GET /api/receipt-market/listings?match_id=...`.
2. Fetch `GET /api/receipt-market/listings/:id/buy-ptb`.
3. Use Dynamic to sign the returned `buy_receipt<USDC>` Move call.
4. After the transaction, call `POST /api/receipt-market/listings/:id/mark-sold` with the buyer wallet and transaction hash.

On-chain, the buyer pays the seller and receives the `StakeReceipt` atomically. The receipt owner is updated, so any later claim/refund goes to the buyer.

## Wallet Screen Data

Use one fetch for the wallet screen:

```http
GET /api/sui/networks/testnet/wallets/:wallet/dashboard?limit=20&offset=0
```

Field mapping:

- Segmented control: `network`
- Available Balance: `available_balance_usdc`
- Locked in Kombats: `locked_in_kombats_usdc`
- Transaction History: `transaction_history`
- Fund Wallet button: `actions.fund_wallet`
- Withdraw button: `actions.withdraw` is currently disabled because Kombat supports on-ramp only.

Amounts are returned in micro-USDC. `1 USDC = 1_000_000`.

## Funding

Use Dynamic native funding as the primary Web2 on-ramp path.

Discover available providers:

```http
GET /api/ramps/providers?country=NG
```

Create a Dynamic-native funding session:

```http
POST /api/ramps/session
Authorization: Bearer <accessToken>
Content-Type: application/json
```

```json
{
  "wallet_address": "0x...",
  "product": "BUY",
  "fiat_currency": "USD",
  "fiat_amount": 50,
  "crypto_currency_code": "USDC",
  "network": "sui"
}
```

The response returns `client_action: "open_dynamic_onramp"`. The frontend should launch Dynamic's native onramp/funding UI and pass the wallet/network/asset values from the response. `SELL` requests are rejected; Kombat is on-ramp only.

The Transak endpoints remain available only as an optional fallback when `TRANSAK_API_KEY` is configured.
