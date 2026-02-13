-- migrations/003_create_notifications.sql

CREATE TABLE IF NOT EXISTS notifications (
    id          UUID         PRIMARY KEY DEFAULT gen_random_uuid(),
    user_wallet VARCHAR(44)  NOT NULL,
    kind        VARCHAR(64)  NOT NULL,
    payload     JSONB,
    is_read     BOOLEAN      NOT NULL DEFAULT FALSE,
    created_at  TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_notifications_user ON notifications (user_wallet);
CREATE INDEX idx_notifications_user_unread ON notifications (user_wallet) WHERE (is_read = FALSE);
