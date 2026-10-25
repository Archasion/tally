-- Version 9: an optional third address line for you and your clients.

ALTER TABLE business ADD COLUMN address_line3 TEXT NOT NULL DEFAULT '';
ALTER TABLE clients  ADD COLUMN address_line3 TEXT NOT NULL DEFAULT '';
