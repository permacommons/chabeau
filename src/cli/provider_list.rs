use std::error::Error;

use crate::{
    auth::{AuthManager, ProviderAuthStatus},
    core::message::{Message, ROLE_ASSISTANT},
    core::{builtin_providers::find_builtin_provider, config::data::Config},
    ui::{
        layout::TableOverflowPolicy,
        markdown::{self, MessageRenderConfig},
        theme::Theme,
    },
};
use ratatui::crossterm::terminal;

pub async fn list_providers() -> Result<(), Box<dyn Error>> {
    let auth_manager = AuthManager::new()?;
    let config = Config::load()?;
    let (providers, default_provider) = auth_manager.get_all_providers_with_auth_status();

    if providers.is_empty() {
        println!("No providers configured.");
        return Ok(());
    }

    let mut content = String::from("Configured Providers:\n\n");
    let mut builtin_providers = Vec::new();
    let mut custom_providers = Vec::new();
    for provider in providers {
        let is_builtin = find_builtin_provider(&provider.id).is_some()
            && config.get_custom_provider(&provider.id).is_none();
        if is_builtin {
            builtin_providers.push(provider);
        } else {
            custom_providers.push(provider);
        }
    }

    if !builtin_providers.is_empty() {
        content.push_str("Built-in providers:\n\n");
        content.push_str(&provider_table(
            &builtin_providers,
            default_provider.as_deref(),
        ));
        content.push('\n');
    }
    if !custom_providers.is_empty() {
        content.push_str("Custom providers:\n\n");
        content.push_str(&provider_table(
            &custom_providers,
            default_provider.as_deref(),
        ));
    }

    if default_provider.is_some() {
        content.push_str("\n\\* = default provider");
    }

    let monochrome_theme = Theme::monochrome();
    let terminal_width = terminal::size().ok().map(|(w, _)| w as usize);
    let rendered = markdown::render_message_with_config(
        &Message {
            role: ROLE_ASSISTANT.to_string(),
            content,
        },
        &monochrome_theme,
        MessageRenderConfig::markdown(true, true)
            .with_terminal_width(terminal_width, TableOverflowPolicy::WrapCells),
    );

    for line in rendered.lines {
        println!("{}", line);
    }

    Ok(())
}

fn provider_table(providers: &[ProviderAuthStatus], default_provider: Option<&str>) -> String {
    let mut table = String::new();
    table.push_str("| Provider | Display Name | URL | Authenticated |\n");
    table.push_str("|---|---|---|:---:|\n");

    for provider in providers {
        let auth_status = if provider.has_token { "✅" } else { "❌" };
        let provider_id = if default_provider.is_some_and(|d| d.eq_ignore_ascii_case(&provider.id))
        {
            format!("{}*", provider.id)
        } else {
            provider.id.clone()
        };

        table.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            provider_id, provider.display_name, provider.base_url, auth_status
        ));
    }
    table
}
