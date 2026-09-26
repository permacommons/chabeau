//! Model and provider pickers, including preview and revert of the
//! provider-to-model transition.

use super::inspect::{provider_metadata_builtin, provider_metadata_custom};
use super::{
    ActivePicker, ModelPickerState, PickerController, PickerData, PickerMode, ProviderPickerState,
};
use crate::api::models::sort_models;
use crate::api::ModelsResponse;
use crate::auth::AuthManager;
use crate::core::app::session::SessionContext;
use crate::core::builtin_providers::load_builtin_providers;
use crate::core::config::data::{Config, CustomProvider};
use crate::ui::picker::{PickerItem, PickerState};

impl PickerController {
    pub fn populate_model_picker_from_response(
        &mut self,
        session_context: &SessionContext,
        default_model_for_provider: Option<String>,
        models_response: ModelsResponse,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if models_response.data.is_empty() {
            return Err("No models available from this provider".into());
        }

        let mut models = models_response.data;
        sort_models(&mut models);

        let has_dates = models.iter().any(|m| {
            m.created.map(|v| v > 0).unwrap_or(false)
                || m.created_at
                    .as_ref()
                    .map(|s| s.len() > 4 && (s.contains('-') || s.contains('/')))
                    .unwrap_or(false)
        });

        let items: Vec<PickerItem> = models
            .into_iter()
            .map(|model| {
                let mut label = if let Some(display_name) = &model.display_name {
                    if display_name != &model.id && !display_name.is_empty() {
                        format!("{} ({})", model.id, display_name)
                    } else {
                        model.id.clone()
                    }
                } else {
                    model.id.clone()
                };

                if let Some(ref def) = default_model_for_provider {
                    if def.eq_ignore_ascii_case(&model.id) {
                        label.push('*');
                    }
                }

                let metadata = if let Some(created) = model.created {
                    if created > 0 && created < u64::MAX / 1000 {
                        let timestamp_secs = if created > 10_000_000_000 {
                            created / 1000
                        } else {
                            created
                        };

                        if timestamp_secs > 0 && timestamp_secs < 32_503_680_000 {
                            chrono::DateTime::<chrono::Utc>::from_timestamp(
                                timestamp_secs as i64,
                                0,
                            )
                            .map(|datetime| {
                                format!("Created: {}", datetime.format("%Y-%m-%d %H:%M UTC"))
                            })
                        } else {
                            Some(format!("Created: {} (invalid timestamp)", created))
                        }
                    } else {
                        None
                    }
                } else if let Some(created_at) = &model.created_at {
                    if !created_at.is_empty() {
                        if created_at.len() > 4
                            && (created_at.contains('-') || created_at.contains('/'))
                        {
                            Some(format!("Created: {}", created_at))
                        } else {
                            Some(format!("Created: {} (unrecognized format)", created_at))
                        }
                    } else {
                        None
                    }
                } else {
                    model
                        .owned_by
                        .as_ref()
                        .filter(|owner| !owner.is_empty() && *owner != "system")
                        .map(|owner| format!("Owner: {}", owner))
                };

                let sort_key = if has_dates {
                    model
                        .created
                        .filter(|&created| created > 0)
                        .map(|created| format!("{:020}", created))
                } else {
                    None
                };

                let inspect_metadata = metadata.clone();
                PickerItem {
                    id: model.id,
                    label,
                    metadata,
                    inspect_metadata,
                    sort_key,
                }
            })
            .collect();

        let mut selected = 0usize;
        if let Some((idx, _)) = items
            .iter()
            .enumerate()
            .find(|(_, it)| it.id == session_context.model)
        {
            selected = idx;
        }

        let picker_state = PickerState::new("Pick Model", items.clone(), selected);
        let session = ActivePicker {
            state: picker_state,
            data: PickerData::Model(Box::new(ModelPickerState {
                search_filter: String::new(),
                all_items: items,
                before_model: Some(session_context.model.clone()),
                has_dates,
            })),
        };

        self.start_active_picker(session, Some(session_context.model.clone()));

        Ok(())
    }

    pub fn filter_models(&mut self) {
        self.filter_session_items(PickerMode::Model, &[]);
    }

    pub fn filter_providers(&mut self) {
        self.filter_session_items(PickerMode::Provider, &[]);
    }

    pub fn revert_model_preview(&mut self, session: &mut SessionContext) {
        let previous_model = self
            .active()
            .and_then(ActivePicker::model_state)
            .and_then(|state| state.before_model.clone());

        if let Some(session) = self.active_mut() {
            if let Some(state) = session.model_state_mut() {
                state.before_model = None;
                state.search_filter.clear();
                state.all_items.clear();
                state.has_dates = false;
            }
        }

        if let Some(prev) = previous_model {
            session.model = prev;
        }

        if self.in_provider_model_transition {
            self.revert_provider_model_transition(session);
        }
    }

    pub fn revert_provider_preview(&mut self, session: &mut SessionContext) {
        let previous_provider = self
            .active()
            .and_then(ActivePicker::provider_state)
            .and_then(|state| state.before_provider.clone());

        if let Some(session) = self.active_mut() {
            if let Some(state) = session.provider_state_mut() {
                state.before_provider = None;
                state.search_filter.clear();
                state.all_items.clear();
            }
        }

        if let Some((prev_name, prev_display)) = previous_provider {
            session.provider_name = prev_name;
            session.provider_display_name = prev_display;
        }
    }

    pub fn revert_provider_model_transition(&mut self, session: &mut SessionContext) {
        if let Some((
            prev_provider_name,
            prev_provider_display,
            prev_model,
            prev_api_key,
            prev_base_url,
        )) = self.provider_model_transition_state.take()
        {
            session.provider_name = prev_provider_name;
            session.provider_display_name = prev_provider_display;
            session.model = prev_model;
            session.api_key = prev_api_key;
            session.base_url = prev_base_url;
        }

        self.in_provider_model_transition = false;
        self.provider_model_transition_state = None;
    }

    pub fn complete_provider_model_transition(&mut self) {
        self.in_provider_model_transition = false;
        self.provider_model_transition_state = None;
    }

    pub fn open_provider_picker(&mut self, session_context: &SessionContext) -> Result<(), String> {
        let auth_manager = AuthManager::new().map_err(|err| err.to_string())?;
        let cfg = Config::load_test_safe().map_err(|err| err.to_string())?;
        let default_provider = cfg.default_provider.clone();
        let mut items: Vec<PickerItem> = Vec::new();

        let builtin_providers = load_builtin_providers();
        for builtin_provider in builtin_providers {
            if let Ok(Some(_)) = auth_manager.get_token(&builtin_provider.id) {
                let is_default = default_provider
                    .as_ref()
                    .map(|dp| dp.eq_ignore_ascii_case(&builtin_provider.id))
                    .unwrap_or(false);
                let label = if is_default {
                    format!("{}*", builtin_provider.display_name)
                } else {
                    builtin_provider.display_name.clone()
                };
                let (metadata, inspect_metadata) =
                    provider_metadata_builtin(&builtin_provider, is_default);
                items.push(PickerItem {
                    id: builtin_provider.id.clone(),
                    label,
                    metadata: Some(metadata),
                    inspect_metadata: Some(inspect_metadata),
                    sort_key: Some(builtin_provider.display_name.clone()),
                });
            }
        }

        let custom_providers = auth_manager.list_custom_providers();
        for (id, display_name, base_url, has_token) in custom_providers {
            if has_token {
                let is_default = default_provider
                    .as_ref()
                    .map(|dp| dp.eq_ignore_ascii_case(&id))
                    .unwrap_or(false);
                let label = if is_default {
                    format!("{} (custom)*", display_name)
                } else {
                    format!("{} (custom)", display_name)
                };
                let provider_details =
                    auth_manager
                        .get_custom_provider(&id)
                        .cloned()
                        .unwrap_or(CustomProvider {
                            id: id.clone(),
                            display_name: display_name.clone(),
                            base_url: base_url.clone(),
                            mode: None,
                        });
                let (metadata, inspect_metadata) =
                    provider_metadata_custom(&provider_details, is_default);
                items.push(PickerItem {
                    id,
                    label,
                    metadata: Some(metadata),
                    inspect_metadata: Some(inspect_metadata),
                    sort_key: Some(display_name),
                });
            }
        }

        if items.is_empty() {
            return Err(
                "No configured providers found. Run 'chabeau provider add' and 'chabeau provider token add <provider-id>' to set up authentication."
                    .to_string(),
            );
        }

        let mut selected = 0usize;
        if let Some((idx, _)) = items
            .iter()
            .enumerate()
            .find(|(_, it)| it.id == session_context.provider_name)
        {
            selected = idx;
        }

        let picker_state = PickerState::new("Pick Provider", items.clone(), selected);
        let session = ActivePicker {
            state: picker_state,
            data: PickerData::Provider(Box::new(ProviderPickerState {
                search_filter: String::new(),
                all_items: items,
                before_provider: Some((
                    session_context.provider_name.clone(),
                    session_context.provider_display_name.clone(),
                )),
            })),
        };

        self.start_active_picker(session, Some(session_context.provider_name.clone()));

        Ok(())
    }
}
