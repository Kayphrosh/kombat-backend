# Authentication

Kombat now uses **Dynamic embedded wallets only** for user authentication.

Native wallet nonce signing has been removed. The backend no longer exposes:

- `GET /auth/nonce/:wallet`
- `POST /auth/verify`
- Solana RPC-backed authentication

## Dynamic Flow

1. The client signs the user in with Dynamic.
2. Dynamic creates or links the user's Sui embedded wallet.
3. The client sends the Dynamic JWT to Kombat.
4. Kombat verifies the token with Dynamic JWKS.
5. Kombat extracts the Sui wallet address and mints an app JWT.

```http
POST /api/auth/verify
Content-Type: application/json

{
  "dynamic_token": "<jwt-from-dynamic-sdk>"
}
```

Response:

```json
{
  "user": {
    "id": "uuid",
    "wallet_address": "0x...",
    "email": "user@example.com",
    "display_name": "user@example.com",
    "avatar_url": null,
    "wins": 0,
    "losses": 0,
    "created_at": "2026-06-14T00:00:00Z",
    "updated_at": "2026-06-14T00:00:00Z"
  },
  "accessToken": "<kombat-jwt>"
}
```

Use the returned app JWT as:

```http
Authorization: Bearer <kombat-jwt>
```

## Required Environment

```env
AUTH_JWT_SECRET=<long-random-secret>
DYNAMIC_ENVIRONMENT_ID=<dynamic-environment-id>
SUI_NETWORK=testnet
SUI_TESTNET_RPC_URL=https://fullnode.testnet.sui.io:443
SUI_MAINNET_RPC_URL=https://fullnode.mainnet.sui.io:443
```

## Wallet Address

The backend expects Dynamic to provide a Sui wallet credential. Sui wallet addresses are stored as canonical `0x`-prefixed hex strings.
