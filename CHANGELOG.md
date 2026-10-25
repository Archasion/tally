# Changelog

## 0.3.0 — 2026-10-03

- **Expenses** tab: cash-basis expenses in Self Assessment (SA103F)
  categories, with per-category totals for each tax year.
- **Tax estimate** (Reports → Tax estimate, and `tally tax [YEAR]`):
  - income tax (England, Wales & NI), Class 4, and Class 2 (not payable or
    voluntary), plus optional student loan Plans 1/2/4/5 and Postgraduate
  - each part marked required, optional or not payable, showing every rate
    and threshold used
  - year to date and full-year projection
  - payments on account schedule, with an optional entered prior-year bill
  - Making Tax Digital threshold warnings
- Rates for 2024-25, 2025-26 and 2026-27 built in (`rates/uk.toml`, checked
  against GOV.UK on 2026-10-03). Add or correct years in
  `<output folder>/rates.toml`; `tally rates` prints the format.
- Settings: student loan on/off, plan, Postgraduate Loan, pay voluntary
  Class 2.
- CSV export adds `expenses.csv`.
- Tabs are now 1 Dashboard, 2 Invoices, 3 Clients, 4 Expenses, 5 Reports,
  6 Settings.
- Database migration 3.

## 0.2.0 — 2026-10-03

- `{LABEL}` number token for per-client numbering, e.g. `LGT-2026-0001`.
  Each label has its own sequence. Labels are uppercase A–Z/0–9, unique,
  required before issuing when the pattern uses them, and locked once used on
  an issued invoice or credit note.
- International clients: country (always printed), tax ID name and number
  (printed under the address), and postcode/ZIP in any format.
- Template data: `client.tax_id`; `client.address` now ends with the country.
- The output folder must now be a full path (`~/…` or `/…`).
- Database migration 2 (existing clients are set to United Kingdom).

## 0.1.0 — 2026-10-03

First version.

- Full-screen TUI: setup wizard, dashboard, invoices, invoice editor, clients,
  reports and settings.
- Hourly, daily and fixed-price lines; supply date or period; due dates from
  payment terms.
- Issuing assigns a configurable number, freezes details, writes an A4 PDF via
  embedded Typst and locks the invoice (also enforced by SQLite triggers).
- Statuses: Draft, Issued, Sent, Part paid, Overdue, Paid, Credited.
- Partial payments, overpayments, credit notes (with PDFs) and refunds.
- Optional statutory late-payment notice for business clients.
- Reports: aging, UK tax-year cash-basis income, CSV export.
- Local database with automatic backups to the output folder on quit;
  `tally backup`, `tally sample-pdf`, `tally where`.
- Template override (`template.typ`) and custom fonts in the output folder.
