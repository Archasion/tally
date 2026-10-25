# Tax estimate

**5 Reports → Tax estimate** (or `tally tax`) estimates your Self Assessment
bill for one UK tax year. Each line shows the rate and threshold it used. Each
kind of payment is marked **required**, **optional** or **not payable**.

> This is an estimate for a sole trader **whose only income is
> self-employment**, using **England, Wales and Northern Ireland** income tax
> rates. It isn't tax advice. Check HMRC's calculation before you pay.

## What goes in

| Input | Where it comes from |
|---|---|
| Income | Payments received minus refunds paid in the tax year, by the date the money moved (cash basis). Invoices issued but not yet paid don't count. |
| Expenses | The **Expenses** tab (`4`), by the date paid. |
| Student loan plan, voluntary Class 2 | **Settings** (`6`). |
| Last year's bill (for payments on account) | Worked out by tally from last year's records, or entered from your HMRC statement (`b` in the tax report). |
| Rates | Built in (2024-25 to 2026-27), or your own `rates.toml` (see [below](#adding-or-correcting-rates)). |

## The calculation

1. **Profit** = income − deduction. The deduction is your recorded expenses
   or the **£1,000 trading allowance**, whichever is higher; you can't use
   both. The allowance can't create a loss. Profit is rounded down to whole
   pounds. If expenses are more than income, the loss is shown but loss
   relief isn't modelled.
2. **Personal allowance**: £12,570. It falls by £1 for every £2 of income
   over £100,000, so it's gone at £125,140.
3. **Income tax** *(required)* on taxable income (profit − personal
   allowance):

   | Band | Taxable income | Rate |
   |---|---|---|
   | Basic rate | up to £37,700 | 20% |
   | Higher rate | £37,700–£125,140 | 40% |
   | Additional rate | over £125,140 | 45% |

4. **Class 4 National Insurance** *(required if profit is over £12,570)*:
   6% on profit between £12,570 and £50,270, and 2% above £50,270.
5. **Class 2 National Insurance:**
   - **Not payable** if profit is at least the Small Profits Threshold. It's
     treated as paid, so your National Insurance record is protected for free.
   - **Optional** below the threshold. Paying voluntarily protects your State
     Pension record. It's added to the total only if Settings → *Pay voluntary
     Class 2* is on.
6. **Student loan** *(required if you have one; switch on in Settings)*: 9% of
   profit over your plan's threshold, plus 6% over the Postgraduate Loan
   threshold if you have one. Each is rounded down to whole pounds, as Self
   Assessment does.
7. **Estimated bill** = income tax + Class 4 + any Class 2 you've opted into +
   student loan.

All amounts are worked out in pence. Each tax amount is rounded down to the
penny.

### Values used

These were checked against GOV.UK on **3 October 2026**. The tax report
shows which set it's using.

| | 2024-25 | 2025-26 | 2026-27 |
|---|---|---|---|
| Personal allowance | £12,570 | £12,570 | £12,570 |
| Allowance taper starts | £100,000 | £100,000 | £100,000 |
| Basic / higher / additional rate | 20 / 40 / 45% | 20 / 40 / 45% | 20 / 40 / 45% |
| Basic-rate band (taxable income) | £37,700 | £37,700 | £37,700 |
| Additional rate from (taxable income) | £125,140 | £125,140 | £125,140 |
| Class 4 Lower / Upper Profits Limit | £12,570 / £50,270 | £12,570 / £50,270 | £12,570 / £50,270 |
| Class 4 main / upper rate | 6% / 2% | 6% / 2% | 6% / 2% |
| Class 2 Small Profits Threshold | £6,725 | £6,845 | £7,105 |
| Class 2 voluntary rate | £3.45/wk (£179.40/yr) | £3.50/wk (£182.00/yr) | £3.65/wk (£189.80/yr) |
| Student loan Plan 1 / 2 / 4 | £24,990 / £27,295 / £31,395 | £26,065 / £28,470 / £32,745 | £26,900 / £29,385 / £33,795 |
| Student loan Plan 5 | — | — | £25,000 |
| Postgraduate Loan | £21,000 | £21,000 | £21,000 |
| Student loan rate / postgraduate rate | 9% / 6% | 9% / 6% | 9% / 6% |
| Trading allowance | £1,000 | £1,000 | £1,000 |
| Payments on account threshold | £1,000 | £1,000 | £1,000 |
| Making Tax Digital: turnover over… | £50,000 → from 6 Apr 2026 | £30,000 → from 6 Apr 2027 | £20,000 → from 6 Apr 2028 |

Sources are listed for each year in [`rates/uk.toml`](../rates/uk.toml) and
at the bottom of the tax report.

## Year to date vs projected

For the current tax year the report shows both:

- **Year to date:** what you've actually received and spent so far.
- **Projected:** those figures scaled to the full year by days elapsed. For
  example, with 181 of 365 days gone, figures are multiplied by 365/181.

Press `p` to switch between them. Projections early in the year are rough.
Past years show their actual totals.

## Payments due

Self Assessment collects tax for a year (6 April *N* to 5 April *N+1*) like
this:

| Due | What |
|---|---|
| 31 January *N+1* | 1st payment on account for the year: half of last year's income tax + Class 4 |
| 31 July *N+1* | 2nd payment on account for the year |
| 31 January *N+2* | **Balancing payment**: the year's bill minus payments on account already made (negative means overpaid) |
| 31 January *N+2* | 1st payment on account for the next year: half of this year's income tax + Class 4 |
| 31 July *N+2* | 2nd payment on account for the next year |

Payments on account are **required** unless last year's income tax + Class 4
was under £1,000. Class 2 and student loans aren't included in them. If you
expect to earn less, you can ask HMRC to reduce them.

For "last year's bill", tally uses your entered HMRC figure if you've given
one (`b`). Otherwise it uses its own estimate for that year from your
records. In your first year of trading the estimate is £0, so there are no
payments on account for that year.

## Making Tax Digital

If a year's turnover (income before expenses) goes over the Making Tax
Digital threshold for that year, the report warns that you'll need
MTD-compatible software for quarterly updates from the start date. tally
can't submit to HMRC.

## Adding or correcting rates

Rates change every April and tally won't guess. A year without rates shows
"No tax rates for …".

To add one:

```bash
tally rates > "$(tally where | sed -n 's/^Output folder: *//p')/rates.toml"
```

Then edit `rates.toml` in your output folder:

- Rename `[years.2026…]` to the new year, e.g. `[years.2027…]`. The number is
  the year the tax year starts.
- Update the figures from GOV.UK, along with `verified` and `sources`.
- Delete any years you didn't change, so built-in corrections still apply to
  them.

Press `R` in the tax report, or `Ctrl+T` in Settings, to reload. Any year in
`rates.toml` replaces the built-in year completely, and the report says
"your rates.toml" in its *Rates* line. Mistakes such as a typo in a field
name or a band in the wrong order are reported with the year and field.

## Not covered

These would make the estimate wrong. Check with HMRC or an accountant if any
apply to you:

- Scottish income tax rates (a different band structure)
- other income: employment, pensions, savings, dividends, property
- pension contributions, Gift Aid, Marriage Allowance, Blind Person's
  Allowance
- loss relief, and capital allowances outside the cash basis
- payments on account reduced by you or HMRC, and tax deducted at source
- capital gains, the High Income Child Benefit Charge, and interest or
  penalties
- Making Tax Digital submissions
