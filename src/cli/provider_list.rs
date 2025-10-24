use std::error::Error;

use crate::{
    auth::AuthManager,
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

    let mut table = String::new();
    table.push_str("| Provider | Display Name | Authenticated | Default |\n");
    table.push_str("|---|---|:---:|:---:|\n");

    for (id, display_name, has_token) in providers {
        let auth_status = if has_token { "✅" } else { "❌" };
        let is_default = default_provider
            .as_ref()
            .map_or(false, |d| d.eq_ignore_ascii_case(&id));
        let default_status = if is_default { "✓" } else { "" };
        table.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            id, display_name, auth_status, default_status
        ));
    }

    let monochrome_theme = Theme::monochrome();
    let terminal_width = terminal::size().ok().map(|(w, _)| w as usize);
    let rendered = markdown::render_message_with_config(
        &Message {
            role: ROLE_ASSISTANT.to_string(),
            content: table,
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
