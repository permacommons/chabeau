// Reserved for future styling options for picker lists
// use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone)]
pub struct PickerItem {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct PickerState {
    pub title: String,
    pub items: Vec<PickerItem>,
    pub selected: usize,
}

impl PickerState {
    pub fn new<T: Into<String>>(title: T, items: Vec<PickerItem>, selected: usize) -> Self {
        Self {
            title: title.into(),
            items,
            selected,
        }
    }

    pub fn selected_id(&self) -> Option<&str> {
        self.items.get(self.selected).map(|i| i.id.as_str())
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
}
