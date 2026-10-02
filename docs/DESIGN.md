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
    numbering.rs     {YEAR}-{SEQ:4} patterns
    dates.rs         UK date parsing/formatting, TaxYear
    model.rs         Business, Client, Invoice, LineItem, CreditNote,
                     Transaction, Totals → Status
  db/                SQLite (rusqlite, bundled)
    schema.sql       Tables + triggers that enforce locking
    mod.rs           Open/migrate, business settings, number allocation
    clients.rs       Client CRUD and archiving
    invoices.rs      Drafts, issuing, sent date, duplicating
    ledger.rs        Payments, refunds, credit notes, totals
  pdf/
    mod.rs           DocumentData (what templates see), Renderer
    world.rs         Sandboxed Typst World: one source, one data file, fonts
  reports/           Invoice summaries, aging, tax-year income, CSV export
  backup.rs          Online backup + pruning
  paths.rs           Database location, output folder layout, safe filenames
  ui/                ratatui + crossterm full-screen app
    mod.rs           App state, event loop, navigation, chrome
    form.rs          Reusable form widget (text / choice / toggle fields)
    business_form.rs Setup wizard and Settings
    dashboard.rs, invoices.rs, editor.rs, clients.rs, report_view.rs
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

- Tokens: `{YEAR}`, `{YY}`, `{MONTH}`, `{SEQ}` / `{SEQ:n}`. A pattern must
  contain `{SEQ}` exactly once. Only letters, digits, `-`, `_` and `.` are
  allowed, so a number is always safe to use as a filename.
- If the pattern contains `{YEAR}` or `{YY}`, the sequence restarts every
  calendar year. The year used is the year of the **issue date**. Without a
  year token it never restarts. `{MONTH}` is only a label and never restarts
  the sequence.
- Counters are stored per (document kind, scope) in `sequences`. If a
  generated number already exists, for example after you change the pattern,
  the counter skips ahead until it finds a free one. Numbers are therefore
  always unique, and gap-free unless you change the pattern.
- Numbers are given out in the order invoices are issued, not by date. A
  back-dated invoice issued today gets the next number.

### Tax year and income

`TaxYear(2026)` runs from 6 April 2026 to 5 April 2027. Income uses the cash
basis:

- payments count in the tax year they were received
- refunds count in the tax year they were paid out
- issuing an invoice and raising a credit note don't affect income

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
| **Structured address fields, not a multi-line text area** | `tui-textarea` doesn't support ratatui 0.30. Separate fields also give cleaner PDFs. |
| **Credit notes from the TUI have one line** | The data model and PDF support several lines. The UI keeps the common case (one amount plus a reason) quick. |
| **Notes are single-line** | Same reason: `tui-input` is single-line. Long notes wrap on the PDF. |
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
  across a tax-year boundary, CSV export and backup.
