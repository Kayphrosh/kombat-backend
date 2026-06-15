-- Secondary market for StakeReceipt objects.
-- Sellers exit before settlement by listing their receipt; buyers enter by
-- atomically paying USDC and receiving the receipt object on Sui.

ALTER TABLE pool_stakes
    ADD COLUMN IF NOT EXISTS stake_receipt_id TEXT;

CREATE INDEX IF NOT EXISTS idx_pool_stakes_receipt_id
    ON pool_stakes(stake_receipt_id)
    WHERE stake_receipt_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS receipt_market_listings (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    network             VARCHAR(30) NOT NULL DEFAULT 'testnet',
    seller_wallet       TEXT NOT NULL,
    buyer_wallet        TEXT,

    receipt_id          TEXT NOT NULL,
    listing_object_id   TEXT,
    match_id            UUID NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    opponent_id         UUID NOT NULL REFERENCES match_opponents(id) ON DELETE CASCADE,

    ask_amount_usdc     BIGINT NOT NULL CHECK (ask_amount_usdc > 0),
    status              VARCHAR(30) NOT NULL DEFAULT 'draft',
    listing_tx_hash     VARCHAR(100),
    sale_tx_hash        VARCHAR(100),
    metadata            JSONB NOT NULL DEFAULT '{}'::jsonb,

    expires_at          TIMESTAMPTZ NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_receipt_market_match_status
    ON receipt_market_listings(match_id, status, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_receipt_market_seller_created
    ON receipt_market_listings(seller_wallet, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_receipt_market_receipt_status
    ON receipt_market_listings(receipt_id, status);
