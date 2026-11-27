use crate::{Error, Result, StorePaths, WAIT, connect, database_error, paths};
use rusqlite::{
    Connection, TransactionBehavior,
    backup::{Backup, StepResult},
};
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::Path,
    thread,
    time::{Duration, Instant},
};
use tempfile::NamedTempFile;

const APP_ID: i64 = 0x454e564c;
const STEPS: [&str; 7] = [
    include_str!("../migrations/001_baseline.sql"),
    include_str!("../migrations/002_project_description.sql"),
    include_str!("../migrations/003_workspace_trust.sql"),
    include_str!("../migrations/004_task_approval.sql"),
    include_str!("../migrations/005_execution_evidence.sql"),
    include_str!("../migrations/006_endpoint_approval.sql"),
    include_str!("../migrations/007_credential_cleanup.sql"),
];
pub(super) const CURRENT: i64 = STEPS.len() as i64;

pub(super) fn migration_lock(paths: &StorePaths) -> Result<File> {
    let path = paths.root().join("data").join("migration.lock");
    paths::check_file(&path)?;
    let file = match paths::create_file(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|_| Error::StorageUnavailable)?,
        Err(_) => return Err(Error::StorageUnavailable),
    };
    let start = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(fs::TryLockError::WouldBlock) if start.elapsed() < WAIT => {
                thread::sleep(Duration::from_millis(25))
            }
            Err(fs::TryLockError::WouldBlock) => return Err(Error::StorageBusy),
            Err(fs::TryLockError::Error(_)) => return Err(Error::StorageUnavailable),
        }
    }
}

pub(super) fn version(conn: &Connection) -> Result<i64> {
    let app: i64 = conn
        .query_row("PRAGMA application_id", [], |r| r.get(0))
        .map_err(database_error)?;
    let v: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(database_error)?;
    if app == 0 && v == 0 {
        let objects: i64 = conn
            .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))
            .map_err(database_error)?;
        return if objects == 0 {
            Ok(0)
        } else {
            Err(Error::InvalidDatabase)
        };
    }
    if app != APP_ID || v < 1 {
        return Err(Error::InvalidDatabase);
    }
    if v > CURRENT {
        return Err(Error::UpgradeRequired);
    }
    let history: (i64, i64, i64) = conn
        .query_row(
            "SELECT count(*), min(version), max(version) FROM schema_migrations",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| Error::InvalidDatabase)?;
    if history != (v, 1, v) {
        return Err(Error::InvalidDatabase);
    }
    Ok(v)
}

fn verify(conn: &Connection, expected: i64) -> Result<()> {
    if version(conn)? != expected {
        return Err(Error::InvalidDatabase);
    }
    let integrity: String = conn
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(database_error)?;
    if integrity != "ok"
        || conn
            .prepare("PRAGMA foreign_key_check")
            .map_err(database_error)?
            .exists([])
            .map_err(database_error)?
    {
        return Err(Error::InvalidDatabase);
    }
    Ok(())
}

pub(super) fn snapshot(source: &Connection, destination: &Path, expected: i64) -> Result<()> {
    let parent = destination.parent().ok_or(Error::StorageUnavailable)?;
    let tmp = NamedTempFile::new_in(parent).map_err(|_| Error::StorageUnavailable)?;
    {
        let mut dest = connect(tmp.path(), true)?;
        if Backup::new(source, &mut dest)
            .map_err(database_error)?
            .step(-1)
            .map_err(database_error)?
            != StepResult::Done
        {
            return Err(Error::StorageBusy);
        }
        // Publish a self-contained file which can be moved without journaling sidecars.
        let mode: String = dest
            .query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))
            .map_err(database_error)?;
        if mode != "delete" {
            return Err(Error::StorageBusy);
        }
        verify(&dest, expected)?;
        dest.close().map_err(|(_, e)| database_error(e))?;
    }
    tmp.as_file()
        .sync_all()
        .map_err(|_| Error::StorageUnavailable)?;
    tmp.persist_noclobber(destination).map_err(|e| {
        if e.error.kind() == io::ErrorKind::AlreadyExists {
            Error::DestinationExists
        } else {
            Error::StorageUnavailable
        }
    })?;
    #[cfg(unix)]
    File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(|_| Error::StorageUnavailable)?;
    Ok(())
}

pub(super) fn migrate(paths: &StorePaths) -> Result<()> {
    let path = paths.database();
    migrate_connection(paths, &mut connect(&path, true)?)
}

pub(super) fn migrate_connection(paths: &StorePaths, conn: &mut Connection) -> Result<()> {
    let current = version(conn)?;
    let mode: String = conn
        .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
        .map_err(database_error)?;
    if mode != "wal" {
        return Err(Error::StorageUnavailable);
    }
    if current == CURRENT {
        return Ok(());
    }
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let from = version(&tx)?;
    if from > 0 && from < CURRENT {
        let source = connect(&paths.database(), false)?;
        // The writer reservation pins the committed source until the upgrade commits.
        let backup = paths.backups().join(format!(
            "schema-{from}-{}.db",
            envlume_domain::RequestId::generate().0
        ));
        snapshot(&source, &backup, from)?;
    }
    for (index, sql) in STEPS.iter().enumerate().skip(from as usize) {
        let next = index as i64 + 1;
        tx.execute_batch(sql).map_err(|_| Error::MigrationFailed)?;
        tx.execute("INSERT INTO schema_migrations(version,applied_at) VALUES (?1,CAST(unixepoch('subsec')*1000 AS INTEGER))",[next]).map_err(|_|Error::MigrationFailed)?;
        tx.pragma_update(None, "application_id", APP_ID)
            .map_err(database_error)?;
        tx.pragma_update(None, "user_version", next)
            .map_err(database_error)?;
    }
    #[cfg(test)]
    crate::tests::pause_uncommitted_migration(paths);
    verify(&tx, CURRENT)?;
    tx.commit().map_err(database_error)
}
