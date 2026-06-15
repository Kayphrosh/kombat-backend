# Kombat Sui Package

Sui Move package for tournament pool staking.

## Contract Shape

- `AdminCap`: deployment admin capability.
- `TournamentPool<CoinType>`: shared pool object for one match.
- `StakeReceipt<CoinType>`: owned receipt minted to a user when they stake.

The contract is generic over `CoinType`, so the same package can be used with testnet USDC, mainnet USDC, or another coin in development.

## Main Calls

- `create_pool<CoinType>`: admin creates a shared pool for one tournament or match.
- `stake<CoinType>`: user deposits a `Coin<CoinType>` and receives a `StakeReceipt`.
- `resolve<CoinType>`: admin resolves the pool with outcome A or B.
- `cancel<CoinType>`: admin cancels before resolution.
- `claim<CoinType>`: winning user consumes their receipt and receives payout.
- `refund<CoinType>`: user consumes their receipt and receives a refund for cancelled or one-sided pools.
- `list_receipt<CoinType>`: seller escrows a `StakeReceipt` in a shared listing before settlement.
- `buy_receipt<CoinType>`: buyer atomically pays the seller and receives the `StakeReceipt`.
- `cancel_receipt_listing<CoinType>`: seller cancels a listing and receives the receipt back.

## Build

Install the Sui CLI, then run:

```bash
cd sui
sui move build
```

## Publish

```bash
cd sui
sui client publish --gas-budget 100000000
```

After publishing, set the package ID in the backend:

```env
SUI_TESTNET_PACKAGE_ID=<published-testnet-package-id>
SUI_MAINNET_PACKAGE_ID=<published-mainnet-package-id>
```

After creating a tournament pool, store the shared pool object ID on the matching backend row as `matches.sui_pool_object_id`. The Smart Pay PTB endpoint uses that object ID to produce the frontend `tournament_staking::stake<USDC>` plan.

## Frontend PTB Direction

The frontend should use Dynamic's embedded Sui wallet to sign PTBs. The backend Smart Pay endpoint `GET /api/payments/intents/:id/ptb` returns the required Move call metadata:

1. Split the user's USDC coin to the stake amount.
2. Call `kombat::tournament_staking::stake<USDC>`.
3. Let Dynamic request user approval.
4. Submit the transaction.
5. Backend indexes `StakePlaced` and updates app views.

For Web2 users, add sponsored transactions so they do not need to hold SUI for gas.

## Receipt Market Direction

Trading in/out is implemented as receipt transfer, not early pool withdrawal:

1. Seller signs `list_receipt<USDC>` to escrow a `StakeReceipt` in a shared `ReceiptListing`.
2. Buyer signs `buy_receipt<USDC>` with exact USDC payment.
3. Move transfers payment to seller, updates receipt owner, and transfers the receipt to buyer.
4. The buyer now owns the future claim/refund right.
