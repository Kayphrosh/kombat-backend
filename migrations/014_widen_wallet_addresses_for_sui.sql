-- Sui addresses are canonical 0x-prefixed 32-byte hex strings, which are
-- longer than the previous Solana-oriented VARCHAR(44) columns.

ALTER TABLE users ALTER COLUMN wallet_address TYPE TEXT;
ALTER TABLE notifications ALTER COLUMN user_wallet TYPE TEXT;
ALTER TABLE notification_settings ALTER COLUMN user_wallet TYPE TEXT;
ALTER TABLE pool_stakes ALTER COLUMN user_wallet TYPE TEXT;

ALTER TABLE idempotency_keys ALTER COLUMN wallet TYPE TEXT;

-- Legacy Kombat wager tables remain for historical data during the Sui
-- migration, but their address columns must also be able to hold Sui object
-- IDs and Sui wallet addresses if reused by compatibility views.
ALTER TABLE wagers ALTER COLUMN on_chain_address TYPE TEXT;
ALTER TABLE wagers ALTER COLUMN initiator TYPE TEXT;
ALTER TABLE wagers ALTER COLUMN challenger TYPE TEXT;
ALTER TABLE wagers ALTER COLUMN resolver TYPE TEXT;
ALTER TABLE wagers ALTER COLUMN winner TYPE TEXT;
ALTER TABLE wagers ALTER COLUMN oracle_feed TYPE TEXT;
ALTER TABLE wagers ALTER COLUMN dispute_opener TYPE TEXT;

ALTER TABLE dispute_submissions ALTER COLUMN wager_address TYPE TEXT;
ALTER TABLE dispute_submissions ALTER COLUMN submitter TYPE TEXT;
ALTER TABLE dispute_submissions ALTER COLUMN declared_winner TYPE TEXT;
