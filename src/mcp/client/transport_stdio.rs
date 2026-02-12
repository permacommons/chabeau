use super::StdioClient;
use rust_mcp_schema::schema_utils::{RequestFromClient, ResultFromClient, ServerMessage};
use rust_mcp_schema::{RequestId, RpcError};
use std::sync::Arc;

pub(crate) async fn send_request(
    client: Option<Arc<StdioClient>>,
    request: RequestFromClient,
) -> Result<ServerMessage, String> {
    let Some(client) = client else {
        return Err("MCP client not connected.".to_string());
    };
    client.send_request(request).await
}

pub(crate) async fn send_result(
    client: Option<Arc<StdioClient>>,
    request_id: RequestId,
    result: ResultFromClient,
) -> Result<(), String> {
    let Some(client) = client else {
        return Err("MCP client not connected.".to_string());
    };
    client.send_result(request_id, result).await
}

pub(crate) async fn send_error(
    client: Option<Arc<StdioClient>>,
    request_id: RequestId,
    error: RpcError,
) -> Result<(), String> {
    let Some(client) = client else {
        return Err("MCP client not connected.".to_string());
    };
    client.send_error(request_id, error).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stdio_requires_connected_client() {
        let err = send_request(None, RequestFromClient::PingRequest(None))
            .await
            .expect_err("expected missing client error");
        assert_eq!(err, "MCP client not connected.");
    }
}
