//! Implementations belong to local adapters. The repository reads a consistent
//! binding-scoped snapshot; it must not retain an authorization decision.
use crate::{
    input::{QueryText, TableName},
    result::*,
};
use envlume_domain::*;

#[derive(Clone)]
pub struct BindingSnapshot {
    pub project: Project,
    pub binding: WorkspaceBinding,
    pub targets: Vec<EnvironmentTarget>,
    pub resources: Vec<ResourceBinding>,
    pub tasks: Vec<TaskDefinition>,
    pub profiles: Vec<ToolchainProfile>,
    pub grants: Vec<CapabilityGrant>,
}

impl BindingSnapshot {
    pub fn has_grant(&self, capability: Capability, at: UtcDateTime) -> bool {
        self.grants.iter().any(|grant| {
            grant.binding_id == self.binding.id
                && grant.capability == capability
                && grant.revoked_at.is_none()
                && grant.expires_at.is_none_or(|end| end > at)
        })
    }
}

pub trait Repository {
    fn load_binding(&self, binding: BindingId) -> Result<Option<BindingSnapshot>, ErrorCode>;
}

impl<R: Repository + ?Sized> Repository for &R {
    fn load_binding(&self, binding: BindingId) -> Result<Option<BindingSnapshot>, ErrorCode> {
        (**self).load_binding(binding)
    }
}

/// A registered endpoint selected by an authorized remote operation. A caller
/// holding just an alias, a credential reference or a local record cannot mint it.
pub struct AuthorizedResource<'a> {
    pub(crate) binding: &'a ResourceBinding,
}
impl AuthorizedResource<'_> {
    pub fn binding(&self) -> &ResourceBinding {
        self.binding
    }
}

/// The provider checks the reference on the authorized resource locally.
pub trait SecretStore {
    fn check_reference(&self, resource: &AuthorizedResource<'_>) -> Result<(), ErrorCode>;
}
pub trait ResourceConnector {
    fn kind(&self) -> ResourceKind;
    fn test_connection(
        &self,
        resource: &AuthorizedResource<'_>,
    ) -> Result<ConnectionResult, ErrorCode>;
}
pub trait ToolchainInspector {
    fn diagnose(
        &self,
        workspace: &std::path::Path,
        profile: &ToolchainProfile,
    ) -> Result<Vec<ToolDiagnostic>, ErrorCode>;
}

pub struct UseCaseFailure {
    pub code: ErrorCode,
    pub evidence_id: Option<EvidenceId>,
}
impl From<ErrorCode> for UseCaseFailure {
    fn from(code: ErrorCode) -> Self {
        Self {
            code,
            evidence_id: None,
        }
    }
}
pub type UseCaseResult<T> = Result<T, UseCaseFailure>;

/// Created only after a fresh grant check. References expire at the end of this invocation.
pub struct Authorized<'a> {
    pub(crate) context: &'a crate::RequestContext,
    pub(crate) snapshot: &'a BindingSnapshot,
    pub(crate) capability: Capability,
}
impl Authorized<'_> {
    pub fn context(&self) -> &crate::RequestContext {
        self.context
    }
    pub fn snapshot(&self) -> &BindingSnapshot {
        self.snapshot
    }
    pub fn capability(&self) -> Capability {
        self.capability
    }
}

/// Trusted local implementations must filter visible metadata and sanitize summaries.
/// T02 only provides test doubles; no real connector or task executor is registered.
pub trait UseCases {
    fn project_context(&self, request: &Authorized<'_>) -> UseCaseResult<ProjectSummary>;
    fn environment_list(&self, request: &Authorized<'_>) -> UseCaseResult<Vec<VisibleEnvironment>>;
    fn toolchain_doctor(
        &self,
        request: &Authorized<'_>,
        profile: &ToolchainProfile,
    ) -> UseCaseResult<Vec<ToolDiagnostic>>;
    fn task_list(&self, request: &Authorized<'_>) -> UseCaseResult<Vec<NamedTask>>;
    fn task_run(
        &self,
        request: &Authorized<'_>,
        task: &TaskDefinition,
    ) -> UseCaseResult<Executed<TaskResult>>;
    fn connection_test(
        &self,
        request: &Authorized<'_>,
        resource: &AuthorizedResource<'_>,
    ) -> UseCaseResult<Executed<ConnectionResult>>;
    fn database_schema(
        &self,
        request: &Authorized<'_>,
        resource: &AuthorizedResource<'_>,
        table: Option<&TableName>,
    ) -> UseCaseResult<Vec<Table>>;
    fn database_query(
        &self,
        request: &Authorized<'_>,
        resource: &AuthorizedResource<'_>,
        sql: &QueryText,
    ) -> UseCaseResult<Executed<QueryResult>>;
}
