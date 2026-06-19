-- Make GRID the default data source for any future match insert that does not
-- explicitly set source. Existing rows keep their current source value.
ALTER TABLE matches
    ALTER COLUMN source SET DEFAULT 'grid';
