
use super::*;
use std::sync::Arc;
use tokio::sync::Mutex;

fn sample_config() -> McpServerConfig {
    McpServerConfig {
        id: "alpha".to_string(),
        display_name: "Alpha MCP".to_string(),
        base_url: Some("https://mcp.example.com".to_string()),
        command: None,
        args: None,
        env: None,
        transport: Some("streamable-http".to_string()),
        allowed_tools: None,
        protocol_version: None,
        enabled: Some(true),
        tool_payloads: None,
        tool_payload_window: None,
        yolo: None,
    }
}

fn init_with_caps(caps: ServerCapabilities) -> InitializeResult {
    InitializeResult {
        capabilities: caps,
        instructions: None,
        meta: None,
        protocol_version: "2025-11-25".to_string(),
        server_info: Implementation {
            name: "server".to_string(),
            version: "0.1.0".to_string(),
            title: None,
            description: None,
            icons: Vec::new(),
            website_url: None,
        },
    }
}

#[test]
fn server_capability_defaults_to_supported() {
    let mut state = McpServerState::new(sample_config());
    state.server_details = None;
    assert!(state.supports_tools());
    assert!(state.supports_resources());
    assert!(state.supports_prompts());
}

#[test]
fn server_capability_flags_disable_missing_lists() {
    let mut state = McpServerState::new(sample_config());
    let caps = ServerCapabilities::default();
    state.server_details = Some(init_with_caps(caps));
    assert!(!state.supports_tools());
    assert!(!state.supports_resources());
    assert!(!state.supports_prompts());
}

#[test]
fn server_capability_flags_enable_present_lists() {
    let mut state = McpServerState::new(sample_config());
    let caps = ServerCapabilities {
        tools: Some(rust_mcp_schema::ServerCapabilitiesTools::default()),
        resources: Some(rust_mcp_schema::ServerCapabilitiesResources::default()),
        prompts: Some(rust_mcp_schema::ServerCapabilitiesPrompts::default()),
        ..ServerCapabilities::default()
    };
    state.server_details = Some(init_with_caps(caps));
    assert!(state.supports_tools());
    assert!(state.supports_resources());
    assert!(state.supports_prompts());
}

#[tokio::test]
async fn connect_all_attempts_each_enabled_server_when_one_fails() {
    let config = Config {
        mcp_servers: vec![
            McpServerConfig {
                id: "alpha".to_string(),
                display_name: "Alpha".to_string(),
                base_url: None,
                command: Some("/definitely-missing-command".to_string()),
                args: None,
                env: None,
                transport: Some("stdio".to_string()),
                allowed_tools: None,
                protocol_version: None,
                enabled: Some(true),
                tool_payloads: None,
                tool_payload_window: None,
                yolo: None,
            },
            McpServerConfig {
                id: "beta".to_string(),
                display_name: "Beta".to_string(),
                base_url: None,
                command: Some("/definitely-missing-command-2".to_string()),
                args: None,
                env: None,
                transport: Some("stdio".to_string()),
                allowed_tools: None,
                protocol_version: None,
                enabled: Some(true),
                tool_payloads: None,
                tool_payload_window: None,
                yolo: None,
            },
        ],
        ..Config::default()
    };

    let mut manager = McpClientManager::from_config(&config);
    let token_store = McpTokenStore::new_with_keyring(false);

    manager.connect_all(&token_store).await;

    assert!(manager
        .server("alpha")
        .and_then(|s| s.last_error.as_ref())
        .is_some());
    assert!(manager
        .server("beta")
        .and_then(|s| s.last_error.as_ref())
        .is_some());
}

#[test]
fn streamable_http_client_post_headers_include_json_and_sse_accept() {
    let client = reqwest::Client::new();
    let request = apply_streamable_http_client_post_headers(client.post("https://example.com"))
        .build()
        .expect("request should build");

    assert_eq!(
        request
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok()),
        Some(MCP_JSON_CONTENT_TYPE)
    );
    assert_eq!(
        request
            .headers()
            .get("Accept")
            .and_then(|v| v.to_str().ok()),
        Some(MCP_JSON_AND_SSE_ACCEPT)
    );
}

#[test]
fn streamable_http_protocol_version_header_is_applied_when_present() {
    let client = reqwest::Client::new();
    let request = apply_streamable_http_protocol_version_header(
        client.post("https://example.com"),
        Some("2025-11-25"),
    )
    .build()
    .expect("request should build");

    assert_eq!(
        request
            .headers()
            .get(MCP_PROTOCOL_VERSION_HEADER)
            .and_then(|value| value.to_str().ok()),
        Some("2025-11-25")
    );
}

#[test]
fn effective_protocol_version_prefers_negotiated_value() {
    let mut config = sample_config();
    config.protocol_version = Some("2025-01-01".to_string());

    assert_eq!(
        effective_protocol_version(&config, Some("2025-11-25")),
        "2025-11-25"
    );
    assert_eq!(effective_protocol_version(&config, None), "2025-01-01");
}

fn sample_tool(name: String) -> rust_mcp_schema::Tool {
    rust_mcp_schema::Tool {
        annotations: None,
        description: None,
        execution: None,
        icons: Vec::new(),
        input_schema: rust_mcp_schema::ToolInputSchema::new(Vec::new(), None, None),
        meta: None,
        name,
        output_schema: None,
        title: None,
    }
}

struct ToolPageState {
    calls: Vec<Option<String>>,
}

type CapturedHttpRequests =
    Arc<Mutex<Vec<(String, String, String, String, String, Option<String>)>>>;

async fn fetch_tools_page_test(
    state: &Arc<Mutex<ToolPageState>>,
    cursor: Option<String>,
) -> Result<Option<ListToolsResult>, String> {
    let mut state = state.lock().await;
    state.calls.push(cursor.clone());
    let result = match cursor.as_deref() {
        None => ListToolsResult {
            meta: None,
            next_cursor: Some("c1".to_string()),
            tools: (0..60)
                .map(|idx| sample_tool(format!("tool-{idx}")))
                .collect(),
        },
        Some("c1") => ListToolsResult {
            meta: None,
            next_cursor: Some("c2".to_string()),
            tools: (60..120)
                .map(|idx| sample_tool(format!("tool-{idx}")))
                .collect(),
        },
        Some("c2") => ListToolsResult {
            meta: None,
            next_cursor: None,
            tools: vec![sample_tool("tool-120".to_string())],
        },
        Some(other) => {
            return Err(format!("Unexpected cursor: {other}"));
        }
    };

    Ok(Some(result))
}

#[tokio::test]
async fn paginate_tools_list_caps_and_preserves_cursor() {
    let state = Arc::new(Mutex::new(ToolPageState { calls: Vec::new() }));
    let result = paginate_tools_list_with!(fetch_tools_page_test, (&state))
        .expect("pagination should succeed")
        .expect("expected list tools result");

    assert_eq!(result.tools.len(), MCP_MAX_TOOL_LIST);
    assert_eq!(result.next_cursor.as_deref(), Some("c2"));
    let calls = state.lock().await.calls.clone();
    assert_eq!(calls, vec![None, Some("c1".to_string())]);
}

async fn fetch_tools_page_first_page_full(
    state: &Arc<Mutex<ToolPageState>>,
    cursor: Option<String>,
) -> Result<Option<ListToolsResult>, String> {
    let mut state = state.lock().await;
    state.calls.push(cursor.clone());
    let result = ListToolsResult {
        meta: None,
        next_cursor: Some("c1".to_string()),
        tools: (0..MCP_MAX_TOOL_LIST + 5)
            .map(|idx| sample_tool(format!("tool-{idx}")))
            .collect(),
    };
    Ok(Some(result))
}

#[tokio::test]
async fn paginate_tools_list_stops_when_first_page_is_full() {
    let state = Arc::new(Mutex::new(ToolPageState { calls: Vec::new() }));
    let result = paginate_tools_list_with!(fetch_tools_page_first_page_full, (&state))
        .expect("pagination should succeed")
        .expect("expected list tools result");

    assert_eq!(result.tools.len(), MCP_MAX_TOOL_LIST);
    assert_eq!(result.next_cursor.as_deref(), Some("c1"));
    let calls = state.lock().await.calls.clone();
    assert_eq!(calls, vec![None]);
}
#[test]
fn event_stream_content_type_parser_handles_parameters_and_case() {
    assert!(is_event_stream_content_type("text/event-stream"));
    assert!(is_event_stream_content_type(
        "Text/Event-Stream; charset=UTF-8"
    ));
    assert!(is_event_stream_content_type(
        "text/event-stream ; version=1"
    ));
    assert!(!is_event_stream_content_type("application/json"));
}

async fn read_http_request(
    stream: &mut tokio::net::TcpStream,
) -> Result<(String, Vec<(String, String)>, Vec<u8>), String> {
    use tokio::io::AsyncReadExt;

    let mut buffer = Vec::new();
    let mut header_end = None;
    while header_end.is_none() {
        let mut chunk = [0_u8; 1024];
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|err| err.to_string())?;
        if read == 0 {
            return Err("Unexpected EOF while reading HTTP headers".to_string());
        }
        buffer.extend_from_slice(&chunk[..read]);
        header_end = buffer
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| index + 4);
    }

    let header_end = header_end.expect("header end should exist");
    let header_bytes = &buffer[..header_end];
    let header_text = std::str::from_utf8(header_bytes).map_err(|err| err.to_string())?;
    let mut lines = header_text.split("\r\n").filter(|line| !line.is_empty());
    let request_line = lines
        .next()
        .ok_or_else(|| "Missing HTTP request line".to_string())?
        .to_string();

    let mut headers = Vec::new();
    let mut content_length = 0_usize;
    for line in lines {
        let mut parts = line.splitn(2, ':');
        let Some(name) = parts.next() else {
            continue;
        };
        let value = parts.next().unwrap_or_default().trim().to_string();
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value.parse::<usize>().map_err(|err| err.to_string())?;
        }
        headers.push((name.to_string(), value));
    }

    let mut body = buffer[header_end..].to_vec();
    while body.len() < content_length {
        let mut chunk = vec![0_u8; content_length.saturating_sub(body.len())];
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|err| err.to_string())?;
        if read == 0 {
            return Err("Unexpected EOF while reading HTTP body".to_string());
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(content_length);

    Ok((request_line, headers, body))
}

#[tokio::test]
async fn streamable_http_end_to_end_handles_json_and_sse_responses() {
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let addr = listener.local_addr().expect("local addr should resolve");
    let captured_requests: CapturedHttpRequests = Arc::new(Mutex::new(Vec::new()));
    let captured_for_server = Arc::clone(&captured_requests);

    let server_task = tokio::spawn(async move {
        for _ in 0..4 {
            let (mut stream, _) = listener.accept().await.map_err(|err| err.to_string())?;
            let (request_line, headers, body) = read_http_request(&mut stream).await?;
            let accept = headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("accept"))
                .map(|(_, value)| value.clone())
                .unwrap_or_default();
            let content_type = headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
                .map(|(_, value)| value.clone())
                .unwrap_or_default();
            let protocol_version = headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(MCP_PROTOCOL_VERSION_HEADER))
                .map(|(_, value)| value.clone())
                .unwrap_or_default();
            let session_id = headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("mcp-session-id"))
                .map(|(_, value)| value.clone());

            let body_json: serde_json::Value =
                serde_json::from_slice(&body).map_err(|err| err.to_string())?;
            let method = body_json
                .get("method")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();

            captured_for_server.lock().await.push((
                request_line,
                method.clone(),
                accept,
                content_type,
                protocol_version,
                session_id,
            ));

            let response = if method == "initialize" {
                let body = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 0,
                    "result": {
                        "protocolVersion": "2025-12-31",
                        "capabilities": {},
                        "serverInfo": {
                            "name": "mock",
                            "version": "0.1.0",
                            "icons": []
                        }
                    }
                })
                .to_string();
                format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nmcp-session-id: test-session\r\ncontent-length: {}\r\n\r\n{}",
                        body.len(), body
                    )
            } else if method == "notifications/initialized" {
                let body = "{}";
                format!(
                        "HTTP/1.1 202 Accepted\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                        body.len(), body
                    )
            } else if method == "resources/list" {
                let event = "data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n\n";
                format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: Text/Event-Stream; Charset=UTF-8\r\nmcp-session-id: test-session-2\r\ncontent-length: {}\r\n\r\n{}",
                        event.len(), event
                    )
            } else {
                let event = "data: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"ok\":true}}\n\n";
                format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: Text/Event-Stream; Charset=UTF-8\r\ncontent-length: {}\r\n\r\n{}",
                        event.len(), event
                    )
            };

            stream
                .write_all(response.as_bytes())
                .await
                .map_err(|err| err.to_string())?;
        }
        Ok::<(), String>(())
    });

    std::env::remove_var("HTTP_PROXY");
    std::env::remove_var("http_proxy");
    std::env::remove_var("HTTPS_PROXY");
    std::env::remove_var("https_proxy");
    std::env::remove_var("ALL_PROXY");
    std::env::remove_var("all_proxy");
    std::env::set_var("NO_PROXY", "*");
    std::env::set_var("no_proxy", "*");

    let config = Config {
        mcp_servers: vec![McpServerConfig {
            id: "alpha".to_string(),
            display_name: "Alpha".to_string(),
            base_url: Some(format!("http://{}", addr)),
            command: None,
            args: None,
            env: None,
            transport: Some("streamable-http".to_string()),
            allowed_tools: None,
            protocol_version: None,
            enabled: Some(true),
            tool_payloads: None,
            tool_payload_window: None,
            yolo: None,
        }],
        ..Config::default()
    };

    let mut manager = McpClientManager::from_config(&config);
    manager
        .ensure_streamable_http_session("alpha")
        .await
        .expect("initialize should succeed");

    let message = manager
        .send_streamable_http_request("alpha", RequestFromClient::ListResourcesRequest(None))
        .await
        .expect("SSE request should succeed");
    let value = parse_response_value(message).expect("response value should parse");
    assert_eq!(value.get("ok").and_then(|item| item.as_bool()), Some(true));

    let message = manager
        .send_streamable_http_request("alpha", RequestFromClient::ListResourcesRequest(None))
        .await
        .expect("second SSE request should succeed");
    let value = parse_response_value(message).expect("response value should parse");
    assert_eq!(value.get("ok").and_then(|item| item.as_bool()), Some(true));

    server_task
        .await
        .expect("mock server task should join")
        .expect("mock server should succeed");

    let captured = captured_requests.lock().await.clone();
    assert_eq!(captured.len(), 4);
    assert_eq!(captured[0].1, "initialize");
    assert_eq!(captured[1].1, "notifications/initialized");
    assert_eq!(captured[2].1, "resources/list");
    assert_eq!(captured[0].2, MCP_JSON_AND_SSE_ACCEPT);
    assert_eq!(captured[1].2, MCP_JSON_AND_SSE_ACCEPT);
    assert_eq!(captured[2].2, MCP_JSON_AND_SSE_ACCEPT);
    assert_eq!(captured[0].3, MCP_JSON_CONTENT_TYPE);
    assert_eq!(captured[1].3, MCP_JSON_CONTENT_TYPE);
    assert_eq!(captured[2].3, MCP_JSON_CONTENT_TYPE);
    assert_eq!(captured[0].4, LATEST_PROTOCOL_VERSION);
    assert_eq!(captured[1].4, "2025-12-31");
    assert_eq!(captured[2].4, "2025-12-31");
    assert_eq!(captured[2].5.as_deref(), Some("test-session"));
    assert_eq!(captured[3].1, "resources/list");
    assert_eq!(captured[3].5.as_deref(), Some("test-session-2"));

    let stored_session = manager
        .server("alpha")
        .and_then(|server| server.session_id.as_deref());
    assert_eq!(stored_session, Some("test-session-2"));
}
