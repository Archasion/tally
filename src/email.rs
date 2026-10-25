//! Emailing invoices and credit notes: the message is written here and
//! opened in Apple Mail as a new message with the PDFs attached. Nothing is
//! ever sent; you review it in Mail and press Send yourself.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::domain::{Client, CreditNote, Invoice, Money, Totals, dates};

/// A message ready to open in Mail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// The client's email; empty means you fill it in in Mail.
    pub to: String,
    /// Your email from Settings; Mail uses it if it's one of your accounts.
    pub from: String,
    pub subject: String,
    pub body: String,
    pub attachments: Vec<PathBuf>,
}

/// The email for an issued invoice to `client` (as they are now, for their
/// email and greeting). `receipts` is how many receipt files are attached
/// after the PDF.
pub fn invoice_message(
    inv: &Invoice,
    totals: &Totals,
    client: &Client,
    from: &str,
    attachments: Vec<PathBuf>,
    receipts: usize,
) -> Result<Message> {
    let Some(snap) = &inv.snapshot else {
        bail!("issue the invoice before emailing it");
    };
    let number = inv.display_number();
    let name = &snap.business.name;
    let mut body = greeting(client);
    body.push_str(&format!(
        "Please find attached invoice {number} for {}, due by {}.\n",
        totals.total,
        dates::long(inv.due_date)
    ));
    let balance = totals.balance();
    if balance.is_positive() && balance < totals.total {
        body.push_str(&format!("{balance} of this is still outstanding.\n"));
    }
    match receipts {
        0 => {}
        1 => body.push_str("\nThe receipt for the reimbursed purchase is attached too.\n"),
        _ => body.push_str("\nThe receipts for the reimbursed purchases are attached too.\n"),
    }
    if balance.is_positive() {
        body.push_str(&format!(
            "\nPayment details are on the invoice; please use {number} as the reference.\n"
        ));
    }
    body.push_str(&sign_off(name));
    Ok(Message {
        to: client.email.trim().to_string(),
        from: from.trim().to_string(),
        subject: format!("Invoice {number} from {name}"),
        body,
        attachments,
    })
}

/// The email for a credit note against `invoice`.
pub fn credit_note_message(
    note: &CreditNote,
    invoice: &Invoice,
    owed_back: Money,
    client: &Client,
    from: &str,
    attachment: PathBuf,
) -> Message {
    let name = &note.snapshot.business.name;
    let mut body = greeting(client);
    body.push_str(&format!(
        "Please find attached credit note {} for {} against invoice {}.\n",
        note.number,
        note.total(),
        invoice.display_number()
    ));
    if !note.reason.trim().is_empty() {
        body.push_str(&format!("\nReason: {}\n", note.reason.trim()));
    }
    if owed_back.is_positive() {
        body.push_str(&format!(
            "\nI'll refund the {owed_back} you've overpaid to your account.\n"
        ));
    }
    body.push_str(&sign_off(name));
    Message {
        to: client.email.trim().to_string(),
        from: from.trim().to_string(),
        subject: format!("Credit note {} from {name}", note.number),
        body,
        attachments: vec![attachment],
    }
}

fn greeting(client: &Client) -> String {
    match client.greeting_name() {
        "" => "Hello,\n\n".into(),
        name => format!("Hi {name},\n\n"),
    }
}

fn sign_off(name: &str) -> String {
    format!("\nThank you,\n{name}\n")
}

/// Mail's scripting: a visible new message with recipient, sender,
/// subject, body and attachments. Values arrive as arguments, never spliced
/// into the script, so no text needs escaping.
const MAIL_SCRIPT: &str = r#"
on run argv
    set theSubject to item 1 of argv
    set theBody to item 2 of argv
    set theTo to item 3 of argv
    set theFrom to item 4 of argv
    tell application "Mail"
        set msg to make new outgoing message with properties {subject:theSubject, content:theBody & return, visible:true}
        tell msg
            if theTo is not "" then
                make new to recipient at end of to recipients with properties {address:theTo}
            end if
            if theFrom is not "" then
                try
                    set sender to theFrom
                end try
            end if
            repeat with i from 5 to count of argv
                make new attachment with properties {file name:(POSIX file (item i of argv)) as alias} at after the last paragraph of content
            end repeat
        end tell
        activate
    end tell
end run
"#;

/// Opens `message` in Apple Mail as a new, unsent message.
pub fn open_in_mail(message: &Message) -> Result<()> {
    for path in &message.attachments {
        if !path.is_file() {
            bail!("can't attach {}: the file is missing", path.display());
        }
    }
    let output = Command::new("osascript")
        .arg("-e")
        .arg(MAIL_SCRIPT)
        .args([&message.subject, &message.body, &message.to, &message.from])
        .args(&message.attachments)
        .output()
        .context("running osascript to open Mail")?;
    if output.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&output.stderr);
    if err.contains("-1743") || err.contains("Not authorized") {
        bail!(
            "macOS didn't let tally control Mail. Allow your terminal under System Settings → Privacy & Security → Automation → Mail, then try again"
        );
    }
    bail!("Mail couldn't open the message: {}", err.trim());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::sample_invoice;

    fn totals(total: i64, paid: i64) -> Totals {
        Totals {
            total: Money::from_pence(total),
            paid: Money::from_pence(paid),
            ..Totals::default()
        }
    }

    #[test]
    fn invoice_wording() {
        let mut inv = sample_invoice();
        inv.snapshot.as_mut().unwrap().business.name = "Nick Schreiner".into();
        let mut client = Client::new_empty();
        client.contact_name = "Jo Smith".into();
        client.email_name = " Jo ".into();
        client.email = " jo@acme.example ".into();
        let nobody = Client::new_empty();
        let m = invoice_message(
            &inv,
            &totals(182_490, 0),
            &client,
            "nick@example.com",
            vec![],
            2,
        )
        .unwrap();
        assert_eq!(m.to, "jo@acme.example");
        assert_eq!(m.subject, "Invoice EXC-2026-0001 from Nick Schreiner");
        assert_eq!(
            m.body,
            format!(
                "Hi Jo,\n\nPlease find attached invoice EXC-2026-0001 for £1,824.90, due by {}.\n\n\
                 The receipts for the reimbursed purchases are attached too.\n\n\
                 Payment details are on the invoice; please use EXC-2026-0001 as the reference.\n\n\
                 Thank you,\nNick Schreiner\n",
                dates::long(inv.due_date)
            )
        );

        // Part paid: say what's left. Paid in full: no payment request.
        let part = invoice_message(&inv, &totals(182_490, 82_490), &client, "", vec![], 0).unwrap();
        assert!(
            part.body
                .contains("£1,000.00 of this is still outstanding.")
        );
        assert!(!part.body.contains("receipt"));
        let paid =
            invoice_message(&inv, &totals(182_490, 182_490), &client, "", vec![], 0).unwrap();
        assert!(!paid.body.contains("Payment details"));

        // No email name: the contact. Neither: a plain hello.
        client.email_name.clear();
        let m = invoice_message(&inv, &totals(100, 0), &client, "", vec![], 0).unwrap();
        assert!(m.body.starts_with("Hi Jo Smith,\n\n"));
        let m = invoice_message(&inv, &totals(100, 0), &nobody, "", vec![], 1).unwrap();
        assert!(m.body.starts_with("Hello,\n\n"));
        assert_eq!(m.to, "");
        assert!(
            m.body
                .contains("The receipt for the reimbursed purchase is attached too.")
        );
        inv.snapshot = None;
        assert!(invoice_message(&inv, &totals(100, 0), &nobody, "", vec![], 0).is_err());
    }

    #[test]
    fn credit_note_wording() {
        let inv = sample_invoice();
        let note = CreditNote {
            id: 1,
            number: "CN-2026-001".into(),
            invoice_id: 0,
            date: inv.issue_date,
            reason: "Hosting no longer required".into(),
            lines: vec![inv.lines[2].clone()],
            snapshot: inv.snapshot.clone().unwrap(),
        };
        let m = credit_note_message(
            &note,
            &inv,
            Money::from_pence(15_000),
            &Client {
                email_name: "Accounts team".into(),
                email: "ap@example.com".into(),
                ..Client::new_empty()
            },
            "",
            PathBuf::from("/x.pdf"),
        );
        assert_eq!(m.subject, "Credit note CN-2026-001 from Your Name");
        assert_eq!(m.to, "ap@example.com");
        assert!(m.body.starts_with("Hi Accounts team,\n\n"));
        assert!(
            m.body
                .contains("credit note CN-2026-001 for £150.00 against invoice EXC-2026-0001.")
        );
        assert!(m.body.contains("Reason: Hosting no longer required"));
        assert!(m.body.contains("I'll refund the £150.00 you've overpaid"));
    }

    #[test]
    fn mail_script_compiles() {
        let out = std::env::temp_dir().join(format!("tally-mail-{}.scpt", std::process::id()));
        let status = Command::new("osacompile")
            .arg("-o")
            .arg(&out)
            .arg("-e")
            .arg(MAIL_SCRIPT)
            .status();
        let _ = std::fs::remove_file(&out);
        // Only checkable where AppleScript exists (macOS).
        if let Ok(status) = status {
            assert!(status.success(), "the Mail AppleScript doesn't compile");
        }
    }
}
