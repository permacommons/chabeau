use super::{SessionContext, UiState};
use crate::api::models::sort_models;
use crate::api::ModelsResponse;
use crate::auth::AuthManager;
use crate::core::builtin_providers::load_builtin_providers;
use crate::core::config::Config;
use crate::ui::builtin_themes::load_builtin_themes;
use crate::ui::picker::{PickerItem, PickerState, SortMode};
use crate::ui::theme::Theme;

/// Special ID for the "turn off character mode" picker entry
pub(super) const TURN_OFF_CHARACTER_ID: &str = "__turn_off_character__";

/// Sanitize metadata text for display in picker
///
/// Removes newlines, carriage returns, and other control characters that could
/// break the TUI layout. Replaces sequences of whitespace with a single space.
fn sanitize_picker_metadata(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c == '\n' || c == '\r' || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerMode {
    Theme,
    Model,
    Provider,
    Character,
}

#[derive(Debug, Clone)]
pub struct ThemePickerState {
    pub search_filter: String,
    pub all_items: Vec<PickerItem>,
    pub before_theme: Option<Theme>,
    pub before_theme_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ModelPickerState {
    pub search_filter: String,
    pub all_items: Vec<PickerItem>,
    pub before_model: Option<String>,
    pub has_dates: bool,
}

#[derive(Debug, Clone)]
pub struct ProviderPickerState {
    pub search_filter: String,
    pub all_items: Vec<PickerItem>,
    pub before_provider: Option<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct CharacterPickerState {
    pub search_filter: String,
    pub all_items: Vec<PickerItem>,
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum PickerData {
    Theme(ThemePickerState),
    Model(ModelPickerState),
    Provider(ProviderPickerState),
    Character(CharacterPickerState),
}

#[derive(Debug, Clone)]
pub struct PickerSession {
    pub mode: PickerMode,
    pub state: PickerState,
    pub data: PickerData,
}

impl PickerSession {
    fn prefers_alphabetical(&self) -> bool {
        match (&self.mode, &self.data) {
            (PickerMode::Theme, _) | (PickerMode::Provider, _) | (PickerMode::Character, _) => true,
            (PickerMode::Model, PickerData::Model(state)) => !state.has_dates,
            _ => false,
        }
    }

    pub(crate) fn default_sort_mode(&self) -> SortMode {
        if self.prefers_alphabetical() {
            SortMode::Name
        } else {
            SortMode::Date
        }
    }

    fn filter_hint_threshold(&self) -> usize {
        match self.mode {
            PickerMode::Model => 20,
            _ => 10,
        }
    }

    fn base_title(&self) -> &'static str {
        match self.mode {
            PickerMode::Model => "Pick Model",
            PickerMode::Provider => "Pick Provider",
            PickerMode::Theme => "Pick Theme",
            PickerMode::Character => "Pick Character",
        }
    }

    fn search_filter(&self) -> &String {
        match &self.data {
            PickerData::Model(state) => &state.search_filter,
            PickerData::Theme(state) => &state.search_filter,
            PickerData::Provider(state) => &state.search_filter,
            PickerData::Character(state) => &state.search_filter,
        }
    }

    fn all_items(&self) -> &Vec<PickerItem> {
        match &self.data {
            PickerData::Model(state) => &state.all_items,
            PickerData::Theme(state) => &state.all_items,
            PickerData::Provider(state) => &state.all_items,
            PickerData::Character(state) => &state.all_items,
        }
    }
}

pub struct PickerController {
    pub picker_session: Option<PickerSession>,
    pub in_provider_model_transition: bool,
    pub provider_model_transition_state: Option<(String, String, String, String, String)>,
    pub startup_requires_provider: bool,
    pub startup_requires_model: bool,
    pub startup_multiple_providers_available: bool,
}

impl PickerController {
    pub(crate) fn new() -> Self {
        Self {
            picker_session: None,
            in_provider_model_transition: false,
            provider_model_transition_state: None,
            startup_requires_provider: false,
            startup_requires_model: false,
            startup_multiple_providers_available: false,
        }
    }

    pub fn session(&self) -> Option<&PickerSession> {
        self.picker_session.as_ref()
    }

    pub fn session_mut(&mut self) -> Option<&mut PickerSession> {
        self.picker_session.as_mut()
    }

    pub fn current_mode(&self) -> Option<PickerMode> {
        self.session().map(|session| session.mode)
    }

    pub fn state(&self) -> Option<&PickerState> {
        self.session().map(|session| &session.state)
    }

    pub fn state_mut(&mut self) -> Option<&mut PickerState> {
        self.session_mut().map(|session| &mut session.state)
    }

    pub fn theme_state(&self) -> Option<&ThemePickerState> {
        match self.session() {
            Some(PickerSession {
                mode: PickerMode::Theme,
                data: PickerData::Theme(state),
                ..
            }) => Some(state),
            _ => None,
        }
    }

    pub fn theme_state_mut(&mut self) -> Option<&mut ThemePickerState> {
        match self.session_mut() {
            Some(PickerSession {
                mode: PickerMode::Theme,
                data: PickerData::Theme(state),
                ..
            }) => Some(state),
            _ => None,
        }
    }

    pub fn model_state(&self) -> Option<&ModelPickerState> {
        match self.session() {
            Some(PickerSession {
                mode: PickerMode::Model,
                data: PickerData::Model(state),
                ..
            }) => Some(state),
            _ => None,
        }
    }

    pub fn model_state_mut(&mut self) -> Option<&mut ModelPickerState> {
        match self.session_mut() {
            Some(PickerSession {
                mode: PickerMode::Model,
                data: PickerData::Model(state),
                ..
            }) => Some(state),
            _ => None,
        }
    }

    pub fn provider_state(&self) -> Option<&ProviderPickerState> {
        match self.session() {
            Some(PickerSession {
                mode: PickerMode::Provider,
                data: PickerData::Provider(state),
                ..
            }) => Some(state),
            _ => None,
        }
    }

    pub fn provider_state_mut(&mut self) -> Option<&mut ProviderPickerState> {
        match self.session_mut() {
            Some(PickerSession {
                mode: PickerMode::Provider,
                data: PickerData::Provider(state),
                ..
            }) => Some(state),
            _ => None,
        }
    }

    pub fn character_state(&self) -> Option<&CharacterPickerState> {
        match self.session() {
            Some(PickerSession {
                mode: PickerMode::Character,
                data: PickerData::Character(state),
                ..
            }) => Some(state),
            _ => None,
        }
    }

    pub fn character_state_mut(&mut self) -> Option<&mut CharacterPickerState> {
        match self.session_mut() {
            Some(PickerSession {
                mode: PickerMode::Character,
                data: PickerData::Character(state),
                ..
            }) => Some(state),
            _ => None,
        }
    }

    pub fn close(&mut self) {
        self.picker_session = None;
    }

    pub fn open_theme_picker(&mut self, ui: &mut UiState) {
        let cfg = Config::load_test_safe();

        let mut items: Vec<PickerItem> = Vec::new();
        let default_theme_id = cfg.theme.clone();

        for t in load_builtin_themes() {
            let is_default = default_theme_id
                .as_ref()
                .map(|dt| dt.eq_ignore_ascii_case(&t.id))
                .unwrap_or(false);
            let label = if is_default {
                format!("{}*", t.display_name)
            } else {
                t.display_name.clone()
            };
            let metadata = if is_default {
                Some("Built-in theme (default from config)".to_string())
            } else {
                Some("Built-in theme".to_string())
            };
            items.push(PickerItem {
                id: t.id.clone(),
                label,
                metadata,
                sort_key: Some(t.display_name.clone()),
            });
        }

        for ct in cfg.list_custom_themes() {
            let is_default = default_theme_id
                .as_ref()
                .map(|dt| dt.eq_ignore_ascii_case(&ct.id))
                .unwrap_or(false);
            let base_label = format!("{} (custom)", ct.display_name);
            let label = if is_default {
                format!("{}*", base_label)
            } else {
                base_label
            };
            let metadata = if is_default {
                Some("Custom theme (config.toml) (default from config)".to_string())
            } else {
                Some("Custom theme (config.toml)".to_string())
            };
            items.push(PickerItem {
                id: ct.id.clone(),
                label,
                metadata,
                sort_key: Some(ct.display_name.clone()),
            });
        }

        let active_theme_id = ui.current_theme_id.as_ref().or(cfg.theme.as_ref()).cloned();

        let mut selected = 0usize;
        if let Some(id) = &active_theme_id {
            if let Some((idx, _)) = items
                .iter()
                .enumerate()
                .find(|(_, it)| it.id.eq_ignore_ascii_case(id))
            {
                selected = idx;
            }
        }

        let picker_state = PickerState::new("Pick Theme", items.clone(), selected);
        let mut session = PickerSession {
            mode: PickerMode::Theme,
            state: picker_state,
            data: PickerData::Theme(ThemePickerState {
                search_filter: String::new(),
                all_items: items,
                before_theme: Some(ui.theme.clone()),
                before_theme_id: cfg.theme.clone(),
            }),
        };

        session.state.sort_mode = session.default_sort_mode();
        self.picker_session = Some(session);

        self.sort_items();
        self.update_title();

        if let (Some(theme_id), Some(session)) = (active_theme_id, self.session_mut()) {
            if let Some((idx, _)) = session
                .state
                .items
                .iter()
                .enumerate()
                .find(|(_, it)| it.id.eq_ignore_ascii_case(&theme_id))
            {
                session.state.selected = idx;
            }
        }
    }

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

                PickerItem {
                    id: model.id,
                    label,
                    metadata,
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
        let mut session = PickerSession {
            mode: PickerMode::Model,
            state: picker_state,
            data: PickerData::Model(ModelPickerState {
                search_filter: String::new(),
                all_items: items,
                before_model: Some(session_context.model.clone()),
                has_dates,
            }),
        };

        session.state.sort_mode = session.default_sort_mode();
        self.picker_session = Some(session);

        self.sort_items();
        self.update_title();

        let current_model = session_context.model.clone();
        if let Some(session) = self.session_mut() {
            if let Some((idx, _)) = session
                .state
                .items
                .iter()
                .enumerate()
                .find(|(_, it)| it.id == current_model)
            {
                session.state.selected = idx;
            }
        }

        Ok(())
    }

    pub fn filter_models(&mut self) {
        let Some(session) = self.session_mut() else {
            return;
        };
        if let (PickerMode::Model, PickerData::Model(model_state)) =
            (session.mode, &mut session.data)
        {
            let search_term = model_state.search_filter.to_lowercase();
            let filtered: Vec<PickerItem> = if search_term.is_empty() {
                model_state.all_items.clone()
            } else {
                model_state
                    .all_items
                    .iter()
                    .filter(|item| {
                        item.id.to_lowercase().contains(&search_term)
                            || item.label.to_lowercase().contains(&search_term)
                    })
                    .cloned()
                    .collect()
            };
            session.state.items = filtered;
            if session.state.selected >= session.state.items.len() {
                session.state.selected = 0;
            }
            self.sort_items();
            self.update_title();
        }
    }

    pub fn filter_themes(&mut self) {
        let Some(session) = self.session_mut() else {
            return;
        };
        if let (PickerMode::Theme, PickerData::Theme(theme_state)) =
            (session.mode, &mut session.data)
        {
            let search_term = theme_state.search_filter.to_lowercase();
            let filtered: Vec<PickerItem> = if search_term.is_empty() {
                theme_state.all_items.clone()
            } else {
                theme_state
                    .all_items
                    .iter()
                    .filter(|item| {
                        item.id.to_lowercase().contains(&search_term)
                            || item.label.to_lowercase().contains(&search_term)
                    })
                    .cloned()
                    .collect()
            };
            session.state.items = filtered;
            if session.state.selected >= session.state.items.len() {
                session.state.selected = 0;
            }
            self.sort_items();
            self.update_title();
        }
    }

    pub fn filter_providers(&mut self) {
        let Some(session) = self.session_mut() else {
            return;
        };
        if let (PickerMode::Provider, PickerData::Provider(provider_state)) =
            (session.mode, &mut session.data)
        {
            let search_term = provider_state.search_filter.to_lowercase();
            let filtered: Vec<PickerItem> = if search_term.is_empty() {
                provider_state.all_items.clone()
            } else {
                provider_state
                    .all_items
                    .iter()
                    .filter(|item| {
                        item.id.to_lowercase().contains(&search_term)
                            || item.label.to_lowercase().contains(&search_term)
                    })
                    .cloned()
                    .collect()
            };
            session.state.items = filtered;
            if session.state.selected >= session.state.items.len() {
                session.state.selected = 0;
            }
            self.sort_items();
            self.update_title();
        }
    }

    pub fn revert_model_preview(&mut self, session: &mut SessionContext) {
        let previous_model = self
            .model_state()
            .and_then(|state| state.before_model.clone());

        if let Some(state) = self.model_state_mut() {
            state.before_model = None;
            state.search_filter.clear();
            state.all_items.clear();
            state.has_dates = false;
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
            .provider_state()
            .and_then(|state| state.before_provider.clone());

        if let Some(state) = self.provider_state_mut() {
            state.before_provider = None;
            state.search_filter.clear();
            state.all_items.clear();
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

    pub fn sort_items(&mut self) {
        let prefers_alpha = self.prefers_alphabetical();
        if let Some(session) = self.session_mut() {
            let picker = &mut session.state;

            // Extract special entries (like "turn off character mode") that should stay at top
            let mut special_entries = Vec::new();
            let mut regular_items = Vec::new();

            for item in picker.items.drain(..) {
                if item.id == TURN_OFF_CHARACTER_ID {
                    special_entries.push(item);
                } else {
                    regular_items.push(item);
                }
            }

            // Sort regular items
            if prefers_alpha {
                match picker.sort_mode {
                    SortMode::Date => {
                        regular_items.sort_by(|a, b| b.label.cmp(&a.label));
                    }
                    SortMode::Name => {
                        regular_items.sort_by(|a, b| a.label.cmp(&b.label));
                    }
                }
            } else {
                match picker.sort_mode {
                    SortMode::Date => {
                        regular_items.sort_by(|a, b| match (&a.sort_key, &b.sort_key) {
                            (Some(a_key), Some(b_key)) => b_key.cmp(a_key),
                            (Some(_), None) => std::cmp::Ordering::Less,
                            (None, Some(_)) => std::cmp::Ordering::Greater,
                            (None, None) => b.label.cmp(&a.label),
                        });
                    }
                    SortMode::Name => {
                        regular_items.sort_by(|a, b| a.label.cmp(&b.label));
                    }
                }
            }

            // Rebuild items with special entries first
            picker.items = special_entries;
            picker.items.extend(regular_items);

            if picker.selected >= picker.items.len() {
                picker.selected = 0;
            }
        }
    }

    pub fn update_title(&mut self) {
        let Some(session) = self.session_mut() else {
            return;
        };

        let prefers_alpha = session.prefers_alphabetical();
        let base_title = session.base_title();
        let item_count = session.all_items().len();
        let threshold = session.filter_hint_threshold();
        let search_filter = session.search_filter().clone();

        let picker = &mut session.state;
        let sort_text = if prefers_alpha {
            match picker.sort_mode {
                SortMode::Name => "A-Z",
                SortMode::Date => "Z-A",
            }
        } else {
            match picker.sort_mode {
                SortMode::Date => "date",
                SortMode::Name => "name",
            }
        };

        picker.title = if search_filter.is_empty() {
            if item_count > threshold {
                format!(
                    "{} ({} available - Sort by: {} - type to filter)",
                    base_title, item_count, sort_text
                )
            } else {
                format!("{} (Sort by: {})", base_title, sort_text)
            }
        } else {
            format!(
                "{} (filter: '{}' - {} matches - Sort by: {})",
                base_title,
                search_filter,
                picker.items.len(),
                sort_text
            )
        };
    }

    pub fn open_provider_picker(&mut self, session_context: &SessionContext) -> Result<(), String> {
        let auth_manager = AuthManager::new();
        let cfg = Config::load_test_safe();
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
                let metadata = if is_default {
                    Some(format!(
                        "Built-in provider ({}) (default from config)",
                        builtin_provider.base_url
                    ))
                } else {
                    Some(format!("Built-in provider ({})", builtin_provider.base_url))
                };
                items.push(PickerItem {
                    id: builtin_provider.id.clone(),
                    label,
                    metadata,
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
                let metadata = if is_default {
                    Some(format!(
                        "Custom provider ({}) (default from config)",
                        base_url
                    ))
                } else {
                    Some(format!("Custom provider ({})", base_url))
                };
                items.push(PickerItem {
                    id,
                    label,
                    metadata,
                    sort_key: Some(display_name),
                });
            }
        }

        if items.is_empty() {
            return Err(
                "No configured providers found. Run 'chabeau auth' to set up authentication."
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
        let mut session = PickerSession {
            mode: PickerMode::Provider,
            state: picker_state,
            data: PickerData::Provider(ProviderPickerState {
                search_filter: String::new(),
                all_items: items,
                before_provider: Some((
                    session_context.provider_name.clone(),
                    session_context.provider_display_name.clone(),
                )),
            }),
        };

        session.state.sort_mode = session.default_sort_mode();
        self.picker_session = Some(session);

        self.sort_items();
        self.update_title();

        let current_provider = session_context.provider_name.clone();
        if let Some(session) = self.session_mut() {
            if let Some((idx, _)) = session
                .state
                .items
                .iter()
                .enumerate()
                .find(|(_, it)| it.id == current_provider)
            {
                session.state.selected = idx;
            }
        }

        Ok(())
    }

    pub fn filter_characters(&mut self) {
        let Some(session) = self.session_mut() else {
            return;
        };
        if let (PickerMode::Character, PickerData::Character(character_state)) =
            (session.mode, &mut session.data)
        {
            let search_term = character_state.search_filter.to_lowercase();
            let filtered: Vec<PickerItem> = if search_term.is_empty() {
                character_state.all_items.clone()
            } else {
                character_state
                    .all_items
                    .iter()
                    .filter(|item| {
                        // Always include the special "turn off" entry
                        item.id == TURN_OFF_CHARACTER_ID
                            || item.id.to_lowercase().contains(&search_term)
                            || item.label.to_lowercase().contains(&search_term)
                            || item
                                .metadata
                                .as_ref()
                                .map(|m| m.to_lowercase().contains(&search_term))
                                .unwrap_or(false)
                    })
                    .cloned()
                    .collect()
            };
            session.state.items = filtered;
            if session.state.selected >= session.state.items.len() {
                session.state.selected = 0;
            }
            self.sort_items();
            self.update_title();
        }
    }

    pub fn open_character_picker(
        &mut self,
        character_cache: &mut crate::character::cache::CardCache,
        session_context: &SessionContext,
    ) -> Result<(), String> {
        // Load all character metadata (uses cache)
        let cards = character_cache
            .get_all_metadata()
            .map_err(|e| format!("Error loading characters: {}", e))?;

        if cards.is_empty() {
            return Err(
                "No character cards found. Use 'chabeau import <file>' to import cards."
                    .to_string(),
            );
        }

        // Get the default character for the current provider/model
        let cfg = Config::load_test_safe();
        let default_character =
            cfg.get_default_character(&session_context.provider_name, &session_context.model);

        let mut items: Vec<PickerItem> = cards
            .into_iter()
            .map(|card| {
                let sanitized_description = sanitize_picker_metadata(&card.description);
                let is_default = default_character
                    .map(|def| def == &card.name)
                    .unwrap_or(false);
                let label = if is_default {
                    format!("{}*", card.name)
                } else {
                    card.name.clone()
                };
                PickerItem {
                    id: card.name.clone(),
                    label,
                    metadata: Some(sanitized_description),
                    sort_key: Some(card.name.clone()),
                }
            })
            .collect();

        // Add "turn off character mode" entry at the beginning if a character is active
        if session_context.active_character.is_some() {
            items.insert(
                0,
                PickerItem {
                    id: TURN_OFF_CHARACTER_ID.to_string(),
                    label: "[Turn off character mode]".to_string(),
                    metadata: Some("Disable character and return to normal mode".to_string()),
                    sort_key: None,
                },
            );
        }

        let selected = 0;
        let picker_state = PickerState::new("Pick Character", items.clone(), selected);
        let mut session = PickerSession {
            mode: PickerMode::Character,
            state: picker_state,
            data: PickerData::Character(CharacterPickerState {
                search_filter: String::new(),
                all_items: items,
            }),
        };

        session.state.sort_mode = session.default_sort_mode();
        self.picker_session = Some(session);

        self.sort_items();
        self.update_title();

        Ok(())
    }

    fn prefers_alphabetical(&self) -> bool {
        self.session()
            .map(|session| session.prefers_alphabetical())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_picker_metadata_removes_newlines() {
        let input = "Line 1\nLine 2\nLine 3";
        let result = sanitize_picker_metadata(input);
        assert_eq!(result, "Line 1 Line 2 Line 3");
    }

    #[test]
    fn test_sanitize_picker_metadata_removes_carriage_returns() {
        let input = "Line 1\r\nLine 2\r\nLine 3";
        let result = sanitize_picker_metadata(input);
        assert_eq!(result, "Line 1 Line 2 Line 3");
    }

    #[test]
    fn test_sanitize_picker_metadata_collapses_whitespace() {
        let input = "Too    many     spaces";
        let result = sanitize_picker_metadata(input);
        assert_eq!(result, "Too many spaces");
    }

    #[test]
    fn test_sanitize_picker_metadata_removes_control_chars() {
        let input = "Text\twith\ttabs\x00and\x01control\x02chars";
        let result = sanitize_picker_metadata(input);
        assert_eq!(result, "Text with tabs and control chars");
    }

    #[test]
    fn test_sanitize_picker_metadata_handles_mixed_whitespace() {
        let input = "Mixed\n\r\t  whitespace\n\n\nhere";
        let result = sanitize_picker_metadata(input);
        assert_eq!(result, "Mixed whitespace here");
    }

    #[test]
    fn test_sanitize_picker_metadata_preserves_normal_text() {
        let input = "Normal text with spaces";
        let result = sanitize_picker_metadata(input);
        assert_eq!(result, "Normal text with spaces");
    }

    #[test]
    fn test_sanitize_picker_metadata_handles_empty_string() {
        let input = "";
        let result = sanitize_picker_metadata(input);
        assert_eq!(result, "");
    }

    #[test]
    fn test_sanitize_picker_metadata_handles_only_whitespace() {
        let input = "\n\r\t   \n";
        let result = sanitize_picker_metadata(input);
        assert_eq!(result, "");
    }

    #[test]
    fn test_turn_off_character_entry_added_when_character_active() {
        use crate::character::cache::CardCache;
        use crate::character::card::{CharacterCard, CharacterData};
        use crate::utils::test_utils::{create_test_app, TestEnvVarGuard};
        use std::fs;
        use tempfile::tempdir;

        let temp_dir = tempdir().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        fs::create_dir_all(&cards_dir).unwrap();

        let card_json = serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "TestChar",
                "description": "Test",
                "personality": "Friendly",
                "scenario": "Testing",
                "first_mes": "Hello!",
                "mes_example": ""
            }
        });

        fs::write(cards_dir.join("test.json"), card_json.to_string()).unwrap();

        let mut app = create_test_app();
        let mut cache = CardCache::new();

        app.session.set_character(CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "TestChar".to_string(),
                description: "Test".to_string(),
                personality: "Friendly".to_string(),
                scenario: "Testing".to_string(),
                first_mes: "Hello!".to_string(),
                mes_example: String::new(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        });

        let mut env_guard = TestEnvVarGuard::new();
        env_guard.set_var("CHABEAU_CARDS_DIR", cards_dir.as_os_str());

        let result = app.picker.open_character_picker(&mut cache, &app.session);

        assert!(result.is_ok());

        let picker_items = &app.picker.session().unwrap().state.items;
        assert!(picker_items.len() >= 2);
        assert_eq!(picker_items[0].id, TURN_OFF_CHARACTER_ID);
        assert_eq!(picker_items[0].label, "[Turn off character mode]");
    }

    #[test]
    fn test_turn_off_character_entry_not_added_when_no_character() {
        use crate::character::cache::CardCache;
        use crate::utils::test_utils::{create_test_app, TestEnvVarGuard};
        use std::fs;
        use tempfile::tempdir;

        let temp_dir = tempdir().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        fs::create_dir_all(&cards_dir).unwrap();

        let card_json = serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "TestChar",
                "description": "Test",
                "personality": "Friendly",
                "scenario": "Testing",
                "first_mes": "Hello!",
                "mes_example": ""
            }
        });

        fs::write(cards_dir.join("test.json"), card_json.to_string()).unwrap();

        let mut app = create_test_app();
        let mut cache = CardCache::new();

        assert!(app.session.active_character.is_none());

        let mut env_guard = TestEnvVarGuard::new();
        env_guard.set_var("CHABEAU_CARDS_DIR", cards_dir.as_os_str());

        let result = app.picker.open_character_picker(&mut cache, &app.session);

        assert!(result.is_ok());

        let picker_items = &app.picker.session().unwrap().state.items;
        assert!(!picker_items
            .iter()
            .any(|item| item.id == TURN_OFF_CHARACTER_ID));
    }

    #[test]
    fn test_turn_off_character_stays_at_top_after_sort() {
        use crate::character::cache::CardCache;
        use crate::character::card::{CharacterCard, CharacterData};
        use crate::utils::test_utils::{create_test_app, TestEnvVarGuard};
        use std::fs;
        use tempfile::tempdir;

        let temp_dir = tempdir().unwrap();
        let cards_dir = temp_dir.path().join("cards");
        fs::create_dir_all(&cards_dir).unwrap();

        for name in &["Alice", "Bob", "Charlie"] {
            let card_json = serde_json::json!({
                "spec": "chara_card_v2",
                "spec_version": "2.0",
                "data": {
                    "name": name,
                    "description": "Test",
                    "personality": "Friendly",
                    "scenario": "Testing",
                    "first_mes": "Hello!",
                    "mes_example": ""
                }
            });
            fs::write(
                cards_dir.join(format!("{}.json", name.to_lowercase())),
                card_json.to_string(),
            )
            .unwrap();
        }

        let mut app = create_test_app();
        let mut cache = CardCache::new();

        app.session.set_character(CharacterCard {
            spec: "chara_card_v2".to_string(),
            spec_version: "2.0".to_string(),
            data: CharacterData {
                name: "Alice".to_string(),
                description: "Test".to_string(),
                personality: "Friendly".to_string(),
                scenario: "Testing".to_string(),
                first_mes: "Hello!".to_string(),
                mes_example: String::new(),
                creator_notes: None,
                system_prompt: None,
                post_history_instructions: None,
                alternate_greetings: None,
                tags: None,
                creator: None,
                character_version: None,
            },
        });

        let mut env_guard = TestEnvVarGuard::new();
        env_guard.set_var("CHABEAU_CARDS_DIR", cards_dir.as_os_str());

        let result = app.picker.open_character_picker(&mut cache, &app.session);

        assert!(result.is_ok());

        let items_before_sort = app.picker.session().unwrap().state.items.len();
        assert!(items_before_sort >= 4); // turn off + 3 characters

        app.picker.sort_items();

        let picker_items = &app.picker.session().unwrap().state.items;

        // First item should always be the turn off entry, regardless of sort
        assert_eq!(picker_items[0].id, TURN_OFF_CHARACTER_ID);
        assert_eq!(picker_items[0].label, "[Turn off character mode]");

        // Verify we still have all items after sorting
        assert_eq!(picker_items.len(), items_before_sort);
    }
}
