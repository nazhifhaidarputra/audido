use std::{
    fs::File,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Instant,
};

use anyhow::Context;
use crossbeam_channel::Receiver;
use lofty::{file::TaggedFileExt, probe::Probe, tag::Accessor};
use rodio::{Decoder, Source};

use crate::{
    commands::RealtimeCommand,
    dsp::{dsp_graph::DspNode, eq::Equalizer, normalization::Normalizer},
    metadata::{AudioMetadata, ChannelLayout},
};

use crate::dsp::pitch_detection::{SongKeyArgsBuilder, detect_song_key};

const CHUNK_SIZE: usize = 512;

/// Shared position tracker between source and engine
#[derive(Clone)]
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

pub struct AudioPlaybackData {
    metadata: Arc<Mutex<AudioMetadata>>,
    buffer: Arc<Vec<f32>>,
    position_tracker: PositionTracker,
}

pub enum AudioSource {
    Local { data: AudioPlaybackData },
}

impl AudioPlaybackData {
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
            buffer: samples_arc,
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
    pub fn buffer(&self) -> Arc<Vec<f32>> {
        Arc::clone(&self.buffer)
    }

    /// Total number of interleaved f32 samples (frames × channels).
    pub fn total_samples(&self) -> usize {
        self.buffer.len()
    }

    /// Create a rodio `Source` from the buffered audio data.  
    /// **Retained for backward compatibility** — the new DSP pipeline uses
    /// `buffer()` directly and does not need a rodio Source.
    pub fn create_source(
        &self,
        initial_eq: Equalizer,
        eq_enabled: bool,
        cmd_rx: Receiver<RealtimeCommand>,
    ) -> BufferedSource {
        BufferedSource::new(
            self.buffer.clone(),
            self.metadata().sample_rate,
            self.metadata().num_channels,
            self.position_tracker.clone(),
            initial_eq,
            eq_enabled,
            cmd_rx,
        )
    }
}

/// A buffered audio source that implements rodio's Source trait.
/// **Note**: This type is retained for legacy/testing use. The new audio pipeline
/// uses the DSP feed loop in `modules::playback` with the SPSC ring buffer directly.
pub struct BufferedSource {
    samples: Arc<Vec<f32>>,
    sample_rate: u32,
    channels: u16,
    position_tracker: PositionTracker,
    equalizer: DspNode<Equalizer>,
    normalizer: DspNode<Normalizer>,
    cmd_rx: Receiver<RealtimeCommand>,

    // Chunk Processing
    process_buffer: Vec<f32>,
    process_buffer_idx: usize,
}

impl BufferedSource {
    pub fn new(
        samples: Arc<Vec<f32>>,
        sample_rate: u32,
        channels: u16,
        position_tracker: PositionTracker,
        equalizer: Equalizer,
        eq_enabled: bool,
        cmd_rx: Receiver<RealtimeCommand>,
    ) -> Self {
        Self {
            samples,
            sample_rate,
            channels,
            position_tracker,
            equalizer: DspNode::new_with_state(equalizer, eq_enabled),
            normalizer: DspNode::new_with_state(Normalizer::new(), false),
            cmd_rx,
            process_buffer: Vec::with_capacity(CHUNK_SIZE),
            process_buffer_idx: 0,
        }
    }

    fn fill_buffer(&mut self) -> bool {
        self.process_buffer.clear();
        self.process_buffer_idx = 0;

        // Process Pending Realtime Commands (Lock-Free)
        while let Ok(cmd) = self.cmd_rx.try_recv() {
            match cmd {
                RealtimeCommand::Stop => return false,
                RealtimeCommand::SeekToFrame(frame) => {
                    self.position_tracker.position.store(frame, Ordering::Relaxed);
                }
                RealtimeCommand::UpdateEqFilter(idx, filter_node) => {
                    self.equalizer.set_filter(idx, filter_node);
                }
                RealtimeCommand::SetAllEqFilters(filter_nodes) => {
                    self.equalizer.set_all_filters(filter_nodes);
                }
                RealtimeCommand::SetEqMasterGain(gain) => {
                    self.equalizer.set_master_gain(gain);
                }
                RealtimeCommand::SetEqPreset(preset) => {
                    self.equalizer.instance.update_preset(preset);
                }
                RealtimeCommand::SetEqEnabled(enabled) => {
                    self.equalizer.on = enabled;
                }
                RealtimeCommand::ResetEq => {
                    self.equalizer.instance.reset_parameters();
                }
                RealtimeCommand::ResetEqFilterNode(index) => {
                    let _ = self.equalizer.instance.reset_filter_node_param(index);
                }
                RealtimeCommand::SetNormalizerMode(mode) => {
                    self.normalizer.instance.set_mode(mode);
                }
                RealtimeCommand::SetNormalizerTargetLevel(level) => {
                    self.normalizer.instance.set_target_level(level);
                }
                RealtimeCommand::SetNormalizerHeadroom(headroom) => {
                    self.normalizer.instance.set_headroom(headroom);
                }
                RealtimeCommand::SetNormalizerEnabled(enabled) => {
                    self.normalizer.on = enabled;
                }
            }
        }

        // Fetch Audio
        let global_pos = self.position_tracker.position.load(Ordering::Relaxed);
        if global_pos >= self.samples.len() {
            return false;
        }

        let end_pos = (global_pos + CHUNK_SIZE).min(self.samples.len());
        self.process_buffer
            .extend_from_slice(&self.samples[global_pos..end_pos]);

        // Apply DSP filters in order: EQ -> Normalizer
        if self.equalizer.on {
            self.equalizer
                .instance
                .process_frame(&mut self.process_buffer);
        }

        // Apply normalizer if enabled
        if self.normalizer.on {
            self.normalizer.instance.process(&mut self.process_buffer);
        }

        true
    }
}

impl Iterator for BufferedSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        // If we've exhausted the process buffer, refill it
        if self.process_buffer_idx >= self.process_buffer.len() && !self.fill_buffer() {
            return None;
        }

        // Return the next sample from our processed buffer
        if self.process_buffer_idx < self.process_buffer.len() {
            let sample = self.process_buffer[self.process_buffer_idx];
            self.process_buffer_idx += 1;

            // Update position tracker
            let pos = self.position_tracker.position.load(Ordering::Relaxed);
            self.position_tracker
                .position
                .store(pos + 1, Ordering::Relaxed);

            Some(sample)
        } else {
            None
        }
    }
}

impl Source for BufferedSource {
    fn current_span_len(&self) -> Option<usize> {
        let pos = self.position_tracker.position.load(Ordering::Relaxed);
        Some(self.samples.len() - pos)
    }

    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        let frames = self.samples.len() / (self.channels as usize);
        Some(std::time::Duration::from_secs_f64(
            (frames as f64) / (self.sample_rate as f64),
        ))
    }
}

// #[cfg(test)]
// mod test {
//     pub fn test_loading_audio() {}

//     pub fn test_reading_metadata() {}

//     pub fn test_audio_analysis() {}
// }
