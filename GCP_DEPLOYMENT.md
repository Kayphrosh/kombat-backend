# Google Cloud Run Deployment Guide

This guide walks you through deploying `wager-api` to Google Cloud Run, a fully managed serverless platform that scales your container automatically.

## Prerequisites

1.  **Google Cloud Project**: Create one at [console.cloud.google.com](https://console.cloud.google.com).
2.  **Google Cloud SDK**: Install the `gcloud` CLI.
3.  **Billing**: Ensure billing is enabled for your project.

## Step 1: Initialize gcloud

Run this locally to log in and set your project ID:

```bash
gcloud auth login
gcloud config set project [YOUR_PROJECT_ID]
```

## Step 2: Build & Push Container

We'll use **Cloud Build** to build your Docker image and store it in the **Google Container Registry** (GCR) or **Artifact Registry**.

```bash
# Enable necessary services
gcloud services enable cloudbuild.googleapis.com run.googleapis.com

# Submit the build (replace [PROJECT_ID])
gcloud builds submit --tag gcr.io/[PROJECT_ID]/wager-api
```

_Note: This uploads your code, runs the `Dockerfile`, and stores the image._

## Step 3: Set up Cloud SQL (Postgres)

1.  Go to **Cloud SQL** in the console.
2.  Create a **PostgreSQL** instance.
    - **Region**: Same as where you plan to deploy Cloud Run (e.g., `us-central1`).
    - **Public IP**: Easiest for starting, but secure it with authorized networks if possible.
    - **User/Pass**: Create a user (e.g., `wager_admin`) and password.
    - **Database**: Create a database named `wager_db`.

3.  **Get Connection Name**:
    - Find the **Instance Connection Name** (e.g., `project:region:instance`). You'll need this.

## Step 4: Deploy to Cloud Run

You can deploy using the CLI. You need to provide the environment variables.

### Option A: Direct Connection (Simplest for Dev)

If your DB has a public IP, you can just use the connection string.

```bash
gcloud run deploy wager-api \
  --image gcr.io/[PROJECT_ID]/wager-api \
  --platform managed \
  --region us-central1 \
  --allow-unauthenticated \
  --set-env-vars DATABASE_URL="postgres://user:pass@IP:5432/wager_db" \
  --set-env-vars SOLANA_RPC_URL="https://api.devnet.solana.com" \
  --set-env-vars WAGER_PROGRAM_ID="Dj2Hot5XJLv9S724BRkWohrhUfzLFERBnZJ9da2WBJQK" \
  --set-env-vars AUTH_ADMIN_TOKEN="[YOUR_SECRET_TOKEN]" \
  --set-env-vars AUTH_JWT_SECRET="[YOUR_JWT_SECRET]"
```

### Option B: Cloud SQL Proxy (Recommended for Prod)

Cloud Run allows secure access to Cloud SQL without exposing public IPs via the Unix socket unique to the instance.

1.  **Grant Role**: Give the Cloud Run service account the `Cloud SQL Client` role.
2.  **Deploy**:

```bash
gcloud run deploy wager-api \
  --image gcr.io/[PROJECT_ID]/wager-api \
  --platform managed \
  --region us-central1 \
  --allow-unauthenticated \
  --add-cloudsql-instances [INSTANCE_CONNECTION_NAME] \
  --set-env-vars DATABASE_URL="postgres://user:pass@/wager_db?host=/cloudsql/[INSTANCE_CONNECTION_NAME]" \
  --set-env-vars SOLANA_RPC_URL="https://api.devnet.solana.com" \
  --set-env-vars WAGER_PROGRAM_ID="Dj2Hot5XJLv9S724BRkWohrhUfzLFERBnZJ9da2WBJQK" \
  --set-env-vars AUTH_ADMIN_TOKEN="[YOUR_SECRET_TOKEN]" \
  --set-env-vars AUTH_JWT_SECRET="[YOUR_JWT_SECRET]"
```

_Note the `host=/cloudsql/...` in the DATABASE_URL. This tells `sqlx` to connect via the Unix socket._

## Step 5: Run Migrations

Since `wager-api` (Cloud Run) is stateless, you can't easily "shell in" to run `sqlx migrate`. You have two options:

1.  **Run Locally (if Public IP enabled)**:
    ```bash
    export DATABASE_URL="postgres://user:pass@[CLOUD_SQL_PUBLIC_IP]:5432/wager_db"
    sqlx migrate run
    ```
2.  **Cloud Build Job**: Create a Step in Cloud Build to run a migration container.

## Verification

After deployment, Cloud Run will give you a URL (e.g., `https://wager-api-xyz.a.run.app`).

Test it:

```bash
curl https://wager-api-xyz.a.run.app/health
```
