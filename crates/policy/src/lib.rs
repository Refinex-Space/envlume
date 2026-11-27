//! Mandatory v0.1 ceiling, shared by grant issuance, discovery and dispatch.
use envlume_domain::{
    Capability, EnvironmentKind, EnvironmentTarget, ResourceBinding, ResourceScope, RiskLevel,
    Status,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteAction {
    Connect,
    Schema,
    Read,
}
impl RemoteAction {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "resource.connect" => Some(Self::Connect),
            "database.schema" => Some(Self::Schema),
            "database.read" => Some(Self::Read),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Connect => "resource.connect",
            Self::Schema => "database.schema",
            Self::Read => "database.read",
        }
    }
    pub fn scoped(self, scope: ResourceScope) -> Capability {
        match self {
            Self::Connect => Capability::ResourceConnect(scope),
            Self::Schema => Capability::DatabaseSchema(scope),
            Self::Read => Capability::DatabaseRead(scope),
        }
    }
    pub fn allowed(self, kind: &EnvironmentKind, risk: RiskLevel) -> bool {
        if risk == RiskLevel::Critical {
            return false;
        }
        match kind {
            EnvironmentKind::Dev | EnvironmentKind::Test => {
                self != Self::Read || risk != RiskLevel::High
            }
            EnvironmentKind::Staging => self != Self::Read,
            EnvironmentKind::Prod | EnvironmentKind::Custom(_) => false,
        }
    }
    pub fn permits(self, target: &EnvironmentTarget, resource: &ResourceBinding) -> bool {
        target.status == Status::Active
            && self.allowed(&target.kind, target.risk)
            && resource.target_id == target.id
            && resource.status == Status::Active
            && resource.endpoint_confirmed
    }
    pub fn from_capability(cap: Capability) -> Option<(Self, ResourceScope)> {
        match cap {
            Capability::ResourceConnect(s) => Some((Self::Connect, s)),
            Capability::DatabaseSchema(s) => Some((Self::Schema, s)),
            Capability::DatabaseRead(s) => Some((Self::Read, s)),
            _ => None,
        }
    }
}
