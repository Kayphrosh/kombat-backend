-- Organizer-created tournaments and verifiable outcome proposals.
-- This lets Kombat support games/events outside PandaScore while preserving
-- matches as the stakeable market primitive.

CREATE TABLE IF NOT EXISTS organizer_tournaments (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organizer_wallet    TEXT NOT NULL,
    name                VARCHAR(512) NOT NULL,
    videogame_name      VARCHAR(100),
    videogame_slug      VARCHAR(100),
    description         TEXT,
    rules_blob_id       TEXT,
    bracket_blob_id     TEXT,
    evidence_blob_id    TEXT,
    status              VARCHAR(30) NOT NULL DEFAULT 'draft',
    starts_at           TIMESTAMPTZ,
    ends_at             TIMESTAMPTZ,
    metadata            JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_organizer_tournaments_wallet_created
    ON organizer_tournaments(organizer_wallet, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_organizer_tournaments_status
    ON organizer_tournaments(status);
CREATE INDEX IF NOT EXISTS idx_organizer_tournaments_game
    ON organizer_tournaments(videogame_slug);

ALTER TABLE matches
    ADD COLUMN IF NOT EXISTS source VARCHAR(30) NOT NULL DEFAULT 'pandascore',
    ADD COLUMN IF NOT EXISTS organizer_tournament_id UUID REFERENCES organizer_tournaments(id) ON DELETE SET NULL,
    ADD COLUMN IF NOT EXISTS organizer_wallet TEXT,
    ADD COLUMN IF NOT EXISTS result_status VARCHAR(30) NOT NULL DEFAULT 'pending',
    ADD COLUMN IF NOT EXISTS rules_blob_id TEXT,
    ADD COLUMN IF NOT EXISTS bracket_blob_id TEXT,
    ADD COLUMN IF NOT EXISTS evidence_blob_id TEXT,
    ADD COLUMN IF NOT EXISTS evidence_summary TEXT,
    ADD COLUMN IF NOT EXISTS verification_status VARCHAR(30) NOT NULL DEFAULT 'unverified';

CREATE INDEX IF NOT EXISTS idx_matches_source_status
    ON matches(source, status);
CREATE INDEX IF NOT EXISTS idx_matches_organizer_tournament
    ON matches(organizer_tournament_id, scheduled_at);
CREATE INDEX IF NOT EXISTS idx_matches_organizer_wallet
    ON matches(organizer_wallet, created_at DESC);

CREATE TABLE IF NOT EXISTS outcome_proposals (
    id                          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    match_id                    UUID NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    proposed_winner_opponent_id UUID REFERENCES match_opponents(id) ON DELETE SET NULL,
    proposed_winner_name        VARCHAR(256),
    source                      VARCHAR(30) NOT NULL,
    proposer_wallet             TEXT,
    confidence                  DECIMAL(5, 4),
    status                      VARCHAR(30) NOT NULL DEFAULT 'pending',
    evidence_blob_id            TEXT,
    evidence_url                TEXT,
    evidence_summary            TEXT,
    raw_data                    JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    reviewed_at                 TIMESTAMPTZ,
    reviewer_wallet             TEXT
);

CREATE INDEX IF NOT EXISTS idx_outcome_proposals_match_created
    ON outcome_proposals(match_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_outcome_proposals_status
    ON outcome_proposals(status);
