//! Picker domain state and reducers for selectable configuration surfaces.
//!
//! # Ownership boundary
//! This module owns active picker state (`ActivePicker`, filtered items,
//! selection, inspect metadata) and picker transition bookkeeping used by the
//! app shell. It delegates rendering to `ui::picker` and delegates event routing
//! to `core::app::actions::picker`.
//!
//! # Main structures and invariants
//! - [`PickerController`] stores at most one active picker.
//! - [`PickerData`] carries mode-specific backing data and preserves the
//!   unfiltered item list for search.
//! - Picker sort/filter/title fields are updated together via helper reducers.
//!
//! # Layout
//! Shared types and helpers live here. Mode-specific [`PickerController`]
//! methods live in `model_provider`, `theme` and `roleplay`.
//!
//! # Call flow entrypoints
//! Picker actions from the event loop call methods on [`PickerController`]
//! through `App` adapters. Selection application may emit follow-up
//! [`crate::core::app::actions::AppCommand`] values (for example model loading)
//! through the action layer.

use crate::ui::picker::{PickerItem, PickerState, SortMode};
use crate::ui::theme::Theme;

mod inspect;
mod model_provider;
mod roleplay;
mod theme;
pub(crate) use inspect::build_inspect_text;

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
pub(super) fn sanitize_picker_metadata(text: &str) -> String {
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

/// Prepare metadata text for the inspect view, preserving intentional
/// newlines while stripping any other control characters.
pub(super) fn sanitize_picker_metadata_for_inspect(text: &str) -> String {
    let mut cleaned = String::with_capacity(text.len());

    for c in text.chars() {
        if c == '\n' {
            cleaned.push('\n');
        } else if c == '\r' || (c.is_control() && c != '\n') {
            continue;
        } else {
            cleaned.push(c);
        }
    }

    cleaned.trim().to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerMode {
    Theme,
    Model,
    Provider,
    Character,
    Persona,
    Preset,
    SavedSession,
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

/// State for the saved-session picker.
#[derive(Debug, Clone)]
pub struct SavedSessionPickerState {
    pub sessions: Vec<crate::core::session_store::SessionSummary>,
    pub selected_index: usize,
    pub search_filter: String,
    pub all_items: Vec<PickerItem>,
}

#[derive(Debug, Clone)]
pub enum PickerData {
    Theme(Box<ThemePickerState>),
    Model(Box<ModelPickerState>),
    Provider(Box<ProviderPickerState>),
    Character(CharacterPickerState),
    Persona(PersonaPickerState),
    Preset(PresetPickerState),
    SavedSession(SavedSessionPickerState),
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
            PickerData::SavedSession(_) => PickerMode::SavedSession,
        }
    }

    fn prefers_alphabetical(&self) -> bool {
        match self {
            PickerData::Model(state) => !state.has_dates,
            PickerData::Theme(_)
            | PickerData::Provider(_)
            | PickerData::Character(_)
            | PickerData::Persona(_)
            | PickerData::Preset(_)
            | PickerData::SavedSession(_) => true,
        }
    }

    fn filter_hint_threshold(&self) -> usize {
        match self.mode() {
            PickerMode::Model => 20,
            _ => 10,
        }
    }

    pub(crate) fn base_title(&self) -> &'static str {
        match self.mode() {
            PickerMode::Model => "Pick Model",
            PickerMode::Provider => "Pick Provider",
            PickerMode::Theme => "Pick Theme",
            PickerMode::Character => "Pick Character",
            PickerMode::Persona => "Pick Persona",
            PickerMode::Preset => "Pick Preset",
            PickerMode::SavedSession => "Load Session",
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
            PickerData::SavedSession(state) => &state.search_filter,
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
            PickerData::SavedSession(state) => &state.all_items,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ActivePicker {
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

        impl ActivePicker {
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

impl ActivePicker {
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

    pub(crate) fn base_title(&self) -> &'static str {
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
    (SavedSession, saved_session_state, saved_session_state_mut, SavedSessionPickerState),
}

pub struct PickerController {
    pub active_picker: Option<ActivePicker>,
    pub in_provider_model_transition: bool,
    pub provider_model_transition_state: Option<(String, String, String, String, String)>,
    pub startup_requires_provider: bool,
    pub startup_requires_model: bool,
    pub startup_multiple_providers_available: bool,
}

impl PickerController {
    pub(crate) fn new() -> Self {
        Self {
            active_picker: None,
            in_provider_model_transition: false,
            provider_model_transition_state: None,
            startup_requires_provider: false,
            startup_requires_model: false,
            startup_multiple_providers_available: false,
        }
    }

    pub fn active(&self) -> Option<&ActivePicker> {
        self.active_picker.as_ref()
    }

    pub fn active_mut(&mut self) -> Option<&mut ActivePicker> {
        self.active_picker.as_mut()
    }

    pub fn current_mode(&self) -> Option<PickerMode> {
        self.active().map(ActivePicker::mode)
    }

    pub fn state(&self) -> Option<&PickerState> {
        self.active().map(|session| &session.state)
    }

    pub fn state_mut(&mut self) -> Option<&mut PickerState> {
        self.active_mut().map(|session| &mut session.state)
    }

    pub fn close(&mut self) {
        self.active_picker = None;
    }

    /// Start a picker for saved chat sessions.
    pub fn open_saved_session_picker(
        &mut self,
        sessions: Vec<crate::core::session_store::SessionSummary>,
        items: Vec<PickerItem>,
    ) {
        let selected = 0;
        let picker_state = PickerState::new("Load Session", items.clone(), selected);
        let active_picker = ActivePicker {
            state: picker_state,
            data: PickerData::SavedSession(SavedSessionPickerState {
                sessions,
                selected_index: selected,
                search_filter: String::new(),
                all_items: items,
            }),
        };
        self.start_active_picker(active_picker, None);
    }

    fn start_active_picker(
        &mut self,
        mut active_picker: ActivePicker,
        preferred_selection: Option<String>,
    ) {
        let mode = active_picker.mode();
        active_picker.state.sort_mode = active_picker.default_sort_mode();
        self.active_picker = Some(active_picker);

        self.sort_items();
        self.update_title();

        if let Some(preferred) = preferred_selection {
            if let Some(session) = self.active_mut() {
                if let Some((idx, _)) =
                    session
                        .state
                        .items
                        .iter()
                        .enumerate()
                        .find(|(_, item)| match mode {
                            PickerMode::Theme => item.id.eq_ignore_ascii_case(preferred.as_str()),
                            _ => item.id == preferred,
                        })
                {
                    session.state.selected = idx;
                }
            }
        }
    }

    fn filter_session_items(&mut self, expected_mode: PickerMode, special_ids: &[&str]) {
        let Some(session) = self.active_mut() else {
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

    pub fn sort_items(&mut self) {
        let prefers_alpha = self.prefers_alphabetical();
        if let Some(session) = self.active_mut() {
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
        let Some(session) = self.active_mut() else {
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

    pub fn filter_saved_sessions(&mut self) {
        self.filter_session_items(PickerMode::SavedSession, &[]);
    }

    fn prefers_alphabetical(&self) -> bool {
        self.active()
            .map(|session| session.prefers_alphabetical())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests;
