use crate::{Error, Store, connect, database_error, schema};
use envlume_application::{
    tasks::{AuthorizedTask, Template},
    workspaces::ManagementError,
};
use envlume_domain::{BindingId, GrantId, TaskId};
use rusqlite::{OptionalExtension, params};
use std::path::Path;

impl Store {
    pub fn enable_task(
        &self,
        binding: BindingId,
        origin: &Path,
        template: Template,
        timeout: u32,
    ) -> Result<(), ManagementError> {
        if !(1..=3600).contains(&timeout) {
            return Err(ManagementError::InvalidPath);
        }
        self.workspace_write(|tx| {
            let project = Self::verified_workspace(tx, binding, origin)?.project_id;
            let args = serde_json::to_string(template.arguments()).map_err(|_| Error::InvalidDatabase)?;
            let add = "INSERT INTO task_definitions(
                    id,project_id,task_key,executable,arguments,workspace_relative_dir,timeout_seconds,input,enabled)
                VALUES(?1,?2,?3,?4,?5,'.',600,'CLOSED',1)
                ON CONFLICT(project_id,task_key) DO NOTHING";
            tx.execute(add, params![TaskId::generate().0, project.0, template.key(), template.executable(), args])
                .map_err(database_error)?;
            let select = "SELECT id FROM task_definitions
                WHERE project_id=?1 AND task_key=?2 AND executable=?3 AND json(arguments)=?4
                  AND workspace_relative_dir='.' AND enabled=1 AND (binding_id IS NULL OR binding_id=?5)";
            let task = tx.query_row(select, params![project.0, template.key(), template.executable(), args, binding.0],
                    |r| Ok(TaskId(r.get(0)?)))
                .optional().map_err(database_error)?.ok_or(ManagementError::ConfirmationRequired)?;
            let expire = "UPDATE capability_grants SET revoked_at=expires_at
                WHERE binding_id=?1 AND revoked_at IS NULL AND expires_at<=unixepoch('subsec')*1000
                  AND (capability='task.inspect' OR (capability='task.run' AND task_id=?2))";
            tx.execute(expire, params![binding.0, task.0]).map_err(database_error)?;

            let inspect = "INSERT INTO capability_grants(id,project_id,binding_id,capability)
                SELECT ?1,?2,?3,'task.inspect' WHERE NOT EXISTS(
                    SELECT 1 FROM capability_grants WHERE binding_id=?3 AND capability='task.inspect' AND revoked_at IS NULL)";
            tx.execute(inspect, params![GrantId::generate().0, project.0, binding.0]).map_err(database_error)?;

            let current = "SELECT id FROM capability_grants
                WHERE binding_id=?1 AND task_id=?2 AND capability='task.run' AND revoked_at IS NULL";
            let grant = tx.query_row(current, params![binding.0, task.0], |r| Ok(GrantId(r.get(0)?)))
                .optional().map_err(database_error)?;
            if let Some(grant) = grant {
                let update = "UPDATE capability_grants
                    SET timeout_seconds=?2, approval_digest=NULL, approved_at_ms=NULL, distribution_digest=NULL
                    WHERE id=?1 AND timeout_seconds IS NOT ?2";
                tx.execute(update, params![grant.0, timeout]).map_err(database_error)?;
            } else {
                let insert = "INSERT INTO capability_grants(id,project_id,binding_id,task_id,capability,timeout_seconds)
                    VALUES(?1,?2,?3,?4,'task.run',?5)";
                tx.execute(insert, params![GrantId::generate().0, project.0, binding.0, task.0, timeout])
                    .map_err(database_error)?;
            }
            Ok(())
        })
    }
    pub fn disable_task(
        &self,
        binding: BindingId,
        origin: &Path,
        template: Template,
    ) -> Result<(), ManagementError> {
        self.workspace_write(|tx| {
            Self::verified_workspace(tx, binding, origin)?;
            let sql =
                "UPDATE capability_grants SET revoked_at=CAST(unixepoch('subsec')*1000 AS INTEGER)
                WHERE binding_id=?1 AND capability='task.run' AND revoked_at IS NULL AND task_id IN
                (SELECT id FROM task_definitions WHERE task_key=?2)";
            tx.execute(sql, params![binding.0, template.key()])
                .map_err(database_error)?;
            Ok(())
        })
    }
    pub fn task_approved(
        &self,
        task: &AuthorizedTask,
        digest: &str,
        distribution: Option<&str>,
    ) -> Result<bool, Error> {
        let mut conn = connect(&self.paths.database(), false)?;
        let tx = conn.transaction().map_err(database_error)?;
        if schema::version(&tx)? != schema::CURRENT {
            return Err(Error::InvalidDatabase);
        }
        let sql = "SELECT 1 FROM capability_grants WHERE id=?1 AND binding_id=?2 AND task_id=?3 AND revoked_at IS NULL
            AND (expires_at IS NULL OR expires_at>unixepoch('subsec')*1000) AND approved_at_ms IS NOT NULL
            AND approval_digest=?4 AND distribution_digest IS ?5";
        tx.prepare(sql)
            .map_err(database_error)?
            .exists(params![
                task.grant.0,
                task.binding.0,
                task.task.0,
                digest,
                distribution
            ])
            .map_err(database_error)
    }
    pub fn approve_task(
        &self,
        task: &AuthorizedTask,
        digest: &str,
        distribution: Option<&str>,
    ) -> Result<(), ManagementError> {
        self.workspace_write(|tx| {
            Self::verified_workspace(tx, task.binding, &task.root)?;
            let sql = "UPDATE capability_grants
                SET approval_digest=?4, approved_at_ms=CAST(unixepoch('subsec')*1000 AS INTEGER), distribution_digest=?5
                WHERE id=?1 AND binding_id=?2 AND task_id=?3 AND capability='task.run' AND revoked_at IS NULL
                  AND (expires_at IS NULL OR expires_at>unixepoch('subsec')*1000)";
            let changed = tx.execute(sql, params![task.grant.0, task.binding.0, task.task.0, digest, distribution])
                .map_err(database_error)?;
            if changed != 1 {
                return Err(ManagementError::ConfirmationRequired);
            }
            Ok(())
        })
    }
}
