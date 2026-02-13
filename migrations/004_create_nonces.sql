-- migrations/004_create_nonces.sql

CREATE TABLE IF NOT EXISTS nonces (
    id          UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    wallet      VARCHAR(44)  NOT NULL,
    nonce       TEXT         NOT NULL,
    used        BOOLEAN      NOT NULL DEFAULT FALSE,
    expires_at  TIMESTAMPTZ  NOT NULL,
    created_at  TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_nonces_wallet ON nonces (wallet);
CREATE INDEX idx_nonces_unused ON nonces (wallet) WHERE (used = FALSE);
