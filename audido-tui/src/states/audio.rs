use audido_core::metadata::AudioMetadata;

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
