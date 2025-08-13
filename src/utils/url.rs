//! URL utilities for consistent URL handling
//!
//! This module provides utilities for normalizing URLs to prevent issues
//! with trailing slashes when constructing API endpoints.

/// Normalize a base URL by removing trailing slashes
///
/// This ensures consistent URL construction when appending endpoints,
/// preventing double slashes in the final URLs.
///
/// # Examples
///
/// ```
/// use chabeau::utils::url::normalize_base_url;
///
/// assert_eq!(normalize_base_url("https://api.example.com/v1"), "https://api.example.com/v1");
/// assert_eq!(normalize_base_url("https://api.example.com/v1/"), "https://api.example.com/v1");
/// assert_eq!(normalize_base_url("https://api.example.com/v1///"), "https://api.example.com/v1");
/// ```
pub fn normalize_base_url(base_url: &str) -> String {
    base_url.trim_end_matches('/').to_string()
}

/// Construct a complete API endpoint URL from a base URL and endpoint path
///
/// This function normalizes the base URL and safely appends the endpoint,
/// ensuring there are no double slashes in the result.
///
/// # Examples
///
/// ```
/// use chabeau::utils::url::construct_api_url;
///
/// assert_eq!(
///     construct_api_url("https://api.example.com/v1", "chat/completions"),
///     "https://api.example.com/v1/chat/completions"
/// );
/// assert_eq!(
///     construct_api_url("https://api.example.com/v1/", "chat/completions"),
///     "https://api.example.com/v1/chat/completions"
/// );
/// ```
pub fn construct_api_url(base_url: &str, endpoint: &str) -> String {
    let normalized_base = normalize_base_url(base_url);
    let endpoint = endpoint.trim_start_matches('/');
    format!("{}/{}", normalized_base, endpoint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_base_url() {
        // No trailing slash - should remain unchanged
        assert_eq!(
            normalize_base_url("https://api.example.com/v1"),
            "https://api.example.com/v1"
        );

        // Single trailing slash - should be removed
        assert_eq!(
            normalize_base_url("https://api.example.com/v1/"),
            "https://api.example.com/v1"
        );

        // Multiple trailing slashes - should all be removed
        assert_eq!(
            normalize_base_url("https://api.example.com/v1///"),
            "https://api.example.com/v1"
        );

        // Root URL with trailing slash
        assert_eq!(
            normalize_base_url("https://api.example.com/"),
            "https://api.example.com"
        );

        // Root URL without trailing slash
        assert_eq!(
            normalize_base_url("https://api.example.com"),
            "https://api.example.com"
        );

        // Empty string
        assert_eq!(normalize_base_url(""), "");

        // Just slashes
        assert_eq!(normalize_base_url("///"), "");
    }

    #[test]
    fn test_construct_api_url() {
        // Normal case - no trailing slash on base URL
        assert_eq!(
            construct_api_url("https://api.example.com/v1", "chat/completions"),
            "https://api.example.com/v1/chat/completions"
        );

        // Base URL with trailing slash
        assert_eq!(
            construct_api_url("https://api.example.com/v1/", "chat/completions"),
            "https://api.example.com/v1/chat/completions"
        );

        // Endpoint with leading slash
        assert_eq!(
            construct_api_url("https://api.example.com/v1", "/chat/completions"),
            "https://api.example.com/v1/chat/completions"
        );

        // Both base URL with trailing slash and endpoint with leading slash
        assert_eq!(
            construct_api_url("https://api.example.com/v1/", "/chat/completions"),
            "https://api.example.com/v1/chat/completions"
        );

        // Multiple trailing slashes on base URL
        assert_eq!(
            construct_api_url("https://api.example.com/v1///", "models"),
            "https://api.example.com/v1/models"
        );

        // Multiple leading slashes on endpoint
        assert_eq!(
            construct_api_url("https://api.example.com/v1", "///models"),
            "https://api.example.com/v1/models"
        );

        // Test with models endpoint
        assert_eq!(
            construct_api_url("https://api.openai.com/v1/", "models"),
            "https://api.openai.com/v1/models"
        );
    }
}
