-- Surface best-effort on-chain wager resolution failures to clients.

ALTER TABLE wagers
    ADD COLUMN IF NOT EXISTS resolution_error TEXT,
    ADD COLUMN IF NOT EXISTS resolution_attempted_at TIMESTAMPTZ;

COMMENT ON COLUMN wagers.resolution_error IS
    'Last backend on-chain resolution error, if mutual-consent social resolution succeeded but signer/RPC settlement failed.';

COMMENT ON COLUMN wagers.resolution_attempted_at IS
    'Timestamp of the last backend on-chain resolution attempt.';
