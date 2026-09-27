//! `chabeau mcp` subcommands: listing, adding, editing and removing MCP
//! servers, and managing their tokens and OAuth grants.

use std::error::Error;

use super::prompts::{prompt_bool_with_default, prompt_optional, prompt_required};
use super::{McpCommands, McpOauthCommands, McpTokenCommands};
use crate::auth::prompt_provider_token;
use crate::core::config::data::{Config, McpServerConfig};
use crate::core::mcp_auth::{McpOAuthGrant, McpTokenStore};
use crate::core::oauth::{
    apply_oauth_token_response, build_authorization_url, current_unix_epoch_s, exchange_oauth_code,
    open_in_browser, pkce_s256_challenge, probe_oauth_support, random_urlsafe,
    register_oauth_client, validate_oauth_endpoint_url, wait_for_oauth_callback,
    AuthorizationUrlParams, OAuthMetadata,
};

pub(super) async fn handle_mcp_command(
    command: McpCommands,
    _env_only: bool,
) -> Result<(), Box<dyn Error>> {
    match command {
        McpCommands::List => handle_mcp_list(),
        McpCommands::Add { advanced } => handle_mcp_add(advanced).await,
        McpCommands::Edit { server, advanced } => handle_mcp_edit(&server, advanced),
        McpCommands::Remove { server } => handle_mcp_remove(&server),
        McpCommands::Token { command } => handle_mcp_token(command),
        McpCommands::Oauth { command } => handle_mcp_oauth(command).await,
    }
}

fn handle_mcp_list() -> Result<(), Box<dyn Error>> {
    let config = Config::load()?;
    let servers = config.list_mcp_servers();
    if servers.is_empty() {
        println!(
            "No MCP servers configured. Add `[[mcp_servers]]` to config.toml or run `chabeau mcp add`."
        );
        return Ok(());
    }

    let store = McpTokenStore::new();
    println!("Configured MCP servers:");
    for server in servers {
        let token_status = match store.get_token(&server.id) {
            Ok(Some(_)) => "token configured",
            Ok(None) => "no token",
            Err(_) => "token status unavailable",
        };
        let transport = server.transport.as_deref().unwrap_or("streamable-http");
        let enabled = if server.is_enabled() {
            "enabled"
        } else {
            "disabled"
        };
        println!(
            "  - {} ({}) [{}; {}; {}]",
            server.display_name, server.id, transport, enabled, token_status
        );
    }

    Ok(())
}

async fn handle_mcp_add(advanced: bool) -> Result<(), Box<dyn Error>> {
    let mut config = Config::load()?;
    if !advanced {
        println!(
            "Basic mode: advanced options are hidden. Re-run with `chabeau mcp add -a` for advanced settings."
        );
    }
    let display_name = prompt_required("Display name: ")?;
    let suggested_id = crate::core::config::data::suggest_provider_id(&display_name);
    let id_input = prompt_optional(&format!("Server id [{suggested_id}]: "))?;
    let server_id = if id_input.is_empty() {
        suggested_id
    } else {
        validate_mcp_server_id(&id_input)?
    };

    if config.get_mcp_server(&server_id).is_some() {
        return Err(format!("MCP server '{server_id}' already exists").into());
    }

    let mut server = McpServerConfig {
        id: server_id,
        display_name,
        base_url: None,
        command: None,
        args: None,
        env: None,
        headers: None,
        transport: Some(prompt_transport(None)?.to_string()),
        allowed_tools: None,
        protocol_version: None,
        enabled: Some(true),
        tool_payloads: None,
        tool_payload_window: None,
        yolo: Some(false),
    };
    configure_mcp_transport_fields(&mut server, false, advanced)?;
    if advanced {
        server.enabled = Some(prompt_bool_with_default("Enabled", server.is_enabled())?);
        server.yolo = Some(prompt_bool_with_default(
            "YOLO auto-approve",
            server.is_yolo(),
        )?);
    }

    config.mcp_servers.push(server.clone());
    config.save()?;
    println!(
        "✅ Added MCP server {} ({})",
        server.display_name, server.id
    );

    let is_http_transport = !matches!(server.transport.as_deref(), Some("stdio"));
    if is_http_transport
        && server
            .base_url
            .as_deref()
            .is_some_and(|url| url.starts_with("http://") || url.starts_with("https://"))
    {
        if let Some(metadata) = probe_oauth_support(&server).await? {
            println!("Detected OAuth metadata for {}.", server.display_name);
            let should_setup_oauth = prompt_bool_with_default("Configure OAuth now", true)?;
            if should_setup_oauth {
                if let Err(err) =
                    add_oauth_grant_for_server(&server, Some(metadata), false, advanced).await
                {
                    eprintln!("⚠️ OAuth setup skipped: {err}");
                }
            }
        } else if prompt_bool_with_default(
            "No OAuth metadata detected. Add bearer token now",
            false,
        )? {
            let token =
                prompt_provider_token(&server.display_name).map_err(|err| err.to_string())?;
            McpTokenStore::new().set_token(&server.id, &token)?;
            println!("✅ Stored MCP token for {}", server.display_name);
        }
    }
    Ok(())
}

fn handle_mcp_edit(server_input: &str, advanced: bool) -> Result<(), Box<dyn Error>> {
    let mut config = Config::load()?;
    if !advanced {
        println!(
            "Basic mode: advanced options are hidden. Re-run with `chabeau mcp edit {server_input} -a` for advanced settings."
        );
    }
    let current = resolve_mcp_server(&config, server_input)?.clone();
    let mut server = current.clone();

    let display_name = prompt_optional(&format!("Display name [{}]: ", current.display_name))?;
    if !display_name.is_empty() {
        server.display_name = display_name;
    }

    let current_transport = current
        .transport
        .as_deref()
        .unwrap_or("streamable-http")
        .to_string();
    let transport = prompt_transport(Some(&current_transport))?;
    server.transport = Some(transport.to_string());
    configure_mcp_transport_fields(&mut server, true, advanced)?;

    if advanced {
        server.enabled = Some(prompt_bool_with_default("Enabled", current.is_enabled())?);
        server.yolo = Some(prompt_bool_with_default(
            "YOLO auto-approve",
            current.is_yolo(),
        )?);
    }

    if let Some(existing) = config
        .mcp_servers
        .iter_mut()
        .find(|candidate| candidate.id.eq_ignore_ascii_case(&server.id))
    {
        *existing = server.clone();
    }
    config.save()?;
    println!(
        "✅ Updated MCP server {} ({})",
        server.display_name, server.id
    );
    Ok(())
}

fn handle_mcp_remove(server_input: &str) -> Result<(), Box<dyn Error>> {
    let mut config = Config::load()?;
    let server = resolve_mcp_server(&config, server_input)?.clone();
    let store = McpTokenStore::new();
    let confirmed = prompt_bool_with_default(
        &format!(
            "Remove MCP server {} ({}) from config? Any associated OAuth tokens or configuration variables will also be removed",
            server.display_name, server.id
        ),
        false,
    )?;
    if !confirmed {
        println!("Cancelled.");
        return Ok(());
    }

    config
        .mcp_servers
        .retain(|candidate| !candidate.id.eq_ignore_ascii_case(&server.id));
    config.save()?;

    match store.remove_oauth_grant(&server.id) {
        Ok(true) => println!("✅ Removed stored OAuth grant for {}", server.display_name),
        Ok(false) => println!("☑️ No stored OAuth grant found for {}", server.display_name),
        Err(err) => eprintln!(
            "⚠️ Could not remove stored OAuth grant for {}: {}",
            server.display_name, err
        ),
    }
    match store.remove_token(&server.id) {
        Ok(true) => println!("✅ Removed stored MCP token for {}", server.display_name),
        Ok(false) => println!("☑️ No stored MCP token found for {}", server.display_name),
        Err(err) => eprintln!(
            "⚠️ Could not remove stored MCP token for {}: {}",
            server.display_name, err
        ),
    }

    println!(
        "✅ Removed MCP server {} ({})",
        server.display_name, server.id
    );
    Ok(())
}

fn handle_mcp_token(command: McpTokenCommands) -> Result<(), Box<dyn Error>> {
    let config = Config::load()?;
    let store = McpTokenStore::new();
    match command {
        McpTokenCommands::Add { server } => {
            let server_config = resolve_mcp_server(&config, &server)?;
            let token = prompt_provider_token(&server_config.display_name)
                .map_err(|err| err.to_string())?;
            store.set_token(&server_config.id, &token)?;
            println!("✅ Stored MCP token for {}", server_config.display_name);
        }
        McpTokenCommands::Remove { server } => {
            let server_config = resolve_mcp_server(&config, &server)?;
            let removed = store.remove_token(&server_config.id)?;
            if removed {
                println!("✅ Removed MCP token for {}", server_config.display_name);
            } else {
                println!("No MCP token was stored for {}", server_config.display_name);
            }
        }
        McpTokenCommands::List { server } => {
            if let Some(server) = server {
                let server_config = resolve_mcp_server(&config, &server)?;
                let status = if store.get_token(&server_config.id)?.is_some() {
                    "configured"
                } else {
                    "missing"
                };
                println!(
                    "MCP token for {} ({}): {}",
                    server_config.display_name, server_config.id, status
                );
            } else {
                let servers = config.list_mcp_servers();
                if servers.is_empty() {
                    println!("No MCP servers configured.");
                    return Ok(());
                }
                println!("MCP token status:");
                for server in servers {
                    let status = if store.get_token(&server.id)?.is_some() {
                        "configured"
                    } else {
                        "missing"
                    };
                    println!("  - {} ({}): {}", server.display_name, server.id, status);
                }
            }
        }
    }

    Ok(())
}

async fn handle_mcp_oauth(command: McpOauthCommands) -> Result<(), Box<dyn Error>> {
    let config = Config::load()?;
    let store = McpTokenStore::new();
    match command {
        McpOauthCommands::List { server } => {
            if let Some(server) = server {
                let server_config = resolve_mcp_server(&config, &server)?;
                if let Some(grant) = store.get_oauth_grant(&server_config.id)? {
                    println!(
                        "MCP OAuth for {} ({}): configured",
                        server_config.display_name, server_config.id
                    );
                    let scope = grant.scope.as_deref().unwrap_or("n/a");
                    println!("  scope: {scope}");
                    let expires = grant
                        .expires_at_epoch_s
                        .map(|epoch| epoch.to_string())
                        .unwrap_or_else(|| "n/a".to_string());
                    println!("  expires_at_epoch_s: {expires}");
                } else {
                    println!(
                        "MCP OAuth for {} ({}): missing",
                        server_config.display_name, server_config.id
                    );
                }
            } else {
                let servers = config.list_mcp_servers();
                if servers.is_empty() {
                    println!("No MCP servers configured.");
                    return Ok(());
                }
                println!("MCP OAuth grant status:");
                for server in servers {
                    let status = if store.get_oauth_grant(&server.id)?.is_some() {
                        "configured"
                    } else {
                        "missing"
                    };
                    println!("  - {} ({}): {}", server.display_name, server.id, status);
                }
            }
        }
        McpOauthCommands::Add { server, advanced } => {
            let server_config = resolve_mcp_server(&config, &server)?;
            add_oauth_grant_for_server(server_config, None, true, advanced).await?;
        }
        McpOauthCommands::Remove { server } => {
            let server_config = resolve_mcp_server(&config, &server)?;
            remove_oauth_grant_for_server(server_config).await?;
        }
    }
    Ok(())
}

async fn add_oauth_grant_for_server(
    server: &McpServerConfig,
    metadata: Option<OAuthMetadata>,
    check_existing: bool,
    advanced: bool,
) -> Result<(), Box<dyn Error>> {
    let store = McpTokenStore::new();

    if check_existing && store.get_oauth_grant(&server.id)?.is_some() {
        println!(
            "OAuth grant already exists for {} ({}).",
            server.display_name, server.id
        );
        if prompt_bool_with_default("Remove existing grant now", false)? {
            remove_oauth_grant_for_server(server).await?;
        } else {
            println!("Cancelled.");
        }
        return Ok(());
    }

    let metadata = if let Some(metadata) = metadata {
        metadata
    } else {
        match probe_oauth_support(server).await? {
            Some(metadata) => metadata,
            None => {
                return Err(format!(
                    "No OAuth metadata discovered for {} ({})",
                    server.display_name, server.id
                )
                .into());
            }
        }
    };

    if let Some(authorization_endpoint) = metadata.authorization_endpoint.as_deref() {
        validate_oauth_endpoint_url("authorization_endpoint", authorization_endpoint)?;
        let token_endpoint = metadata
            .token_endpoint
            .as_deref()
            .ok_or("OAuth metadata is missing token_endpoint.")?;
        validate_oauth_endpoint_url("token_endpoint", token_endpoint)?;

        let mut revocation_endpoint = metadata.revocation_endpoint.clone();
        sanitize_optional_oauth_endpoint(
            "revocation_endpoint",
            &mut revocation_endpoint,
            "Skipping token revocation metadata for this grant.",
        );

        let mut client_id = if advanced {
            let client_id_input = prompt_optional("OAuth client id (optional): ")?;
            if client_id_input.is_empty() {
                None
            } else {
                Some(client_id_input)
            }
        } else {
            println!(
                "Basic mode: trying automatic OAuth client registration. Re-run with `-a` to provide a client id manually."
            );
            None
        };

        let mut registration_endpoint = metadata.registration_endpoint.clone();
        sanitize_optional_oauth_endpoint(
            "registration_endpoint",
            &mut registration_endpoint,
            "Ignoring registration endpoint and continuing without automatic client registration.",
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let redirect_uri = format!("http://127.0.0.1:{port}/oauth/callback");

        if client_id.is_none() {
            if let Some(registration_endpoint) = registration_endpoint.as_deref() {
                match register_oauth_client(registration_endpoint, &redirect_uri).await {
                    Ok(registered_id) => {
                        println!("Registered OAuth client automatically.");
                        client_id = Some(registered_id);
                    }
                    Err(err) => {
                        eprintln!("⚠️ OAuth client registration failed: {err}");
                    }
                }
            } else if !advanced {
                println!(
                    "OAuth metadata does not advertise dynamic registration. Re-run with `-a` to provide a client id if authorization fails."
                );
            }
        }

        let scope = metadata.scopes_supported.as_ref().and_then(|scopes| {
            let joined = scopes
                .iter()
                .map(|scope| scope.trim())
                .filter(|scope| !scope.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            if joined.is_empty() {
                None
            } else {
                Some(joined)
            }
        });

        let state = random_urlsafe(24)?;
        let code_verifier = random_urlsafe(64)?;
        let code_challenge = pkce_s256_challenge(&code_verifier);
        let authorization_url = build_authorization_url(AuthorizationUrlParams {
            authorization_endpoint,
            client_id: client_id.as_deref(),
            redirect_uri: &redirect_uri,
            state: &state,
            code_challenge: &code_challenge,
            code_challenge_method: "S256",
            issuer: metadata.issuer.as_deref(),
            scope: scope.as_deref(),
        })?;

        if open_in_browser(authorization_url.as_str()).is_ok() {
            println!("Opened OAuth authorization URL in your browser.");
        } else {
            eprintln!(
                "⚠️ Could not launch browser automatically. Open this URL manually:
{}",
                authorization_url
            );
        }

        println!("Waiting for OAuth redirect on {redirect_uri} ...");
        let auth_code = wait_for_oauth_callback(listener, &state).await?;
        let token = exchange_oauth_code(
            token_endpoint,
            client_id.as_deref(),
            &redirect_uri,
            &auth_code,
            &code_verifier,
        )
        .await?;

        let now_epoch_s = current_unix_epoch_s().unwrap_or_default();
        let grant_seed = McpOAuthGrant {
            access_token: String::new(),
            refresh_token: None,
            token_type: None,
            scope: None,
            expires_at_epoch_s: None,
            client_id,
            redirect_uri: Some(redirect_uri),
            authorization_endpoint: metadata.authorization_endpoint.clone(),
            token_endpoint: metadata.token_endpoint.clone(),
            revocation_endpoint,
            issuer: metadata.issuer.clone(),
        };
        let grant = apply_oauth_token_response(&grant_seed, token, now_epoch_s);
        store.set_oauth_grant(&server.id, &grant)?;
        store.set_token(&server.id, &grant.access_token)?;
        println!("✅ Stored OAuth grant for {}", server.display_name);
        Ok(())
    } else {
        Err("OAuth metadata is missing authorization_endpoint.".into())
    }
}

async fn remove_oauth_grant_for_server(server: &McpServerConfig) -> Result<(), Box<dyn Error>> {
    let store = McpTokenStore::new();
    let grant = match store.get_oauth_grant(&server.id)? {
        Some(grant) => grant,
        None => {
            println!("No OAuth grant stored for {}.", server.display_name);
            return Ok(());
        }
    };

    if let Some(revocation_endpoint) = grant.revocation_endpoint.as_deref() {
        if let Err(err) = validate_oauth_endpoint_url("revocation_endpoint", revocation_endpoint) {
            eprintln!("⚠️ {err} Removing local grant only.");
        } else {
            let client = reqwest::Client::new();
            match client
                .post(revocation_endpoint)
                .form(&[("token", grant.access_token.as_str())])
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => {
                    println!("OAuth token revoked at server endpoint.");
                }
                Ok(response) => {
                    eprintln!(
                        "⚠️ OAuth revocation returned HTTP {}. Removing local grant anyway.",
                        response.status()
                    );
                }
                Err(err) => {
                    eprintln!("⚠️ OAuth revocation failed ({err}). Removing local grant anyway.");
                }
            }
        }
    } else {
        eprintln!("⚠️ No revocation endpoint in OAuth metadata. Removing local grant only.");
    }

    store.remove_oauth_grant(&server.id)?;
    let _ = store.remove_token(&server.id)?;
    println!("✅ Removed OAuth grant for {}", server.display_name);
    Ok(())
}

fn validate_mcp_server_id(input: &str) -> Result<String, Box<dyn Error>> {
    if !input
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-' || character == '_')
    {
        return Err("Server id must contain only letters, numbers, '-' or '_'.".into());
    }
    Ok(input.to_ascii_lowercase())
}

fn prompt_transport(current: Option<&str>) -> Result<&'static str, Box<dyn Error>> {
    let default = current.unwrap_or("streamable-http");
    loop {
        let input = prompt_optional(&format!("Transport [streamable-http|stdio] [{default}]: "))?;
        let normalized = if input.is_empty() {
            default.to_ascii_lowercase()
        } else {
            input.to_ascii_lowercase()
        };
        match normalized.as_str() {
            "streamable-http" | "streamable_http" | "http" => return Ok("streamable-http"),
            "stdio" => return Ok("stdio"),
            _ => println!("Unsupported transport. Enter streamable-http or stdio."),
        }
    }
}

fn configure_mcp_transport_fields(
    server: &mut McpServerConfig,
    is_edit: bool,
    advanced: bool,
) -> Result<(), Box<dyn Error>> {
    match server.transport.as_deref().unwrap_or("streamable-http") {
        "stdio" => {
            server.base_url = None;
            server.headers = None;
            let command_prompt = if is_edit {
                format!(
                    "Command [{}]: ",
                    server.command.as_deref().unwrap_or_default()
                )
            } else {
                "Command: ".to_string()
            };
            let command_input = prompt_optional(&command_prompt)?;
            if is_edit {
                if !command_input.is_empty() {
                    server.command = Some(command_input);
                }
            } else if command_input.is_empty() {
                return Err("Command cannot be empty for stdio transport.".into());
            } else {
                server.command = Some(command_input);
            }

            if advanced {
                let args_default = server
                    .args
                    .as_ref()
                    .map(|args| args.join(" "))
                    .unwrap_or_default();
                let args_input =
                    prompt_optional(&format!("Args (space-separated) [{}]: ", args_default))?;
                if !args_input.is_empty() {
                    server.args = Some(
                        args_input
                            .split_whitespace()
                            .map(ToString::to_string)
                            .collect(),
                    );
                } else if !is_edit {
                    server.args = None;
                }
            }

            if advanced {
                let env_default = server
                    .env
                    .as_ref()
                    .map(|env| {
                        let mut pairs: Vec<String> = env
                            .iter()
                            .map(|(key, value)| format!("{key}={value}"))
                            .collect();
                        pairs.sort();
                        pairs.join(",")
                    })
                    .unwrap_or_default();
                let env_input = prompt_optional(&format!(
                    "Env (KEY=VALUE, comma-separated) [{}]: ",
                    env_default
                ))?;
                if !env_input.is_empty() {
                    server.env = Some(parse_key_value_pairs(
                        &env_input,
                        "env entry",
                        "Environment variable name",
                    )?);
                } else if !is_edit {
                    server.env = None;
                }
            }
        }
        _ => {
            server.command = None;
            server.args = None;
            server.env = None;
            let base_prompt = if is_edit {
                format!(
                    "Base URL [{}]: ",
                    server.base_url.as_deref().unwrap_or_default()
                )
            } else {
                "Base URL: ".to_string()
            };
            let base_input = prompt_optional(&base_prompt)?;
            if is_edit {
                if !base_input.is_empty() {
                    server.base_url = Some(base_input);
                }
                if server.base_url.as_deref().unwrap_or_default().is_empty() {
                    return Err("Base URL cannot be empty for HTTP transport.".into());
                }
            } else if base_input.is_empty() {
                return Err("Base URL cannot be empty for HTTP transport.".into());
            } else {
                server.base_url = Some(base_input);
            }

            if advanced {
                let headers_default = server
                    .headers
                    .as_ref()
                    .map(format_key_value_pairs)
                    .unwrap_or_default();
                let headers_input = prompt_optional(&format!(
                    "HTTP headers (KEY=VALUE, comma-separated, optional) [{}]: ",
                    headers_default
                ))?;
                if !headers_input.is_empty() {
                    server.headers = Some(parse_key_value_pairs(
                        &headers_input,
                        "header entry",
                        "Header name",
                    )?);
                } else if !is_edit {
                    server.headers = None;
                }
            }
        }
    }

    if advanced {
        let protocol_default = server.protocol_version.as_deref().unwrap_or_default();
        let protocol_input = prompt_optional(&format!(
            "Protocol version (optional) [{}]: ",
            protocol_default
        ))?;
        if !protocol_input.is_empty() {
            server.protocol_version = Some(protocol_input);
        } else if !is_edit {
            server.protocol_version = None;
        }

        let tools_default = server
            .allowed_tools
            .as_ref()
            .map(|tools| tools.join(","))
            .unwrap_or_default();
        let tools_input = prompt_optional(&format!(
            "Allowed tools (comma-separated, optional) [{}]: ",
            tools_default
        ))?;
        if !tools_input.is_empty() {
            let tools: Vec<String> = tools_input
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .collect();
            server.allowed_tools = if tools.is_empty() { None } else { Some(tools) };
        } else if !is_edit {
            server.allowed_tools = None;
        }
    }

    Ok(())
}

pub(super) fn parse_key_value_pairs(
    input: &str,
    entry_label: &str,
    key_label: &str,
) -> Result<std::collections::HashMap<String, String>, Box<dyn Error>> {
    let mut pairs = std::collections::HashMap::new();
    for pair in input
        .split(',')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
    {
        let Some((key, value)) = pair.split_once('=') else {
            return Err(format!("Invalid {entry_label} '{pair}'. Expected KEY=VALUE.").into());
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(format!("{key_label} cannot be empty.").into());
        }
        pairs.insert(key.to_string(), value.trim().to_string());
    }
    Ok(pairs)
}

fn format_key_value_pairs(pairs: &std::collections::HashMap<String, String>) -> String {
    let mut formatted: Vec<String> = pairs
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    formatted.sort();
    formatted.join(",")
}

pub(super) fn sanitize_optional_oauth_endpoint(
    endpoint_name: &str,
    endpoint: &mut Option<String>,
    invalid_endpoint_message: &str,
) {
    if let Some(endpoint_value) = endpoint.as_deref() {
        if let Err(err) = validate_oauth_endpoint_url(endpoint_name, endpoint_value) {
            eprintln!("⚠️ {err} {invalid_endpoint_message}");
            *endpoint = None;
        }
    }
}

fn resolve_mcp_server<'a>(
    config: &'a Config,
    input: &str,
) -> Result<&'a McpServerConfig, Box<dyn Error>> {
    if let Some(server) = config.get_mcp_server(input) {
        return Ok(server);
    }

    let known: Vec<String> = config
        .list_mcp_servers()
        .iter()
        .map(|server| format!("{} ({})", server.display_name, server.id))
        .collect();

    eprintln!("❌ MCP server '{}' not found.", input);
    if known.is_empty() {
        eprintln!("   Add MCP servers to config.toml first.");
    } else {
        eprintln!("   Available MCP servers:");
        for server in known {
            eprintln!("   {}", server);
        }
    }
    std::process::exit(1);
}
