//! Interactive line prompts shared by the provider and MCP subcommands.

use std::error::Error;
use std::io::{self, IsTerminal, Write};

use crate::utils::line_editor::{prompt_line_editor, LineEditorOptions, MaskMode};

pub(super) fn prompt_required(prompt: &str) -> Result<String, Box<dyn Error>> {
    loop {
        let value = prompt_optional(prompt)?;
        if value.is_empty() {
            println!("Value cannot be empty.");
            continue;
        }
        return Ok(value);
    }
}

pub(super) fn prompt_optional(prompt: &str) -> Result<String, Box<dyn Error>> {
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        let options = LineEditorOptions {
            initial_text: String::new(),
            allow_cancel: true,
            mask_mode: MaskMode::None,
        };
        return prompt_line_editor(prompt, &options)
            .map(|value| value.trim().to_string())
            .map_err(|err| Box::new(err) as Box<dyn Error>);
    }

    print!("{prompt}");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

pub(super) fn prompt_bool_with_default(label: &str, default: bool) -> Result<bool, Box<dyn Error>> {
    let default_hint = if default { "Y/n" } else { "y/N" };
    loop {
        let input = prompt_optional(&format!("{label} [{default_hint}]: "))?;
        if input.is_empty() {
            return Ok(default);
        }
        match input.to_ascii_lowercase().as_str() {
            "y" | "yes" | "on" | "true" => return Ok(true),
            "n" | "no" | "off" | "false" => return Ok(false),
            _ => println!("Please enter yes or no."),
        }
    }
}
