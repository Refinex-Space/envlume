//! Load all policy inputs in one SQLite read transaction, scoped to the registered binding.
use crate::{Store, connect, database_error, schema};
use envlume_application::{
    ErrorCode,
    ports::{BindingSnapshot, Repository},
};
use envlume_domain::*;
use rusqlite::{Connection, Row, params};
use std::{path::PathBuf, time::Duration};

type Decode<T> = rusqlite::Result<T>;
fn invalid() -> rusqlite::Error {
    rusqlite::Error::InvalidQuery
}
fn utc(ms: i64) -> Decode<UtcDateTime> {
    UtcDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000).map_err(|_| invalid())
}
fn alias(row: &Row<'_>, index: usize) -> Decode<Alias> {
    row.get::<_, String>(index)?
        .try_into()
        .map_err(|_| invalid())
}
fn status(row: &Row<'_>, index: usize) -> Decode<Status> {
    match row.get::<_, String>(index)?.as_str() {
        "ACTIVE" => Ok(Status::Active),
        "DISABLED" => Ok(Status::Disabled),
        _ => Err(invalid()),
    }
}

pub(super) fn read_snapshot(conn: &Connection, id: BindingId) -> Decode<Option<BindingSnapshot>> {
    use rusqlite::OptionalExtension;
    let sql = "SELECT p.id,p.name,p.description,p.created_at,p.updated_at,p.deleted_at,b.alias,b.canonical_path,b.git_fingerprint,b.profile_id,b.trust_state,b.revoked_at
         FROM workspace_bindings b JOIN projects p ON p.id=b.project_id WHERE b.id=?1";
    let main = conn
        .query_row(sql, [id.0], |row| {
            let project_id = ProjectId(row.get(0)?);
            let project = Project {
                id: project_id,
                name: row.get(1)?,
                description: row.get(2)?,
                created_at: utc(row.get(3)?)?,
                updated_at: utc(row.get(4)?)?,
                deleted_at: row.get::<_, Option<i64>>(5)?.map(utc).transpose()?,
            };
            let state = match row.get::<_, String>(10)?.as_str() {
                "TRUSTED" => BindingState::Trusted,
                "STALE" => BindingState::Stale,
                "UNTRUSTED" => BindingState::Untrusted,
                "REVOKED" => BindingState::Revoked(utc(row.get(11)?)?),
                _ => return Err(invalid()),
            };
            Ok((
                project,
                WorkspaceBinding {
                    id,
                    project_id,
                    alias: alias(row, 6)?,
                    canonical_path: PathBuf::from(row.get::<_, String>(7)?),
                    git_fingerprint: row.get(8)?,
                    profile_id: row.get::<_, Option<_>>(9)?.map(ProfileId),
                    state,
                },
            ))
        })
        .optional()?;
    let Some((project, binding)) = main else {
        return Ok(None);
    };
    let project_id = project.id;
    let profiles = conn
        .prepare(
            "SELECT id,name,rules_json FROM toolchain_profiles WHERE project_id=?1 ORDER BY id",
        )?
        .query_map([project_id.0], |r| {
            Ok(ToolchainProfile {
                id: ProfileId(r.get(0)?),
                project_id,
                name: alias(r, 1)?,
                rules: serde_json::from_str(&r.get::<_, String>(2)?).map_err(|_| invalid())?,
            })
        })?
        .collect::<Decode<Vec<_>>>()?;
    let sql = "SELECT id,alias,kind,risk_level,is_default,status FROM environment_targets WHERE project_id=?1 ORDER BY id";
    let targets = conn
        .prepare(sql)?
        .query_map([project_id.0], |r| {
            Ok(EnvironmentTarget {
                id: TargetId(r.get(0)?),
                project_id,
                alias: alias(r, 1)?,
                kind: match r.get::<_, String>(2)?.as_str() {
                    "DEV" => EnvironmentKind::Dev,
                    "TEST" => EnvironmentKind::Test,
                    "STAGING" => EnvironmentKind::Staging,
                    "PROD" => EnvironmentKind::Prod,
                    s => EnvironmentKind::Custom(s.to_owned()),
                },
                risk: match r.get::<_, String>(3)?.as_str() {
                    "LOW" => RiskLevel::Low,
                    "MEDIUM" => RiskLevel::Medium,
                    "HIGH" => RiskLevel::High,
                    "CRITICAL" => RiskLevel::Critical,
                    _ => return Err(invalid()),
                },
                is_default: r.get(4)?,
                status: status(r, 5)?,
            })
        })?
        .collect::<Decode<Vec<_>>>()?;
    let sql = "SELECT r.id,r.target_id,r.alias,r.kind,r.host,r.port,r.tls_mode,r.credential_ref,r.policy_labels,r.status,r.database_name,r.confirmed_at_ms IS NOT NULL,r.mysql_route,r.mysql_tables
        FROM resource_bindings r JOIN environment_targets t ON t.id=r.target_id WHERE t.project_id=?1 ORDER BY r.id";
    let resources = conn
        .prepare(sql)?
        .query_map([project_id.0], |r| {
            Ok(ResourceBinding {
                id: ResourceId(r.get(0)?),
                target_id: TargetId(r.get(1)?),
                alias: alias(r, 2)?,
                kind: match r.get::<_, String>(3)?.as_str() {
                    "MYSQL" => ResourceKind::MySql,
                    _ => return Err(invalid()),
                },
                host: r.get(4)?,
                port: r.get(5)?,
                tls: match r.get::<_, String>(6)?.as_str() {
                    "VERIFY_IDENTITY" => TlsMode::VerifyIdentity,
                    "DISABLED" => TlsMode::Disabled,
                    _ => return Err(invalid()),
                },
                credential_ref: r.get::<_, Option<_>>(7)?.map(CredentialRef),
                policy_labels: serde_json::from_str(&r.get::<_, String>(8)?)
                    .map_err(|_| invalid())?,
                status: status(r, 9)?,
                database: r.get(10)?,
                endpoint_confirmed: r.get(11)?,
                route: r
                    .get::<_, Option<String>>(12)?
                    .map(|s| serde_json::from_str(&s))
                    .transpose()
                    .map_err(|_| invalid())?,
                tables: serde_json::from_str(&r.get::<_, String>(13)?).map_err(|_| invalid())?,
            })
        })?
        .collect::<Decode<Vec<_>>>()?;
    let sql = "SELECT id,binding_id,task_key,executable,arguments,workspace_relative_dir,coalesce((SELECT g.timeout_seconds FROM capability_grants g WHERE g.task_id=task_definitions.id AND g.binding_id=?2 AND g.capability='task.run' AND g.revoked_at IS NULL),timeout_seconds),input,enabled FROM task_definitions
        WHERE project_id=?1 AND (binding_id IS NULL OR binding_id=?2) ORDER BY id";
    let tasks = conn
        .prepare(sql)?
        .query_map(params![project_id.0, id.0], |r| {
            Ok(TaskDefinition {
                id: TaskId(r.get(0)?),
                project_id,
                binding_id: r.get::<_, Option<_>>(1)?.map(BindingId),
                key: alias(r, 2)?,
                executable: r.get(3)?,
                arguments: serde_json::from_str(&r.get::<_, String>(4)?).map_err(|_| invalid())?,
                workspace_relative_dir: PathBuf::from(r.get::<_, String>(5)?),
                timeout: Duration::from_secs(
                    r.get::<_, i64>(6)?.try_into().map_err(|_| invalid())?,
                ),
                input: match r.get::<_, String>(7)?.as_str() {
                    "CLOSED" => TaskInput::Closed,
                    _ => return Err(invalid()),
                },
                enabled: r.get(8)?,
            })
        })?
        .collect::<Decode<Vec<_>>>()?;
    let sql = "SELECT id,capability,target_id,resource_id,task_id,expires_at,revoked_at FROM capability_grants WHERE binding_id=?1 AND project_id=?2 ORDER BY id";
    let grants = conn
        .prepare(sql)?
        .query_map(params![id.0, project_id.0], |r| {
            let scope = || -> Decode<ResourceScope> {
                Ok(ResourceScope {
                    target: TargetId(r.get(2)?),
                    resource: ResourceId(r.get(3)?),
                })
            };
            let capability = match r.get::<_, String>(1)?.as_str() {
                "project.inspect" => Capability::ProjectInspect,
                "environment.inspect" => Capability::EnvironmentInspect,
                "task.inspect" => Capability::TaskInspect,
                "toolchain.inspect" => Capability::ToolchainInspect,
                "task.run" => Capability::TaskRun(TaskId(r.get(4)?)),
                "resource.connect" => Capability::ResourceConnect(scope()?),
                "database.schema" => Capability::DatabaseSchema(scope()?),
                "database.read" => Capability::DatabaseRead(scope()?),
                _ => return Err(invalid()),
            };
            Ok(CapabilityGrant {
                id: GrantId(r.get(0)?),
                binding_id: id,
                capability,
                expires_at: r.get::<_, Option<i64>>(5)?.map(utc).transpose()?,
                revoked_at: r.get::<_, Option<i64>>(6)?.map(utc).transpose()?,
            })
        })?
        .collect::<Decode<Vec<_>>>()?;
    Ok(Some(BindingSnapshot {
        project,
        binding,
        profiles,
        targets,
        resources,
        tasks,
        grants,
    }))
}

impl Store {
    pub fn binding_snapshot(&self, id: BindingId) -> crate::Result<Option<BindingSnapshot>> {
        let mut conn = connect(&self.paths.database(), false)?;
        let tx = conn.transaction().map_err(database_error)?;
        if schema::version(&tx)? != schema::CURRENT {
            return Err(crate::Error::InvalidDatabase);
        }
        let result = read_snapshot(&tx, id).map_err(database_error)?;
        tx.commit().map_err(database_error)?;
        Ok(result)
    }
}

impl Repository for Store {
    fn load_binding(&self, id: BindingId) -> Result<Option<BindingSnapshot>, ErrorCode> {
        self.binding_snapshot(id).map_err(|err| match err {
            crate::Error::StorageBusy => ErrorCode::Timeout,
            _ => ErrorCode::InternalError,
        })
    }
}
