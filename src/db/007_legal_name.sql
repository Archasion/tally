-- Version 7: your legal name, when you trade under a different one.

ALTER TABLE business ADD COLUMN legal_name TEXT NOT NULL DEFAULT '';
