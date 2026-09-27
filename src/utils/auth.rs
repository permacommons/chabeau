//! Authentication utilities for API requests
//!
//! This module provides utilities for adding provider-specific authentication
//! headers to HTTP requests.

/// Add provider-specific authentication headers to an HTTP request
///
/// This function handles the different authentication schemes used by various providers:
/// - Anthropic: Uses `x-api-key` header with `anthropic-version`
/// - All others: Use standard `Authorization: Bearer` header
///
/// # Arguments
/// * `request` - The reqwest RequestBuilder to add headers to
/// * `auth_mode` - The resolved authentication mode for the provider
/// * `api_key` - The API key to use for authentication
///
/// # Returns
/// The RequestBuilder with appropriate authentication headers added
pub fn add_auth_headers(
    request: reqwest::RequestBuilder,
    auth_mode: &str,
    api_key: &str,
) -> reqwest::RequestBuilder {
    if auth_mode.eq_ignore_ascii_case("anthropic") {
        return request
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01");
    }

    // Default to OpenAI-style authentication for all other providers
    request.header("Authorization", format!("Bearer {api_key}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anthropic_auth_headers() {
        let client = reqwest::Client::new();
        let request = client.get("https://example.com");

        let request_with_auth = add_auth_headers(request, "anthropic", "test-key");
        let final_request = request_with_auth.build().unwrap();
        let headers = final_request.headers();

        assert_eq!(
            headers
                .get("x-api-key")
                .and_then(|value| value.to_str().ok()),
            Some("test-key")
        );
        assert_eq!(
            headers
                .get("anthropic-version")
                .and_then(|value| value.to_str().ok()),
            Some("2023-06-01")
        );
        assert!(headers.get("Authorization").is_none());
    }

    #[test]
    fn test_openai_auth_headers() {
        let client = reqwest::Client::new();
        let request = client.get("https://example.com");

        let request_with_auth = add_auth_headers(request, "openai", "test-key");
        let final_request = request_with_auth.build().unwrap();
        let headers = final_request.headers();

        assert_eq!(
            headers
                .get("Authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer test-key")
        );
        assert!(headers.get("x-api-key").is_none());
        assert!(headers.get("anthropic-version").is_none());
    }

    #[test]
    fn test_custom_provider_auth_headers() {
        let client = reqwest::Client::new();
        let request = client.get("https://example.com");

        let request_with_auth = add_auth_headers(request, "custom-provider", "test-key");
        let final_request = request_with_auth.build().unwrap();
        let headers = final_request.headers();

        assert_eq!(
            headers
                .get("Authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer test-key")
        );
        assert!(headers.get("x-api-key").is_none());
        assert!(headers.get("anthropic-version").is_none());
    }

    #[test]
    fn test_custom_anthropic_mode_auth_headers() {
        let mut config = crate::core::config::data::Config::default();
        config.add_custom_provider(crate::core::config::data::CustomProvider::new(
            "custom-anthropic".into(),
            "Custom Anthropic".into(),
            "https://example.com/v1".into(),
            Some("anthropic".into()),
        ));
        let request = reqwest::Client::new().get("https://example.com");
        let auth_mode = config.provider_auth_mode("custom-anthropic");
        let final_request = add_auth_headers(request, &auth_mode, "custom-key")
            .build()
            .unwrap();
        let headers = final_request.headers();

        assert_eq!(headers.get("x-api-key").unwrap(), "custom-key");
        assert_eq!(headers.get("anthropic-version").unwrap(), "2023-06-01");
        assert!(headers.get("Authorization").is_none());
    }
}
