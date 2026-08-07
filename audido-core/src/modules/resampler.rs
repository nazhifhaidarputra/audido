//! # Resampler Module
//!
//! Pre-resamples decoded audio buffers so their sample rate matches the audio
//! output device's fixed sample rate. This must run once, up front, at load
//! time — the CPAL output stream is opened at a single sample rate for its
//! entire lifetime, and the DSP feed loop pushes decoded samples straight
//! into the ring buffer with no per-sample-rate conversion. Without this
//! step, a file whose native rate differs from the device's plays back at
//! the wrong pitch/speed.
//!
//! A polynomial (non-FFT) asynchronous resampler is used deliberately: it
//! runs in linear time regardless of the input/output rate ratio, so a
//! full-track resample stays well within the "must be fast" budget even for
//! long files, unlike an FFT-based resampler whose chunk size can blow up
//! for awkward rate pairs.

use anyhow::Context;
use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Async, FixedAsync, PolynomialDegree, Resampler};

/// Input frames processed per resampler iteration.
const CHUNK_SIZE: usize = 2048;

/// Resample an interleaved f32 buffer from `input_rate` to `output_rate`.
///
/// Returns a copy of `samples` unchanged when the rates already match (or
/// either rate is unknown/zero), so callers can invoke this unconditionally
/// on every load without a separate fast-path check.
pub fn resample_to_device_rate(
    samples: &[f32],
    channels: u16,
    input_rate: u32,
    output_rate: u32,
) -> anyhow::Result<Vec<f32>> {
    if samples.is_empty() || input_rate == 0 || output_rate == 0 || input_rate == output_rate {
        return Ok(samples.to_vec());
    }

    let channels = channels as usize;
    anyhow::ensure!(channels > 0, "cannot resample audio with zero channels");
    anyhow::ensure!(
        samples.len().is_multiple_of(channels),
        "interleaved sample count {} is not a multiple of channel count {}",
        samples.len(),
        channels
    );

    let input_frames = samples.len() / channels;
    let ratio = output_rate as f64 / input_rate as f64;

    let mut resampler = Async::<f32>::new_poly(
        ratio,
        1.0,
        PolynomialDegree::Cubic,
        CHUNK_SIZE,
        channels,
        FixedAsync::Input,
    )
    .context("Failed to construct resampler")?;

    let output_frames = resampler.process_all_needed_output_len(input_frames);
    let mut output = vec![0.0f32; output_frames * channels];

    let input_adapter = InterleavedSlice::new(samples, channels, input_frames)
        .context("Failed to build resampler input adapter")?;
    let mut output_adapter = InterleavedSlice::new_mut(&mut output, channels, output_frames)
        .context("Failed to build resampler output adapter")?;

    let (_, frames_out) = resampler
        .process_all_into_buffer(&input_adapter, &mut output_adapter, input_frames, None)
        .context("Resampling failed")?;

    output.truncate(frames_out * channels);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn sine_buffer(seconds: u32, sample_rate: u32, channels: u16) -> Vec<f32> {
        let frames = seconds as usize * sample_rate as usize;
        let mut buf = Vec::with_capacity(frames * channels as usize);
        for i in 0..frames {
            let t = i as f32 / sample_rate as f32;
            let sample = (t * 440.0 * std::f32::consts::TAU).sin();
            for _ in 0..channels {
                buf.push(sample);
            }
        }
        buf
    }

    #[test]
    fn identity_when_rates_match() {
        let input = sine_buffer(1, 44100, 2);
        let output = resample_to_device_rate(&input, 2, 44100, 44100).unwrap();
        assert_eq!(input, output);
    }

    #[test]
    fn resamples_44100_to_48000() {
        let input = sine_buffer(1, 44100, 2);
        let output = resample_to_device_rate(&input, 2, 44100, 48000).unwrap();
        let expected_frames = 48000;
        let actual_frames = output.len() / 2;
        // Polynomial resampler is allowed a small margin of frames.
        assert!(
            (actual_frames as i64 - expected_frames as i64).abs() < 100,
            "expected ~{} frames, got {}",
            expected_frames,
            actual_frames
        );
    }

    /// A ~10 minute stereo track is a realistic worst case for a single track
    /// load. This must stay comfortably under the 2 second budget.
    ///
    /// The hard timing assertion only applies to optimized builds: a scalar
    /// per-sample resampler run under an unoptimized debug build is not
    /// representative of the shipped binary's performance.
    #[test]
    fn resamples_long_track_within_budget() {
        let input = sine_buffer(600, 44100, 2);
        let start = Instant::now();
        let output = resample_to_device_rate(&input, 2, 44100, 48000).unwrap();
        let elapsed = start.elapsed();
        assert!(!output.is_empty());
        println!("Resampled 10 min stereo track in {:?}", elapsed);

        #[cfg(not(debug_assertions))]
        assert!(
            elapsed.as_secs_f64() < 2.0,
            "resampling took too long: {:?}",
            elapsed
        );
    }
}
