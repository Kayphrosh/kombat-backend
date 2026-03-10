# Kombat Backend

A Rust-based REST API powering the **Kombat** platform — a peer-to-peer wager (Kombat) system and esports tournament pool-staking game built on Solana. Users stake USDC, challenge each other, and bet on real esports matches.

---

### All program activity

To see every transaction ever executed by the Kombat smart contract:

```
https://explorer.solana.com/address/Dj2Hot5XJLv9S724BRkWohrhUfzLFERBnZJ9da2WBJQK
https://explorer.solana.com/address/Dj2Hot5XJLv9S724BRkWohrhUfzLFERBnZJ9da2WBJQK?cluster=devnet
```

> **Tip:** Swap `?cluster=devnet` for `?cluster=mainnet-beta` depending on the environment.

---

## Table of Contents

- [Architecture Overview](#architecture-overview)
- [Tech Stack](#tech-stack)
- [Core Concepts](#core-concepts)
- [Authentication Flow](#authentication-flow)
- [Kombat (Wager) Lifecycle](#kombat-wager-lifecycle)
- [Tournament Pool Staking](#tournament-pool-staking)
- [Notifications](#notifications)
- [Viewing Transactions on Solana](#viewing-transactions-on-solana)
- [File Uploads](#file-uploads)
- [API Reference](#api-reference)
- [Environment Variables](#environment-variables)
- [Running Locally](#running-locally)
- [Docker](#docker)
- [Database Migrations](#database-migrations)

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────┐
│                   Client (Mobile App)               │
│  Signs transactions with user's Solana wallet       │
└────────────────────────┬────────────────────────────┘
                         │ HTTPS + JWT
                         ▼
┌─────────────────────────────────────────────────────┐
│              Kombat Backend (Axum / Rust)           │
│                                                     │
│  ┌───────────┐  ┌──────────────┐  ┌─────────────┐  │
│  │  Wager    │  │  Tournament  │  │    Auth     │  │
│  │ Handlers  │  │  Handlers    │  │  Handlers   │  │
│  └─────┬─────┘  └──────┬───────┘  └──────┬──────┘  │
│        │                │                 │         │
│  ┌─────▼────────────────▼─────────────────▼──────┐  │
│  │                 Services Layer                │  │
│  │  DbService  │  SolanaService  │  DynamicSvc  │  │
│  └──────┬──────┴────────┬────────┴──────────────┘  │
│         │               │                           │
│    PostgreSQL      Solana RPC                       │
│    (SQLx)          (Devnet/Mainnet)                 │
└─────────────────────────────────────────────────────┘
          │
     Redis (optional) — cross-instance pub/sub
     Expo Push — mobile push notifications
```

**Key design principle:** The backend is **non-custodial** for Kombat wagers. It builds unsigned Solana transactions and returns them as `base64` to the client. The client's wallet signs them. The backend never holds private keys.

For **tournament pool stakes**, the backend acts as a custodial escrow (custodial pool staking pattern), tracking amounts in PostgreSQL and computing payouts directly.

---

## Tech Stack

| Component          | Technology                                  |
| ------------------ | ------------------------------------------- |
| Language           | Rust (2021 edition)                         |
| Web Framework      | Axum 0.7                                    |
| Database           | PostgreSQL via SQLx 0.8                     |
| Blockchain         | Solana (`solana-sdk` v2)                    |
| Token              | USDC (SPL Token)                            |
| Auth               | JWT (HS256) + Dynamic SDK (RS256/ES256)     |
| Push Notifications | Expo Push API                               |
| Real-time          | Tokio broadcast + WebSocket + Redis pub/sub |
| Metrics            | Prometheus                                  |
| Containerization   | Docker + Docker Compose                     |

---

## Core Concepts

### USDC amounts

All USDC amounts in the API use **micro-USDC** (6 decimal places). So `1 USDC = 1,000,000`.

### Wager / Kombat

A peer-to-peer bet between two users on some outcome. Each wager has:

- An **initiator** (creator) and a **challenger**
- A **stake** in micro-USDC locked in a Solana escrow PDA
- A **description** of the bet (up to 256 chars)
- An **expiry timestamp**
- A **resolution source**: `MutualConsent`, `Arbitrator`, or `OracleFeed`
- A **resolver** wallet (for arbitrated wagers)
- Optional `initiator_option` (e.g., "yes"/"no") to describe each side's position

### Tournament Match

An esports match sourced from PandaScore. Users stake on one of two opponents. The pool is split pro-rata among winners minus a protocol fee.

---

## Authentication Flow

The API supports two auth methods:

### 1. Solana Wallet Signature (native)

```
Client                         Server
  │                               │
  │── GET /auth/nonce/:wallet ────►│  Generate one-time nonce (UUID, 5min TTL)
  │◄─ { nonce, expires_at } ──────│  Rate limited: 5 nonces/minute/wallet
  │                               │
  │  Client signs: nacl.sign(nonce, keypair)
  │                               │
  │── POST /auth/verify ─────────►│  Verify ed25519 signature over nonce
  │   { wallet, signature }       │  Upsert user profile on first login
  │◄─ { token, expires_at } ──────│  Issue 15-min JWT (HS256)
```

### 2. Dynamic SDK (Web3 social login)

```
Client                         Server
  │                               │
  │  User logs in via Dynamic SDK │
  │  (email, Google, wallets...)  │
  │                               │
  │── POST /api/auth/verify ─────►│  Verify Dynamic JWT via JWKS endpoint
  │   { dynamicToken }            │  Extract Solana wallet from credentials
  │◄─ { user, accessToken } ──────│  Upsert user, issue app-level 15-min JWT
```

The issued JWT contains `{ wallet, exp }` and must be sent as `Authorization: Bearer <token>` for protected endpoints.

---

## Kombat (Wager) Lifecycle

```
                   ┌──────────────────────────────────┐
                   │         KOMBAT LIFECYCLE          │
                   └──────────────────────────────────┘

  POST /api/kombats
  ┌──────────┐
  │ pending  │  ◄── Unsigned tx returned; user signs and broadcasts
  └────┬─────┘
       │   POST /api/kombats/:address/accept
       ▼
  ┌──────────┐
  │  active  │  ◄── Challenger locks their stake in escrow
  └────┬─────┘
       │
       ├────── POST /api/kombats/:address/declare-winner ──────► resolved
       │        (MutualConsent: both parties declare same winner,
       │         auto-pays on-chain. Arbitrator: resolver declares.)
       │
       ├────── POST /api/kombats/:address/dispute ─────────────► disputed
       │        (Either participant opens dispute on-chain)
       │        POST /api/kombats/:address/dispute/submit
       │        (Both parties submit evidence + declared winner)
       │
       └────── POST /api/kombats/:address/cancel ──────────────► cancelled
                POST /api/kombats/:address/decline ─────────────► declined
```

### Resolution Modes

| Mode            | How it resolves                                                                                  |
| --------------- | ------------------------------------------------------------------------------------------------ |
| `MutualConsent` | Both sides call declare-winner with the same address → auto-pays via `consent_resolve` on-chain  |
| `Arbitrator`    | Only the designated `resolver` wallet can call declare-winner → `resolve_by_arbitrator` on-chain |
| `OracleFeed`    | Price-feed oracle resolves automatically (oracle target price)                                   |

### On-Chain Mechanics

The backend builds **unsigned** Solana transactions using Anchor instruction builders:

- `initialize_registry` — auto-prepended on first wager for a new user
- `create_wager` — locks initiator's USDC in escrow PDA
- `accept_wager` — locks challenger's USDC in the same escrow
- `cancel_wager` / decline — refunds initiator from escrow
- `resolve_by_arbitrator` — pays winner + treasury fee
- `consent_resolve` — mutual consent auto-payment
- `open_dispute` — flags the wager as disputed on-chain

All PDAs are derived deterministically:

```
registry_pda = ["registry", authority]
wager_pda    = ["wager", initiator, wager_id_le_bytes]
escrow_pda   = ["escrow", wager_pda]
config_pda   = ["config"]  ← holds treasury + USDC mint
```

USDC mint and treasury are read dynamically from the on-chain `ProtocolConfig` PDA — no hardcoded addresses.

---

## Tournament Pool Staking

A **custodial** pool model for esports match betting. The backend tracks all stakes in PostgreSQL.

```
  POST /api/tournaments          ← Sync match from PandaScore (requires JWT)
  GET  /api/tournaments          ← List matches with live odds
  GET  /api/tournaments/:id      ← Match detail with pool breakdown per opponent

  POST /api/tournaments/:id/calculate  ← Preview payout before staking
  POST /api/tournaments/:id/stake      ← Place stake on an opponent (requires JWT)

  POST /api/tournaments/:id/resolve    ← Admin: resolve match, compute payouts
  POST /api/tournaments/:id/cancel     ← Admin: cancel match, refund all stakes
  POST /api/tournaments/:id/sync       ← Sync match status; auto-resolves if finished
```

### Odds Calculation

Odds are dynamic (parimutuel-style) and shift as more stakes come in:

```
opponent_odds = total_pool / opponent_pool
payout        = stake_amount × odds × (1 - protocol_fee)
```

The `GET /api/tournaments/:id` response includes per-opponent:

- `pool_usdc` — total USDC staked on this opponent
- `pool_percentage` — share of total pool
- `odds` — current multiplier
- `staker_count`

### Auto-Resolution

When the frontend syncs a finished match (`POST /api/tournaments/:id/sync`), the server checks `raw_data.winner_id` and auto-resolves if a matching opponent is found.

---

## Notifications

### In-App

Stored in `notifications` table. Fetched via:

```
GET  /api/notifications/:wallet
POST /api/notifications/:id/read
```

### Real-Time (SSE + WebSocket)

```
GET /notifications/stream/:wallet   ← Server-Sent Events stream
GET /ws/notifications/:wallet       ← WebSocket connection
```

Both are backed by a Tokio `broadcast` channel. Redis pub/sub is used when `REDIS_URL` is set, enabling cross-instance delivery.

### Push (Expo)

Push tokens are registered per user:

```
POST /api/users/:wallet/push-token  { expo_token }
```

Relevant events that trigger push notifications:

- `wager_challenge` — when you're challenged
- `wager_accepted` — when your challenge is accepted
- `wager_resolved` — when a wager resolves
- `wager_disputed` — when the other party opens a dispute
- `stake_placed` — when stake activity occurs in a tournament

---

## Viewing Transactions on Solana

All Kombat wagers are settled on-chain. Every API response that touches the chain returns enough data to look up the transaction on **Solana Explorer**.

### Kombat / Wager transactions

Every kombat response includes an `on_chain_address` field — this is the **wager PDA** on Solana. Paste it into:

```
https://explorer.solana.com/address/<on_chain_address>
https://explorer.solana.com/address/<on_chain_address>?cluster=devnet   ← devnet
```

This page shows the full account state and every transaction that has ever touched that wager (create, accept, resolve, dispute, cancel).

### Tournament stake transactions

Tournament stakes are custodial, but the backend stores the on-chain hash for each movement:

| Field in `pool_stakes` | What it links to                                   |
| ---------------------- | -------------------------------------------------- |
| `stake_tx_hash`        | The Solana tx where the user's stake was deposited |
| `payout_tx_hash`       | The Solana tx where the payout/refund was sent     |

Look them up at:

```
https://explorer.solana.com/tx/<stake_tx_hash>
https://explorer.solana.com/tx/<payout_tx_hash>
```


## File Uploads

When `UPLOAD_DIR` is configured, avatar and media uploads are enabled:

```
POST /api/uploads   ← multipart/form-data, returns { url }
GET  /uploads/*     ← Static file serving
```

---

## API Reference

### Health

| Method | Path      | Auth | Description          |
| ------ | --------- | ---- | -------------------- |
| GET    | `/health` | —    | Service health check |

### Auth

| Method | Path                  | Auth  | Description                             |
| ------ | --------------------- | ----- | --------------------------------------- |
| GET    | `/auth/nonce/:wallet` | —     | Get one-time nonce (rate limited 5/min) |
| POST   | `/auth/verify`        | —     | Verify Solana wallet signature → JWT    |
| POST   | `/api/auth/verify`    | —     | Verify Dynamic SDK token → JWT          |
| POST   | `/auth/token`         | Admin | Mint JWT for a wallet (admin only)      |

### Kombats (Wagers)

| Method | Path                                   | Auth | Description                                            |
| ------ | -------------------------------------- | ---- | ------------------------------------------------------ |
| GET    | `/api/kombats`                         | —    | List kombats (filter by initiator, challenger, status) |
| POST   | `/api/kombats`                         | —    | Create kombat → returns unsigned tx                    |
| GET    | `/api/kombats/:address`                | —    | Get kombat detail with participant profiles            |
| POST   | `/api/kombats/:address/accept`         | —    | Accept → unsigned tx for challenger                    |
| POST   | `/api/kombats/:address/decline`        | —    | Decline challenge                                      |
| POST   | `/api/kombats/:address/cancel`         | —    | Cancel (initiator only)                                |
| POST   | `/api/kombats/:address/resolve`        | —    | Resolve via arbitrator                                 |
| POST   | `/api/kombats/:address/declare-winner` | —    | Mutual consent or arbitrator declare winner            |
| POST   | `/api/kombats/:address/dispute`        | —    | Open dispute on-chain                                  |
| POST   | `/api/kombats/:address/dispute/submit` | —    | Submit dispute evidence                                |
| GET    | `/api/kombats/:address/dispute`        | —    | Get all dispute submissions                            |

### Users

| Method | Path                                       | Auth | Description                                      |
| ------ | ------------------------------------------ | ---- | ------------------------------------------------ |
| GET    | `/api/users/:wallet`                       | —    | Get user profile                                 |
| POST   | `/api/users/:wallet`                       | —    | Update profile (display_name, avatar_url, email) |
| DELETE | `/api/users/:wallet`                       | —    | Delete user                                      |
| GET    | `/api/users/:wallet/stats`                 | —    | Win/loss stats and total stake                   |
| GET    | `/api/users/:wallet/notification-settings` | —    | Get notification preferences                     |
| PUT    | `/api/users/:wallet/notification-settings` | —    | Update notification preferences                  |
| POST   | `/api/users/:wallet/push-token`            | —    | Register Expo push token                         |
| GET    | `/api/users/:wallet/stakes`                | JWT  | Get user's tournament stake history              |
| GET    | `/api/users/:wallet/stake-stats`           | JWT  | Aggregate stake statistics                       |

### Tournaments

| Method | Path                             | Auth  | Description                                                  |
| ------ | -------------------------------- | ----- | ------------------------------------------------------------ |
| GET    | `/api/tournaments`               | —     | List matches with odds (filter by status, videogame, league) |
| POST   | `/api/tournaments`               | JWT   | Create/sync match from PandaScore                            |
| GET    | `/api/tournaments/:id`           | —     | Get match with live pool odds                                |
| POST   | `/api/tournaments/:id/stake`     | JWT   | Place stake on an opponent                                   |
| POST   | `/api/tournaments/:id/calculate` | —     | Preview payout calculation                                   |
| GET    | `/api/tournaments/:id/stakes`    | —     | Get pool stats for a match                                   |
| POST   | `/api/tournaments/:id/resolve`   | Admin | Resolve match and process payouts                            |
| POST   | `/api/tournaments/:id/cancel`    | Admin | Cancel match and refund stakes                               |
| POST   | `/api/tournaments/:id/sync`      | —     | Sync match data + auto-resolve if finished                   |

### Notifications

| Method | Path                            | Auth | Description                    |
| ------ | ------------------------------- | ---- | ------------------------------ |
| GET    | `/api/notifications/:wallet`    | —    | List notifications (paginated) |
| POST   | `/api/notifications/:id/read`   | —    | Mark notification as read      |
| GET    | `/notifications/stream/:wallet` | —    | SSE real-time stream           |
| GET    | `/ws/notifications/:wallet`     | —    | WebSocket real-time stream     |

### Other

| Method | Path           | Auth | Description             |
| ------ | -------------- | ---- | ----------------------- |
| POST   | `/api/uploads` | —    | Upload file (multipart) |
| GET    | `/uploads/*`   | —    | Serve uploaded files    |
| GET    | `/metrics`     | —    | Prometheus metrics      |

---

## Environment Variables

Copy `.env.example` to `.env`:

```bash
cp .env.example .env
```

| Variable                 | Required | Description                                             |
| ------------------------ | -------- | ------------------------------------------------------- |
| `DATABASE_URL`           | ✅       | PostgreSQL connection string                            |
| `SOLANA_RPC_URL`         | ✅       | Solana RPC endpoint (devnet or mainnet-beta)            |
| `AUTH_JWT_SECRET`        | ✅       | Secret for signing app-level JWTs                       |
| `AUTH_ADMIN_TOKEN`       | ✅       | Token for admin-only endpoints                          |
| `PORT`                   | —        | Listen port (default: `3000`)                           |
| `WAGER_PROGRAM_ID`       | —        | Deployed Anchor program ID (default: `Dj2Hot5X...`)     |
| `DYNAMIC_ENVIRONMENT_ID` | —        | Dynamic SDK environment ID (enables `/api/auth/verify`) |
| `UPLOAD_DIR`             | —        | Directory for file uploads (enables `/api/uploads`)     |
| `UPLOAD_BASE_URL`        | —        | Base URL for uploaded file URLs                         |
| `REDIS_URL`              | —        | Redis URL for cross-instance notifications              |
| `RUST_LOG`               | —        | Log level (e.g. `wager_api=debug,tower_http=debug`)     |

---

## Running Locally

### Prerequisites

- Rust (stable, 1.75+)
- PostgreSQL 14+
- Solana CLI (for local validator, optional)

### Steps

```bash
# 1. Clone the repo
git clone <repo-url>
cd kombat-backend

# 2. Set up environment
cp .env.example .env
# Edit .env with your DATABASE_URL, AUTH_JWT_SECRET, etc.

# 3. Run database migrations
cd app
sqlx migrate run

# 4. Start the server
cargo run -p wager-api
```

The server starts on `http://0.0.0.0:3000` by default.

---

## Docker

```bash
# Build and start all services (API + PostgreSQL)
docker-compose up --build

# Stop
docker-compose down
```

The `docker-compose.yml` spins up:

- **postgres** — database
- **api** — the Rust backend (exposes port 3000)

---

## Database Migrations

Migrations live in `app/migrations/` (SQLx format).

```bash
# Run all pending migrations
cd app && sqlx migrate run

# Revert last migration
cd app && sqlx migrate revert

# Add a new migration
sqlx migrate add <migration_name>
```

### Core Tables

| Table                   | Purpose                                                       |
| ----------------------- | ------------------------------------------------------------- |
| `wagers`                | Kombat records (on-chain address, status, participants)       |
| `dispute_submissions`   | Evidence and declared winners from each party                 |
| `users`                 | User profiles (wallet, display_name, avatar_url, wins/losses) |
| `nonces`                | One-time auth nonces for Solana wallet login                  |
| `notifications`         | In-app notification records                                   |
| `notification_settings` | Per-user notification preferences                             |
| `push_tokens`           | Expo push tokens per user                                     |
| `matches`               | Esports match records from PandaScore                         |
| `match_opponents`       | Two opponents per match with team/player info                 |
| `pool_stakes`           | Individual stakes placed on match outcomes                    |

---

## On-Chain Program

The smart contract (`programs/`) is built with **Anchor** and deployed on Solana. The backend interacts with it purely through instruction construction (no on-chain calls for reads except for config PDAs).

Program ID: `Dj2Hot5XJLv9S724BRkWohrhUfzLFERBnZJ9da2WBJQK` (devnet default)

### Protocol Fees

- Default protocol fee: **1% (100 bps)**
- Fee is sent to the protocol `treasury` wallet on resolution
- Treasury address is read from the on-chain `ProtocolConfig` PDA at resolution time

---

## Monitoring

Prometheus metrics are exposed at `GET /metrics`:

- `nonce_rate_limit_exceeded_total` — number of rate limit violations on nonce requests
- `nonce_rate_limit_requests_total` — total nonce requests

Standard HTTP request metrics are available via `TraceLayer`.
