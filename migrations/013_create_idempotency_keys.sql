CREATE TABLE IF NOT EXISTS idempotency_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    scope VARCHAR(64) NOT NULL,
    wallet VARCHAR(44) NOT NULL,
    request_hash VARCHAR(128) NOT NULL,
    response_json JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (scope, wallet, request_hash)
);

CREATE INDEX IF NOT EXISTS idx_idempotency_keys_scope_wallet_created_at
    ON idempotency_keys (scope, wallet, created_at DESC);
