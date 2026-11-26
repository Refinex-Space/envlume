use crate::{Error, Store, connect, database_error, repository::read_snapshot, schema};
use envlume_application::{
    ErrorCode,
    ports::{BindingSnapshot, Repository},
    workspaces::{BindingInfo, ManagementError, WorkspaceRepository},
};
use envlume_domain::{BindingId, GrantId, ProjectId};
use envlume_workspace::{Workspace, discover};
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, ManagementError>;
const INFO: &str = "SELECT b.id,b.project_id,p.name,b.alias,coalesce(b.entry_path,b.canonical_path),
    b.canonical_path,b.trust_state,b.confirmation_source,b.confirmed_at_ms,b.git_fingerprint,b.root_identity
    FROM workspace_bindings b JOIN projects p ON p.id=b.project_id";

fn info(row: &Row<'_>) -> rusqlite::Result<BindingInfo> {
    Ok(BindingInfo {
        binding_id: BindingId(row.get(0)?),
        project_id: ProjectId(row.get(1)?),
        project: row.get(2)?,
        alias: row.get(3)?,
        entry_path: PathBuf::from(row.get::<_, String>(4)?),
        canonical_path: PathBuf::from(row.get::<_, String>(5)?),
        state: row.get(6)?,
        confirmation_source: row.get(7)?,
        confirmed_at_ms: row.get(8)?,
        fingerprint: row.get(9)?,
        root_identity: row.get(10)?,
    })
}
fn by_id(conn: &Connection, id: BindingId) -> Result<BindingInfo> {
    conn.query_row(&format!("{INFO} WHERE b.id=?1"), [id.0], info)
        .optional()
        .map_err(database_error)?
        .ok_or(ManagementError::NotFound)
}
fn by_path(conn: &Connection, entry: &Path, ws: Option<&Workspace>) -> Result<Option<BindingInfo>> {
    let sql = format!(
        "{INFO} WHERE b.entry_path=?1 OR b.canonical_path=?1
        OR b.canonical_path=?2 OR b.root_identity=?3
        ORDER BY (b.trust_state IN ('TRUSTED','UNTRUSTED')) DESC,
        (b.entry_path=?1) DESC,b.created_at DESC,b.rowid DESC LIMIT 1"
    );
    let parameters = params![
        entry.to_str(),
        ws.and_then(|w| w.canonical_path.to_str()),
        ws.map(|w| &w.root_identity)
    ];
    Ok(conn
        .query_row(&sql, parameters, info)
        .optional()
        .map_err(database_error)?)
}
fn still_valid(binding: &BindingInfo) -> bool {
    discover(&binding.entry_path).is_ok_and(|w| binding.matches(&w))
}
fn revoke_grants(conn: &Connection, id: BindingId) -> Result<()> {
    conn.execute(
        "UPDATE capability_grants SET revoked_at=CAST(unixepoch('subsec')*1000 AS INTEGER)
        WHERE binding_id=?1 AND revoked_at IS NULL",
        [id.0],
    )
    .map_err(database_error)?;
    Ok(())
}
fn stale(conn: &Connection, id: BindingId) -> Result<()> {
    let sql = "UPDATE workspace_bindings SET trust_state='STALE'
        WHERE id=?1 AND trust_state IN ('TRUSTED','UNTRUSTED')";
    if conn.execute(sql, [id.0]).map_err(database_error)? != 0 {
        revoke_grants(conn, id)?;
    }
    Ok(())
}
impl Store {
    pub(super) fn verified_workspace(
        tx: &Connection,
        binding: BindingId,
        origin: &Path,
    ) -> Result<BindingInfo> {
        let row = by_id(tx, binding)?;
        let observed = discover(origin).map_err(|_| ManagementError::WorkspaceChanged)?;
        if row.state != "TRUSTED" || !still_valid(&row) || !row.matches(&observed) {
            return Err(ManagementError::WorkspaceChanged);
        }
        Ok(row)
    }

    /// Replace one approved profile, select it, and grant this binding inspection atomically.
    pub fn configure_toolchain(
        &self,
        binding: BindingId,
        origin: &Path,
        name: &envlume_domain::Alias,
        rules: &[envlume_domain::ToolRule],
    ) -> Result<envlume_domain::ProfileId> {
        let encoded = serde_json::to_string(rules).map_err(|_| ManagementError::InvalidPath)?;
        if encoded.len() > 65536 {
            return Err(ManagementError::InvalidPath);
        }
        self.workspace_write(|tx| {
            let row = by_id(tx, binding)?;
            let observed = discover(origin).map_err(|_| ManagementError::WorkspaceChanged)?;
            if row.state != "TRUSTED" || !still_valid(&row) || !row.matches(&observed) {
                return Err(ManagementError::WorkspaceChanged);
            }
            let lookup = "SELECT id, rules_json FROM toolchain_profiles WHERE project_id=?1 AND name=?2";
            let existing = tx.query_row(lookup, params![row.project_id.0, name.as_str()], |r| {
                Ok((envlume_domain::ProfileId(r.get(0)?), r.get::<_, String>(1)?))
            }).optional().map_err(database_error)?;
            let id = if let Some((id, old)) = existing {
                if old != encoded {
                    let used = "SELECT 1 FROM workspace_bindings
                        WHERE profile_id=?1 AND id<>?2 AND trust_state='TRUSTED'";
                    if tx.prepare(used).map_err(database_error)?
                        .exists(params![id.0, binding.0]).map_err(database_error)? {
                        return Err(ManagementError::SharedProfile);
                    }
                    let update = "UPDATE toolchain_profiles SET rules_json=?2 WHERE id=?1";
                    tx.execute(update, params![id.0, encoded]).map_err(database_error)?;
                }
                id
            } else {
                let id = envlume_domain::ProfileId::generate();
                let insert = "INSERT INTO toolchain_profiles(id,project_id,name,rules_json,created_at)
                    VALUES(?1,?2,?3,?4,CAST(unixepoch('subsec')*1000 AS INTEGER))";
                tx.execute(insert, params![id.0, row.project_id.0, name.as_str(), encoded]).map_err(database_error)?;
                id
            };
            let select = "UPDATE workspace_bindings SET profile_id=?2 WHERE id=?1";
            tx.execute(select, params![binding.0, id.0]).map_err(database_error)?;
            let retire = "UPDATE capability_grants SET revoked_at=expires_at WHERE binding_id=?1
                AND capability='toolchain.inspect' AND revoked_at IS NULL AND expires_at<=unixepoch('subsec')*1000";
            tx.execute(retire, [binding.0]).map_err(database_error)?;
            let grant = "INSERT INTO capability_grants(id,project_id,binding_id,capability)
                SELECT ?1,?2,?3,'toolchain.inspect' WHERE NOT EXISTS(SELECT 1 FROM capability_grants
                WHERE binding_id=?3 AND capability='toolchain.inspect' AND revoked_at IS NULL)";
            tx.execute(grant, params![GrantId::generate().0, row.project_id.0, binding.0]).map_err(database_error)?;
            Ok(id)
        })
    }

    pub(super) fn workspace_write<T>(
        &self,
        op: impl FnOnce(&Connection) -> Result<T>,
    ) -> Result<T> {
        let mut conn = connect(&self.paths.database(), true)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database_error)?;
        if schema::version(&tx)? != schema::CURRENT {
            return Err(Error::InvalidDatabase.into());
        }
        let result = op(&tx)?;
        tx.commit().map_err(database_error)?;
        Ok(result)
    }
}

impl WorkspaceRepository for Store {
    fn register_confirmed(
        &self,
        ws: &Workspace,
        project: Option<ProjectId>,
    ) -> Result<BindingInfo> {
        self.workspace_write(|tx| {
            let observed =
                discover(&ws.entry_path).map_err(|_| ManagementError::WorkspaceChanged)?;
            if &observed != ws {
                return Err(ManagementError::WorkspaceChanged);
            }
            // A retargeted alias and its new destination can match two separate rows.
            while let Some(old) = by_path(tx, &ws.entry_path, Some(ws))? {
                if old.state != "TRUSTED" && old.state != "UNTRUSTED" {
                    break;
                }
                if old.state == "TRUSTED" && still_valid(&old) && old.matches(ws) {
                    if project.is_some_and(|id| id != old.project_id) {
                        return Err(ManagementError::ProjectMismatch);
                    }
                    return Ok(old);
                }
                stale(tx, old.binding_id)?;
            }
            let id = BindingId::generate();
            let timestamp = envlume_domain::UtcDateTime::now().unix_timestamp_nanos() / 1_000_000;
            let now = i64::try_from(timestamp).map_err(|_| Error::StorageUnavailable)?;
            let name = ws
                .canonical_path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or(ManagementError::InvalidPath)?;
            if name.chars().any(char::is_control)
                || name.trim().is_empty()
                || name.chars().count() > 128
            {
                return Err(ManagementError::InvalidPath);
            }
            let project_id = if let Some(selected) = project {
                let sql = "SELECT 1 FROM projects WHERE id=?1 AND deleted_at IS NULL";
                if !tx
                    .prepare(sql)
                    .map_err(database_error)?
                    .exists([selected.0])
                    .map_err(database_error)?
                {
                    return Err(ManagementError::NotFound);
                }
                selected
            } else {
                let new_id = ProjectId::generate();
                let sql = "INSERT INTO projects(id,name,created_at,updated_at) VALUES(?1,?2,?3,?3)";
                tx.execute(sql, params![new_id.0, name, now])
                    .map_err(database_error)?;
                new_id
            };
            let slug: String = name
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                .take(24)
                .map(|c| c.to_ascii_lowercase())
                .collect();
            let alias = format!("ws-{slug}-{}", id.0.simple());
            let insert = "INSERT INTO workspace_bindings(
                id, project_id, alias, canonical_path, git_fingerprint, trust_state, created_at,
                entry_path, root_identity, confirmation_source, confirmed_at_ms)
                VALUES(?1,?2,?3,?4,?5,'TRUSTED',?6,?7,?8,'LOCAL_CLI_TTY',?6)";
            tx.execute(
                insert,
                params![
                    id.0,
                    project_id.0,
                    alias,
                    ws.canonical_path.to_str(),
                    ws.fingerprint,
                    now,
                    ws.entry_path.to_str(),
                    ws.root_identity
                ],
            )
            .map_err(database_error)?;
            let grant = "INSERT INTO capability_grants(id,project_id,binding_id,capability)
                VALUES(?1,?2,?3,'project.inspect')";
            tx.execute(grant, params![GrantId::generate().0, project_id.0, id.0])
                .map_err(database_error)?;
            by_id(tx, id)
        })
    }
    fn inspect_binding(&self, entry: &Path, ws: Option<&Workspace>) -> Result<BindingInfo> {
        self.workspace_write(|tx| {
            let old = by_path(tx, entry, ws)?.ok_or(ManagementError::NotFound)?;
            if old.state == "TRUSTED" && !still_valid(&old) {
                stale(tx, old.binding_id)?;
                return by_id(tx, old.binding_id);
            }
            Ok(old)
        })
    }
    fn revoke(&self, id: BindingId) -> Result<()> {
        self.workspace_write(|tx| {
            by_id(tx,id)?;
            let sql="UPDATE workspace_bindings SET trust_state='REVOKED',
                revoked_at=coalesce(revoked_at, max(created_at,CAST(unixepoch('subsec')*1000 AS INTEGER)))
                WHERE id=?1";
            tx.execute(sql,[id.0]).map_err(database_error)?;
            revoke_grants(tx,id)
        })
    }
    fn delete_project(&self, id: ProjectId) -> Result<()> {
        self.workspace_write(|tx| {
            let now=envlume_domain::UtcDateTime::now().unix_timestamp_nanos()/1_000_000;
            let now=i64::try_from(now).map_err(|_|Error::StorageUnavailable)?;
            let delete="UPDATE projects SET
                updated_at=CASE WHEN deleted_at IS NULL THEN max(updated_at,?2) ELSE updated_at END,
                deleted_at=coalesce(deleted_at,max(updated_at,?2)) WHERE id=?1";
            if tx.execute(delete,params![id.0,now]).map_err(database_error)?==0 {
                return Err(ManagementError::NotFound);
            }
            let revoke="UPDATE workspace_bindings SET trust_state='REVOKED',
                revoked_at=coalesce(revoked_at,max(created_at,?2)) WHERE project_id=?1";
            tx.execute(revoke,params![id.0,now]).map_err(database_error)?;
            let grants="UPDATE capability_grants SET revoked_at=?2 WHERE project_id=?1 AND revoked_at IS NULL";
            tx.execute(grants,params![id.0,now]).map_err(database_error)?;
            Ok(())
        })
    }
}

/// Production MCP repository. Origin comes from the local launch directory;
/// protocol parameters never select a store, binding or path.
pub struct LocalRegistration {
    pub store: Store,
    pub origin: PathBuf,
}
impl LocalRegistration {
    fn checked(&self, id: BindingId) -> Result<Option<BindingSnapshot>> {
        let mut conn = connect(&self.store.paths.database(), false)?;
        let tx = conn.transaction().map_err(database_error)?;
        if schema::version(&tx)? != schema::CURRENT {
            return Err(Error::InvalidDatabase.into());
        }
        let Some(state) = read_snapshot(&tx, id).map_err(database_error)? else {
            return Ok(None);
        };
        if !matches!(state.binding.state, envlume_domain::BindingState::Trusted)
            || state.project.deleted_at.is_some()
        {
            return Ok(Some(state));
        }
        let binding = by_id(&tx, id)?;
        if !still_valid(&binding) {
            drop(tx);
            self.store.workspace_write(|tx| stale(tx, id))?;
            return Err(ManagementError::WorkspaceChanged);
        }
        let origin = discover(&self.origin).map_err(|_| ManagementError::ConfirmationRequired)?;
        if !binding.matches(&origin) {
            return Err(ManagementError::ConfirmationRequired);
        }
        Ok(Some(state))
    }
}
impl Repository for LocalRegistration {
    fn load_binding(
        &self,
        id: BindingId,
    ) -> std::result::Result<Option<BindingSnapshot>, ErrorCode> {
        self.checked(id).map_err(|e| match e {
            ManagementError::Storage(Error::StorageBusy) => ErrorCode::Timeout,
            ManagementError::Storage(_) => ErrorCode::InternalError,
            _ => ErrorCode::ProjectNotTrusted,
        })
    }
}
