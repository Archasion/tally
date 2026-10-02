// tally invoice / credit note template.
//
// To customise, copy this file to `template.typ` in your output folder and
// edit it; tally uses that copy instead of this built-in one. Extra fonts can
// be dropped into `<output folder>/fonts/`. The data shape is documented in
// docs/TEMPLATES.md.

#let data = json("/data.json")

#let muted = luma(95)
#let hairline = 0.5pt + luma(185)
#let caps(body) = text(8pt, fill: muted, tracking: 0.06em, weight: "semibold", upper(body))

#set document(title: data.title + " " + data.number, author: data.business.name)
#set text(font: "Libertinus Serif", size: 10.5pt, lang: "en", region: "gb")
#set par(spacing: 0.7em, leading: 0.6em)
#set page(
  paper: "a4",
  margin: (x: 2cm, top: 2cm, bottom: 2.4cm),
  footer: context {
    set text(8pt, fill: muted)
    data.business.name
    h(1fr)
    data.title + " " + data.number
    h(1fr)
    [Page #counter(page).display() of #counter(page).final().first()]
  },
)

// ----- Header ---------------------------------------------------------------

#grid(
  columns: (1fr, auto),
  align: (left + top, right + top),
  [
    #text(17pt, weight: "bold", data.business.name) \
    #for l in data.business.address [#l \ ]
    #if data.business.email != "" [#data.business.email \ ]
    #if data.business.phone != "" [#data.business.phone]
  ],
  [
    #text(24pt, weight: "bold", tracking: 0.1em, data.title) \
    #v(1pt)
    #text(12pt, data.number)
  ],
)

#v(0.6em)
#line(length: 100%, stroke: hairline)
#v(0.6em)

// ----- Parties and dates ----------------------------------------------------

#grid(
  columns: (1fr, auto),
  column-gutter: 2em,
  [
    #caps[#data.client_heading] \
    #v(1pt)
    #text(weight: "bold", data.client.name) \
    #if data.client.contact != "" [#data.client.contact \ ]
    #for l in data.client.address [#l \ ]
  ],
  grid(
    columns: (auto, auto),
    column-gutter: 1.5em,
    row-gutter: 0.75em,
    align: (left + horizon, right + horizon),
    ..data.meta.map(((k, v)) => (caps(k), v)).flatten(),
  ),
)

#if data.reason != "" {
  v(1em)
  [#caps[Reason] \ #data.reason]
}

// ----- Lines ----------------------------------------------------------------

#v(1.4em)
#table(
  columns: (1fr, auto, auto, auto),
  align: (left, right, right, right),
  inset: (x: 5pt, y: 7pt),
  stroke: (_, y) => if y == 0 { (bottom: 0.8pt + black) } else { (bottom: hairline) },
  table.header(caps[Description], caps[Quantity], caps[Rate], caps[Amount]),
  ..data.lines.map(l => (l.description, l.quantity, l.rate, l.amount)).flatten(),
)

#v(0.4em)
// Right padding matches the table's cell inset so the total lines up with Amount.
#align(right, pad(right: 5pt, grid(
  columns: (auto, auto),
  column-gutter: 2em,
  align: right + horizon,
  text(weight: "bold", data.total_label), text(13pt, weight: "bold", data.total),
)))

// ----- Payment --------------------------------------------------------------

#if data.payment != none {
  v(1.6em)
  block(width: 100%, inset: 12pt, radius: 3pt, stroke: hairline, breakable: false)[
    #caps[Payment details]
    #v(0.3em)
    #grid(
      columns: (auto, 1fr),
      column-gutter: 1.5em,
      row-gutter: 0.6em,
      [Account name], data.payment.account_name,
      [Sort code], data.payment.sort_code,
      [Account number], data.payment.account_number,
      [Reference], text(weight: "bold", data.payment.reference),
    )
    #v(0.3em)
    #text(fill: muted, data.payment.terms)
  ]
}

#if data.notes != "" {
  v(1.2em)
  data.notes
}

#if data.late_payment_notice != "" {
  v(1.2em)
  text(8.5pt, fill: muted, data.late_payment_notice)
}
