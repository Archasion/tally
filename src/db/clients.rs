use anyhow::{Context, Result, bail};
use rusqlite::{Row, params};

use super::Db;
use crate::domain::numbering::normalize_label;
use crate::domain::{Address, Client, LineKind, Money};

impl Db {
    pub fn clients(&self, include_archived: bool) -> Result<Vec<Client>> {
        let mut stmt = self.conn.prepare(
            "SELECT * FROM clients WHERE archived = 0 OR ?1 ORDER BY archived, name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([include_archived], client_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn client(&self, id: i64) -> Result<Client> {
        self.conn
            .query_row("SELECT * FROM clients WHERE id = ?1", [id], client_from_row)
            .with_context(|| format!("client {id} not found"))
    }

    pub fn insert_client(&self, c: &Client) -> Result<i64> {
        let c = validate(c)?;
        self.write_client(
            "INSERT INTO clients (id, name, contact_name, email, address_line1, address_line2,
                town, postcode, country, label, tax_id_label, tax_id, is_business,
                late_payment_notice, default_rate_pence, default_rate_kind, archived)
             VALUES (NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            &c,
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_client(&self, c: &Client) -> Result<()> {
        let c = validate(c)?;
        let changed = self.write_client(
            "UPDATE clients SET name = ?2, contact_name = ?3, email = ?4, address_line1 = ?5,
                address_line2 = ?6, town = ?7, postcode = ?8, country = ?9, label = ?10,
                tax_id_label = ?11, tax_id = ?12, is_business = ?13, late_payment_notice = ?14,
                default_rate_pence = ?15, default_rate_kind = ?16, archived = ?17
             WHERE id = ?1",
            &c,
        )?;
        if changed == 0 {
            bail!("client {} not found", c.id);
        }
        Ok(())
    }

    /// Runs an insert/update with the client's fields bound as ?1..?17,
    /// turning constraint failures into readable messages.
    fn write_client(&self, sql: &str, c: &Client) -> Result<usize> {
        let result = self.conn.execute(
            sql,
            params![
                c.id,
                c.name,
                c.contact_name.trim(),
                c.email.trim(),
                c.address.line1.trim(),
                c.address.line2.trim(),
                c.address.town.trim(),
                c.address.postcode.trim(),
                c.address.country.trim(),
                c.label,
                c.tax_id_label.trim(),
                c.tax_id.trim(),
                c.is_business,
                c.late_payment_notice,
                c.default_rate.map(Money::pence),
                c.default_rate_kind.as_str(),
                c.archived,
            ],
        );
        match result {
            Ok(n) => Ok(n),
            Err(e) if e.to_string().contains("clients.label") => {
                let owner: String = self
                    .conn
                    .query_row(
                        "SELECT name FROM clients WHERE label = ?1",
                        [&c.label],
                        |r| r.get(0),
                    )
                    .unwrap_or_default();
                bail!("The label {} is already used by {owner}", c.label)
            }
            Err(e) if e.to_string().contains("client label is locked") => {
                bail!(
                    "{}'s label can't change: invoices have already been issued with it, so their numbering must stay continuous",
                    c.name
                )
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn set_client_archived(&self, id: i64, archived: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE clients SET archived = ?2 WHERE id = ?1",
            params![id, archived],
        )?;
        Ok(())
    }
}

/// Checks required fields and returns a copy with the name trimmed and the
/// label normalised (uppercase A–Z/0–9).
fn validate(c: &Client) -> Result<Client> {
    if c.name.trim().is_empty() {
        bail!("Client name is required");
    }
    if c.tax_id.trim().is_empty() != c.tax_id_label.trim().is_empty() {
        bail!("Enter both the tax ID name (e.g. VAT ID) and the number, or neither");
    }
    let mut c = c.clone();
    c.name = c.name.trim().to_string();
    c.label = normalize_label(&c.label).map_err(anyhow::Error::msg)?;
    Ok(c)
}

fn client_from_row(r: &Row) -> rusqlite::Result<Client> {
    let kind: String = r.get("default_rate_kind")?;
    Ok(Client {
        id: r.get("id")?,
        name: r.get("name")?,
        contact_name: r.get("contact_name")?,
        email: r.get("email")?,
        address: Address {
            line1: r.get("address_line1")?,
            line2: r.get("address_line2")?,
            town: r.get("town")?,
            postcode: r.get("postcode")?,
            country: r.get("country")?,
        },
        label: r.get("label")?,
        tax_id_label: r.get("tax_id_label")?,
        tax_id: r.get("tax_id")?,
        is_business: r.get("is_business")?,
        late_payment_notice: r.get("late_payment_notice")?,
        default_rate: r
            .get::<_, Option<i64>>("default_rate_pence")?
            .map(Money::from_pence),
        default_rate_kind: LineKind::parse(&kind).unwrap_or(LineKind::Hours),
        archived: r.get("archived")?,
    })
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;

    #[test]
    fn archive_hides_clients() {
        let (db, id) = db_with_client();
        assert_eq!(db.clients(false).unwrap().len(), 1);
        db.set_client_archived(id, true).unwrap();
        assert!(db.clients(false).unwrap().is_empty());
        assert_eq!(db.clients(true).unwrap().len(), 1);
    }

    #[test]
    fn update_round_trips() {
        let (db, id) = db_with_client();
        let mut c = db.client(id).unwrap();
        c.default_rate = Some(crate::domain::Money::from_pence(45_000));
        c.default_rate_kind = crate::domain::LineKind::Days;
        c.late_payment_notice = true;
        db.update_client(&c).unwrap();
        assert_eq!(db.client(id).unwrap(), c);
        c.name = "  ".into();
        assert!(db.update_client(&c).is_err());
    }
}
