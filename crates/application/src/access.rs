use crate::{input::ToolCall, ports::*, result::*};
use envlume_domain::*;
use envlume_policy::RemoteAction;
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

    pub(crate) fn snapshot(&self) -> Result<BindingSnapshot, ErrorCode> {
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
            operation.run(cases, request, &|| self.snapshot())
        })
    }

    pub fn project_context(&self, arguments: Value) -> Report {
        self.dispatch("project_context", arguments, |_, request| {
            Ok((Payload::Project(project_summary(request.snapshot())), None))
        })
    }

    pub fn can_inspect_environments(&self) -> Result<bool, ErrorCode> {
        Ok(self
            .snapshot()?
            .has_grant(Capability::EnvironmentInspect, UtcDateTime::now()))
    }
    pub fn environment_list(&self, arguments: Value) -> Report {
        self.environment_health(arguments, &|_| Health::Unknown)
    }
    pub fn environment_health(
        &self,
        arguments: Value,
        health: &dyn Fn(ResourceId) -> Health,
    ) -> Report {
        self.dispatch("environment_list", arguments, |_, request| {
            Ok((
                Payload::Environments {
                    environments: crate::environments::visible_with_health(
                        request.snapshot(),
                        health,
                    ),
                },
                None,
            ))
        })
    }

    pub fn refresh_remote(
        &self,
        backend: Option<&dyn crate::remote::DatabaseBackend>,
    ) -> Result<(bool, bool), ErrorCode> {
        let state = match self.snapshot() {
            Ok(s) => s,
            Err(e) => {
                if let Some(b) = backend {
                    b.retain(&|key, _| key.binding != self.binding);
                }
                return Err(e);
            }
        };
        let now = UtcDateTime::now();
        let live = |g: &CapabilityGrant| {
            let (a, scope) = RemoteAction::from_capability(g.capability)?;
            if g.binding_id != self.binding
                || g.revoked_at.is_some()
                || g.expires_at.is_some_and(|v| v <= now)
            {
                return None;
            }
            let t = state
                .targets
                .iter()
                .find(|t| t.id == scope.target && t.project_id == self.project)?;
            let r = state
                .resources
                .iter()
                .find(|r| r.id == scope.resource && a.permits(t, r))?;
            Some((a, r))
        };
        if let Some(backend) = backend {
            backend.retain(&|key, prior| {
                key.binding != self.binding
                    || state
                        .grants
                        .iter()
                        .any(|g| g.id == key.grant && live(g).is_some_and(|(_, r)| r == prior))
            });
        }
        let mut connect = false;
        let mut schema = false;
        for (action, resource) in state.grants.iter().filter_map(live) {
            if resource.route.is_none()
                || backend.is_some_and(|b| !b.capabilities().contains(&action))
            {
                continue;
            }
            connect |= action == RemoteAction::Connect;
            schema |= action == RemoteAction::Schema && !resource.tables.is_empty();
        }
        Ok((connect, schema))
    }
    pub fn remote_operation(
        &self,
        name: &str,
        args: Value,
        backend: &dyn crate::remote::DatabaseBackend,
    ) -> Report {
        let mut fault = None;
        let _visibility = self.refresh_remote(Some(backend));
        let mut report = self.dispatch(name, args, |op, request| {
            let (endpoint, table, is_test) = match op {
                Resolved::Connect(r) => (r, None, true),
                Resolved::Schema(r, t) => (r, t, false),
                _ => return Err(ErrorCode::InvalidInput.into()),
            };
            let r = AuthorizedResource {
                binding: endpoint.resource,
                request,
                reload: &|| self.snapshot(),
            };
            let result = if is_test {
                backend.test(&r).map(Payload::Connection)
            } else {
                backend
                    .schema(&r, table)
                    .map(|tables| Payload::Schema { tables })
            };
            match result {
                Ok(payload) => Ok((payload, is_test.then(EvidenceId::generate))),
                Err(e) => {
                    fault = Some(e);
                    Err(e.code().into())
                }
            }
        });
        if let Some(f) = fault {
            report.summary = f.summary().into();
        }
        report
    }
    pub fn task_list(&self) -> Report {
        self.task_list_with(serde_json::json!({}))
    }

    pub fn task_list_with(&self, arguments: Value) -> Report {
        self.dispatch("task_list", arguments, |_, request| {
            Ok((
                Payload::Tasks {
                    tasks: crate::tasks::visible_tasks(request.snapshot()),
                },
                None,
            ))
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
            let profile = selected_profile(state, name)?;
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
        let operation = crate::input::TOOL_NAMES
            .iter()
            .copied()
            .find(|known| *known == name)
            .unwrap_or("unknown");
        let mut report = Report::new(operation);
        let context = RequestContext {
            binding_id: self.binding,
            project_id: self.project,
            actor: self.actor,
            request_id: report.request_id,
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
            report.resource_scope = RemoteAction::from_capability(capability).map(|(_, s)| s);
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
                report.fail(code, code.summary());
                report.evidence_id = evidence_id;
            }
        }
        report.duration_ms = start.elapsed().as_millis().try_into().unwrap_or(u64::MAX);
        report
    }
}

fn selected_profile<'a>(
    state: &'a BindingSnapshot,
    name: &Alias,
) -> Result<&'a ToolchainProfile, ErrorCode> {
    state
        .profiles
        .iter()
        .find(|p| {
            Some(p.id) == state.binding.profile_id
                && p.project_id == state.project.id
                && p.name == *name
        })
        .ok_or(ErrorCode::ToolchainMissing)
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
        if let Capability::TaskRun(id) = grant.capability
            && !state.tasks.iter().any(|t| {
                t.id == id
                    && t.project_id == state.project.id
                    && t.enabled
                    && t.binding_id.is_none_or(|b| b == state.binding.id)
            })
        {
            continue;
        }
        if let Some((action, scope)) = RemoteAction::from_capability(grant.capability) {
            let Some(target) = state
                .targets
                .iter()
                .find(|t| t.project_id == state.project.id && t.id == scope.target)
            else {
                continue;
            };
            if !state
                .resources
                .iter()
                .any(|r| r.id == scope.resource && action.permits(target, r))
            {
                continue;
            }
            targets.push(target.alias.clone());
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
        let remote = |target: &Alias, resource: &Alias, action: RemoteAction| {
            let target = state
                .targets
                .iter()
                .find(|t| {
                    t.project_id == project && t.alias == *target && t.status == Status::Active
                })
                .ok_or(ErrorCode::TargetNotAllowed)?;
            if !action.allowed(&target.kind, target.risk) {
                return Err(ErrorCode::TargetNotAllowed);
            }
            let resource = state
                .resources
                .iter()
                .find(|r| r.alias == *resource && action.permits(target, r))
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
            ToolCall::ConnectionTest(i) => {
                Self::Connect(remote(&i.target, &i.resource, RemoteAction::Connect)?)
            }
            ToolCall::DatabaseSchema(i) => Self::Schema(
                remote(&i.target, &i.resource, RemoteAction::Schema)?,
                i.table.as_ref(),
            ),
            ToolCall::DatabaseQuery(i) => {
                let remote = remote(&i.target, &i.resource, RemoteAction::Read)?;
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
        reload: &dyn Fn() -> Result<BindingSnapshot, ErrorCode>,
    ) -> UseCaseResult<(Payload, Option<EvidenceId>)> {
        Ok(match self {
            Self::Project => (Payload::Project(cases.project_context(request)?), None),
            Self::Environments => {
                let allowed = crate::environments::visible(request.snapshot());
                let mut reported = cases.environment_list(request)?;
                reported.retain_mut(|e| {
                    let Some(t) = allowed.iter().find(|t| t.target == e.target) else {
                        return false;
                    };
                    e.resources
                        .retain(|r| t.resources.iter().any(|a| a.alias == r.alias));
                    !e.resources.is_empty()
                });
                (
                    Payload::Environments {
                        environments: reported,
                    },
                    None,
                )
            }
            Self::Doctor(name) => {
                let profile = selected_profile(request.snapshot(), name)?;
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
                        request,
                        reload,
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
                            request,
                            reload,
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
                        request,
                        reload,
                    },
                    sql,
                )?;
                (Payload::Query(e.result), Some(e.evidence_id))
            }
        })
    }
}
