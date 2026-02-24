// Reserved for future styling options for picker lists
// use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, PartialEq)]
pub struct PickerItem {
    pub id: String,
    pub label: String,
    pub metadata: Option<String>, // For creation date, theme type, etc.
    pub inspect_metadata: Option<String>, // For full-screen inspect view
    pub sort_key: Option<String>, // For sorting by date or name
}

#[derive(Debug, Clone, PartialEq)]
pub enum SortMode {
    Date,
    Name,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PickerState {
    pub title: String,
    pub items: Vec<PickerItem>,
    pub selected: usize,
    pub sort_mode: SortMode,
}

impl PickerState {
    pub fn new<T: Into<String>>(title: T, items: Vec<PickerItem>, selected: usize) -> Self {
        Self {
            title: title.into(),
            items,
            selected,
            sort_mode: SortMode::Date, // Default to date sorting
        }
    }

    pub fn selected_id(&self) -> Option<&str> {
        self.items.get(self.selected).map(|i| i.id.as_str())
    }

    pub fn get_selected_item(&self) -> Option<&PickerItem> {
        self.items.get(self.selected)
    }

    pub fn move_up(&mut self) {
        if !self.items.is_empty() {
            if self.selected == 0 {
                self.selected = self.items.len() - 1;
            } else {
                self.selected -= 1;
            }
        }
    }

    pub fn move_down(&mut self) {
        if !self.items.is_empty() {
            self.selected = (self.selected + 1) % self.items.len();
        }
    }

    pub fn move_page_up(&mut self, page_lines: usize) {
        if !self.items.is_empty() {
            self.selected = self.selected.saturating_sub(page_lines);
        }
    }

    pub fn move_page_down(&mut self, page_lines: usize) {
        if !self.items.is_empty() {
            let max_index = self.items.len() - 1;
            self.selected = self.selected.saturating_add(page_lines).min(max_index);
        }
    }

    pub fn move_to_start(&mut self) {
        if !self.items.is_empty() {
            self.selected = 0;
        }
    }

    pub fn move_to_end(&mut self) {
        if !self.items.is_empty() {
            self.selected = self.items.len() - 1;
        }
    }

    pub fn cycle_sort_mode(&mut self) {
        self.sort_mode = match self.sort_mode {
            SortMode::Date => SortMode::Name,
            SortMode::Name => SortMode::Date,
        };
    }

    pub fn get_selected_metadata(&self) -> Option<&str> {
        self.items.get(self.selected)?.metadata.as_deref()
    }

    pub fn get_selected_inspect_metadata(&self) -> Option<&str> {
        let item = self.items.get(self.selected)?;
        item.inspect_metadata
            .as_deref()
            .or(item.metadata.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::{PickerItem, PickerState, SortMode};

    fn test_items(count: usize) -> Vec<PickerItem> {
        (0..count)
            .map(|idx| PickerItem {
                id: format!("{idx}"),
                label: format!("Item {idx}"),
                metadata: None,
                inspect_metadata: None,
                sort_key: None,
            })
            .collect()
    }

    #[test]
    fn move_page_up_saturates_at_start() {
        let mut state = PickerState {
            title: "Pick".to_string(),
            items: test_items(30),
            selected: 4,
            sort_mode: SortMode::Name,
        };

        state.move_page_up(10);

        assert_eq!(state.selected, 0);
    }

    #[test]
    fn move_page_down_saturates_at_end() {
        let mut state = PickerState {
            title: "Pick".to_string(),
            items: test_items(30),
            selected: 24,
            sort_mode: SortMode::Name,
        };

        state.move_page_down(10);

        assert_eq!(state.selected, 29);
    }
}
