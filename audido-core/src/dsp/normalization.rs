use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// Normalization mode: Peak or RMS-based
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizationMode {
    Peak,
    RMS,
}

/// Real-time audio normalizer with peak and RMS-based algorithms
#[derive(Clone, Debug)]
pub struct Normalizer {
    /// Current normalization mode
    mode: NormalizationMode,
    /// Target loudness level (-20.0 to 0.0 dB for RMS, or 0.0 to 1.0 for peak)
    target_level: f32,
    /// Headroom to preserve in dB (e.g., 3.0 for -3dB headroom)
    headroom_db: f32,
    /// Current gain factor to apply (atomic for lock-free updates)
    gain: Arc<AtomicU32>,
    /// RMS smoothing factor (0.0-1.0) for exponential moving average
    rms_smoothing: f32,
    /// Last calculated RMS value
    last_rms: f32,
}

impl Normalizer {
    /// Create a new normalizer with default settings
    pub fn new() -> Self {
        Self {
            mode: NormalizationMode::Peak,
            target_level: 0.9, // Peak: target 90% of full scale
            headroom_db: 3.0,  // Preserve 3dB headroom
            gain: Arc::new(AtomicU32::new(f32::to_bits(1.0))),
            rms_smoothing: 0.2, // Exponential moving average factor
            last_rms: 0.0,
        }
    }

    /// Set the normalization mode
    pub fn set_mode(&mut self, mode: NormalizationMode) {
        self.mode = mode;
    }

    /// Get the current normalization mode
    pub fn mode(&self) -> NormalizationMode {
        self.mode
    }

    /// Set the target loudness level
    /// For Peak mode: 0.0-1.0 (fraction of full scale)
    /// For RMS mode: -40.0-0.0 dB
    pub fn set_target_level(&mut self, level: f32) {
        self.target_level = match self.mode {
            NormalizationMode::Peak => level.clamp(0.1, 1.0),
            NormalizationMode::RMS => level.clamp(-40.0, 0.0),
        };
    }

    /// Get the current target level
    pub fn target_level(&self) -> f32 {
        self.target_level
    }

    /// Set headroom in dB (only applies to RMS mode)
    pub fn set_headroom(&mut self, headroom_db: f32) {
        self.headroom_db = headroom_db.max(0.0);
    }

    /// Calculate peak normalization gain
    /// Finds the maximum absolute value and calculates gain to reach target level
    fn calculate_peak_gain(buffer: &[f32], target_level: f32) -> f32 {
        let peak = buffer
            .iter()
            .map(|s| s.abs())
            .fold(0.0f32, |a, b| a.max(b));

        if peak > 0.0 && peak < target_level {
            target_level / peak
        } else if peak > target_level {
            target_level / peak
        } else {
            1.0
        }
    }

    /// Calculate RMS normalization gain
    /// Computes RMS (root mean square) loudness and calculates gain to reach target
    fn calculate_rms_gain(buffer: &[f32], target_level_db: f32, headroom_db: f32) -> f32 {
        if buffer.is_empty() {
            return 1.0;
        }

        // Calculate RMS value
        let rms_value = {
            let sum_squares: f32 = buffer.iter().map(|s| s * s).sum();
            (sum_squares / buffer.len() as f32).sqrt()
        };

        if rms_value < 1e-6 {
            return 1.0; // Avoid division by very small numbers
        }

        // Convert RMS to dB
        let rms_db = 20.0 * rms_value.log10();

        // Calculate target with headroom
        let adjusted_target = target_level_db - headroom_db;

        // Calculate gain needed in dB
        let gain_db = adjusted_target - rms_db;

        // Convert dB back to linear gain factor
        10.0f32.powf(gain_db / 20.0)
    }

    /// Process a chunk of audio with the current normalization settings
    pub fn process(&mut self, buffer: &mut [f32]) {
        if buffer.is_empty() {
            return;
        }

        // Calculate gain based on mode
        let gain = match self.mode {
            NormalizationMode::Peak => {
                Self::calculate_peak_gain(buffer, self.target_level)
            }
            NormalizationMode::RMS => {
                let new_rms_gain =
                    Self::calculate_rms_gain(buffer, self.target_level, self.headroom_db);
                // Apply exponential moving average for smooth gain transitions
                self.last_rms = self.rms_smoothing * new_rms_gain
                    + (1.0 - self.rms_smoothing) * self.last_rms;
                self.last_rms
            }
        };

        // Clamp gain to reasonable range to prevent extreme amplification/reduction
        let safe_gain = gain.clamp(0.1, 10.0);

        // Store gain atomically for lock-free access
        self.gain.store(f32::to_bits(safe_gain), Ordering::Relaxed);

        // Apply gain to all samples
        for sample in buffer.iter_mut() {
            *sample *= safe_gain;
        }
    }

    /// Get the current applied gain (for monitoring/UI)
    pub fn current_gain(&self) -> f32 {
        f32::from_bits(self.gain.load(Ordering::Relaxed))
    }

    /// Get current gain in dB (for display purposes)
    pub fn current_gain_db(&self) -> f32 {
        20.0 * self.current_gain().log10()
    }
}

impl Default for Normalizer {
    fn default() -> Self {
        Self::new()
    }
}