use std::fmt::{self, Debug};

use audido_core::metadata::AudioMetadata;
use ratatui_image::protocol::Protocol;
use ringbuf::{
    HeapCons,
    traits::{Consumer, Observer},
};

/// Audio-related state (playback status, position, volume, metadata, messages)
#[derive(Debug, Clone)]
pub struct AudioState {
    /// Whether audio is currently playing
    pub is_playing: bool,
    /// Current playback position in seconds
    pub position: f32,
    /// Total duration in seconds
    pub duration: f32,
    /// Duration in seconds currently decoded and available for seeking.
    pub buffered: f32,
    /// Current volume (0.0 to 1.0)
    pub volume: f32,
    /// Currently loaded audio metadata
    pub metadata: Option<AudioMetadata>,
    /// Status message to display
    pub status_message: String,
    /// Error message if any
    pub error_message: Option<String>,
    pub cover_image_protocol: ImageProtocolWrapper,
    pub visualizer_config: AudioVisualizerConfig,
}

/// Configuration and live data for the audio spectrum visualizer.
/// The SPSC consumer and display buffer are private; the UI reads them via `bins()`.
pub struct AudioVisualizerConfig {
    /// Number of frequency bins to display (configurable by the user).
    pub bin_size: usize,
    /// Smoothed display buffer: one f32 (dB) per bin, updated each TUI frame.
    spectrum_bins: Vec<f32>,
    /// Consumer end of the spectrum SPSC ring buffer fed by the DSP thread.
    consumer: Option<HeapCons<f32>>,
}

impl Debug for AudioVisualizerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AudioVisualizerConfig")
            .field("bin_size", &self.bin_size)
            .field("spectrum_bins_len", &self.spectrum_bins.len())
            .field("has_consumer", &self.consumer.is_some())
            .finish()
    }
}

impl Clone for AudioVisualizerConfig {
    fn clone(&self) -> Self {
        // HeapCons is not Clone — the clone gets no consumer (display-only copy).
        Self {
            bin_size: self.bin_size,
            spectrum_bins: self.spectrum_bins.clone(),
            consumer: None,
        }
    }
}

impl Default for AudioVisualizerConfig {
    fn default() -> Self {
        Self {
            bin_size: audido_core::modules::core::DEFAULT_SPECTRUM_BIN_SIZE,
            spectrum_bins: vec![-140.0; audido_core::modules::core::DEFAULT_SPECTRUM_BIN_SIZE],
            consumer: None,
        }
    }
}

impl AudioVisualizerConfig {
    /// Attach the SPSC consumer received from `CoreHandle::take_spectrum_consumer`.
    pub fn attach_consumer(&mut self, consumer: HeapCons<f32>) {
        self.consumer = Some(consumer);
    }

    /// Drain the ring buffer and update the display buffer with the latest complete frame.
    /// Call this once per TUI frame before rendering.
    pub fn update(&mut self) {
        let Some(ref mut cons) = self.consumer else {
            return;
        };
        let bin_size = self.bin_size;

        // Collect all available samples from the ring into a staging buffer.
        let available = cons.occupied_len();
        if available == 0 {
            return;
        }

        let mut staging = vec![0.0f32; available];
        cons.pop_slice(&mut staging);

        // Keep only the last complete frame of `bin_size` bins.
        // This automatically drops stale frames if we lagged behind the DSP thread.
        if staging.len() >= bin_size {
            let last_frame_start = staging.len() - (staging.len() % bin_size).max(bin_size);
            let last_frame = if last_frame_start + bin_size <= staging.len() {
                &staging[last_frame_start..last_frame_start + bin_size]
            } else {
                // Partial frame at the very end — use what we have padded to silence.
                &staging[staging.len() - bin_size.min(staging.len())..]
            };
            let copy_len = last_frame.len().min(bin_size);
            self.spectrum_bins[..copy_len].copy_from_slice(&last_frame[..copy_len]);
        }
    }

    /// Read-only view of the current display buffer (one dB value per bin).
    pub fn bins(&self) -> &[f32] {
        &self.spectrum_bins
    }

    /// Write reference slices view of the current display buffer (one dB value per bin).
    pub fn bins_mut(&mut self) -> &mut [f32] {
        &mut self.spectrum_bins
    }
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
            buffered: 0.0,
            volume: 1.0,
            metadata: None,
            status_message: "No audio loaded. Pass a file path as argument.".to_string(),
            error_message: None,
            cover_image_protocol: ImageProtocolWrapper::default(),
            visualizer_config: AudioVisualizerConfig::default(),
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

    /// Get the loaded-buffer percentage (0.0 to 1.0).
    pub fn buffered_progress(&self) -> f32 {
        if self.duration > 0.0 {
            (self.buffered / self.duration).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    pub fn is_youtube_stream(&self) -> bool {
        self.metadata
            .as_ref()
            .is_some_and(|metadata| metadata.format == "youtube-stream")
    }

    /// Format time as MM:SS
    pub fn format_time(seconds: f32) -> String {
        let mins = (seconds / 60.0).floor() as u32;
        let secs = (seconds % 60.0).floor() as u32;
        format!("{:02}:{:02}", mins, secs)
    }
}
