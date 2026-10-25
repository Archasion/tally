//! End-to-end: set up, invoice, get paid, credit, refund, report, export, back up —
//! using the same library calls as the TUI, with real PDFs written to a temp folder.

use chrono::{Days, NaiveDate};
use tally::db::{Db, TxKind};
use tally::domain::dates::TaxYear;
use tally::domain::*;
use tally::pdf::Renderer;
use tally::{backup, paths, reports};

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

#[test]
fn full_invoice_lifecycle() {
    let out = std::env::temp_dir().join(format!("tally-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let db = Db::open(&out.join("tally.db")).unwrap();

    db.save_business(&Business {
        name: "Nick Reiners".into(),
        address: Address {
            line1: "1 High St".into(),
            line2: String::new(),
            town: "Bristol".into(),
            postcode: "BS1 1AA".into(),
            country: String::new(),
        },
        email: String::new(),
        phone: String::new(),
        bank: BankDetails {
            account_name: "N Reiners".into(),
            sort_code: "12-34-56".into(),
            account_number: "12345678".into(),
        },
        payment_terms_days: 30,
        default_notes: String::new(),
        invoice_pattern: "{LABEL}-{YEAR}-{SEQ:4}".into(),
        credit_note_pattern: "CN-{YEAR}-{SEQ:3}".into(),
        output_dir: out.display().to_string(),
        backups_to_keep: 2,
    })
    .unwrap();

    let mut client = Client::new_empty();
    client.name = "Acme Ltd".into();
    client.label = "acm".into();
    client.address.line1 = "Hauptstraße 1".into();
    client.address.town = "Berlin".into();
    client.address.postcode = "10115".into();
    client.address.country = "Germany".into();
    client.tax_id_label = "VAT ID".into();
    client.tax_id = "DE123456789".into();
    let client_id = db.insert_client(&client).unwrap();

    let issue = d(2026, 9, 30);
    let draft = Invoice {
        id: 0,
        number: None,
        client_id,
        issue_date: issue,
        due_date: issue + Days::new(30),
        supply_start: d(2026, 9, 1),
        supply_end: Some(d(2026, 9, 30)),
        notes: String::new(),
        issued_at: None,
        sent_on: None,
        snapshot: None,
        lines: vec![LineItem {
            kind: LineKind::Days,
            description: "Consultancy".into(),
            quantity: Quantity::parse("4").unwrap(),
            unit_price: Money::parse("450").unwrap(),
        }],
    };
    let id = db.create_draft(&draft).unwrap();

    // Issue: number assigned, PDF written inside the same transaction.
    let renderer = Renderer::new(&out).unwrap();
    let inv = db
        .issue_invoice(id, issue.and_hms_opt(9, 0, 0).unwrap(), |inv| {
            renderer.write_invoice(&out, inv).map(|_| ())
        })
        .unwrap();
    assert_eq!(inv.number.as_deref(), Some("ACM-2026-0001"));
    let pdf = paths::invoice_pdf(&out, &inv);
    assert!(pdf.ends_with("2026/ACM-2026-0001_Acme-Ltd.pdf"));
    assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF-"));
    let data = tally::pdf::DocumentData::for_invoice(&inv).unwrap();
    assert_eq!(
        data.client.address.last().map(String::as_str),
        Some("Germany")
    );
    assert_eq!(data.client.tax_id, "VAT ID: DE123456789");
    assert!(data.late_payment_notice.is_empty());

    // Renaming the client later doesn't change the issued invoice.
    let mut renamed = db.client(client_id).unwrap();
    renamed.name = "Acme Group plc".into();
    db.update_client(&renamed).unwrap();
    assert_eq!(
        db.invoice(id).unwrap().snapshot.unwrap().client.name,
        "Acme Ltd"
    );

    db.set_sent(id, Some(issue)).unwrap();
    let pay = |amount: &str, date| Transaction {
        id: 0,
        invoice_id: id,
        date,
        amount: Money::parse(amount).unwrap(),
        method: "Bank transfer".into(),
        reference: String::new(),
    };
    db.add_payment(&pay("1000", d(2026, 10, 10))).unwrap();
    db.add_payment(&pay("800", d(2026, 10, 20))).unwrap();
    let today = d(2026, 11, 15);
    let rows = reports::invoice_rows(&db, today).unwrap();
    assert_eq!(rows[0].status, Status::Paid);

    // Credit £450 after payment → owed back → refund it in the next tax year.
    let cn = db
        .create_credit_note(
            id,
            d(2027, 3, 1),
            "One day not delivered",
            vec![LineItem {
                kind: LineKind::Fixed,
                description: "Credit".into(),
                quantity: Quantity::ONE,
                unit_price: Money::parse("450").unwrap(),
            }],
            |note, inv| renderer.write_credit_note(&out, note, inv).map(|_| ()),
        )
        .unwrap();
    assert!(paths::credit_note_pdf(&out, &cn).exists());
    assert_eq!(
        db.totals(id).unwrap().balance(),
        Money::parse("-450").unwrap()
    );
    db.add_refund(&pay("450", d(2027, 4, 10))).unwrap();
    assert_eq!(db.totals(id).unwrap().balance(), Money::ZERO);

    // Cash basis: the refund lands in 2027-28, not 2026-27.
    let y26 = reports::income(&db, TaxYear(2026)).unwrap();
    assert_eq!(y26.net(), Money::parse("1800").unwrap());
    let y27 = reports::income(&db, TaxYear(2027)).unwrap();
    assert_eq!(y27.net(), Money::parse("-450").unwrap());

    // Tax: £600 of expenses is under the £1,000 trading allowance, so the allowance is used.
    db.save_expense(&tally::domain::expense::Expense {
        id: 0,
        date: d(2026, 6, 1),
        description: "Software".into(),
        category: tally::domain::expense::ExpenseCategory::Office,
        amount: Money::parse("600").unwrap(),
        notes: String::new(),
    })
    .unwrap();
    let book = tally::tax::rates::RatesBook::load(&out).unwrap();
    let reports::TaxReport::Ready(view) =
        reports::tax_report(&db, &book, TaxYear(2026), today).unwrap()
    else {
        panic!("2026-27 rates are built in");
    };
    assert_eq!(view.actual.figures.turnover, Money::parse("1800").unwrap());
    assert_eq!(
        view.actual.deduction,
        tally::tax::Deduction::TradingAllowance(Money::parse("1000").unwrap())
    );
    assert_eq!(view.actual.profit, Money::parse("800").unwrap());
    assert!(reports::tax_text(&view, false).contains("Voluntary contributions"));

    // No 2027-28 rates until rates.toml provides them.
    assert!(matches!(
        reports::tax_report(&db, &book, TaxYear(2027), today).unwrap(),
        reports::TaxReport::NoRates(_)
    ));
    let next_year = tally::tax::rates::BUILTIN_RATES
        .split("\n\n\n[years.2025]")
        .next()
        .unwrap()
        .replace("[years.2026", "[years.2027");
    std::fs::write(paths::rates_override(&out), next_year).unwrap();
    let book = tally::tax::rates::RatesBook::load(&out).unwrap();
    assert!(matches!(
        reports::tax_report(&db, &book, TaxYear(2027), today).unwrap(),
        reports::TaxReport::Ready(_)
    ));

    // Export and backups.
    let export = reports::export_csv(&db, &out, today).unwrap();
    let invoices_csv = std::fs::read_to_string(export.join("invoices.csv")).unwrap();
    assert!(
        invoices_csv.contains("ACM-2026-0001,Paid,Acme Ltd,"),
        "{invoices_csv}"
    );
    assert_eq!(db.transactions(TxKind::Refund, None).unwrap().len(), 1);
    backup::run(&db, &out, 2).unwrap();

    std::fs::remove_dir_all(&out).unwrap();
}
