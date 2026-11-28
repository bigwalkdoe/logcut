//! MCP stdio server: JSON-RPC on stdout, diagnostics only on stderr.

use std::path::Path;

use rmcp::{
    Json, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router,
};

use crate::core::{
    self,
    models::{
        LastCrashRequest, SliceBuildArgs, SliceBuildReport, SliceContainerRequest, SliceLogRequest,
        SliceReport, SliceServiceRequest, SliceTestArgs, SliceTestReport,
    },
};

/// Tool handler shared by every MCP session.
#[derive(Debug, Clone)]
pub struct LogcutServer {
    tool_router: ToolRouter<Self>,
}

impl Default for LogcutServer {
    fn default() -> Self {
        Self::new()
    }
}

impl LogcutServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl LogcutServer {
    #[tool(
        name = "slice_log",
        description = "Redact, deduplicate and slice a log file, piped text, or raw log string"
    )]
    async fn slice_log(
        &self,
        Parameters(request): Parameters<SliceLogRequest>,
    ) -> Result<Json<SliceReport>, String> {
        core::slice_log(request)
            .map(Json)
            .map_err(|error| error.to_string())
    }

    #[tool(
        name = "slice_test",
        description = "Strip passing test lines and slice failing tests with source context"
    )]
    async fn slice_test(
        &self,
        Parameters(request): Parameters<SliceTestArgs>,
    ) -> Result<Json<SliceTestReport>, String> {
        core::slice_test(request)
            .map(Json)
            .map_err(|error| error.to_string())
    }

    #[tool(
        name = "get_last_crash",
        description = "Read the last crash report saved by `logcut watch`"
    )]
    async fn get_last_crash(
        &self,
        Parameters(request): Parameters<LastCrashRequest>,
    ) -> Result<Json<String>, String> {
        let root = request.workspace_root.as_deref().unwrap_or(".");
        core::watch::read_last_crash(Path::new(root))
            .map(Json)
            .map_err(|error| error.to_string())
    }

    #[tool(
        name = "slice_build",
        description = "Collapse compiler error cascades into root causes with source context"
    )]
    async fn slice_build(
        &self,
        Parameters(request): Parameters<SliceBuildArgs>,
    ) -> Result<Json<SliceBuildReport>, String> {
        core::slice_build(request)
            .map(Json)
            .map_err(|error| error.to_string())
    }

    #[tool(
        name = "slice_container",
        description = "Slice Docker container logs with the logcut engine"
    )]
    async fn slice_container(
        &self,
        Parameters(request): Parameters<SliceContainerRequest>,
    ) -> Result<Json<SliceReport>, String> {
        core::slice_container(request)
            .await
            .map(Json)
            .map_err(|error| error.to_string())
    }

    #[tool(
        name = "slice_service",
        description = "Slice a systemd unit's journal with the logcut engine"
    )]
    async fn slice_service(
        &self,
        Parameters(request): Parameters<SliceServiceRequest>,
    ) -> Result<Json<SliceReport>, String> {
        core::slice_service(request)
            .await
            .map(Json)
            .map_err(|error| error.to_string())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for LogcutServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Slice noisy logs, stacktraces and error bursts down to diagnostic signal.",
        )
    }
}

/// Serve until the MCP client closes stdio. No writes to stdout except JSON-RPC.
pub async fn serve() -> anyhow::Result<()> {
    let service = LogcutServer::new()
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|error| anyhow::anyhow!("MCP initialize failed: {error}"))?;
    service
        .waiting()
        .await
        .map_err(|error| anyhow::anyhow!("MCP session ended: {error}"))?;
    Ok(())
}
