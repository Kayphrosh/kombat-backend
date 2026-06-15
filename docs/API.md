# API

This API is now Dynamic/Sui-oriented. Native wallet nonce auth and Solana transaction-building endpoints have been removed.

## Auth

### `POST /api/auth/verify`

Verifies a Dynamic JWT and returns a Kombat app JWT.

Request:

```json
{
  "dynamic_token": "<jwt-from-dynamic-sdk>"
}
```

Response:

```json
{
  "user": {
    "wallet_address": "0x..."
  },
  "accessToken": "<kombat-jwt>"
}
```

## Tournaments

- `GET /api/tournaments`
- `POST /api/tournaments`
- `GET /api/tournaments/:id`
- `POST /api/tournaments/:id/stake`
- `POST /api/tournaments/:id/calculate`
- `GET /api/tournaments/:id/stakes`
- `POST /api/tournaments/:id/resolve`
- `POST /api/tournaments/:id/cancel`
- `POST /api/tournaments/:id/sync`

Tournament pool persistence remains in Postgres until the Sui Move staking package is added.

## Programmable Payments

### `POST /api/payments/intents`

Requires `Authorization: Bearer <accessToken>`. Creates a Smart Pay intent for tournament staking and calculates whether the user needs to on-ramp first.

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

The reserve rule preserves wallet balance after staking. In this example, the backend requires enough USDC for a 25 USDC stake plus a 5 USDC reserve.

### `GET /api/payments/intents/:id`

Refreshes the intent, current Sui USDC balance, required balance, and funding shortfall.

### `POST /api/payments/intents/:id/onramp-session`

Returns `client_action: "open_dynamic_onramp"` only for the intent's shortfall. If the wallet is already funded, `onramp_required` is `false` and `ramp_session` is `null`.

### `GET /api/payments/intents/:id/ptb`

Returns a frontend-readable Sui PTB plan for `tournament_staking::stake<USDC>`. `can_build` is false until the intent is funded and the Sui package, USDC coin type, and match `sui_pool_object_id` are configured.

The PTB plan points the frontend at the Move call that locks funds and mints a `StakeReceipt`.

## Receipt Market

The receipt market enables trade in/out before settlement. Sellers list a `StakeReceipt`; buyers pay the ask and receive that receipt atomically on Sui.

### `POST /api/receipt-market/listings`

Requires `Authorization: Bearer <accessToken>`.

```json
{
  "wallet_address": "0x...",
  "receipt_id": "0x...",
  "match_id": "match-uuid",
  "opponent_id": "opponent-uuid",
  "ask_amount_usdc": 18000000,
  "network": "testnet"
}
```

Creates a draft listing. Fetch `list-ptb` next so the seller can escrow the receipt on-chain.

### `GET /api/receipt-market/listings`

Lists draft/active receipt listings. Supports `match_id`, `seller_wallet`, `status`, `limit`, and `offset`.

### `GET /api/receipt-market/listings/:id/list-ptb`

Returns a PTB plan for `tournament_staking::list_receipt<USDC>`. The seller signs this to share an on-chain `ReceiptListing` object.

### `POST /api/receipt-market/listings/:id/activate`

Requires the seller wallet. Stores the shared listing object ID after the list transaction succeeds.

```json
{
  "wallet_address": "0x...",
  "listing_object_id": "0x...",
  "listing_tx_hash": "0x..."
}
```

### `GET /api/receipt-market/listings/:id/buy-ptb`

Returns a PTB plan for `tournament_staking::buy_receipt<USDC>`. The buyer pays exact USDC ask, receives the `StakeReceipt`, and becomes the receipt owner for claim/refund.

## Sui

### `GET /api/sui/config`

Returns the active Sui network plus the configured `testnet` and `mainnet` RPC/package/USDC settings.

### `GET /api/sui/health`

Checks the active Sui RPC and returns chain metadata.

### `GET /api/sui/wallets/:wallet/balances`

Returns all coin balances for a Sui wallet on the active network.

### `GET /api/sui/wallets/:wallet/usdc-balance`

Returns the wallet's USDC balance on the active network.

### `GET /api/sui/wallets/:wallet/dashboard`

Returns the Wallet screen view model for the active network.

```json
{
  "network": "testnet",
  "wallet": "0x...",
  "usdc_coin_type": "0x...::usdc::USDC",
  "available_balance_usdc": 250000000,
  "locked_in_kombats_usdc": 300000000,
  "total_balance_usdc": 550000000,
  "transaction_history": [
    {
      "id": "stake-id:resolution",
      "kind": "won",
      "title": "Won · Madrid wins El Clasico",
      "subtitle": "Madrid",
      "amount_usdc": 250000000,
      "direction": "in",
      "status": "won",
      "tx_hash": "0x...",
      "occurred_at": "2026-06-14T12:00:00Z"
    }
  ],
  "actions": {
    "fund_wallet": {
      "enabled": true,
      "provider": "dynamic_native",
      "requires_frontend_wallet": true
    },
    "withdraw": {
      "enabled": false,
      "provider": "not_supported",
      "requires_frontend_wallet": true
    }
  }
}
```

### `GET /api/sui/networks/:network/config`

Returns config for `testnet` or `mainnet`.

### `GET /api/sui/networks/:network/health`

Checks `testnet` or `mainnet` and returns chain metadata.

### `GET /api/sui/networks/:network/wallets/:wallet/balances`

Returns all coin balances for a Sui wallet on `testnet` or `mainnet`.

### `GET /api/sui/networks/:network/wallets/:wallet/usdc-balance`

Returns the wallet's USDC balance on `testnet` or `mainnet`.

### `GET /api/sui/networks/:network/wallets/:wallet/dashboard`

Returns the Wallet screen view model for `testnet` or `mainnet`. Supports `limit` and `offset` query params for transaction history pagination.

## Funding / On-Ramp Provider Router

### `GET /api/ramps/providers`

Returns enabled funding options.

```json
{
  "primary_provider": "dynamic_native",
  "default_network": "sui",
  "default_crypto_currency": "USDC",
  "default_fiat_currency": "USD",
  "partner_fee_bps": 0,
  "country": "NG",
  "providers": [
    {
      "provider": "dynamic_native",
      "label": "Card / bank transfer",
      "kind": "onramp",
      "enabled": true,
      "reason": null,
      "launch_method": "dynamic_sdk"
    },
    {
      "provider": "manual_crypto_deposit",
      "label": "Deposit crypto",
      "kind": "deposit",
      "enabled": true,
      "reason": null,
      "launch_method": "copy_wallet_address"
    }
  ]
}
```

### `POST /api/ramps/session`

Requires `Authorization: Bearer <accessToken>`. The requested `wallet_address` must match the wallet in the Kombat JWT.

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

Returns a frontend action:

```json
{
  "provider": "dynamic_native",
  "product": "BUY",
  "wallet_address": "0x...",
  "launch_method": "dynamic_sdk",
  "client_action": "open_dynamic_onramp",
  "network": "sui",
  "crypto_currency_code": "USDC",
  "fiat_currency": "USD",
  "fiat_amount": 50,
  "crypto_amount": null,
  "note": "Launch Dynamic's native funding flow in the frontend; provider availability is configured in the Dynamic dashboard."
}
```

`SELL` requests are rejected; Kombat only supports on-ramp sessions.

## Transak Fallback

### `GET /api/transak/config`

Returns Transak enabled status and default on-ramp settings.

### `POST /api/transak/quote`

Requires `Authorization: Bearer <accessToken>`.

```json
{
  "product": "BUY",
  "fiat_currency": "USD",
  "fiat_amount": 50,
  "crypto_currency_code": "USDC",
  "network": "sui",
  "payment_method": "credit_debit_card",
  "country_code": "US"
}
```

Returns the raw Transak quote payload so the frontend can show the exact receive amount and fee breakdown.

### `POST /api/transak/widget-url`

Requires `Authorization: Bearer <accessToken>`. The requested `wallet_address` must match the wallet in the Kombat JWT.

```json
{
  "wallet_address": "0x...",
  "product": "BUY",
  "fiat_currency": "USD",
  "fiat_amount": 50,
  "crypto_currency_code": "USDC",
  "network": "sui",
  "payment_method": "credit_debit_card",
  "redirect_url": "https://app.example.com/wallet"
}
```

Returns:

```json
{
  "provider": "transak",
  "product": "BUY",
  "wallet_address": "0x...",
  "widget_url": "https://global.transak.com?..."
}
```

## Users

- `GET /api/users/search`
- `GET /api/users/:wallet`
- `POST /api/users/:wallet`
- `DELETE /api/users/:wallet`
- `GET /api/users/:wallet/stats`
- `GET /api/users/:wallet/stakes`
- `GET /api/users/:wallet/stake-stats`
- `GET /api/users/:wallet/notification-settings`
- `PUT /api/users/:wallet/notification-settings`
- `POST /api/users/:wallet/push-token`

## Notifications

- `GET /api/notifications/:wallet`
- `POST /api/notifications/:id/read`
- `GET /notifications/stream/:wallet`
- `GET /ws/notifications/:wallet`

Realtime notification endpoints require `Authorization: Bearer <accessToken>`. Do not send JWTs in query strings.
