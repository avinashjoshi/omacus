//! `omacus mcp`: a stdio Model Context Protocol server.
//! Tools read and write the same pages as the desktop window.

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, Implementation, ListResourceTemplatesResult, ListResourcesResult,
    PaginatedRequestParams, ProtocolVersion, ReadResourceRequestParams, ReadResourceResponse,
    ReadResourceResult, Resource, ResourceContents, ResourceTemplate, ServerCapabilities,
    ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt, schemars, tool, tool_handler,
    tool_router, transport::stdio,
};
use serde::Deserialize;
use serde_json::json;

use crate::library::{self, Library};

#[derive(Clone)]
pub struct OmacusServer {
    /// Read by the `tool_router` macro.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EvaluateArgs {
    /// An expression, or a whole page with one calculation per line.
    text: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct NameArgs {
    /// A page title, its id, or a unique prefix of the title.
    name: String,
    /// Password for a locked page. Omit it for a page that is not locked.
    #[serde(default)]
    password: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CreateArgs {
    /// The page title.
    name: String,
    /// Lines to put under the title. Omit this for an empty page.
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AppendArgs {
    /// A page title, its id, or a unique prefix of the title.
    name: String,
    /// The line or lines to add at the end.
    text: String,
    /// Password for a locked page.
    #[serde(default)]
    password: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct UpdateArgs {
    /// A page title, its id, or a unique prefix of the title.
    name: String,
    /// The full new text of the page, replacing what is there.
    text: String,
    /// Password for a locked page.
    #[serde(default)]
    password: Option<String>,
}

const INSTRUCTIONS: &str = "\
Omacus is a notepad calculator. evaluate runs an expression or a whole page and \
returns each line's answer. list_pages, read_page, create_page, append_page and \
update_page use the same pages as the Omacus window. delete_page moves a page to \
Recently deleted; it does not erase it. A locked page is not returned or changed \
unless the tool is given the password. Page resources are omacus://page/{id}.";

#[tool_router]
impl OmacusServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Evaluate an expression or a whole page. Returns each line's answer.")]
    fn evaluate(&self, Parameters(args): Parameters<EvaluateArgs>) -> CallToolResult {
        let lines = library::evaluate(&args.text);
        let body = serde_json::to_string_pretty(&lines).unwrap_or_else(|_| "[]".into());
        if library::eval_status(&args.text, &lines) == 1 {
            tool_err(body)
        } else {
            tool_ok(body)
        }
    }

    #[tool(description = "List saved pages: id, title, and whether the page is locked.")]
    fn list_pages(&self) -> CallToolResult {
        let pages = Library::open().list();
        tool_ok(serde_json::to_string_pretty(&pages).unwrap_or_else(|_| "[]".into()))
    }

    #[tool(description = "Read one page, including its text and each line's answer.")]
    fn read_page(&self, Parameters(args): Parameters<NameArgs>) -> CallToolResult {
        match Library::open().show(&args.name, args.password.as_deref()) {
            Ok(view) => tool_ok(serde_json::to_string_pretty(&view).unwrap_or_default()),
            Err(err) => tool_err(err),
        }
    }

    #[tool(description = "Create a page. The title becomes its first line.")]
    fn create_page(&self, Parameters(args): Parameters<CreateArgs>) -> CallToolResult {
        match Library::open().create(&args.name, args.text.as_deref()) {
            Ok(page) => tool_ok(serde_json::to_string_pretty(&page).unwrap_or_default()),
            Err(err) => tool_err(err),
        }
    }

    #[tool(description = "Append lines to a page.")]
    fn append_page(&self, Parameters(args): Parameters<AppendArgs>) -> CallToolResult {
        match Library::open().append(&args.name, &args.text, args.password.as_deref()) {
            Ok(page) => tool_ok(serde_json::to_string_pretty(&page).unwrap_or_default()),
            Err(err) => tool_err(err),
        }
    }

    #[tool(description = "Replace a page's text with new text.")]
    fn update_page(&self, Parameters(args): Parameters<UpdateArgs>) -> CallToolResult {
        match Library::open().update(&args.name, &args.text, args.password.as_deref()) {
            Ok(page) => tool_ok(serde_json::to_string_pretty(&page).unwrap_or_default()),
            Err(err) => tool_err(err),
        }
    }

    #[tool(description = "Move a page to Recently deleted. This does not delete it forever.")]
    fn delete_page(&self, Parameters(args): Parameters<NameArgs>) -> CallToolResult {
        match Library::open().remove(&args.name, args.password.as_deref()) {
            Ok(title) => tool_ok(format!("Moved \"{title}\" to Recently deleted")),
            Err(err) => tool_err(err),
        }
    }
}

#[tool_handler]
impl ServerHandler for OmacusServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("omacus", env!("CARGO_PKG_VERSION")))
        .with_protocol_version(ProtocolVersion::V_2024_11_05)
        .with_instructions(INSTRUCTIONS.to_string())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let resources = Library::open()
            .list()
            .into_iter()
            .map(|page| {
                let mut resource = Resource::new(page_uri(&page.id), page.title);
                if page.locked {
                    resource = resource.with_description(crate::sheets::LOCKED_TITLE);
                }
                resource
            })
            .collect();
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let uri = request.uri.clone();
        let Some(id) = uri.strip_prefix("omacus://page/") else {
            return Err(McpError::resource_not_found(
                "unknown resource",
                Some(json!({ "uri": uri })),
            ));
        };
        if id.is_empty() {
            return Err(McpError::resource_not_found(
                "unknown resource",
                Some(json!({ "uri": uri })),
            ));
        }
        match Library::open().show(id, None) {
            Ok(view) => Ok(ReadResourceResult::new(vec![ResourceContents::text(
                view.rendered,
                request.uri,
            )])
            .into()),
            Err(err) => Err(McpError::resource_not_found(
                err,
                Some(json!({ "uri": uri })),
            )),
        }
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult::with_all_items(vec![
            ResourceTemplate::new("omacus://page/{id}", "page"),
        ]))
    }
}

fn page_uri(id: &str) -> String {
    format!("omacus://page/{id}")
}

fn tool_ok(text: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(text)])
}

fn tool_err(text: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(text)])
}

pub fn serve() -> i32 {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("omacus: {err}");
            return 1;
        }
    };
    let result = runtime.block_on(async {
        let service = OmacusServer::new()
            .serve(stdio())
            .await
            .map_err(|err| format!("{err}"))?;
        service.waiting().await.map_err(|err| format!("{err}"))?;
        Ok::<(), String>(())
    });
    match result {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("omacus: {err}");
            1
        }
    }
}
