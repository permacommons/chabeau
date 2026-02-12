use rust_mcp_schema::schema_utils::ServerMessage;

use super::{list_fetch_from_response, ListFetch};

pub fn fetch_list<T>(
    response: Result<ServerMessage, String>,
    parse: impl FnOnce(ServerMessage) -> Result<T, String>,
) -> ListFetch<T> {
    list_fetch_from_response(response, parse)
}
