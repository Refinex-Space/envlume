use crate::{
    ports::BindingSnapshot,
    result::{Health, VisibleEnvironment, VisibleResource},
};
use envlume_domain::*;
pub use envlume_policy::RemoteAction;

pub fn visible(state: &BindingSnapshot) -> Vec<VisibleEnvironment> {
    visible_with_health(state, &|_| Health::Unknown)
}
pub(crate) fn visible_with_health(
    state: &BindingSnapshot,
    health: &dyn Fn(ResourceId) -> Health,
) -> Vec<VisibleEnvironment> {
    let now = UtcDateTime::now();
    let mut visible = Vec::new();
    for t in &state.targets {
        if t.project_id != state.project.id || t.status != Status::Active {
            continue;
        }
        let mut resources: Vec<_> = state
            .resources
            .iter()
            .filter(|r| {
                [
                    RemoteAction::Connect,
                    RemoteAction::Schema,
                    RemoteAction::Read,
                ]
                .iter()
                .any(|a| {
                    a.permits(t, r)
                        && state.has_grant(
                            a.scoped(ResourceScope {
                                target: t.id,
                                resource: r.id,
                            }),
                            now,
                        )
                })
            })
            .map(|r| VisibleResource {
                alias: r.alias.clone(),
                health: health(r.id),
            })
            .collect();
        if resources.is_empty() {
            continue;
        }
        resources.sort_by(|a, b| a.alias.as_str().cmp(b.alias.as_str()));
        visible.push(VisibleEnvironment {
            target: t.alias.clone(),
            resources,
        });
    }
    visible.sort_by(|a, b| a.target.as_str().cmp(b.target.as_str()));
    visible
}

use crate::{ErrorCode, ports::Repository};
use std::path::Path;

#[derive(Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    pub database: Option<String>,
    pub tls: TlsMode,
}
impl Endpoint {
    fn from_resource(r: &ResourceBinding) -> Self {
        Self {
            host: r.host.clone(),
            port: r.port,
            database: r.database.clone(),
            tls: r.tls,
        }
    }
    fn validate(&self) -> bool {
        let host = &self.host;
        let valid_host = host.parse::<std::net::IpAddr>().is_ok()
            || host.trim_end_matches('.').split('.').all(|s| {
                !s.is_empty()
                    && s.len() <= 63
                    && !s.starts_with('-')
                    && !s.ends_with('-')
                    && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            });
        host.len() <= 253
            && !host.ends_with("..")
            && valid_host
            && self.port > 0
            && self.database.as_ref().is_none_or(|s| {
                !s.is_empty()
                    && s.len() <= 64
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'$'))
            })
    }
    fn display(&self) -> String {
        format!(
            "host={} port={} database={} TLS={:?}",
            self.host,
            self.port,
            self.database.as_deref().unwrap_or("(none)"),
            self.tls
        )
    }
}
pub enum Edit {
    Target {
        alias: Alias,
        kind: EnvironmentKind,
        risk: RiskLevel,
    },
    Rename {
        alias: Alias,
        to: Alias,
    },
    Resource {
        target: Alias,
        alias: Alias,
        endpoint: Endpoint,
    },
    QueryPolicy {
        target: Alias,
        alias: Alias,
        policy: QueryPolicy,
    },
    MySqlPolicy {
        target: Alias,
        alias: Alias,
        route: MySqlRoute,
        tables: TableScope,
    },
    Grant {
        target: Alias,
        alias: Alias,
        action: RemoteAction,
        expires_at: Option<UtcDateTime>,
    },
    Revoke {
        target: Alias,
        alias: Alias,
        action: RemoteAction,
    },
}
impl Edit {
    pub fn preset(name: Alias) -> Result<Self, ErrorCode> {
        let (kind, risk) = match name.as_str() {
            "dev" => (EnvironmentKind::Dev, RiskLevel::Low),
            "test" => (EnvironmentKind::Test, RiskLevel::Medium),
            "staging" => (EnvironmentKind::Staging, RiskLevel::High),
            "prod" => (EnvironmentKind::Prod, RiskLevel::Critical),
            _ => return Err(ErrorCode::InvalidInput),
        };
        Ok(Self::Target {
            alias: name,
            kind,
            risk,
        })
    }

    fn selectors(&self) -> (&Alias, Option<&Alias>) {
        match self {
            Self::Target { alias, .. } | Self::Rename { alias, .. } => (alias, None),
            Self::QueryPolicy { target, alias, .. }
            | Self::MySqlPolicy { target, alias, .. }
            | Self::Resource { target, alias, .. }
            | Self::Grant { target, alias, .. }
            | Self::Revoke { target, alias, .. } => (target, Some(alias)),
        }
    }
}

// What the user reviewed. Recompute inside the write transaction before applying.
#[derive(PartialEq, Eq)]
struct Reviewed {
    project: ProjectId,
    binding: BindingId,
    target: Option<EnvironmentTarget>,
    resource: Option<ResourceBinding>,
    grants: Vec<CapabilityGrant>,
}
impl Reviewed {
    fn read(state: &BindingSnapshot, edit: &Edit) -> Result<Self, ErrorCode> {
        if state.binding.project_id != state.project.id
            || state.project.deleted_at.is_some()
            || state.binding.state != BindingState::Trusted
        {
            return Err(ErrorCode::ProjectNotTrusted);
        }
        let (alias, resource_alias) = edit.selectors();
        let target = state
            .targets
            .iter()
            .find(|t| t.project_id == state.project.id && &t.alias == alias);
        if target.is_some_and(|t| t.status != Status::Active) {
            return Err(ErrorCode::TargetNotAllowed);
        }
        let resource = target.and_then(|t| {
            state
                .resources
                .iter()
                .find(|r| r.target_id == t.id && Some(&r.alias) == resource_alias)
        });
        let mut grants = Vec::new();
        match edit {
            Edit::Target { kind, risk, .. } => {
                if matches!(kind, EnvironmentKind::Custom(_)) {
                    return Err(ErrorCode::InvalidInput);
                }
                if let Some(t) = target {
                    // Classification may tighten. Renaming never changes semantic kind or risk.
                    if rank_kind(kind) < rank_kind(&t.kind) || rank_risk(*risk) < rank_risk(t.risk)
                    {
                        return Err(ErrorCode::PolicyDenied);
                    }
                }
            }
            Edit::Rename { to, .. } => {
                if target.is_none()
                    || state.targets.iter().any(|t| {
                        t.project_id == state.project.id && t.alias == *to && &t.alias != alias
                    })
                {
                    return Err(ErrorCode::InvalidInput);
                }
            }
            Edit::QueryPolicy { policy, .. } => {
                let r = resource
                    .filter(|r| {
                        r.endpoint_confirmed && r.route.is_some() && r.status == Status::Active
                    })
                    .ok_or(ErrorCode::TargetNotAllowed)?;
                if !policy.valid_for(&r.tables) {
                    return Err(ErrorCode::InvalidInput);
                }
            }
            Edit::MySqlPolicy { route, tables, .. } => {
                let r = resource
                    .filter(|r| {
                        r.status == Status::Active && r.endpoint_confirmed && r.database.is_some()
                    })
                    .ok_or(ErrorCode::TargetNotAllowed)?;
                crate::mysql_policy::validate(r, route, tables)?;
            }
            Edit::Resource { endpoint, .. } => {
                if !endpoint.validate() {
                    return Err(ErrorCode::InvalidInput);
                }
                if target.is_none() || resource.is_some_and(|r| r.status != Status::Active) {
                    return Err(ErrorCode::TargetNotAllowed);
                }
            }
            Edit::Grant { action, .. } | Edit::Revoke { action, .. } => {
                let t = target.ok_or(ErrorCode::TargetNotAllowed)?;
                let r = resource.ok_or(ErrorCode::TargetNotAllowed)?;
                if let Edit::Grant { expires_at, .. } = edit {
                    if !action.permits(t, r) {
                        return Err(ErrorCode::TargetNotAllowed);
                    }
                    if expires_at.is_some_and(|e| e <= UtcDateTime::now()) {
                        return Err(ErrorCode::InvalidInput);
                    }
                }
                let cap = action.scoped(ResourceScope {
                    target: t.id,
                    resource: r.id,
                });
                grants = state
                    .grants
                    .iter()
                    .filter(|g| g.binding_id == state.binding.id && g.capability == cap)
                    .cloned()
                    .collect();
            }
        }
        Ok(Self {
            project: state.project.id,
            binding: state.binding.id,
            target: target.cloned(),
            resource: resource.cloned(),
            grants,
        })
    }
}
fn rank_kind(k: &EnvironmentKind) -> u8 {
    match k {
        EnvironmentKind::Dev | EnvironmentKind::Test => 0,
        EnvironmentKind::Staging => 1,
        EnvironmentKind::Prod | EnvironmentKind::Custom(_) => 2,
    }
}
fn rank_risk(r: RiskLevel) -> u8 {
    match r {
        RiskLevel::Low => 0,
        RiskLevel::Medium => 1,
        RiskLevel::High => 2,
        RiskLevel::Critical => 3,
    }
}

pub struct ConfirmedEdit {
    edit: Edit,
    reviewed: Reviewed,
}
impl ConfirmedEdit {
    pub fn edit(&self) -> &Edit {
        &self.edit
    }
    pub fn recheck(&self, state: &BindingSnapshot) -> Result<(), ErrorCode> {
        if Reviewed::read(state, &self.edit)? != self.reviewed {
            return Err(ErrorCode::PolicyDenied);
        }
        Ok(())
    }
    pub fn target(&self) -> Option<&EnvironmentTarget> {
        self.reviewed.target.as_ref()
    }
    pub fn resource(&self) -> Option<&ResourceBinding> {
        self.reviewed.resource.as_ref()
    }
}
pub trait EnvironmentWriter {
    fn apply(
        &self,
        binding: BindingId,
        origin: &Path,
        edit: ConfirmedEdit,
    ) -> Result<(), ErrorCode>;
}
pub fn manage(
    repository: &impl Repository,
    writer: &impl EnvironmentWriter,
    binding: BindingId,
    origin: &Path,
    edit: Edit,
    confirm: impl FnOnce(&str) -> bool,
) -> Result<(), ErrorCode> {
    let state = repository
        .load_binding(binding)?
        .filter(|s| s.binding.id == binding)
        .ok_or(ErrorCode::ProjectNotTrusted)?;
    let reviewed = Reviewed::read(&state, &edit)?;
    let (target, resource) = edit.selectors();
    let mut prompt = format!(
        "Project {}, binding {}, target {}",
        state.project.id.0,
        binding.0,
        target.as_str()
    );
    if let Some(t) = &reviewed.target {
        prompt.push_str(&format!(" ({:?}, {:?})", t.kind, t.risk));
    }
    if let Some(r) = resource {
        prompt.push_str(&format!(", resource {}", r.as_str()));
    }
    let action = match &edit {
        Edit::Target { kind, risk, .. } => format!(
            "Set target kind={kind:?} risk={risk:?}. Classification changes revoke its remote grants."
        ),
        Edit::QueryPolicy { policy, .. } => format!(
            "Approve MySQL query policy {policy:?}. Changes revoke database.read for every binding of this resource."
        ),
        Edit::Rename { to, .. } => format!("Rename target to {}.", to.as_str()),
        Edit::MySqlPolicy { route, tables, .. } => {
            let r = reviewed
                .resource
                .as_ref()
                .ok_or(ErrorCode::TargetNotAllowed)?;
            format!(
                "Approve MySQL {}:{} {:?}, database {:?}, dial IPs {:?}, CA SHA-256 {}, tables and columns {:?}. Route changes detach credentials; policy changes revoke grants.",
                r.host,
                r.port,
                r.tls,
                r.database,
                route.ips,
                crate::mysql_policy::ca_summary(route),
                tables
            )
        }
        Edit::Resource { endpoint, .. } => {
            let existing = reviewed.resource.as_ref().map(Endpoint::from_resource);
            format!(
                "Confirm MySQL endpoint {} (previous: {}). Endpoint changes revoke its grants and detach its credential reference.",
                endpoint.display(),
                existing
                    .as_ref()
                    .map(Endpoint::display)
                    .unwrap_or_else(|| "unregistered".into())
            )
        }
        Edit::Grant { action, .. } | Edit::Revoke { action, .. } => {
            let r = reviewed
                .resource
                .as_ref()
                .ok_or(ErrorCode::TargetNotAllowed)?;
            let endpoint = Endpoint::from_resource(r);
            match &edit {
                Edit::Grant {
                    expires_at,
                    action: RemoteAction::Read,
                    ..
                } => format!(
                    "Grant environment.inspect and database.read for {}, expiry {}. Approved query policy: {:?}.",
                    endpoint.display(),
                    expires_at.map_or("none".to_owned(), |v| v.to_string()),
                    r.query_policy
                ),
                Edit::Grant { expires_at, .. } => format!(
                    "Grant environment.inspect and {} for {}, expiry {}.",
                    action.name(),
                    endpoint.display(),
                    expires_at.map_or("none".to_owned(), |v| v.to_string())
                ),
                _ => format!("Revoke {} for {}.", action.name(), endpoint.display()),
            }
        }
    };
    if !confirm(&format!("{prompt}\n{action}")) {
        return Err(ErrorCode::PolicyDenied);
    }
    writer.apply(binding, origin, ConfirmedEdit { edit, reviewed })
}
