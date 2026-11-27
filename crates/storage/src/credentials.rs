use crate::{Error, Store, connect, database_error, paths, repository::read_snapshot, schema};
use envlume_application::{
    credentials::{Catalogue, Fault, ItemKey, Selection},
    workspaces::ManagementError,
};
use envlume_domain::ProjectId;
use rusqlite::{Connection, TransactionBehavior, params};
use std::{
    fs::{File, OpenOptions, TryLockError},
    io,
};

fn storage(e: Error) -> Fault {
    if e == Error::StorageBusy {
        Fault::Busy
    } else {
        Fault::Storage
    }
}
fn check(tx: &Connection, s: &Selection) -> Result<(), Fault> {
    Store::verified_workspace(tx, s.binding(), s.origin()).map_err(|e| match e {
        ManagementError::Storage(e) => storage(e),
        _ => Fault::Untrusted,
    })?;
    let state = read_snapshot(tx, s.binding())
        .map_err(|e| storage(database_error(e)))?
        .ok_or(Fault::Untrusted)?;
    s.recheck(&state)
}
impl Store {
    fn credential_tx<T>(
        &self,
        write: bool,
        f: impl FnOnce(&Connection) -> Result<T, Fault>,
    ) -> Result<T, Fault> {
        let mut c = connect(&self.paths.database(), write).map_err(storage)?;
        let tx = c
            .transaction_with_behavior(if write {
                TransactionBehavior::Immediate
            } else {
                TransactionBehavior::Deferred
            })
            .map_err(|e| storage(database_error(e)))?;
        if schema::version(&tx).map_err(storage)? != schema::CURRENT {
            return Err(Fault::Storage);
        }
        let result = f(&tx)?;
        tx.commit().map_err(|e| storage(database_error(e)))?;
        Ok(result)
    }
}
impl Catalogue for Store {
    type Lock = File;
    fn lock_credentials(&self) -> Result<File, Fault> {
        let path = self.paths.root().join("data/credentials.lock");
        paths::check_file(&path).map_err(storage)?;
        let file = match paths::create_file(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(|_| Fault::Storage)?,
            Err(_) => return Err(Fault::Storage),
        };
        file.try_lock().map_err(|e| match e {
            TryLockError::WouldBlock => Fault::Busy,
            _ => Fault::Storage,
        })?;
        Ok(file)
    }
    fn verified_project(
        &self,
        binding: envlume_domain::BindingId,
        origin: &std::path::Path,
    ) -> Result<ProjectId, Fault> {
        self.credential_tx(false, |tx| {
            let row = Store::verified_workspace(tx, binding, origin).map_err(|e| match e {
                ManagementError::Storage(e) => storage(e),
                _ => Fault::Untrusted,
            })?;
            let state = read_snapshot(tx, binding)
                .map_err(|e| storage(database_error(e)))?
                .ok_or(Fault::Untrusted)?;
            if state.project.deleted_at.is_some() {
                return Err(Fault::Untrusted);
            }
            Ok(row.project_id)
        })
    }
    fn validate(&self, s: &Selection) -> Result<(), Fault> {
        self.credential_tx(false, |tx| check(tx, s))
    }
    fn stage(&self, s: &Selection, key: &ItemKey) -> Result<(), Fault> {
        self.credential_tx(true, |tx| {
            check(tx, s)?;
            tx.execute(
                "INSERT INTO credential_cleanup(reference,project_id) VALUES(?1,?2)",
                params![key.reference(), s.project().0],
            )
            .map_err(|e| storage(database_error(e)))?;
            Ok(())
        })
    }
    fn publish(
        &self,
        s: &Selection,
        new: Option<&ItemKey>,
        remove_resource: bool,
    ) -> Result<(), Fault> {
        self.credential_tx(true,|tx|{
            check(tx,s)?;
            if let Some(key)=new {
                let affected=tx.execute("DELETE FROM credential_cleanup WHERE reference=?1 AND project_id=?2",params![key.reference(),s.project().0]).map_err(|e|storage(database_error(e)))?;
                if affected!=1{return Err(Fault::Changed);}
            }
            let updated=tx.execute("UPDATE resource_bindings SET credential_ref=?1,status=CASE WHEN ?2 THEN 'DISABLED' ELSE status END WHERE id=?3 AND target_id=?4",
                params![new.map(ItemKey::reference),remove_resource,s.resource().id.0,s.target().id.0]).map_err(|e|storage(database_error(e)))?;
            if updated!=1{return Err(Fault::Changed);}
            Ok(())
        })
    }
    fn garbage(&self, project: ProjectId) -> Result<Vec<String>, Fault> {
        self.credential_tx(false,|tx|{
            let mut stmt=tx.prepare("SELECT reference FROM credential_cleanup WHERE project_id=?1 AND NOT EXISTS
                (SELECT 1 FROM resource_bindings WHERE credential_ref=reference) ORDER BY reference").map_err(|e|storage(database_error(e)))?;
            stmt.query_map([project.0],|row|row.get(0)).and_then(Iterator::collect).map_err(|e|storage(database_error(e)))
        })
    }
    fn collected(&self, project: ProjectId, reference: &str) -> Result<(), Fault> {
        self.credential_tx(true, |tx| {
            tx.execute(
                "DELETE FROM credential_cleanup WHERE project_id=?1 AND reference=?2",
                params![project.0, reference],
            )
            .map_err(|e| storage(database_error(e)))?;
            Ok(())
        })
    }
}
