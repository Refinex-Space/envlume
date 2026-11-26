//! Closed task vocabulary. Enabling a template never admits caller-provided argv.
use crate::{
    ErrorCode, Session,
    ports::{BindingSnapshot, Repository},
    result::NamedTask,
};
use envlume_domain::*;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Template {
    MavenTest,
    NodeBuild,
}
impl Template {
    pub fn parse(key: &str) -> Result<Self, ErrorCode> {
        match key {
            "maven-test" => Ok(Self::MavenTest),
            "node-build" => Ok(Self::NodeBuild),
            _ => Err(ErrorCode::InvalidInput),
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::MavenTest => "maven-test",
            Self::NodeBuild => "node-build",
        }
    }
    pub fn executable(self) -> &'static str {
        match self {
            Self::MavenTest => "mvn",
            Self::NodeBuild => "package-manager",
        }
    }
    pub fn arguments(self) -> &'static [&'static str] {
        match self {
            Self::MavenTest => &["-B", "-ntp", "test"],
            Self::NodeBuild => &["run", "build"],
        }
    }
    pub fn accepts(self, task: &TaskDefinition) -> bool {
        task.enabled
            && task.key.as_str() == self.key()
            && task.executable == self.executable()
            && task.arguments == self.arguments()
            && task.workspace_relative_dir == Path::new(".")
            && task.input == TaskInput::Closed
            && (1..=3600).contains(&task.timeout.as_secs())
    }
}

#[non_exhaustive]
pub struct AuthorizedTask {
    pub binding: BindingId,
    pub binding_alias: Alias,
    pub project: ProjectId,
    pub grant: GrantId,
    pub task: TaskId,
    pub root: PathBuf,
    pub profile: ToolchainProfile,
    pub template: Template,
    pub timeout: Duration,
}
pub(crate) fn visible_tasks(state: &BindingSnapshot) -> Vec<NamedTask> {
    let now = UtcDateTime::now();
    state
        .tasks
        .iter()
        .filter(|t| {
            t.project_id == state.project.id
                && t.binding_id.is_none_or(|b| b == state.binding.id)
                && Template::parse(t.key.as_str()).is_ok_and(|v| v.accepts(t))
                && state.has_grant(Capability::TaskRun(t.id), now)
        })
        .map(|t| NamedTask {
            task_id: t.key.clone(),
            timeout_seconds: t.timeout.as_secs() as u32,
        })
        .collect()
}
impl<R: Repository> Session<R> {
    pub fn task_visibility(&self) -> Result<(bool, bool), ErrorCode> {
        let state = self.snapshot()?;
        Ok((
            state.has_grant(Capability::TaskInspect, UtcDateTime::now()),
            !visible_tasks(&state).is_empty(),
        ))
    }
    pub fn authorize_task(&self, template: Template) -> Result<AuthorizedTask, ErrorCode> {
        let state = self.snapshot()?;
        let task = state
            .tasks
            .iter()
            .find(|t| {
                t.project_id == state.project.id
                    && t.binding_id.is_none_or(|b| b == state.binding.id)
                    && template.accepts(t)
            })
            .ok_or(ErrorCode::PolicyDenied)?;
        let cap = Capability::TaskRun(task.id);
        let now = UtcDateTime::now();
        let grant = state
            .grants
            .iter()
            .find(|g| {
                g.binding_id == state.binding.id
                    && g.capability == cap
                    && g.revoked_at.is_none()
                    && g.expires_at.is_none_or(|end| end > now)
            })
            .ok_or(ErrorCode::PolicyDenied)?;
        let profile = state
            .profiles
            .into_iter()
            .find(|p| p.project_id == state.project.id && Some(p.id) == state.binding.profile_id)
            .ok_or(ErrorCode::ToolchainMissing)?;
        Ok(AuthorizedTask {
            binding: state.binding.id,
            binding_alias: state.binding.alias,
            root: state.binding.canonical_path,
            project: state.project.id,
            profile,
            template,
            task: task.id,
            grant: grant.id,
            timeout: task.timeout,
        })
    }
}
