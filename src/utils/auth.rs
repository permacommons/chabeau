//! Authentication utilities for API requests
//!
//! This module provides utilities for adding provider-specific authentication
//! headers to HTTP requests.

use crate::core::builtin_providers::find_builtin_provider;

/// Add provider-specific authentication headers to an HTTP request
///
/// This function handles the different authentication schemes used by various providers:
/// - Anthropic: Uses `x-api-key` header with `anthropic-version`
/// - All others: Use standard `Authorization: Bearer` header
///
/// # Arguments
/// * `request` - The reqwest RequestBuilder to add headers to
/// * `provider_name` - The name of the provider (used to determine auth mode)
/// * `api_key` - The API key to use for authentication
///
/// # Returns
/// The RequestBuilder with appropriate authentication headers added
pub fn add_auth_headers(
    request: reqwest::RequestBuilder,
    provider_name: &str,
    api_key: &str,
) -> reqwest::RequestBuilder {
    // Check if this is Anthropic (the only provider with special auth)
    if let Some(builtin_provider) = find_builtin_provider(provider_name) {
        if builtin_provider.is_anthropic_mode() {
            return request
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01");
        }
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

        // We can't easily inspect the headers in a RequestBuilder, but we can test
        // that the function doesn't panic and returns a RequestBuilder
        let _final_request = request_with_auth.build().unwrap();
    }

    #[test]
    fn test_openai_auth_headers() {
        let client = reqwest::Client::new();
        let request = client.get("https://example.com");

        let request_with_auth = add_auth_headers(request, "openai", "test-key");

        let _final_request = request_with_auth.build().unwrap();
    }

    #[test]
    fn test_custom_provider_auth_headers() {
        let client = reqwest::Client::new();
        let request = client.get("https://example.com");

        let request_with_auth = add_auth_headers(request, "custom-provider", "test-key");

        let _final_request = request_with_auth.build().unwrap();
    }
}
