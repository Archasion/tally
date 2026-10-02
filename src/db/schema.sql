-- tally schema, version 1.
-- Money is stored as INTEGER pence, quantities as INTEGER hundredths,
-- dates as ISO-8601 TEXT (YYYY-MM-DD).

CREATE TABLE business (
    id                  INTEGER PRIMARY KEY CHECK (id = 1),
    name                TEXT NOT NULL,
    address_line1       TEXT NOT NULL,
    address_line2       TEXT NOT NULL DEFAULT '',
    town                TEXT NOT NULL,
    postcode            TEXT NOT NULL,
    email               TEXT NOT NULL DEFAULT '',
    phone               TEXT NOT NULL DEFAULT '',
    bank_account_name   TEXT NOT NULL,
    bank_sort_code      TEXT NOT NULL,
    bank_account_number TEXT NOT NULL,
    payment_terms_days  INTEGER NOT NULL DEFAULT 30 CHECK (payment_terms_days >= 0),
    default_notes       TEXT NOT NULL DEFAULT '',
    invoice_pattern     TEXT NOT NULL DEFAULT '{YEAR}-{SEQ:4}',
    credit_note_pattern TEXT NOT NULL DEFAULT 'CN-{YEAR}-{SEQ:3}',
    output_dir          TEXT NOT NULL,
    backups_to_keep     INTEGER NOT NULL DEFAULT 10 CHECK (backups_to_keep >= 1)
);

CREATE TABLE clients (
    id                  INTEGER PRIMARY KEY,
    name                TEXT NOT NULL,
    contact_name        TEXT NOT NULL DEFAULT '',
    email               TEXT NOT NULL DEFAULT '',
    address_line1       TEXT NOT NULL DEFAULT '',
    address_line2       TEXT NOT NULL DEFAULT '',
    town                TEXT NOT NULL DEFAULT '',
    postcode            TEXT NOT NULL DEFAULT '',
    is_business         INTEGER NOT NULL DEFAULT 1,
    late_payment_notice INTEGER NOT NULL DEFAULT 0,
    default_rate_pence  INTEGER,
    default_rate_kind   TEXT NOT NULL DEFAULT 'hours' CHECK (default_rate_kind IN ('hours', 'days', 'fixed')),
    archived            INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE invoices (
    id           INTEGER PRIMARY KEY,
    number       TEXT UNIQUE,              -- NULL while a draft
    client_id    INTEGER NOT NULL REFERENCES clients(id),
    issue_date   TEXT NOT NULL,
    due_date     TEXT NOT NULL,
    supply_start TEXT NOT NULL,
    supply_end   TEXT,                     -- NULL for a single supply date
    notes        TEXT NOT NULL DEFAULT '',
    issued_at    TEXT,                     -- NULL while a draft
    sent_on      TEXT,
    snapshot     TEXT,                     -- JSON business + client, set on issue
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    CHECK ((issued_at IS NULL) = (number IS NULL)),
    CHECK ((issued_at IS NULL) = (snapshot IS NULL))
);

CREATE TABLE line_items (
    id                INTEGER PRIMARY KEY,
    invoice_id        INTEGER NOT NULL REFERENCES invoices(id) ON DELETE CASCADE,
    position          INTEGER NOT NULL,
    kind              TEXT NOT NULL CHECK (kind IN ('hours', 'days', 'fixed')),
    description       TEXT NOT NULL,
    quantity_centi    INTEGER NOT NULL CHECK (quantity_centi > 0),
    unit_price_pence  INTEGER NOT NULL
);
CREATE INDEX line_items_invoice ON line_items(invoice_id, position);

CREATE TABLE payments (
    id           INTEGER PRIMARY KEY,
    invoice_id   INTEGER NOT NULL REFERENCES invoices(id),
    date         TEXT NOT NULL,
    amount_pence INTEGER NOT NULL CHECK (amount_pence > 0),
    method       TEXT NOT NULL DEFAULT '',
    reference    TEXT NOT NULL DEFAULT ''
);
CREATE INDEX payments_invoice ON payments(invoice_id);

CREATE TABLE refunds (
    id           INTEGER PRIMARY KEY,
    invoice_id   INTEGER NOT NULL REFERENCES invoices(id),
    date         TEXT NOT NULL,
    amount_pence INTEGER NOT NULL CHECK (amount_pence > 0),
    method       TEXT NOT NULL DEFAULT '',
    reference    TEXT NOT NULL DEFAULT ''
);
CREATE INDEX refunds_invoice ON refunds(invoice_id);

CREATE TABLE credit_notes (
    id         INTEGER PRIMARY KEY,
    number     TEXT NOT NULL UNIQUE,
    invoice_id INTEGER NOT NULL REFERENCES invoices(id),
    date       TEXT NOT NULL,
    reason     TEXT NOT NULL,
    snapshot   TEXT NOT NULL
);
CREATE INDEX credit_notes_invoice ON credit_notes(invoice_id);

CREATE TABLE credit_note_lines (
    id               INTEGER PRIMARY KEY,
    credit_note_id   INTEGER NOT NULL REFERENCES credit_notes(id) ON DELETE CASCADE,
    position         INTEGER NOT NULL,
    kind             TEXT NOT NULL CHECK (kind IN ('hours', 'days', 'fixed')),
    description      TEXT NOT NULL,
    quantity_centi   INTEGER NOT NULL CHECK (quantity_centi > 0),
    unit_price_pence INTEGER NOT NULL
);

-- Last sequence number used per document kind and scope (year, or 'all').
CREATE TABLE sequences (
    kind  TEXT NOT NULL,
    scope TEXT NOT NULL,
    last  INTEGER NOT NULL,
    PRIMARY KEY (kind, scope)
);

-- Issued invoices are locked: only sent_on may change, and they can't be deleted.
CREATE TRIGGER invoices_lock_update
BEFORE UPDATE OF number, client_id, issue_date, due_date, supply_start, supply_end, notes, issued_at, snapshot
ON invoices WHEN OLD.issued_at IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'invoice is issued and locked');
END;

CREATE TRIGGER invoices_lock_delete
BEFORE DELETE ON invoices WHEN OLD.issued_at IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'invoice is issued and locked');
END;

CREATE TRIGGER line_items_lock_insert
BEFORE INSERT ON line_items
WHEN (SELECT issued_at FROM invoices WHERE id = NEW.invoice_id) IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'invoice is issued and locked');
END;

CREATE TRIGGER line_items_lock_update
BEFORE UPDATE ON line_items
WHEN (SELECT issued_at FROM invoices WHERE id = OLD.invoice_id) IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'invoice is issued and locked');
END;

CREATE TRIGGER line_items_lock_delete
BEFORE DELETE ON line_items
WHEN (SELECT issued_at FROM invoices WHERE id = OLD.invoice_id) IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'invoice is issued and locked');
END;

-- Money can only move against issued invoices.
CREATE TRIGGER payments_require_issued
BEFORE INSERT ON payments
WHEN (SELECT issued_at FROM invoices WHERE id = NEW.invoice_id) IS NULL
BEGIN
    SELECT RAISE(ABORT, 'payments can only be recorded against issued invoices');
END;

CREATE TRIGGER refunds_require_issued
BEFORE INSERT ON refunds
WHEN (SELECT issued_at FROM invoices WHERE id = NEW.invoice_id) IS NULL
BEGIN
    SELECT RAISE(ABORT, 'refunds can only be recorded against issued invoices');
END;

CREATE TRIGGER credit_notes_require_issued
BEFORE INSERT ON credit_notes
WHEN (SELECT issued_at FROM invoices WHERE id = NEW.invoice_id) IS NULL
BEGIN
    SELECT RAISE(ABORT, 'credit notes can only be raised against issued invoices');
END;
