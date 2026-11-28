use crate::{
    claude,
    config::{self, Document, Edit},
    file::{Target, relative},
};
use envlume_application::{
    integrations::{Action, Fault, Registration},
    workspaces::BindingInfo,
};
use envlume_domain::ClientKind;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Layout {
    Project(ClientKind),
    Local {
        path: PathBuf,
        dir_id: String,
        project_key: String,
        client: Option<claude::Client>,
    },
}
impl Layout {
    pub fn resolve(
        s: &crate::Service<'_>,
        action: Action,
        root: &BindingInfo,
        row: Option<&Registration>,
    ) -> Result<Self, Fault> {
        let remove = matches!(action, Action::Uninstall | Action::PreviewUninstall);
        let use_record = remove || matches!(action, Action::Verify | Action::Detect);
        let shared =
            s.shared_project || use_record && row.is_some_and(|r| r.scope == "shared-project");
        if s.client != ClientKind::ClaudeCode || shared {
            if row.is_some_and(|r| r.scope == "local") {
                return Err(Fault::ScopeConflict);
            }
            return Ok(Self::Project(s.client));
        }
        if row.is_some_and(|r| r.scope != "local") {
            return Err(Fault::ScopeConflict);
        }
        let path = claude::profile_path(&root.canonical_path)?;
        if row.is_some_and(|r| r.location != path) {
            return Err(Fault::ScopeConflict);
        }
        let (_, dir_id) = Target::profile(path.parent().unwrap(), None)?;
        let client = if remove || row.is_some_and(|r| r.pending.is_some()) {
            None
        } else {
            match claude::Client::detect(&root.canonical_path) {
                Ok(c) => Some(c),
                Err(Fault::ClientUnavailable) => None,
                Err(e) => return Err(e),
            }
        };
        let registered = row.and_then(|r| r.project_key.as_deref());
        let key = if let Some(c) = &client {
            let observed = c.project_key(&root.canonical_path, &s.paths.root().join("tmp"))?;
            if registered.is_some_and(|k| k != observed) {
                return Err(Fault::ProjectScopeMismatch);
            }
            observed
        } else {
            registered
                .unwrap_or(root.canonical_path.to_str().ok_or(Fault::UnsafePath)?)
                .to_owned()
        };
        Ok(Self::Local {
            path,
            dir_id,
            project_key: key,
            client,
        })
    }
    pub fn location(&self, root: &BindingInfo) -> PathBuf {
        match self {
            Self::Project(c) => root.canonical_path.join(relative(*c)),
            Self::Local { path, .. } => path.clone(),
        }
    }
    pub fn scope(&self) -> &'static str {
        match self {
            Self::Project(ClientKind::ClaudeCode) => "shared-project",
            Self::Project(_) => "project",
            Self::Local { .. } => "local",
        }
    }
    pub fn project_key(&self) -> Option<&str> {
        match self {
            Self::Local { project_key, .. } => Some(project_key),
            _ => None,
        }
    }
    pub fn open(&self, root: &BindingInfo, create: bool) -> Result<Option<Target>, Fault> {
        match self {
            Self::Project(c) => Target::open(root, *c, create),
            Self::Local { path, dir_id, .. } => Ok(Some(
                Target::profile(path.parent().unwrap(), Some(dir_id))?.0,
            )),
        }
    }
    pub fn document(&self, before: Option<&[u8]>, edit: Edit<'_>) -> Result<Document, Fault> {
        match self {
            Self::Project(c) => config::document(*c, before, edit),
            Self::Local { project_key, .. } => config::local(before, project_key, edit),
        }
    }
    /// Prepare only an inverse insertion for a local registration. No pre-existing
    /// Claude field (including a third-party server in the same project) is needed.
    pub fn original_backup(&self, before: &[u8], after: &[u8]) -> Result<Vec<u8>, Fault> {
        if !matches!(self, Self::Local { .. }) {
            return Ok(before.to_vec());
        }
        let offset = before.iter().zip(after).take_while(|(a, b)| a == b).count();
        let length = after
            .len()
            .checked_sub(before.len())
            .ok_or(Fault::InvalidConfig)?;
        if after.get(offset + length..) != Some(&before[offset..]) || length == 0 {
            return Err(Fault::InvalidConfig);
        }
        serde_json::to_vec(&InverseInsertion { offset, length }).map_err(|_| Fault::Storage)
    }
    pub fn restore_backup(&self, current: &[u8], backup: Vec<u8>) -> Result<Vec<u8>, Fault> {
        if !matches!(self, Self::Local { .. }) {
            return Ok(backup);
        }
        let undo: InverseInsertion =
            serde_json::from_slice(&backup).map_err(|_| Fault::RecoveryConflict)?;
        let end = undo
            .offset
            .checked_add(undo.length)
            .filter(|v| *v <= current.len())
            .ok_or(Fault::RecoveryConflict)?;
        let mut restored = current.to_vec();
        restored.drain(undo.offset..end);
        Ok(restored)
    }
    pub fn change_backup(&self, current: &[u8], expected: &str) -> Result<Vec<u8>, Fault> {
        let Self::Local { project_key, .. } = self else {
            return Ok(current.to_vec());
        };
        let source = std::str::from_utf8(current).map_err(|_| Fault::InvalidConfig)?;
        config::owned_local_entry(source, project_key, expected)
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct InverseInsertion {
    offset: usize,
    length: usize,
}
