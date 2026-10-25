-- Version 6: an issued invoice's business and client details can be updated
-- to the current ones (e.g. to add an IBAN), but only on the record: the
-- update must set details_updated_on, and the client label used for its
-- number can't change. Everything else stays locked as before.

ALTER TABLE invoices ADD COLUMN details_updated_on TEXT;

DROP TRIGGER invoices_lock_update;
CREATE TRIGGER invoices_lock_update
BEFORE UPDATE OF number, client_id, issue_date, due_date, supply_start, supply_end, notes, issued_at
ON invoices WHEN OLD.issued_at IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'invoice is issued and locked');
END;

CREATE TRIGGER invoices_lock_snapshot
BEFORE UPDATE OF snapshot ON invoices
WHEN OLD.issued_at IS NOT NULL AND (
    NEW.details_updated_on IS NULL
    OR NEW.details_updated_on IS OLD.details_updated_on AND NEW.snapshot IS NOT OLD.snapshot
    OR json_extract(NEW.snapshot, '$.client.label') IS NOT json_extract(OLD.snapshot, '$.client.label')
)
BEGIN
    SELECT RAISE(ABORT, 'issued invoice details can only be updated on the record, keeping the client label');
END;
