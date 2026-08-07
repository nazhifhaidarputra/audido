use crate::metadata::{ChannelLayout, MusicalSongKey};
use rustfft::{FftPlanner, num_complex::Complex};
use thiserror::Error;

// TODO: Move this to configurable file for better UX
const WINDOW_SIZE: usize = 4095;
const HOP_SIZE: usize = WINDOW_SIZE / 4;

/// Chromatic scale profiles for major and minor keys (Krumhansl-Kessler profiles)
const MAJOR_PROFILE: [f32; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];

const MINOR_PROFILE: [f32; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.6, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];

#[derive(Error, Debug)]
pub enum KeyDetectionError {
    #[error("error when doing the DSP: {0}")]
    DSPError(String),
    #[error("empty buffer")]
    EmptyBuffer,
    #[error("sample rate must be positive")]
    InvalidSampleRate,
    #[error("buffer length incompatible with channel layout")]
    InvalidBufferLength,
}

pub struct SongKeyArgsBuilder<'a> {
    buffer: &'a [f32],
    sample_rate: f32,
    channel_layout: Option<ChannelLayout>,
}

pub struct SongKeyArgs<'a> {
    buffer: &'a [f32],
    sample_rate: f32,
    channel_layout: ChannelLayout,
}

impl<'a> SongKeyArgsBuilder<'a> {
    pub fn new(buffer: &'a [f32], sample_rate: f32) -> Self {
        Self {
            buffer,
            sample_rate,
            channel_layout: None,
        }
    }

    pub fn channel_layout(mut self, layout: ChannelLayout) -> Self {
        self.channel_layout = Some(layout);
        self
    }

    pub fn build(self) -> Result<SongKeyArgs<'a>, KeyDetectionError> {
        if self.buffer.is_empty() {
            return Err(KeyDetectionError::EmptyBuffer);
        }
        if self.sample_rate <= 0.0 {
            return Err(KeyDetectionError::InvalidSampleRate);
        }
        Ok(SongKeyArgs {
            buffer: self.buffer,
            sample_rate: self.sample_rate,
            channel_layout: self.channel_layout.unwrap_or(ChannelLayout::Unsupported),
        })
    }
}

/// Detect musical song key of the provided audio buffer
pub fn detect_song_key(args: SongKeyArgs) -> Result<MusicalSongKey, KeyDetectionError> {
    if args.buffer.is_empty() {
        return Err(KeyDetectionError::EmptyBuffer);
    }
    if args.sample_rate <= 0.0 {
        return Err(KeyDetectionError::InvalidSampleRate);
    }
    if let ChannelLayout::Unsupported = args.channel_layout {
        return Err(KeyDetectionError::DSPError(
            "Unsupported channel layout".to_string(),
        ));
    }
    // let mut detector = McLeodDetector::new(WINDOW_SIZE, PADDING_SIZE);

    // FIXME: Incorrect implementation of key detection. need more research
    let chromagram = compute_chromagram(args.buffer, args.sample_rate, args.channel_layout)?;
    // let pitch;
    let key = estimate_key(&chromagram);
    Ok(key)
}

fn compute_chromagram(
    buffer: &[f32],
    sample_rate: f32,
    channel_layout: ChannelLayout,
) -> Result<[f32; 12], KeyDetectionError> {
    let num_channels = match channel_layout {
        ChannelLayout::Mono => 1,
        ChannelLayout::Stereo => 2,
        ChannelLayout::Unsupported => {
            return Err(KeyDetectionError::DSPError(
                "Unsupported channel layout".to_string(),
            ));
        }
    };
    // Validate buffer length is compatible with channel layout
    if !buffer.len().is_multiple_of(num_channels) {
        return Err(KeyDetectionError::InvalidBufferLength);
    }

    let num_samples = buffer.len() / num_channels;
    let num_frames = if num_samples >= WINDOW_SIZE {
        (num_samples - WINDOW_SIZE) / HOP_SIZE + 1
    } else {
        0
    };

    if num_frames == 0 {
        return Err(KeyDetectionError::DSPError(
            "Buffer too short for analysis".to_string(),
        ));
    }

    let mut fft_planner = FftPlanner::new();
    let fft = fft_planner.plan_fft_forward(WINDOW_SIZE);

    let mut chroma_bins = [0.0f32; 12];
    let mut frame_buffer = vec![Complex::new(0.0f32, 0.0f32); WINDOW_SIZE];

    let window = hann_window(WINDOW_SIZE);

    for frame_idx in 0..num_frames {
        let sample_start = frame_idx * HOP_SIZE;

        // Mix down to mono for this frame based on channel layout
        for i in 0..WINDOW_SIZE {
            let sample_idx = sample_start + i;
            let mono_sample = match channel_layout {
                ChannelLayout::Mono => buffer[sample_idx],
                ChannelLayout::Stereo => {
                    let left = buffer[sample_idx * 2];
                    let right = buffer[sample_idx * 2 + 1];
                    0.5 * (left + right)
                }
                ChannelLayout::Unsupported => unreachable!(),
            };
            frame_buffer[i] = Complex::new(mono_sample * window[i], 0.0);
        }

        // FFT
        fft.process(&mut frame_buffer);

        // Map fft bins to chroma bins
        for (bin, item) in frame_buffer
            .iter()
            .enumerate()
            .take(WINDOW_SIZE / 2)
            .skip(1)
        {
            let magnitude = item.norm();
            let freq = bin as f32 * sample_rate / WINDOW_SIZE as f32;

            // Convert frequency to MIDI note number, then to pitch class
            if freq > 20.0 && freq < 20000.0 {
                // Only consider audible frequencies
                let midi_note = 69.0 + 12.0 * (freq / 440.0).log2();
                let pitch_class = ((midi_note.round() as i32).rem_euclid(12)) as usize;
                chroma_bins[pitch_class] += magnitude;
            }
        }
    }

    // Normalize chromagram
    let max_val = chroma_bins.iter().fold(0.0f32, |a, &b| a.max(b));
    if max_val > 0.0 {
        for val in &mut chroma_bins {
            *val /= max_val;
        }
    }

    Ok(chroma_bins)
}

fn estimate_key(chromagram: &[f32; 12]) -> MusicalSongKey {
    let mut best_correlation = f32::MIN;
    let mut best_key = MusicalSongKey::CMaj; // Default

    for semitone in 0..12 {
        let rotated_major = rotate_profile(&MAJOR_PROFILE, semitone);
        let corr_major = correlation(chromagram, &rotated_major);
        if corr_major > best_correlation {
            best_correlation = corr_major;
            best_key = match semitone {
                0 => MusicalSongKey::CMaj,
                1 => MusicalSongKey::CSharpMaj,
                2 => MusicalSongKey::DMaj,
                3 => MusicalSongKey::DSharpMaj,
                4 => MusicalSongKey::EMaj,
                5 => MusicalSongKey::FMaj,
                6 => MusicalSongKey::FSharpMaj,
                7 => MusicalSongKey::GMaj,
                8 => MusicalSongKey::GSharpMaj,
                9 => MusicalSongKey::AMaj,
                10 => MusicalSongKey::ASharpMaj,
                11 => MusicalSongKey::BMaj,
                _ => unreachable!(),
            };
        }

        let rotated_minor = rotate_profile(&MINOR_PROFILE, semitone);
        let corr_minor = correlation(chromagram, &rotated_minor);
        if corr_minor > best_correlation {
            best_correlation = corr_minor;
            best_key = match semitone {
                0 => MusicalSongKey::CMin,
                1 => MusicalSongKey::CSharpMin,
                2 => MusicalSongKey::DMin,
                3 => MusicalSongKey::DSharpMin,
                4 => MusicalSongKey::EMin,
                5 => MusicalSongKey::FMin,
                6 => MusicalSongKey::FSharpMin,
                7 => MusicalSongKey::GMin,
                8 => MusicalSongKey::GSharpMin,
                9 => MusicalSongKey::AMin,
                10 => MusicalSongKey::ASharpMin,
                11 => MusicalSongKey::BMin,
                _ => unreachable!(),
            };
        }
    }

    best_key
}

// ==================================
// Helper functions
// ==================================

#[inline(always)]
fn hann_window(window_size: usize) -> Vec<f32> {
    (0..window_size)
        .map(|i| {
            0.5 * (1.0 - ((2.0 * std::f32::consts::PI * (i as f32)) / (window_size as f32)).cos())
        })
        .collect()
}

fn rotate_profile(profile: &[f32; 12], semitones: usize) -> [f32; 12] {
    let mut rotated = [0.0f32; 12];
    for i in 0..12 {
        rotated[i] = profile[(i + semitones) % 12];
    }
    rotated
}

fn correlation(a: &[f32; 12], b: &[f32; 12]) -> f32 {
    let mean_a = a.iter().sum::<f32>() / 12.0;
    let mean_b = b.iter().sum::<f32>() / 12.0;

    let mut num = 0.0;
    let mut den_a = 0.0;
    let mut den_b = 0.0;

    for i in 0..12 {
        let diff_a = a[i] - mean_a;
        let diff_b = b[i] - mean_b;
        num += diff_a * diff_b;
        den_a += diff_a * diff_a;
        den_b += diff_b * diff_b;
    }

    if den_a == 0.0 || den_b == 0.0 {
        0.0
    } else {
        num / (den_a.sqrt() * den_b.sqrt())
    }
}
