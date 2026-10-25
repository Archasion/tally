-- Version 8: the name to greet a client by in emails.

ALTER TABLE clients ADD COLUMN email_name TEXT NOT NULL DEFAULT '';
