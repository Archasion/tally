# tally

A full-screen terminal invoice manager for a UK sole trader who isn't
VAT-registered. It keeps clients, invoices, payments, credit notes and refunds
in a local SQLite database, and writes each invoice as an A4 PDF using an
embedded [Typst](https://typst.app) compiler. No external tools are needed.

```
 tally   1 Dashboard   2 Invoices   3 Clients   4 Expenses   5 Reports   6 Settings

 ┌ Outstanding ─────────┐ ┌ Overdue ─────────────┐ ┌ Income 2026-27 ──────┐ ┌ Drafts ──────────────┐
 │ £1,800.00            │ │ £1,800.00            │ │ £1,650.00            │ │ 0                    │
 │ 1 invoice            │ │ 1 invoice            │ │ received, cash basis │ │ drafts               │
 └──────────────────────┘ └──────────────────────┘ └──────────────────────┘ └──────────────────────┘

 Needs attention & recent
 Number      Client      Issued       Due                       Total     Balance  Status
 2026-0002   Acme Ltd    01 Jul 2026  31 Jul 2026 (64d late)  £1,800.00  £1,800.00  Overdue
 2026-0001   Acme Ltd    30 Sep 2026  30 Oct 2026             £1,800.00      £0.00  Paid
```

## Features

- **Invoices** with hourly, daily and fixed-price lines, a supply date or
  period (required on UK invoices), and a due date that follows your payment
  terms.
- **Lock on issue.** Issuing assigns the next number, freezes your details and
  the client's, writes the PDF and locks the invoice. Corrections are made with
  **credit notes**. SQLite triggers enforce the lock as well as the app.
- **Configurable numbering**, e.g. `{YEAR}-{SEQ:4}` → `2026-0001`, restarting
  every 1 January. Add `{LABEL}` for per-client numbering:
  `{LABEL}-{YEAR}-{SEQ:4}` → `LGT-2026-0001`, `ACM-2026-0001`, each client
  counting separately.
- **International clients:** each client has a country, which is always
  printed, and an optional tax ID with its local name, e.g. *VAT ID:
  DE123456789* or *ABN: 51 824 753 556*. Amounts are always in GBP.
  Invoices to clients outside the UK also show your IBAN, BIC and bank, if
  set in Settings.
- **Status tracking:** Draft → Issued → Sent → Part paid → Paid, with
  Overdue and Credited worked out automatically.
- **Partial payments, overpayments and refunds.**
- **Optional late-payment notice** for business clients (statutory interest
  under the Late Payment of Commercial Debts Act).
- **Expenses** grouped by the Self Assessment categories, with totals per tax
  year ready for your return.
- **Reimbursed equipment and software:**
  - mark an expense as rebilled to a client, and record who approved it and
    when
  - set a billing period (one-off, monthly, quarterly or annual); `u` adds
    the next payment of a subscription
  - store the supplier's invoice number, and optionally attach the receipt
    file, which is copied into your output folder
  - add it to an invoice with `r` in the editor. The line prints the
    approval and supplier invoice number, and issuing places the receipts
    next to the invoice PDF, ready to attach to the same email. Each
    purchase can be billed only once.
- **Trading names:** invoice under a name such as *Northgate Design* and set
  your legal name in Settings. tally prints "Trading name of Jana Šimková"
  under it, as UK law requires. Keep the bank account name exactly as your
  bank has it, so payers' name checks match.
- **Invoice notes**, printed under the payment details. New invoices start
  with the default notes from Settings.
- **Tax estimate:**
  - income tax, Class 4 and Class 2 National Insurance, and student loan
    repayments, each marked required, optional or not payable
  - every rate and threshold used, with the GOV.UK sources and the date they
    were checked
  - year-to-date and full-year projections
  - payments on account and due dates
  - Making Tax Digital warnings

  Rates are built in for 2024-25 to 2026-27. You can add or correct a year in
  a `rates.toml` file without updating tally. See [docs/TAX.md](docs/TAX.md).
- **Reports:**
  - outstanding and overdue invoices, grouped by days late
  - tax-year income (6 April to 5 April) on a cash basis
  - CSV export
- **Safe storage.** The live database stays on your Mac. PDFs, CSV exports
  and automatic backups go to a folder you choose, such as iCloud Drive.
- **Customisable PDF layout** through a `template.typ` override, with no
  rebuild needed.

## Install

You need Rust 1.88 or newer, and Xcode command-line tools so the bundled
SQLite can compile.

```bash
cargo install --path .
```

This puts `tally` in `~/.cargo/bin`. To run it from the source tree instead:

```bash
cargo run --release
```

## First run

Running `tally` the first time opens a setup wizard that asks for:

- your name, address and contact details
- your bank details, which are printed on every invoice
- your payment terms, e.g. 30 days
- your invoice and credit-note number patterns
- an **output folder** for PDFs, backups and exports

Everything can be changed later under **6 Settings**, which also has the tax
settings (student loan plan, voluntary Class 2). Changes only affect
invoices you issue after that, because issued invoices keep the details they
were issued with.

## Everyday use

| Task | Keys |
|---|---|
| Add a client | `3`, then `n` (give them a label if your number pattern uses `{LABEL}`) |
| Start an invoice | `n` on the Dashboard or Invoices tab, or `i` on a client |
| Add or edit lines | in the editor, move down past *Notes* into Lines; `a` adds, `Enter` edits, `d` deletes, `K`/`J` reorder |
| Bill a reimbursed purchase | in the editor's Lines, `r` picks an approved, unbilled expense for that client |
| Save a draft | `Ctrl+S` |
| Preview a draft as a PDF | `Ctrl+P` in the editor (includes unsaved edits), or `o` on a draft. Nothing is saved or numbered |
| Issue (number + PDF + lock) | `i` on the draft |
| Preview a draft's email | `m` on a draft shows the email it would get once issued, inside tally (`o` there opens the PDF preview) |
| Email it to the client | `m` opens a new message in Apple Mail with the PDF and any receipts attached; you check it and press Send. tally then asks whether to mark it sent today |
| Mark as sent | `s` |
| Record a payment | `p` (pre-filled with the balance) |
| Credit note | `c` (`Enter` on one in the invoice's history opens or emails it) |
| Refund an overpayment or credit | `r` (only offered when money is owed back) |
| Open or regenerate the PDF | `o` / `R` (`R` asks whether to keep the details as issued or update them to your current ones, e.g. to add an IBAN) |
| Duplicate an invoice as a new draft | `u` |
| Record an expense | `4`, then `n` (`←`/`→` changes tax year). Set *Rebill to* for a purchase a client reimburses |
| Next payment of a subscription | `u` on the expense |
| Open an expense's receipt | `o` on the expense |
| Reports | `5`, then `Tab` to switch reports and `←`/`→` to change tax year |
| Tax estimate | `5`, `Tab` to *Tax estimate*; `p` switches year to date / projected, `b` enters last year's bill |
| Back up now | `Ctrl+B` in Settings, or `tally backup` |
| Quit (backs up automatically) | `q` or `Ctrl+C` |

Lists use `↑`/`↓` or `j`/`k`. Forms use `↑`/`↓`/`Tab` to move between
fields, `Enter` to go to the next field (or save on the last one), `Ctrl+S`
to save and `Esc` to cancel. Dates can be typed as `2026-10-02`,
`02/10/2026`, `2/10/26` or `today`.

## Command line

```
tally                     Open the full-screen app
tally backup              Back up the database to <output folder>/backups
tally sample-pdf [FILE]   Render a sample invoice (good for testing a template)
tally tax [YEAR] [--projected]
                          Print the tax estimate, e.g. `tally tax 2025-26`
tally rates               Print the built-in tax rates (rates.toml format)
tally where               Show where the database and output folder are
tally help                Show help
```

Set `TALLY_DB=/path/to/test.db` to use a different database, e.g. for trying
things out.

## Where things live

| What | Where |
|---|---|
| Database (live) | `~/Library/Application Support/tally/tally.db` |
| Invoice and credit-note PDFs | `<output folder>/2026/2026-0001_Acme-Ltd.pdf` |
| Receipts sent with an invoice | `<output folder>/2026/2026-0001_Acme-Ltd_receipt-1.pdf` |
| Receipts attached to expenses | `<output folder>/receipts/2026/2026-10-03_<supplier invoice no.>.pdf` (or the description if there's no number) |
| Backups (newest *n* kept) | `<output folder>/backups/tally-YYYYMMDD-HHMMSS.db` |
| CSV exports | `<output folder>/exports/YYYYMMDD-HHMMSS/` |
| Template override (optional) | `<output folder>/template.typ` |
| Extra fonts (optional) | `<output folder>/fonts/*.ttf` / `*.otf` |
| Tax rates override (optional) | `<output folder>/rates.toml` |

The live database is deliberately kept out of the synced folder. Sync
services can corrupt an SQLite file by copying it while it's being written.
Backups use SQLite's online backup API and are written under a temporary
name first, so a half-written backup never gets synced.

To restore a backup:

1. Quit tally.
2. Delete `tally.db-wal` and `tally.db-shm` next to the database, if they exist.
3. Copy the backup file over `tally.db`.

## Customising the invoice

Copy [`templates/invoice.typ`](templates/invoice.typ) to
`<output folder>/template.typ` and edit it. Preview it with
`tally sample-pdf`, and press `Ctrl+T` in Settings to reload it in a running
app. See [docs/TEMPLATES.md](docs/TEMPLATES.md) for the data available to the
template.

## UK specifics

What the app does on your behalf, and why:

- **Required invoice fields.** Every invoice includes the details GOV.UK says
  must appear: a unique number, your name and address, the client's name and
  address, a description, the supply date, the invoice date, the amounts and
  the total.
- **No VAT.** Invoices show no VAT lines and no VAT number, since a business
  that isn't registered can't charge VAT.
- **Cash-basis income.** The income report counts money in the tax year it was
  received, and refunds in the tax year they were paid out. Cash basis has been
  the default for sole traders since April 2024.
- **Late-payment notice.** The notice can only be switched on for business
  clients, because statutory interest doesn't apply to individuals.

- **Tax estimate.** It uses England, Wales and NI income tax rates and
  assumes self-employment is your only income. [docs/TAX.md](docs/TAX.md)
  lists exactly what it covers and what it doesn't.

tally is a record-keeping tool, not tax advice.

## Development

```bash
cargo test                  # unit tests + an end-to-end lifecycle test
cargo clippy --all-targets
cargo fmt
```

[docs/DESIGN.md](docs/DESIGN.md) covers the architecture, the business rules
and the reasons behind the main decisions.
