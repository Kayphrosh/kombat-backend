-- migrations/006_add_declared_winners.sql

ALTER TABLE wagers ADD COLUMN IF NOT EXISTS creator_declared_winner TEXT;
ALTER TABLE wagers ADD COLUMN IF NOT EXISTS challenger_declared_winner TEXT;
