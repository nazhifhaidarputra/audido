use audido_core::{commands::CoreEvent, queue::LoopMode};
use strum::IntoEnumIterator;

use crate::states::{
    AudioState, BrowserState, EqState, QueueState, SettingsState, normalizer::NormalizerState,
};

/// Application state for the TUI
pub struct AppState {
    /// Audio playback state
    pub audio: AudioState,
    /// Browser state
    pub browser: BrowserState,
    /// Queue state
    pub queue: QueueState,
    /// EQ State
    pub eq: EqState,
    /// Settings State
    pub settings: SettingsState,
    /// Normalizer State
    pub normalizer: NormalizerState,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            audio: AudioState::new(),
            browser: BrowserState::new(),
            queue: QueueState::new(),
            eq: EqState::new(),
            settings: SettingsState::new(),
            normalizer: NormalizerState::new(),
        }
    }

    /// Handle a CoreEvent broadcast from the audio engine
    pub fn handle_event(&mut self, event: CoreEvent) {
        self.audio.error_message = None;

        match event {
            CoreEvent::Playing => {
                self.audio.is_playing = true;
                self.audio.status_message = "Playing".to_string();
            }
            CoreEvent::Paused => {
                self.audio.is_playing = false;
                self.audio.status_message = "Paused".to_string();
            }
            CoreEvent::Stopped => {
                self.audio.is_playing = false;
                self.audio.position = 0.0;
                self.audio.status_message = "Stopped".to_string();
            }
            CoreEvent::Loaded(metadata) => {
                self.audio.duration = metadata.duration;
                self.audio.status_message = format!(
                    "Loaded: {} - {}",
                    metadata.title.as_deref().unwrap_or("Unknown"),
                    metadata.author.as_deref().unwrap_or("Unknown")
                );
                self.audio.metadata = Some(metadata);
            }
            CoreEvent::Position { current, total } => {
                self.audio.position = current;
                self.audio.duration = total;
            }
            CoreEvent::Error(msg) => {
                self.audio.error_message = Some(msg.clone());
                self.audio.status_message = format!("Error: {}", msg);
            }
            CoreEvent::Shutdown => {
                self.audio.status_message = "Engine shutdown".to_string();
            }
            CoreEvent::QueueUpdated(queue_items) => {
                self.queue.queue = queue_items;
                if !self.queue.queue.is_empty() && self.queue.queue_state.selected().is_none() {
                    self.queue.queue_state.select(Some(0));
                }
            }
            CoreEvent::LoopModeChanged(mode) => {
                self.queue.loop_mode = mode;
            }
            CoreEvent::TrackChanged { index, metadata } => {
                self.queue.current_queue_index = Some(index);
                self.queue.queue_state.select(Some(index));
                self.audio.metadata = Some(metadata);
                self.audio.status_message =
                    format!("Track {}/{}", index + 1, self.queue.queue.len());
            }
            CoreEvent::DeviceInvalidated => {
                self.audio.error_message = Some("Audio device disconnected. Attempting recovery...".to_string());
                self.audio.status_message = "Reconnecting audio...".to_string();
            },
        }
    }

    // ==============================================
    // Queue Navigation Methods
    // ==============================================

    pub fn queue_next(&mut self) {
        if self.queue.queue.is_empty() {
            return;
        }
        let i = match self.queue.queue_state.selected() {
            Some(i) => {
                if i >= self.queue.queue.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.queue.queue_state.select(Some(i));
    }

    pub fn queue_prev(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        let i = match self.queue.queue_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.queue.queue.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.queue.queue_state.select(Some(i));
    }

    /// Get currently selected queue index
    pub fn queue_selected(&self) -> Option<usize> {
        self.queue.queue_state.selected()
    }

    // ==============================================
    // Loop Mode Methods
    // ==============================================

    /// Cycle to the next loop mode
    pub fn next_loop_mode(&self) -> LoopMode {
        // use strum enum iter
        let mut modes = LoopMode::iter();
        for mode in modes.by_ref() {
            if mode == self.queue.loop_mode {
                break;
            }
        }
        // Return the next mode in the sequence, or the first if at the end
        modes.next().unwrap_or(LoopMode::Off)
    }
}
