use anyhow::{Context, Result, bail};
use rusqlite::{Row, params};

use super::Db;
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
        validate(c)?;
        self.conn.execute(
            "INSERT INTO clients (name, contact_name, email, address_line1, address_line2, town,
                postcode, is_business, late_payment_notice, default_rate_pence, default_rate_kind, archived)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                c.name.trim(),
                c.contact_name,
                c.email,
                c.address.line1,
                c.address.line2,
                c.address.town,
                c.address.postcode,
                c.is_business,
                c.late_payment_notice,
                c.default_rate.map(Money::pence),
                c.default_rate_kind.as_str(),
                c.archived,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_client(&self, c: &Client) -> Result<()> {
        validate(c)?;
        let changed = self.conn.execute(
            "UPDATE clients SET name = ?2, contact_name = ?3, email = ?4, address_line1 = ?5,
                address_line2 = ?6, town = ?7, postcode = ?8, is_business = ?9,
                late_payment_notice = ?10, default_rate_pence = ?11, default_rate_kind = ?12,
                archived = ?13
             WHERE id = ?1",
            params![
                c.id,
                c.name.trim(),
                c.contact_name,
                c.email,
                c.address.line1,
                c.address.line2,
                c.address.town,
                c.address.postcode,
                c.is_business,
                c.late_payment_notice,
                c.default_rate.map(Money::pence),
                c.default_rate_kind.as_str(),
                c.archived,
            ],
        )?;
        if changed == 0 {
            bail!("client {} not found", c.id);
        }
        Ok(())
    }

    pub fn set_client_archived(&self, id: i64, archived: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE clients SET archived = ?2 WHERE id = ?1",
            params![id, archived],
        )?;
        Ok(())
    }
}

fn validate(c: &Client) -> Result<()> {
    if c.name.trim().is_empty() {
        bail!("Client name is required");
    }
    Ok(())
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
        },
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
