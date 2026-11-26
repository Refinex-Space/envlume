use crate::{input::ToolCall, ports::*, result::*};
use envlume_domain::*;
use serde_json::Value;
use std::time::Instant;

/// A trusted local entry point selects the binding. Never deserialize this from tool arguments.
pub struct Session<R> {
    repository: R,
    binding: BindingId,
    project: ProjectId,
    actor: Actor,
}
#[non_exhaustive]
pub struct RequestContext {
    pub binding_id: BindingId,
    pub project_id: ProjectId,
    pub actor: Actor,
    pub request_id: RequestId,
}
fn trusted(repository: &impl Repository, id: BindingId) -> Result<BindingSnapshot, ErrorCode> {
    let state = repository
        .load_binding(id)?
        .ok_or(ErrorCode::ProjectNotTrusted)?;
    if state.binding.id != id
        || state.binding.project_id != state.project.id
        || state.project.deleted_at.is_some()
    {
        return Err(ErrorCode::ProjectNotTrusted);
    }
    match state.binding.state {
        BindingState::Trusted => Ok(state),
        BindingState::Revoked(_) => Err(ErrorCode::BindingRevoked),
        _ => Err(ErrorCode::ProjectNotTrusted),
    }
}

impl<R: Repository> Session<R> {
    pub fn from_registration(
        repository: R,
        binding: BindingId,
        actor: Actor,
    ) -> Result<Self, ErrorCode> {
        let state = trusted(&repository, binding)?;
        Ok(Self {
            repository,
            binding,
            project: state.project.id,
            actor,
        })
    }

    fn snapshot(&self) -> Result<BindingSnapshot, ErrorCode> {
        let state = trusted(&self.repository, self.binding)?;
        if state.project.id != self.project {
            return Err(ErrorCode::ProjectNotTrusted);
        }
        Ok(state)
    }

    pub fn can_inspect_project(&self) -> Result<bool, ErrorCode> {
        Ok(self
            .snapshot()?
            .has_grant(Capability::ProjectInspect, UtcDateTime::now()))
    }

    pub fn invoke(&self, cases: &impl UseCases, name: &str, arguments: Value) -> Report {
        self.dispatch(name, arguments, |operation, request| {
            operation.run(cases, request)
        })
    }

    pub fn project_context(&self, arguments: Value) -> Report {
        self.dispatch("project_context", arguments, |_, request| {
            Ok((Payload::Project(project_summary(request.snapshot())), None))
        })
    }

    pub fn can_inspect_toolchain(&self) -> Result<bool, ErrorCode> {
        let state = self.snapshot()?;
        Ok(state.binding.profile_id.is_some()
            && state.has_grant(Capability::ToolchainInspect, UtcDateTime::now()))
    }

    pub fn toolchain_doctor(
        &self,
        arguments: Value,
        inspector: &impl ToolchainInspector,
    ) -> Report {
        self.dispatch("toolchain_doctor", arguments, |operation, request| {
            let Resolved::Doctor(name) = operation else {
                return Err(ErrorCode::InternalError.into());
            };
            let state = request.snapshot();
            let profile = state
                .profiles
                .iter()
                .find(|p| {
                    Some(p.id) == state.binding.profile_id
                        && p.project_id == state.project.id
                        && p.name == *name
                })
                .ok_or(ErrorCode::ToolchainMissing)?;
            let tools = inspector.diagnose(&state.binding.canonical_path, profile)?;
            Ok((Payload::Doctor { tools }, None))
        })
    }

    fn dispatch(
        &self,
        name: &str,
        arguments: Value,
        run: impl FnOnce(Resolved<'_>, &Authorized<'_>) -> UseCaseResult<(Payload, Option<EvidenceId>)>,
    ) -> Report {
        let start = Instant::now();
        let context = RequestContext {
            binding_id: self.binding,
            project_id: self.project,
            actor: self.actor,
            request_id: RequestId::generate(),
        };
        let mut report = Report {
            schema_version: 1,
            request_id: context.request_id,
            timestamp_utc: UtcDateTime::now(),
            duration_ms: 0,
            ok: true,
            operation: crate::input::TOOL_NAMES
                .iter()
                .copied()
                .find(|known| *known == name)
                .unwrap_or("unknown"),
            binding_alias: None,
            target_alias: None,
            capability: None,
            evidence_id: None,
            status: ReportStatus::Success,
            summary: "Completed",
            error_code: None,
            result: None,
        };
        let response = (|| {
            let call = ToolCall::parse(name, arguments)?;
            let state = self.snapshot()?;
            report.binding_alias = Some(state.binding.alias.clone());
            let operation = Resolved::resolve(&call, &state)?;
            let capability = operation.capability();
            if !state.has_grant(capability, UtcDateTime::now()) {
                return Err(ErrorCode::PolicyDenied.into());
            }
            report.target_alias = operation.target().map(|target| target.alias.clone());
            report.capability = Some(capability.wire_name());
            let request = Authorized {
                context: &context,
                snapshot: &state,
                capability,
            };
            run(operation, &request)
        })();
        match response {
            Ok((payload, evidence)) => {
                report.result = Some(payload);
                report.evidence_id = evidence;
            }
            Err(UseCaseFailure { code, evidence_id }) => {
                report.ok = false;
                report.status = if code.exit_code() == 3 {
                    ReportStatus::Denied
                } else {
                    ReportStatus::Failed
                };
                report.summary = code.summary();
                report.error_code = Some(code);
                report.evidence_id = evidence_id;
            }
        }
        report.duration_ms = start.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        report
    }
}

fn remote_enabled(t: &EnvironmentTarget) -> bool {
    t.status == Status::Active && t.kind != EnvironmentKind::Prod && t.risk != RiskLevel::Critical
}
fn query_enabled(t: &EnvironmentTarget) -> bool {
    remote_enabled(t)
        && matches!(t.kind, EnvironmentKind::Dev | EnvironmentKind::Test)
        && t.risk != RiskLevel::High
}

fn project_summary(state: &BindingSnapshot) -> ProjectSummary {
    let now = UtcDateTime::now();
    let mut targets = Vec::new();
    let mut capabilities = Vec::new();
    for grant in &state.grants {
        if grant.binding_id != state.binding.id
            || grant.revoked_at.is_some()
            || grant.expires_at.is_some_and(|t| t <= now)
        {
            continue;
        }
        match grant.capability {
            Capability::TaskRun(id)
                if !state.tasks.iter().any(|t| {
                    t.id == id
                        && t.project_id == state.project.id
                        && t.enabled
                        && t.binding_id.is_none_or(|b| b == state.binding.id)
                }) =>
            {
                continue;
            }
            Capability::DatabaseSchema(scope)
            | Capability::DatabaseRead(scope)
            | Capability::ResourceConnect(scope) => {
                let Some(target) = state.targets.iter().find(|t| {
                    t.id == scope.target && t.project_id == state.project.id && remote_enabled(t)
                }) else {
                    continue;
                };
                if matches!(grant.capability, Capability::DatabaseRead(_)) && !query_enabled(target)
                {
                    continue;
                }
                if !state.resources.iter().any(|r| {
                    r.target_id == target.id && r.id == scope.resource && r.status == Status::Active
                }) {
                    continue;
                }
                targets.push(target.alias.clone());
            }
            _ => (),
        }
        capabilities.push(grant.capability.wire_name());
    }
    targets.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    targets.dedup();
    capabilities.sort();
    capabilities.dedup();
    ProjectSummary {
        project: state.project.name.clone(),
        workspace: state.binding.alias.as_str().into(),
        targets,
        capabilities,
    }
}

struct Remote<'a> {
    target: &'a EnvironmentTarget,
    resource: &'a ResourceBinding,
}
impl Remote<'_> {
    fn scope(&self) -> ResourceScope {
        ResourceScope {
            target: self.target.id,
            resource: self.resource.id,
        }
    }
}
enum Resolved<'a> {
    Project,
    Environments,
    Doctor(&'a Alias),
    Tasks,
    Run(&'a TaskDefinition),
    Connect(Remote<'a>),
    Schema(Remote<'a>, Option<&'a crate::input::TableName>),
    Query(Remote<'a>, &'a crate::input::QueryText),
}
impl<'a> Resolved<'a> {
    fn resolve(call: &'a ToolCall, state: &'a BindingSnapshot) -> Result<Self, ErrorCode> {
        let project = state.project.id;
        let remote = |target: &Alias, resource: &Alias| {
            let target = state
                .targets
                .iter()
                .find(|t| {
                    t.project_id == project && t.alias == *target && t.status == Status::Active
                })
                .ok_or(ErrorCode::TargetNotAllowed)?;
            if !remote_enabled(target) {
                return Err(ErrorCode::TargetNotAllowed);
            }
            let resource = state
                .resources
                .iter()
                .find(|r| {
                    r.target_id == target.id && r.alias == *resource && r.status == Status::Active
                })
                .ok_or(ErrorCode::TargetNotAllowed)?;
            Ok(Remote { target, resource })
        };
        Ok(match call {
            ToolCall::ProjectContext(_) => Self::Project,
            ToolCall::EnvironmentList(_) => Self::Environments,
            ToolCall::ToolchainDoctor(i) => Self::Doctor(&i.profile),
            ToolCall::TaskList(_) => Self::Tasks,
            ToolCall::TaskRun(i) => Self::Run(
                state
                    .tasks
                    .iter()
                    .find(|t| {
                        t.project_id == project
                            && t.binding_id.is_none_or(|id| id == state.binding.id)
                            && t.enabled
                            && t.key == i.task_id
                    })
                    .ok_or(ErrorCode::PolicyDenied)?,
            ),
            ToolCall::ConnectionTest(i) => Self::Connect(remote(&i.target, &i.resource)?),
            ToolCall::DatabaseSchema(i) => {
                Self::Schema(remote(&i.target, &i.resource)?, i.table.as_ref())
            }
            ToolCall::DatabaseQuery(i) => {
                let remote = remote(&i.target, &i.resource)?;
                if !query_enabled(remote.target) {
                    return Err(ErrorCode::TargetNotAllowed);
                }
                Self::Query(remote, &i.sql)
            }
        })
    }
    fn capability(&self) -> Capability {
        match self {
            Self::Project => Capability::ProjectInspect,
            Self::Environments => Capability::EnvironmentInspect,
            Self::Doctor(_) => Capability::ToolchainInspect,
            Self::Tasks => Capability::TaskInspect,
            Self::Run(task) => Capability::TaskRun(task.id),
            Self::Connect(r) => Capability::ResourceConnect(r.scope()),
            Self::Schema(r, _) => Capability::DatabaseSchema(r.scope()),
            Self::Query(r, _) => Capability::DatabaseRead(r.scope()),
        }
    }
    fn target(&self) -> Option<&EnvironmentTarget> {
        match self {
            Self::Connect(r) | Self::Schema(r, _) | Self::Query(r, _) => Some(r.target),
            _ => None,
        }
    }
    fn run(
        self,
        cases: &impl UseCases,
        request: &Authorized<'_>,
    ) -> UseCaseResult<(Payload, Option<EvidenceId>)> {
        Ok(match self {
            Self::Project => (Payload::Project(cases.project_context(request)?), None),
            Self::Environments => (
                Payload::Environments {
                    environments: cases.environment_list(request)?,
                },
                None,
            ),
            Self::Doctor(name) => {
                let profile = request
                    .snapshot
                    .profiles
                    .iter()
                    .find(|p| p.project_id == request.context.project_id && p.name == *name)
                    .ok_or(ErrorCode::ToolchainMissing)?;
                (
                    Payload::Doctor {
                        tools: cases.toolchain_doctor(request, profile)?,
                    },
                    None,
                )
            }
            Self::Tasks => (
                Payload::Tasks {
                    tasks: cases.task_list(request)?,
                },
                None,
            ),
            Self::Run(t) => {
                let e = cases.task_run(request, t)?;
                (Payload::Task(e.result), Some(e.evidence_id))
            }
            Self::Connect(r) => {
                let e = cases.connection_test(
                    request,
                    &AuthorizedResource {
                        binding: r.resource,
                    },
                )?;
                (Payload::Connection(e.result), Some(e.evidence_id))
            }
            Self::Schema(r, t) => (
                Payload::Schema {
                    tables: cases.database_schema(
                        request,
                        &AuthorizedResource {
                            binding: r.resource,
                        },
                        t,
                    )?,
                },
                None,
            ),
            Self::Query(r, sql) => {
                let e = cases.database_query(
                    request,
                    &AuthorizedResource {
                        binding: r.resource,
                    },
                    sql,
                )?;
                (Payload::Query(e.result), Some(e.evidence_id))
            }
        })
    }
}
