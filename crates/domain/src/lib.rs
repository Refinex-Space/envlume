//! Local domain records. Only explicit application projections cross the wire.
mod identity;
pub use identity::*;
use std::{collections::HashSet, path::PathBuf, time::Duration};
pub use time::UtcDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Active,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub description: String,
    pub created_at: UtcDateTime,
    pub updated_at: UtcDateTime,
    pub deleted_at: Option<UtcDateTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingState {
    Untrusted,
    Trusted,
    Stale,
    Revoked(UtcDateTime),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceBinding {
    pub alias: Alias,
    pub id: BindingId,
    pub project_id: ProjectId,
    /// Local adapter canonicalizes and fingerprints the selected directory.
    pub canonical_path: PathBuf,
    pub git_fingerprint: Option<String>,
    pub profile_id: Option<ProfileId>,
    pub state: BindingState,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolRule {
    pub tool: Alias,
    pub version_requirement: String,
    pub selected_path: Option<PathBuf>,
    pub priority: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolchainProfile {
    pub id: ProfileId,
    pub project_id: ProjectId,
    pub name: Alias,
    pub rules: Vec<ToolRule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentKind {
    Dev,
    Test,
    Staging,
    Prod,
    Custom(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentTarget {
    pub id: TargetId,
    pub project_id: ProjectId,
    pub alias: Alias,
    pub kind: EnvironmentKind,
    pub risk: RiskLevel,
    pub is_default: bool,
    pub status: Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    MySql,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMode {
    VerifyIdentity,
    Disabled,
}

/// A local OS credential-store reference. Never part of an agent response.
#[derive(Clone, PartialEq, Eq)]
pub struct CredentialRef(pub String);
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MySqlRoute {
    pub ips: std::collections::BTreeSet<std::net::IpAddr>,
    pub ca_pem: Option<String>,
}
pub type TableScope = std::collections::BTreeMap<String, std::collections::BTreeSet<String>>;
#[derive(Clone, PartialEq, Eq)]
pub struct ResourceBinding {
    pub id: ResourceId,
    pub target_id: TargetId,
    pub alias: Alias,
    pub kind: ResourceKind,
    pub host: String,
    pub port: u16,
    pub tls: TlsMode,
    pub credential_ref: Option<CredentialRef>,
    pub endpoint_confirmed: bool,
    pub database: Option<String>,
    pub route: Option<MySqlRoute>,
    pub tables: TableScope,
    pub policy_labels: Vec<Alias>,
    pub status: Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskInput {
    Closed,
}

#[derive(Debug, Clone)]
pub struct TaskDefinition {
    pub id: TaskId,
    pub project_id: ProjectId,
    pub binding_id: Option<BindingId>,
    pub key: Alias,
    pub executable: String,
    pub arguments: Vec<String>,
    pub workspace_relative_dir: PathBuf,
    pub timeout: Duration,
    pub input: TaskInput,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceScope {
    pub target: TargetId,
    pub resource: ResourceId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    ProjectInspect,
    EnvironmentInspect,
    ToolchainInspect,
    TaskInspect,
    TaskRun(TaskId),
    ResourceConnect(ResourceScope),
    DatabaseSchema(ResourceScope),
    DatabaseRead(ResourceScope),
}

impl Capability {
    /// Stable wire spelling. Scoped permissions identify the registered object by UUID.
    pub fn wire_name(self) -> String {
        match self {
            Self::ProjectInspect => "project.inspect".into(),
            Self::EnvironmentInspect => "environment.inspect".into(),
            Self::ToolchainInspect => "toolchain.inspect".into(),
            Self::TaskInspect => "task.inspect".into(),
            Self::TaskRun(id) => format!("task.run:{}", id.0),
            Self::ResourceConnect(scope) => format!("resource.connect:{}", scope.resource.0),
            Self::DatabaseSchema(scope) => format!("database.schema:{}", scope.resource.0),
            Self::DatabaseRead(scope) => format!("database.read:{}", scope.resource.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityGrant {
    pub id: GrantId,
    pub binding_id: BindingId,
    pub capability: Capability,
    pub expires_at: Option<UtcDateTime>,
    pub revoked_at: Option<UtcDateTime>,
}

impl Project {
    /// Persist the project and affected records in a single repository transaction.
    pub fn soft_delete(
        &mut self,
        bindings: &mut [WorkspaceBinding],
        grants: &mut [CapabilityGrant],
        at: UtcDateTime,
    ) {
        if self.deleted_at.is_none() {
            self.deleted_at = Some(at);
            self.updated_at = at;
        }
        let mut affected = HashSet::new();
        for binding in bindings.iter_mut().filter(|b| b.project_id == self.id) {
            affected.insert(binding.id);
            if !matches!(binding.state, BindingState::Revoked(_)) {
                binding.state = BindingState::Revoked(at);
            }
        }
        for grant in grants
            .iter_mut()
            .filter(|g| affected.contains(&g.binding_id))
        {
            grant.revoked_at.get_or_insert(at);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    ClaudeCode,
    Codex,
    Cursor,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    Desktop,
    Cli,
    Mcp(ClientKind),
    /// Direct STDIO launch, without an authenticated client integration.
    LocalStdio,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionStatus {
    Success,
    Failed,
    Denied,
    Skipped,
}
#[derive(Debug, Clone)]
pub struct ExecutionRecord {
    pub id: ExecutionId,
    pub request_id: RequestId,
    pub evidence_id: Option<EvidenceId>,
    pub binding_id: BindingId,
    pub capability: Capability,
    pub actor: Actor,
    pub status: ExecutionStatus,
    pub sanitized_summary: String,
    pub exit_code: Option<i32>,
    pub duration: Duration,
    pub created_at: UtcDateTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrationStatus {
    Installed,
    Stale,
    Removed,
}
#[derive(Debug, Clone)]
pub struct ClientIntegration {
    pub id: IntegrationId,
    pub binding_id: BindingId,
    pub client: ClientKind,
    pub config_location: PathBuf,
    pub config_fingerprint: String,
    pub status: IntegrationStatus,
}
