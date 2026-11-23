//! Local desktop/CLI project catalogue. MCP discovery stays binding-scoped.
use envlume_domain::ProjectId;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectEntry {
    pub id: ProjectId,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StorageError {
    UpgradeRequired,
    InvalidDatabase,
    StorageBusy,
    PrivateDirectoryRequired,
    StorageUnavailable,
    MigrationFailed,
    DestinationExists,
}

impl StorageError {
    pub fn message(self) -> &'static str {
        match self {
            Self::UpgradeRequired => "Upgrade Envlume before opening this database.",
            Self::InvalidDatabase => {
                "Unrecognized or damaged Envlume database. Open a verified backup in a fresh directory."
            }
            Self::StorageBusy => "Storage is busy. Retry after the other operation finishes.",
            Self::PrivateDirectoryRequired => {
                "Use a private Envlume data directory owned by the current user."
            }
            Self::StorageUnavailable => "Check local storage access and free space.",
            Self::MigrationFailed => {
                "Migration failed. The transaction was rolled back. Inspect the pre-upgrade backup."
            }
            Self::DestinationExists => "Restore into a new, empty data directory.",
        }
    }
}

pub trait ProjectRepository {
    fn active_projects(&self) -> Result<Vec<ProjectEntry>, StorageError>;
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectList {
    pub schema_version: u8,
    pub projects: Vec<ProjectEntry>,
}

pub fn list_projects(repository: &impl ProjectRepository) -> Result<ProjectList, StorageError> {
    Ok(ProjectList {
        schema_version: 1,
        projects: repository.active_projects()?,
    })
}
