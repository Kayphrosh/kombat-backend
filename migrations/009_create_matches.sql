-- migrations/009_create_matches.sql
-- Esports matches from PandaScore (the betting events)
-- Note: We call them "matches" to align with PandaScore terminology
-- but display them as "Tournaments" in the UI

CREATE TABLE IF NOT EXISTS matches (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    
    -- PandaScore identifiers
    pandascore_id       BIGINT UNIQUE,                -- PandaScore match ID
    slug                VARCHAR(256),                 -- PandaScore slug
    
    -- Match info
    name                VARCHAR(512) NOT NULL,        -- e.g., "T1 vs Gen.G"
    
    -- Videogame info
    videogame_id        INTEGER,
    videogame_name      VARCHAR(100),                 -- e.g., "League of Legends"
    videogame_slug      VARCHAR(100),                 -- e.g., "league-of-legends"
    
    -- League info (parent)
    league_id           INTEGER,
    league_name         VARCHAR(256),
    league_slug         VARCHAR(256),
    league_image_url    TEXT,
    
    -- Series info
    series_id           INTEGER,
    series_name         VARCHAR(256),
    series_full_name    VARCHAR(512),
    
    -- Tournament info (parent of match)
    tournament_id       INTEGER,
    tournament_name     VARCHAR(256),
    tournament_slug     VARCHAR(256),
    
    -- Match timing
    scheduled_at        TIMESTAMPTZ,                  -- When match is scheduled
    begin_at            TIMESTAMPTZ,                  -- When match actually started
    end_at              TIMESTAMPTZ,                  -- When match ended
    
    -- Match format
    match_type          VARCHAR(50),                  -- e.g., "best_of"
    number_of_games     INTEGER,                      -- e.g., 3 for best of 3
    
    -- Match status (from PandaScore)
    -- not_started, running, finished, canceled, postponed
    pandascore_status   VARCHAR(30) DEFAULT 'not_started',
    
    -- Our internal status for staking
    -- upcoming: accepting stakes
    -- live: staking locked, match in progress
    -- completed: match finished, payouts done
    -- cancelled: match cancelled, stakes refunded
    -- refunded: stakes refunded (one-sided pool, etc.)
    status              VARCHAR(30) DEFAULT 'upcoming',
    
    -- Winner info (set when match finishes)
    winner_id           INTEGER,                      -- PandaScore team/player ID
    winner_type         VARCHAR(20),                  -- "Team" or "Player"
    forfeit             BOOLEAN DEFAULT FALSE,
    
    -- Match streams
    streams_list        JSONB,                        -- Array of stream URLs
    
    -- Detailed stats available?
    detailed_stats      BOOLEAN DEFAULT FALSE,
    
    -- Full PandaScore response for reference
    raw_data            JSONB,
    
    -- Timestamps
    created_at          TIMESTAMPTZ DEFAULT NOW(),
    updated_at          TIMESTAMPTZ DEFAULT NOW()
);

-- Indexes
CREATE INDEX idx_matches_pandascore_id ON matches(pandascore_id);
CREATE INDEX idx_matches_status ON matches(status);
CREATE INDEX idx_matches_scheduled_at ON matches(scheduled_at);
CREATE INDEX idx_matches_videogame ON matches(videogame_slug);
CREATE INDEX idx_matches_league ON matches(league_id);
