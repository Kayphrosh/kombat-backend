-- Semantic wallet activity used by the Sui dashboard.
-- Stake lifecycle rows are derived from pool_stakes; this ledger stores
-- confirmed wallet-level events such as funding and withdrawals.

CREATE TABLE IF NOT EXISTS wallet_events (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    wallet          TEXT NOT NULL,
    network         VARCHAR(30) NOT NULL DEFAULT 'testnet',
    kind            VARCHAR(40) NOT NULL CHECK (
        kind IN (
            'fund_wallet',
            'stake_lost',
            'stake_locked',
            'stake_won',
            'withdrawal'
        )
    ),
    title           TEXT NOT NULL,
    subtitle        TEXT,
    amount_usdc     BIGINT NOT NULL CHECK (amount_usdc >= 0),
    direction       VARCHAR(8) NOT NULL CHECK (direction IN ('in', 'out')),
    status          VARCHAR(20) CHECK (status IN ('success', 'failed', 'pending')),
    tx_hash         TEXT,
    metadata        JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_wallet_events_wallet_network_created
    ON wallet_events(wallet, network, created_at DESC);

CREATE UNIQUE INDEX IF NOT EXISTS idx_wallet_events_wallet_network_tx_kind
    ON wallet_events(wallet, network, tx_hash, kind)
    WHERE tx_hash IS NOT NULL;
