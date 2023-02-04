-- Disable foreign keys so we can add non-null defaults
-- See <https://www.sqlite.org/foreignkeys.html#fk_schemacommands> for details
-- These cannot be in a transaction
PRAGMA foreign_keys = OFF;

BEGIN TRANSACTION;

CREATE TABLE accounts (
    id_str TEXT PRIMARY KEY NOT NULL,
    user_name TEXT NOT NULL,
    display_name TEXT NOT NULL
) STRICT;

-- Default unknown account
INSERT INTO accounts VALUES('0', '<Unknown>', '<Unknown>');

-- Existing tweets reference this, will be updated in application
ALTER TABLE tweets ADD COLUMN account_id TEXT NOT NULL DEFAULT '0' REFERENCES accounts(id_str);

COMMIT TRANSACTION;

-- Re-enable foreign keys, the above should ensure consistency
PRAGMA foreign_keys = ON;
