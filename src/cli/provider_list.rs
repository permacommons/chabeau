use std::error::Error;

use crate::{
    auth::{AuthManager, ProviderAuthStatus},
    core::message::{Message, ROLE_ASSISTANT},
    ui::{
        layout::TableOverflowPolicy,
        markdown::{self, MessageRenderConfig},
        theme::Theme,
    },
};
use ratatui::crossterm::terminal;

pub async fn list_providers() -> Result<(), Box<dyn Error>> {
    let auth_manager = AuthManager::new()?;
    let (providers, default_provider) = auth_manager.get_all_providers_with_auth_status();

    if providers.is_empty() {
        println!("No providers configured.");
        return Ok(());
    }

    let mut content = String::from("Configured Providers:\n\n");

    let mut table = String::new();
    table.push_str("| Provider | Display Name | URL | Authenticated |\n");
    table.push_str("|---|---|---|:---:|\n");

    for ProviderAuthStatus {
        id,
        display_name,
        base_url,
        has_token,
    } in providers
    {
        let auth_status = if has_token { "✅" } else { "❌" };
        let provider_id = if default_provider
            .as_ref()
            .is_some_and(|d| d.eq_ignore_ascii_case(&id))
        {
            format!("{}*", id)
        } else {
            id
        };

        table.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            provider_id, display_name, base_url, auth_status
        ));
    }

    content.push_str(&table);

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
        MessageRenderConfig::markdown(true)
            .with_terminal_width(terminal_width, TableOverflowPolicy::WrapCells),
    );

    for line in rendered.lines {
        println!("{}", line);
    }

    Ok(())
}
