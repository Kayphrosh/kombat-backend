-- migrations/007_create_dispute_submissions.sql
-- Each wager dispute can have a submission from both participants.
-- UPSERT on (wager_address, submitter) allows updating an existing submission.

CREATE TABLE IF NOT EXISTS dispute_submissions (
    id               UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    wager_address    VARCHAR(44)  NOT NULL,
    submitter        VARCHAR(44)  NOT NULL,
    description      TEXT         NOT NULL DEFAULT '',
    evidence_url     TEXT,
    declared_winner  VARCHAR(44),
    created_at       TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at       TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    UNIQUE (wager_address, submitter)
);

CREATE INDEX idx_dispute_submissions_wager ON dispute_submissions (wager_address);
