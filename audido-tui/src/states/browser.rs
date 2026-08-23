use std::{any::Any, fmt, path::PathBuf, sync::Arc};

use audido_core::{
    browser::{self, FileEntry},
    modules::{
        core::CoreContext,
        youtube::ytdlp::{PlaylistEntry, YoutubeSearchError},
    },
    source::AudioSource,
};

use crate::state::StatefulList;

const DEFAULT_YOUTUBE_PAGE_SIZE: usize = 10;
const MAX_YOUTUBE_SEARCH_RESULTS: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum BrowserSource {
    #[default]
    LocalFiles,
    YouTube,
    Playlists,
}

/// Type tags keep each `StatefulList` use distinct without constraining its item type.
#[derive(Debug, Clone)]
pub struct BrowserEntryTag;
#[derive(Debug, Clone)]
pub struct SourceItemTag;

/// Type-erased value stored by the browser's generic entry list.
pub trait BrowserEntryValue: Any + fmt::Debug + Send + Sync {
    fn as_any(&self) -> &dyn Any;
}

impl<T> BrowserEntryValue for T
where
    T: Any + fmt::Debug + Send + Sync,
{
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A clonable browser row capable of holding any provider-specific entry type.
#[derive(Clone)]
pub struct BrowserEntry(Arc<dyn BrowserEntryValue>);

impl BrowserEntry {
    pub fn new<T>(value: T) -> Self
    where
        T: BrowserEntryValue,
    {
        Self(Arc::new(value))
    }

    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.0.as_ref().as_any().downcast_ref()
    }
}

impl fmt::Debug for BrowserEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("BrowserEntry")
            .field(&self.0)
            .finish()
    }
}

/// Dialog shown when selecting a local or remote browser entry.
#[derive(Debug, Clone, Default)]
pub enum BrowserFileDialog {
    #[default]
    None,
    /// Dialog open with source and selected option (0=Play Now, 1=Add to Queue).
    Open {
        source: AudioSource,
        title: String,
        selected: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum YoutubeBrowserFocus {
    #[default]
    Search,
    Entries,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ActiveBrowserPane {
    Sources,
    #[default]
    Entries,
}

#[derive(Debug, Clone)]
enum YoutubeSearchOutcome {
    Entries(Vec<PlaylistEntry>),
    NoResults,
    Error(String),
}

#[derive(Debug, Clone)]
struct YoutubeSearchResponse {
    request_id: u64,
    page_idx: usize,
    outcome: YoutubeSearchOutcome,
}

/// Browser state shared by local-file and remote providers.
#[derive(Debug, Clone)]
pub struct BrowserState {
    pub current_dir: PathBuf,
    pub entries: StatefulList<BrowserEntry, BrowserEntryTag>,
    pub sources: StatefulList<BrowserSource, SourceItemTag>,
    pub dialog: BrowserFileDialog,

    pub search_query: String,
    pub submitted_query: Option<String>,
    pub page_idx: usize,
    pub page_size: usize,
    pub is_searching: bool,
    pub search_error: Option<String>,
    pub has_next_page: bool,
    pub youtube_focus: YoutubeBrowserFocus,

    local_entries: Vec<FileEntry>,
    youtube_entries: Vec<PlaylistEntry>,
    latest_search_request: u64,
    search_tx: crossbeam_channel::Sender<YoutubeSearchResponse>,
    search_rx: crossbeam_channel::Receiver<YoutubeSearchResponse>,
}

impl BrowserState {
    pub fn new() -> Self {
        let mut current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        if let Some(cache_path) = get_cache_path()
            && let Ok(saved_path_str) = std::fs::read_to_string(&cache_path)
        {
            let saved_path = PathBuf::from(saved_path_str.trim());
            if saved_path.exists() && saved_path.is_dir() {
                current_dir = saved_path;
            }
        }

        let local_entries = browser::get_directory_content(&current_dir).unwrap_or_default();
        let entries = local_entries
            .iter()
            .cloned()
            .map(BrowserEntry::new)
            .collect();
        let source_items = vec![
            BrowserSource::LocalFiles,
            BrowserSource::YouTube,
            BrowserSource::Playlists,
        ];
        let (search_tx, search_rx) = crossbeam_channel::unbounded();

        Self {
            current_dir,
            entries: StatefulList::new(entries),
            sources: StatefulList::new(source_items),
            dialog: BrowserFileDialog::None,
            search_query: String::new(),
            submitted_query: None,
            page_idx: 0,
            page_size: DEFAULT_YOUTUBE_PAGE_SIZE,
            is_searching: false,
            search_error: None,
            has_next_page: false,
            youtube_focus: YoutubeBrowserFocus::Search,
            local_entries,
            youtube_entries: Vec::new(),
            latest_search_request: 0,
            search_tx,
            search_rx,
        }
    }

    pub fn next(&mut self, active_pane: ActiveBrowserPane) {
        match active_pane {
            ActiveBrowserPane::Sources => {
                self.sources.next();
                self.activate_current_source();
            }
            ActiveBrowserPane::Entries => self.entries.next(),
        }
    }

    pub fn prev(&mut self, active_pane: ActiveBrowserPane) {
        match active_pane {
            ActiveBrowserPane::Sources => {
                self.sources.prev();
                self.activate_current_source();
            }
            ActiveBrowserPane::Entries => self.entries.prev(),
        }
    }

    /// Enter the selected local directory or return its file path.
    pub fn enter(&mut self, active_pane: ActiveBrowserPane) -> Option<PathBuf> {
        if active_pane != ActiveBrowserPane::Entries {
            return None;
        }

        let item = self
            .entries
            .selected_item()?
            .downcast_ref::<FileEntry>()?
            .clone();

        if item.is_dir {
            let new_path = item.path;
            if let Ok(new_items) = browser::get_directory_content(&new_path) {
                self.current_dir = new_path.clone();
                self.local_entries = new_items;
                self.activate_current_source();

                if let Some(cache_path) = get_cache_path() {
                    let _ = std::fs::write(cache_path, new_path.to_string_lossy().as_ref());
                }
            }
            None
        } else {
            Some(item.path)
        }
    }

    /// Submit the current query and reset pagination to the first page.
    pub fn search_youtube(&mut self, ctx: Arc<CoreContext>) {
        let query = self.search_query.trim().to_owned();
        if query.is_empty() {
            self.search_error = Some("Enter a search query".to_string());
            return;
        }

        self.submitted_query = Some(query.clone());
        self.page_idx = 0;
        self.has_next_page = false;
        self.youtube_focus = YoutubeBrowserFocus::Search;
        self.youtube_entries.clear();
        self.activate_current_source();
        self.request_youtube_page(ctx, query, 0);
    }

    pub fn next_youtube_page(&mut self, ctx: Arc<CoreContext>) {
        if self.is_searching || !self.has_next_page {
            return;
        }
        let Some(query) = self.submitted_query.clone() else {
            return;
        };
        self.request_youtube_page(ctx, query, self.page_idx.saturating_add(1));
    }

    pub fn previous_youtube_page(&mut self, ctx: Arc<CoreContext>) {
        if self.is_searching || self.page_idx == 0 {
            return;
        }
        let Some(query) = self.submitted_query.clone() else {
            return;
        };
        self.request_youtube_page(ctx, query, self.page_idx - 1);
    }

    /// Apply completed async search requests without blocking the TUI thread.
    pub fn poll_search_results(&mut self) {
        while let Ok(response) = self.search_rx.try_recv() {
            if response.request_id != self.latest_search_request {
                continue;
            }

            self.is_searching = false;
            match response.outcome {
                YoutubeSearchOutcome::Entries(entries) => {
                    self.page_idx = response.page_idx;
                    self.has_next_page = entries.len() == self.page_size
                        && response
                            .page_idx
                            .saturating_add(1)
                            .saturating_mul(self.page_size)
                            < MAX_YOUTUBE_SEARCH_RESULTS;
                    self.search_error = None;
                    self.youtube_entries = entries;
                    if !self.youtube_entries.is_empty() {
                        self.youtube_focus = YoutubeBrowserFocus::Entries;
                    }
                    if self.current_source() == &BrowserSource::YouTube {
                        self.activate_current_source();
                    }
                }
                YoutubeSearchOutcome::NoResults => {
                    self.has_next_page = false;
                    if response.page_idx == 0 {
                        self.youtube_entries.clear();
                        self.search_error = Some("No YouTube results found".to_string());
                        self.youtube_focus = YoutubeBrowserFocus::Search;
                        if self.current_source() == &BrowserSource::YouTube {
                            self.activate_current_source();
                        }
                    }
                }
                YoutubeSearchOutcome::Error(error) => {
                    self.search_error = Some(error);
                }
            }
        }
    }

    fn request_youtube_page(&mut self, ctx: Arc<CoreContext>, query: String, page_idx: usize) {
        self.latest_search_request = self.latest_search_request.wrapping_add(1);
        let request_id = self.latest_search_request;
        let page_size = self.page_size.clamp(1, MAX_YOUTUBE_SEARCH_RESULTS);
        let tx = self.search_tx.clone();
        let tokio_handle = ctx.tokio_handle.clone();

        self.is_searching = true;
        self.search_error = None;
        tokio_handle.spawn(async move {
            let outcome = match ctx
                .yt
                .search_youtube_by_query(&query, page_size, page_idx)
                .await
            {
                Ok(entries) => YoutubeSearchOutcome::Entries(entries),
                Err(YoutubeSearchError::NoResults { .. }) => YoutubeSearchOutcome::NoResults,
                Err(error) => YoutubeSearchOutcome::Error(error.to_string()),
            };
            let _ = tx.send(YoutubeSearchResponse {
                request_id,
                page_idx,
                outcome,
            });
        });
    }

    fn activate_current_source(&mut self) {
        let entries = match self.current_source() {
            BrowserSource::LocalFiles => self
                .local_entries
                .iter()
                .cloned()
                .map(BrowserEntry::new)
                .collect(),
            BrowserSource::YouTube => self
                .youtube_entries
                .iter()
                .cloned()
                .map(BrowserEntry::new)
                .collect(),
            BrowserSource::Playlists => Vec::new(),
        };
        self.entries = StatefulList::new(entries);
    }

    pub fn replace_local_entries(&mut self, current_dir: PathBuf, entries: Vec<FileEntry>) {
        self.current_dir = current_dir;
        self.local_entries = entries;
        if self.current_source() == &BrowserSource::LocalFiles {
            self.activate_current_source();
        }
    }

    pub fn open_dialog(&mut self, path: PathBuf) {
        let title = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Unknown File".to_string());
        self.dialog = BrowserFileDialog::Open {
            source: AudioSource::Local { path },
            title,
            selected: 0,
        };
    }

    pub fn open_selected_youtube_dialog(&mut self) -> bool {
        let Some(entry) = self
            .entries
            .selected_item()
            .and_then(|entry| entry.downcast_ref::<PlaylistEntry>())
            .cloned()
        else {
            return false;
        };

        self.dialog = BrowserFileDialog::Open {
            source: AudioSource::Youtube { url: entry.url },
            title: entry.title,
            selected: 0,
        };
        true
    }

    pub fn dialog_toggle(&mut self) {
        if let BrowserFileDialog::Open { selected, .. } = &mut self.dialog {
            *selected = if *selected == 0 { 1 } else { 0 };
        }
    }

    pub fn close_dialog(&mut self) {
        self.dialog = BrowserFileDialog::None;
    }

    pub fn is_dialog_open(&self) -> bool {
        !matches!(self.dialog, BrowserFileDialog::None)
    }

    pub fn current_source(&self) -> &BrowserSource {
        self.sources
            .selected_item()
            .unwrap_or(&BrowserSource::LocalFiles)
    }
}

fn get_cache_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "Audido", "AudidoTui").map(|proj_dirs| {
        let cache_dir = proj_dirs.cache_dir();
        let _ = std::fs::create_dir_all(cache_dir);
        cache_dir.join("last_dir.txt")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn youtube_entry(id: &str) -> PlaylistEntry {
        PlaylistEntry {
            id: id.to_string(),
            title: format!("Video {id}"),
            url: format!("https://youtube.test/watch?v={id}"),
            index: None,
            duration: Some(123.0),
            thumbnail: None,
            uploader: Some("Test channel".to_string()),
            channel_id: None,
            availability: None,
        }
    }

    #[test]
    fn applies_latest_youtube_page_to_generic_entries() {
        let mut state = BrowserState::new();
        state.sources.state.select(Some(1));
        state.activate_current_source();
        state.page_size = 2;
        state.latest_search_request = 7;
        state.is_searching = true;

        state
            .search_tx
            .send(YoutubeSearchResponse {
                request_id: 7,
                page_idx: 1,
                outcome: YoutubeSearchOutcome::Entries(vec![
                    youtube_entry("one"),
                    youtube_entry("two"),
                ]),
            })
            .unwrap();
        state.poll_search_results();

        assert!(!state.is_searching);
        assert_eq!(state.page_idx, 1);
        assert!(state.has_next_page);
        assert_eq!(state.entries.items.len(), 2);
        assert_eq!(
            state.entries.items[0]
                .downcast_ref::<PlaylistEntry>()
                .map(|entry| entry.id.as_str()),
            Some("one")
        );
        assert_eq!(state.youtube_focus, YoutubeBrowserFocus::Entries);
        assert!(state.open_selected_youtube_dialog());
        assert!(matches!(
            &state.dialog,
            BrowserFileDialog::Open {
                source: AudioSource::Youtube { url },
                title,
                selected: 0,
            } if url.ends_with("one") && title == "Video one"
        ));
    }

    #[test]
    fn ignores_stale_search_responses_and_stops_at_result_cap() {
        let mut state = BrowserState::new();
        state.sources.state.select(Some(1));
        state.activate_current_source();
        state.page_size = 10;
        state.latest_search_request = 2;
        state.is_searching = true;

        state
            .search_tx
            .send(YoutubeSearchResponse {
                request_id: 1,
                page_idx: 0,
                outcome: YoutubeSearchOutcome::Error("stale error".to_string()),
            })
            .unwrap();
        state
            .search_tx
            .send(YoutubeSearchResponse {
                request_id: 2,
                page_idx: 4,
                outcome: YoutubeSearchOutcome::Entries(
                    (0..10)
                        .map(|index| youtube_entry(&index.to_string()))
                        .collect(),
                ),
            })
            .unwrap();
        state.poll_search_results();

        assert_eq!(state.page_idx, 4);
        assert!(!state.has_next_page);
        assert!(state.search_error.is_none());
    }
}
