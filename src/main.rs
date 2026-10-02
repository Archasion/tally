use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};

use tally::db::Db;
use tally::pdf::{DocumentData, Renderer, sample_invoice};
use tally::{backup, paths, ui};

const HELP: &str = "\
tally — a terminal invoice manager for UK sole traders

USAGE:
    tally                     Open the full-screen app
    tally backup              Back up the database to <output folder>/backups
    tally sample-pdf [FILE]   Render a sample invoice (default: sample-invoice.pdf)
                              using your template override if one exists
    tally where               Show where the database and output folder are
    tally help                Show this help

ENVIRONMENT:
    TALLY_DB                  Use this database file instead of the default
";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => {
            let db = Db::open(&paths::database_path()?)?;
            ui::run(db)
        }
        Some("backup") => {
            let db = Db::open(&paths::database_path()?)?;
            let business = db.require_business().context("run `tally` once to set up first")?;
            let path = backup::run(&db, &paths::expand_home(&business.output_dir), business.backups_to_keep)?;
            println!("Backed up to {}", path.display());
            Ok(())
        }
        Some("sample-pdf") => {
            let out = PathBuf::from(args.get(1).map(String::as_str).unwrap_or("sample-invoice.pdf"));
            let renderer = match Db::open(&paths::database_path()?)?.business()? {
                Some(b) => Renderer::new(&paths::expand_home(&b.output_dir))?,
                None => Renderer::builtin(),
            };
            let pdf = renderer.render(&DocumentData::for_invoice(&sample_invoice())?)?;
            std::fs::write(&out, pdf).with_context(|| format!("writing {}", out.display()))?;
            let source = if renderer.uses_template_override() { "your template override" } else { "the built-in template" };
            println!("Wrote {} using {source}", out.display());
            Ok(())
        }
        Some("where") => {
            println!("Database:      {}", paths::database_path()?.display());
            match Db::open(&paths::database_path()?)?.business()? {
                Some(b) => println!("Output folder: {}", paths::expand_home(&b.output_dir).display()),
                None => println!("Output folder: (not set up yet — run `tally`)"),
            }
            Ok(())
        }
        Some("help" | "-h" | "--help") => {
            print!("{HELP}");
            Ok(())
        }
        Some(other) => anyhow::bail!("unknown command '{other}'\n\n{HELP}"),
    }
}
