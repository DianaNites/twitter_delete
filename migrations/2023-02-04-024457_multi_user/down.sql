BEGIN TRANSACTION;

ALTER TABLE tweets DROP COLUMN account_id;

DROP TABLE accounts;

COMMIT TRANSACTION;
