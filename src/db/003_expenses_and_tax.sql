-- Version 3: expenses and tax-estimate settings.

CREATE TABLE expenses (
    id           INTEGER PRIMARY KEY,
    date         TEXT NOT NULL,          -- date paid (cash basis)
    description  TEXT NOT NULL,
    category     TEXT NOT NULL,
    amount_pence INTEGER NOT NULL CHECK (amount_pence > 0),
    notes        TEXT NOT NULL DEFAULT ''
);
CREATE INDEX expenses_date ON expenses(date);

ALTER TABLE business ADD COLUMN student_loan      INTEGER NOT NULL DEFAULT 0;
ALTER TABLE business ADD COLUMN student_loan_plan TEXT NOT NULL DEFAULT '';  -- '' = none
ALTER TABLE business ADD COLUMN postgraduate_loan INTEGER NOT NULL DEFAULT 0;
ALTER TABLE business ADD COLUMN voluntary_class2  INTEGER NOT NULL DEFAULT 0;

-- A tax year's income tax + Class 4 as shown on your HMRC statement, used
-- for the next year's payments on account instead of tally's estimate.
CREATE TABLE tax_year_bills (
    year         INTEGER PRIMARY KEY,    -- tax year starting 6 April of this year
    amount_pence INTEGER NOT NULL CHECK (amount_pence >= 0)
);
