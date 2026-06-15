-- Walrus-backed artifacts and autonomous outcome agent runs.
-- Walrus stores durable public evidence/manifests; Postgres indexes them for
-- frontend lookup, admin review, and proposal traceability.

CREATE TABLE IF NOT EXISTS walrus_artifacts (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    blob_id              TEXT NOT NULL,
    object_id            TEXT,
    artifact_type        VARCHAR(40) NOT NULL,
    owner_wallet         TEXT,
    match_id             UUID REFERENCES matches(id) ON DELETE SET NULL,
    outcome_proposal_id  UUID REFERENCES outcome_proposals(id) ON DELETE SET NULL,
    content_type         TEXT NOT NULL DEFAULT 'application/json',
    size_bytes           BIGINT NOT NULL DEFAULT 0,
    aggregator_url       TEXT,
    publisher_url        TEXT,
    metadata             JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_walrus_artifacts_blob_id
    ON walrus_artifacts(blob_id);
CREATE INDEX IF NOT EXISTS idx_walrus_artifacts_match_created
    ON walrus_artifacts(match_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_walrus_artifacts_owner_created
    ON walrus_artifacts(owner_wallet, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_walrus_artifacts_type
    ON walrus_artifacts(artifact_type);

CREATE TABLE IF NOT EXISTS agent_runs (
    id                          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    match_id                    UUID REFERENCES matches(id) ON DELETE SET NULL,
    agent_name                  VARCHAR(100) NOT NULL,
    status                      VARCHAR(30) NOT NULL DEFAULT 'queued',
    watch_sources               JSONB NOT NULL DEFAULT '[]'::jsonb,
    evidence_blob_id            TEXT,
    evidence_url                TEXT,
    outcome_proposal_id         UUID REFERENCES outcome_proposals(id) ON DELETE SET NULL,
    proposed_winner_opponent_id UUID REFERENCES match_opponents(id) ON DELETE SET NULL,
    proposed_winner_name        VARCHAR(256),
    confidence                  DECIMAL(5, 4),
    summary                     TEXT,
    error                       TEXT,
    raw_output                  JSONB NOT NULL DEFAULT '{}'::jsonb,
    started_at                  TIMESTAMPTZ,
    completed_at                TIMESTAMPTZ,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_agent_runs_match_created
    ON agent_runs(match_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_agent_runs_status_created
    ON agent_runs(status, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_agent_runs_agent_name
    ON agent_runs(agent_name);
