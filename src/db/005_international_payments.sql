-- Version 5: bank details for clients paying from abroad.

ALTER TABLE business ADD COLUMN bank_iban    TEXT NOT NULL DEFAULT '';
ALTER TABLE business ADD COLUMN bank_bic     TEXT NOT NULL DEFAULT '';
ALTER TABLE business ADD COLUMN bank_name    TEXT NOT NULL DEFAULT '';
ALTER TABLE business ADD COLUMN bank_address TEXT NOT NULL DEFAULT '';
