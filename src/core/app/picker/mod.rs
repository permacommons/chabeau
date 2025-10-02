use super::{SessionContext, UiState};
use crate::api::models::{fetch_models, sort_models};
use crate::auth::AuthManager;
use crate::core::builtin_providers::load_builtin_providers;
use crate::core::config::Config;
use crate::ui::builtin_themes::load_builtin_themes;
use crate::ui::picker::{PickerItem, PickerState, SortMode};
use crate::ui::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerMode {
    Theme,
    Model,
    Provider,
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
#[allow(clippy::large_enum_variant)]
pub enum PickerData {
    Theme(ThemePickerState),
    Model(ModelPickerState),
    Provider(ProviderPickerState),
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
            (PickerMode::Theme, _) | (PickerMode::Provider, _) => true,
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
        }
    }

    fn search_filter(&self) -> &String {
        match &self.data {
            PickerData::Model(state) => &state.search_filter,
            PickerData::Theme(state) => &state.search_filter,
            PickerData::Provider(state) => &state.search_filter,
        }
    }

    fn all_items(&self) -> &Vec<PickerItem> {
        match &self.data {
            PickerData::Model(state) => &state.all_items,
            PickerData::Theme(state) => &state.all_items,
            PickerData::Provider(state) => &state.all_items,
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

    pub async fn open_model_picker(
        &mut self,
        session_context: &SessionContext,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let cfg = Config::load_test_safe();
        let default_model_for_provider = cfg
            .get_default_model(&session_context.provider_name)
            .cloned();

        let models_response = fetch_models(
            &session_context.client,
            &session_context.base_url,
            &session_context.api_key,
            &session_context.provider_name,
        )
        .await?;

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
            if prefers_alpha {
                match picker.sort_mode {
                    SortMode::Date => {
                        picker.items.sort_by(|a, b| b.label.cmp(&a.label));
                    }
                    SortMode::Name => {
                        picker.items.sort_by(|a, b| a.label.cmp(&b.label));
                    }
                }
            } else {
                match picker.sort_mode {
                    SortMode::Date => {
                        picker
                            .items
                            .sort_by(|a, b| match (&a.sort_key, &b.sort_key) {
                                (Some(a_key), Some(b_key)) => b_key.cmp(a_key),
                                (Some(_), None) => std::cmp::Ordering::Less,
                                (None, Some(_)) => std::cmp::Ordering::Greater,
                                (None, None) => b.label.cmp(&a.label),
                            });
                    }
                    SortMode::Name => {
                        picker.items.sort_by(|a, b| a.label.cmp(&b.label));
                    }
                }
            }

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

    fn prefers_alphabetical(&self) -> bool {
        self.session()
            .map(|session| session.prefers_alphabetical())
            .unwrap_or(false)
    }
}
