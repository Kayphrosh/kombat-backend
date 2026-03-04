-- Migration: Rename stake_lamports to stake_usdc
-- This migration supports the switch from SOL (lamports) to USDC (micro-USDC, 6 decimals)
-- 1 USDC = 1_000_000 micro-USDC

-- Rename the column from stake_lamports to stake_usdc
ALTER TABLE wagers RENAME COLUMN stake_lamports TO stake_usdc;

-- Add a comment to document the change
COMMENT ON COLUMN wagers.stake_usdc IS 'USDC stake amount in micro-USDC (6 decimals). 1 USDC = 1,000,000 micro-USDC';
