use envlume_domain::{Alias, EvidenceId, RequestId, UtcDateTime};
use serde::Serialize;
use time::format_description::well_known::Rfc3339;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidInput,
    ProjectNotTrusted,
    BindingRevoked,
    TargetNotAllowed,
    PolicyDenied,
    QueryNotAllowed,
    ToolchainMissing,
    CredentialUnavailable,
    NetworkUnreachable,
    Timeout,
    InternalError,
}
impl ErrorCode {
    pub fn exit_code(self) -> u8 {
        match self {
            Self::InvalidInput => 2,
            Self::ProjectNotTrusted
            | Self::BindingRevoked
            | Self::TargetNotAllowed
            | Self::PolicyDenied
            | Self::QueryNotAllowed => 3,
            Self::ToolchainMissing | Self::CredentialUnavailable => 4,
            Self::NetworkUnreachable => 5,
            Self::Timeout => 6,
            Self::InternalError => 7,
        }
    }
    pub fn summary(self) -> &'static str {
        match self {
            Self::InvalidInput => "Check the tool name and arguments",
            Self::ProjectNotTrusted => "Verify workspace trust",
            Self::BindingRevoked => "Register the workspace again",
            Self::TargetNotAllowed => "Select an authorized target and resource",
            Self::PolicyDenied => "Review the workspace grant",
            Self::QueryNotAllowed => "Use an allowed read-only query",
            Self::ToolchainMissing => "Select or install the required toolchain",
            Self::CredentialUnavailable => "Repair the local credential reference",
            Self::NetworkUnreachable => "Check the connection and network policy",
            Self::Timeout => "Check the operation time limit",
            Self::InternalError => "Inspect the local diagnostic record",
        }
    }
}

// These projections contain only the fields an adapter can return to a caller.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub project: String,
    pub workspace: String,
    pub targets: Vec<Alias>,
    pub capabilities: Vec<String>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    Unknown,
    Healthy,
    Unavailable,
}
#[derive(Debug, Serialize)]
pub struct VisibleResource {
    pub alias: Alias,
    pub health: Health,
}
#[derive(Debug, Serialize)]
pub struct VisibleEnvironment {
    pub target: Alias,
    pub resources: Vec<VisibleResource>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDiagnostic {
    pub tool: Alias,
    pub version: Option<String>,
    pub path_summary: Option<String>,
    pub repair: Option<String>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedTask {
    pub task_id: Alias,
    pub timeout_seconds: u32,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskResult {
    pub exit_code: i32,
    pub stdout_summary: String,
    pub stderr_summary: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionResult {
    pub latency_ms: u64,
    pub version_summary: String,
}
#[derive(Debug, Serialize)]
pub struct Column {
    pub name: String,
    #[serde(rename = "type")]
    pub sql_type: String,
    pub nullable: bool,
}
#[derive(Debug, Serialize)]
pub struct Index {
    pub name: String,
    pub columns: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct Table {
    pub table: String,
    pub columns: Vec<Column>,
    pub indexes: Vec<Index>,
}
/// Sanitized scalar cell; nested objects cannot introduce new response fields.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Cell {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    Text(String),
}
#[derive(Debug, Serialize)]
pub struct QueryResult {
    columns: Vec<String>,
    rows: Vec<Vec<Cell>>,
    truncated: bool,
}
impl QueryResult {
    pub fn new(
        columns: Vec<String>,
        rows: Vec<Vec<Cell>>,
        truncated: bool,
    ) -> Result<Self, ErrorCode> {
        if rows.len() > 200 || rows.iter().any(|row| row.len() != columns.len()) {
            return Err(ErrorCode::InternalError);
        }
        Ok(Self {
            columns,
            rows,
            truncated,
        })
    }
}

/// The executor supplies the ID of the actual evidence record.
pub struct Executed<T> {
    pub evidence_id: EvidenceId,
    pub result: T,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Payload {
    Project(ProjectSummary),
    Environments {
        environments: Vec<VisibleEnvironment>,
    },
    Doctor {
        tools: Vec<ToolDiagnostic>,
    },
    Tasks {
        tasks: Vec<NamedTask>,
    },
    Task(TaskResult),
    Connection(ConnectionResult),
    Schema {
        tables: Vec<Table>,
    },
    Query(QueryResult),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportStatus {
    Success,
    Failed,
    Denied,
}

fn utc<S: serde::Serializer>(value: &UtcDateTime, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&value.format(&Rfc3339).map_err(serde::ser::Error::custom)?)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding_alias: Option<Alias>,
    pub(crate) schema_version: u8,
    pub request_id: RequestId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_id: Option<EvidenceId>,
    #[serde(serialize_with = "utc")]
    pub timestamp_utc: UtcDateTime,
    pub operation: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_alias: Option<Alias>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    pub duration_ms: u64,
    pub status: ReportStatus,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<ErrorCode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Payload>,
}
impl Report {
    pub fn new(operation: &'static str) -> Self {
        Self {
            schema_version: 1,
            request_id: RequestId::generate(),
            timestamp_utc: UtcDateTime::now(),
            duration_ms: 0,
            ok: true,
            operation,
            binding_alias: None,
            target_alias: None,
            capability: None,
            evidence_id: None,
            status: ReportStatus::Success,
            summary: "Completed".into(),
            error_code: None,
            result: None,
        }
    }

    pub fn fail(&mut self, code: ErrorCode, message: &str) {
        self.ok = false;
        self.status = if code.exit_code() == 3 {
            ReportStatus::Denied
        } else {
            ReportStatus::Failed
        };
        self.error_code = Some(code);
        self.result = None;
        self.summary = message.into();
    }

    pub fn exit_code(&self) -> u8 {
        self.error_code.map_or(0, ErrorCode::exit_code)
    }
    pub fn human_text(&self) -> String {
        let mut text = format!(
            "{}: {}\nrequestId: {}",
            self.operation, self.summary, self.request_id.0
        );
        if let Some(Payload::Task(task)) = &self.result {
            text.push_str(&format!("\nexitCode: {}", task.exit_code));
        }
        if let Some(id) = self.evidence_id {
            text.push_str(&format!("\nevidenceId: {}", id.0));
        }
        text
    }
}
