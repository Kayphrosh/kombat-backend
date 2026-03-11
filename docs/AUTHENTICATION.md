# Frontend Authentication Guide

> Base URL (Production): `https://kombat-backend-production.up.railway.app`

This document covers everything the frontend needs to authenticate users and make authorized API calls.

---

## Overview

The backend supports two authentication flows. Both produce the same result: an **HS256 JWT** containing the user's wallet address, which you attach to all protected requests.

| Flow                     | When to use                                    | Endpoint                                        |
| ------------------------ | ---------------------------------------------- | ----------------------------------------------- |
| **Dynamic SDK**          | Mobile / web apps using the Dynamic wallet SDK | `POST /api/auth/verify`                         |
| **Wallet Nonce Signing** | MWA or any direct wallet adapter flow          | `GET /auth/nonce/:wallet` → `POST /auth/verify` |

The issued JWT **expires in 15 minutes**. You must re-authenticate before it expires.

---

## Flow 1 — Dynamic SDK (Recommended)

Use this when your app signs users in through the [Dynamic](https://dynamic.xyz) SDK.

### Steps

1. **Sign the user in** with the Dynamic SDK on the client side.
2. **Get the Dynamic JWT** from the SDK (e.g. `authToken` from `useDynamicContext()`).
3. **Exchange it for an app JWT**:

```
POST /api/auth/verify
Content-Type: application/json

{
  "dynamic_token": "<jwt-from-dynamic-sdk>"
}
```

### Response

```json
{
  "success": true,
  "data": {
    "user": {
      "id": "uuid",
      "wallet_address": "So1anaWa11etAddress...",
      "display_name": "Player1",
      "avatar_url": null,
      "created_at": "2026-01-15T12:00:00Z",
      "updated_at": "2026-01-15T12:00:00Z"
    },
    "accessToken": "<app-jwt>"
  },
  "error": null
}
```

4. **Store `accessToken`** — this is your app JWT for all subsequent requests.

### Error Responses

| Status | Meaning                                                         |
| ------ | --------------------------------------------------------------- |
| `400`  | Missing or malformed `dynamic_token`                            |
| `401`  | Dynamic token is invalid, expired, or contains no Solana wallet |
| `500`  | Server-side config issue (Dynamic not configured)               |

---

## Flow 2 — Wallet Nonce Signing

Use this when users connect directly with a Solana wallet (Phantom, Solflare, MWA, etc.) without the Dynamic SDK.

### Step 1 — Request a nonce

```
GET /auth/nonce/{wallet_address}
```

**Response:**

```json
{
  "success": true,
  "data": {
    "nonce": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
    "expires_at": "2026-03-08T12:05:00Z"
  },
  "error": null
}
```

- The nonce is a UUID, valid for **5 minutes**, single-use.
- Rate limited to **5 requests per 60 seconds** per wallet address. Exceeding this returns `429`.

### Step 2 — Sign the nonce with the wallet

The user's wallet signs the **raw nonce string** as a message (Ed25519 signature).

```javascript
// Using @solana/wallet-adapter
const message = new TextEncoder().encode(nonce);
const signatureBytes = await wallet.signMessage(message);
```

### Step 3 — Verify and get a JWT

```
POST /auth/verify
Content-Type: application/json

{
  "wallet": "<wallet_address>",
  "signature": "<base64-encoded-signature>"
}
```

**Encoding the signature as base64:**

```javascript
const sigBase64 = btoa(String.fromCharCode(...signatureBytes));
```

The backend also accepts the signature as a JSON byte array (e.g. `[12, 34, 56, ...]`), but **base64 is preferred**.

**Response:**

```json
{
  "success": true,
  "data": {
    "token": "<app-jwt>",
    "expires_at": 1741435500
  },
  "error": null
}
```

### Error Responses

| Status | Meaning                                                     |
| ------ | ----------------------------------------------------------- |
| `400`  | Missing fields or malformed signature                       |
| `401`  | Signature verification failed or nonce expired/already used |
| `404`  | No valid nonce found for this wallet (request a new one)    |
| `429`  | Nonce rate limit exceeded (5 per 60s)                       |

---

## Using the JWT

Attach the token to all authenticated requests via the `Authorization` header:

```
Authorization: Bearer <app-jwt>
```

### JWT Contents

The token contains two claims:

| Claim    | Type   | Description                               |
| -------- | ------ | ----------------------------------------- |
| `wallet` | string | The user's Solana wallet address (base58) |
| `exp`    | number | Unix timestamp when the token expires     |

### Token Lifetime

- **15 minutes** from issue time.
- There is no refresh token — re-authenticate using either flow when the token expires.

### Handling Expiry

Check `expires_at` from the auth response and re-authenticate before it lapses. If a request returns `401`, assume the token expired and re-run the auth flow.

```javascript
// Example: simple expiry check
function isTokenExpired(expiresAt) {
  return Date.now() / 1000 >= expiresAt - 30; // 30s buffer
}
```

---

## Which Endpoints Require Auth?

### Public (no token needed)

| Method | Path                             | Description                  |
| ------ | -------------------------------- | ---------------------------- |
| `GET`  | `/health`                        | Health check                 |
| `GET`  | `/auth/nonce/:wallet`            | Get nonce                    |
| `POST` | `/auth/verify`                   | Verify wallet signature      |
| `POST` | `/api/auth/verify`               | Verify Dynamic token         |
| `GET`  | `/api/tournaments`               | List tournaments             |
| `GET`  | `/api/tournaments/:id`           | Get tournament details       |
| `GET`  | `/api/tournaments/:id/stakes`    | List stakes for a tournament |
| `POST` | `/api/tournaments/:id/calculate` | Calculate payout             |
| `GET`  | `/api/kombats`                   | List kombats                 |

### Authenticated (JWT required)

| Method | Path                            | Description               |
| ------ | ------------------------------- | ------------------------- |
| `POST` | `/api/tournaments`              | Create tournament         |
| `POST` | `/api/tournaments/:id/stake`    | Place a stake             |
| `POST` | `/api/uploads`                  | Upload a file             |
| `GET`  | `/notifications/stream/:wallet` | SSE notification stream\* |

\*The notification stream also accepts the token as a query parameter (`?token=<jwt>`) for SSE/WebSocket clients that can't set headers.

### Admin only (server-to-server)

| Method | Path                           | Description               |
| ------ | ------------------------------ | ------------------------- |
| `POST` | `/auth/token`                  | Mint a JWT for any wallet |
| `POST` | `/api/tournaments/:id/resolve` | Resolve tournament        |
| `POST` | `/api/tournaments/:id/cancel`  | Cancel tournament         |
| `POST` | `/api/tournaments/:id/sync`    | Sync tournament on-chain  |

---

## Full Example — Dynamic SDK (React Native)

```javascript
import { useDynamicContext } from '@dynamic-labs/sdk-react-native';

const API = 'https://kombat-backend-production.up.railway.app';

async function login() {
  const { authToken } = useDynamicContext();

  const res = await fetch(`${API}/api/auth/verify`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ dynamic_token: authToken }),
  });

  const { data } = await res.json();
  // data.accessToken → store securely
  // data.user → user profile
  return data;
}
```

## Full Example — Wallet Nonce Signing

```javascript
const API = 'https://kombat-backend-production.up.railway.app';

async function loginWithWallet(wallet) {
  // 1. Get nonce
  const nonceRes = await fetch(`${API}/auth/nonce/${wallet.publicKey}`);
  const {
    data: { nonce },
  } = await nonceRes.json();

  // 2. Sign nonce
  const message = new TextEncoder().encode(nonce);
  const signatureBytes = await wallet.signMessage(message);
  const sigBase64 = btoa(String.fromCharCode(...signatureBytes));

  // 3. Exchange for JWT
  const verifyRes = await fetch(`${API}/auth/verify`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      wallet: wallet.publicKey.toBase58(),
      signature: sigBase64,
    }),
  });

  const {
    data: { token, expires_at },
  } = await verifyRes.json();
  // token → store securely
  // expires_at → track for refresh
  return { token, expires_at };
}
```

## Making Authenticated Requests

```javascript
async function placeTournamentStake(token, tournamentId, body) {
  const res = await fetch(`${API}/api/tournaments/${tournamentId}/stake`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      Authorization: `Bearer ${token}`,
    },
    body: JSON.stringify(body),
  });
  return res.json();
}
```

---

## Quick Reference

| Item                 | Value                                           |
| -------------------- | ----------------------------------------------- |
| JWT algorithm        | HS256                                           |
| JWT lifetime         | 15 minutes                                      |
| Nonce lifetime       | 5 minutes                                       |
| Nonce rate limit     | 5 per 60 seconds per wallet                     |
| Auth header format   | `Authorization: Bearer <token>`                 |
| Dynamic SDK endpoint | `POST /api/auth/verify`                         |
| Nonce flow endpoints | `GET /auth/nonce/:wallet` → `POST /auth/verify` |
