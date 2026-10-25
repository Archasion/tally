//! SQLite storage. All money-moving rules that must never be broken
//! (locking issued invoices, unique numbers) are also enforced by the schema.

mod clients;
mod invoices;
mod ledger;
mod tax;

pub use ledger::TxKind;

use std::path::Path;

use anyhow::{Context, Result, bail};
use chrono::NaiveDate;
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::domain::numbering::NumberPattern;
use crate::domain::{Address, BankDetails, Business, Client, LineItem, LineKind, Money, Quantity};

/// Migrations, applied in order. `PRAGMA user_version` records how many have run.
const MIGRATIONS: &[&str] = &[
    include_str!("schema.sql"),
    include_str!("002_international_clients.sql"),
    include_str!("003_expenses_and_tax.sql"),
];

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut db = Db { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&mut self) -> Result<()> {
        let version: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?;
        let version = usize::try_from(version).unwrap_or(0);
        if version > MIGRATIONS.len() {
            bail!(
                "database schema version {version} is newer than this build of tally supports ({})",
                MIGRATIONS.len()
            );
        }
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)
                .with_context(|| format!("applying migration {}", i + 1))?;
            tx.pragma_update(None, "user_version", (i + 1) as i64)?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    // ----- business -------------------------------------------------------

    pub fn business(&self) -> Result<Option<Business>> {
        Ok(self
            .conn
            .query_row("SELECT * FROM business WHERE id = 1", [], business_from_row)
            .optional()?)
    }

    pub fn require_business(&self) -> Result<Business> {
        self.business()?
            .context("business details have not been set up yet")
    }

    pub fn save_business(&self, b: &Business) -> Result<()> {
        NumberPattern::parse(&b.invoice_pattern).map_err(anyhow::Error::msg)?;
        NumberPattern::parse(&b.credit_note_pattern).map_err(anyhow::Error::msg)?;
        self.conn.execute(
            "INSERT INTO business (id, name, address_line1, address_line2, town, postcode, email, phone,
                bank_account_name, bank_sort_code, bank_account_number, payment_terms_days,
                default_notes, invoice_pattern, credit_note_pattern, output_dir, backups_to_keep)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
             ON CONFLICT (id) DO UPDATE SET
                name = excluded.name, address_line1 = excluded.address_line1,
                address_line2 = excluded.address_line2, town = excluded.town,
                postcode = excluded.postcode, email = excluded.email, phone = excluded.phone,
                bank_account_name = excluded.bank_account_name,
                bank_sort_code = excluded.bank_sort_code,
                bank_account_number = excluded.bank_account_number,
                payment_terms_days = excluded.payment_terms_days,
                default_notes = excluded.default_notes,
                invoice_pattern = excluded.invoice_pattern,
                credit_note_pattern = excluded.credit_note_pattern,
                output_dir = excluded.output_dir, backups_to_keep = excluded.backups_to_keep",
            params![
                b.name,
                b.address.line1,
                b.address.line2,
                b.address.town,
                b.address.postcode,
                b.email,
                b.phone,
                b.bank.account_name,
                b.bank.sort_code,
                b.bank.account_number,
                b.payment_terms_days,
                b.default_notes,
                b.invoice_pattern,
                b.credit_note_pattern,
                b.output_dir,
                b.backups_to_keep,
            ],
        )?;
        Ok(())
    }

    // ----- numbering ------------------------------------------------------

    /// Reserves the next free number for `kind` (`"invoice"` or `"credit_note"`).
    /// Must be called inside a transaction together with the insert/update that
    /// uses the number. Skips numbers already taken (e.g. after a pattern change).
    fn next_number(
        conn: &Connection,
        kind: &str,
        pattern: &str,
        date: NaiveDate,
        client: &Client,
        taken: impl Fn(&Connection, &str) -> Result<bool>,
    ) -> Result<String> {
        let pattern = NumberPattern::parse(pattern).map_err(anyhow::Error::msg)?;
        let label = client.label.as_str();
        if pattern.uses_label() && label.is_empty() {
            bail!(
                "{} has no label, and your number pattern uses {{LABEL}}. Give them one under Clients (e).",
                client.name
            );
        }
        let scope = pattern.scope(date, label);
        let last: u32 = conn
            .query_row(
                "SELECT last FROM sequences WHERE kind = ?1 AND scope = ?2",
                params![kind, scope],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        let mut seq = last + 1;
        let mut number = pattern.render(seq, date, label);
        while taken(conn, &number)? {
            seq += 1;
            number = pattern.render(seq, date, label);
        }
        conn.execute(
            "INSERT INTO sequences (kind, scope, last) VALUES (?1, ?2, ?3)
             ON CONFLICT (kind, scope) DO UPDATE SET last = excluded.last",
            params![kind, scope, seq],
        )?;
        Ok(number)
    }
}

fn business_from_row(r: &Row) -> rusqlite::Result<Business> {
    Ok(Business {
        name: r.get("name")?,
        address: Address {
            line1: r.get("address_line1")?,
            line2: r.get("address_line2")?,
            town: r.get("town")?,
            postcode: r.get("postcode")?,
            country: String::new(),
        },
        email: r.get("email")?,
        phone: r.get("phone")?,
        bank: BankDetails {
            account_name: r.get("bank_account_name")?,
            sort_code: r.get("bank_sort_code")?,
            account_number: r.get("bank_account_number")?,
        },
        payment_terms_days: r.get("payment_terms_days")?,
        default_notes: r.get("default_notes")?,
        invoice_pattern: r.get("invoice_pattern")?,
        credit_note_pattern: r.get("credit_note_pattern")?,
        output_dir: r.get("output_dir")?,
        backups_to_keep: r.get("backups_to_keep")?,
    })
}

fn line_from_row(r: &Row) -> rusqlite::Result<LineItem> {
    let kind: String = r.get("kind")?;
    Ok(LineItem {
        kind: LineKind::parse(&kind).unwrap_or(LineKind::Fixed),
        description: r.get("description")?,
        quantity: Quantity::from_hundredths(r.get("quantity_centi")?),
        unit_price: Money::from_pence(r.get("unit_price_pence")?),
    })
}

fn json_column<T: serde::de::DeserializeOwned>(r: &Row, col: &str) -> rusqlite::Result<Option<T>> {
    let raw: Option<String> = r.get(col)?;
    raw.map(|s| {
        serde_json::from_str(&s).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })
    })
    .transpose()
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub fn business() -> Business {
        Business {
            name: "Nick Reiners".into(),
            address: Address {
                line1: "1 High Street".into(),
                line2: String::new(),
                town: "Bristol".into(),
                postcode: "BS1 1AA".into(),
                country: String::new(),
            },
            email: "nick@example.com".into(),
            phone: String::new(),
            bank: BankDetails {
                account_name: "N Reiners".into(),
                sort_code: "12-34-56".into(),
                account_number: "12345678".into(),
            },
            payment_terms_days: 30,
            default_notes: "Thank you for your business.".into(),
            invoice_pattern: "{YEAR}-{SEQ:4}".into(),
            credit_note_pattern: "CN-{YEAR}-{SEQ:3}".into(),
            output_dir: "/tmp/tally-test".into(),
            backups_to_keep: 5,
        }
    }

    pub fn db_with_client() -> (Db, i64) {
        let db = Db::open_in_memory().unwrap();
        db.save_business(&business()).unwrap();
        let mut c = Client::new_empty();
        c.name = "Acme Ltd".into();
        c.address.line1 = "2 Market Square".into();
        c.address.town = "Bath".into();
        c.address.postcode = "BA1 1AA".into();
        let id = db.insert_client(&c).unwrap();
        (db, id)
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    #[test]
    fn migrations_are_idempotent() {
        let mut db = Db::open_in_memory().unwrap();
        db.migrate().unwrap();
        let v: i64 = db
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v as usize, MIGRATIONS.len());
    }

    #[test]
    fn upgrades_a_version_1_database() {
        // Build a v1 database with a client, then reopen it with the current code.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute("INSERT INTO clients (name) VALUES ('Old Client')", [])
            .unwrap();
        let db = Db::init(conn).unwrap();
        let c = &db.clients(true).unwrap()[0];
        assert_eq!(c.name, "Old Client");
        assert_eq!(c.address.country, "United Kingdom");
        assert_eq!(c.label, "");
    }

    #[test]
    fn business_round_trips_and_validates_patterns() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.business().unwrap().is_none());
        let mut b = business();
        db.save_business(&b).unwrap();
        assert_eq!(db.business().unwrap().unwrap(), b);
        b.address.town = "Leeds".into();
        db.save_business(&b).unwrap();
        assert_eq!(db.business().unwrap().unwrap().address.town, "Leeds");
        b.invoice_pattern = "{YEAR}".into();
        assert!(db.save_business(&b).is_err());
    }
}
