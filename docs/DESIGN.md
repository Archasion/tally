# Design notes

This document covers how tally is built, the rules it enforces, and the
reasons behind decisions that aren't obvious from the code.

## Layout

```
src/
  main.rs            CLI entry point: TUI, backup, sample-pdf, where
  lib.rs
  domain/            Pure logic, no I/O, unit-tested
    money.rs         Money (i64 pence), Quantity (i64 hundredths), rounding
    numbering.rs     {LABEL}-{YEAR}-{SEQ:4} patterns, client labels
    dates.rs         UK date parsing/formatting, TaxYear
    model.rs         Business, Client, Invoice, LineItem, CreditNote,
                     Transaction, Totals → Status
    expense.rs       Expense, SA103F categories, billing periods
  db/                SQLite (rusqlite, bundled)
    schema.sql       Tables + triggers that enforce locking
    mod.rs           Open/migrate, business settings, number allocation
    clients.rs       Client CRUD and archiving
    invoices.rs      Drafts, issuing, sent date, duplicating
    ledger.rs        Payments, refunds, credit notes, totals
  pdf/
    mod.rs           DocumentData (what templates see), Renderer
    world.rs         Sandboxed Typst World: one source, one data file, fonts
  reports/           Invoice summaries, aging, tax-year income, tax report
                     assembly (records + rates), CSV export
  tax/
    mod.rs           Pure tax maths: profit, income tax, Class 2/4, student
                     loans, projection, payments on account, MTD, breakdown
    rates.rs         Typed per-year rates from rates/uk.toml + rates.toml
rates/uk.toml        Built-in rates with GOV.UK sources and verified dates
  backup.rs          Online backup + pruning
  receipts.rs        Copying receipt files in, and next to invoice PDFs
  paths.rs           Database location, output folder layout, safe filenames
  ui/                ratatui + crossterm full-screen app
    mod.rs           App state, event loop, navigation, chrome
    form.rs          Reusable form widget (text / choice / toggle fields,
                     optional overview panel listing a choice's options)
    business_form.rs Setup wizard and Settings
    dashboard.rs, invoices.rs, editor.rs, clients.rs, expenses.rs,
    report_view.rs   (includes the Tax estimate tab)
    modals.rs        Pop-up forms (payment, refund, credit note, client, line)
                     and confirmations
templates/invoice.typ  Built-in Typst template (embedded at compile time)
tests/workflow.rs      End-to-end lifecycle test
```

## Business rules

### Money

All amounts are integer pence. A line's amount is `quantity × unit price`,
where quantity is stored in hundredths (so 7.5 hours is `750`). It's rounded
half away from zero to the penny, **once per line**, and the invoice total is
the sum of the rounded lines.

Totals are calculated two ways: `LineItem::amount` in Rust, and in SQL in
`Db::totals`. A test checks that both give the same answer.

### Invoice life cycle

```
Draft ──issue──▶ Issued ──mark sent──▶ Sent
                   │                     │
                   └──── payments / credit notes / refunds ────▶ Part paid / Paid / Credited
                   (Overdue whenever money is owed and the due date has passed)
```

- **Draft:** has no number and can be freely edited or deleted.
- **Preview** (`Db::preview_invoice`) renders a draft, including unsaved
  edits, as it would be issued now. It attaches the current business and
  client details and reimbursement details, and shows the number it would
  get. That number comes from a read-only peek at the sequence, which writes
  nothing. Issuing later can still get a different number if another invoice
  is issued first. The PDF is titled DRAFT INVOICE, watermarked, and written
  to `$TMPDIR/tally-preview/`, never the output folder.
- **Issuing** happens in one transaction:
  1. Allocate the next number.
  2. Store a JSON *snapshot* of the business and client details.
  3. Set `issued_at`.
  4. Render and write the PDF.
  5. Commit.

  If the PDF fails, everything rolls back and the number is not used up.
- **Locked:** once issued, an invoice's content can't change and it can't be
  deleted. This is enforced in Rust *and* by SQLite triggers, so even a manual
  `UPDATE` is rejected. Only `sent_on` can change.
- **Status is calculated, never stored**, by `Totals::status`, from:
  - total, payments, credit notes and refunds
  - whether it's been issued and sent
  - today's date

  The rules, checked in this order:
  1. *Credited* if credit notes cover the whole total.
  2. Otherwise *Paid* if nothing is owed.
  3. Otherwise *Overdue* if past the due date. This counts from the due date
     whether or not the invoice was marked as sent.
  4. Otherwise *Part paid* if anything has been received.
  5. Otherwise *Sent* or *Issued*.
- **Balance** = total − payments − credit notes + refunds. If it's negative,
  money is owed back to the client.

### Credit notes and refunds

- A credit note can only be raised against an issued invoice. Its total can't
  be more than the part of the invoice not already credited.
- Credit notes have their own number pattern and sequence, their own snapshot
  and their own PDF. They can't be deleted.
- Overpayments are allowed. If the balance goes negative, the app offers a
  **refund**, which can be at most the amount owed back.
- Payments and refunds *can* be deleted, with a confirmation, to fix mistaken
  entries. They don't appear on any PDF.

### Numbering

- Tokens: `{YEAR}`, `{YY}`, `{MONTH}`, `{LABEL}`, `{SEQ}` / `{SEQ:n}`. A pattern must
  contain `{SEQ}` exactly once. Only letters, digits, `-`, `_` and `.` are
  allowed, so a number is always safe to use as a filename.
- If the pattern contains `{YEAR}` or `{YY}`, the sequence restarts every
  calendar year. The year used is the year of the **issue date**. Without a
  year token it never restarts. `{MONTH}` is only a label and never restarts
  the sequence.
- `{LABEL}` is the client's label (e.g. `LGT`). If a pattern contains it,
  **each label has its own sequence**, so `LGT-2026-0001` and `ACM-2026-0001`
  can both exist. The scope is `LABEL:YEAR`, or `LABEL:all` without a year
  token. Both the invoice and credit-note patterns can use it.
- Client labels:
  - stored uppercase, A–Z and 0–9 only, up to 10 characters
  - unique across clients (a partial unique index allows any number of
    clients with no label)
  - **required before issuing** when the pattern uses `{LABEL}`; otherwise
    issuing fails before a number is used up
  - **locked** once any issued invoice or credit note has the label in its
    snapshot, so that client's numbering stays continuous. A trigger
    enforces this. A client whose earlier invoices had no label can still be
    given one.
- Counters are stored per (document kind, scope) in `sequences`. If a
  generated number already exists, for example after you change the pattern,
  the counter skips ahead until it finds a free one. Numbers are therefore
  always unique, and gap-free unless you change the pattern.
- Numbers are given out in the order invoices are issued, not by date. A
  back-dated invoice issued today gets the next number.

### International clients

- Every client address has a **country**. It's always printed as the last
  address line, including "United Kingdom". Existing clients were set to
  United Kingdom by migration 2.
- The postcode field takes any format (e.g. ZIP codes) and isn't
  upper-cased. Put states or regions in the town field.
- An optional **tax ID** is stored as a name plus a number (e.g. *VAT ID* +
  *DE123456789*), printed under the client's address. You must fill in both
  or neither.
- All amounts are in **GBP**; there's no currency conversion.
- New client fields use `#[serde(default)]`, so snapshots on invoices issued
  before these fields existed still load and render unchanged.

### Tax year and income

`TaxYear(2026)` runs from 6 April 2026 to 5 April 2027. Income uses the cash
basis:

- payments count in the tax year they were received
- refunds count in the tax year they were paid out
- issuing an invoice and raising a credit note don't affect income

### Expenses and tax

- Expenses are recorded on the cash basis (by the date paid), using the
  categories from the full Self Assessment self-employment pages (SA103F).
  Depreciation and bad debts are left out: depreciation isn't allowable, and
  bad debts don't arise on the cash basis.
- The tax estimate (`tax::estimate`) is pure and works in pence. Rates are
  stored as basis points, so no floating point reaches the maths. Pounds and
  percentages in TOML are converted once, on load.
- Rates are data, not code. `rates/uk.toml` is embedded at compile time, and
  `<output>/rates.toml` replaces whole years. Unknown fields are rejected, and
  bands must be in ascending order. A year without rates is reported, never
  guessed.
- Every breakdown line carries the rate and threshold it used, and every
  section carries an obligation (required, optional or not payable), so the
  TUI and `tally tax` show the same explanation.
- Payments on account use, in order: the bill you entered for the previous
  year (`tax_year_bills`), tally's estimate for that year if it has rates,
  or "unknown".
- Migration 3 adds `expenses`, the tax settings columns on `business`, and
  `tax_year_bills`.

See [TAX.md](TAX.md) for the user-facing rules and values.

### Reimbursements

Equipment or software a client pays you back for is a **recharge**: you
bought it in your own name, so it's your expense, and the client's payment is
income. Profit isn't affected, but turnover includes the reimbursement, which
counts towards the MTD and VAT thresholds. Disbursements (bought as the
client's agent, outside your accounts) aren't modelled.

- An expense with a **rebill client** can be added to that client's invoice
  as a fixed-price line linked by `line_items.expense_id`. The line's
  `detail` (billing period, payment date, approval and supplier invoice
  number) is generated when it's added, refreshed from the expense when the
  invoice is issued, and then **frozen** with the line, so editing the
  expense later never changes an issued invoice.
- Saving a draft, and issuing it, checks that each linked line:
  - belongs to the invoice's client
  - is approved (**who** and **when** are both required)
  - bills exactly one unit at the expense's amount
  - isn't on another invoice or twice on this one (also enforced by a
    partial unique index)
- Once an expense is on any invoice, its amount and rebill client can't
  change and it can't be deleted. Triggers enforce this for issued invoices,
  and the foreign key from `line_items` stops deleting one on a draft. To
  change one on a draft, remove the line first.
- **Limitation:** a reimbursement stays billed even if its invoice is
  credited, so it can't be put on a corrected invoice. Re-billing it needs a
  rule for credited reimbursements that isn't designed yet.
- Duplicating an invoice leaves out its reimbursement lines.
- **Billing period** (one-off, monthly, quarterly, annual) is descriptive. On
  the cash basis each payment is its own expense, so `u` copies one to the
  next period's date, keeping the approval and clearing the receipt. Dates
  are clamped to the month end, so copying from 31 January gives 28
  February, and then 28 March.
- **Receipts:** a file path given on the expense form is copied to
  `<output>/receipts/<year>/<paid date>_<supplier invoice no.>.<ext>` (or
  the description, cut to 40 characters, if there's no number). The
  original file name is never used, and an existing receipt is never
  overwritten. The path is stored relative to the output folder and must
  stay inside it, and the original file isn't needed again.
  After an invoice is issued, or `R` regenerates it, earlier copies are
  removed and the receipts its reimbursements have *now* are copied next to
  the PDF as `<pdf name>_receipt-N.<ext>`. This happens after the commit: a
  missing receipt is reported, but it doesn't stop the invoice being issued.
- Migration 4 adds the expense columns, `expense_id` and `detail` on both
  line tables (credit notes don't set `expense_id` yet), the unique index
  and the triggers.

### Aging

Outstanding invoices are grouped by days past the due date: not yet due,
1–30, 31–60 and 60+.

## Decisions and deviations from the original plan

| Decision | Why |
|---|---|
| **Typst instead of genpdf** | genpdf hasn't been released since 2021 and depends on `printpdf` 0.3. Typst is pure Rust too, gives better typography, and lets you change the layout without recompiling. |
| **Sandboxed Typst `World`** | Templates can read only `/data.json` and the given fonts. They can't download packages or read your files, so output is repeatable. |
| **Data is pre-formatted in Rust** | Templates get strings like `£1,800.00`, so they never repeat money or date formatting logic. |
| **A small `PRAGMA user_version` migration runner, not `rusqlite_migration`** | `rusqlite_migration` 2.6 needs Rust 1.95, newer than the toolchain this was built on. The runner applies numbered SQL files in a transaction each. |
| **No `config.toml`** | All settings live in the `business` row, so there's one source of truth that's backed up with your data. Only the database path is fixed (overridable with `TALLY_DB`). |
| **The database is local, not in the synced folder** | Sync services can corrupt a SQLite file mid-write. The synced folder gets PDFs and atomic backups instead. |
| **Labels are per client with their own sequences, not a business-wide prefix** | Requested: invoices to each client are numbered separately (LGT-…-0001, ACM-…-0001). |
| **Tax rates in a TOML file, overridable** | Rates change every April. Users can add a year (or fix a figure) without a new release, and every year records its sources and verification date. |
| **England, Wales & NI income tax only; self-employment the only income** | As requested. Scottish bands and other income would change the band calculation and aren't modelled. |
| **Voluntary Class 2 is opt-in** | It's optional, so it's shown but only added to the bill when Settings says you'll pay it. |
| **Labels lock once used** | Changing a label after issuing would split one client's numbering into two series. |
| **Structured address fields, not a multi-line text area** | `tui-textarea` doesn't support ratatui 0.30. Separate fields also give cleaner PDFs. |
| **Credit notes from the TUI have one line** | The data model and PDF support several lines. The UI keeps the common case (one amount plus a reason) quick. |
| **Notes are single-line** | Same reason: `tui-input` is single-line. Long notes wrap on the PDF. |
| **Reimbursements are recharges, linked to expenses** | One record of each purchase feeds both the tax estimate and the invoice, and a purchase can't be billed twice or silently changed after billing. |
| **Receipts are copied, not merged into the PDF** | The Typst sandbox only reads `/data.json`, which keeps templates repeatable. A separate file next to the invoice is just as easy to attach and stays exactly as the supplier issued it. |
| **Reimbursement approval is required before billing** | The client agreed to pay for it; recording who and when settles a dispute before it starts. |
| **Backups on quit** | Automatic, with no background process. `tally backup` is there for scripts. |

## Testing

- `domain/*`: money parsing, formatting and rounding; number patterns; tax-year
  boundaries; status rules.
- `db/*`: run against in-memory SQLite. Covers locking (including direct SQL),
  numbering across years and pattern changes, roll-back when the PDF step
  fails, partial payments, credit notes after payment, and refunds.
- `pdf`: real Typst compiles of an invoice and a credit note; template errors
  reported with line numbers.
- `ui/form`, `ui/business_form`: form navigation and validation.
- `tests/workflow.rs`: the whole life cycle with real PDFs, cash-basis income
  across a tax-year boundary, CSV export and backup; a reimbursed
  subscription with an attached receipt, from approval to issue.
