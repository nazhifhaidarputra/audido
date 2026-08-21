use std::{marker::PhantomData, path::PathBuf};

use audido_core::browser::{self, FileEntry};
use ratatui::widgets::ListState;

use crate::state::StatefulList;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum BrowserSource {
    #[default]
    LocalFiles,
    YouTube,
    Playlists,
}

/// Type tags for PhantomData
#[derive(Debug, Clone)] pub struct FileItemTag;
#[derive(Debug, Clone)] pub struct SourceItemTag;

/// Dialog shown when selecting a file in browser
#[derive(Debug, Clone, Default)]
pub enum BrowserFileDialog {
    #[default]
    None,
    /// Dialog open with path and selected option (0=Play Now, 1=Add to Queue)
    Open { path: PathBuf, selected: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ActiveBrowserPane {
    Sources,
    #[default]
    Files,
}

/// Browser state for file navigation
#[derive(Debug, Clone)]
pub struct BrowserState {
    pub current_dir: PathBuf,
    pub files: StatefulList<FileEntry, FileItemTag>,
    pub sources: StatefulList<BrowserSource, SourceItemTag>,
    pub dialog: BrowserFileDialog,
}

impl BrowserState {
    pub fn new() -> Self {
        let mut current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        if let Some(cache_path) = get_cache_path() {
            if let Ok(saved_path_str) = std::fs::read_to_string(&cache_path) {
                let saved_path = PathBuf::from(saved_path_str.trim());
                if saved_path.exists() && saved_path.is_dir() {
                    current_dir = saved_path;
                }
            }
        }
        
        let file_items = browser::get_directory_content(&current_dir).unwrap_or_default();
        let source_items = vec![
            BrowserSource::LocalFiles,
            BrowserSource::YouTube,
            BrowserSource::Playlists,
        ];

        Self {
            current_dir,
            files: StatefulList::new(file_items),
            sources: StatefulList::new(source_items),
            dialog: BrowserFileDialog::None,
        }
    }


pub fn next(&mut self, active_pane: ActiveBrowserPane) {
        match active_pane {
            ActiveBrowserPane::Sources => self.sources.next(),
            ActiveBrowserPane::Files => self.files.next(),
        }
    }

    pub fn prev(&mut self, active_pane: ActiveBrowserPane) {
        match active_pane {
            ActiveBrowserPane::Sources => self.sources.prev(),
            ActiveBrowserPane::Files => self.files.prev(),
        }
    }

    /// Enter selected directory or return PathBuf if it's a file
    pub fn enter(&mut self, active_pane: ActiveBrowserPane) -> Option<PathBuf> {
        match active_pane {
            ActiveBrowserPane::Sources => None,
            ActiveBrowserPane::Files => {
                let item = self.files.selected_item()?;
                if item.is_dir {
                    let new_path = item.path.clone();
                    if let Ok(new_items) = browser::get_directory_content(&new_path) {
                        self.current_dir = new_path.clone();
                        self.files = StatefulList::new(new_items);

                        if let Some(cache_path) = get_cache_path() {
                            let _ = std::fs::write(cache_path, new_path.to_string_lossy().as_ref());
                        }
                    }
                    None
                } else {
                    Some(item.path.clone())
                }
            }
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

    /// Convenience getter to maintain API surface for the currently selected source
    pub fn current_source(&self) -> &BrowserSource {
        self.sources.selected_item().unwrap_or(&BrowserSource::LocalFiles)
    }
}

fn get_cache_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("com", "Audido", "AudidoTui").map(|proj_dirs| {
            let cache_dir = proj_dirs.cache_dir();
            // Ensure the cache directory exists before returning the file path
            let _ = std::fs::create_dir_all(cache_dir);
            cache_dir.join("last_dir.txt")
        })
    }