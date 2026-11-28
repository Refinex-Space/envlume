//! Local client configuration lifecycle. These operations confer no MCP capabilities.
use envlume_domain::{BindingId, ClientKind, IntegrationId};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Detect,
    Preview,
    PreviewUninstall,
    Install,
    Verify,
    Uninstall,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    pub id: IntegrationId,
    pub binding: BindingId,
    pub client: ClientKind,
    pub location: PathBuf,
    pub scope: String,
    pub project_key: Option<String>,
    pub fingerprint: String,
    pub status: String,
    pub original: Option<String>,
    pub published: Option<String>,
    pub pending: Option<String>,
    pub removal: Option<String>,
    pub source: Option<String>,
    pub verification: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfigChange {
    Add,
    Remove,
    #[serde(rename = "none")]
    Unchanged,
    Conflict,
    Edited,
    Recover,
}
impl ConfigChange {
    /// An identical unregistered entry is still foreign. A binding owns only its recorded fingerprint.
    pub fn for_entry(
        action: Action,
        registration: Option<&Registration>,
        actual: Option<&str>,
    ) -> Self {
        if registration.is_some_and(|r| r.pending.is_some()) {
            return Self::Recover;
        }
        let remove = matches!(action, Action::Uninstall | Action::PreviewUninstall);
        match (registration, actual) {
            (None, None) if remove => Self::Unchanged,
            (None, None) => Self::Add,
            (None, Some(_)) => Self::Conflict,
            (Some(r), Some(hash)) if r.fingerprint == hash => {
                if remove {
                    Self::Remove
                } else {
                    Self::Unchanged
                }
            }
            _ => Self::Edited,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Fault {
    InvalidInput,
    UntrustedBinding,
    ClientUnavailable,
    ClientUnsupported,
    ClientDisabled,
    ProjectScopeMismatch,
    ScopeConflict,
    CodexProjectNotLoaded,
    CodexConfigConflict,
    CliUnavailable,
    InvalidConfig,
    UnsafePath,
    NameConflict,
    EntryChanged,
    ConcurrentEdit,
    Cancelled,
    SharedScopeRequired,
    NotRegistered,
    VerificationFailed,
    ClientRejected,
    Busy,
    Timeout,
    Storage,
    RecoveryConflict,
}
impl Fault {
    pub fn exit(self) -> u8 {
        match self {
            Self::InvalidInput | Self::InvalidConfig => 2,
            Self::UntrustedBinding
            | Self::UnsafePath
            | Self::NameConflict
            | Self::EntryChanged
            | Self::ConcurrentEdit
            | Self::Cancelled
            | Self::CodexProjectNotLoaded
            | Self::CodexConfigConflict
            | Self::ScopeConflict
            | Self::ProjectScopeMismatch
            | Self::SharedScopeRequired
            | Self::NotRegistered
            | Self::ClientDisabled
            | Self::ClientRejected
            | Self::RecoveryConflict => 3,
            Self::ClientUnavailable | Self::ClientUnsupported | Self::CliUnavailable => 4,
            Self::Timeout | Self::Busy => 6,
            Self::Storage | Self::VerificationFailed => 7,
        }
    }
    pub fn message(self) -> &'static str {
        match self {
            Self::InvalidInput => "Check the integration operation, client and binding.",
            Self::UntrustedBinding => "Open the registered trusted workspace.",
            Self::CodexProjectNotLoaded => {
                "Open this workspace in Codex, review project trust, then reload its configuration and verify again."
            }
            Self::CodexConfigConflict => {
                "Review the effective envlume server in Codex. Its options or source differ from the registered project entry."
            }
            Self::ScopeConflict => {
                "Review the registered profile and scope before changing the integration."
            }
            Self::ProjectScopeMismatch => {
                "Claude Code resolves this directory to another project. Review the workspace or explicitly select project sharing."
            }
            Self::ClientUnsupported => {
                "Check the native client version and its support for project-scoped MCP configuration."
            }
            Self::ClientUnavailable => "Install the client or make its executable discoverable.",
            Self::CliUnavailable => "Install or repair the stable Envlume CLI.",
            Self::InvalidConfig => "Repair the project configuration and preview again.",
            Self::UnsafePath => "Check the project configuration path, links and permissions.",
            Self::NameConflict => "An unowned envlume entry already exists.",
            Self::EntryChanged => "The managed entry was edited. Review it locally.",
            Self::ConcurrentEdit => "Configuration changed since preview. Preview again.",
            Self::SharedScopeRequired => {
                "Repository .mcp.json requires --shared-project and explicit local confirmation."
            }
            Self::Cancelled => "Local confirmation is required.",
            Self::NotRegistered => "No managed entry is registered for this binding.",
            Self::VerificationFailed => {
                "The gateway could not verify the expected project context."
            }
            Self::ClientDisabled => {
                "Claude Code disabled this server for this project. Review it in /mcp."
            }
            Self::ClientRejected => "The client rejected the project configuration.",
            Self::Timeout => "Project context verification timed out.",
            Self::Busy => "Another integration transaction is running.",
            Self::Storage => "Check the private catalogue, backups and project write permissions.",
            Self::RecoveryConflict => {
                "Recovery requires local review of the changed configuration."
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Launch {
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verification {
    Gateway,
    ProjectConfig,
    Client,
}

/// A client adapter can attest actual client-side loading or report rejection.
/// The generic gateway probe can attest only STDIO and the registered project.
pub trait Adapter {
    fn detected(&self, client: ClientKind, root: &std::path::Path) -> bool;
    fn version(
        &self,
        _client: ClientKind,
        _root: &std::path::Path,
    ) -> Result<Option<String>, Fault> {
        Ok(None)
    }
    fn verify(
        &self,
        client: ClientKind,
        root: &std::path::Path,
        launch: &Launch,
        expected: &crate::workspaces::BindingInfo,
    ) -> Result<Verification, Fault>;
}

pub fn client_name(c: ClientKind) -> &'static str {
    match c {
        ClientKind::ClaudeCode => "claude-code",
        ClientKind::Codex => "codex",
        ClientKind::Cursor => "cursor",
    }
}
pub fn client_key(c: ClientKind) -> &'static str {
    match c {
        ClientKind::ClaudeCode => "CLAUDE_CODE",
        ClientKind::Codex => "CODEX",
        ClientKind::Cursor => "CURSOR",
    }
}
