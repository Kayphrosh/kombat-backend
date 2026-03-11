# Frontend Integration Guide — Kombat Backend API

> **Base URL (Production):** `https://kombat-backend-production.up.railway.app`
> **Program ID:** `Dj2Hot5XJLv9S724BRkWohrhUfzLFERBnZJ9da2WBJQK`
> **Platform Signer (Delegate):** `8Ntpb36UGj4f34zWWhbp3aEt8fD2FMr396NxkGNwuULf`
> **USDC Mint (Devnet):** `4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU`

---

## Table of Contents

1. [Local Development](#local-development)
2. [Response Format](#response-format)
3. [Authentication](#authentication)
4. [Delegation (One-Time Wallet Approval)](#delegation-one-time-wallet-approval)
5. [PandaScore Integration (Frontend Responsibility)](#pandascore-integration-frontend-responsibility)
6. [Tournament Betting (Pool Staking)](#tournament-betting-pool-staking)
7. [Kombats (1v1 Wagers)](#kombats-1v1-wagers)
8. [Users](#users)
9. [User Stakes](#user-stakes)
10. [File Upload](#file-upload)
11. [Notifications](#notifications)
12. [Complete User Flow](#complete-user-flow)

---

## Local Development

```bash
# Start the API (from project root)
cd app && cargo run

# Server runs at http://localhost:3000
```

| Platform         | Base URL                      |
| ---------------- | ----------------------------- |
| iOS Simulator    | `http://localhost:3000`       |
| Android Emulator | `http://10.0.2.2:3000`        |
| Physical device  | `http://<your-local-ip>:3000` |

---

## Response Format

All endpoints return:

```json
{ "success": true, "data": <T>, "error": null }
// or on error:
{ "success": false, "data": null, "error": "message" }
```

HTTP status codes: `200` success, `400` bad request, `401` unauthorized, `404` not found, `500` server error.

---

## Authentication

### Option A: Dynamic SDK (Recommended for Mobile)

```
POST /api/auth/verify
Content-Type: application/json
```

```json
// Request
{ "dynamic_token": "<jwt-from-dynamic-sdk>" }

// Response
{
  "user": { "id": "uuid", "wallet_address": "pubkey", "display_name": "...", ... },
  "accessToken": "<app-jwt>"
}
```

Store the `accessToken` — it's required for all authenticated endpoints as:

```
Authorization: Bearer <accessToken>
```

### Option B: Wallet Nonce Flow (MWA)

```javascript
// 1. Get nonce
const {
  data: { nonce },
} = await fetch(`${API}/auth/nonce/${wallet}`).then((r) => r.json());

// 2. Sign with wallet
const sig = await wallet.signMessage(new TextEncoder().encode(nonce));
const sigBase64 = btoa(String.fromCharCode(...sig));

// 3. Verify → get JWT
const {
  data: { token },
} = await fetch(`${API}/auth/verify`, {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ wallet, signature: sigBase64 }),
}).then((r) => r.json());
```

### Admin Token Mint (server-to-server)

```
POST /auth/token
Header: X-Admin-Token: <AUTH_ADMIN_TOKEN>
Body: { "wallet": "<wallet>", "ttl_seconds": 900 }
→ { "token": "...", "expires_at": <unix_ts> }
```

---

## Delegation (One-Time Wallet Approval)

Delegation allows the backend to move USDC on behalf of a user **without a wallet popup every time they stake**. The user signs a single SPL Token `approve` transaction, and all subsequent stakes are handled server-side via the platform delegate.

### How It Works

1. User calls `GET /api/delegation/approve-tx?wallet=X` → receives an **unsigned** transaction
2. User signs it once in their wallet (Phantom, Solflare, etc.)
3. User submits the signed transaction to Solana
4. Done — all future `place_stake` calls silently transfer USDC without wallet popups
5. User can revoke at any time via `GET /api/delegation/revoke-tx?wallet=X`

### Endpoints

#### Check Delegation Status (No Auth Required)

```
GET /api/delegation/status?wallet=<wallet_pubkey>
```

```json
// Response
{
  "success": true,
  "data": {
    "enabled": true,
    "delegate": "8Ntpb36UGj4f34zWWhbp3aEt8fD2FMr396NxkGNwuULf",
    "delegated_amount": 500000000,
    "token_account": "UserUsdcAtaAddress..."
  }
}
```

| Field              | Type    | Description                                                    |
| ------------------ | ------- | -------------------------------------------------------------- |
| `enabled`          | bool    | Whether delegation service is running on the backend           |
| `delegate`         | string  | Platform signer public key                                     |
| `delegated_amount` | number? | Remaining USDC allowance in micro-USDC (null if no ATA exists) |
| `token_account`    | string? | User's USDC token account address                              |

**Frontend logic:**

- If `delegated_amount > 0` → user has already approved, skip the approval step
- If `delegated_amount === 0` or `null` → show "Approve Delegation" button

#### Get Approve Transaction (Auth Required)

```
GET /api/delegation/approve-tx?wallet=<wallet_pubkey>&amount=<optional_micro_usdc>
Authorization: Bearer <jwt>
```

```json
// Response
{
  "success": true,
  "data": {
    "transaction": "<base64-encoded-unsigned-tx>",
    "delegate": "8Ntpb36UGj4f34zWWhbp3aEt8fD2FMr396NxkGNwuULf",
    "amount": 500000000
  }
}
```

| Param    | Type   | Default       | Description                                      |
| -------- | ------ | ------------- | ------------------------------------------------ |
| `wallet` | string | **required**  | User's wallet public key                         |
| `amount` | number | `500_000_000` | Max allowance in micro-USDC (capped at 500 USDC) |

**Frontend signing flow:**

```typescript
// 1. Get the unsigned approve transaction
const res = await fetch(`${API}/api/delegation/approve-tx?wallet=${wallet}`, {
  headers: { Authorization: `Bearer ${jwt}` },
});
const { data } = await res.json();

// 2. Decode, sign, and submit
const txBytes = Buffer.from(data.transaction, 'base64');
const transaction = Transaction.from(txBytes);
const signed = await wallet.signTransaction(transaction);
const sig = await connection.sendRawTransaction(signed.serialize());
await connection.confirmTransaction(sig);

// 3. Done! All future stakes will work without wallet popups
```

#### Get Revoke Transaction (Auth Required)

```
GET /api/delegation/revoke-tx?wallet=<wallet_pubkey>
Authorization: Bearer <jwt>
```

```json
{
  "success": true,
  "data": {
    "transaction": "<base64-encoded-unsigned-tx>"
  }
}
```

Same signing flow as approve — decode, sign, submit.

---

## PandaScore Integration (Frontend Responsibility)

The backend does **not** call PandaScore directly. The frontend is responsible for fetching match data from the PandaScore API and pushing it to the backend. This keeps the backend stateless with respect to PandaScore and avoids API key management on the server.

### What the Frontend Must Do

#### 1. Fetch Upcoming Matches from PandaScore

Use the PandaScore REST API to get upcoming esports matches:

```typescript
// Example: fetch upcoming CS:GO matches
const matches = await fetch(
  'https://api.pandascore.co/csgo/matches/upcoming?per_page=50',
  { headers: { Authorization: `Bearer ${PANDASCORE_API_KEY}` } },
).then((r) => r.json());
```

PandaScore API docs: `https://developers.pandascore.co/reference`

You need your own PandaScore API key (free tier available).

#### 2. Push Match Data to Backend Before Staking

When a user wants to stake on a match, the frontend must first ensure that match exists in the backend by calling `POST /api/tournaments` with the PandaScore match data:

```typescript
// Map PandaScore match object → backend CreateMatchRequest
const createMatchPayload = {
  pandascore_id: pandascoreMatch.id, // required — unique PandaScore match ID
  slug: pandascoreMatch.slug,
  name: pandascoreMatch.name, // e.g. "Fnatic vs Na'Vi"
  videogame_id: pandascoreMatch.videogame?.id,
  videogame_name: pandascoreMatch.videogame?.name,
  videogame_slug: pandascoreMatch.videogame?.slug,
  league_id: pandascoreMatch.league?.id,
  league_name: pandascoreMatch.league?.name,
  league_slug: pandascoreMatch.league?.slug,
  league_image_url: pandascoreMatch.league?.image_url,
  series_id: pandascoreMatch.serie?.id,
  series_name: pandascoreMatch.serie?.name,
  series_full_name: pandascoreMatch.serie?.full_name,
  tournament_id: pandascoreMatch.tournament?.id,
  tournament_name: pandascoreMatch.tournament?.name,
  tournament_slug: pandascoreMatch.tournament?.slug,
  scheduled_at: pandascoreMatch.scheduled_at, // ISO 8601 string
  match_type: pandascoreMatch.match_type, // e.g. "best_of"
  number_of_games: pandascoreMatch.number_of_games,
  pandascore_status: pandascoreMatch.status, // "not_started", "running", "finished"
  opponents: pandascoreMatch.opponents.map((o) => ({
    pandascore_id: o.opponent.id, // required — used to match winner later
    opponent_type: o.type, // "Team" or "Player"
    name: o.opponent.name,
    acronym: o.opponent.acronym,
    image_url: o.opponent.image_url,
    location: o.opponent.location,
  })),
  streams_list: pandascoreMatch.streams_list,
  raw_data: pandascoreMatch, // store full PandaScore object
};

// Push to backend (idempotent — safe to call multiple times)
const { data: matchWithOdds } = await fetch(`${API}/api/tournaments`, {
  method: 'POST',
  headers: {
    'Content-Type': 'application/json',
    Authorization: `Bearer ${jwt}`,
  },
  body: JSON.stringify(createMatchPayload),
}).then((r) => r.json());

// Now use matchWithOdds.id for staking
```

**This call is idempotent** — if the match already exists (by `pandascore_id`), it returns the existing record. Safe to call every time a user views a match.

#### 3. Sync Match Results (Admin/Cron Job)

After a match finishes, someone needs to push the updated status to trigger payouts. This can be:

- A **cron job** that polls PandaScore for finished matches
- An **admin action** in a dashboard
- The **frontend** checking match status on page load

```typescript
// Poll PandaScore for a specific match result
const result = await fetch(
  `https://api.pandascore.co/matches/${pandascoreId}`,
  { headers: { Authorization: `Bearer ${PANDASCORE_API_KEY}` } },
).then((r) => r.json());

// If finished, push the update to auto-resolve and pay out winners
if (result.status === 'finished' && result.winner_id) {
  await fetch(`${API}/api/tournaments/${backendMatchId}/sync`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'X-Admin-Token': ADMIN_TOKEN, // admin auth required
    },
    body: JSON.stringify({
      ...createMatchPayload, // same shape as create
      pandascore_status: 'finished',
      raw_data: {
        ...result,
        winner_id: result.winner_id, // must be in raw_data.winner_id
        forfeit: result.forfeit, // boolean
      },
    }),
  });
  // Backend auto-resolves + sends on-chain payouts to winners
}
```

### Key Points

| Responsibility                            | Who Does It                                                 |
| ----------------------------------------- | ----------------------------------------------------------- |
| Fetch match list from PandaScore API      | **Frontend**                                                |
| Display matches with odds to users        | **Frontend** (uses `GET /api/tournaments` for pool data)    |
| Push match data to backend on first stake | **Frontend** (calls `POST /api/tournaments`)                |
| Transfer USDC on stake                    | **Backend** (via delegation, no popup)                      |
| Poll PandaScore for match results         | **Frontend/Cron**                                           |
| Push finished status to backend           | **Frontend/Admin** (calls `POST /api/tournaments/:id/sync`) |
| Resolve match + pay winners on-chain      | **Backend** (automatic on sync)                             |
| PandaScore API key management             | **Frontend** (backend never calls PandaScore)               |

---

## Tournament Betting (Pool Staking)

Tournament betting is a **parimutuel pool** system: users stake USDC on one of two opponents. When the match ends, the total pool is split proportionally among winners (minus no protocol fee currently).

**Key concepts:**

- All amounts are in **micro-USDC** (6 decimals). `1 USDC = 1_000_000`
- Minimum stake: **1 USDC** (`1_000_000` micro-USDC)
- Stakes are **on-chain**: USDC moves from user → pool vault via delegation
- Payouts are **on-chain**: USDC moves from pool vault → winners automatically
- Odds are dynamic — they change as more stakes are placed

### Endpoints Overview

| Method | Path                             | Auth  | Description                                |
| ------ | -------------------------------- | ----- | ------------------------------------------ |
| `GET`  | `/api/tournaments`               | No    | List tournaments with pool stats           |
| `POST` | `/api/tournaments`               | JWT   | Create/sync tournament from PandaScore     |
| `GET`  | `/api/tournaments/:id`           | No    | Get single tournament with odds            |
| `POST` | `/api/tournaments/:id/stake`     | JWT   | Place a stake (on-chain transfer)          |
| `POST` | `/api/tournaments/:id/calculate` | No    | Preview potential payout                   |
| `GET`  | `/api/tournaments/:id/stakes`    | No    | Get tournament pool stats                  |
| `POST` | `/api/tournaments/:id/resolve`   | Admin | Resolve + pay winners on-chain             |
| `POST` | `/api/tournaments/:id/cancel`    | Admin | Cancel + refund all on-chain               |
| `POST` | `/api/tournaments/:id/sync`      | Admin | Sync status from PandaScore + auto-resolve |
| `GET`  | `/api/users/:wallet/stakes`      | No    | User's stake history                       |
| `GET`  | `/api/users/:wallet/stake-stats` | No    | User's aggregate stake stats               |

---

### List Tournaments

```
GET /api/tournaments?status=upcoming&videogame=cs-go&search=fnatic&limit=20&offset=0
```

| Query Param | Type   | Options                                         |
| ----------- | ------ | ----------------------------------------------- |
| `status`    | string | `upcoming`, `live`, `completed`, `cancelled`    |
| `videogame` | string | PandaScore slug (e.g. `cs-go`, `dota-2`, `lol`) |
| `league_id` | number | PandaScore league ID                            |
| `search`    | string | Search in tournament name                       |
| `limit`     | number | Default: 50                                     |
| `offset`    | number | Default: 0                                      |

**Response: `MatchWithOdds[]`**

```json
{
  "success": true,
  "data": [
    {
      "id": "uuid",
      "pandascore_id": 123456,
      "slug": "fnatic-vs-navi-2026-03-07",
      "name": "Fnatic vs Na'Vi",
      "videogame_id": 3,
      "videogame_name": "CS:GO",
      "videogame_slug": "cs-go",
      "league_id": 4197,
      "league_name": "ESL Pro League",
      "league_slug": "esl-pro-league",
      "league_image_url": "https://cdn.pandascore.co/...",
      "series_id": 6001,
      "series_name": "Season 19",
      "series_full_name": "ESL Pro League Season 19",
      "tournament_id": 12345,
      "tournament_name": "Group Stage",
      "tournament_slug": "group-stage",
      "scheduled_at": "2026-03-07T18:00:00Z",
      "begin_at": null,
      "end_at": null,
      "match_type": "best_of",
      "number_of_games": 3,
      "pandascore_status": "not_started",
      "status": "upcoming",
      "winner_id": null,
      "winner_type": null,
      "forfeit": false,
      "streams_list": [
        { "language": "en", "raw_url": "https://twitch.tv/..." }
      ],
      "detailed_stats": true,
      "raw_data": null,
      "created_at": "2026-03-07T12:00:00Z",
      "updated_at": "2026-03-07T12:00:00Z",

      "opponents": [
        {
          "id": "opponent-uuid-1",
          "match_id": "match-uuid",
          "pandascore_id": 100,
          "opponent_type": "Team",
          "name": "Fnatic",
          "acronym": "FNC",
          "image_url": "https://cdn.pandascore.co/images/team/...",
          "location": "EU",
          "position": 1,
          "is_winner": null,
          "created_at": "2026-03-07T12:00:00Z",
          "pool_usdc": 5000000000,
          "pool_percentage": 62.5,
          "odds": 1.6,
          "staker_count": 15
        },
        {
          "id": "opponent-uuid-2",
          "match_id": "match-uuid",
          "pandascore_id": 200,
          "opponent_type": "Team",
          "name": "Na'Vi",
          "acronym": "NAVI",
          "image_url": "https://cdn.pandascore.co/images/team/...",
          "location": "UA",
          "position": 2,
          "is_winner": null,
          "created_at": "2026-03-07T12:00:00Z",
          "pool_usdc": 3000000000,
          "pool_percentage": 37.5,
          "odds": 2.67,
          "staker_count": 8
        }
      ],
      "total_pool_usdc": 8000000000,
      "total_stakers": 23
    }
  ]
}
```

**Key display fields:**

- `opponents[].odds` — multiplier (e.g. 2.67 means $1 bet returns $2.67)
- `opponents[].pool_percentage` — % of total pool on this side
- `total_pool_usdc` — total USDC in the match pool (divide by 1_000_000 for display)
- `total_stakers` — total number of unique stakers

---

### Create / Sync Tournament

```
POST /api/tournaments
Authorization: Bearer <jwt>
Content-Type: application/json
```

The frontend pushes PandaScore match data when a user wants to stake on a match that doesn't exist yet in the backend.

```json
{
  "pandascore_id": 123456,
  "slug": "fnatic-vs-navi-2026-03-07",
  "name": "Fnatic vs Na'Vi",
  "videogame_id": 3,
  "videogame_name": "CS:GO",
  "videogame_slug": "cs-go",
  "league_id": 4197,
  "league_name": "ESL Pro League",
  "league_slug": "esl-pro-league",
  "league_image_url": "https://cdn.pandascore.co/...",
  "series_id": 6001,
  "series_name": "Season 19",
  "series_full_name": "ESL Pro League Season 19",
  "tournament_id": 12345,
  "tournament_name": "Group Stage",
  "tournament_slug": "group-stage",
  "scheduled_at": "2026-03-07T18:00:00Z",
  "match_type": "best_of",
  "number_of_games": 3,
  "pandascore_status": "not_started",
  "opponents": [
    {
      "pandascore_id": 100,
      "opponent_type": "Team",
      "name": "Fnatic",
      "acronym": "FNC",
      "image_url": "https://cdn.pandascore.co/...",
      "location": "EU"
    },
    {
      "pandascore_id": 200,
      "opponent_type": "Team",
      "name": "Na'Vi",
      "acronym": "NAVI",
      "image_url": "https://cdn.pandascore.co/...",
      "location": "UA"
    }
  ],
  "streams_list": [
    { "language": "en", "raw_url": "https://twitch.tv/esl_csgo" }
  ],
  "raw_data": {}
}
```

Returns: `MatchWithOdds` (same as list response). If the match already exists by `pandascore_id`, returns the existing record.

---

### Get Single Tournament

```
GET /api/tournaments/:id
```

Returns: `MatchWithOdds` — same shape as the list items.

`:id` can be the internal UUID.

---

### Place a Stake

```
POST /api/tournaments/:id/stake
Authorization: Bearer <jwt>
Content-Type: application/json
```

```json
{
  "user_wallet": "UserWalletPubkey...",
  "opponent_id": "opponent-uuid-1",
  "amount_usdc": 5000000
}
```

| Field         | Type   | Description                                     |
| ------------- | ------ | ----------------------------------------------- |
| `user_wallet` | string | Must match the wallet in the JWT                |
| `opponent_id` | string | UUID of the opponent (from `opponents[].id`)    |
| `amount_usdc` | number | Amount in micro-USDC. Min: `1_000_000` (1 USDC) |

**What happens on-chain:** The backend uses the SPL Token delegation to transfer `amount_usdc` from the user's USDC ATA to the pool vault — **no wallet popup**.

**Prerequisite:** The user must have approved delegation first (see [Delegation](#delegation-one-time-wallet-approval) section).

**Response: `PoolStakeRecord`**

```json
{
  "success": true,
  "data": {
    "id": "stake-uuid",
    "match_id": "match-uuid",
    "opponent_id": "opponent-uuid-1",
    "user_wallet": "UserWalletPubkey...",
    "amount_usdc": 5000000,
    "odds_at_stake": "1.60",
    "status": "active",
    "payout_usdc": null,
    "stake_tx_hash": null,
    "payout_tx_hash": null,
    "created_at": "2026-03-07T14:30:00Z",
    "resolved_at": null
  }
}
```

**Error cases:**

- `400` — "Stake amount must be positive"
- `400` — "Minimum stake is 1 USDC"
- `400` — "On-chain transfer failed: ..." (insufficient USDC, no delegation, etc.)
- `401` — "wallet in token does not match request"

---

### Calculate Potential Payout (Preview)

```
POST /api/tournaments/:id/calculate
Content-Type: application/json
```

```json
{
  "opponent_id": "opponent-uuid-1",
  "amount_usdc": 5000000
}
```

**Response:**

```json
{
  "success": true,
  "data": {
    "stake_amount_usdc": 5000000,
    "current_odds": 1.6,
    "min_payout_usdc": 8000000,
    "min_profit_usdc": 3000000,
    "profit_percentage": 60.0,
    "warning": null
  }
}
```

| Field               | Type    | Description                                       |
| ------------------- | ------- | ------------------------------------------------- |
| `stake_amount_usdc` | number  | Echo of the input amount                          |
| `current_odds`      | number  | Current multiplier (changes as pool grows)        |
| `min_payout_usdc`   | number  | Minimum payout if this side wins (includes stake) |
| `min_profit_usdc`   | number  | Minimum profit (payout - stake)                   |
| `profit_percentage` | number  | Profit as percentage of stake                     |
| `warning`           | string? | E.g. "You would be the only staker on this side"  |

Use this to show users "If you stake $5 on Fnatic, you could win $8 (1.6x)" before they confirm.

---

### Get Tournament Pool Stats

```
GET /api/tournaments/:id/stakes
```

Returns: `MatchWithOdds` — same as get tournament, with up-to-date pool numbers.

---

### Resolve Tournament (Admin)

```
POST /api/tournaments/:id/resolve
X-Admin-Token: <AUTH_ADMIN_TOKEN>
Content-Type: application/json
```

```json
{
  "winner_opponent_id": "opponent-uuid-1",
  "pandascore_winner_id": 100,
  "forfeit": false
}
```

This triggers on-chain USDC payouts from the pool vault to all winning stakers proportionally. If only one side has stakers, everyone gets refunded instead.

---

### Cancel Tournament (Admin)

```
POST /api/tournaments/:id/cancel
X-Admin-Token: <AUTH_ADMIN_TOKEN>
```

All active stakes are refunded on-chain in full.

---

### Sync Tournament from PandaScore (Admin)

```
POST /api/tournaments/:id/sync
X-Admin-Token: <AUTH_ADMIN_TOKEN>
Content-Type: application/json
```

```json
{
  "pandascore_status": "finished",
  "winner_id": 100,
  "begin_at": "2026-03-07T18:00:00Z",
  "end_at": "2026-03-07T19:30:00Z"
}
```

If `pandascore_status` is `"finished"` and `winner_id` is provided, the backend auto-resolves the match and pays out winners on-chain.

---

## Kombats (1v1 Wagers)

All routes available under both `/wagers/*` and `/api/kombats/*`.

| Method | Path                                   | Auth | Body                                                       | Returns      |
| ------ | -------------------------------------- | ---- | ---------------------------------------------------------- | ------------ |
| `GET`  | `/api/kombats`                         | No   | Query: `?initiator=&challenger=&status=&limit=&offset=`    | `[Wager]`    |
| `GET`  | `/api/kombats/:address`                | No   | —                                                          | `Wager`      |
| `POST` | `/api/kombats`                         | No   | `CreateWagerRequest`                                       | `TxResponse` |
| `POST` | `/api/kombats/:address/accept`         | No   | `{ "challenger": "pubkey" }`                               | `TxResponse` |
| `POST` | `/api/kombats/:address/cancel`         | No   | `{ "initiator": "pubkey" }`                                | `TxResponse` |
| `POST` | `/api/kombats/:address/decline`        | No   | `{ "initiator": "pubkey" }`                                | `TxResponse` |
| `POST` | `/api/kombats/:address/resolve`        | No   | `{ "winner": "pubkey", "caller": "pubkey" }`               | `TxResponse` |
| `POST` | `/api/kombats/:address/declare-winner` | No   | `{ "participant": "pubkey", "declared_winner": "pubkey" }` | `TxResponse` |
| `POST` | `/api/kombats/:address/dispute`        | No   | `{ "opener": "pubkey" }`                                   | `TxResponse` |

### CreateWagerRequest

```json
{
  "initiator": "PubkeyString",
  "stake_usdc": 1000000,
  "description": "Wager description",
  "expiry_ts": 1700000000,
  "resolution_source": "manual",
  "resolver": "PubkeyString",
  "initiator_option": "Team A wins",
  "oracle_feed": null,
  "oracle_target": null,
  "oracle_initiator_wins_above": null
}
```

### TxResponse

```json
{
  "transaction_b64": "<base64-encoded-unsigned-transaction>",
  "description": "Human readable description"
}
```

**Frontend flow:** Decode → Sign with wallet → Submit to Solana RPC.

```typescript
const { data } = await fetch(`${API}/api/kombats`, {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify(createWagerRequest),
}).then((r) => r.json());

const txBytes = Buffer.from(data.transaction_b64, 'base64');
const tx = Transaction.from(txBytes);
const signed = await wallet.signTransaction(tx);
const sig = await connection.sendRawTransaction(signed.serialize());
```

---

## Users

Available under both `/users/*` and `/api/users/*`.

| Method   | Path                                       | Auth | Body                                           | Returns                |
| -------- | ------------------------------------------ | ---- | ---------------------------------------------- | ---------------------- |
| `GET`    | `/api/users/:wallet`                       | No   | —                                              | `User`                 |
| `POST`   | `/api/users/:wallet`                       | No   | `{ "display_name"?, "avatar_url"?, "email"? }` | `User`                 |
| `DELETE` | `/api/users/:wallet`                       | No   | —                                              | `null`                 |
| `GET`    | `/api/users/:wallet/stats`                 | No   | —                                              | `UserStats`            |
| `GET`    | `/api/users/:wallet/notification-settings` | No   | —                                              | `NotificationSettings` |
| `PUT`    | `/api/users/:wallet/notification-settings` | No   | `UpdateNotificationSettings`                   | `NotificationSettings` |
| `POST`   | `/api/users/:wallet/push-token`            | No   | `{ "expo_token": "ExponentPushToken[...]" }`   | `null`                 |

### User Object

```json
{
  "id": "uuid",
  "wallet_address": "pubkey",
  "email": "user@example.com",
  "display_name": "Alice",
  "avatar_url": "https://kombat-backend-production.up.railway.app/uploads/avatar_xxx.png",
  "wins": 0,
  "losses": 0,
  "created_at": "2026-02-18T20:22:02Z",
  "updated_at": "2026-02-18T20:22:02Z"
}
```

---

## User Stakes

| Method | Path                             | Auth | Description                          |
| ------ | -------------------------------- | ---- | ------------------------------------ |
| `GET`  | `/api/users/:wallet/stakes`      | No   | User's stake history with match info |
| `GET`  | `/api/users/:wallet/stake-stats` | No   | Aggregate stats                      |

### User Stakes List

```
GET /api/users/:wallet/stakes?status=active&match_id=uuid&limit=20&offset=0
```

| Query Param | Type   | Options                             |
| ----------- | ------ | ----------------------------------- |
| `status`    | string | `active`, `won`, `lost`, `refunded` |
| `match_id`  | string | Filter by specific match UUID       |
| `limit`     | number | Default: 50                         |
| `offset`    | number | Default: 0                          |

**Response: `StakeWithMatch[]`**

```json
{
  "success": true,
  "data": [
    {
      "id": "stake-uuid",
      "match_id": "match-uuid",
      "opponent_id": "opponent-uuid",
      "user_wallet": "pubkey",
      "amount_usdc": 5000000,
      "odds_at_stake": "1.60",
      "status": "won",
      "payout_usdc": 8000000,
      "stake_tx_hash": null,
      "payout_tx_hash": "5xYz...Solana-tx-signature",
      "created_at": "2026-03-07T14:30:00Z",
      "resolved_at": "2026-03-07T19:31:00Z",
      "match_name": "Fnatic vs Na'Vi",
      "match_status": "completed",
      "opponent_name": "Fnatic",
      "opponent_image_url": "https://cdn.pandascore.co/...",
      "videogame_name": "CS:GO",
      "scheduled_at": "2026-03-07T18:00:00Z"
    }
  ]
}
```

### User Stake Stats

```
GET /api/users/:wallet/stake-stats
```

```json
{
  "success": true,
  "data": {
    "active_stakes": 2,
    "total_staked_usdc": 15000000,
    "total_won_usdc": 8000000,
    "total_lost_usdc": 5000000,
    "win_count": 3,
    "loss_count": 1
  }
}
```

---

## File Upload

```
POST /api/uploads
Content-Type: multipart/form-data
```

| Field       | Type   | Values                            |
| ----------- | ------ | --------------------------------- |
| `file`      | File   | Image (png, jpg, jpeg, gif, webp) |
| `file_type` | String | `avatar` or `evidence`            |

**Response:**

```json
{
  "url": "https://kombat-backend-production.up.railway.app/uploads/avatar_abc123.png"
}
```

Max file size: 5 MB. Uploaded files served statically from `/uploads/*`.

---

## Notifications

| Method | Path                                           | Description               |
| ------ | ---------------------------------------------- | ------------------------- |
| `GET`  | `/api/notifications/:wallet?limit=50&offset=0` | List notifications        |
| `POST` | `/notifications/:id/read`                      | Mark notification as read |

### Realtime

```javascript
const evtSrc = new EventSource(`${API}/notifications/stream/${wallet}`);
evtSrc.onmessage = (e) => {
  const notif = JSON.parse(e.data);
  console.log('notification:', notif);
};
```

**WebSocket:**

```javascript
const ws = new WebSocket(
  `${API.replace('http', 'ws')}/ws/notifications/${wallet}`,
);
ws.onmessage = (ev) => {
  const notif = JSON.parse(ev.data);
  // optional ACK: ws.send(JSON.stringify({ ack: notif.id }));
};
```

### Notification Kinds

| Kind             | Payload                                                            |
| ---------------- | ------------------------------------------------------------------ |
| `wager_accepted` | `{ wager_address, challenger }`                                    |
| `wager_disputed` | `{ wager_address, opener }`                                        |
| `wager_resolved` | `{ wager_address, winner }`                                        |
| `stake_placed`   | `{ match_id, match_name, opponent_name, amount_usdc, total_pool }` |

---

## Complete User Flow

### First-Time Setup (Once Only)

```
1. User logs in via Dynamic SDK → POST /api/auth/verify → get JWT
2. Check delegation: GET /api/delegation/status?wallet=X
3. If delegated_amount === 0:
   a. GET /api/delegation/approve-tx?wallet=X → get unsigned tx
   b. User signs tx in wallet (Phantom popup)
   c. Submit signed tx to Solana
4. User is now set up — no more wallet popups needed
```

### Browsing & Staking on a Tournament

```
1. GET /api/tournaments?status=upcoming  → show match list with odds
2. User taps a match → GET /api/tournaments/:id → show detail + odds
3. User picks a side → POST /api/tournaments/:id/calculate → show preview
   "Stake $5 on Fnatic → potential win $8 (1.6x odds)"
4. User confirms → POST /api/tournaments/:id/stake → USDC moves on-chain silently
5. Poll GET /api/tournaments/:id to see updated odds
```

### Checking Stake History

```
1. GET /api/users/:wallet/stake-stats → show summary card
   "Active: 2 | Won: 3 ($8 USDC) | Lost: 1 ($5 USDC)"
2. GET /api/users/:wallet/stakes → show full history with match details
3. Filter by status: ?status=active (pending matches)
                     ?status=won (won bets)
                     ?status=lost (lost bets)
                     ?status=refunded (cancelled/one-sided matches)
```

### After a Match Ends

```
The backend auto-resolves matches via PandaScore sync:
- POST /api/tournaments/:id/sync (called by admin/cron)
- Winners receive USDC payouts on-chain automatically
- payout_tx_hash is populated on the stake record
- User can verify on Solana explorer
```

### Create 1v1 Kombat

```
1. POST /api/kombats with wager details → get transaction_b64
2. Decode, sign with wallet, submit to Solana
3. Indexer picks up the on-chain event and stores the wager in DB
```

### Accept Kombat

```
1. POST /api/kombats/:address/accept → get transaction_b64
2. Sign and submit
3. Initiator receives wager_accepted notification via SSE/WS
```

### Declare Winner (Mutual Consent)

```
1. POST /api/kombats/:address/declare-winner with { participant, declared_winner }
2. Sign and submit
3. Both parties receive wager_resolved notification
```

---

## USDC Amount Formatting

All USDC amounts in the API use **micro-USDC** (6 decimal places):

| Display | API Value   |
| ------- | ----------- |
| $1.00   | `1000000`   |
| $5.00   | `5000000`   |
| $100.00 | `100000000` |
| $0.50   | `500000`    |

```typescript
// Convert API value to display
const displayUSDC = (microUsdc: number) => (microUsdc / 1_000_000).toFixed(2);

// Convert user input to API value
const toMicroUSDC = (dollars: number) => Math.round(dollars * 1_000_000);
```

---

## Environment Variables

```env
DATABASE_URL=postgres://user:pass@host:5432/wager_db
SOLANA_RPC_URL=https://api.devnet.solana.com
PORT=3000
AUTH_JWT_SECRET=<secret>
AUTH_ADMIN_TOKEN=<admin-token>
WAGER_PROGRAM_ID=Dj2Hot5XJLv9S724BRkWohrhUfzLFERBnZJ9da2WBJQK
PLATFORM_SIGNER_KEYPAIR=[...64-byte-json-array...]  # enables delegation + on-chain payouts

# Optional
DYNAMIC_ENVIRONMENT_ID=<your-dynamic-env-id>   # enables POST /api/auth/verify
UPLOAD_DIR=./uploads                            # enables file uploads
UPLOAD_BASE_URL=https://your-domain.com         # prefix for upload URLs
REDIS_URL=redis://localhost:6379                 # enables cross-instance notifications
```
