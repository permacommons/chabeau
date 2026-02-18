//! Stdio transport adapters for list-fetch behavior.
//!
//! This module keeps stdio list handling aligned with HTTP list semantics so
//! capability detection and cache refresh paths are transport-agnostic.

use rust_mcp_schema::schema_utils::{RequestFromClient, ServerMessage};

use super::{list_fetch_from_response, ListFetch};

/// Awaits a stdio list request and maps it into normalized list semantics.
pub async fn fetch_list<T>(
    send: impl std::future::Future<Output = Result<ServerMessage, String>>,
    parse: impl FnOnce(ServerMessage) -> Result<T, String>,
) -> ListFetch<T> {
    let response = send.await;
    list_fetch_from_response(response, parse)
}

/// Marker helper for stdio list requests.
///
/// Kept as a dedicated seam so contributors can add stdio-specific wrapping in
/// one place without touching call sites.
pub fn list_request(request: RequestFromClient) -> RequestFromClient {
    request
}
