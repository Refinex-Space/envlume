//! Local credential management. Native backends implement only write, status and cleanup
//! here. A connector receives credential material through a separately authorized port.
use crate::{
    ErrorCode,
    ports::{BindingSnapshot, Repository},
};
use envlume_domain::*;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Clone, PartialEq, Eq)]
pub struct ItemKey {
    target: TargetId,
    item: CredentialId,
}
impl ItemKey {
    fn fresh(target: TargetId) -> Self {
        Self {
            target,
            item: CredentialId::generate(),
        }
    }
    pub fn reference(&self) -> String {
        format!("envlume:v1:{}:{}", self.target.0, self.item.0)
    }
    pub fn parse(s: &str) -> Option<Self> {
        let (t, i) = s.strip_prefix("envlume:v1:")?.split_once(':')?;
        let key = Self {
            target: TargetId(t.parse().ok()?),
            item: CredentialId(i.parse().ok()?),
        };
        (key.reference() == s).then_some(key)
    }
    pub fn for_resource(r: &ResourceBinding) -> Result<Self, Fault> {
        let raw = r.credential_ref.as_ref().ok_or(Fault::NotSet)?;
        Self::parse(&raw.0)
            .filter(|key| key.target == r.target_id)
            .ok_or(Fault::Damaged)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Fault {
    NotSet,
    Missing,
    Damaged,
    AccessDenied,
    Unavailable,
    Changed,
    Untrusted,
    InvalidInput,
    Busy,
    Storage,
}
impl Fault {
    pub fn hint(self) -> &'static str {
        match self {
            Self::NotSet => "Set a credential for this resource.",
            Self::Missing | Self::Damaged => "Re-enter the credential with credential repair.",
            Self::AccessDenied => "Allow access in the operating system and retry.",
            Self::Unavailable => {
                "Unlock the credential store or sign in to a local user session and retry."
            }
            Self::Changed => "Inspect the resource again before confirming the change.",
            Self::Untrusted => "Verify the workspace and confirm the resource endpoint.",
            Self::InvalidInput => "Check the credential input and command arguments.",
            Self::Busy => "Another credential change is in progress. Retry.",
            Self::Storage => "Check access to the local Envlume catalogue and retry.",
        }
    }
    pub fn code(self) -> ErrorCode {
        match self {
            Self::Changed | Self::AccessDenied => ErrorCode::PolicyDenied,
            Self::Untrusted => ErrorCode::ProjectNotTrusted,
            Self::InvalidInput => ErrorCode::InvalidInput,
            Self::Busy => ErrorCode::Timeout,
            Self::Storage => ErrorCode::InternalError,
            _ => ErrorCode::CredentialUnavailable,
        }
    }
}

pub struct Selection {
    binding: BindingId,
    project: ProjectId,
    origin: PathBuf,
    target: EnvironmentTarget,
    resource: ResourceBinding,
}
impl Selection {
    pub fn binding(&self) -> BindingId {
        self.binding
    }
    pub fn project(&self) -> ProjectId {
        self.project
    }
    pub fn origin(&self) -> &Path {
        &self.origin
    }
    pub fn resource(&self) -> &ResourceBinding {
        &self.resource
    }
    pub fn target(&self) -> &EnvironmentTarget {
        &self.target
    }
    pub fn recheck(&self, state: &BindingSnapshot) -> Result<(), Fault> {
        if state.project.id != self.project
            || state.binding.id != self.binding
            || state.binding.project_id != self.project
            || state.project.deleted_at.is_some()
            || state.binding.state != BindingState::Trusted
            || !state.targets.iter().any(|t| t == &self.target)
            || !state.resources.iter().any(|r| r == &self.resource)
        {
            return Err(Fault::Changed);
        }
        Ok(())
    }
}
pub fn select(
    repo: &impl Repository,
    binding: BindingId,
    target: &Alias,
    resource: &Alias,
) -> Result<Selection, Fault> {
    let state = repo
        .load_binding(binding)
        .map_err(|e| match e {
            ErrorCode::Timeout => Fault::Busy,
            ErrorCode::InternalError => Fault::Storage,
            _ => Fault::Untrusted,
        })?
        .ok_or(Fault::Untrusted)?;
    if state.binding.id != binding
        || state.binding.state != BindingState::Trusted
        || state.binding.project_id != state.project.id
        || state.project.deleted_at.is_some()
    {
        return Err(Fault::Untrusted);
    }
    let t = state
        .targets
        .iter()
        .find(|t| {
            t.project_id == state.project.id && &t.alias == target && t.status == Status::Active
        })
        .ok_or(Fault::Untrusted)?;
    let r = state
        .resources
        .iter()
        .find(|r| {
            r.target_id == t.id
                && &r.alias == resource
                && r.endpoint_confirmed
                && r.status == Status::Active
        })
        .ok_or(Fault::Untrusted)?;
    Ok(Selection {
        binding,
        project: state.project.id,
        origin: state.binding.canonical_path,
        target: t.clone(),
        resource: r.clone(),
    })
}

pub trait Catalogue {
    type Lock;
    fn lock_credentials(&self) -> Result<Self::Lock, Fault>;
    fn verified_project(&self, binding: BindingId, origin: &Path) -> Result<ProjectId, Fault>;
    fn validate(&self, s: &Selection) -> Result<(), Fault>;
    fn stage(&self, s: &Selection, key: &ItemKey) -> Result<(), Fault>;
    fn publish(
        &self,
        s: &Selection,
        new: Option<&ItemKey>,
        remove_resource: bool,
    ) -> Result<(), Fault>;
    fn garbage(&self, project: ProjectId) -> Result<Vec<String>, Fault>;
    fn collected(&self, project: ProjectId, reference: &str) -> Result<(), Fault>;
}
pub trait CredentialWriter {
    type Input;
    fn write_new(&self, key: &ItemKey, input: &Self::Input) -> Result<(), Fault>;
    fn available(&self, key: &ItemKey) -> Result<(), Fault>;
    fn delete(&self, key: &ItemKey) -> Result<(), Fault>;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Set,
    Replace,
    Repair,
    Clear,
    RemoveResource,
}
pub fn status(db: &impl Catalogue, os: &impl CredentialWriter, s: &Selection) -> Result<(), Fault> {
    db.validate(s)?;
    let key = ItemKey::for_resource(&s.resource)?;
    os.available(&key)?;
    db.validate(s)
}
fn collect(
    db: &impl Catalogue,
    os: &impl CredentialWriter,
    project: ProjectId,
) -> Result<(), Fault> {
    for reference in db.garbage(project)? {
        if let Some(key) = ItemKey::parse(&reference) {
            match os.delete(&key) {
                Ok(()) | Err(Fault::Missing) => (),
                Err(e) => return Err(e),
            }
        }
        db.collected(project, &reference)?;
    }
    Ok(())
}
/// Retry cleanup after a crash or an operating-system denial without changing a resource.
pub fn cleanup(
    db: &impl Catalogue,
    os: &impl CredentialWriter,
    binding: BindingId,
    origin: &Path,
) -> Result<(), Fault> {
    let _lock = db.lock_credentials()?;
    let project = db.verified_project(binding, origin)?;
    collect(db, os, project)
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub cleanup_pending: bool,
}
pub fn change<P: CredentialWriter>(
    db: &impl Catalogue,
    os: &P,
    s: &Selection,
    action: Change,
    input: Option<&P::Input>,
) -> Result<Saved, Fault> {
    let _lock = db.lock_credentials()?;
    db.validate(s)?;
    match action {
        Change::Set if s.resource.credential_ref.is_some() => return Err(Fault::Changed),
        Change::Replace | Change::Repair if s.resource.credential_ref.is_none() => {
            return Err(Fault::NotSet);
        }
        _ => (),
    }
    match action {
        Change::Set | Change::Replace | Change::Repair => {
            let input = input.ok_or(Fault::InvalidInput)?;
            let key = ItemKey::fresh(s.target.id);
            db.stage(s, &key)?;
            os.write_new(&key, input)?;
            db.publish(s, Some(&key), false)?;
        }
        Change::Clear | Change::RemoveResource => {
            if input.is_some() {
                return Err(Fault::InvalidInput);
            }
            db.publish(s, None, action == Change::RemoveResource)?;
        }
    }
    // The reference switch has committed. A cleanup failure must not be reported
    // as a failed rotation; its durable journal entry remains retryable.
    Ok(Saved {
        cleanup_pending: collect(db, os, s.project).is_err(),
    })
}
