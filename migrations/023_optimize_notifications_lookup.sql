-- Keep notification inbox queries fast for Sui-length wallet addresses.

ALTER TABLE notifications ALTER COLUMN user_wallet TYPE TEXT;

CREATE INDEX IF NOT EXISTS idx_notifications_user_created_at
    ON notifications (user_wallet, created_at DESC);
