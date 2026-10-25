use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};

use tally::db::Db;
use tally::domain::dates;
use tally::pdf::{DocumentData, Renderer, sample_invoice};
use tally::tax::rates::RatesBook;
use tally::{backup, paths, reports, tax, ui};

const HELP: &str = "\
tally — a terminal invoice manager for UK sole traders

USAGE:
    tally                     Open the full-screen app
    tally backup              Back up the database to <output folder>/backups
    tally sample-pdf [FILE]   Render a sample invoice (default: sample-invoice.pdf)
                              using your template override if one exists
    tally tax [YEAR] [--projected]
                              Print the tax estimate for a tax year (default:
                              the current one), e.g. `tally tax 2025-26`
    tally rates               Print the built-in tax rates in rates.toml format
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
            let business = db
                .require_business()
                .context("run `tally` once to set up first")?;
            let path = backup::run(
                &db,
                &paths::expand_home(&business.output_dir),
                business.backups_to_keep,
            )?;
            println!("Backed up to {}", path.display());
            Ok(())
        }
        Some("sample-pdf") => {
            let out = PathBuf::from(
                args.get(1)
                    .map(String::as_str)
                    .unwrap_or("sample-invoice.pdf"),
            );
            let renderer = match Db::open(&paths::database_path()?)?.business()? {
                Some(b) => Renderer::new(&paths::expand_home(&b.output_dir))?,
                None => Renderer::builtin(),
            };
            let pdf = renderer.render(&DocumentData::for_invoice(&sample_invoice())?)?;
            std::fs::write(&out, pdf).with_context(|| format!("writing {}", out.display()))?;
            let source = if renderer.uses_template_override() {
                "your template override"
            } else {
                "the built-in template"
            };
            println!("Wrote {} using {source}", out.display());
            Ok(())
        }
        Some("tax") => {
            let db = Db::open(&paths::database_path()?)?;
            let business = db
                .require_business()
                .context("run `tally` once to set up first")?;
            let book = RatesBook::load(&paths::expand_home(&business.output_dir))?;
            let projected = args.iter().any(|a| a == "--projected");
            let year = match args.iter().skip(1).find(|a| !a.starts_with("--")) {
                Some(y) => tax::parse_year(y)
                    .with_context(|| format!("'{y}' isn't a tax year; use e.g. 2026 or 2026-27"))?,
                None => reports::current_tax_year(),
            };
            match reports::tax_report(&db, &book, year, dates::today())? {
                reports::TaxReport::Ready(view) => {
                    print!("{}", reports::tax_text(&view, projected))
                }
                reports::TaxReport::NoRates(y) => anyhow::bail!(
                    "no tax rates for {}. Add a [years.{}] block to {} (see `tally rates`)",
                    y.label(),
                    y.0,
                    paths::rates_override(&paths::expand_home(&business.output_dir)).display()
                ),
            }
            Ok(())
        }
        Some("rates") => {
            print!("{}", tally::tax::rates::BUILTIN_RATES);
            Ok(())
        }
        Some("where") => {
            println!("Database:      {}", paths::database_path()?.display());
            match Db::open(&paths::database_path()?)?.business()? {
                Some(b) => println!(
                    "Output folder: {}",
                    paths::expand_home(&b.output_dir).display()
                ),
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
