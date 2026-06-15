# Google Cloud Run Deployment

Build and deploy the backend container with Dynamic/Sui configuration.

```bash
gcloud builds submit --tag gcr.io/[PROJECT_ID]/wager-api
```

```bash
gcloud run deploy wager-api \
  --image gcr.io/[PROJECT_ID]/wager-api \
  --platform managed \
  --region [REGION] \
  --allow-unauthenticated \
  --set-env-vars DATABASE_URL="postgres://user:pass@host:5432/wager_db" \
  --set-env-vars AUTH_JWT_SECRET="[LONG_RANDOM_SECRET]" \
  --set-env-vars DYNAMIC_ENVIRONMENT_ID="[DYNAMIC_ENVIRONMENT_ID]" \
  --set-env-vars SUI_NETWORK="testnet" \
  --set-env-vars SUI_TESTNET_RPC_URL="https://fullnode.testnet.sui.io:443" \
  --set-env-vars SUI_MAINNET_RPC_URL="https://fullnode.mainnet.sui.io:443" \
  --set-env-vars RAMP_PRIMARY_PROVIDER="dynamic_native" \
  --set-env-vars DYNAMIC_ONRAMP_ENABLED="true" \
  --set-env-vars MANUAL_CRYPTO_DEPOSIT_ENABLED="true" \
  --set-env-vars RAMP_DEFAULT_NETWORK="sui" \
  --set-env-vars RAMP_DEFAULT_CRYPTO_CURRENCY="USDC" \
  --set-env-vars RAMP_DEFAULT_FIAT_CURRENCY="USD" \
  --set-env-vars RAMP_PARTNER_FEE_BPS="0" \
  --set-env-vars RUST_LOG="wager_api=info,tower_http=info"
```

No Solana RPC environment variable is required. Transak variables are optional fallback only; Dynamic native on-ramp support should be configured in the Dynamic dashboard.
