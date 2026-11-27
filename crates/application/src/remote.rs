//! Typed operations expose approved database reads. The connection driver never receives a caller SQL string.
use crate::{
    ErrorCode,
    input::TableName,
    ports::AuthorizedResource,
    result::{ConnectionResult, Health, Table},
};
use envlume_domain::{BindingId, GrantId, ResourceBinding, ResourceId, ResourceKind, TargetId};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PoolIdentity {
    pub binding: BindingId,
    pub target: TargetId,
    pub resource: ResourceId,
    pub grant: GrantId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteFault {
    Access(ErrorCode),
    Dns,
    Address,
    Tcp,
    Tls,
    Authentication,
    Privileges,
    Version,
    Service,
    Timeout,
    Limit,
    Busy,
}
impl RemoteFault {
    pub fn code(self) -> ErrorCode {
        match self {
            Self::Access(code) => code,
            Self::Dns | Self::Tcp | Self::Tls => ErrorCode::NetworkUnreachable,
            Self::Address => ErrorCode::TargetNotAllowed,
            Self::Authentication => ErrorCode::CredentialUnavailable,
            Self::Timeout | Self::Busy => ErrorCode::Timeout,
            Self::Service | Self::Version => ErrorCode::InternalError,
            Self::Privileges => ErrorCode::PolicyDenied,
            Self::Limit => ErrorCode::PolicyDenied,
        }
    }
    pub fn summary(self) -> &'static str {
        match self {
            Self::Access(code) => code.summary(),
            Self::Dns => "DNS resolution failed. Check the registered host and VPN DNS.",
            Self::Address => "No approved route. Review the registered endpoint and its IP policy.",
            Self::Tcp => {
                "TCP connection failed. Check service availability, routing, port and VPN."
            }
            Self::Tls => "TLS verification failed. Check the trusted CA and certificate identity.",
            Self::Authentication => {
                "MySQL rejected the login. Verify the account and its database privileges."
            }
            Self::Version => {
                "The MySQL service advertised an unsupported version. Verify the registered service."
            }
            Self::Privileges => "Check the MySQL read-only grants for this database.",
            Self::Service => {
                "MySQL did not complete the database operation. Check service health and grants."
            }
            Self::Timeout => "MySQL operation exceeded its deadline.",
            Self::Limit => {
                "The result exceeds local limits or contains unsupported data. Narrow the projection."
            }
            Self::Busy => {
                "The local connection pool limit was reached. Retry after current operations finish."
            }
        }
    }
}
pub use envlume_policy::query::{Parameter, ReadQuery};
pub struct AuthorizedQuery<'a> {
    pub(crate) resource: &'a AuthorizedResource<'a>,
    pub(crate) plan: ReadQuery,
}
impl<'a> AuthorizedQuery<'a> {
    pub fn resource(&self) -> &AuthorizedResource<'a> {
        self.resource
    }
    pub fn plan(&self) -> &ReadQuery {
        &self.plan
    }
}
pub trait DatabaseBackend: Send + Sync {
    fn kind(&self) -> ResourceKind;
    fn capabilities(&self) -> &'static [crate::environments::RemoteAction] {
        use crate::environments::RemoteAction::*;
        &[Connect, Schema]
    }
    fn test(&self, r: &AuthorizedResource<'_>) -> Result<ConnectionResult, RemoteFault>;
    fn schema(
        &self,
        r: &AuthorizedResource<'_>,
        table: Option<&TableName>,
    ) -> Result<Vec<Table>, RemoteFault>;
    fn query(&self, _: &AuthorizedQuery<'_>) -> Result<crate::result::QueryResult, RemoteFault> {
        Err(RemoteFault::Access(ErrorCode::QueryNotAllowed))
    }
    fn retain(&self, live: &dyn Fn(PoolIdentity, &ResourceBinding) -> bool);
    fn health(&self, binding: BindingId, resource: ResourceId) -> Health;
    fn close(&self) -> Result<(), RemoteFault>;
}
