-- migrations/003_add_user_email.sql

ALTER TABLE users ADD COLUMN email VARCHAR(255);
