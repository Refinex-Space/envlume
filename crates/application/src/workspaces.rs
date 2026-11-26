//! Local management use cases. Confirmation is supplied only by the trusted host.
use crate::projects::StorageError;
use envlume_domain::{BindingId, ProjectId};
use envlume_workspace::Workspace;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagementError {
    InvalidPath,
    ConfirmationRequired,
    WorkspaceChanged,
    NotFound,
    ProjectMismatch,
    Storage(StorageError),
}
impl From<StorageError> for ManagementError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}
impl ManagementError {
    pub fn diagnostic(self) -> (&'static str, &'static str, u8) {
        match self {
            Self::InvalidPath => (
                "INVALID_INPUT",
                "Select an existing supported local workspace path.",
                2,
            ),
            Self::NotFound => ("INVALID_INPUT", "Unknown project or binding.", 2),
            Self::ConfirmationRequired => (
                "POLICY_DENIED",
                "Explicit local terminal confirmation is required.",
                3,
            ),
            Self::WorkspaceChanged => (
                "PROJECT_NOT_TRUSTED",
                "Workspace identity changed; inspect and confirm again.",
                3,
            ),
            Self::ProjectMismatch => (
                "POLICY_DENIED",
                "The directory already belongs to another project.",
                3,
            ),
            Self::Storage(error) => (
                "STORAGE_ERROR",
                error.message(),
                if error == StorageError::StorageBusy {
                    6
                } else {
                    7
                },
            ),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingInfo {
    pub binding_id: BindingId,
    pub project_id: ProjectId,
    pub project: String,
    pub alias: String,
    pub entry_path: PathBuf,
    pub canonical_path: PathBuf,
    pub state: String,
    pub confirmation_source: Option<String>,
    pub confirmed_at_ms: Option<i64>,
    #[serde(skip)]
    pub fingerprint: Option<String>,
    #[serde(skip)]
    pub root_identity: Option<String>,
}

impl BindingInfo {
    pub fn matches(&self, observation: &Workspace) -> bool {
        self.confirmation_source.as_deref() == Some("LOCAL_CLI_TTY")
            && self.confirmed_at_ms.is_some()
            && self.canonical_path == observation.canonical_path
            && self.fingerprint.as_ref() == Some(&observation.fingerprint)
            && self.root_identity.as_ref() == Some(&observation.root_identity)
    }
}

pub trait WorkspaceRepository {
    fn register_confirmed(
        &self,
        workspace: &Workspace,
        project: Option<ProjectId>,
    ) -> Result<BindingInfo, ManagementError>;
    fn inspect_binding(
        &self,
        entry: &Path,
        observation: Option<&Workspace>,
    ) -> Result<BindingInfo, ManagementError>;
    fn revoke(&self, id: BindingId) -> Result<(), ManagementError>;
    fn delete_project(&self, id: ProjectId) -> Result<(), ManagementError>;
}

pub fn add_project(
    repo: &impl WorkspaceRepository,
    path: &Path,
    project: Option<ProjectId>,
    confirm: impl FnOnce(&Workspace, Option<ProjectId>) -> bool,
) -> Result<BindingInfo, ManagementError> {
    let before = envlume_workspace::discover(path).map_err(|_| ManagementError::InvalidPath)?;
    if !confirm(&before, project) {
        return Err(ManagementError::ConfirmationRequired);
    }
    let after = envlume_workspace::discover(&before.entry_path)
        .map_err(|_| ManagementError::WorkspaceChanged)?;
    if before != after {
        return Err(ManagementError::WorkspaceChanged);
    }
    repo.register_confirmed(&after, project)
}

#[derive(Serialize)]
pub struct Inspection {
    pub binding: BindingInfo,
    pub workspace: Option<Workspace>,
}
pub fn inspect_project(
    repo: &impl WorkspaceRepository,
    path: &Path,
) -> Result<Inspection, ManagementError> {
    let entry = envlume_workspace::entry_path(path).map_err(|_| ManagementError::InvalidPath)?;
    let workspace = envlume_workspace::discover(&entry).ok();
    let binding = repo.inspect_binding(&entry, workspace.as_ref())?;
    Ok(Inspection { binding, workspace })
}
