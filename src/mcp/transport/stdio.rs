use rust_mcp_schema::schema_utils::{RequestFromClient, ServerMessage};

use super::{list_fetch_from_response, ListFetch};

pub async fn fetch_list<T>(
    send: impl std::future::Future<Output = Result<ServerMessage, String>>,
    parse: impl FnOnce(ServerMessage) -> Result<T, String>,
) -> ListFetch<T> {
    let response = send.await;
    list_fetch_from_response(response, parse)
}

pub fn list_request(request: RequestFromClient) -> RequestFromClient {
    request
}
