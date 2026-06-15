# Deployment

The backend is now Dynamic/Sui-oriented and no longer requires Solana RPC or an Anchor deployment.

## Required Environment

```env
DATABASE_URL=postgres://user:password@host:5432/wager_db
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
RUST_LOG=wager_api=info,tower_http=info
```

Transak is optional fallback only. Leave `TRANSAK_*` unset unless you later register and enable a Transak account for supported regions.

## Local Run

```bash
cd app
cargo run
```

## Docker

```bash
docker compose up --build
```

## Database

Run the SQL migrations in `migrations/` against the configured Postgres database.

## Sui Contract

The Sui Move package for tournament staking is expected to be added separately. Once available, deployment docs should include package publishing, package ID configuration, and event indexing setup.
