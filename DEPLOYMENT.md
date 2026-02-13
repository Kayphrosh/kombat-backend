# Deployment Guide

Since you are "vibe coding" and not a backend specialist, this guide simplifies the deployment into 3 clear steps.

## The 3 Parts You Need to Deploy

1.  **The Smart Contract (Solana Program)** → Deployed to Solana Blockchain.
2.  **The Database (PostgreSQL)** → Hosted on a cloud provider.
3.  **The API (Rust Backend)** → Hosted on a cloud provider, connects to the DB and Solana.

---

## Step 1: Deploy the Smart Contract

You need a Solana wallet with some SOL on Devnet.

1.  **Build**:
    ```bash
    anchor build
    ```
2.  **Get your Program ID**:
    Run `solana address -k target/deploy/wager-keypair.json`
    - Update `Anchor.toml` and `programs/wager/src/lib.rs` with this new address if it changed.
    - Run `anchor build` again if you changed the ID.
3.  **Deploy**:
    ```bash
    # Change provider.cluster to devnet in Anchor.toml first!
    anchor deploy --provider.cluster devnet
    ```

    - _Copy the Program ID_ — you will need this for your frontend.

---

## Step 2: Deploy the Database & API (Easiest Method: Railway.app)

I recommend **Railway** or **Render** because they handle Rust and Postgres automatically without complex setup.

### Option A: Using Railway (Recommended)

1.  Create an account at [railway.app](https://railway.app).
2.  **New Project** → **Provision PostgreSQL**.
    - Railway will give you a `DATABASE_URL`.
3.  **New Service** → **GitHub Repo** → Select your `kombat-backend` repo.
    - Railway will detect the `Dockerfile` I just created.
4.  **Variables**: Add these in the Railway dashboard for the API service:
    - `DATABASE_URL`: (Paste the one from the Postgres service)
    - `SOLANA_RPC_URL`: `https://api.devnet.solana.com` (or your QuickNode/Helius RPC URL)
    - `PORT`: `3000`
5.  **Deploy**:
    - Railway will build the Docker container and start it.
    - It gives you a public URL (e.g., `https://kombat-backend-production.up.railway.app`).

### Option B: Using Docker Compose (Self-Hosted)

If you have a VPS (like DigitalOcean Droplet), you can just run:

```bash
docker-compose up -d --build
```

This starts both the Database and the API on that server.

---

## Step 3: Run Migrations

Once your database is live (on Railway or elsewhere), you need to create the tables.

**From your local machine:**

```bash
# 1. Export the production DB URL
export DATABASE_URL="postgres://railway:password@containers-us-west.railway.app:5432/railway"

# 2. Run the migrations
sqlx migrate run
```

---

## What to send the Frontend Dev

The frontend developer needs:

1.  **The API URL**: (e.g., `https://kombat-backend.railway.app`)
2.  **The Program ID**: (The address of your deployed smart contract)
3.  **The IDL**: The file located at `target/idl/wager.json`. This tells their frontend how to talk to Solana.
