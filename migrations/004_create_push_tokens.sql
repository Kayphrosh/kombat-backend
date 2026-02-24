-- migrations/004_create_push_tokens.sql

CREATE TABLE IF NOT EXISTS push_tokens (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    wallet_address  VARCHAR(44)  NOT NULL,
    expo_token      TEXT         NOT NULL,
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    UNIQUE(wallet_address, expo_token)
);

CREATE INDEX idx_push_tokens_wallet ON push_tokens (wallet_address);
