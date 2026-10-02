-- Version 2: international clients and per-client number labels.

ALTER TABLE clients ADD COLUMN country      TEXT NOT NULL DEFAULT 'United Kingdom';
ALTER TABLE clients ADD COLUMN label        TEXT NOT NULL DEFAULT '';
ALTER TABLE clients ADD COLUMN tax_id_label TEXT NOT NULL DEFAULT '';
ALTER TABLE clients ADD COLUMN tax_id       TEXT NOT NULL DEFAULT '';

-- Labels are optional, but no two clients may share one.
CREATE UNIQUE INDEX clients_label_unique ON clients(label) WHERE label <> '';

-- A label that appears on an issued invoice or credit note can't change,
-- so that client's numbering stays continuous. Setting a first label on a
-- client whose earlier invoices had none is allowed.
CREATE TRIGGER clients_label_locked
BEFORE UPDATE OF label ON clients
WHEN OLD.label <> '' AND NEW.label <> OLD.label AND (
    EXISTS (SELECT 1 FROM invoices
            WHERE client_id = OLD.id AND issued_at IS NOT NULL
              AND json_extract(snapshot, '$.client.label') = OLD.label)
    OR EXISTS (SELECT 1 FROM credit_notes n JOIN invoices i ON i.id = n.invoice_id
               WHERE i.client_id = OLD.id
                 AND json_extract(n.snapshot, '$.client.label') = OLD.label)
)
BEGIN
    SELECT RAISE(ABORT, 'client label is locked because it is used on an issued invoice');
END;
