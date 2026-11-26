//! Binding-scoped MCP over the process's stdin/stdout. The resolver belongs to
//! the local entry point; tool arguments and client metadata cannot replace it.
use envlume_application::{
    ErrorCode, Session,
    ports::{Repository, ToolchainInspector, UseCases},
    result::Report,
};
use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, DiscoverResult, Implementation,
        InitializeRequestParams, InitializeResult, ListToolsResult, PaginatedRequestParams,
        ServerCapabilities, ServerConfig, Tool,
    },
    service::{QuitReason, RequestContext},
};
use serde_json::{Value, json};

const NAME: &str = "project_context";

pub trait BoundProject: Send + Sync {
    fn visible(&self) -> Result<bool, ErrorCode>;
    fn call(&self, arguments: Value) -> Report;
    fn doctor_visible(&self) -> Result<bool, ErrorCode> {
        Ok(false)
    }
    fn doctor(&self, _: Value) -> Option<Report> {
        None
    }
}

/// A fresh use-case instance may be supplied for each request. The session keeps
/// the original binding and reloads its current grants on discovery and calls.
pub struct ProjectTool<R, F> {
    pub session: Session<R>,
    pub use_cases: F,
}
impl<R, F, C> BoundProject for ProjectTool<R, F>
where
    R: Repository + Send + Sync,
    F: Fn() -> C + Send + Sync,
    C: UseCases,
{
    fn visible(&self) -> Result<bool, ErrorCode> {
        self.session.can_inspect_project()
    }
    fn call(&self, arguments: Value) -> Report {
        self.session.invoke(&(self.use_cases)(), NAME, arguments)
    }
}

impl<R: Repository + Send + Sync> BoundProject for Session<R> {
    fn visible(&self) -> Result<bool, ErrorCode> {
        self.can_inspect_project()
    }
    fn call(&self, arguments: Value) -> Report {
        self.project_context(arguments)
    }
}

pub struct DiagnosticProject<R, I> {
    pub session: Session<R>,
    pub inspector: I,
}
impl<R: Repository + Send + Sync, I: ToolchainInspector + Send + Sync> BoundProject
    for DiagnosticProject<R, I>
{
    fn visible(&self) -> Result<bool, ErrorCode> {
        self.session.can_inspect_project()
    }
    fn call(&self, arguments: Value) -> Report {
        self.session.project_context(arguments)
    }
    fn doctor_visible(&self) -> Result<bool, ErrorCode> {
        self.session.can_inspect_toolchain()
    }
    fn doctor(&self, arguments: Value) -> Option<Report> {
        Some(self.session.toolchain_doctor(arguments, &self.inspector))
    }
}

pub type Resolution = Result<Box<dyn BoundProject>, ErrorCode>;
struct Server {
    project: Result<std::sync::Arc<dyn BoundProject>, ErrorCode>,
}

fn rejection(code: ErrorCode) -> ErrorData {
    // Never attach raw requests, SDK errors or paths to diagnostics.
    ErrorData::new(
        rmcp::model::ErrorCode(-32000),
        code.summary(),
        Some(json!({"errorCode":code})),
    )
}
impl Server {
    fn project(&self) -> Result<&dyn BoundProject, ErrorData> {
        self.project.as_deref().map_err(|code| rejection(*code))
    }
}
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("envlume", envlume_application::VERSION))
    }
    async fn discover(&self, _: RequestContext<RoleServer>) -> Result<DiscoverResult, ErrorData> {
        self.project()?.visible().map_err(rejection)?;
        Ok(DiscoverResult::from_server_info(
            self.supported_protocol_versions().into_owned(),
            self.get_info(),
        ))
    }
    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, ErrorData> {
        self.project()?.visible().map_err(rejection)?;
        context.peer.set_peer_info(request.clone());
        self.negotiate_initialize(&request)
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let visible = self.project()?.visible().map_err(rejection)?;
        let mut tools = if visible {
            let schema = serde_json::Map::from_iter([
                ("type".into(), json!("object")),
                ("properties".into(), json!({})),
                ("required".into(), json!([])),
                ("additionalProperties".into(), json!(false)),
            ]);
            vec![Tool::new(
                NAME,
                "Inspect the registered workspace context.",
                schema,
            )]
        } else {
            vec![]
        };
        if self.project()?.doctor_visible().map_err(rejection)? {
            let schema = serde_json::Map::from_iter([
                ("type".into(), json!("object")),
                (
                    "properties".into(),
                    json!({"profile":{"type":"string","minLength":1,"maxLength":64,"pattern":"^[a-z][a-z0-9_-]*$"}}),
                ),
                ("required".into(), json!(["profile"])),
                ("additionalProperties".into(), json!(false)),
            ]);
            tools.push(Tool::new(
                "toolchain_doctor",
                "Diagnose the selected local toolchain.",
                schema,
            ));
        }
        Ok(ListToolsResult::with_all_items(tools))
    }
    async fn call_tool(
        &self,
        params: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let project = self.project.as_ref().map_err(|e| rejection(*e))?;
        let arguments = Value::Object(params.arguments.unwrap_or_default());
        let report = match params.name.as_ref() {
            NAME => project.call(arguments),
            "toolchain_doctor" => {
                let project = project.clone();
                tokio::task::spawn_blocking(move || project.doctor(arguments))
                    .await
                    .map_err(|_| rejection(ErrorCode::InternalError))?
                    .ok_or_else(|| ErrorData::invalid_params("Unknown tool", None))?
            }
            _ => return Err(ErrorData::invalid_params("Unknown tool", None)),
        };
        let ok = report.ok;
        let value =
            serde_json::to_value(report).map_err(|_| rejection(ErrorCode::InternalError))?;
        let result = if ok {
            CallToolResult::structured(value)
        } else {
            CallToolResult::structured_error(value)
        };
        Ok(result.into())
    }
}

pub fn run_stdio(project: Resolution) -> Result<(), ErrorCode> {
    let startup_error = project.as_ref().err().copied();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(|_| ErrorCode::InternalError)?;
    eprintln!("Envlume MCP: starting STDIO");
    let result = runtime.block_on(async {
        let service = Server {
            project: project.map(std::sync::Arc::from),
        }
        .serve((tokio::io::stdin(), tokio::io::stdout()))
        .await
        .map_err(|_| {
            eprintln!("Envlume MCP: handshake failed");
            startup_error.unwrap_or(ErrorCode::InvalidInput)
        })?;
        match service.waiting().await {
            Ok(QuitReason::Closed | QuitReason::Cancelled) => startup_error.map_or(Ok(()), Err),
            _ => Err(ErrorCode::InternalError),
        }
    });
    // A blocking standard-input read may still be outstanding after a rejected
    // handshake. Exiting the independent process must not wait for another line.
    runtime.shutdown_background();
    result
}
