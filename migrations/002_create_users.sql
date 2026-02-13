-- migrations/002_create_users.sql

CREATE TABLE IF NOT EXISTS users (
    id              UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    wallet_address  VARCHAR(44)  NOT NULL UNIQUE,
    display_name    VARCHAR(64),
    avatar_url      TEXT,
    wins            INTEGER      NOT NULL DEFAULT 0,
    losses          INTEGER      NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_users_wallet ON users (wallet_address);