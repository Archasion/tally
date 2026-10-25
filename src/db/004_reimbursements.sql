-- Version 4: rebillable expenses (reimbursements), billing periods and receipts.

ALTER TABLE expenses ADD COLUMN billing          TEXT NOT NULL DEFAULT 'one_off'
    CHECK (billing IN ('one_off', 'monthly', 'quarterly', 'annual'));
ALTER TABLE expenses ADD COLUMN rebill_client_id INTEGER REFERENCES clients(id);  -- NULL = not rebilled
ALTER TABLE expenses ADD COLUMN approved_by      TEXT NOT NULL DEFAULT '';
ALTER TABLE expenses ADD COLUMN approved_on      TEXT;
ALTER TABLE expenses ADD COLUMN receipt_ref      TEXT NOT NULL DEFAULT '';        -- supplier's invoice number
ALTER TABLE expenses ADD COLUMN receipt_file     TEXT NOT NULL DEFAULT '';        -- relative to the output folder

-- A line that bills an expense back to the client, and extra text printed
-- under the description. Both are frozen with the rest of the line on issue.
ALTER TABLE line_items ADD COLUMN expense_id INTEGER REFERENCES expenses(id);
ALTER TABLE line_items ADD COLUMN detail     TEXT NOT NULL DEFAULT '';
ALTER TABLE credit_note_lines ADD COLUMN expense_id INTEGER REFERENCES expenses(id);
ALTER TABLE credit_note_lines ADD COLUMN detail     TEXT NOT NULL DEFAULT '';

-- An expense can be billed on at most one invoice.
CREATE UNIQUE INDEX line_items_expense_unique ON line_items(expense_id) WHERE expense_id IS NOT NULL;

-- Once billed on an issued invoice, an expense keeps its amount and client,
-- and can't be deleted, so the books agree with what was invoiced. (The
-- foreign key from line_items also stops deleting one that's on a draft.)
CREATE TRIGGER expenses_billed_lock_update
BEFORE UPDATE OF amount_pence, rebill_client_id ON expenses
WHEN (NEW.amount_pence <> OLD.amount_pence OR NEW.rebill_client_id IS NOT OLD.rebill_client_id)
 AND EXISTS (SELECT 1 FROM line_items l JOIN invoices i ON i.id = l.invoice_id
             WHERE l.expense_id = OLD.id AND i.issued_at IS NOT NULL)
BEGIN
    SELECT RAISE(ABORT, 'expense is billed on an issued invoice');
END;

CREATE TRIGGER expenses_billed_lock_delete
BEFORE DELETE ON expenses
WHEN EXISTS (SELECT 1 FROM line_items l JOIN invoices i ON i.id = l.invoice_id
             WHERE l.expense_id = OLD.id AND i.issued_at IS NOT NULL)
BEGIN
    SELECT RAISE(ABORT, 'expense is billed on an issued invoice');
END;
