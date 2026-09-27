//! Theme picker over built-in and custom themes.

use super::inspect::{theme_metadata, ThemeSource};
use super::{ActivePicker, PickerController, PickerData, PickerMode, ThemePickerState};
use crate::core::app::ui_state::UiState;
use crate::core::config::data::Config;
use crate::ui::builtin_themes::load_builtin_themes;
use crate::ui::picker::{PickerItem, PickerState};

impl PickerController {
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
            let (metadata, inspect_metadata) = theme_metadata(&t, ThemeSource::Builtin, is_default);
            items.push(PickerItem {
                id: t.id.clone(),
                label,
                metadata: Some(metadata),
                inspect_metadata: Some(inspect_metadata),
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
            let spec = crate::ui::builtin_themes::theme_spec_from_custom(ct);
            let (metadata, inspect_metadata) =
                theme_metadata(&spec, ThemeSource::Custom, is_default);
            items.push(PickerItem {
                id: ct.id.clone(),
                label,
                metadata: Some(metadata),
                inspect_metadata: Some(inspect_metadata),
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
        let session = ActivePicker {
            state: picker_state,
            data: PickerData::Theme(Box::new(ThemePickerState {
                search_filter: String::new(),
                all_items: items,
                before_theme: Some(ui.theme.clone()),
                before_theme_id: cfg.theme.clone(),
            })),
        };

        self.start_active_picker(session, active_theme_id);
        Ok(())
    }

    pub fn filter_themes(&mut self) {
        self.filter_session_items(PickerMode::Theme, &[]);
    }
}
