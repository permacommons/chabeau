use crate::api::ModelsResponse;
use crate::core::builtin_providers::find_builtin_provider;
use crate::utils::url::construct_api_url;

pub async fn fetch_models(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    provider_name: &str,
) -> Result<ModelsResponse, Box<dyn std::error::Error>> {
    let models_url = construct_api_url(base_url, "models");
    let mut request = client
        .get(models_url)
        .header("Content-Type", "application/json");

    // Handle provider-specific authentication headers
    // Check if this is a built-in provider with special authentication mode
    if let Some(builtin_provider) = find_builtin_provider(provider_name) {
        if builtin_provider.is_anthropic_mode() {
            request = request
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01");
        } else {
            request = request.header("Authorization", format!("Bearer {api_key}"));
        }
    } else {
        // For custom providers, default to OpenAI-style authentication
        request = request.header("Authorization", format!("Bearer {api_key}"));
    }

    let response = request.send().await?;

    if !response.status().is_success() {
        let status = response.status();
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!("API request failed with status {status}: {error_text}").into());
    }

    let models_response = response.json::<ModelsResponse>().await?;
    Ok(models_response)
}

pub fn sort_models(models: &mut [crate::api::ModelInfo]) {
    // Sort models by creation date (newest first), then by ID for consistent display
    models.sort_by(|a, b| {
        // First sort by creation date (newest first)
        // Handle both created (OpenAI-style) and created_at (Anthropic-style) fields
        match (&a.created, &b.created, &a.created_at, &b.created_at) {
            // Both have created (OpenAI-style)
            (Some(a_created), Some(b_created), _, _) => b_created.cmp(a_created),
            // Only a has created
            (Some(_), None, _, _) => std::cmp::Ordering::Less,
            // Only b has created
            (None, Some(_), _, _) => std::cmp::Ordering::Greater,
            // Neither has created, check created_at (Anthropic-style)
            (None, None, Some(a_created_at), Some(b_created_at)) => {
                // For Anthropic, we want newest first, so we reverse the comparison
                b_created_at.cmp(a_created_at)
            }
            // Only a has created_at
            (None, None, Some(_), None) => std::cmp::Ordering::Less,
            // Only b has created_at
            (None, None, None, Some(_)) => std::cmp::Ordering::Greater,
            // Neither has any creation date, fall back to ID sorting
            (None, None, None, None) => b.id.cmp(&a.id), // Reverse for newest first
        }
    });
}
