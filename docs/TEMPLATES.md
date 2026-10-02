# Invoice templates

tally lays out invoices and credit notes with [Typst](https://typst.app/docs/).
The built-in template is [`templates/invoice.typ`](../templates/invoice.typ).

## Overriding the template

1. Copy `templates/invoice.typ` to `<output folder>/template.typ`.
   `tally where` shows where your output folder is.
2. Edit it.
3. Preview it with `tally sample-pdf preview.pdf`. That command uses your
   override when one exists.
4. If tally is already running, press `Ctrl+T` on the Settings tab to reload
   the template.

If the template has an error, issuing fails with a message that includes the
line number. Nothing is issued and no number is used up.

Delete `template.typ` to go back to the built-in design.

> **Regenerating old PDFs.** `R` on an issued invoice re-renders it with the
> details frozen when it was issued, but with the **current** template. If
> you've changed the template since, the new PDF will look different from the
> one you sent, and tally warns you before it overwrites the file.

## Fonts

The template runs in a sandbox. Only fonts bundled with tally are available,
plus any `.ttf`, `.otf`, `.ttc` or `.otc` files you put in
`<output folder>/fonts/`.

Bundled fonts:
- Libertinus Serif, used by the default template
- New Computer Modern
- DejaVu Sans Mono

To use your own font, drop the font files into the `fonts/` folder and set it
in the template:

```typst
#set text(font: "Inter")
```

## Sandbox

A template can only read `/data.json`. It can't read other files or download
Typst packages, so a template can't change between machines or over time
without you editing it.

## Data available to the template

Read it with `#let data = json("/data.json")`. Every value is a ready-to-print
string. Money is already formatted (`£1,800.00`) and dates are written out
(`30 September 2026`).

| Field | Example | Notes |
|---|---|---|
| `kind` | `"invoice"` / `"credit_note"` | |
| `title` | `"INVOICE"` / `"CREDIT NOTE"` | |
| `number` | `"2026-0001"` or `"LGT-2026-0001"` | |
| `business.name` | `"Nick Reiners"` | |
| `business.address` | `["1 High Street", "Bristol", "BS1 1AA"]` | blank lines removed |
| `business.email`, `business.phone` | | may be `""` |
| `client_heading` | `"Bill to"` / `"Credit to"` | |
| `client.name` | `"Acme Ltd"` | as it was when issued |
| `client.contact` | `"Accounts Payable"` | may be `""` |
| `client.address` | `["Hauptstraße 1", "Berlin", "10115", "Germany"]` | always ends with the country |
| `client.tax_id` | `"VAT ID: DE123456789"` | `""` if the client has no tax ID |
| `meta` | `[["Invoice date", "30 September 2026"], ["Supply period", "1–30 September 2026"], ["Due date", "30 October 2026"]]` | label/value pairs; a credit note also lists the invoice it's against |
| `reason` | `"Hosting cancelled"` | credit notes only, otherwise `""` |
| `lines` | `[{description, quantity, rate, amount}]` | e.g. `"12.5 hrs"`, `"£60.00/hr"`, `"£750.00"` |
| `total_label` | `"Total due"` / `"Total credited"` | |
| `total` | `"£1,800.00"` | |
| `payment` | `{account_name, sort_code, account_number, reference, terms}` or `none` | `none` on credit notes; `reference` is the invoice number |
| `notes` | `"Thank you for your business."` | may be `""` |
| `late_payment_notice` | statutory interest wording, or `""` | only for business clients with the notice switched on |

New fields may be added in later versions. Existing ones won't be renamed
without a note in the changelog.
