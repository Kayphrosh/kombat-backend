-- migrations/011_create_pool_stakes.sql
-- User stakes on match outcomes (parimutuel betting)

CREATE TABLE IF NOT EXISTS pool_stakes (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    
    -- What match and which side
    match_id            UUID NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    opponent_id         UUID NOT NULL REFERENCES match_opponents(id) ON DELETE CASCADE,
    
    -- Who staked
    user_wallet         VARCHAR(44) NOT NULL,
    
    -- Stake amount (micro-USDC, 6 decimals)
    amount_usdc         BIGINT NOT NULL CHECK (amount_usdc > 0),
    
    -- Odds at time of stake (for reference/display)
    odds_at_stake       DECIMAL(10, 4),
    
    -- Status
    -- active: stake is live
    -- won: user won, payout calculated
    -- lost: user lost
    -- refunded: stake returned (match cancelled, one-sided pool, etc.)
    status              VARCHAR(20) DEFAULT 'active',
    
    -- Payout (set when match resolves and user wins)
    payout_usdc         BIGINT,
    
    -- Transaction hashes (if on-chain)
    stake_tx_hash       VARCHAR(100),
    payout_tx_hash      VARCHAR(100),
    
    -- Timestamps
    created_at          TIMESTAMPTZ DEFAULT NOW(),
    resolved_at         TIMESTAMPTZ
);

-- Indexes
CREATE INDEX idx_stakes_match ON pool_stakes(match_id);
CREATE INDEX idx_stakes_opponent ON pool_stakes(opponent_id);
CREATE INDEX idx_stakes_user ON pool_stakes(user_wallet);
CREATE INDEX idx_stakes_status ON pool_stakes(status);
CREATE INDEX idx_stakes_created ON pool_stakes(created_at DESC);

-- Composite index for user's stakes on a match
CREATE INDEX idx_stakes_user_match ON pool_stakes(user_wallet, match_id);
