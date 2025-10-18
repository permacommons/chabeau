use super::{input, App, AppAction, AppActionContext, AppCommand};
use crate::core::app::picker::PickerMode;
use crate::core::config::data::Config;
use crate::core::message::AppMessageKind;

pub(super) fn handle_picker_action(
    app: &mut App,
    action: AppAction,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    match action {
        AppAction::PickerEscape => {
            handle_picker_escape(app, ctx);
            None
        }
        AppAction::PickerMoveUp => {
            handle_picker_movement(app, PickerMovement::Up);
            None
        }
        AppAction::PickerMoveDown => {
            handle_picker_movement(app, PickerMovement::Down);
            None
        }
        AppAction::PickerMoveToStart => {
            handle_picker_movement(app, PickerMovement::Start);
            None
        }
        AppAction::PickerMoveToEnd => {
            handle_picker_movement(app, PickerMovement::End);
            None
        }
        AppAction::PickerCycleSortMode => {
            handle_picker_cycle_sort_mode(app);
            None
        }
        AppAction::PickerApplySelection { persistent } => {
            handle_picker_apply_selection(app, persistent, ctx)
        }
        AppAction::PickerUnsetDefault => handle_picker_unset_default(app, ctx),
        AppAction::PickerBackspace => {
            handle_picker_backspace(app);
            None
        }
        AppAction::PickerTypeChar { ch } => {
            handle_picker_type_char(app, ch);
            None
        }
        AppAction::PickerInspectSelection => {
            handle_picker_inspect(app, ctx);
            None
        }
        AppAction::PickerInspectScroll { lines } => {
            app.scroll_picker_inspect(lines);
            None
        }
        AppAction::PickerInspectScrollToStart => {
            app.scroll_picker_inspect_to_start();
            None
        }
        AppAction::PickerInspectScrollToEnd => {
            app.scroll_picker_inspect_to_end();
            None
        }
        AppAction::ModelPickerLoaded {
            default_model_for_provider,
            models_response,
        } => {
            if let Err(e) =
                app.complete_model_picker_request(default_model_for_provider, models_response)
            {
                input::set_status_message(app, format!("Model picker error: {}", e), ctx);
            }
            None
        }
        AppAction::ModelPickerLoadFailed { error } => {
            app.fail_model_picker_request();
            input::set_status_message(app, format!("Model picker error: {}", error), ctx);
            None
        }
        _ => unreachable!("non-picker action routed to picker handler"),
    }
}

enum PickerMovement {
    Up,
    Down,
    Start,
    End,
}

fn handle_picker_movement(app: &mut App, movement: PickerMovement) {
    if app.picker_inspect_state().is_some() {
        return;
    }
    let mode = app.current_picker_mode();
    let selected_theme = {
        let mut selected = None;
        if let Some(state) = app.picker_state_mut() {
            match movement {
                PickerMovement::Up => state.move_up(),
                PickerMovement::Down => state.move_down(),
                PickerMovement::Start => state.move_to_start(),
                PickerMovement::End => state.move_to_end(),
            }
            if mode == Some(PickerMode::Theme) {
                selected = state.selected_id().map(|s| s.to_string());
            }
        }
        selected
    };
    if let Some(id) = selected_theme {
        app.preview_theme_by_id(&id);
    }
}

fn handle_picker_cycle_sort_mode(app: &mut App) {
    if app.picker_inspect_state().is_some() {
        return;
    }
    if let Some(state) = app.picker_state_mut() {
        state.cycle_sort_mode();
    }
    app.sort_picker_items();
    app.update_picker_title();
}

fn handle_picker_backspace(app: &mut App) {
    if app.picker_inspect_state().is_some() {
        return;
    }
    match app.current_picker_mode() {
        Some(PickerMode::Model) => {
            if let Some(state) = app.model_picker_state_mut() {
                if !state.search_filter.is_empty() {
                    state.search_filter.pop();
                    app.filter_models();
                }
            }
        }
        Some(PickerMode::Theme) => {
            if let Some(state) = app.theme_picker_state_mut() {
                if !state.search_filter.is_empty() {
                    state.search_filter.pop();
                    app.filter_themes();
                }
            }
        }
        Some(PickerMode::Provider) => {
            if let Some(state) = app.provider_picker_state_mut() {
                if !state.search_filter.is_empty() {
                    state.search_filter.pop();
                    app.filter_providers();
                }
            }
        }
        Some(PickerMode::Character) => {
            if let Some(state) = app.character_picker_state_mut() {
                if !state.search_filter.is_empty() {
                    state.search_filter.pop();
                    app.filter_characters();
                }
            }
        }
        Some(PickerMode::Persona) => {
            if let Some(state) = app.persona_picker_state_mut() {
                if !state.search_filter.is_empty() {
                    state.search_filter.pop();
                    app.filter_personas();
                }
            }
        }
        Some(PickerMode::Preset) => {
            if let Some(state) = app.preset_picker_state_mut() {
                if !state.search_filter.is_empty() {
                    state.search_filter.pop();
                    app.filter_presets();
                }
            }
        }
        None => {}
    }
}

fn handle_picker_type_char(app: &mut App, ch: char) {
    if app.picker_inspect_state().is_some() {
        return;
    }
    if ch.is_control() {
        return;
    }

    match app.current_picker_mode() {
        Some(PickerMode::Model) => {
            if let Some(state) = app.model_picker_state_mut() {
                state.search_filter.push(ch);
                app.filter_models();
            }
        }
        Some(PickerMode::Theme) => {
            if let Some(state) = app.theme_picker_state_mut() {
                state.search_filter.push(ch);
                app.filter_themes();
            }
        }
        Some(PickerMode::Provider) => {
            if let Some(state) = app.provider_picker_state_mut() {
                state.search_filter.push(ch);
                app.filter_providers();
            }
        }
        Some(PickerMode::Character) => {
            if let Some(state) = app.character_picker_state_mut() {
                state.search_filter.push(ch);
                app.filter_characters();
            }
        }
        Some(PickerMode::Persona) => {
            if let Some(state) = app.persona_picker_state_mut() {
                state.search_filter.push(ch);
                app.filter_personas();
            }
        }
        Some(PickerMode::Preset) => {
            if let Some(state) = app.preset_picker_state_mut() {
                state.search_filter.push(ch);
                app.filter_presets();
            }
        }
        None => {}
    }
}

fn handle_picker_inspect(app: &mut App, ctx: AppActionContext) {
    let (title, metadata) = {
        let Some(session) = app.picker_session() else {
            return;
        };
        let state = &session.state;

        let Some(item) = state.get_selected_item() else {
            return;
        };

        let mut display_label = item.label.clone();
        if display_label.ends_with('*') {
            display_label.pop();
            display_label = display_label.trim_end().to_string();
        }

        let title = format!("{} – {}", session.base_title(), display_label);
        (
            title,
            state
                .get_selected_inspect_metadata()
                .map(|text| text.to_string()),
        )
    };

    match metadata {
        Some(text) if text.trim().is_empty() => {
            input::set_status_message(app, "Nothing to inspect for this item".to_string(), ctx);
        }
        Some(text) => {
            app.open_picker_inspect(title, text);
            input::set_status_message(
                app,
                "Inspecting selection (Esc=Back to picker)".to_string(),
                ctx,
            );
        }
        None => {
            input::set_status_message(app, "Nothing to inspect for this item".to_string(), ctx);
        }
    }
}

fn handle_picker_escape(app: &mut App, ctx: AppActionContext) {
    if app.picker_inspect_state().is_some() {
        app.close_picker_inspect();
        input::set_status_message(
            app,
            "Returned to picker (Ctrl+O=Inspect again)".to_string(),
            ctx,
        );
        return;
    }
    match app.current_picker_mode() {
        Some(PickerMode::Theme) => {
            app.revert_theme_preview();
            app.close_picker();
        }
        Some(PickerMode::Model) => {
            if app.picker.startup_requires_model {
                app.close_picker();
                if app.picker.startup_multiple_providers_available {
                    app.picker.startup_requires_model = false;
                    app.picker.startup_requires_provider = true;
                    app.session.provider_name.clear();
                    app.session.provider_display_name = "(no provider selected)".to_string();
                    app.session.api_key.clear();
                    app.session.base_url.clear();
                    app.open_provider_picker();
                } else {
                    app.request_exit();
                }
            } else {
                let was_transitioning = app.picker.in_provider_model_transition;
                app.revert_model_preview();
                if was_transitioning {
                    input::set_status_message(app, "Selection cancelled".to_string(), ctx);
                }
                app.close_picker();
            }
        }
        Some(PickerMode::Provider) => {
            if app.picker.startup_requires_provider {
                app.close_picker();
                app.request_exit();
            } else {
                app.revert_provider_preview();
                app.close_picker();
            }
        }
        Some(PickerMode::Character) => {
            app.close_picker();
        }
        Some(PickerMode::Persona) => {
            app.close_picker();
        }
        Some(PickerMode::Preset) => {
            app.close_picker();
        }
        None => {}
    }
}

fn handle_picker_apply_selection(
    app: &mut App,
    persistent: bool,
    ctx: AppActionContext,
) -> Option<AppCommand> {
    match app.current_picker_mode() {
        Some(PickerMode::Theme) => {
            let Some(id) = selected_picker_id(app) else {
                app.close_picker();
                return None;
            };
            let mut controller = app.theme_controller();
            if persistent {
                match controller.apply_theme_by_id(&id) {
                    Ok(_) => input::set_status_message(
                        app,
                        format!("Theme set: {} (saved to config)", id),
                        ctx,
                    ),
                    Err(e) => input::set_status_message(app, format!("Theme error: {}", e), ctx),
                }
            } else {
                match controller.apply_theme_by_id_session_only(&id) {
                    Ok(_) => input::set_status_message(
                        app,
                        format!("Theme set: {} (session only)", id),
                        ctx,
                    ),
                    Err(e) => input::set_status_message(app, format!("Theme error: {}", e), ctx),
                }
            }
            app.close_picker();
            None
        }
        Some(PickerMode::Model) => {
            let Some(id) = selected_picker_id(app) else {
                app.close_picker();
                return None;
            };
            let persist_to_config = persistent && !app.session.startup_env_only;
            let result = {
                let mut controller = app.provider_controller();
                if persist_to_config {
                    controller.apply_model_by_id_persistent(&id)
                } else {
                    controller.apply_model_by_id(&id);
                    Ok(())
                }
            };
            match result {
                Ok(_) => {
                    input::set_status_message(
                        app,
                        format!(
                            "Model set: {}{}",
                            id,
                            picker_status_suffix(persist_to_config)
                        ),
                        ctx,
                    );
                    if app.picker.in_provider_model_transition {
                        app.complete_provider_model_transition();
                    }
                    if app.picker.startup_requires_model {
                        app.picker.startup_requires_model = false;
                    }
                    load_default_character_if_configured(app, ctx);
                    load_default_persona_if_configured(app, ctx);
                    load_default_preset_if_configured(app, ctx);
                }
                Err(e) => input::set_status_message(app, format!("Model error: {}", e), ctx),
            }
            app.close_picker();
            None
        }
        Some(PickerMode::Provider) => {
            let Some(id) = selected_picker_id(app) else {
                app.close_picker();
                return None;
            };
            let (result, should_open_model_picker) = {
                let mut controller = app.provider_controller();
                if persistent {
                    controller.apply_provider_by_id_persistent(&id)
                } else {
                    controller.apply_provider_by_id(&id)
                }
            };

            let mut followup = None;
            match result {
                Ok(_) => {
                    input::set_status_message(
                        app,
                        format!("Provider set: {}{}", id, picker_status_suffix(persistent)),
                        ctx,
                    );
                    app.close_picker();
                    if should_open_model_picker {
                        if app.picker.startup_requires_provider {
                            app.picker.startup_requires_provider = false;
                            app.picker.startup_requires_model = true;
                        }
                        match app.prepare_model_picker_request() {
                            Ok(request) => {
                                followup = Some(AppCommand::LoadModelPicker(request));
                            }
                            Err(err) => {
                                input::set_status_message(
                                    app,
                                    format!("Model picker error: {}", err),
                                    ctx,
                                );
                            }
                        }
                    } else {
                        load_default_character_if_configured(app, ctx);
                        load_default_persona_if_configured(app, ctx);
                        load_default_preset_if_configured(app, ctx);
                    }
                }
                Err(e) => {
                    input::set_status_message(app, format!("Provider error: {}", e), ctx);
                    app.close_picker();
                }
            }

            followup
        }
        Some(PickerMode::Character) => {
            app.apply_selected_character(persistent);
            None
        }
        Some(PickerMode::Persona) => {
            app.apply_selected_persona(persistent);
            None
        }
        Some(PickerMode::Preset) => {
            app.apply_selected_preset(persistent);
            None
        }
        None => None,
    }
}

fn handle_picker_unset_default(app: &mut App, ctx: AppActionContext) -> Option<AppCommand> {
    let mode = app.current_picker_mode();
    let (selected_id, selected_label) = app.picker_state().and_then(|state| {
        state
            .get_selected_item()
            .map(|item| (item.id.clone(), item.label.clone()))
    })?;

    if !selected_label.ends_with('*') {
        input::set_status_message(
            app,
            "Del key only works on default items (marked with *)".to_string(),
            ctx,
        );
        return None;
    }

    match mode {
        Some(PickerMode::Model) => {
            let provider_name = app.session.provider_name.clone();
            let mut controller = app.provider_controller();
            match controller.unset_default_model(&provider_name) {
                Ok(_) => {
                    input::set_status_message(
                        app,
                        format!("Removed default: {}", selected_id),
                        ctx,
                    );
                    match app.prepare_model_picker_request() {
                        Ok(request) => Some(AppCommand::LoadModelPicker(request)),
                        Err(err) => {
                            input::set_status_message(
                                app,
                                format!("Model picker error: {}", err),
                                ctx,
                            );
                            None
                        }
                    }
                }
                Err(e) => {
                    input::set_status_message(app, format!("Error removing default: {}", e), ctx);
                    None
                }
            }
        }
        Some(PickerMode::Theme) => {
            let mut controller = app.theme_controller();
            match controller.unset_default_theme() {
                Ok(_) => {
                    input::set_status_message(
                        app,
                        format!("Removed default: {}", selected_id),
                        ctx,
                    );
                    if let Err(err) = app.open_theme_picker() {
                        input::set_status_message(app, format!("Theme picker error: {}", err), ctx);
                    }
                    None
                }
                Err(e) => {
                    input::set_status_message(app, format!("Error removing default: {}", e), ctx);
                    None
                }
            }
        }
        Some(PickerMode::Provider) => {
            let mut controller = app.provider_controller();
            match controller.unset_default_provider() {
                Ok(_) => {
                    input::set_status_message(
                        app,
                        format!("Removed default: {}", selected_id),
                        ctx,
                    );
                    app.open_provider_picker();
                    None
                }
                Err(e) => {
                    input::set_status_message(app, format!("Error removing default: {}", e), ctx);
                    None
                }
            }
        }
        Some(PickerMode::Character) => {
            let provider_name = app.session.provider_name.clone();
            let model = app.session.model.clone();
            match Config::mutate(move |config| {
                config.unset_default_character(&provider_name, &model);
                Ok(())
            }) {
                Ok(_) => {
                    input::set_status_message(
                        app,
                        format!("Removed default: {}", selected_id),
                        ctx,
                    );
                    app.open_character_picker();
                    None
                }
                Err(e) => {
                    input::set_status_message(app, format!("Error removing default: {}", e), ctx);
                    None
                }
            }
        }
        Some(PickerMode::Persona) => {
            let provider_name = app.session.provider_name.clone();
            let model = app.session.model.clone();
            match app
                .persona_manager
                .unset_default_for_provider_model_persistent(&provider_name, &model)
            {
                Ok(()) => {
                    input::set_status_message(
                        app,
                        format!("Removed default: {}", selected_id),
                        ctx,
                    );
                    app.open_persona_picker();
                    None
                }
                Err(e) => {
                    input::set_status_message(app, format!("Error removing default: {}", e), ctx);
                    None
                }
            }
        }
        Some(PickerMode::Preset) => {
            let provider_name = app.session.provider_name.clone();
            let model = app.session.model.clone();
            match app
                .preset_manager
                .unset_default_for_provider_model_persistent(&provider_name, &model)
            {
                Ok(()) => {
                    input::set_status_message(
                        app,
                        format!("Removed default: {}", selected_id),
                        ctx,
                    );
                    app.open_preset_picker();
                    None
                }
                Err(e) => {
                    input::set_status_message(app, format!("Error removing default: {}", e), ctx);
                    None
                }
            }
        }
        _ => None,
    }
}

fn picker_status_suffix(is_persistent: bool) -> &'static str {
    if is_persistent {
        " (saved to config)"
    } else {
        " (session only)"
    }
}

fn selected_picker_id(app: &App) -> Option<String> {
    app.picker_state()
        .and_then(|state| state.selected_id().map(|id| id.to_string()))
}

fn push_error_app_message(app: &mut App, message: String, ctx: AppActionContext) {
    if ctx.term_width > 0 && ctx.term_height > 0 {
        let input_area_height = app.input_area_height(ctx.term_width);
        let mut conversation = app.conversation();
        conversation.add_app_message(AppMessageKind::Error, message);
        let available_height =
            conversation.calculate_available_height(ctx.term_height, input_area_height);
        conversation.update_scroll_position(available_height, ctx.term_width);
    } else {
        app.conversation()
            .add_app_message(AppMessageKind::Error, message);
    }
}

fn load_default_character_if_configured(app: &mut App, ctx: AppActionContext) {
    let cfg = match Config::load_test_safe() {
        Ok(cfg) => cfg,
        Err(err) => {
            push_error_app_message(app, format!("Failed to load configuration: {}", err), ctx);
            return;
        }
    };
    if let Some(default_name) =
        cfg.get_default_character(&app.session.provider_name, &app.session.model)
    {
        match app.character_service.load_default_for_session(
            &app.session.provider_name,
            &app.session.model,
            &cfg,
        ) {
            Ok(Some((_name, card))) => {
                app.session.set_character(card);
                app.conversation().show_character_greeting_if_needed();
            }
            Ok(None) => {}
            Err(e) => {
                push_error_app_message(
                    app,
                    format!("Could not load default character '{}': {}", default_name, e),
                    ctx,
                );
            }
        }
    }
}

fn load_default_persona_if_configured(app: &mut App, ctx: AppActionContext) {
    if app.persona_manager.get_active_persona().is_some() {
        return;
    }

    if let Some(persona_id) = app
        .persona_manager
        .get_default_for_provider_model(&app.session.provider_name, &app.session.model)
    {
        let persona_id = persona_id.to_string();
        match app.persona_manager.set_active_persona(&persona_id) {
            Ok(()) => {
                if let Some(persona) = app.persona_manager.get_active_persona() {
                    app.update_user_display_name(persona.display_name.clone());
                }
            }
            Err(e) => {
                push_error_app_message(
                    app,
                    format!("Could not load default persona '{}': {}", persona_id, e),
                    ctx,
                );
            }
        }
    }
}

fn load_default_preset_if_configured(app: &mut App, ctx: AppActionContext) {
    if app.preset_manager.get_active_preset().is_some() {
        return;
    }

    if let Some(preset_id) = app
        .preset_manager
        .get_default_for_provider_model(&app.session.provider_name, &app.session.model)
    {
        let preset_id = preset_id.to_string();
        if let Err(e) = app.preset_manager.set_active_preset(&preset_id) {
            push_error_app_message(
                app,
                format!("Could not load default preset '{}': {}", preset_id, e),
                ctx,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::card::{CharacterCard, CharacterData};
    use crate::core::app::picker::{
        CharacterPickerState, ModelPickerState, PickerData, PickerSession,
    };
    use crate::core::config::data::{Config, Persona, Preset};
    use crate::ui::picker::{PickerItem, PickerState};
    use crate::utils::test_utils::{create_test_app, TestEnvVarGuard};
    use std::fs;
    use tempfile::tempdir;

    fn default_ctx() -> AppActionContext {
        AppActionContext {
            term_width: 80,
            term_height: 24,
        }
    }

    #[test]
    fn theme_picker_escape_reverts_preview() {
        let mut app = create_test_app();
        let ctx = default_ctx();
        let original_color = app.ui.theme.background_color;

        app.open_theme_picker().expect("theme picker opens");
        handle_picker_action(&mut app, AppAction::PickerMoveDown, ctx);

        assert_ne!(app.ui.theme.background_color, original_color);

        handle_picker_action(&mut app, AppAction::PickerEscape, ctx);

        assert_eq!(app.ui.theme.background_color, original_color);
        assert!(app.picker_session().is_none());
    }

    #[test]
    fn model_picker_escape_reverts_provider_transition() {
        let mut app = create_test_app();
        let ctx = default_ctx();

        app.session.provider_name = "new-prov".into();
        app.session.provider_display_name = "New Provider".into();
        app.session.model = "new-model".into();
        app.session.api_key = "new-key".into();
        app.session.base_url = "https://api.new".into();

        app.picker.in_provider_model_transition = true;
        app.picker.provider_model_transition_state = Some((
            "old-prov".into(),
            "Old Provider".into(),
            "old-model".into(),
            "old-key".into(),
            "https://api.old".into(),
        ));

        let items = vec![PickerItem {
            id: "new-model".into(),
            label: "New Model".into(),
            metadata: None,
            inspect_metadata: None,
            sort_key: None,
        }];

        let picker_state = PickerState::new("Pick Model", items.clone(), 0);

        app.picker.picker_session = Some(PickerSession {
            state: picker_state,
            data: PickerData::Model(ModelPickerState {
                search_filter: String::new(),
                all_items: items,
                before_model: Some("old-model".into()),
                has_dates: false,
            }),
        });

        handle_picker_action(&mut app, AppAction::PickerEscape, ctx);

        assert_eq!(app.session.provider_name, "old-prov");
        assert_eq!(app.session.provider_display_name, "Old Provider");
        assert_eq!(app.session.model, "old-model");
        assert_eq!(app.session.api_key, "old-key");
        assert_eq!(app.session.base_url, "https://api.old");
        assert!(!app.picker.in_provider_model_transition);
        assert!(app.picker.provider_model_transition_state.is_none());
        assert!(app.picker_session().is_none());
        assert_eq!(app.ui.status.as_deref(), Some("Selection cancelled"));
    }

    #[test]
    fn character_picker_closes_after_selection() {
        let mut app = create_test_app();
        let ctx = default_ctx();

        let items = vec![PickerItem {
            id: "alice".to_string(),
            label: "Alice".to_string(),
            metadata: Some("A helpful assistant".to_string()),
            inspect_metadata: Some("A helpful assistant".to_string()),
            sort_key: Some("Alice".to_string()),
        }];

        let picker_state = PickerState::new("Pick Character", items.clone(), 0);
        app.picker.picker_session = Some(PickerSession {
            state: picker_state,
            data: PickerData::Character(CharacterPickerState {
                search_filter: String::new(),
                all_items: items,
            }),
        });

        assert!(app.picker_session().is_some());
        handle_picker_action(
            &mut app,
            AppAction::PickerApplySelection { persistent: false },
            ctx,
        );
        assert!(app.picker_session().is_none());
    }

    #[test]
    fn character_picker_marks_default_with_asterisk() {
        let mut app = create_test_app();

        let items = vec![
            PickerItem {
                id: "alice".to_string(),
                label: "alice*".to_string(),
                metadata: Some("A helpful assistant".to_string()),
                inspect_metadata: Some("A helpful assistant".to_string()),
                sort_key: Some("alice".to_string()),
            },
            PickerItem {
                id: "bob".to_string(),
                label: "bob".to_string(),
                metadata: Some("Another character".to_string()),
                inspect_metadata: Some("Another character".to_string()),
                sort_key: Some("bob".to_string()),
            },
        ];

        let picker_state = PickerState::new("Pick Character", items.clone(), 0);
        app.picker.picker_session = Some(PickerSession {
            state: picker_state,
            data: PickerData::Character(CharacterPickerState {
                search_filter: String::new(),
                all_items: items,
            }),
        });

        let selected_item = app.picker_state().unwrap().get_selected_item().unwrap();
        assert_eq!(selected_item.label, "alice*");
    }

    #[test]
    fn model_selection_loads_default_character() {
        let mut app = create_test_app();

        let temp_dir = tempdir().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        fs::create_dir_all(&cards_dir).unwrap();

        let test_card = CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "alice".to_string(),
                description: "A helpful assistant".to_string(),
                personality: "Friendly".to_string(),
                scenario: "Helping users".to_string(),
                first_mes: "Hello!".to_string(),
                mes_example: "Example".to_string(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        };

        let card_path = cards_dir.join("alice.json");
        fs::write(&card_path, serde_json::to_string(&test_card).unwrap()).unwrap();

        let mut env_guard = TestEnvVarGuard::new();
        env_guard.set_var("CHABEAU_CONFIG_DIR", temp_dir.path());

        assert!(app.session.active_character.is_none());

        load_default_character_if_configured(&mut app, AppActionContext::default());
        load_default_persona_if_configured(&mut app, AppActionContext::default());

        assert!(app.session.active_character.is_none());
        assert!(app.persona_manager.get_active_persona().is_none());
    }

    #[test]
    fn test_load_default_persona_if_configured() {
        let mut config = Config {
            personas: vec![
                Persona {
                    id: "alice-dev".to_string(),
                    display_name: "Alice".to_string(),
                    bio: Some("A developer persona".to_string()),
                },
                Persona {
                    id: "bob-student".to_string(),
                    display_name: "Bob".to_string(),
                    bio: Some("A student persona".to_string()),
                },
            ],
            ..Default::default()
        };

        config.set_default_persona(
            "test".to_string(),
            "test-model".to_string(),
            "alice-dev".to_string(),
        );

        let mut app = create_test_app();
        app.persona_manager = crate::core::persona::PersonaManager::load_personas(&config)
            .expect("Failed to load personas");

        assert!(app.persona_manager.get_active_persona().is_none());

        load_default_persona_if_configured(&mut app, AppActionContext::default());

        let active_persona = app.persona_manager.get_active_persona();
        assert!(active_persona.is_some());
        assert_eq!(active_persona.unwrap().id, "alice-dev");
        assert_eq!(app.ui.user_display_name, "Alice");
    }

    #[test]
    fn test_load_default_persona_respects_existing_active_persona() {
        let mut config = Config {
            personas: vec![
                Persona {
                    id: "alice-dev".to_string(),
                    display_name: "Alice".to_string(),
                    bio: Some("A developer persona".to_string()),
                },
                Persona {
                    id: "bob-student".to_string(),
                    display_name: "Bob".to_string(),
                    bio: Some("A student persona".to_string()),
                },
            ],
            ..Default::default()
        };

        config.set_default_persona(
            "test".to_string(),
            "test-model".to_string(),
            "alice-dev".to_string(),
        );

        let mut app = create_test_app();
        app.persona_manager = crate::core::persona::PersonaManager::load_personas(&config)
            .expect("Failed to load personas");

        app.persona_manager
            .set_active_persona("bob-student")
            .unwrap();
        app.update_user_display_name("Bob".to_string());

        load_default_persona_if_configured(&mut app, AppActionContext::default());

        let active_persona = app.persona_manager.get_active_persona();
        assert!(active_persona.is_some());
        assert_eq!(active_persona.unwrap().id, "bob-student");
        assert_eq!(app.ui.user_display_name, "Bob");
    }

    #[test]
    fn test_load_default_preset_if_configured() {
        let mut config = Config {
            presets: vec![
                Preset {
                    id: "focus".to_string(),
                    pre: "Focus on details.".to_string(),
                    post: String::new(),
                },
                Preset {
                    id: "summary".to_string(),
                    pre: String::new(),
                    post: "Summarize at the end.".to_string(),
                },
            ],
            ..Default::default()
        };

        config.set_default_preset(
            "test".to_string(),
            "test-model".to_string(),
            "focus".to_string(),
        );

        let mut app = create_test_app();
        app.preset_manager = crate::core::preset::PresetManager::load_presets(&config)
            .expect("Failed to load presets");

        assert!(app.preset_manager.get_active_preset().is_none());

        load_default_preset_if_configured(&mut app, AppActionContext::default());

        let active = app
            .preset_manager
            .get_active_preset()
            .expect("preset active");
        assert_eq!(active.id, "focus");
    }

    #[test]
    fn test_load_default_preset_respects_existing_active_preset() {
        let mut config = Config {
            presets: vec![
                Preset {
                    id: "focus".to_string(),
                    pre: "Focus on details.".to_string(),
                    post: String::new(),
                },
                Preset {
                    id: "summary".to_string(),
                    pre: String::new(),
                    post: "Summarize at the end.".to_string(),
                },
            ],
            ..Default::default()
        };

        config.set_default_preset(
            "test".to_string(),
            "test-model".to_string(),
            "focus".to_string(),
        );

        let mut app = create_test_app();
        app.preset_manager = crate::core::preset::PresetManager::load_presets(&config)
            .expect("Failed to load presets");
        app.preset_manager
            .set_active_preset("summary")
            .expect("activate preset");

        load_default_preset_if_configured(&mut app, AppActionContext::default());

        let active = app
            .preset_manager
            .get_active_preset()
            .expect("preset active");
        assert_eq!(active.id, "summary");
    }
}
