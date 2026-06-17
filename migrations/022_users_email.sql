-- Current auth/profile code stores user emails, but older schemas do not.

ALTER TABLE users
    ADD COLUMN IF NOT EXISTS email TEXT;

CREATE INDEX IF NOT EXISTS idx_users_email_lower
    ON users (lower(email))
    WHERE email IS NOT NULL;
