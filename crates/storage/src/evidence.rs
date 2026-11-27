use crate::{Store, database_error};
use envlume_application::workspaces::ManagementError;
use envlume_audit::{Entry, RETENTION_DAYS};
use envlume_domain::{Actor, BindingId, ClientKind, EvidenceId, ExecutionId, ProjectId, RequestId};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::path::Path;

pub struct EvidenceScope {
    binding: BindingId,
    project: ProjectId,
    alias: String,
}
pub enum RecordKey {
    Request(RequestId),
    Evidence(EvidenceId),
}

const EXPIRES: i64 = RETENTION_DAYS * 86_400_000;
fn purge_expired(tx: &rusqlite::Connection) -> Result<(), ManagementError> {
    tx.execute(
        "DELETE FROM execution_records WHERE created_at <= unixepoch('subsec')*1000-?1",
        [EXPIRES],
    )
    .map_err(database_error)?;
    Ok(())
}
impl Store {
    fn authorize_history(
        tx: &rusqlite::Connection,
        binding: BindingId,
        origin: &Path,
    ) -> Result<(), ManagementError> {
        let workspace = Self::verified_workspace(tx, binding, origin)?;
        let allowed: bool = tx
            .query_row(
                "SELECT EXISTS (
                SELECT 1 FROM capability_grants g JOIN projects p ON p.id=g.project_id
                WHERE g.binding_id=?1 AND g.project_id=?2 AND p.deleted_at IS NULL
                  AND g.capability='project.inspect' AND g.revoked_at IS NULL
                  AND (g.expires_at IS NULL OR g.expires_at > unixepoch('subsec')*1000)
            )",
                params![binding.0, workspace.project_id.0],
                |row| row.get(0),
            )
            .map_err(database_error)?;
        if !allowed {
            return Err(ManagementError::ConfirmationRequired);
        }
        Ok(())
    }

    pub fn evidence_scope(
        &self,
        binding: BindingId,
        origin: &Path,
    ) -> Result<EvidenceScope, ManagementError> {
        self.workspace_write(|tx| {
            let row = Self::verified_workspace(tx, binding, origin)?;
            Ok(EvidenceScope {
                binding,
                project: row.project_id,
                alias: row.alias,
            })
        })
    }
    /// The scope was verified at admission. Completion still belongs to it if a
    /// concurrent revocation cancelled the task. Reads recheck trust independently.
    pub fn record(
        &self,
        scope: &EvidenceScope,
        actor: Actor,
        entry: Entry<'_>,
    ) -> Result<(), ManagementError> {
        self.write_record(scope, actor, ExecutionId::generate(), entry, false)
    }
    pub fn begin_execution(
        &self,
        scope: &EvidenceScope,
        actor: Actor,
        entry: Entry<'_>,
    ) -> Result<ExecutionId, ManagementError> {
        let id = ExecutionId::generate();
        self.write_record(scope, actor, id, entry, true)?;
        Ok(id)
    }
    pub fn finish_execution(
        &self,
        scope: &EvidenceScope,
        actor: Actor,
        id: ExecutionId,
        entry: Entry<'_>,
    ) -> Result<(), ManagementError> {
        self.write_record(scope, actor, id, entry, false)
    }
    fn write_record(
        &self,
        scope: &EvidenceScope,
        actor: Actor,
        id: ExecutionId,
        entry: Entry<'_>,
        pending: bool,
    ) -> Result<(), ManagementError> {
        let actor = match actor {
            Actor::Cli => "CLI",
            Actor::LocalStdio => "LOCAL_STDIO",
            Actor::Desktop => "DESKTOP",
            Actor::Mcp(ClientKind::ClaudeCode) => "CLAUDE_CODE",
            Actor::Mcp(ClientKind::Codex) => "CODEX",
            Actor::Mcp(ClientKind::Cursor) => "CURSOR",
        };
        let report = entry.report;
        let mut details = entry.details();
        details["workspace"] = json!(scope.alias);
        let (decision, summary) = if pending {
            details["diagnostic"] =
                json!("Wait for completion or inspect the local build if the worker exited.");
            ("SKIPPED", "Execution admitted; final outcome not recorded.")
        } else {
            (entry.decision(), report.summary.as_str())
        };
        let sql = "INSERT INTO execution_records(id,request_id,evidence_id,project_id,binding_id,
                capability,actor,decision,sanitized_summary,exit_code,duration_ms,created_at,details_json,target_id,resource_id)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
            ON CONFLICT(id) DO UPDATE SET evidence_id=excluded.evidence_id,decision=excluded.decision,
                sanitized_summary=excluded.sanitized_summary,exit_code=excluded.exit_code,
                duration_ms=excluded.duration_ms,details_json=excluded.details_json
            WHERE execution_records.binding_id=excluded.binding_id AND execution_records.request_id=excluded.request_id";
        self.workspace_write(|tx| {
            purge_expired(tx)?;
            let changed = tx
                .execute(
                    sql,
                    params![
                        id.0,
                        report.request_id.0,
                        report.evidence_id.map(|e| e.0),
                        scope.project.0,
                        scope.binding.0,
                        report.capability.as_deref().unwrap_or(report.operation),
                        actor,
                        decision,
                        summary,
                        entry.outcome.and_then(|r| r.exit_code),
                        i64::try_from(report.duration_ms).unwrap_or(i64::MAX),
                        (report.timestamp_utc.unix_timestamp_nanos() / 1_000_000) as i64,
                        details.to_string(),
                        report.resource_scope.map(|s| s.target.0),
                        report.resource_scope.map(|s| s.resource.0)
                    ],
                )
                .map_err(database_error)?;
            if changed != 1 {
                return Err(crate::Error::InvalidDatabase.into());
            }
            Ok(())
        })
    }
    pub fn lookup_record(
        &self,
        binding: BindingId,
        origin: &Path,
        key: RecordKey,
    ) -> Result<Option<Value>, ManagementError> {
        self.workspace_write(|tx| {
            Self::authorize_history(tx,binding,origin)?;
            purge_expired(tx)?;
            let (request, evidence) = match key {
                RecordKey::Request(r) => (Some(r.0),None), RecordKey::Evidence(e) => (None,Some(e.0)),
            };
            let lookup = "SELECT request_id,evidence_id,project_id,capability,actor,decision,sanitized_summary,
                    exit_code,duration_ms,created_at,details_json FROM execution_records
                WHERE binding_id=?1 AND (request_id=?2 OR evidence_id=?3)";
            tx.query_row(lookup, params![binding.0,request,evidence], |row| record_json(row,binding))
                .optional().map_err(|e|database_error(e).into())
        })
    }
    pub fn clear_records(
        &self,
        binding: BindingId,
        origin: &Path,
    ) -> Result<usize, ManagementError> {
        self.workspace_write(|tx| {
            Self::authorize_history(tx, binding, origin)?;
            Ok(tx
                .execute(
                    "DELETE FROM execution_records WHERE binding_id=?1",
                    [binding.0],
                )
                .map_err(database_error)?)
        })
    }
}

fn record_json(r: &rusqlite::Row<'_>, binding: BindingId) -> rusqlite::Result<Value> {
    let detail: Option<String> = r.get(10)?;
    let mut record = match detail {
        Some(text) => serde_json::from_str::<Value>(&text).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(10, rusqlite::types::Type::Text, Box::new(e))
        })?,
        None => json!({"schemaVersion":1,"createdAtMs":r.get::<_,i64>(9)?}),
    };
    if !record.is_object() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    record["requestId"] = json!(RequestId(r.get(0)?));
    record["evidenceId"] = json!(r.get::<_, Option<_>>(1)?.map(EvidenceId));
    record["projectId"] = json!(ProjectId(r.get(2)?));
    record["bindingId"] = json!(binding);
    for (index, field) in [
        (3, "capability"),
        (4, "actor"),
        (5, "decision"),
        (6, "summary"),
    ] {
        record[field] = json!(r.get::<_, String>(index)?);
    }
    record["exitCode"] = json!(r.get::<_, Option<i32>>(7)?);
    record["durationMs"] = json!(r.get::<_, i64>(8)?);
    Ok(record)
}
