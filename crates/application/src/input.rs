//! The protocol adapter supplies a tool name and a separate arguments object.
use crate::ErrorCode;
use envlume_domain::Alias;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Empty {}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DoctorInput {
    pub profile: Alias,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskInput {
    pub task_id: Alias,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceInput {
    pub target: Alias,
    pub resource: Alias,
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "String")]
pub struct TableName(String);
impl TableName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for TableName {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let b = value.as_bytes();
        if !(1..=64).contains(&b.len())
            || !(b[0].is_ascii_alphabetic() || b[0] == b'_')
            || !b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_')
        {
            return Err("invalid table selector");
        }
        Ok(Self(value))
    }
}

fn present_table<'de, D: Deserializer<'de>>(d: D) -> Result<Option<TableName>, D::Error> {
    TableName::deserialize(d).map(Some)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchemaInput {
    pub target: Alias,
    pub resource: Alias,
    #[serde(default, deserialize_with = "present_table")]
    pub table: Option<TableName>,
}

/// Bounded *untrusted* text. AST and object policy checks belong to T12 before execution.
#[derive(Deserialize)]
#[serde(try_from = "String")]
pub struct QueryText(String);
impl QueryText {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for QueryText {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.trim().is_empty() || value.chars().count() > 8192 {
            return Err("invalid SQL length");
        }
        Ok(Self(value))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryInput {
    pub target: Alias,
    pub resource: Alias,
    pub sql: QueryText,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCall {
    ProjectContext(Empty),
    EnvironmentList(Empty),
    ToolchainDoctor(DoctorInput),
    TaskList(Empty),
    TaskRun(TaskInput),
    ConnectionTest(ResourceInput),
    DatabaseSchema(SchemaInput),
    DatabaseQuery(QueryInput),
}

impl ToolCall {
    pub fn parse(name: &str, arguments: Value) -> Result<Self, ErrorCode> {
        if !arguments.is_object() {
            return Err(ErrorCode::InvalidInput);
        }
        let tagged = Value::Object([(name.to_owned(), arguments)].into_iter().collect());
        serde_json::from_value(tagged).map_err(|_| ErrorCode::InvalidInput)
    }
}

pub const TOOL_NAMES: [&str; 8] = [
    "project_context",
    "environment_list",
    "toolchain_doctor",
    "task_list",
    "task_run",
    "connection_test",
    "database_schema",
    "database_query",
];
