use crate::{Error, Store, database_error, repository::read_snapshot};
use envlume_application::{
    ErrorCode,
    environments::{ConfirmedEdit, Edit, EnvironmentWriter},
    workspaces::ManagementError,
};
use envlume_domain::*;
use rusqlite::params;
use std::path::Path;

impl EnvironmentWriter for Store {
    fn apply(
        &self,
        binding: BindingId,
        origin: &Path,
        approved: ConfirmedEdit,
    ) -> Result<(), ErrorCode> {
        self.workspace_write(|tx| persist(tx, binding, origin, &approved))
            .map_err(|e| match e {
                ManagementError::Storage(Error::StorageBusy) => ErrorCode::Timeout,
                ManagementError::Storage(_) => ErrorCode::InternalError,
                ManagementError::WorkspaceChanged | ManagementError::NotFound => {
                    ErrorCode::ProjectNotTrusted
                }
                _ => ErrorCode::PolicyDenied,
            })
    }
}

fn persist(
    tx: &rusqlite::Connection,
    binding: BindingId,
    origin: &Path,
    approved: &ConfirmedEdit,
) -> Result<(), ManagementError> {
    let project = Store::verified_workspace(tx, binding, origin)?.project_id;
    let current = read_snapshot(tx, binding)
        .map_err(database_error)?
        .ok_or(ManagementError::NotFound)?;
    approved
        .recheck(&current)
        .map_err(|_| ManagementError::ConfirmationRequired)?;
    let target_id = || {
        approved
            .target()
            .map(|t| t.id.0)
            .ok_or(ManagementError::NotFound)
    };
    match approved.edit() {
        Edit::Target { alias, kind, risk } => {
            let kind = match kind {
                EnvironmentKind::Dev => "DEV",
                EnvironmentKind::Test => "TEST",
                EnvironmentKind::Staging => "STAGING",
                EnvironmentKind::Prod => "PROD",
                EnvironmentKind::Custom(_) => return Err(ManagementError::ConfirmationRequired),
            };
            let risk = match risk {
                RiskLevel::Low => "LOW",
                RiskLevel::Medium => "MEDIUM",
                RiskLevel::High => "HIGH",
                RiskLevel::Critical => "CRITICAL",
            };
            let sql = "INSERT INTO environment_targets(id,project_id,alias,kind,risk_level,is_default,status)
                        VALUES(?1,?2,?3,?4,?5,0,'ACTIVE') ON CONFLICT(project_id,alias) DO UPDATE SET
                        kind=excluded.kind,risk_level=excluded.risk_level,
                        is_default=CASE WHEN excluded.kind='PROD' OR excluded.risk_level='CRITICAL' THEN 0 ELSE is_default END";
            tx.execute(
                sql,
                params![
                    TargetId::generate().0,
                    project.0,
                    alias.as_str(),
                    kind,
                    risk
                ],
            )
            .map_err(database_error)?;
        }
        Edit::Rename { to, .. } => {
            tx.execute(
                "UPDATE environment_targets SET alias=?2 WHERE id=?1",
                params![target_id()?, to.as_str()],
            )
            .map_err(database_error)?;
        }
        Edit::Resource {
            alias, endpoint, ..
        } => {
            let id = approved
                .resource()
                .map(|r| r.id)
                .unwrap_or_else(ResourceId::generate);
            let tls = match endpoint.tls {
                TlsMode::VerifyIdentity => "VERIFY_IDENTITY",
                TlsMode::Disabled => "DISABLED",
            };
            let sql = "INSERT INTO resource_bindings(id,target_id,alias,kind,host,port,tls_mode,database_name,policy_labels,status)
                        VALUES(?1,?2,?3,'MYSQL',?4,?5,?6,?7,'[]','ACTIVE') ON CONFLICT(id) DO UPDATE SET
                        host=excluded.host,port=excluded.port,tls_mode=excluded.tls_mode,database_name=excluded.database_name";
            tx.execute(
                sql,
                params![
                    id.0,
                    target_id()?,
                    alias.as_str(),
                    endpoint.host,
                    endpoint.port,
                    tls,
                    endpoint.database
                ],
            )
            .map_err(database_error)?;
            // Endpoint-change triggers have already retired old grants and detached the reference.
            tx.execute("UPDATE resource_bindings SET confirmed_at_ms=CAST(unixepoch('subsec')*1000 AS INTEGER) WHERE id=?1",[id.0]).map_err(database_error)?;
        }
        Edit::MySqlPolicy { route, tables, .. } => {
            let r = approved.resource().ok_or(ManagementError::NotFound)?;
            let route = serde_json::to_string(route).map_err(|_| Error::InvalidDatabase)?;
            let tables = serde_json::to_string(tables).map_err(|_| Error::InvalidDatabase)?;
            tx.execute(
                "UPDATE resource_bindings SET mysql_route=?2,mysql_tables=?3 WHERE id=?1",
                params![r.id.0, route, tables],
            )
            .map_err(database_error)?;
        }
        Edit::Grant { action, .. } | Edit::Revoke { action, .. } => {
            let target = target_id()?;
            let resource = approved.resource().ok_or(ManagementError::NotFound)?.id.0;
            let retire = "UPDATE capability_grants SET revoked_at=CAST(unixepoch('subsec')*1000 AS INTEGER)
                        WHERE binding_id=?1 AND target_id=?2 AND resource_id=?3 AND capability=?4 AND revoked_at IS NULL";
            tx.execute(retire, params![binding.0, target, resource, action.name()])
                .map_err(database_error)?;
            if let Edit::Grant { expires_at, .. } = approved.edit() {
                let grant = "INSERT INTO capability_grants(id,project_id,binding_id,target_id,resource_id,capability,expires_at)
                            VALUES(?1,?2,?3,?4,?5,?6,?7)";
                let end = expires_at.map(|t| (t.unix_timestamp_nanos() / 1_000_000) as i64);
                tx.execute(
                    grant,
                    params![
                        GrantId::generate().0,
                        project.0,
                        binding.0,
                        target,
                        resource,
                        action.name(),
                        end
                    ],
                )
                .map_err(database_error)?;
                let expired = "UPDATE capability_grants SET revoked_at=expires_at WHERE binding_id=?1 AND capability='environment.inspect'
                            AND revoked_at IS NULL AND expires_at<=unixepoch('subsec')*1000";
                tx.execute(expired, [binding.0]).map_err(database_error)?;
                // A later explicit approval can extend inspection to cover that grant.
                let extend = "UPDATE capability_grants SET expires_at=?2
                    WHERE binding_id=?1 AND capability='environment.inspect' AND revoked_at IS NULL
                      AND expires_at IS NOT NULL AND (?2 IS NULL OR expires_at < ?2)";
                tx.execute(extend, params![binding.0, end])
                    .map_err(database_error)?;
                let inspect = "INSERT INTO capability_grants(id,project_id,binding_id,capability,expires_at)
                    SELECT ?1,?2,?3,'environment.inspect',?4 WHERE NOT EXISTS(
                        SELECT 1 FROM capability_grants WHERE binding_id=?3 AND capability='environment.inspect' AND revoked_at IS NULL)";
                tx.execute(
                    inspect,
                    params![GrantId::generate().0, project.0, binding.0, end],
                )
                .map_err(database_error)?;
            }
        }
    }
    Ok(())
}
