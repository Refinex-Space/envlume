//! SQLite metadata shared by the local entry points. Connections stay in Rust.
mod evidence;
pub use evidence::{EvidenceScope, RecordKey};
mod paths;
mod repository;
mod schema;
mod tasks;
mod workspaces;
pub use envlume_application::projects::StorageError as Error;
use envlume_application::projects::{ProjectEntry, ProjectRepository};
use envlume_domain::ProjectId;
pub use paths::StorePaths;
use rusqlite::{Connection, OpenFlags};
use std::{io, path::Path, time::Duration};
pub use workspaces::LocalRegistration;

type Result<T> = std::result::Result<T, Error>;
const WAIT: Duration = Duration::from_secs(5);

fn database_error(error: rusqlite::Error) -> Error {
    use rusqlite::ffi::ErrorCode::*;
    match error.sqlite_error_code() {
        Some(DatabaseBusy | DatabaseLocked) => Error::StorageBusy,
        Some(DatabaseCorrupt | NotADatabase) => Error::InvalidDatabase,
        _ => Error::StorageUnavailable,
    }
}

fn connect(path: &Path, writable: bool) -> Result<Connection> {
    paths::check_file(path)?;
    let mode = if writable {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let conn = Connection::open_with_flags(path, mode | OpenFlags::SQLITE_OPEN_NOFOLLOW)
        .map_err(database_error)?;
    conn.busy_timeout(WAIT).map_err(database_error)?;
    conn.execute_batch(
        "PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON; PRAGMA temp_store=MEMORY;",
    )
    .map_err(database_error)?;
    Ok(conn)
}

#[derive(Clone)]
pub struct Store {
    paths: StorePaths,
}

impl Store {
    pub fn open(paths: StorePaths) -> Result<Self> {
        paths.prepare()?;
        let path = paths.database();
        if path.try_exists().map_err(|_| Error::StorageUnavailable)? {
            let mut conn = connect(&path, false)?;
            let tx = conn.transaction().map_err(database_error)?;
            if schema::version(&tx)? == schema::CURRENT
                && tx
                    .query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
                    .map_err(database_error)?
                    == "wal"
            {
                return Ok(Self { paths });
            }
        }
        // Only initial creation, migration and restore need this process-wide file lock.
        let _lock = schema::migration_lock(&paths)?;
        if !path.try_exists().map_err(|_| Error::StorageUnavailable)? {
            for suffix in ["db-wal", "db-shm", "db-journal"] {
                if path
                    .with_extension(suffix)
                    .try_exists()
                    .map_err(|_| Error::StorageUnavailable)?
                {
                    return Err(Error::InvalidDatabase);
                }
            }
        }
        match paths::create_file(&path) {
            Ok(_) => (),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
            Err(_) => return Err(Error::StorageUnavailable),
        }
        schema::migrate(&paths)?;
        Ok(Self { paths })
    }

    /// Restores a verified SQLite snapshot into a fresh destination. Never replaces
    /// an open database or copies old WAL/SHM sidecars over newer committed work.
    pub fn restore_into(backup: &Path, paths: StorePaths) -> Result<Self> {
        paths.prepare()?;
        {
            let _lock = schema::migration_lock(&paths)?;
            let file = paths.database();
            for candidate in [
                &file,
                &file.with_extension("db-wal"),
                &file.with_extension("db-shm"),
                &file.with_extension("db-journal"),
            ] {
                if candidate
                    .try_exists()
                    .map_err(|_| Error::StorageUnavailable)?
                {
                    return Err(Error::DestinationExists);
                }
            }
            let source = connect(backup, false)?;
            let version = schema::version(&source)?;
            if version == 0 {
                return Err(Error::InvalidDatabase);
            }
            schema::snapshot(&source, &file, version)?;
        }
        Self::open(paths)
    }
}

impl ProjectRepository for Store {
    fn active_projects(&self) -> Result<Vec<ProjectEntry>> {
        let mut conn = connect(&self.paths.database(), false)?;
        let tx = conn.transaction().map_err(database_error)?;
        // Version and rows must come from the same snapshot, even if another
        // process upgrades after this Store handle was created.
        if schema::version(&tx)? != schema::CURRENT {
            return Err(Error::InvalidDatabase);
        }
        let projects = {
            let mut stmt = tx
                .prepare("SELECT id, name FROM projects WHERE deleted_at IS NULL ORDER BY name, id")
                .map_err(database_error)?;
            stmt.query_map([], |row| {
                Ok(ProjectEntry {
                    id: ProjectId(row.get(0)?),
                    name: row.get(1)?,
                })
            })
            .map_err(database_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(database_error)?
        };
        tx.commit().map_err(database_error)?;
        Ok(projects)
    }
}

#[cfg(test)]
mod tests;
