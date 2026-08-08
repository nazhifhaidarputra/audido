use crate::{
    dsp::{eq::{EqPreset, FilterNode}, normalization::NormalizationMode},
    metadata::AudioMetadata,
    queue::{LoopMode, QueueItem},
};

/// Events broadcast from the audio core to the TUI via tokio::sync::broadcast.
/// These replace the old `AudioResponse` channel pattern — the TUI subscribes to these
/// events via `CoreHandle::subscribe()` and reacts accordingly.
#[derive(Debug, Clone)]
pub enum CoreEvent {
    /// Playback has started
    Playing,
    /// Playback has been paused
    Paused,
    /// Playback has been stopped
    Stopped,
    /// Audio file loaded successfully with metadata
    Loaded(AudioMetadata),
    /// Current playback position in seconds and total duration
    Position {
        current: f32,
        total: f32,
    },
    /// Queue contents changed
    QueueUpdated(Vec<QueueItem>),
    /// Loop mode changed
    LoopModeChanged(LoopMode),
    /// Active track changed (index and new metadata)
    TrackChanged {
        index: usize,
        metadata: AudioMetadata,
    },
    DeviceInvalidated,
    /// A non-fatal error occurred in the audio core
    Error(String),
    /// Engine is shutting down
    Shutdown,
}

/// Internal real-time audio commands sent from the engine state to the DSP processing task.
/// These are dispatched through an SPSC or unbounded channel from the control side
/// into the audio processing loop that fills the CPAL ringbuffer.
#[derive(Debug, Clone)]
pub enum RealtimeCommand {
    /// Update a specific EQ filter by index
    UpdateEqFilter(usize, FilterNode),
    /// Replace all EQ filters at once
    SetAllEqFilters(Vec<FilterNode>),
    /// Update EQ master gain (linear scale)
    SetEqMasterGain(f32),
    /// Apply a named EQ preset
    SetEqPreset(EqPreset),
    /// Reset EQ to default parameters
    ResetEq,
    /// Reset a single EQ filter node to preset default
    ResetEqFilterNode(usize),
    /// Enable or disable the equalizer
    SetEqEnabled(bool),
    /// Change the normalizer mode
    SetNormalizerMode(NormalizationMode),
    /// Change the normalizer target level
    SetNormalizerTargetLevel(f32),
    /// Change the normalizer headroom (dB)
    SetNormalizerHeadroom(f32),
    /// Enable or disable the normalizer
    SetNormalizerEnabled(bool),
    /// Seek to a new position in the buffer (frame index)
    SeekToFrame(usize),
    /// Signal the processing task to stop gracefully
    Stop,
}
