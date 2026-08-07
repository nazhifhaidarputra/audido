use audido_core::queue::{LoopMode, QueueItem};
use ratatui::widgets::ListState;

/// Queue-related state (track list, selection, loop mode)
#[derive(Debug, Clone)]
pub struct QueueState {
    pub queue: Vec<QueueItem>,
    pub current_queue_index: Option<usize>,
    pub loop_mode: LoopMode,
    pub queue_state: ListState,
}

impl QueueState {
    pub fn new() -> Self {
        Self {
            queue: Vec::new(),
            current_queue_index: None,
            loop_mode: LoopMode::Off,
            queue_state: ListState::default(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}
