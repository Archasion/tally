# Changelog

## 0.4.0 — 2026-10-25

- **Reimbursed equipment and software.** Expenses can be marked *Rebill to*
  a client, with who approved them and when. In the invoice editor, `r` adds
  an approved, unbilled one as a line that prints the approval, payment date
  and supplier invoice number. Each purchase is billed once, and its amount
  and client are locked once it's on an invoice. Approval is checked again
  when you issue.
- **Billing period** on expenses: one-off, monthly, quarterly or annual.
  `u` on the Expenses tab adds the next payment of a subscription.
- **Receipts:** store the supplier's invoice number and optionally attach
  the file. It's copied into `<output folder>/receipts/`, `o` opens it, and
  issuing an invoice copies the receipts it bills next to the PDF
  (`…_receipt-1.pdf`).
- Expenses tab: *Rebill* column (client and invoice, *to bill* or *needs
  approval*), a count of purchases still to rebill, and details of the
  selected expense.
- The expense form lists every Self Assessment category beside the fields,
  with the selected one's full name and what belongs in it. Hints in
  pop-up forms wrap instead of being cut off.
- **Draft preview:** `o` on a draft, or `Ctrl+P` in the editor (including
  unsaved edits), opens a PDF marked DRAFT INVOICE with a watermark. It
  shows the number the draft would get if issued now, but saves nothing and
  uses up no number. Previews go in the system temp folder, not your output
  folder.
- The client's email is printed under their name and contact, when they
  have one.
- **International payments:** IBAN, BIC / SWIFT, bank name and bank address
  in Settings. They're printed in the payment box only for clients outside
  the UK. tally checks the IBAN's check digits, that a UK IBAN matches your
  sort code and account number, and that the BIC's country matches.
  Issuing or previewing for a client abroad without an IBAN set warns you.
- **Regenerate with current details:** `R` on an issued invoice asks
  whether to keep the details as issued or update them to your current
  business and client details. Updating lists what changes, asks again, and
  is recorded on the invoice ("Details updated to current details on …").
  The number, dates, lines and amounts never change.
- Database migrations 5 and 6.
- Template data: `lines[].detail`, and `draft` (true on previews).
- CSV export: `expenses.csv` adds billing, rebill client, approval, invoice,
  supplier invoice and receipt file columns.
- Database migration 4.

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
