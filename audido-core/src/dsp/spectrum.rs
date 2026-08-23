use std::sync::Arc;

use realfft::{RealFftPlanner, RealToComplex};
use rustfft::num_complex::Complex32;

/// FFT Spectrum engine for frequency visualization.
///
/// Uses a **circular accumulator** so the full FFT window is always populated
/// regardless of how small each incoming DSP chunk is.  The FFT is re-run
/// every `hop_size` new mono samples (default: `fft_size / 4`, 75 % overlap),
/// giving smooth animation.  Between hops the previous frame is returned.
///
/// Because the audio is for visualization only, stereo input is mixed 50/50
/// into mono before accumulation.
pub struct FftSpectrumEngine {
    pub bin_size: usize,
    fft_size: usize,
    input_buffer: Vec<f32>,
    output_buffer: Vec<Complex32>,
    planner: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,

    /// Circular mono-sample accumulator of length `fft_size`.
    accum: Vec<f32>,
    /// Next write position in `accum` (wraps at `fft_size`).
    accum_pos: usize,
    /// New mono samples written since the last FFT run.
    samples_since_last_fft: usize,
    /// Trigger a new FFT every `hop_size` new samples.
    hop_size: usize,
    /// Result from the last successful FFT run.
    last_bins: Vec<f32>,
}

impl FftSpectrumEngine {
    pub fn new(fft_size: usize, bin_size: usize) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft_plan = planner.plan_fft_forward(fft_size);

        let window = apodize::hanning_iter(fft_size)
            .map(|v| v as f32)
            .collect::<Vec<f32>>();

        let hop_size = fft_size / 4;

        Self {
            bin_size,
            fft_size,
            input_buffer: vec![0.0; fft_size],
            output_buffer: vec![Complex32::new(0.0, 0.0); fft_size / 2 + 1],
            planner: fft_plan,
            window,
            accum: vec![0.0; fft_size],
            accum_pos: 0,
            samples_since_last_fft: 0,
            hop_size,
            last_bins: vec![-140.0; bin_size],
        }
    }

    pub fn bin_size(mut self, v: usize) -> Self {
        self.bin_size = v;
        self.last_bins.resize(v, -140.0);
        self
    }

    /// Process an interleaved audio buffer.
    ///
    /// Downmixes each frame to mono and writes it into the circular accumulator.
    /// When `hop_size` new samples have accumulated, the full FFT window is
    /// computed and the resulting `bin_size` dB-scaled bins are returned and
    /// cached.  Between hops the previous cached frame is returned so callers
    /// always receive a complete result.
    pub fn process(&mut self, audio_data: &[f32], channels: u16) -> Vec<f32> {
        let channels = channels as usize;
        if channels == 0 || audio_data.is_empty() {
            return self.last_bins.clone();
        }

        // Downmix to mono and write into the circular accumulator.
        for frame in audio_data.chunks(channels) {
            let mono = frame.iter().sum::<f32>() / channels as f32;
            self.accum[self.accum_pos] = mono;
            self.accum_pos = (self.accum_pos + 1) % self.fft_size;
            self.samples_since_last_fft += 1;
        }

        // Only run FFT when we have a full hop of new data.
        if self.samples_since_last_fft < self.hop_size {
            return self.last_bins.clone();
        }
        self.samples_since_last_fft = 0;

        // Linearise the circular buffer into input_buffer (oldest sample first)
        // and apply the Hann window.  `accum_pos` is the *next write* position,
        // so the oldest sample sits exactly at `accum_pos`.
        let fft_size = self.fft_size;
        for i in 0..fft_size {
            let src = (self.accum_pos + i) % fft_size;
            self.input_buffer[i] = self.accum[src] * self.window[i];
        }

        if let Err(e) = self
            .planner
            .process(&mut self.input_buffer, &mut self.output_buffer)
        {
            log::error!("FFT processing failed: {}", e);
            return self.last_bins.clone();
        }

        // Map complex FFT output → display bins using linear grouping.
        // We use the positive-frequency half only (DC to Nyquist).
        let num_fft_bins = self.output_buffer.len(); // fft_size / 2 + 1
        let bin_size = self.bin_size;

        // Resize cached result if bin_size was hot-swapped externally.
        if self.last_bins.len() != bin_size {
            self.last_bins.resize(bin_size, -140.0);
        }

        for i in 0..bin_size {
            let start = (i * num_fft_bins) / bin_size;
            let end = (((i + 1) * num_fft_bins) / bin_size)
                .max(start + 1)
                .min(num_fft_bins);

            // Peak magnitude within this display bin, normalised by FFT size.
            let max_mag = self.output_buffer[start..end]
                .iter()
                .map(|c| c.norm() / fft_size as f32)
                .fold(0.0_f32, f32::max);

            self.last_bins[i] = Self::magnitude_to_db(max_mag);
        }

        self.last_bins.clone()
    }

    /// Converts a normalised magnitude amplitude to Decibels (dB).
    fn magnitude_to_db(magnitude: f32) -> f32 {
        if magnitude <= 1e-7 {
            -140.0
        } else {
            20.0 * magnitude.log10()
        }
    }
}
