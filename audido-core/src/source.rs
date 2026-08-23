use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Instant,
};

use anyhow::Context;
use lofty::{file::TaggedFileExt, probe::Probe, tag::Accessor};
use rodio::{Decoder, Source};

use crate::metadata::{AudioMetadata, ChannelLayout};

use crate::dsp::pitch_detection::{SongKeyArgsBuilder, detect_song_key};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioSource {
    Local { path: PathBuf },
    Youtube { url: String },
}

impl AudioSource {
    pub fn get_path(&self) -> Option<String> {
        match self {
            AudioSource::Local { path } => {
                path.file_name().map(|s| s.to_string_lossy().to_string())
            }
            AudioSource::Youtube { url } => Some(url.clone()),
        }
    }
}

/// Shared position tracker between source and engine
#[derive(Clone, Debug)]
pub struct PositionTracker {
    /// Current sample position (atomic for thread-safe access)
    position: Arc<AtomicUsize>,
    /// Total number of samples
    total_samples: usize,
    /// Sample rate for time calculations
    sample_rate: u32,
    /// Number of channels
    channels: u16,
}

impl PositionTracker {
    pub fn new(total_samples: usize, sample_rate: u32, channels: u16) -> Self {
        Self {
            position: Arc::new(AtomicUsize::new(0)),
            total_samples,
            sample_rate,
            channels,
        }
    }

    /// Get current position in seconds
    pub fn position_seconds(&self) -> f32 {
        let pos = self.position.load(Ordering::Relaxed);
        let frames = pos / (self.channels as usize);
        (frames as f32) / (self.sample_rate as f32)
    }

    /// Get total duration in seconds
    pub fn duration_seconds(&self) -> f32 {
        let frames = self.total_samples / (self.channels as usize);
        (frames as f32) / (self.sample_rate as f32)
    }

    /// Set position from seconds
    pub fn seek_to_seconds(&self, seconds: f32) {
        let frames = (seconds * (self.sample_rate as f32)) as usize;
        let sample_pos = (frames * (self.channels as usize)).min(self.total_samples);
        self.position.store(sample_pos, Ordering::Relaxed);
    }

    /// Reset position to start
    pub fn reset(&self) {
        self.position.store(0, Ordering::Relaxed);
    }
}

#[derive(Debug)]
pub struct AudioPlaybackData {
    metadata: Arc<Mutex<AudioMetadata>>,
    buffer: AudioBuffer,
    position_tracker: PositionTracker,
}

/// A growing, retained PCM buffer used by streaming sources.
///
/// Unlike a receiver channel, decoded samples remain addressable after they
/// have been played. The DSP loop can therefore move its read cursor backward
/// or forward anywhere inside the decoded portion of the stream.
#[derive(Clone, Debug, Default)]
pub struct StreamingAudioBuffer {
    samples: Arc<RwLock<Vec<f32>>>,
    complete: Arc<AtomicBool>,
}

impl StreamingAudioBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Retain another batch of interleaved PCM samples.
    pub fn append(&self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }

        self.samples
            .write()
            .expect("streaming audio buffer poisoned")
            .extend_from_slice(samples);
    }

    /// Mark the decoder as finished. No samples will be appended afterward.
    pub fn mark_complete(&self) {
        self.complete.store(true, Ordering::Release);
    }

    pub fn is_complete(&self) -> bool {
        self.complete.load(Ordering::Acquire)
    }

    /// Number of interleaved PCM samples currently available for seeking.
    pub fn buffered_samples(&self) -> usize {
        self.samples
            .read()
            .expect("streaming audio buffer poisoned")
            .len()
    }

    /// Whether two playback buffers reference the same retained stream.
    #[cfg(test)]
    pub(crate) fn shares_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.samples, &other.samples)
    }

    /// Copy up to `max_samples` retained samples beginning at `position`.
    pub fn copy_from(&self, position: usize, max_samples: usize, output: &mut Vec<f32>) -> usize {
        let samples = self
            .samples
            .read()
            .expect("streaming audio buffer poisoned");
        if position >= samples.len() {
            return 0;
        }

        let end = position.saturating_add(max_samples).min(samples.len());
        output.extend_from_slice(&samples[position..end]);
        end - position
    }
}

#[derive(Clone, Debug)]
pub enum AudioBuffer {
    InMemory(Arc<Vec<f32>>),
    Stream(StreamingAudioBuffer),
}

impl AudioBuffer {
    /// Number of samples that can be addressed immediately without waiting
    /// for more data to be decoded.
    pub fn buffered_samples(&self) -> usize {
        match self {
            Self::InMemory(samples) => samples.len(),
            Self::Stream(samples) => samples.buffered_samples(),
        }
    }
}

impl AudioPlaybackData {
    pub fn new(
        metadata: AudioMetadata,
        buffer: AudioBuffer,
        position_tracker: PositionTracker,
    ) -> Self {
        Self {
            metadata: Arc::new(Mutex::new(metadata)),
            buffer,
            position_tracker,
        }
    }
    /// Decode `path` and, when `target_sample_rate` is given and differs from the
    /// file's native rate, pre-resample the decoded buffer to that rate so it's
    /// already synchronized with the output device before playback starts.
    pub fn load_local_audio(
        path: &str,
        target_sample_rate: Option<u32>,
    ) -> anyhow::Result<AudioPlaybackData> {
        // calculate time required for performance monitoring
        let start_time = Instant::now();

        let file = File::open(path).context("Failed to open the file")?;
        let decoder = Decoder::try_from(file).context("Failed to decode the opened audio file")?;

        let source_sample_rate = decoder.sample_rate();
        let num_channels = decoder.channels();

        let channel_layout = match num_channels {
            1 => ChannelLayout::Mono,
            2 => ChannelLayout::Stereo,
            _ => ChannelLayout::Unsupported,
        };

        log::debug!("Starting full decode with rodio.");
        let samples: Vec<f32> = decoder.collect();
        log::debug!("Finished decoding {} samples.", samples.len());

        let (samples, sample_rate) = match target_sample_rate {
            Some(target_rate) if target_rate != source_sample_rate => {
                let resample_start = Instant::now();
                let resampled = crate::modules::resampler::resample_to_device_rate(
                    &samples,
                    num_channels,
                    source_sample_rate,
                    target_rate,
                )
                .context("Failed to pre-resample audio to device sample rate")?;
                log::info!(
                    "Pre-resampled {} Hz -> {} Hz in {:?} ({} -> {} samples)",
                    source_sample_rate,
                    target_rate,
                    resample_start.elapsed(),
                    samples.len(),
                    resampled.len()
                );
                (resampled, target_rate)
            }
            _ => (samples, source_sample_rate),
        };

        let n_frames = (samples.len() / (num_channels as usize)) as u32;
        let duration_in_seconds = if sample_rate > 0 {
            (n_frames as f32) / (sample_rate as f32)
        } else {
            0.0
        };

        let file_ext = Path::new(path)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        // Create metadata with default values first
        let mut initial_metadata = AudioMetadata {
            sample_rate,
            num_channels,
            channel_layout,
            duration: duration_in_seconds,
            format: file_ext.clone(),
            ..Default::default()
        };

        // Read static metadata immediately
        Self::read_audio_metadata(path, &mut initial_metadata)?;

        let metadata = Arc::new(Mutex::new(initial_metadata));
        let samples_arc = Arc::new(samples);

        // Spawn analysis in background thread
        let metadata_for_thread = Arc::clone(&metadata);
        let samples_for_thread = Arc::clone(&samples_arc);

        thread::spawn(move || {
            if let Err(e) = Self::analyze_audio_properties(
                &samples_for_thread,
                sample_rate as f32,
                num_channels,
                &metadata_for_thread,
            ) {
                log::error!("Audio analysis failed: {}", e);
            }
        });

        let total_samples = samples_arc.len();
        let position_tracker = PositionTracker::new(total_samples, sample_rate, num_channels);

        let playback_data = AudioPlaybackData {
            metadata,
            buffer: AudioBuffer::InMemory(samples_arc),
            position_tracker,
        };

        log::debug!("Load audio finished in {:?} seconds", start_time.elapsed());
        Ok(playback_data)
    }

    /// Analyze audio properties in background and update metadata when done
    // FIXME: Incorrectly classify the song key
    fn analyze_audio_properties(
        buffer: &[f32],
        sample_rate: f32,
        num_channels: u16,
        metadata: &Arc<Mutex<AudioMetadata>>,
    ) -> anyhow::Result<()> {
        let start = Instant::now();
        log::info!("Starting background audio analysis...");

        // Perform key detection
        let song_key_args = SongKeyArgsBuilder::new(buffer, sample_rate)
            .channel_layout(ChannelLayout::from_channels(num_channels))
            .build()?;

        let key = detect_song_key(song_key_args)?;

        // Lock mutex and update metadata
        {
            let mut meta = metadata
                .lock()
                .map_err(|e| anyhow::anyhow!("Failed to lock metadata mutex: {}", e))?;
            meta.key = Some(key);
            log::info!(
                "Audio analysis completed in {:?}. Detected key: {:?}",
                start.elapsed(),
                meta.key
            );
        }

        Ok(())
    }

    //// Get audio metadata from loaded file (title, author, album, genre, etc)
    fn read_audio_metadata(path: &str, metadata: &mut AudioMetadata) -> anyhow::Result<()> {
        match Probe::open(path).and_then(|p| p.read()) {
            Ok(tagged_file) => {
                if let Some(tag) = tagged_file.primary_tag() {
                    metadata.title = tag.title().map(|s| s.to_string());
                    metadata.author = tag.artist().map(|s| s.to_string());
                    metadata.album = tag.album().map(|s| s.to_string());
                    metadata.genre = tag.genre().map(|s| s.to_string());
                    metadata.cover = tag.pictures().first().map(|pic| pic.data().to_vec());

                    log::info!(
                        "Metadata loaded: {:?} by {:?}",
                        metadata.title,
                        metadata.author
                    );
                }
            }
            Err(e) => {
                log::warn!("Failed to read metadata: {}", e);
            }
        }

        if metadata.title.is_none() {
            metadata.title = Path::new(path)
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string());
        }
        Ok(())
    }

    /// Get a cloned copy of the audio metadata
    pub fn metadata(&self) -> AudioMetadata {
        let guard = self.metadata.lock().expect("metadata mutex poisoned");
        guard.clone()
    }

    /// Get a reference to the position tracker (legacy — prefer atomics in CoreContext)
    pub fn position_tracker(&self) -> &PositionTracker {
        &self.position_tracker
    }

    /// Direct access to the raw decoded sample buffer (interleaved f32).
    /// Used by the new DSP feed loop to read chunks into the SPSC ring buffer.
    pub fn buffer(&self) -> &AudioBuffer {
        &self.buffer
    }

    /// Total number of interleaved f32 samples (frames × channels).
    pub fn total_samples(&self) -> usize {
        match &self.buffer {
            AudioBuffer::InMemory(samples) => samples.len(),
            AudioBuffer::Stream(_) => {
                let metadata = self.metadata();
                (metadata.sample_rate as f32 * metadata.duration * metadata.num_channels as f32)
                    as usize
            }
        }
    }

    /// Number of interleaved samples currently loaded and seekable.
    pub fn buffered_samples(&self) -> usize {
        self.buffer.buffered_samples()
    }
}

#[cfg(test)]
mod streaming_buffer_tests {
    use super::StreamingAudioBuffer;

    #[test]
    fn retained_stream_can_be_read_again_at_any_buffered_position() {
        let buffer = StreamingAudioBuffer::new();
        buffer.append(&[0.0, 0.1, 0.2, 0.3]);
        buffer.append(&[0.4, 0.5]);

        let mut output = Vec::new();
        assert_eq!(buffer.copy_from(3, 2, &mut output), 2);
        assert_eq!(output, vec![0.3, 0.4]);

        output.clear();
        assert_eq!(buffer.copy_from(0, 3, &mut output), 3);
        assert_eq!(output, vec![0.0, 0.1, 0.2]);
        assert_eq!(buffer.buffered_samples(), 6);
        assert!(!buffer.is_complete());

        buffer.mark_complete();
        assert!(buffer.is_complete());
    }
}

// #[cfg(test)]
// mod test {
//     pub fn test_loading_audio() {}

//     pub fn test_reading_metadata() {}

//     pub fn test_audio_analysis() {}
// }
