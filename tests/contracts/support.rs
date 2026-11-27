//! Synthetic registry for application contract tests.
#[path = "fixture_cases.rs"]
mod fixture_cases;
use envlume_application::{ErrorCode, ports::*};
use envlume_domain::*;
pub use fixture_cases::*;
use std::{cell::RefCell, path::PathBuf, time::Duration};
pub fn alias(s: &str) -> Alias {
    s.to_owned().try_into().unwrap()
}
pub const TASK: &str = "30000000-0000-4000-8000-000000000001";

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
        endpoint_confirmed: true,
        route: None,
        tables: Default::default(),
        query_policy: None,
        database: Some("orders".into()),
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
