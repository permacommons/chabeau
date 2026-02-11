use std::error::Error;
use std::io::Read;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::core::builtin_oauth::{render_oauth_callback_page, OAuthCallbackVariant};
use crate::core::config::data::McpServerConfig;
use crate::core::mcp_auth::{McpOAuthGrant, McpTokenStore};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct OAuthMetadata {
    pub authorization_endpoint: Option<String>,
    pub token_endpoint: Option<String>,
    pub revocation_endpoint: Option<String>,
    pub issuer: Option<String>,
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub scopes_supported: Option<Vec<String>>,
    #[serde(default)]
    pub authorization_servers: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize)]
pub struct OAuthTokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct OAuthClientRegistrationResponse {
    client_id: String,
}

const OAUTH_REFRESH_SAFETY_WINDOW_S: i64 = 60;

pub struct AuthorizationUrlParams<'a> {
    pub authorization_endpoint: &'a str,
    pub client_id: Option<&'a str>,
    pub redirect_uri: &'a str,
    pub state: &'a str,
    pub code_challenge: &'a str,
    pub code_challenge_method: &'a str,
    pub issuer: Option<&'a str>,
    pub scope: Option<&'a str>,
}

pub fn current_unix_epoch_s() -> Option<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs() as i64)
}

fn oauth_grant_needs_refresh(
    expires_at_epoch_s: Option<i64>,
    now_epoch_s: i64,
    safety_window_s: i64,
) -> bool {
    match expires_at_epoch_s {
        Some(expires_at) => expires_at <= now_epoch_s.saturating_add(safety_window_s),
        None => false,
    }
}

pub fn apply_oauth_token_response(
    existing_grant: &McpOAuthGrant,
    token: OAuthTokenResponse,
    now_epoch_s: i64,
) -> McpOAuthGrant {
    let expires_at_epoch_s = token
        .expires_in
        .and_then(|seconds| now_epoch_s.checked_add(seconds));

    McpOAuthGrant {
        access_token: token.access_token,
        refresh_token: token
            .refresh_token
            .or_else(|| existing_grant.refresh_token.clone()),
        token_type: token
            .token_type
            .or_else(|| existing_grant.token_type.clone()),
        scope: token.scope.or_else(|| existing_grant.scope.clone()),
        expires_at_epoch_s,
        client_id: existing_grant.client_id.clone(),
        redirect_uri: existing_grant.redirect_uri.clone(),
        authorization_endpoint: existing_grant.authorization_endpoint.clone(),
        token_endpoint: existing_grant.token_endpoint.clone(),
        revocation_endpoint: existing_grant.revocation_endpoint.clone(),
        issuer: existing_grant.issuer.clone(),
    }
}

async fn refresh_oauth_access_token(
    token_endpoint: &str,
    refresh_token: &str,
    client_id: Option<&str>,
) -> Result<OAuthTokenResponse, Box<dyn Error>> {
    let mut form_fields = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
    ];
    if let Some(client_id) = client_id.filter(|value| !value.trim().is_empty()) {
        form_fields.push(("client_id", client_id));
    }

    let response = reqwest::Client::new()
        .post(token_endpoint)
        .form(&form_fields)
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("OAuth refresh failed ({status}): {text}").into());
    }

    Ok(response.json::<OAuthTokenResponse>().await?)
}

pub async fn refresh_oauth_grant_if_needed(
    server_id: &str,
    token_store: &McpTokenStore,
) -> Result<Option<String>, Box<dyn Error>> {
    let Some(grant) = token_store.get_oauth_grant(server_id)? else {
        return Ok(None);
    };

    let Some(now_epoch_s) = current_unix_epoch_s() else {
        return Ok(Some(grant.access_token));
    };

    if !oauth_grant_needs_refresh(
        grant.expires_at_epoch_s,
        now_epoch_s,
        OAUTH_REFRESH_SAFETY_WINDOW_S,
    ) {
        return Ok(Some(grant.access_token));
    }

    let Some(refresh_token) = grant.refresh_token.clone() else {
        return Ok(Some(grant.access_token));
    };

    let token_endpoint = grant
        .token_endpoint
        .as_deref()
        .ok_or("OAuth grant is missing token endpoint; re-auth required.")?;

    let token =
        refresh_oauth_access_token(token_endpoint, &refresh_token, grant.client_id.as_deref())
            .await?;
    let updated_grant = apply_oauth_token_response(&grant, token, now_epoch_s);
    token_store.set_oauth_grant(server_id, &updated_grant)?;
    token_store.set_token(server_id, &updated_grant.access_token)?;
    Ok(Some(updated_grant.access_token))
}

pub async fn probe_oauth_support(
    server: &McpServerConfig,
) -> Result<Option<OAuthMetadata>, Box<dyn Error>> {
    let Some(base_url) = server.base_url.as_deref() else {
        return Ok(None);
    };
    if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
        return Ok(None);
    }

    let url = reqwest::Url::parse(base_url)?;
    let scheme = url.scheme();
    let Some(host) = url.host_str() else {
        return Ok(None);
    };
    let authority = if let Some(port) = url.port() {
        format!("{host}:{port}")
    } else {
        host.to_string()
    };
    let origin = format!("{scheme}://{authority}");
    let candidates = [
        format!("{origin}/.well-known/oauth-authorization-server"),
        format!("{origin}/.well-known/openid-configuration"),
        format!("{origin}/.well-known/oauth-protected-resource"),
    ];

    let client = reqwest::Client::new();
    for candidate in candidates {
        let Some(metadata) = fetch_oauth_metadata(&client, &candidate).await else {
            continue;
        };
        if metadata.authorization_endpoint.is_some()
            || metadata.token_endpoint.is_some()
            || metadata.revocation_endpoint.is_some()
        {
            return Ok(Some(metadata));
        }
        if let Some(servers) = metadata.authorization_servers.as_ref() {
            for issuer in servers {
                let issuer = issuer.trim_end_matches('/');
                let auth_server_well_known =
                    format!("{issuer}/.well-known/oauth-authorization-server");
                if let Some(mut delegated) =
                    fetch_oauth_metadata(&client, &auth_server_well_known).await
                {
                    if delegated.issuer.is_none() {
                        delegated.issuer = Some(issuer.to_string());
                    }
                    if delegated.authorization_endpoint.is_some()
                        || delegated.token_endpoint.is_some()
                        || delegated.revocation_endpoint.is_some()
                    {
                        return Ok(Some(delegated));
                    }
                }
            }
        }
    }

    Ok(None)
}

async fn fetch_oauth_metadata(client: &reqwest::Client, url: &str) -> Option<OAuthMetadata> {
    let response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<OAuthMetadata>().await.ok()
}

pub fn open_in_browser(url: &str) -> Result<(), Box<dyn Error>> {
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("open").arg(url).status()?;
        if status.success() {
            return Ok(());
        }
        return Err("failed to launch browser with open".into());
    }
    #[cfg(target_os = "windows")]
    {
        let status = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .status()?;
        if status.success() {
            return Ok(());
        }
        return Err("failed to launch browser with start".into());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let status = std::process::Command::new("xdg-open").arg(url).status()?;
        if status.success() {
            return Ok(());
        }
        return Err("failed to launch browser with xdg-open".into());
    }

    #[allow(unreachable_code)]
    Err(format!("no browser launcher configured for URL: {url}").into())
}

pub fn random_urlsafe(bytes_len: usize) -> String {
    let bytes = best_effort_random_bytes(bytes_len);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn pkce_s256_challenge(verifier: &str) -> String {
    let digest = sha256_digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn best_effort_random_bytes(len: usize) -> Vec<u8> {
    let mut out = vec![0_u8; len];

    #[cfg(unix)]
    {
        if let Ok(mut file) = std::fs::File::open("/dev/urandom") {
            if file.read_exact(&mut out).is_ok() {
                return out;
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        if windows_fill_random(&mut out) {
            return out;
        }
    }

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut x = nanos ^ ((std::process::id() as u64) << 32) ^ (len as u64);
    for byte in &mut out {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *byte = (x & 0xFF) as u8;
    }
    out
}

#[cfg(target_os = "windows")]
fn windows_fill_random(buffer: &mut [u8]) -> bool {
    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptGenRandom(
            h_algorithm: usize,
            pb_buffer: *mut u8,
            cb_buffer: u32,
            dw_flags: u32,
        ) -> i32;
    }

    const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x00000002;
    let status = unsafe {
        BCryptGenRandom(
            0,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    status == 0
}

fn sha256_digest(input: &[u8]) -> [u8; 32] {
    const H0: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let mut message = input.to_vec();
    let bit_len = (message.len() as u64) * 8;
    message.push(0x80);
    while (message.len() % 64) != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    let mut h = H0;
    for chunk in message.chunks_exact(64) {
        let mut w = [0_u32; 64];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            let base = i * 4;
            *word = u32::from_be_bytes([
                chunk[base],
                chunk[base + 1],
                chunk[base + 2],
                chunk[base + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0_u8; 32];
    for (index, word) in h.iter().enumerate() {
        out[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

pub fn build_authorization_url(
    params: AuthorizationUrlParams<'_>,
) -> Result<reqwest::Url, Box<dyn Error>> {
    let mut url = reqwest::Url::parse(params.authorization_endpoint)?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("response_type", "code");
        if let Some(client_id) = params.client_id.filter(|value| !value.trim().is_empty()) {
            query.append_pair("client_id", client_id);
        }
        query.append_pair("redirect_uri", params.redirect_uri);
        query.append_pair("state", params.state);
        query.append_pair("code_challenge", params.code_challenge);
        query.append_pair("code_challenge_method", params.code_challenge_method);
        if let Some(scope) = params.scope.filter(|value| !value.trim().is_empty()) {
            query.append_pair("scope", scope);
        }
        if let Some(issuer) = params.issuer {
            query.append_pair("resource", issuer);
        }
    }
    Ok(url)
}

pub async fn wait_for_oauth_callback(
    listener: TcpListener,
    expected_state: &str,
) -> Result<String, Box<dyn Error>> {
    let (mut stream, _) =
        tokio::time::timeout(Duration::from_secs(300), listener.accept()).await??;
    let mut buffer = vec![0_u8; 16 * 1024];
    let bytes_read = stream.read(&mut buffer).await?;
    if bytes_read == 0 {
        return Err("OAuth callback received no data".into());
    }
    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    let first_line = request
        .lines()
        .next()
        .ok_or("OAuth callback request line missing")?;
    let mut parts = first_line.split_whitespace();
    let _method = parts.next().ok_or("OAuth callback method missing")?;
    let target = parts.next().ok_or("OAuth callback target missing")?;
    let callback_url = reqwest::Url::parse(&format!("http://localhost{target}"))?;

    let mut state = None::<String>;
    let mut code = None::<String>;
    let mut error = None::<String>;
    let mut error_description = None::<String>;
    for (key, value) in callback_url.query_pairs() {
        match key.as_ref() {
            "state" => state = Some(value.to_string()),
            "code" => code = Some(value.to_string()),
            "error" => error = Some(value.to_string()),
            "error_description" => error_description = Some(value.to_string()),
            _ => {}
        }
    }

    if let Some(error) = error {
        write_oauth_callback_response(
            &mut stream,
            "400 Bad Request",
            "OAuth authorization failed",
            "The identity provider rejected the authorization request. Close this tab and retry in Chabeau.",
            OAuthCallbackVariant::Error,
        )
        .await?;
        let detail = error_description.unwrap_or_default();
        return Err(format!("OAuth callback error: {error} {detail}").into());
    }

    if state.as_deref() != Some(expected_state) {
        write_oauth_callback_response(
            &mut stream,
            "400 Bad Request",
            "OAuth state validation failed",
            "The callback state token did not match this session. Close this tab and retry in Chabeau.",
            OAuthCallbackVariant::Error,
        )
        .await?;
        return Err("OAuth callback state mismatch".into());
    }

    let Some(code) = code else {
        write_oauth_callback_response(
            &mut stream,
            "400 Bad Request",
            "OAuth callback missing code",
            "The callback response did not include an authorization code. Close this tab and retry in Chabeau.",
            OAuthCallbackVariant::Error,
        )
        .await?;
        return Err("OAuth callback missing code".into());
    };

    write_oauth_callback_response(
        &mut stream,
        "200 OK",
        "You're signed in to Chabeau",
        "OAuth authorization completed successfully. Close this tab and return to Chabeau.",
        OAuthCallbackVariant::Success,
    )
    .await?;
    Ok(code)
}

async fn write_oauth_callback_response(
    stream: &mut tokio::net::TcpStream,
    status: &str,
    heading: &str,
    detail: &str,
    variant: OAuthCallbackVariant,
) -> Result<(), Box<dyn Error>> {
    let body = render_oauth_callback_page("Chabeau OAuth", heading, detail, variant);
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

pub async fn exchange_oauth_code(
    token_endpoint: &str,
    client_id: Option<&str>,
    redirect_uri: &str,
    code: &str,
    code_verifier: &str,
) -> Result<OAuthTokenResponse, Box<dyn Error>> {
    let mut form_fields = vec![
        ("grant_type", "authorization_code"),
        ("redirect_uri", redirect_uri),
        ("code", code),
        ("code_verifier", code_verifier),
    ];
    if let Some(client_id) = client_id.filter(|value| !value.trim().is_empty()) {
        form_fields.push(("client_id", client_id));
    }
    let response = reqwest::Client::new()
        .post(token_endpoint)
        .form(&form_fields)
        .send()
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("OAuth token exchange failed ({status}): {text}").into());
    }
    let token = response.json::<OAuthTokenResponse>().await?;
    Ok(token)
}

pub async fn register_oauth_client(
    registration_endpoint: &str,
    redirect_uri: &str,
) -> Result<String, Box<dyn Error>> {
    let payload = serde_json::json!({
        "client_name": "chabeau",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none"
    });
    let response = reqwest::Client::new()
        .post(registration_endpoint)
        .json(&payload)
        .send()
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("registration failed ({status}): {body}").into());
    }
    let data = response.json::<OAuthClientRegistrationResponse>().await?;
    Ok(data.client_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_random_urlsafe_is_urlsafe() {
        let token = random_urlsafe(32);
        assert!(token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'));
        assert!(!token.contains('='));
    }

    #[test]
    fn test_pkce_s256_matches_rfc_example() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = pkce_s256_challenge(verifier);
        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn test_build_authorization_url_includes_required_params() {
        let url = build_authorization_url(AuthorizationUrlParams {
            authorization_endpoint: "https://auth.example.com/authorize",
            client_id: Some("chabeau"),
            redirect_uri: "http://127.0.0.1:7777/oauth/callback",
            state: "state123",
            code_challenge: "challenge123",
            code_challenge_method: "S256",
            issuer: None,
            scope: None,
        })
        .expect("authorization URL should build");
        let params: std::collections::HashMap<String, String> =
            url.query_pairs().into_owned().collect();
        assert_eq!(params.get("response_type"), Some(&"code".to_string()));
        assert_eq!(params.get("client_id"), Some(&"chabeau".to_string()));
        assert_eq!(
            params.get("redirect_uri"),
            Some(&"http://127.0.0.1:7777/oauth/callback".to_string())
        );
        assert_eq!(params.get("state"), Some(&"state123".to_string()));
        assert_eq!(
            params.get("code_challenge"),
            Some(&"challenge123".to_string())
        );
        assert_eq!(
            params.get("code_challenge_method"),
            Some(&"S256".to_string())
        );
    }

    #[test]
    fn test_build_authorization_url_omits_empty_client_id() {
        let url = build_authorization_url(AuthorizationUrlParams {
            authorization_endpoint: "https://auth.example.com/authorize",
            client_id: None,
            redirect_uri: "http://127.0.0.1:7777/oauth/callback",
            state: "state123",
            code_challenge: "challenge123",
            code_challenge_method: "S256",
            issuer: None,
            scope: None,
        })
        .expect("authorization URL should build");
        let params: std::collections::HashMap<String, String> =
            url.query_pairs().into_owned().collect();
        assert!(!params.contains_key("client_id"));
    }

    #[test]
    fn test_build_authorization_url_includes_scope() {
        let url = build_authorization_url(AuthorizationUrlParams {
            authorization_endpoint: "https://auth.example.com/authorize",
            client_id: Some("chabeau"),
            redirect_uri: "http://127.0.0.1:7777/oauth/callback",
            state: "state123",
            code_challenge: "challenge123",
            code_challenge_method: "S256",
            issuer: None,
            scope: Some("mcp.read mcp.write"),
        })
        .expect("authorization URL should build");
        let params: std::collections::HashMap<String, String> =
            url.query_pairs().into_owned().collect();
        assert_eq!(params.get("scope"), Some(&"mcp.read mcp.write".to_string()));
    }

    #[test]
    fn test_oauth_grant_needs_refresh_with_safety_window() {
        assert!(oauth_grant_needs_refresh(Some(160), 100, 60));
        assert!(oauth_grant_needs_refresh(Some(100), 100, 60));
        assert!(!oauth_grant_needs_refresh(Some(161), 100, 60));
        assert!(!oauth_grant_needs_refresh(None, 100, 60));
    }

    #[test]
    fn test_apply_oauth_token_response_preserves_existing_refresh_token() {
        let grant = McpOAuthGrant {
            access_token: "old-access".to_string(),
            refresh_token: Some("existing-refresh".to_string()),
            token_type: Some("Bearer".to_string()),
            scope: Some("mcp.read".to_string()),
            expires_at_epoch_s: Some(10),
            client_id: Some("client-id".to_string()),
            redirect_uri: Some("http://127.0.0.1/callback".to_string()),
            authorization_endpoint: Some("https://auth.example.com/authorize".to_string()),
            token_endpoint: Some("https://auth.example.com/token".to_string()),
            revocation_endpoint: Some("https://auth.example.com/revoke".to_string()),
            issuer: Some("https://auth.example.com".to_string()),
        };
        let token = OAuthTokenResponse {
            access_token: "new-access".to_string(),
            token_type: None,
            expires_in: Some(120),
            refresh_token: None,
            scope: None,
        };

        let updated = apply_oauth_token_response(&grant, token, 1_000);
        assert_eq!(updated.access_token, "new-access");
        assert_eq!(updated.refresh_token.as_deref(), Some("existing-refresh"));
        assert_eq!(updated.expires_at_epoch_s, Some(1_120));
        assert_eq!(updated.client_id.as_deref(), Some("client-id"));
    }

    #[test]
    fn test_refresh_token_response_deserializes_expected_fields() {
        let token: OAuthTokenResponse = serde_json::from_str(
            r#"{"access_token":"new-token","refresh_token":"new-refresh","expires_in":120}"#,
        )
        .expect("token response should deserialize");

        assert_eq!(token.access_token, "new-token");
        assert_eq!(token.refresh_token.as_deref(), Some("new-refresh"));
        assert_eq!(token.expires_in, Some(120));
    }
}
