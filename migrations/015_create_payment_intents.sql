-- Payment intents turn a user payment into a programmable staking action.
-- They let the app calculate funding shortfall, preserve a wallet reserve,
-- and hand the frontend a Sui PTB plan for the final stake.

ALTER TABLE matches
    ADD COLUMN IF NOT EXISTS sui_network VARCHAR(30),
    ADD COLUMN IF NOT EXISTS sui_pool_object_id TEXT;

CREATE INDEX IF NOT EXISTS idx_matches_sui_pool_object
    ON matches(sui_pool_object_id)
    WHERE sui_pool_object_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS payment_intents (
    id                      UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_wallet             TEXT NOT NULL,
    kind                    VARCHAR(40) NOT NULL DEFAULT 'STAKE_TOURNAMENT',
    status                  VARCHAR(30) NOT NULL DEFAULT 'requires_funding',
    network                 VARCHAR(30) NOT NULL DEFAULT 'testnet',

    match_id                UUID NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    opponent_id             UUID NOT NULL REFERENCES match_opponents(id) ON DELETE CASCADE,

    amount_usdc             BIGINT NOT NULL CHECK (amount_usdc > 0),
    reserve_balance_usdc    BIGINT NOT NULL DEFAULT 0 CHECK (reserve_balance_usdc >= 0),
    settlement_rule         VARCHAR(40) NOT NULL DEFAULT 'return_to_wallet',

    current_balance_usdc    BIGINT,
    funding_shortfall_usdc  BIGINT NOT NULL DEFAULT 0 CHECK (funding_shortfall_usdc >= 0),

    stake_tx_hash           VARCHAR(100),
    stake_receipt_id        TEXT,
    metadata                JSONB NOT NULL DEFAULT '{}'::jsonb,

    expires_at              TIMESTAMPTZ NOT NULL DEFAULT (NOW() + INTERVAL '30 minutes'),
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_payment_intents_wallet_created
    ON payment_intents(user_wallet, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_payment_intents_status
    ON payment_intents(status);
CREATE INDEX IF NOT EXISTS idx_payment_intents_match
    ON payment_intents(match_id);
