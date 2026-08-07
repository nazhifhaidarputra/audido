use std::path::PathBuf;

use strum::EnumIter;

use crate::metadata::AudioMetadata;

/// Loop/repeat mode for queue playback
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, EnumIter, strum::Display)]
pub enum LoopMode {
    #[default]
    #[strum(serialize = "➡️ Off")]
    Off,
    #[strum(serialize = "🔂 One")]
    RepeatOne,
    #[strum(serialize = "🔁 All")]
    LoopAll,
    #[strum(serialize = "🔀 Shuffle")]
    Shuffle,
}

/// A single item in the playback queue
#[derive(Debug, Clone)]
pub struct QueueItem {
    pub id: usize,
    pub path: PathBuf,
    pub metadata: Option<AudioMetadata>,
}

/// The playback queue state
#[derive(Debug, Clone, Default)]
pub struct PlaybackQueue {
    pub items: Vec<QueueItem>,
    pub current_index: Option<usize>,
    pub loop_mode: LoopMode,
    pub shuffle_order: Vec<usize>,
    next_id: usize,
}

impl PlaybackQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add paths to queue, returns assigned IDs
    pub fn add(&mut self, paths: Vec<PathBuf>) -> Vec<usize> {
        let mut ids = Vec::with_capacity(paths.len());
        for path in paths {
            let id = self.next_id;
            self.next_id += 1;
            self.items.push(QueueItem {
                id,
                path,
                metadata: None,
            });
            ids.push(id);
        }
        // Regenerate shuffle order when items change
        if self.loop_mode == LoopMode::Shuffle {
            self.reshuffle();
        }
        ids
    }

    /// Remove item by ID, returns true if found and removed
    pub fn remove(&mut self, id: usize) -> bool {
        if let Some(pos) = self.items.iter().position(|item| item.id == id) {
            self.items.remove(pos);
            // Adjust current_index if needed
            if let Some(idx) = self.current_index {
                if pos < idx {
                    self.current_index = Some(idx - 1);
                } else if pos == idx {
                    // Current track removed, stay at same index or clamp
                    if self.items.is_empty() {
                        self.current_index = None;
                    } else if idx >= self.items.len() {
                        self.current_index = Some(self.items.len() - 1);
                    }
                }
            }
            if self.loop_mode == LoopMode::Shuffle {
                self.reshuffle();
            }
            true
        } else {
            false
        }
    }

    /// Clear all items from queue
    pub fn clear(&mut self) {
        self.items.clear();
        self.current_index = None;
        self.shuffle_order.clear();
    }

    /// Get next track index based on loop mode
    pub fn next_index(&self) -> Option<usize> {
        let current = self.current_index?;
        if self.items.is_empty() {
            return None;
        }

        match self.loop_mode {
            LoopMode::Off => {
                if current + 1 < self.items.len() {
                    Some(current + 1)
                } else {
                    None // End of queue
                }
            }
            LoopMode::RepeatOne => Some(current),
            LoopMode::LoopAll => Some((current + 1) % self.items.len()),
            LoopMode::Shuffle => {
                // Find current position in shuffle order and advance
                if let Some(shuffle_pos) = self.shuffle_order.iter().position(|&i| i == current) {
                    let next_shuffle_pos = (shuffle_pos + 1) % self.shuffle_order.len();
                    Some(self.shuffle_order[next_shuffle_pos])
                } else {
                    self.shuffle_order.first().copied()
                }
            }
        }
    }

    /// Get previous track index
    pub fn prev_index(&self) -> Option<usize> {
        let current = self.current_index?;
        if self.items.is_empty() {
            return None;
        }

        match self.loop_mode {
            LoopMode::Off => {
                if current > 0 {
                    Some(current - 1)
                } else {
                    None
                }
            }
            LoopMode::RepeatOne => Some(current),
            LoopMode::LoopAll => {
                if current > 0 {
                    Some(current - 1)
                } else {
                    Some(self.items.len() - 1)
                }
            }
            LoopMode::Shuffle => {
                if let Some(shuffle_pos) = self.shuffle_order.iter().position(|&i| i == current) {
                    let prev_shuffle_pos = if shuffle_pos > 0 {
                        shuffle_pos - 1
                    } else {
                        self.shuffle_order.len() - 1
                    };
                    Some(self.shuffle_order[prev_shuffle_pos])
                } else {
                    self.shuffle_order.last().copied()
                }
            }
        }
    }

    /// Generate new shuffle order using Fisher-Yates
    pub fn reshuffle(&mut self) {
        use rand::seq::SliceRandom;
        let mut order: Vec<usize> = (0..self.items.len()).collect();
        let mut rng = rand::rng();
        order.shuffle(&mut rng);
        self.shuffle_order = order;
    }

    /// Get current track
    pub fn current(&self) -> Option<&QueueItem> {
        self.current_index.and_then(|i| self.items.get(i))
    }

    /// Get item by index
    pub fn get(&self, index: usize) -> Option<&QueueItem> {
        self.items.get(index)
    }

    /// Set metadata for an item by ID
    pub fn set_metadata(&mut self, id: usize, metadata: AudioMetadata) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.metadata = Some(metadata);
        }
    }
}
