-- migrations/005_add_initiator_option.sql

ALTER TABLE wagers ADD COLUMN initiator_option VARCHAR(3) DEFAULT 'yes';
