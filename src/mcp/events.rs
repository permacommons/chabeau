use rust_mcp_schema::schema_utils::ServerJsonrpcRequest;

#[derive(Debug, Clone)]
pub struct McpServerRequest {
    pub server_id: String,
    pub request: ServerJsonrpcRequest,
}
