use std::fmt::{self, Debug};

use audido_core::metadata::AudioMetadata;
use ratatui_image::protocol::Protocol;

/// Audio-related state (playback status, position, volume, metadata, messages)
#[derive(Debug, Clone)]
pub struct AudioState {
    /// Whether audio is currently playing
    pub is_playing: bool,
    /// Current playback position in seconds
    pub position: f32,
    /// Total duration in seconds
    pub duration: f32,
    /// Current volume (0.0 to 1.0)
    pub volume: f32,
    /// Currently loaded audio metadata
    pub metadata: Option<AudioMetadata>,
    /// Status message to display
    pub status_message: String,
    /// Error message if any
    pub error_message: Option<String>,
    pub cover_image_protocol: ImageProtocolWrapper
}

#[derive(Clone)]
pub struct ImageProtocolWrapper(Option<Protocol>);

impl Debug for ImageProtocolWrapper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[Image Protocol]")
    }
}

impl ImageProtocolWrapper {
    pub fn get(&self) -> Option<&Protocol> {
        self.0.as_ref()
    }

    #[allow(unused)]
    pub fn get_mut(&mut self) -> Option<&mut Protocol> {
        self.0.as_mut()
    }

    pub fn new(protocol: Option<Protocol>) -> Self {
        Self(protocol)
    }
}

impl Default for ImageProtocolWrapper {
    fn default() -> Self {
        Self(None)
    }
}

impl AudioState {
    pub fn new() -> Self {
        Self {
            is_playing: false,
            position: 0.0,
            duration: 0.0,
            volume: 1.0,
            metadata: None,
            status_message: "No audio loaded. Pass a file path as argument.".to_string(),
            error_message: None,
            cover_image_protocol: ImageProtocolWrapper::default(),
        }
    }

    /// Get the progress percentage (0.0 to 1.0)
    pub fn progress(&self) -> f32 {
        if self.duration > 0.0 {
            (self.position / self.duration).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// Format time as MM:SS
    pub fn format_time(seconds: f32) -> String {
        let mins = (seconds / 60.0).floor() as u32;
        let secs = (seconds % 60.0).floor() as u32;
        format!("{:02}:{:02}", mins, secs)
    }
}
