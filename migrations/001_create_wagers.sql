-- migrations/001_create_wagers.sql
-- Wager index table — mirrors on-chain Wager account state

CREATE EXTENSION IF NOT EXISTS "pgcrypto";

CREATE TABLE IF NOT EXISTS wagers (
    id                 UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    on_chain_address   VARCHAR(44)  NOT NULL UNIQUE,  -- Solana PDA (base58)
    wager_id           BIGINT       NOT NULL,
    initiator          VARCHAR(44)  NOT NULL,
    challenger         VARCHAR(44),
    stake_lamports     BIGINT       NOT NULL,
    description        TEXT         NOT NULL DEFAULT '',
    status             VARCHAR(20)  NOT NULL DEFAULT 'pending',
    resolution_source  VARCHAR(20)  NOT NULL DEFAULT 'arbitrator',
    resolver           VARCHAR(44)  NOT NULL,
    expiry_ts          BIGINT       NOT NULL,
    created_at         TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    resolved_at        TIMESTAMPTZ,
    winner             VARCHAR(44),
    protocol_fee_bps   SMALLINT     NOT NULL DEFAULT 100,
    oracle_feed        VARCHAR(44),
    oracle_target      BIGINT,
    dispute_opened_at  TIMESTAMPTZ,
    dispute_opener     VARCHAR(44)
);

-- Indexes for common query patterns
CREATE INDEX idx_wagers_initiator   ON wagers (initiator);
CREATE INDEX idx_wagers_challenger  ON wagers (challenger);
CREATE INDEX idx_wagers_status      ON wagers (status);
CREATE INDEX idx_wagers_created_at  ON wagers (created_at DESC);
CREATE INDEX idx_wagers_expiry_ts   ON wagers (expiry_ts);