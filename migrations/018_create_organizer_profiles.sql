-- Organizer authentication and KYC state.
-- Dynamic proves wallet ownership; this table controls whether that wallet can
-- create organizer markets and submit organizer-backed outcomes.

CREATE TABLE IF NOT EXISTS organizer_profiles (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    wallet_address      TEXT NOT NULL UNIQUE,
    organization_name   VARCHAR(256) NOT NULL,
    contact_email       VARCHAR(256),
    website_url         TEXT,
    country             VARCHAR(100),
    description         TEXT,

    status              VARCHAR(30) NOT NULL DEFAULT 'pending',
    kyc_status          VARCHAR(30) NOT NULL DEFAULT 'not_started',
    kyc_provider        VARCHAR(60),
    kyc_reference_id    TEXT,
    kyc_session_url     TEXT,

    rejection_reason    TEXT,
    reviewed_by         TEXT,
    reviewed_at         TIMESTAMPTZ,
    metadata            JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_organizer_profiles_status
    ON organizer_profiles(status, kyc_status);
CREATE INDEX IF NOT EXISTS idx_organizer_profiles_country
    ON organizer_profiles(country);
