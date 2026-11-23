//! Synthetic registry and use cases. No I/O, process launch or credentials.
use envlume_application::{
    input::{QueryText, TableName},
    ports::*,
    result::*,
};
use envlume_domain::*;
use std::{
    cell::{Cell as Counter, RefCell},
    path::PathBuf,
    time::Duration,
};
pub fn alias(s: &str) -> Alias {
    s.to_owned().try_into().unwrap()
}
pub const TASK: &str = "30000000-0000-4000-8000-000000000001";
pub const RESOURCE: &str = "40000000-0000-4000-8000-000000000001";
pub const EVIDENCE: &str = "20000000-0000-4000-8000-000000000001";

pub fn state() -> BindingSnapshot {
    let now = UtcDateTime::now();
    let project = Project {
        id: ProjectId::generate(),
        name: "orders".into(),
        description: String::new(),
        created_at: now,
        updated_at: now,
        deleted_at: None,
    };
    let profile = ToolchainProfile {
        id: ProfileId::generate(),
        project_id: project.id,
        name: alias("default"),
        rules: vec![ToolRule {
            tool: alias("jdk"),
            version_requirement: "8".into(),
            selected_path: None,
            priority: 1,
        }],
    };
    let binding = WorkspaceBinding {
        alias: alias("orders-main"),
        id: BindingId::generate(),
        project_id: project.id,
        profile_id: Some(profile.id),
        canonical_path: std::env::temp_dir().join("orders-main"),
        git_fingerprint: Some("fixture".into()),
        state: BindingState::Trusted,
    };
    let dev = EnvironmentTarget {
        id: TargetId::generate(),
        project_id: project.id,
        alias: alias("dev"),
        kind: EnvironmentKind::Dev,
        risk: RiskLevel::Low,
        is_default: true,
        status: Status::Active,
    };
    let test = EnvironmentTarget {
        id: TargetId::generate(),
        alias: alias("test"),
        kind: EnvironmentKind::Test,
        is_default: false,
        ..dev.clone()
    };
    let resource = ResourceBinding {
        id: ResourceId(RESOURCE.parse().unwrap()),
        target_id: dev.id,
        alias: alias("primary-db"),
        kind: ResourceKind::MySql,
        host: "private.invalid".into(),
        port: 3306,
        tls: TlsMode::VerifyIdentity,
        credential_ref: Some(CredentialRef("fixture-reference".into())),
        policy_labels: vec![],
        status: Status::Active,
    };
    let other = ResourceBinding {
        id: ResourceId::generate(),
        target_id: test.id,
        ..resource.clone()
    };
    let task = TaskDefinition {
        id: TaskId(TASK.parse().unwrap()),
        project_id: project.id,
        binding_id: Some(binding.id),
        key: alias("maven-test"),
        executable: "mvn".into(),
        arguments: vec!["test".into()],
        workspace_relative_dir: PathBuf::from("."),
        timeout: Duration::from_secs(600),
        input: TaskInput::Closed,
        enabled: true,
    };
    let scope = ResourceScope {
        target: dev.id,
        resource: resource.id,
    };
    let grants = [
        Capability::ProjectInspect,
        Capability::EnvironmentInspect,
        Capability::ToolchainInspect,
        Capability::TaskInspect,
        Capability::TaskRun(task.id),
        Capability::ResourceConnect(scope),
        Capability::DatabaseSchema(scope),
        Capability::DatabaseRead(scope),
    ]
    .into_iter()
    .map(|capability| CapabilityGrant {
        id: GrantId::generate(),
        binding_id: binding.id,
        capability,
        expires_at: None,
        revoked_at: None,
    })
    .collect();
    BindingSnapshot {
        binding,
        project,
        targets: vec![dev, test],
        resources: vec![resource, other],
        tasks: vec![task],
        profiles: vec![profile],
        grants,
    }
}

pub struct Memory(pub RefCell<BindingSnapshot>);
impl Repository for Memory {
    fn load_binding(&self, id: BindingId) -> Result<Option<BindingSnapshot>, ErrorCode> {
        let state = self.0.borrow();
        Ok((state.binding.id == id).then(|| state.clone()))
    }
}
#[derive(Default)]
pub struct FixtureCases {
    pub calls: Counter<usize>,
    pub failure: Counter<Option<ErrorCode>>,
}
impl FixtureCases {
    fn enter(&self) -> UseCaseResult<()> {
        self.calls.set(self.calls.get() + 1);
        match self.failure.get() {
            Some(code) => Err(code.into()),
            None => Ok(()),
        }
    }
}
fn executed<T>(result: T) -> Executed<T> {
    Executed {
        evidence_id: EvidenceId(EVIDENCE.parse().unwrap()),
        result,
    }
}

fn visible_environments(state: &BindingSnapshot) -> Vec<VisibleEnvironment> {
    let now = UtcDateTime::now();
    let mut visible = Vec::new();
    for target in &state.targets {
        if target.project_id != state.project.id
            || target.status != Status::Active
            || target.kind == EnvironmentKind::Prod
            || target.risk == RiskLevel::Critical
        {
            continue;
        }
        let mut resources = Vec::new();
        for resource in &state.resources {
            let scope = ResourceScope {
                target: target.id,
                resource: resource.id,
            };
            if resource.target_id == target.id
                && resource.status == Status::Active
                && [
                    Capability::DatabaseSchema(scope),
                    Capability::DatabaseRead(scope),
                    Capability::ResourceConnect(scope),
                ]
                .into_iter()
                .any(|c| state.has_grant(c, now))
            {
                resources.push(VisibleResource {
                    alias: resource.alias.clone(),
                    health: Health::Unknown,
                });
            }
        }
        if !resources.is_empty() {
            visible.push(VisibleEnvironment {
                target: target.alias.clone(),
                resources,
            });
        }
    }
    visible
}

impl UseCases for FixtureCases {
    fn project_context(&self, r: &Authorized<'_>) -> UseCaseResult<ProjectSummary> {
        self.enter()?;
        Ok(ProjectSummary {
            project: r.snapshot().project.name.clone(),
            workspace: r.snapshot().binding.alias.as_str().into(),
            targets: visible_environments(r.snapshot())
                .into_iter()
                .map(|v| v.target)
                .collect(),
            capabilities: vec![r.capability().wire_name()],
        })
    }
    fn environment_list(&self, r: &Authorized<'_>) -> UseCaseResult<Vec<VisibleEnvironment>> {
        self.enter()?;
        Ok(visible_environments(r.snapshot()))
    }
    fn toolchain_doctor(
        &self,
        _: &Authorized<'_>,
        p: &ToolchainProfile,
    ) -> UseCaseResult<Vec<ToolDiagnostic>> {
        self.enter()?;
        Ok(vec![ToolDiagnostic {
            tool: p.rules[0].tool.clone(),
            version: Some(p.rules[0].version_requirement.clone()),
            path_summary: Some("selected JDK".into()),
            repair: None,
        }])
    }
    fn task_list(&self, r: &Authorized<'_>) -> UseCaseResult<Vec<NamedTask>> {
        self.enter()?;
        let state = r.snapshot();
        let now = UtcDateTime::now();
        Ok(state
            .tasks
            .iter()
            .filter(|t| {
                t.enabled
                    && t.project_id == state.project.id
                    && t.binding_id.is_none_or(|id| id == state.binding.id)
                    && state.has_grant(Capability::TaskRun(t.id), now)
            })
            .map(|t| NamedTask {
                task_id: t.key.clone(),
                timeout_seconds: t.timeout.as_secs().try_into().unwrap(),
            })
            .collect())
    }
    fn task_run(
        &self,
        _: &Authorized<'_>,
        task: &TaskDefinition,
    ) -> UseCaseResult<Executed<TaskResult>> {
        self.enter()?;
        assert_eq!(task.key.as_str(), "maven-test");
        Ok(executed(TaskResult {
            exit_code: 0,
            stdout_summary: "Tests passed".into(),
            stderr_summary: String::new(),
        }))
    }
    fn connection_test(
        &self,
        _: &Authorized<'_>,
        resource: &AuthorizedResource<'_>,
    ) -> UseCaseResult<Executed<ConnectionResult>> {
        self.enter()?;
        assert_eq!(resource.binding().id.0.to_string(), RESOURCE);
        Ok(executed(ConnectionResult {
            latency_ms: 6,
            version_summary: "MySQL 8".into(),
        }))
    }
    fn database_schema(
        &self,
        _: &Authorized<'_>,
        _: &AuthorizedResource<'_>,
        table: Option<&TableName>,
    ) -> UseCaseResult<Vec<Table>> {
        self.enter()?;
        Ok(vec![Table {
            table: table.map_or("orders", TableName::as_str).into(),
            columns: vec![Column {
                name: "id".into(),
                sql_type: "bigint".into(),
                nullable: false,
            }],
            indexes: vec![Index {
                name: "PRIMARY".into(),
                columns: vec!["id".into()],
            }],
        }])
    }
    fn database_query(
        &self,
        _: &Authorized<'_>,
        _: &AuthorizedResource<'_>,
        sql: &QueryText,
    ) -> UseCaseResult<Executed<QueryResult>> {
        self.enter()?;
        assert_eq!(sql.as_str(), "SELECT id FROM orders");
        Ok(executed(QueryResult::new(
            vec!["id".into()],
            vec![vec![Cell::Number(1.into())]],
            false,
        )?))
    }
}
