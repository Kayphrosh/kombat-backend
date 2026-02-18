-- migrations/005_create_notification_settings.sql

CREATE TABLE IF NOT EXISTS notification_settings (
    user_wallet  VARCHAR(44)  PRIMARY KEY,
    challenges   BOOLEAN      NOT NULL DEFAULT TRUE,
    funds        BOOLEAN      NOT NULL DEFAULT TRUE,
    disputes     BOOLEAN      NOT NULL DEFAULT TRUE,
    marketing    BOOLEAN      NOT NULL DEFAULT FALSE
);
