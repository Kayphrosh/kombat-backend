-- migrations/010_create_match_opponents.sql
-- Opponents (teams or players) for each match
-- These are the betting options

CREATE TABLE IF NOT EXISTS match_opponents (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    match_id            UUID NOT NULL REFERENCES matches(id) ON DELETE CASCADE,
    
    -- PandaScore opponent info
    pandascore_id       INTEGER NOT NULL,             -- Team or player ID
    opponent_type       VARCHAR(20) NOT NULL,         -- "Team" or "Player"
    
    -- Display info
    name                VARCHAR(256) NOT NULL,
    acronym             VARCHAR(20),                  -- Team acronym e.g., "T1"
    image_url           TEXT,
    
    -- Location info (for teams)
    location            VARCHAR(100),
    
    -- Position in match (0 or 1)
    position            SMALLINT NOT NULL DEFAULT 0,
    
    -- Is this the winner? (set when match resolves)
    is_winner           BOOLEAN,
    
    -- Timestamps
    created_at          TIMESTAMPTZ DEFAULT NOW()
);

-- Indexes
CREATE INDEX idx_opponents_match ON match_opponents(match_id);
CREATE INDEX idx_opponents_pandascore ON match_opponents(pandascore_id);
CREATE UNIQUE INDEX idx_opponents_match_position ON match_opponents(match_id, position);
