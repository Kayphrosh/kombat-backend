# Kombat Backend

Rust API for Kombat, moving to a **Sui-first** architecture with Dynamic embedded wallets.

## Current Scope

- Dynamic-only authentication through `POST /api/auth/verify`
- Sui wallet addresses extracted from Dynamic credentials
- User profiles, notifications, uploads, and tournament metadata APIs
- Existing tournament pool records kept in Postgres while the Sui Move staking contract is introduced

Removed from the active backend:

- Solana RPC service and indexer
- Native wallet nonce authentication
- Server-built Solana transaction routes
- Anchor/Solana program workspace

## Required Environment

```env
DATABASE_URL=postgres://user:password@localhost:5432/wager_db
AUTH_JWT_SECRET=<long-random-secret>
DYNAMIC_ENVIRONMENT_ID=<dynamic-environment-id>
SUI_NETWORK=testnet
SUI_TESTNET_RPC_URL=https://fullnode.testnet.sui.io:443
SUI_MAINNET_RPC_URL=https://fullnode.mainnet.sui.io:443
SUI_TESTNET_PACKAGE_ID=<testnet-move-package-id-optional-for-now>
SUI_MAINNET_PACKAGE_ID=<mainnet-move-package-id-optional-for-now>
SUI_TESTNET_USDC_COIN_TYPE=0xa1ec7fc00a6f40db9693ad1415d0c193ad3906494428cf252621037bd7117e29::usdc::USDC
SUI_MAINNET_USDC_COIN_TYPE=0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC
SUI_STAKING_MODULE=tournament_staking
RAMP_PRIMARY_PROVIDER=dynamic_native
DYNAMIC_ONRAMP_ENABLED=true
MANUAL_CRYPTO_DEPOSIT_ENABLED=true
RAMP_DEFAULT_NETWORK=sui
RAMP_DEFAULT_CRYPTO_CURRENCY=USDC
RAMP_DEFAULT_FIAT_CURRENCY=USD
RAMP_PARTNER_FEE_BPS=0
PORT=3000
RUST_LOG=wager_api=debug,tower_http=debug
```

## Run Locally

```bash
cd app
cargo run
```

Health check:

```bash
curl http://localhost:3000/health
```

## Authentication

```http
POST /api/auth/verify
Content-Type: application/json

{
  "dynamic_token": "<jwt-from-dynamic-sdk>"
}
```

The response returns `accessToken`, which should be sent as:

```http
Authorization: Bearer <accessToken>
```

## Sui Direction

The on-chain layer is a Sui Move tournament staking package. The backend syncs match metadata, verifies Dynamic users, indexes Sui events, and exposes app-friendly views. Funds should move through Sui PTBs and Move objects rather than backend custody.

## Kombat Smart Pay

Kombat Smart Pay turns a user action into a programmable payment intent:

- Create an intent for `STAKE_TOURNAMENT`.
- Check Sui USDC balance and calculate the exact funding shortfall.
- Preserve an optional wallet reserve with `reserve_balance_usdc`.
- On-ramp only the shortfall through Dynamic native funding.
- Return a frontend-readable PTB plan for `tournament_staking::stake<USDC>`, which locks funds and mints a `StakeReceipt`.

Endpoints:

- `POST /api/payments/intents`
- `GET /api/payments/intents/:id`
- `POST /api/payments/intents/:id/onramp-session`
- `GET /api/payments/intents/:id/ptb`

## Receipt Market

Kombat supports trade in/out before settlement through a receipt market. A seller lists their `StakeReceipt`; a buyer pays the ask in USDC and receives the receipt atomically on Sui. The pool itself is not unwound early.

Endpoints:

- `POST /api/receipt-market/listings`
- `GET /api/receipt-market/listings`
- `GET /api/receipt-market/listings/:id`
- `POST /api/receipt-market/listings/:id/activate`
- `GET /api/receipt-market/listings/:id/list-ptb`
- `GET /api/receipt-market/listings/:id/buy-ptb`
- `POST /api/receipt-market/listings/:id/mark-sold`

## Notifications

Kombat stores wallet-scoped notifications and broadcasts the same records over SSE/WebSocket. Every transactional notification includes `payload.title`, `payload.body`, `payload.action`, and `payload.entities` so the frontend can render a CTA without hard-coding the next endpoint.

Endpoints:

- `GET /api/notifications/:wallet`
- `POST /api/notifications/:id/read`
- `GET /notifications/stream/:wallet`
- `GET /ws/notifications/:wallet`

All notification endpoints require `Authorization: Bearer <accessToken>`.

## Sui Endpoints

- `GET /api/sui/config` returns the active network plus testnet/mainnet RPC, package, USDC, and staking module config.
- `GET /api/sui/health` checks the active Sui RPC and returns the chain identifier plus reference gas price.
- `GET /api/sui/wallets/:wallet/balances` returns all Sui coin balances for a wallet on the active network.
- `GET /api/sui/wallets/:wallet/usdc-balance` returns the wallet's USDC balance on the active network.
- `GET /api/sui/wallets/:wallet/dashboard` returns the Wallet screen view model: available USDC, locked Kombat stake amount, transaction history, and action metadata.
- `GET /api/sui/networks/:network/config` returns config for `testnet` or `mainnet`.
- `GET /api/sui/networks/:network/health` checks `testnet` or `mainnet`.
- `GET /api/sui/networks/:network/wallets/:wallet/balances` returns balances on `testnet` or `mainnet`.
- `GET /api/sui/networks/:network/wallets/:wallet/usdc-balance` returns the wallet's USDC balance on `testnet` or `mainnet`.
- `GET /api/sui/networks/:network/wallets/:wallet/dashboard` returns the Wallet screen view model for `testnet` or `mainnet`.

## Tournament Data

Tournament data is ingested server-side from PandaScore and stored in Kombat's database. The frontend should use Kombat tournament endpoints only.

PandaScore config:

- `PANDASCORE_ENABLED=true`
- `PANDASCORE_API_KEY=<server-side key>`
- `PANDASCORE_BASE_URL=https://api.pandascore.co`
- `PANDASCORE_DEFAULT_STATUSES=upcoming,running,past`
- `PANDASCORE_VIDEOGAME_SLUGS=csgo,dota2,lol` optional
- `PANDASCORE_PER_PAGE=50`

Endpoints:

- `GET /api/tournaments/source/pandascore`
- `POST /api/tournaments/source/pandascore/sync` with `X-Admin-Token`

For events outside PandaScore coverage, organizers can create tournaments and stakeable matches directly:

- `POST /api/organizers/apply`
- `POST /api/organizers/kyc-session`
- `GET /api/organizers/:wallet` with organizer JWT or `X-Admin-Token`
- `POST /api/organizers/:wallet/review` with `X-Admin-Token`
- `GET /api/admin/organizers` with `X-Admin-Token`
- `GET /api/admin/organizers/:wallet` with `X-Admin-Token`
- `GET /api/organizer/tournaments`
- `POST /api/organizer/tournaments`
- `POST /api/organizer/tournaments/:id/matches`

Organizer-created matches require the organizer wallet to have `status = approved` and `kyc_status = verified`. They appear in the normal `GET /api/tournaments` response with `source = organizer`. Rules, brackets, and result evidence can reference Walrus blobs through `rules_blob_id`, `bracket_blob_id`, and `evidence_blob_id`.

Outcome proposals support organizer or agent-driven result verification:

- `GET /api/tournaments/:id/outcome-proposals`
- `POST /api/tournaments/:id/outcome-proposals`
- `GET /api/admin/outcome-proposals` with `X-Admin-Token`
- `GET /api/admin/outcome-proposals/:id` with `X-Admin-Token`
- `POST /api/outcome-proposals/:id/review` with `X-Admin-Token`

## Walrus And Agents

Walrus is used for durable public evidence, rules, brackets, and agent reports. Do not upload private KYC documents or sensitive user data to these endpoints unless a privacy layer such as Seal is added.

Config:

- `WALRUS_ENABLED=false`
- `WALRUS_NETWORK=testnet`
- `WALRUS_PUBLISHER_URL=<publisher URL>`
- `WALRUS_AGGREGATOR_URL=<aggregator URL>`
- `WALRUS_EPOCHS=5`
- `AGENT_API_TOKEN=<server-side agent token>`

Endpoints:

- `GET /api/walrus/config`
- `POST /api/walrus/artifacts` with a user JWT, `X-Admin-Token`, or `X-Agent-Token`
- `GET /api/walrus/artifacts/:id`
- `GET /api/walrus/blobs/:blob_id/url`
- `POST /api/agents/outcome-proposals` with `X-Agent-Token`
- `GET /api/admin/agent-runs` with `X-Admin-Token`
- `GET /api/admin/agent-runs/:id` with `X-Admin-Token`

Agent outcome submissions create normal `source = agent` outcome proposals. Admin review still decides whether the match settles.

## Funding / On-Ramp Endpoints

- `GET /api/ramps/providers` returns available funding providers. Dynamic native is the default primary provider.
- `POST /api/ramps/session` returns the frontend action needed to launch Dynamic's native funding flow. Requires `Authorization: Bearer <accessToken>` and a `wallet_address` matching the authenticated Dynamic wallet.

## Transak Fallback Endpoints

- `GET /api/transak/config` returns on-ramp provider config and enabled status.
- `POST /api/transak/quote` returns a live Transak quote. Requires `Authorization: Bearer <accessToken>`.
- `POST /api/transak/widget-url` returns a locked Transak widget URL for the authenticated user's Sui wallet. Requires `Authorization: Bearer <accessToken>`.
