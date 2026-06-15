-- Per-agent identity for outcome agents. Lets us map each run to the specific
-- agent token that submitted it and audit individual agent accuracy over time.

ALTER TABLE agent_runs
    ADD COLUMN IF NOT EXISTS agent_id VARCHAR(100),
    ADD COLUMN IF NOT EXISTS verification_status VARCHAR(30),
    ADD COLUMN IF NOT EXISTS verification_note TEXT;

CREATE INDEX IF NOT EXISTS idx_agent_runs_agent_id
    ON agent_runs(agent_id, created_at DESC);
