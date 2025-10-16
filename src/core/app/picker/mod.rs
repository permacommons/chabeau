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
/// Special ID for the "turn off persona" picker entry
pub(super) const TURN_OFF_PERSONA_ID: &str = "[turn_off_persona]";
/// Special ID for the "turn off preset" picker entry
pub(super) const TURN_OFF_PRESET_ID: &str = "[turn_off_preset]";

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
    Persona,
    Preset,
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
pub struct PersonaPickerState {
    pub search_filter: String,
    pub all_items: Vec<PickerItem>,
}

#[derive(Debug, Clone)]
pub struct PresetPickerState {
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
    Persona(PersonaPickerState),
    Preset(PresetPickerState),
}

impl PickerData {
    pub fn mode(&self) -> PickerMode {
        match self {
            PickerData::Theme(_) => PickerMode::Theme,
            PickerData::Model(_) => PickerMode::Model,
            PickerData::Provider(_) => PickerMode::Provider,
            PickerData::Character(_) => PickerMode::Character,
            PickerData::Persona(_) => PickerMode::Persona,
            PickerData::Preset(_) => PickerMode::Preset,
        }
    }

    fn prefers_alphabetical(&self) -> bool {
        match self {
            PickerData::Model(state) => !state.has_dates,
            PickerData::Theme(_)
            | PickerData::Provider(_)
            | PickerData::Character(_)
            | PickerData::Persona(_)
            | PickerData::Preset(_) => true,
        }
    }

    fn filter_hint_threshold(&self) -> usize {
        match self.mode() {
            PickerMode::Model => 20,
            _ => 10,
        }
    }

    fn base_title(&self) -> &'static str {
        match self.mode() {
            PickerMode::Model => "Pick Model",
            PickerMode::Provider => "Pick Provider",
            PickerMode::Theme => "Pick Theme",
            PickerMode::Character => "Pick Character",
            PickerMode::Persona => "Pick Persona",
            PickerMode::Preset => "Pick Preset",
        }
    }

    fn search_filter(&self) -> &String {
        match self {
            PickerData::Theme(state) => &state.search_filter,
            PickerData::Model(state) => &state.search_filter,
            PickerData::Provider(state) => &state.search_filter,
            PickerData::Character(state) => &state.search_filter,
            PickerData::Persona(state) => &state.search_filter,
            PickerData::Preset(state) => &state.search_filter,
        }
    }

    fn all_items(&self) -> &Vec<PickerItem> {
        match self {
            PickerData::Theme(state) => &state.all_items,
            PickerData::Model(state) => &state.all_items,
            PickerData::Provider(state) => &state.all_items,
            PickerData::Character(state) => &state.all_items,
            PickerData::Persona(state) => &state.all_items,
            PickerData::Preset(state) => &state.all_items,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PickerSession {
    pub state: PickerState,
    pub data: PickerData,
}

macro_rules! picker_state_accessors {
    ($(($variant:ident, $getter:ident, $getter_mut:ident, $state:ty)),+ $(,)?) => {
        impl PickerData {
            $(
                pub fn $getter(&self) -> Option<&$state> {
                    if let Self::$variant(state) = self {
                        Some(state)
                    } else {
                        None
                    }
                }

                pub fn $getter_mut(&mut self) -> Option<&mut $state> {
                    if let Self::$variant(state) = self {
                        Some(state)
                    } else {
                        None
                    }
                }
            )+
        }

        impl PickerSession {
            $(
                pub fn $getter(&self) -> Option<&$state> {
                    self.data.$getter()
                }

                pub fn $getter_mut(&mut self) -> Option<&mut $state> {
                    self.data.$getter_mut()
                }
            )+
        }
    };
}

impl PickerSession {
    pub fn mode(&self) -> PickerMode {
        self.data.mode()
    }

    fn prefers_alphabetical(&self) -> bool {
        self.data.prefers_alphabetical()
    }

    pub(crate) fn default_sort_mode(&self) -> SortMode {
        if self.prefers_alphabetical() {
            SortMode::Name
        } else {
            SortMode::Date
        }
    }

    fn filter_hint_threshold(&self) -> usize {
        self.data.filter_hint_threshold()
    }

    fn base_title(&self) -> &'static str {
        self.data.base_title()
    }

    fn search_filter(&self) -> &String {
        self.data.search_filter()
    }

    fn all_items(&self) -> &Vec<PickerItem> {
        self.data.all_items()
    }
}

picker_state_accessors! {
    (Theme, theme_state, theme_state_mut, ThemePickerState),
    (Model, model_state, model_state_mut, ModelPickerState),
    (Provider, provider_state, provider_state_mut, ProviderPickerState),
    (Character, character_state, character_state_mut, CharacterPickerState),
    (Persona, persona_state, persona_state_mut, PersonaPickerState),
    (Preset, preset_state, preset_state_mut, PresetPickerState),
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
        self.session().map(PickerSession::mode)
    }

    pub fn state(&self) -> Option<&PickerState> {
        self.session().map(|session| &session.state)
    }

    pub fn state_mut(&mut self) -> Option<&mut PickerState> {
        self.session_mut().map(|session| &mut session.state)
    }

    pub fn close(&mut self) {
        self.picker_session = None;
    }

    pub fn open_theme_picker(
        &mut self,
        ui: &mut UiState,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let cfg = Config::load_test_safe()?;

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
        Ok(())
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

    fn filter_session_items(&mut self, expected_mode: PickerMode, special_ids: &[&str]) {
        let Some(session) = self.session_mut() else {
            return;
        };

        if session.mode() != expected_mode {
            return;
        }

        let search_term = session.search_filter().to_lowercase();
        let all_items = session.all_items();
        session.state.items = if search_term.is_empty() {
            all_items.clone()
        } else {
            all_items
                .iter()
                .filter(|item| {
                    let matches_text = item.id.to_lowercase().contains(&search_term)
                        || item.label.to_lowercase().contains(&search_term)
                        || item
                            .metadata
                            .as_ref()
                            .map(|metadata| metadata.to_lowercase().contains(&search_term))
                            .unwrap_or(false);

                    matches_text || special_ids.iter().any(|special_id| item.id == *special_id)
                })
                .cloned()
                .collect()
        };

        if session.state.selected >= session.state.items.len() {
            session.state.selected = 0;
        }

        self.sort_items();
        self.update_title();
    }

    pub fn filter_models(&mut self) {
        self.filter_session_items(PickerMode::Model, &[]);
    }

    pub fn filter_themes(&mut self) {
        self.filter_session_items(PickerMode::Theme, &[]);
    }

    pub fn filter_providers(&mut self) {
        self.filter_session_items(PickerMode::Provider, &[]);
    }

    pub fn revert_model_preview(&mut self, session: &mut SessionContext) {
        let previous_model = self
            .session()
            .and_then(PickerSession::model_state)
            .and_then(|state| state.before_model.clone());

        if let Some(session) = self.session_mut() {
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
            .session()
            .and_then(PickerSession::provider_state)
            .and_then(|state| state.before_provider.clone());

        if let Some(session) = self.session_mut() {
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

    pub fn sort_items(&mut self) {
        let prefers_alpha = self.prefers_alphabetical();
        if let Some(session) = self.session_mut() {
            let picker = &mut session.state;

            // Extract special entries (like "turn off character mode") that should stay at top
            let mut special_entries = Vec::new();
            let mut regular_items = Vec::new();

            for item in picker.items.drain(..) {
                if item.id == TURN_OFF_CHARACTER_ID
                    || item.id == TURN_OFF_PERSONA_ID
                    || item.id == TURN_OFF_PRESET_ID
                {
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
        self.filter_session_items(PickerMode::Character, &[TURN_OFF_CHARACTER_ID]);
    }

    pub fn filter_personas(&mut self) {
        self.filter_session_items(PickerMode::Persona, &[TURN_OFF_PERSONA_ID]);
    }

    pub fn filter_presets(&mut self) {
        self.filter_session_items(PickerMode::Preset, &[TURN_OFF_PRESET_ID]);
    }

    pub fn open_character_picker(
        &mut self,
        cards: Vec<crate::character::cache::CachedCardMetadata>,
        session_context: &SessionContext,
    ) -> Result<(), String> {
        if cards.is_empty() {
            return Err(
                "No character cards found. Use 'chabeau import <file>' to import cards."
                    .to_string(),
            );
        }

        // Get the default character for the current provider/model
        let cfg = Config::load_test_safe().map_err(|err| err.to_string())?;
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

    pub fn open_persona_picker(
        &mut self,
        persona_manager: &crate::core::persona::PersonaManager,
        session_context: &SessionContext,
    ) -> Result<(), String> {
        let personas = persona_manager.list_personas();

        if personas.is_empty() {
            return Err("No personas found. Add personas to your config.toml file.".to_string());
        }

        // Get the default persona for the current provider/model
        let default_persona = persona_manager
            .get_default_for_provider_model(&session_context.provider_name, &session_context.model);

        let active_character_name = session_context
            .get_character()
            .map(|character| character.data.name.as_str());

        let mut items: Vec<PickerItem> = personas
            .iter()
            .map(|persona| {
                let is_default = default_persona
                    .map(|def| def == persona.id)
                    .unwrap_or(false);
                let display_label = if is_default {
                    format!("{} ({})*", persona.display_name, persona.id)
                } else {
                    format!("{} ({})", persona.display_name, persona.id)
                };
                let metadata = persona
                    .bio
                    .as_ref()
                    .and_then(|bio| {
                        let char_replacement = active_character_name.unwrap_or("Assistant");
                        let user_replacement = persona.display_name.as_str();
                        let substituted = bio
                            .replace("{{char}}", char_replacement)
                            .replace("{{user}}", user_replacement);
                        let sanitized = sanitize_picker_metadata(&substituted);
                        if sanitized.is_empty() {
                            None
                        } else {
                            Some(sanitized)
                        }
                    })
                    .unwrap_or_else(|| "No bio".to_string());
                PickerItem {
                    id: persona.id.clone(),
                    label: display_label,
                    metadata: Some(metadata),
                    sort_key: Some(persona.display_name.clone()),
                }
            })
            .collect();

        // Add "turn off persona" entry at the beginning if a persona is active
        if persona_manager.get_active_persona().is_some() {
            items.insert(
                0,
                PickerItem {
                    id: TURN_OFF_PERSONA_ID.to_string(),
                    label: "[Turn off persona]".to_string(),
                    metadata: Some(
                        "Deactivate current persona and return to normal mode".to_string(),
                    ),
                    sort_key: None,
                },
            );
        }

        let selected = 0;
        let picker_state = PickerState::new("Pick Persona", items.clone(), selected);
        let mut session = PickerSession {
            state: picker_state,
            data: PickerData::Persona(PersonaPickerState {
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

    pub fn open_preset_picker(
        &mut self,
        preset_manager: &crate::core::preset::PresetManager,
        session_context: &SessionContext,
    ) -> Result<(), String> {
        let presets = preset_manager.list_presets();

        if presets.is_empty() {
            return Err("No presets found. Add presets to your config.toml file.".to_string());
        }

        let default_preset = preset_manager
            .get_default_for_provider_model(&session_context.provider_name, &session_context.model);

        let mut items: Vec<PickerItem> = presets
            .iter()
            .map(|preset| {
                let is_default = default_preset.map(|def| def == preset.id).unwrap_or(false);
                let label = if is_default {
                    format!("{}*", preset.id)
                } else {
                    preset.id.clone()
                };

                let mut parts = Vec::new();
                let pre_trim = preset.pre.trim();
                if !pre_trim.is_empty() {
                    let sanitized = sanitize_picker_metadata(pre_trim);
                    if !sanitized.is_empty() {
                        parts.push(format!("Pre: {}", sanitized));
                    }
                }
                let post_trim = preset.post.trim();
                if !post_trim.is_empty() {
                    let sanitized = sanitize_picker_metadata(post_trim);
                    if !sanitized.is_empty() {
                        parts.push(format!("Post: {}", sanitized));
                    }
                }

                let metadata = if parts.is_empty() {
                    Some("No instructions".to_string())
                } else {
                    Some(parts.join(" • "))
                };

                PickerItem {
                    id: preset.id.clone(),
                    label,
                    metadata,
                    sort_key: Some(preset.id.clone()),
                }
            })
            .collect();

        if preset_manager.get_active_preset().is_some() {
            items.insert(
                0,
                PickerItem {
                    id: TURN_OFF_PRESET_ID.to_string(),
                    label: "[Turn off preset]".to_string(),
                    metadata: Some("Deactivate current preset".to_string()),
                    sort_key: None,
                },
            );
        }

        let selected = 0usize;
        let picker_state = PickerState::new("Pick Preset", items.clone(), selected);
        let mut session = PickerSession {
            state: picker_state,
            data: PickerData::Preset(PresetPickerState {
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

    fn picker_item(id: &str, label: &str, metadata: Option<&str>) -> PickerItem {
        PickerItem {
            id: id.to_string(),
            label: label.to_string(),
            metadata: metadata.map(|value| value.to_string()),
            sort_key: None,
        }
    }

    #[test]
    fn test_filter_models_resets_selection_and_matches_case_insensitively() {
        let mut controller = PickerController::new();
        let items = vec![
            picker_item("gpt-4", "GPT-4", None),
            picker_item("gpt-3.5", "GPT-3.5", None),
            picker_item("claude-3", "Claude 3", None),
        ];

        let mut picker_state = PickerState::new("Pick Model", items.clone(), 2);
        picker_state.sort_mode = SortMode::Name;

        let mut session = PickerSession {
            state: picker_state,
            data: PickerData::Model(ModelPickerState {
                search_filter: "GPT".to_string(),
                all_items: items,
                before_model: None,
                has_dates: false,
            }),
        };
        session.state.sort_mode = session.default_sort_mode();

        controller.picker_session = Some(session);
        controller.filter_models();

        let session = controller.session().expect("model picker session");
        assert_eq!(session.state.selected, 0);
        assert_eq!(session.state.items.len(), 2);
        let ids: Vec<&str> = session
            .state
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        assert!(ids.contains(&"gpt-4"));
        assert!(ids.contains(&"gpt-3.5"));
    }

    #[test]
    fn test_filter_characters_preserves_special_entry_and_selection_bounds() {
        let mut controller = PickerController::new();
        let items = vec![
            picker_item(
                TURN_OFF_CHARACTER_ID,
                "[Turn off character mode]",
                Some("Disable character"),
            ),
            picker_item("alice", "Alice", Some("Friendly adventurer")),
            picker_item("gamma", "Gamma", Some("Galactic explorer")),
        ];

        let mut picker_state = PickerState::new("Pick Character", items.clone(), 2);
        picker_state.sort_mode = SortMode::Name;

        let mut session = PickerSession {
            state: picker_state,
            data: PickerData::Character(CharacterPickerState {
                search_filter: "GAMMA".to_string(),
                all_items: items,
            }),
        };
        session.state.sort_mode = session.default_sort_mode();

        controller.picker_session = Some(session);
        controller.filter_characters();

        let session = controller.session().expect("character picker session");
        assert_eq!(session.state.selected, 0);
        assert_eq!(session.state.items.len(), 2);
        assert_eq!(session.state.items[0].id, TURN_OFF_CHARACTER_ID);
        assert!(session.state.items.iter().any(|item| item.id == "gamma"));
    }

    #[test]
    fn test_filter_personas_preserves_special_entry_and_selection_bounds() {
        let mut controller = PickerController::new();
        let items = vec![
            picker_item(
                TURN_OFF_PERSONA_ID,
                "[Turn off persona]",
                Some("Deactivate persona"),
            ),
            picker_item("mentor", "Mentor", Some("Helpful adviser")),
            picker_item("artist", "Artist", Some("Creative mind")),
        ];

        let mut picker_state = PickerState::new("Pick Persona", items.clone(), 2);
        picker_state.sort_mode = SortMode::Name;

        let mut session = PickerSession {
            state: picker_state,
            data: PickerData::Persona(PersonaPickerState {
                search_filter: "ADVISER".to_string(),
                all_items: items,
            }),
        };
        session.state.sort_mode = session.default_sort_mode();

        controller.picker_session = Some(session);
        controller.filter_personas();

        let session = controller.session().expect("persona picker session");
        assert_eq!(session.state.selected, 0);
        assert_eq!(session.state.items.len(), 2);
        assert_eq!(session.state.items[0].id, TURN_OFF_PERSONA_ID);
        assert!(session.state.items.iter().any(|item| item.id == "mentor"));
    }

    #[test]
    fn test_filter_presets_preserves_special_entry_and_selection_bounds() {
        let mut controller = PickerController::new();
        let items = vec![
            picker_item(
                TURN_OFF_PRESET_ID,
                "[Turn off preset]",
                Some("Deactivate preset"),
            ),
            picker_item("focus", "Focus Mode", Some("Deep work profile")),
            picker_item("chatty", "Chatty", Some("Casual conversation")),
        ];

        let mut picker_state = PickerState::new("Pick Preset", items.clone(), 2);
        picker_state.sort_mode = SortMode::Name;

        let mut session = PickerSession {
            state: picker_state,
            data: PickerData::Preset(PresetPickerState {
                search_filter: "FOCUS".to_string(),
                all_items: items,
            }),
        };
        session.state.sort_mode = session.default_sort_mode();

        controller.picker_session = Some(session);
        controller.filter_presets();

        let session = controller.session().expect("preset picker session");
        assert_eq!(session.state.selected, 0);
        assert_eq!(session.state.items.len(), 2);
        assert_eq!(session.state.items[0].id, TURN_OFF_PRESET_ID);
        assert!(session.state.items.iter().any(|item| item.id == "focus"));
    }

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
        use crate::character::card::{CharacterCard, CharacterData};
        use crate::character::service::CharacterService;
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
        let mut service = CharacterService::new();

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

        let cards = service.list_metadata().expect("metadata");
        let result = app.picker.open_character_picker(cards, &app.session);

        assert!(result.is_ok());

        let picker_items = &app.picker.session().unwrap().state.items;
        assert!(picker_items.len() >= 2);
        assert_eq!(picker_items[0].id, TURN_OFF_CHARACTER_ID);
        assert_eq!(picker_items[0].label, "[Turn off character mode]");
    }

    #[test]
    fn test_turn_off_character_entry_not_added_when_no_character() {
        use crate::character::service::CharacterService;
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
        let mut service = CharacterService::new();

        assert!(app.session.active_character.is_none());

        let mut env_guard = TestEnvVarGuard::new();
        env_guard.set_var("CHABEAU_CARDS_DIR", cards_dir.as_os_str());

        let cards = service.list_metadata().expect("metadata");
        let result = app.picker.open_character_picker(cards, &app.session);

        assert!(result.is_ok());

        let picker_items = &app.picker.session().unwrap().state.items;
        assert!(!picker_items
            .iter()
            .any(|item| item.id == TURN_OFF_CHARACTER_ID));
    }

    #[test]
    fn test_turn_off_character_stays_at_top_after_sort() {
        use crate::character::card::{CharacterCard, CharacterData};
        use crate::character::service::CharacterService;
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
        let mut service = CharacterService::new();

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

        let cards = service.list_metadata().expect("metadata");
        let result = app.picker.open_character_picker(cards, &app.session);

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

    #[test]
    fn test_turn_off_persona_stays_at_top_after_sort() {
        use crate::core::config::{Config, Persona};
        use crate::core::persona::PersonaManager;
        use crate::utils::test_utils::create_test_app;

        let config = Config {
            personas: vec![
                Persona {
                    id: "alpha".to_string(),
                    display_name: "Alpha".to_string(),
                    bio: Some("Alpha bio".to_string()),
                },
                Persona {
                    id: "beta".to_string(),
                    display_name: "Beta".to_string(),
                    bio: Some("Beta bio".to_string()),
                },
                Persona {
                    id: "gamma".to_string(),
                    display_name: "Gamma".to_string(),
                    bio: Some("Gamma bio".to_string()),
                },
            ],
            ..Default::default()
        };

        let mut persona_manager = PersonaManager::load_personas(&config).unwrap();
        persona_manager.set_active_persona("alpha").unwrap();

        let mut app = create_test_app();
        app.persona_manager = persona_manager;

        let result = app
            .picker
            .open_persona_picker(&app.persona_manager, &app.session);

        assert!(result.is_ok());

        let items_before_sort = app.picker.session().unwrap().state.items.len();
        assert!(items_before_sort >= 2);

        app.picker.sort_items();

        let picker_items = &app.picker.session().unwrap().state.items;

        // First item should always be the turn off persona entry, regardless of sort
        assert_eq!(picker_items[0].id, TURN_OFF_PERSONA_ID);
        assert_eq!(picker_items[0].label, "[Turn off persona]");

        // Verify we still have all items after sorting
        assert_eq!(picker_items.len(), items_before_sort);
    }

    #[test]
    fn test_persona_picker_sanitizes_bio_metadata() {
        use crate::core::config::{Config, Persona};
        use crate::core::persona::PersonaManager;
        use crate::utils::test_utils::create_test_app;

        let config = Config {
            personas: vec![Persona {
                id: "neat".to_string(),
                display_name: "Neat".to_string(),
                bio: Some("First line\nSecond\tline".to_string()),
            }],
            ..Default::default()
        };

        let persona_manager = PersonaManager::load_personas(&config).unwrap();

        let mut app = create_test_app();
        app.persona_manager = persona_manager;

        app.picker
            .open_persona_picker(&app.persona_manager, &app.session)
            .unwrap();

        let picker_items = &app.picker.session().unwrap().state.items;
        let persona_item = picker_items
            .iter()
            .find(|item| item.id == "neat")
            .expect("persona entry present");

        assert_eq!(
            persona_item.metadata.as_deref(),
            Some("First line Second line")
        );
    }

    #[test]
    fn test_persona_picker_metadata_defaults_to_no_bio_when_empty() {
        use crate::core::config::{Config, Persona};
        use crate::core::persona::PersonaManager;
        use crate::utils::test_utils::create_test_app;

        let config = Config {
            personas: vec![Persona {
                id: "blank".to_string(),
                display_name: "Blank".to_string(),
                bio: Some("   \n\t".to_string()),
            }],
            ..Default::default()
        };

        let persona_manager = PersonaManager::load_personas(&config).unwrap();

        let mut app = create_test_app();
        app.persona_manager = persona_manager;

        app.picker
            .open_persona_picker(&app.persona_manager, &app.session)
            .unwrap();

        let picker_items = &app.picker.session().unwrap().state.items;
        let persona_item = picker_items
            .iter()
            .find(|item| item.id == "blank")
            .expect("persona entry present");

        assert_eq!(persona_item.metadata.as_deref(), Some("No bio"));
    }
}
