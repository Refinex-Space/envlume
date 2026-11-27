use crate::{Store, StorePaths, database_error, paths};
use envlume_application::{
    integrations::{Fault, Registration, client_key},
    workspaces::{BindingInfo, ManagementError},
};
use envlume_domain::{BindingId, ClientKind, IntegrationId};
use rusqlite::{OptionalExtension, params};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

fn fault(e: ManagementError) -> Fault {
    match e {
        ManagementError::Storage(crate::Error::StorageBusy) => Fault::Busy,
        ManagementError::Storage(_) => Fault::Storage,
        _ => Fault::UntrustedBinding,
    }
}
impl StorePaths {
    pub fn lock_integrations(&self) -> Result<File, Fault> {
        self.prepare().map_err(|_| Fault::Storage)?;
        let lock = self.root().join("data").join("integrations.lock");
        paths::check_file(&lock).map_err(|_| Fault::UnsafePath)?;
        let file = match paths::create_file(&lock) {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                fs::OpenOptions::new().read(true).write(true).open(&lock)
            }
            r => r,
        }
        .map_err(|_| Fault::Storage)?;
        file.try_lock().map_err(|e| match e {
            std::fs::TryLockError::WouldBlock => Fault::Busy,
            _ => Fault::Storage,
        })?;
        Ok(file)
    }
    fn config_backup(&self, id: IntegrationId) -> PathBuf {
        self.backups().join(format!("integration-{}.bak", id.0))
    }
    pub fn save_config_backup(&self, id: IntegrationId, bytes: &[u8]) -> Result<(), Fault> {
        let mut f = paths::create_file(&self.config_backup(id)).map_err(|_| Fault::Storage)?;
        f.write_all(bytes)
            .and_then(|_| f.sync_all())
            .map_err(|_| Fault::Storage)?;
        #[cfg(unix)]
        File::open(self.backups())
            .and_then(|d| d.sync_all())
            .map_err(|_| Fault::Storage)?;
        Ok(())
    }
    /// Keep a private snapshot of the exact source of an uninstall or compensating write.
    pub fn save_change_backup(&self, id: IntegrationId, bytes: &[u8]) -> Result<(), Fault> {
        use sha2::{Digest, Sha256};
        let digest = format!("{:x}", Sha256::digest(bytes));
        let path = self
            .backups()
            .join(format!("integration-{}-{digest}.bak", id.0));
        paths::check_file(&path).map_err(|_| Fault::UnsafePath)?;
        let mut temp =
            tempfile::NamedTempFile::new_in(self.backups()).map_err(|_| Fault::Storage)?;
        temp.write_all(bytes)
            .and_then(|_| temp.as_file().sync_all())
            .map_err(|_| Fault::Storage)?;
        match temp.persist_noclobber(&path) {
            Ok(_) => (),
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {
                let mut old = Vec::new();
                File::open(path)
                    .and_then(|f| f.take(bytes.len() as u64 + 1).read_to_end(&mut old))
                    .map_err(|_| Fault::Storage)?;
                if old != bytes {
                    return Err(Fault::RecoveryConflict);
                }
            }
            Err(_) => return Err(Fault::Storage),
        }
        #[cfg(unix)]
        File::open(self.backups())
            .and_then(|f| f.sync_all())
            .map_err(|_| Fault::Storage)?;
        Ok(())
    }
    pub fn load_config_backup(&self, id: IntegrationId, limit: usize) -> Result<Vec<u8>, Fault> {
        let path = self.config_backup(id);
        paths::check_file(&path).map_err(|_| Fault::UnsafePath)?;
        let mut bytes = Vec::new();
        File::open(path)
            .and_then(|f| f.take(limit as u64 + 1).read_to_end(&mut bytes))
            .map_err(|_| Fault::Storage)?;
        if bytes.len() > limit {
            return Err(Fault::RecoveryConflict);
        }
        Ok(bytes)
    }
}
impl Store {
    pub fn integration_binding(
        &self,
        binding: BindingId,
        origin: &Path,
    ) -> Result<BindingInfo, Fault> {
        self.workspace_write(|tx| Self::verified_workspace(tx, binding, origin))
            .map_err(fault)
    }
    /// Keep publication ordered with trust changes. The durable intent already exists;
    /// no verifier or user prompt runs inside this SQLite transaction.
    pub fn with_integration_binding<T>(
        &self,
        binding: BindingId,
        origin: &Path,
        change: impl FnOnce() -> Result<T, Fault>,
    ) -> Result<T, Fault> {
        self.workspace_write(|tx| {
            Self::verified_workspace(tx, binding, origin)?;
            Ok(change())
        })
        .map_err(fault)?
    }
    pub fn integration_record(
        &self,
        binding: BindingId,
        origin: &Path,
        client: ClientKind,
    ) -> Result<Option<Registration>, Fault> {
        let query="SELECT id,config_location,scope,config_fingerprint,status,original_hash,published_hash,pending,removal_hash,verification,source_hash
                   FROM client_integrations WHERE binding_id=? AND client_kind=? AND status!='REMOVED'";
        self.workspace_write(|tx| {
            Self::verified_workspace(tx, binding, origin)?;
            tx.query_row(query, params![binding.0, client_key(client)], |r| {
                Ok(Registration {
                    id: IntegrationId(r.get(0)?),
                    binding,
                    client,
                    location: PathBuf::from(r.get::<_, String>(1)?),
                    scope: r.get(2)?,
                    fingerprint: r.get(3)?,
                    status: r.get(4)?,
                    original: r.get(5)?,
                    published: r.get(6)?,
                    pending: r.get(7)?,
                    removal: r.get(8)?,
                    verification: r.get(9)?,
                    source: r.get(10)?,
                })
            })
            .optional()
            .map_err(|e| database_error(e).into())
        })
        .map_err(fault)
    }
    pub fn put_integration(&self, origin: &Path, r: &Registration) -> Result<(), Fault> {
        let upsert = "INSERT INTO client_integrations (id,binding_id,client_kind,config_location,scope,config_fingerprint,status,original_hash,published_hash,pending,removal_hash,verification,source_hash)
            VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)
            ON CONFLICT(binding_id,client_kind) DO UPDATE SET id=excluded.id,config_location=excluded.config_location,scope=excluded.scope,config_fingerprint=excluded.config_fingerprint,status=excluded.status,original_hash=excluded.original_hash,published_hash=excluded.published_hash,pending=excluded.pending,removal_hash=excluded.removal_hash,verification=excluded.verification,source_hash=excluded.source_hash";
        self.workspace_write(|tx| {
            Self::verified_workspace(tx, r.binding, origin)?;
            tx.execute(
                upsert,
                params![
                    r.id.0,
                    r.binding.0,
                    client_key(r.client),
                    r.location.to_str().ok_or(ManagementError::InvalidPath)?,
                    r.scope,
                    r.fingerprint,
                    r.status,
                    r.original,
                    r.published,
                    r.pending,
                    r.removal,
                    r.verification,
                    r.source
                ],
            )
            .map_err(database_error)?;
            Ok(())
        })
        .map_err(fault)
    }
}
