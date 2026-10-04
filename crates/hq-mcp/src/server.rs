//! MCP stdio server — implements the rmcp `ServerHandler` trait.

use hq_tools::registry::ToolRegistry;
use rmcp::ErrorData;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, Implementation, ListToolsResult,
    PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerConfig,
};
use rmcp::service::{RequestContext, RoleServer};
use std::borrow::Cow;
use std::sync::Arc;

use crate::gateway;

/// The MCP server handler. Holds a shared reference to the tool registry.
#[derive(Clone)]
pub struct HqMcpServer {
    registry: Arc<ToolRegistry>,
    /// Opened once at construction and reused for gateway telemetry on every
    /// `hq_call` invocation. `Database` wraps an r2d2 pool and is cheap to clone.
    db: hq_db::Database,
}

/// Clients negotiate down to this, so the wire format stays what existing clients were built against.
const PROTOCOL_VERSIONS: &[ProtocolVersion] = &[ProtocolVersion::V_2024_11_05];

impl HqMcpServer {
    pub fn new(registry: Arc<ToolRegistry>, db: hq_db::Database) -> Self {
        Self { registry, db }
    }

    /// Start serving over stdin/stdout using the rmcp transport.
    pub async fn serve_stdio(self) -> anyhow::Result<()> {
        let transport = rmcp::transport::stdio();
        let server = rmcp::serve_server(self, transport).await?;
        server.waiting().await?;
        Ok(())
    }
}

impl ServerHandler for HqMcpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_server_info(Implementation::new("agent-hq", env!("CARGO_PKG_VERSION")))
            .with_instructions(gateway::server_instructions(&self.registry))
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(PROTOCOL_VERSIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(gateway::create_gateway_tools(&self.registry)))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        // A pane HQ launched carries its session id, so what it calls is marked as spawned.
        let marked = std::env::var(hq_tools::harness_session::SESSION_ENV)
            .ok()
            .filter(|id| !id.trim().is_empty())
            .and_then(|id| gateway::mark_spawned_session(request.arguments.as_ref(), &id));
        gateway::dispatch(
            &self.registry,
            request.name.as_ref(),
            marked.as_ref().or(request.arguments.as_ref()),
            &self.db,
            None,
        )
        .await
        .map(CallToolResponse::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_tools::registry::HqTool;

    struct Stub {
        name: String,
        description: String,
        category: String,
    }

    #[async_trait::async_trait]
    impl HqTool for Stub {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            &self.description
        }
        fn category(&self) -> &str {
            &self.category
        }
        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        async fn execute(&self, _: serde_json::Value) -> anyhow::Result<serde_json::Value> {
            Ok(serde_json::json!("ok"))
        }
    }

    fn server_with_stub_registry() -> HqMcpServer {
        let mut registry = ToolRegistry::new();
        for i in 0..208 {
            registry.register(Box::new(Stub {
                name: format!("tool_number_{i:03}"),
                description:
                    "Performs a representative operation against the configured backend and \
                     returns the resulting payload for the caller to inspect"
                        .into(),
                category: format!("category_{}", i % 43),
            }));
        }
        // Instructions never touch the database; in-memory keeps the test
        // free of filesystem setup.
        let db = hq_db::Database::open_memory().unwrap();
        HqMcpServer::new(Arc::new(registry), db)
    }

    /// Instructions ship on every MCP handshake. Swapping the full
    /// description for a derived hint is the whole point of the change.
    #[test]
    fn instructions_are_smaller_than_a_full_description_catalog() {
        let server = server_with_stub_registry();
        let instructions = gateway::server_instructions(&server.registry);
        let description_catalog: usize = server
            .registry
            .list()
            .iter()
            .map(|t| t.name.len() + t.description.len() + 8)
            .sum();
        assert!(
            instructions.len() < description_catalog,
            "instructions {} bytes vs description catalog {} bytes — the hint swap saved nothing",
            instructions.len(),
            description_catalog
        );
    }

    #[test]
    fn instructions_list_every_tool_once() {
        let server = server_with_stub_registry();
        let instructions = gateway::server_instructions(&server.registry);
        for i in [0usize, 99, 207] {
            let name = format!("tool_number_{i:03}");
            assert_eq!(
                instructions.matches(&format!("**{name}**")).count(),
                1,
                "{name} not listed exactly once"
            );
        }
        assert!(instructions.contains("hq_call(tool, args)"));
    }
}
