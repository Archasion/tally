# Changelog

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
