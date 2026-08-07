use std::path::PathBuf;

use audido_core::browser::{self, FileEntry};
use ratatui::widgets::ListState;

/// Dialog shown when selecting a file in browser
#[derive(Debug, Clone, Default)]
pub enum BrowserFileDialog {
    #[default]
    None,
    /// Dialog open with path and selected option (0=Play Now, 1=Add to Queue)
    Open { path: PathBuf, selected: usize },
}

/// Browser state for file navigation
#[derive(Debug, Clone)]
pub struct BrowserState {
    pub current_dir: PathBuf,
    pub items: Vec<FileEntry>,
    pub list_state: ListState,
    pub dialog: BrowserFileDialog,
}

impl BrowserState {
    pub fn new() -> Self {
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let items = browser::get_directory_content(&current_dir).unwrap_or_default();
        let mut list_state = ListState::default();
        if !items.is_empty() {
            list_state.select(Some(0));
        }

        Self {
            current_dir,
            items,
            list_state,
            dialog: BrowserFileDialog::None,
        }
    }

    pub fn next(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) => {
                if i >= self.items.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    pub fn prev(&mut self) {
        let i = match self.list_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.items.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.list_state.select(Some(i));
    }

    /// Enter selected directory or return PathBuf if it's a file
    pub fn enter(&mut self) -> Option<PathBuf> {
        let i = self.list_state.selected()?;
        let item = &self.items.get(i)?;
        if item.is_dir {
            let new_path = item.path.clone();
            if let Ok(new_items) = browser::get_directory_content(&new_path) {
                self.current_dir = new_path;
                self.items = new_items;
                self.list_state.select(Some(0));
            }
            None
        } else {
            Some(item.path.clone())
        }
    }

    /// Open the browser file dialog for a given path
    pub fn open_dialog(&mut self, path: PathBuf) {
        self.dialog = BrowserFileDialog::Open { path, selected: 0 };
    }

    /// Navigate dialog selection
    pub fn dialog_toggle(&mut self) {
        if let BrowserFileDialog::Open { selected, .. } = &mut self.dialog {
            *selected = if *selected == 0 { 1 } else { 0 };
        }
    }

    /// Close the dialog
    pub fn close_dialog(&mut self) {
        self.dialog = BrowserFileDialog::None;
    }

    /// Check if dialog is open
    pub fn is_dialog_open(&self) -> bool {
        !matches!(self.dialog, BrowserFileDialog::None)
    }
}
