use crate::utils::line_editor::{prompt_line_editor, LineEditorOptions, MaskMode};
use std::collections::HashSet;
use std::fmt;
use std::io::{self, Write};

const MASKED_INPUT_PROMPT: &str = "Enter your API token (press F2 to reveal last 4 chars): ";
const INVALID_CHOICE_MSG: &str = "Invalid choice";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderMenuItem {
    pub id: String,
    pub display_name: String,
    pub configured: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomProviderInput {
    pub display_name: String,
    pub provider_id: String,
    pub base_url: String,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMenuSelection {
    Provider(usize),
    Custom,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeauthMenuItem {
    pub id: String,
    pub display_name: String,
    pub is_custom: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeauthSelection {
    pub provider_id: String,
    pub is_custom: bool,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmationChoice {
    Yes,
    No,
    Cancel,
}

#[derive(Debug, Clone)]
pub struct UiError {
    message: String,
}

impl UiError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for UiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for UiError {}

pub fn prompt_auth_menu(providers: &[ProviderMenuItem]) -> Result<AuthMenuSelection, UiError> {
    println!("🔐 Chabeau Authentication Setup");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();

    println!("Available providers:");
    for (index, provider) in providers.iter().enumerate() {
        let status = if provider.configured {
            "✓ configured"
        } else {
            "not configured"
        };
        println!(
            "  {}. {} ({}) - {}",
            index + 1,
            provider.display_name,
            provider.id,
            status
        );
    }
    println!("  {}. Custom provider", providers.len() + 1);
    println!();

    print!("Select a provider (1-{}): ", providers.len() + 1);
    io::stdout()
        .flush()
        .map_err(|err| UiError::new(err.to_string()))?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|err| UiError::new(err.to_string()))?;

    parse_provider_selection(
        &input,
        providers.iter().map(|p| p.id.clone()).collect(),
        true,
        false,
    )
}

pub fn prompt_custom_provider_details<F>(
    existing_ids: &HashSet<String>,
    mut suggest_id: F,
) -> Result<CustomProviderInput, UiError>
where
    F: FnMut(&str) -> String,
{
    println!();
    print!("Enter a display name for your custom provider: ");
    io::stdout()
        .flush()
        .map_err(|err| UiError::new(err.to_string()))?;

    let mut display_name = String::new();
    io::stdin()
        .read_line(&mut display_name)
        .map_err(|err| UiError::new(err.to_string()))?;
    let display_name = display_name.trim();
    if display_name.is_empty() {
        return Err(UiError::new("Display name cannot be empty"));
    }

    let suggested_id = suggest_id(display_name);
    print!("Enter an ID for your provider [default: {suggested_id}]: ");
    io::stdout()
        .flush()
        .map_err(|err| UiError::new(err.to_string()))?;

    let mut id_input = String::new();
    io::stdin()
        .read_line(&mut id_input)
        .map_err(|err| UiError::new(err.to_string()))?;

    let provider_id = resolve_provider_id(id_input.trim(), &suggested_id, existing_ids)?;

    print!("Enter the API base URL (typically, https://some-url.example/api/v1): ");
    io::stdout()
        .flush()
        .map_err(|err| UiError::new(err.to_string()))?;

    let mut base_url = String::new();
    io::stdin()
        .read_line(&mut base_url)
        .map_err(|err| UiError::new(err.to_string()))?;
    let base_url = base_url.trim();
    if base_url.is_empty() {
        return Err(UiError::new("Base URL cannot be empty"));
    }

    let token = prompt_masked_input()?;

    if token.is_empty() {
        return Err(UiError::new("Token cannot be empty"));
    }

    Ok(CustomProviderInput {
        display_name: display_name.to_string(),
        provider_id,
        base_url: base_url.to_string(),
        token,
    })
}

pub fn prompt_provider_token(display_name: &str) -> Result<String, UiError> {
    println!();
    println!("Selected provider: {display_name}");
    let token = prompt_masked_input()?;
    if token.is_empty() {
        return Err(UiError::new("Token cannot be empty"));
    }
    Ok(token)
}

pub fn prompt_deauth_menu(
    providers: &[DeauthMenuItem],
) -> Result<Option<DeauthSelection>, UiError> {
    println!("🗑️  Chabeau Authentication Removal");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();

    if providers.is_empty() {
        println!("No configured providers found.");
        return Ok(None);
    }

    println!("Configured providers:");
    for (index, provider) in providers.iter().enumerate() {
        let provider_type = if provider.is_custom { " (custom)" } else { "" };
        println!(
            "  {}. {}{}",
            index + 1,
            provider.display_name,
            provider_type
        );
    }
    println!("  {}. Cancel", providers.len() + 1);
    println!();

    print!("Select a provider to remove (1-{}): ", providers.len() + 1);
    io::stdout()
        .flush()
        .map_err(|err| UiError::new(err.to_string()))?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|err| UiError::new(err.to_string()))?;

    match parse_provider_selection(
        &input,
        providers.iter().map(|p| p.id.clone()).collect(),
        false,
        true,
    )? {
        AuthMenuSelection::Provider(index) => {
            let item = &providers[index];
            print!(
                "Are you sure you want to remove authentication for {}? (y/N): ",
                item.display_name
            );
            io::stdout()
                .flush()
                .map_err(|err| UiError::new(err.to_string()))?;

            let mut confirm = String::new();
            io::stdin()
                .read_line(&mut confirm)
                .map_err(|err| UiError::new(err.to_string()))?;

            match parse_confirmation(&confirm)? {
                ConfirmationChoice::Yes => Ok(Some(DeauthSelection {
                    provider_id: item.id.clone(),
                    is_custom: item.is_custom,
                    display_name: item.display_name.clone(),
                })),
                ConfirmationChoice::No => {
                    println!("Cancelled.");
                    Ok(None)
                }
                ConfirmationChoice::Cancel => {
                    println!("Cancelled.");
                    Ok(None)
                }
            }
        }
        AuthMenuSelection::Custom | AuthMenuSelection::Cancel => {
            println!("Cancelled.");
            Ok(None)
        }
    }
}

pub fn prompt_masked_input() -> Result<String, UiError> {
    let options = LineEditorOptions {
        initial_text: String::new(),
        allow_cancel: true,
        mask_mode: MaskMode::RevealTail { tail_chars: 4 },
    };
    prompt_line_editor(MASKED_INPUT_PROMPT, &options).map_err(|err| UiError::new(err.to_string()))
}

pub fn parse_confirmation(input: &str) -> Result<ConfirmationChoice, UiError> {
    let trimmed = input.trim().to_lowercase();
    if trimmed.is_empty() {
        return Ok(ConfirmationChoice::No);
    }
    match trimmed.as_str() {
        "y" | "yes" => Ok(ConfirmationChoice::Yes),
        "n" | "no" => Ok(ConfirmationChoice::No),
        "c" | "cancel" => Ok(ConfirmationChoice::Cancel),
        _ => Err(UiError::new("Invalid confirmation response")),
    }
}

pub fn parse_provider_selection(
    input: &str,
    provider_ids: Vec<String>,
    include_custom: bool,
    include_cancel: bool,
) -> Result<AuthMenuSelection, UiError> {
    if provider_ids.is_empty() && !include_custom {
        return Err(UiError::new(INVALID_CHOICE_MSG));
    }

    let mut unique = HashSet::new();
    for id in &provider_ids {
        if !unique.insert(id) {
            return Err(UiError::new("Duplicate provider entries are not allowed"));
        }
    }

    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(UiError::new("Selection cannot be empty"));
    }

    let choice: usize = trimmed
        .parse()
        .map_err(|_| UiError::new(INVALID_CHOICE_MSG))?;

    let base_count = provider_ids.len();
    let custom_position = if include_custom {
        Some(base_count + 1)
    } else {
        None
    };
    let cancel_position = if include_cancel {
        Some(base_count + if include_custom { 2 } else { 1 })
    } else {
        None
    };

    let max_choice =
        base_count + if include_custom { 1 } else { 0 } + if include_cancel { 1 } else { 0 };

    if choice == 0 || choice > max_choice {
        return Err(UiError::new(INVALID_CHOICE_MSG));
    }

    if Some(choice) == custom_position {
        return Ok(AuthMenuSelection::Custom);
    }

    if Some(choice) == cancel_position {
        return Ok(AuthMenuSelection::Cancel);
    }

    Ok(AuthMenuSelection::Provider(choice - 1))
}

pub fn resolve_provider_id(
    input: &str,
    suggested_id: &str,
    existing_ids: &HashSet<String>,
) -> Result<String, UiError> {
    let final_id = if input.is_empty() {
        suggested_id.to_string()
    } else {
        if !input.chars().all(|c| c.is_alphanumeric()) {
            return Err(UiError::new(
                "Provider ID can only contain alphanumeric characters",
            ));
        }
        input.to_lowercase()
    };

    if existing_ids.contains(&final_id) {
        return Err(UiError::new(format!(
            "Provider with ID '{final_id}' already exists"
        )));
    }

    Ok(final_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirmation_parsing_handles_empty_and_cancel() {
        assert_eq!(parse_confirmation(" ").unwrap(), ConfirmationChoice::No);
        assert_eq!(
            parse_confirmation("cancel").unwrap(),
            ConfirmationChoice::Cancel
        );
        assert!(parse_confirmation("maybe").is_err());
    }

    #[test]
    fn provider_selection_rejects_duplicates() {
        let result = parse_provider_selection(
            "1",
            vec!["openai".to_string(), "openai".to_string()],
            true,
            false,
        );
        assert!(result.is_err());
    }

    #[test]
    fn provider_selection_handles_cancel_option() {
        let result = parse_provider_selection(
            "3",
            vec!["openai".to_string(), "anthropic".to_string()],
            false,
            true,
        )
        .unwrap();
        assert_eq!(result, AuthMenuSelection::Cancel);
    }

    #[test]
    fn provider_selection_handles_custom_option() {
        let result = parse_provider_selection(
            "3",
            vec!["openai".to_string(), "anthropic".to_string()],
            true,
            false,
        )
        .unwrap();
        assert_eq!(result, AuthMenuSelection::Custom);
    }

    #[test]
    fn resolve_provider_id_detects_duplicates() {
        let mut existing = HashSet::new();
        existing.insert("openai".to_string());
        let err = resolve_provider_id("openai", "openai", &existing)
            .expect_err("duplicate id should error");
        assert_eq!(
            err.message,
            "Provider with ID 'openai' already exists".to_string()
        );
    }
}
