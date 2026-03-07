-- migrations/012_add_match_to_wagers.sql
-- Link P2P wagers to matches (optional - for tournament-based P2P wagers)

-- Add match reference to existing wagers table
ALTER TABLE wagers ADD COLUMN IF NOT EXISTS match_id UUID REFERENCES matches(id);
ALTER TABLE wagers ADD COLUMN IF NOT EXISTS match_opponent_id UUID REFERENCES match_opponents(id);

-- Index for finding wagers by match
CREATE INDEX IF NOT EXISTS idx_wagers_match ON wagers(match_id);

-- Add comment
COMMENT ON COLUMN wagers.match_id IS 'Optional reference to a PandaScore match for tournament-linked wagers';
COMMENT ON COLUMN wagers.match_opponent_id IS 'Which opponent the initiator picked for this wager';
