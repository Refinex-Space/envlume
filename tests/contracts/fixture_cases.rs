//! Application stand-in shared by the domain contract and STDIO subprocess tests.
use envlume_application::{
    input::{QueryText, TableName},
    ports::*,
    result::*,
};
use envlume_domain::*;
use std::cell::Cell as Counter;
pub const RESOURCE: &str = "40000000-0000-4000-8000-000000000001";
pub const EVIDENCE: &str = "20000000-0000-4000-8000-000000000001";
#[derive(Default)]
pub struct FixtureCases {
    pub calls: Counter<usize>,
    pub secret_checks: Counter<usize>,
    pub connections: Counter<usize>,
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

use envlume_application::environments::visible as visible_environments;

impl SecretStore for FixtureCases {
    fn check_reference(&self, resource: &AuthorizedResource<'_>) -> Result<(), ErrorCode> {
        resource.binding()?;
        self.secret_checks.set(self.secret_checks.get() + 1);
        Ok(())
    }
}
impl ResourceConnector for FixtureCases {
    fn kind(&self) -> ResourceKind {
        ResourceKind::MySql
    }
    fn test_connection(
        &self,
        resource: &AuthorizedResource<'_>,
    ) -> Result<ConnectionResult, ErrorCode> {
        resource.binding()?;
        self.connections.set(self.connections.get() + 1);
        Ok(ConnectionResult {
            latency_ms: 6,
            version_summary: "MySQL 8".into(),
        })
    }
}
impl FixtureCases {
    fn open(&self, resource: &AuthorizedResource<'_>) -> UseCaseResult<ConnectionResult> {
        self.check_reference(resource)?;
        Ok(self.test_connection(resource)?)
    }
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
        // Deliberately return the complete health catalogue. The application must filter this adapter output.
        let state = r.snapshot();
        Ok(state
            .targets
            .iter()
            .map(|t| VisibleEnvironment {
                target: t.alias.clone(),
                resources: state
                    .resources
                    .iter()
                    .filter(|v| v.target_id == t.id)
                    .map(|v| VisibleResource {
                        alias: v.alias.clone(),
                        health: Health::Unknown,
                    })
                    .collect(),
            })
            .collect())
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
        assert_eq!(resource.binding()?.id.0.to_string(), RESOURCE);
        Ok(executed(self.open(resource)?))
    }
    fn database_schema(
        &self,
        _: &Authorized<'_>,
        resource: &AuthorizedResource<'_>,
        table: Option<&TableName>,
    ) -> UseCaseResult<Vec<Table>> {
        self.enter()?;
        self.open(resource)?;
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
        resource: &AuthorizedResource<'_>,
        sql: &QueryText,
    ) -> UseCaseResult<Executed<QueryResult>> {
        self.enter()?;
        assert_eq!(sql.as_str(), "SELECT id FROM orders");
        self.open(resource)?;
        Ok(executed(QueryResult::new(
            vec!["id".into()],
            vec![vec![Cell::Number(1.into())]],
            false,
        )?))
    }
}
