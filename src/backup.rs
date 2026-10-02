//! Consistent database backups into the (synced) output folder, using
//! SQLite's online backup API rather than copying a file that may be open.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Local;
use rusqlite::MAIN_DB;

use crate::db::Db;
use crate::paths;

/// Writes `backups/tally-YYYYMMDD-HHMMSS.db` and keeps only the newest `keep`.
pub fn run(db: &Db, output_dir: &Path, keep: u32) -> Result<PathBuf> {
    let dir = paths::backups_dir(output_dir);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let stamp = Local::now().format("%Y%m%d-%H%M%S");
    let target = dir.join(format!("tally-{stamp}.db"));
    // Back up to a temporary name first so a half-written file is never
    // mistaken for a good backup (or synced as one).
    let partial = dir.join(format!(".tally-{stamp}.db.partial"));
    db.connection()
        .backup(MAIN_DB, &partial, None)
        .with_context(|| format!("backing up to {}", partial.display()))?;
    std::fs::rename(&partial, &target)?;
    prune(&dir, keep.max(1) as usize)?;
    Ok(target)
}

fn prune(dir: &Path, keep: usize) -> Result<()> {
    let mut backups: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("tally-") && n.ends_with(".db"))
        })
        .collect();
    // Timestamped names sort chronologically.
    backups.sort();
    let excess = backups.len().saturating_sub(keep);
    for old in &backups[..excess] {
        std::fs::remove_file(old).with_context(|| format!("removing {}", old.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prunes_oldest_backups() {
        let dir = std::env::temp_dir().join(format!("tally-prune-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for stamp in ["20260101-000000", "20260102-000000", "20260103-000000"] {
            std::fs::write(dir.join(format!("tally-{stamp}.db")), b"x").unwrap();
        }
        std::fs::write(dir.join("notes.txt"), b"keep me").unwrap();
        prune(&dir, 2).unwrap();
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "notes.txt",
                "tally-20260102-000000.db",
                "tally-20260103-000000.db"
            ]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn backup_is_a_readable_database() {
        let dir = std::env::temp_dir().join(format!("tally-backup-{}", std::process::id()));
        let (db, _) = crate::db::test_support::db_with_client();
        let path = run(&db, &dir, 3).unwrap();
        let restored = Db::open(&path).unwrap();
        assert_eq!(restored.clients(true).unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
