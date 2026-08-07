#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SettingsOption {
    Equalizer,
    Normalize,
}

impl SettingsOption {
    pub fn label(&self) -> &str {
        match self {
            SettingsOption::Equalizer => "Equalizer",
            SettingsOption::Normalize => "Normalize Audio",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SettingsState {
    pub items: Vec<SettingsOption>,
    pub selected_index: usize,
    /// Is the choice dialog currently open?
    pub is_dialog_open: bool,
    /// Selection index inside the dialog (e.g., 0=On, 1=Off)
    pub dialog_selection_index: usize,
}

impl SettingsState {
    pub fn new() -> Self {
        Self {
            items: vec![SettingsOption::Equalizer, SettingsOption::Normalize],
            selected_index: 0,
            is_dialog_open: false,
            dialog_selection_index: 0,
        }
    }

    pub fn next_item(&mut self) {
        if !self.items.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.items.len();
        }
    }

    pub fn prev_item(&mut self) {
        if !self.items.is_empty() {
            self.selected_index = (self.selected_index + self.items.len() - 1) % self.items.len();
        }
    }

    #[allow(dead_code)]
    pub fn open_dialog(&mut self) {
        self.is_dialog_open = true;
        self.dialog_selection_index = 0;
    }

    #[allow(dead_code)]
    pub fn close_dialog(&mut self) {
        self.is_dialog_open = false;
    }

    #[allow(dead_code)]
    pub fn next_dialog(&mut self, choice_count: usize) {
        if choice_count > 0 {
            self.dialog_selection_index = (self.dialog_selection_index + 1) % choice_count;
        }
    }

    #[allow(dead_code)]
    pub fn prev_dialog(&mut self, choice_count: usize) {
        if choice_count > 0 {
            self.dialog_selection_index =
                (self.dialog_selection_index + choice_count - 1) % choice_count;
        }
    }
}
