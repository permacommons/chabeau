use super::{required_arg, usage_status};
use crate::commands::registry::CommandInvocation;
use crate::commands::CommandResult;
use crate::core::app::App;

const USAGE_MARKDOWN: &str = "Usage: /markdown [on|off|toggle]";
const USAGE_SYNTAX: &str = "Usage: /syntax [on|off|toggle]";

pub(crate) fn handle_theme(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    if invocation.args_len() == 0 {
        return CommandResult::OpenThemePicker;
    }

    let Some(id) = required_arg(app, &invocation, 0, "Usage: /theme <id>") else {
        return CommandResult::Continue;
    };

    let res = {
        let mut controller = app.theme_controller();
        controller.apply_theme_by_id(id)
    };
    match res {
        Ok(_) => {
            app.conversation().set_status(format!("Theme set: {}", id));
            CommandResult::Continue
        }
        Err(_) => usage_status(app, "Theme error"),
    }
}

pub(crate) fn handle_model(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    if invocation.args_len() == 0 {
        return CommandResult::OpenModelPicker;
    }

    let Some(model_id) = required_arg(app, &invocation, 0, "Usage: /model <id>") else {
        return CommandResult::Continue;
    };

    {
        let mut controller = app.provider_controller();
        controller.apply_model_by_id(model_id);
    }
    app.conversation()
        .set_status(format!("Model set: {}", model_id));
    CommandResult::Continue
}

pub(crate) fn handle_provider(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    if invocation.args_len() == 0 {
        return CommandResult::OpenProviderPicker;
    }

    let Some(provider_id) = required_arg(app, &invocation, 0, "Usage: /provider <id>") else {
        return CommandResult::Continue;
    };

    let (result, should_open_model_picker) = {
        let mut controller = app.provider_controller();
        controller.apply_provider_by_id(provider_id)
    };

    match result {
        Ok(_) => {
            app.conversation()
                .set_status(format!("Provider set: {}", provider_id));
            if should_open_model_picker {
                CommandResult::OpenModelPicker
            } else {
                CommandResult::Continue
            }
        }
        Err(e) => {
            app.conversation()
                .set_status(format!("Provider error: {}", e));
            CommandResult::Continue
        }
    }
}

pub(crate) fn handle_markdown(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    handle_toggle_command(
        app,
        invocation,
        app.ui.markdown_enabled,
        ToggleText {
            usage: USAGE_MARKDOWN,
            feature: "Markdown",
            on_word: "enabled",
            off_word: "disabled",
        },
        |app, new_state| app.ui.markdown_enabled = new_state,
        |cfg, new_state| cfg.markdown = Some(new_state),
    )
}

pub(crate) fn handle_syntax(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    handle_toggle_command(
        app,
        invocation,
        app.ui.syntax_enabled,
        ToggleText {
            usage: USAGE_SYNTAX,
            feature: "Syntax",
            on_word: "on",
            off_word: "off",
        },
        |app, new_state| app.ui.syntax_enabled = new_state,
        |cfg, new_state| cfg.syntax = Some(new_state),
    )
}

pub(crate) fn handle_character(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    if invocation.args_text().is_empty() {
        CommandResult::OpenCharacterPicker
    } else {
        let character_name = invocation.args_text();
        match app.character_service.resolve(character_name) {
            Ok(card) => {
                let name = card.data.name.clone();
                app.session.set_character(card);
                app.conversation()
                    .set_status(format!("Character set: {}", name));
                CommandResult::Continue
            }
            Err(e) => {
                app.conversation()
                    .set_status(format!("Character error: {}", e));
                CommandResult::Continue
            }
        }
    }
}

pub(crate) fn handle_persona(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    if invocation.args_len() == 0 {
        return CommandResult::OpenPersonaPicker;
    }

    let Some(persona_id) = required_arg(app, &invocation, 0, "Usage: /persona <id>") else {
        return CommandResult::Continue;
    };

    match app.persona_manager.set_active_persona(persona_id) {
        Ok(()) => {
            let active_persona_name = app
                .persona_manager
                .get_active_persona()
                .map(|p| p.display_name.clone());
            let persona_name = active_persona_name
                .clone()
                .unwrap_or_else(|| "Unknown".to_string());

            if active_persona_name.is_some() {
                let display_name = app.persona_manager.get_display_name();
                app.ui.update_user_display_name(display_name);
            } else {
                app.ui.update_user_display_name("You".to_string());
            }
            app.conversation()
                .set_status(format!("Persona activated: {}", persona_name));
            CommandResult::Continue
        }
        Err(e) => {
            app.conversation()
                .set_status(format!("Persona error: {}", e));
            CommandResult::Continue
        }
    }
}

pub(crate) fn handle_preset(app: &mut App, invocation: CommandInvocation<'_>) -> CommandResult {
    if invocation.args_len() == 0 {
        return CommandResult::OpenPresetPicker;
    }

    let Some(preset_id) = required_arg(app, &invocation, 0, "Usage: /preset <id>") else {
        return CommandResult::Continue;
    };

    if preset_id.eq_ignore_ascii_case("off") || preset_id == "[turn_off_preset]" {
        app.preset_manager.clear_active_preset();
        app.conversation()
            .set_status("Preset deactivated".to_string());
        CommandResult::Continue
    } else {
        match app.preset_manager.set_active_preset(preset_id) {
            Ok(()) => {
                app.conversation()
                    .set_status(format!("Preset activated: {}", preset_id));
                CommandResult::Continue
            }
            Err(e) => {
                app.conversation()
                    .set_status(format!("Preset error: {}", e));
                CommandResult::Continue
            }
        }
    }
}

struct ToggleText {
    usage: &'static str,
    feature: &'static str,
    on_word: &'static str,
    off_word: &'static str,
}

fn handle_toggle_command<F, G>(
    app: &mut App,
    invocation: CommandInvocation<'_>,
    current_state: bool,
    text: ToggleText,
    mut apply_ui: F,
    mut persist_config: G,
) -> CommandResult
where
    F: FnMut(&mut App, bool),
    G: FnMut(&mut crate::core::config::data::Config, bool),
{
    let action = match invocation.toggle_action() {
        Ok(action) => action,
        Err(_) => return usage_status(app, text.usage),
    };

    let new_state = action.apply(current_state);
    apply_ui(app, new_state);
    let state_word = if new_state {
        text.on_word
    } else {
        text.off_word
    };

    match crate::core::config::data::Config::load() {
        Ok(mut cfg) => {
            persist_config(&mut cfg, new_state);
            let status = if cfg.save().is_ok() {
                format!("{} {}", text.feature, state_word)
            } else {
                format!("{} {} (unsaved)", text.feature, state_word)
            };
            app.conversation().set_status(status);
        }
        Err(_) => {
            app.conversation()
                .set_status(format!("{} {}", text.feature, state_word));
        }
    }

    CommandResult::Continue
}
