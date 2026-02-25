# Kombat Backend — API Documentation

> **Base URL:** `https://kombat-backend-production.up.railway.app`
> All responses use the format `{ "success": true|false, "data": ..., "error": "..." }`

---

## Authentication

### `POST /api/auth/verify-dynamic`

Verifies a Dynamic SDK JWT and returns an app-level JWT. Creates the user on first login.

**Request:**

```json
{
  "token": "<Dynamic SDK JWT token>"
}
```

**Response:**

```json
{
  "success": true,
  "data": {
    "token": "eyJhbGciOiJIUzI1...",
    "user": {
      "id": "95534631-d9a1-4156-8ea4-002e2a405aa9",
      "wallet_address": "8AVLybVb...",
      "email": "user@example.com",
      "display_name": "Kayphrosh",
      "avatar_url": "https://...",
      "wins": 0,
      "losses": 0,
      "created_at": "2026-02-23T23:06:46.751942Z",
      "updated_at": "2026-02-25T10:57:25.668443Z"
    }
  }
}
```

### `POST /api/auth/verify`

Verifies a signed nonce (MWA/wallet adapter flow).

**Request:**

```json
{
  "wallet": "8AVLybVb...",
  "signature": "<base58 signature>",
  "nonce": "abc123"
}
```

---

## User Profile

### `GET /api/users/:wallet`

**Tested ✅** — Returns the user's profile.

**Example:** `GET /api/users/8AVLybVbDhmtxxNCBmbSWUK4SinhD1iKzkspbTBoRQBU`

**Response:**

```json
{
  "success": true,
  "data": {
    "id": "95534631-d9a1-4156-8ea4-002e2a405aa9",
    "wallet_address": "8AVLybVbDhmtxxNCBmbSWUK4SinhD1iKzkspbTBoRQBU",
    "email": "olakunbiolabode01@gmail.com",
    "display_name": "Kayphrosh",
    "avatar_url": "https://...",
    "wins": 0,
    "losses": 0,
    "created_at": "2026-02-23T23:06:46.751942Z",
    "updated_at": "2026-02-25T10:57:25.668443Z"
  }
}
```

### `POST /api/users/:wallet`

Updates the user profile. All fields are optional — only sends what changed.

**Request:**

```json
{
  "display_name": "Kayphrosh",
  "avatar_url": "https://...",
  "email": "user@example.com"
}
```

### `GET /api/users/:wallet/stats`

**Tested ✅** — Returns wager statistics for the home screen dashboard.

**Response:**

```json
{
  "success": true,
  "data": {
    "live_count": 4,
    "completed_count": 2,
    "total_stake": 600000000,
    "total_won": 0
  }
}
```

> [!TIP]
> `total_stake` and `total_won` are in **lamports** (1 SOL = 1,000,000,000 lamports). Divide by `1e9` for SOL display.

---

## Kombats (Wagers)

### `POST /api/kombats` — Create a Kombat

**Tested ✅** — Creates a new wager and returns an unsigned Solana transaction for the initiator to sign.

**Request:**

```json
{
  "initiator": "8AVLybVbDhmtxxNCBmbSWUK4SinhD1iKzkspbTBoRQBU",
  "stake_lamports": 200000000,
  "description": "Chelsea would win 2025/2026 Premiere League by June 2026",
  "expiry_ts": 1772013000,
  "resolution_source": "arbitrator",
  "resolver": "8AVLybVbDhmtxxNCBmbSWUK4SinhD1iKzkspbTBoRQBU",
  "challenger_address": "5rCKS5bwiBa7v3WAdL3jntNWw7zphjXsS8vU8JJW3T9t",
  "initiator_option": "yes"
}
```

| Field                | Type   | Required | Description                                      |
| -------------------- | ------ | -------- | ------------------------------------------------ |
| `initiator`          | string | ✅       | Wallet address of the creator                    |
| `stake_lamports`     | number | ✅       | Stake amount in lamports                         |
| `description`        | string | ✅       | Kombat description (max 256 chars)               |
| `expiry_ts`          | number | ✅       | Unix timestamp for deadline                      |
| `resolution_source`  | string | ✅       | `"arbitrator"`, `"oracle"`, or `"mutual"`        |
| `resolver`           | string | ✅       | Wallet that resolves (usually initiator)         |
| `challenger_address` | string | ❌       | Wallet of the challenged user                    |
| `initiator_option`   | string | ❌       | `"yes"` or `"no"` — the side the initiator picks |

**Response:**

```json
{
  "success": true,
  "data": {
    "transaction_b64": "AQAAAA...",
    "description": "Create wager: 'Chelsea would win...' for 200000000 lamports"
  }
}
```

> [!IMPORTANT]
> The frontend must **deserialize** the base64 transaction, have the user **sign** it with their wallet, and then **send** it to the Solana network. The wager is stored in the DB immediately as `"pending"`.

---

### `GET /api/kombats` — List Kombats

**Tested ✅** — Returns enriched wager list with participant names and avatars.

**Query Parameters:**

| Param        | Type   | Default | Description                                                                |
| ------------ | ------ | ------- | -------------------------------------------------------------------------- |
| `initiator`  | string | —       | Filter by initiator wallet                                                 |
| `challenger` | string | —       | Filter by challenger wallet                                                |
| `status`     | string | —       | Filter by status: `pending`, `active`, `declined`, `resolved`, `cancelled` |
| `limit`      | number | 20      | Max results (cap: 100)                                                     |
| `offset`     | number | 0       | Pagination offset                                                          |

**Example:** `GET /api/kombats?initiator=8AVLybVb...&status=pending&limit=10`

**Response:**

```json
{
  "success": true,
  "data": [
    {
      "id": "57ce3093-b9fa-4d1f-9db5-6c69321728b9",
      "on_chain_address": "CXh95M67wtgSRDzoY6Ksf8gRrRf3n9BsCgU1sQaz6Tff",
      "wager_id": 3,
      "initiator": "8AVLybVbDhmtxxNCBmbSWUK4SinhD1iKzkspbTBoRQBU",
      "challenger": "5rCKS5bwiBa7v3WAdL3jntNWw7zphjXsS8vU8JJW3T9t",
      "stake_lamports": 200000000,
      "description": "Testing testing testiing",
      "status": "pending",
      "resolution_source": "arbitrator",
      "resolver": "8AVLybVbDhmtxxNCBmbSWUK4SinhD1iKzkspbTBoRQBU",
      "expiry_ts": 1772013000,
      "created_at": "2026-02-25T09:38:48.905527Z",
      "resolved_at": null,
      "winner": null,
      "protocol_fee_bps": 100,
      "oracle_feed": null,
      "oracle_target": null,
      "dispute_opened_at": null,
      "dispute_opener": null,
      "initiator_option": "yes",
      "initiator_name": "Kayphrosh",
      "initiator_avatar": "https://...",
      "challenger_name": "kayyyyy",
      "challenger_avatar": null,
      "challenger_option": "no"
    }
  ]
}
```

> [!TIP]
> **For the Home Screen "vs Kendrick" cards**, use `challenger_name` (or `initiator_name` if the current user is the challenger). Compute the countdown from `expiry_ts - now()`.

---

### `GET /api/kombats/:address` — Kombat Detail

**Tested ✅** — Returns a single wager with enriched participant info.

**Example:** `GET /api/kombats/CXh95M67wtgSRDzoY6Ksf8gRrRf3n9BsCgU1sQaz6Tff`

**Response:** Same shape as a single item in the list response above.

**UI Mapping:**

| UI Element         | Field                                                         |
| ------------------ | ------------------------------------------------------------- |
| Kombat Title       | `description`                                                 |
| Kombat ID          | `on_chain_address` (truncate for display)                     |
| Total Stake        | `stake_lamports * 2` (both sides combined)                    |
| Status badge       | `status` → `"pending"` / `"active"` / `"resolved"`            |
| Deadline countdown | `expiry_ts` (unix) — compute `expiry_ts - Date.now()/1000`    |
| Date Created       | `created_at`                                                  |
| YOU side           | `initiator_option` / `initiator_name` / `initiator_avatar`    |
| Opponent side      | `challenger_option` / `challenger_name` / `challenger_avatar` |

---

### `POST /api/kombats/:address/accept` — Accept a Kombat

Returns an unsigned transaction for the **challenger** to sign.

**Request:**

```json
{
  "challenger": "5rCKS5bwiBa7v3WAdL3jntNWw7zphjXsS8vU8JJW3T9t"
}
```

**Response:**

```json
{
  "success": true,
  "data": {
    "transaction_b64": "AQAAAA...",
    "description": "Accept wager #3"
  }
}
```

---

### `POST /api/kombats/:address/decline` — Decline a Kombat

**Fixed ✅** — Now accepts `{ challenger }` in the body (the wallet declining).

**Request:**

```json
{
  "challenger": "5rCKS5bwiBa7v3WAdL3jntNWw7zphjXsS8vU8JJW3T9t"
}
```

**Response:**

```json
{
  "success": true,
  "data": {
    "transaction_b64": "AQAAAA...",
    "description": "Decline wager #3"
  }
}
```

---

### `POST /api/kombats/:address/cancel` — Cancel a Kombat

Only the **initiator** can cancel. No body required.

**Response:**

```json
{
  "success": true,
  "data": {
    "transaction_b64": "AQAAAA...",
    "description": "Cancel wager #3"
  }
}
```

---

### `POST /api/kombats/:address/declare-winner` — Declare Winner (Consent)

**Request:**

```json
{
  "wallet": "8AVLybVbDhmtxxNCBmbSWUK4SinhD1iKzkspbTBoRQBU",
  "winner": "8AVLybVbDhmtxxNCBmbSWUK4SinhD1iKzkspbTBoRQBU"
}
```

---

## Notifications

### `GET /api/notifications/:wallet` — List Notifications

**Tested ✅** — Returns notifications for a user.

**Query:** `?limit=50&offset=0`

**Response:**

```json
{
  "success": true,
  "data": [
    {
      "id": "f5464b60-33ea-4906-a623-636167385549",
      "user_wallet": "8AVLybVbDhmtxxNCBmbSWUK4SinhD1iKzkspbTBoRQBU",
      "kind": "wager_challenge",
      "payload": {
        "description": "Chelsea would win...",
        "initiator": "5rCKS5bwiBa7v3WAdL3jntNWw7zphjXsS8vU8JJW3T9t",
        "stake_lamports": 200000000,
        "wager_address": "8XXmoEwN8fdF8Tg7onBKyCn4isgJ6iVB1ruBAc7U5TQG"
      },
      "is_read": false,
      "created_at": "2026-02-24T20:46:29.016370Z"
    }
  ]
}
```

**Notification `kind` values:**

| Kind              | Description                   | Icon |
| ----------------- | ----------------------------- | ---- |
| `wager_challenge` | Someone challenged you        | ⚔️   |
| `wager_accepted`  | Opponent accepted your Kombat | ✅   |
| `wager_resolved`  | Kombat has been resolved      | 🏆   |
| `wager_cancelled` | Kombat was cancelled          | ❌   |
| `fund_wallet`     | Wallet funded                 | ↑    |
| `withdrawal`      | Withdrawal made               | ↓    |

### `POST /api/notifications/:id/read` — Mark as Read

**Tested ✅** — Marks a notification as read. No body required.

**Example:** `POST /api/notifications/f5464b60-33ea-4906-a623-636167385549/read`

**Response:**

```json
{
  "success": true,
  "data": null
}
```

---

## Push Tokens

### `POST /api/users/:wallet/push-token` — Register Expo Push Token

Call this on app launch after receiving notification permissions.

**Request:**

```json
{
  "expo_token": "ExponentPushToken[xxxxxxxxxxxxxxxxxxxxxx]"
}
```

**Response:**

```json
{
  "success": true,
  "data": null
}
```

---

## Notification Settings

### `GET /api/users/:wallet/notification-settings`

**Response:**

```json
{
  "success": true,
  "data": {
    "challenges": true,
    "funds": true,
    "disputes": true,
    "marketing": false
  }
}
```

### `PUT /api/users/:wallet/notification-settings`

**Request:**

```json
{
  "challenges": true,
  "funds": true,
  "disputes": true,
  "marketing": false
}
```

---

## Transaction Flow (Frontend)

All wager mutation endpoints return a `transaction_b64` field. Here's how to handle it:

```typescript
// 1. Call the backend endpoint
const res = await fetch(`${API_URL}/api/kombats`, {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({
    initiator: walletAddress,
    stake_lamports: 200000000,
    description: 'Chelsea wins the league',
    expiry_ts: Math.floor(Date.now() / 1000) + 86400 * 7,
    resolution_source: 'arbitrator',
    resolver: walletAddress,
    challenger_address: opponentWallet,
    initiator_option: 'yes',
  }),
});
const { data } = await res.json();

// 2. Deserialize the transaction
const txBytes = Buffer.from(data.transaction_b64, 'base64');
const transaction = Transaction.from(txBytes);

// 3. Sign with the user's wallet
const signedTx = await wallet.signTransaction(transaction);

// 4. Send to Solana
const sig = await connection.sendRawTransaction(signedTx.serialize());
await connection.confirmTransaction(sig);
```
