//! Bounded task evidence. Raw command input and child output never enter a record.
use envlume_application::{
    ErrorCode,
    result::{Payload, Report},
    tasks::Template,
};
use envlume_domain::{EvidenceId, RequestId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const RETENTION_DAYS: i64 = 30;

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Assertions {
    pub tests: u32,
    pub failures: u32,
    pub errors: u32,
    pub skipped: u32,
}
impl Assertions {
    /// Only accept Surefire's aggregate, without a test name, path or message.
    pub fn from_summary(line: &[u8]) -> Option<Self> {
        let line = std::str::from_utf8(line).ok()?;
        let s = line
            .strip_prefix("[INFO] ")
            .or_else(|| line.strip_prefix("[ERROR] "))?;
        let (tests, s) = s.strip_prefix("Tests run: ")?.split_once(", Failures: ")?;
        let (failures, s) = s.split_once(", Errors: ")?;
        let (errors, skipped) = s.split_once(", Skipped: ")?;
        Some(Self {
            tests: tests.parse().ok()?,
            failures: failures.parse().ok()?,
            errors: errors.parse().ok()?,
            skipped: skipped.parse().ok()?,
        })
    }
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputSummary {
    pub bytes: u64,
    pub truncated: bool,
    pub events: Vec<String>,
    pub assertions: Option<Assertions>,
}
impl std::fmt::Display for OutputSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} bytes", self.bytes)?;
        if self.truncated {
            write!(f, " (truncated)")?;
        }
        for event in &self.events {
            write!(f, ", {event}")?;
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunResult {
    pub schema_version: u8,
    pub request_id: RequestId,
    pub evidence_id: EvidenceId,
    pub task_id: String,
    pub timestamp_utc: String,
    pub duration_ms: u64,
    pub status: String,
    pub exit_code: Option<i32>,
    pub versions: BTreeMap<String, String>,
    pub stdout_summary: OutputSummary,
    pub stderr_summary: OutputSummary,
}

impl RunResult {
    pub fn tests_skipped(&self) -> bool {
        self.stdout_summary
            .assertions
            .as_ref()
            .or(self.stderr_summary.assertions.as_ref())
            .is_some_and(|a| a.skipped == a.tests && a.failures == 0 && a.errors == 0)
    }
}

pub struct Entry<'a> {
    pub report: &'a Report,
    pub template: Option<Template>,
    pub outcome: Option<&'a RunResult>,
}
impl Entry<'_> {
    pub fn decision(&self) -> &'static str {
        match (self.outcome, self.report.error_code) {
            (Some(r), _) => match r.status.as_str() {
                "success" if r.tests_skipped() => "SKIPPED",
                "success" => "SUCCESS",
                "cancelled" => "SKIPPED",
                _ => "FAILED",
            },
            (_, Some(ErrorCode::ToolchainMissing)) => "SKIPPED",
            (_, Some(code)) if code.exit_code() == 3 || code == ErrorCode::InvalidInput => "DENIED",
            (_, Some(_)) => "FAILED",
            _ => "SUCCESS",
        }
    }
    pub fn details(&self) -> Value {
        let r = self.outcome;
        let layer = match self.report.error_code {
            Some(ErrorCode::NetworkUnreachable) => "network",
            Some(ErrorCode::CredentialUnavailable) => "credential",
            Some(ErrorCode::QueryNotAllowed) => "policy",
            _ if self.report.resource_scope.is_some() => "service",
            Some(ErrorCode::ToolchainMissing) => "toolchain",
            Some(ErrorCode::ProjectNotTrusted | ErrorCode::BindingRevoked) => "workspace",
            Some(ErrorCode::PolicyDenied) if r.is_none() => "policy",
            Some(ErrorCode::InternalError) => "host",
            _ if self.report.operation == "toolchain_doctor" => "toolchain",
            _ => "workflow",
        };
        let tools = match &self.report.result {
            Some(Payload::Doctor { tools }) => Some(
                tools
                    .iter()
                    .map(|t| {
                        json!({
                            "tool":t.tool,"version":t.version,"repair":t.repair
                        })
                    })
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        };
        let diagnostic = match (self.report.error_code, r.map(|r| r.status.as_str())) {
            (_, Some("cancelled")) => {
                "Check caller connection and task authorization before retrying."
            }
            (Some(ErrorCode::ToolchainMissing), _) => {
                "Run envlume doctor for this binding and repair the selected profile."
            }
            (Some(ErrorCode::Timeout), _) => "Check the task time limit and the build duration.",
            (Some(ErrorCode::PolicyDenied), _) => {
                "Inspect the workspace grant and confirm the first build in a local terminal."
            }
            (Some(code), _) => code.summary(),
            (_, Some("failed")) => {
                "Inspect the local build or test report and fix the failing step."
            }
            _ if r.is_some_and(RunResult::tests_skipped) => {
                "Check the test selection and skip settings."
            }
            _ if tools
                .as_ref()
                .is_some_and(|tools| tools.iter().any(|v| !v["repair"].is_null())) =>
            {
                "Review the repair steps for the reported tools."
            }
            _ => "Completed",
        };
        let diagnostic = if self.report.resource_scope.is_some() {
            self.report.summary.as_str()
        } else {
            diagnostic
        };
        json!({
            "schemaVersion":1, "operation":self.report.operation, "diagnosticLayer":layer,
            "host":{"os":std::env::consts::OS,"architecture":std::env::consts::ARCH}, "tools":tools,
            "targetAlias":self.report.target_alias,
            "resourceId":self.report.resource_scope.map(|s|s.resource),"targetId":self.report.resource_scope.map(|s|s.target),
            "connection":match &self.report.result {Some(Payload::Connection(c))=>Some(c),_=>None},
            "query":match &self.report.result {Some(Payload::Query(q))=>Some(q.audit_summary()),_=>None},
            "metadataTableCount":match &self.report.result {Some(Payload::Schema{tables})=>Some(tables.len()),_=>None}, "remoteDiagnostic":self.report.resource_scope.map(|_|&self.report.summary),
            "taskId":self.template.map(Template::key), "workingDirectory":"<workspace>",
            "command":self.template.map(|t| [vec![t.executable()], t.arguments().to_vec()].concat()),
            "timestampUtc":self.report.timestamp_text(), "errorCode":self.report.error_code,
            "startedAtUtc":r.map(|v| &v.timestamp_utc),
            "versions":r.map(|v| &v.versions), "stdoutSummary":r.map(|v| &v.stdout_summary),
            "stderrSummary":r.map(|v| &v.stderr_summary), "diagnostic":diagnostic,
        })
    }
}
