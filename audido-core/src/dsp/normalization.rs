use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

pub const DEFAULT_PEAK_TARGET: f32 = 0.9;
pub const DEFAULT_LOUDNESS_TARGET_LUFS: f32 = -18.0;
pub const DEFAULT_HEADROOM_DB: f32 = 1.0;

const MIN_GAIN: f32 = 0.1;
const MAX_GAIN: f32 = 3.981_071_7; // +12 dB
const SILENCE_LUFS: f32 = -70.0;
const MIN_SAMPLE_RATE: u32 = 8_000;
const MAX_SAMPLE_RATE: u32 = 384_000;
const MAX_CHANNELS: u16 = 32;

/// A single second-order IIR section (Direct Form I) with persistent state,
/// used to build the two-stage ITU-R BS.1770 K-weighting filter.
#[derive(Clone, Copy, Debug, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn process(&mut self, x0: f32) -> f32 {
        let y0 = self.b0 * x0 + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x0;
        self.y2 = self.y1;
        self.y1 = y0;
        y0
    }

    fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}

#[derive(Clone, Copy, Debug)]
struct KWeightingFilter {
    stage1: Biquad,
    stage2: Biquad,
}

impl KWeightingFilter {
    fn new(sample_rate: f32) -> Self {
        let (stage1, stage2) = k_weighting_biquads(sample_rate);
        Self { stage1, stage2 }
    }

    fn process(&mut self, sample: f32) -> f32 {
        self.stage2.process(self.stage1.process(sample))
    }

    fn reset(&mut self) {
        self.stage1.reset();
        self.stage2.reset();
    }
}

/// Derives the ITU-R BS.1770 K-weighting filter for a given sample rate, as
/// two cascaded biquads:
///   1. a high-frequency shelf modeling head diffraction/acoustic effects
///   2. the "RLB" high-pass, attenuating sub-bass the ear barely perceives
///      as loud
///
/// Coefficients depend on sample rate, so they're recalculated rather than
/// hardcoded for 48kHz. The magic constants (f0, G, Q for each stage) are
/// the standard's defining constants, computed in f64 for accuracy and cast
/// down to f32 for the per-sample hot loop. Verified against the commonly
/// published 48kHz reference coefficients.
fn k_weighting_biquads(sample_rate: f32) -> (Biquad, Biquad) {
    let sample_rate = sample_rate as f64;

    // Stage 1: high-shelf pre-filter.
    let f0 = 1681.974450955533_f64;
    let g = 3.999843853973347_f64;
    let q = 0.7071752369554196_f64;
    let k = (std::f64::consts::PI * f0 / sample_rate).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let a0 = 1.0 + k / q + k * k;
    let stage1 = Biquad {
        b0: ((vh + vb * k / q + k * k) / a0) as f32,
        b1: (2.0 * (k * k - vh) / a0) as f32,
        b2: ((vh - vb * k / q + k * k) / a0) as f32,
        a1: (2.0 * (k * k - 1.0) / a0) as f32,
        a2: ((1.0 - k / q + k * k) / a0) as f32,
        ..Default::default()
    };

    // Stage 2: RLB high-pass.
    let f0 = 38.13547087602444_f64;
    let q = 0.5003270373238773_f64;
    let k = (std::f64::consts::PI * f0 / sample_rate).tan();
    let a0 = 1.0 + k / q + k * k;
    let stage2 = Biquad {
        b0: 1.0,
        b1: -2.0,
        b2: 1.0,
        a1: (2.0 * (k * k - 1.0) / a0) as f32,
        a2: ((1.0 - k / q + k * k) / a0) as f32,
        ..Default::default()
    };

    (stage1, stage2)
}

/// Normalization mode: Peak or LUFS-based ("Loudness") normalization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizationMode {
    Peak,
    Loudness,
}

impl std::fmt::Display for NormalizationMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Peak => write!(f, "Peak"),
            Self::Loudness => write!(f, "Loudness"),
        }
    }
}

/// Real-time audio normalizer with peak and BS.1770 momentary-loudness modes.
/// This is an adaptive playback controller, not EBU R128 programme
/// normalization (which requires measuring the complete programme).
///
/// Real-time normalization has no lookahead: it only ever knows the peak or
/// loudness of the buffer it's currently holding, so the "ideal" gain to hit the
/// target exactly is never applied directly. Instead it's run through an
/// attack/release smoother, which is what actually makes this "correct" for
/// real-time use rather than a per-buffer snap:
///   * without smoothing, the gain jumps every buffer -> zipper noise/clicks
///   * a near-silent buffer would otherwise get boosted toward the gain
///     ceiling and then be yanked back down as soon as the signal returns,
///     i.e. audible pumping
///   * gain is allowed to fall quickly (attack) so a sudden loud transient
///     doesn't clip, but only climbs back up slowly (release)
///
/// Note on `Clone`: clones share the gain and meter atomics, but not filter,
/// smoothing, mode, or target state. Only the realtime clone should process.
#[derive(Clone, Debug)]
pub struct Normalizer {
    mode: NormalizationMode,
    peak_target: f32,
    loudness_target_lufs: f32,
    /// Sample-peak headroom below full scale (Loudness mode only).
    headroom_db: f32,
    /// Currently-applied gain, published for lock-free reads from other threads.
    gain: Arc<AtomicU32>,
    /// release time constants. The coefficient is calculated from the
    /// number of frames in each call, so behaviour does not depend on chunk size.
    attack_seconds: f32,
    /// release time constants. The coefficient is calculated from the
    /// number of frames in each call, so behaviour does not depend on chunk size.
    release_seconds: f32,
    /// Smoothed gain carried between calls to `process` (per-instance state).
    smoothed_gain: f32,
    /// Last measured input level, for metering/UI: linear peak amplitude in
    /// Peak mode, momentary LUFS (ITU-R BS.1770) in Loudness mode.
    measured_level: Arc<AtomicU32>,

    // Sample rate audio is arriving at. Needed because the K-weighting
    // filter's coefficients and the 400ms measurement window are both
    // defined in real time, not in samples.
    sample_rate: u32,
    num_channels: u16,
    
    /// BS.1770 requires independent K-weighting state for every channel.
    k_filters: Vec<KWeightingFilter>,
    
    // Ring buffer of channel-weighted energy per frame spanning the most
    // recent 400 ms, plus a running sum for O(1) updates.
    window_buf: Vec<f32>,
    window_size: usize,
    window_pos: usize,
    
    // How many frames have been written so far, capped at `window_size`.
    // Used so the mean-square divisor doesn't include not-yet-written
    // zeroes and underestimate loudness while the window is still filling.
    window_filled: usize,
    window_sum: f64,
}

impl Normalizer {
    /// Create a new mono normalizer with default settings at 48 kHz.
    pub fn new() -> Self {
        Self::with_format(48_000, 1)
    }

    /// Create a new mono normalizer for a specific sample rate.
    pub fn with_sample_rate(sample_rate: u32) -> Self {
        Self::with_format(sample_rate, 1)
    }

    /// Create a normalizer for interleaved audio with the given format.
    pub fn with_format(sample_rate: u32, num_channels: u16) -> Self {
        let sample_rate = sample_rate.clamp(MIN_SAMPLE_RATE, MAX_SAMPLE_RATE);
        let num_channels = num_channels.clamp(1, MAX_CHANNELS);
        let window_size = ((sample_rate as f32) * 0.4).round().max(1.0) as usize;
        Self {
            mode: NormalizationMode::Peak,
            peak_target: DEFAULT_PEAK_TARGET,
            loudness_target_lufs: DEFAULT_LOUDNESS_TARGET_LUFS,
            headroom_db: DEFAULT_HEADROOM_DB,
            gain: Arc::new(AtomicU32::new(f32::to_bits(1.0))),
            attack_seconds: 0.050,
            release_seconds: 1.0,
            smoothed_gain: 1.0,
            measured_level: Arc::new(AtomicU32::new(f32::to_bits(0.0))),
            sample_rate,
            num_channels,
            k_filters: (0..num_channels)
                .map(|_| KWeightingFilter::new(sample_rate as f32))
                .collect(),
            window_buf: vec![0.0; window_size],
            window_size,
            window_pos: 0,
            window_filled: 0,
            window_sum: 0.0,
        }
    }

    /// Change the sample rate while preserving the channel count.
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        self.set_format(sample_rate, self.num_channels);
    }

    /// Change the interleaved audio format and clear measurement history.
    pub fn set_format(&mut self, sample_rate: u32, num_channels: u16) {
        let sample_rate = sample_rate.clamp(MIN_SAMPLE_RATE, MAX_SAMPLE_RATE);
        let num_channels = num_channels.clamp(1, MAX_CHANNELS);
        if sample_rate == self.sample_rate && num_channels == self.num_channels {
            self.reset_measurement();
            return;
        }

        self.sample_rate = sample_rate;
        self.num_channels = num_channels;
        self.k_filters = (0..num_channels)
            .map(|_| KWeightingFilter::new(sample_rate as f32))
            .collect();
        self.window_size = ((sample_rate as f32) * 0.4).round().max(1.0) as usize;
        self.window_buf = vec![0.0; self.window_size];
        self.reset_measurement();
    }

    /// Get the sample rate the normalizer is currently configured for.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn num_channels(&self) -> u16 {
        self.num_channels
    }

    /// Clear filters, the momentary window, and gain smoothing. This should
    /// be called after seeks and when re-enabling the processor.
    pub fn reset_measurement(&mut self) {
        self.k_filters.iter_mut().for_each(KWeightingFilter::reset);
        self.window_buf.fill(0.0);
        self.window_pos = 0;
        self.window_filled = 0;
        self.window_sum = 0.0;
        self.smoothed_gain = 1.0;
        self.gain.store(f32::to_bits(1.0), Ordering::Relaxed);
        let idle_level = match self.mode {
            NormalizationMode::Peak => 0.0,
            NormalizationMode::Loudness => SILENCE_LUFS,
        };
        self.measured_level
            .store(f32::to_bits(idle_level), Ordering::Relaxed);
    }

    /// Set the normalization mode.
    ///
    /// Peak and loudness targets are retained independently across switches.
    pub fn set_mode(&mut self, mode: NormalizationMode) {
        if mode == self.mode {
            return;
        }
        self.mode = mode;
        self.reset_measurement();
    }

    /// Get the current normalization mode.
    pub fn mode(&self) -> NormalizationMode {
        self.mode
    }

    /// Set the target loudness level.
    /// For Peak mode: 0.1-1.0 (fraction of full scale)
    /// For Loudness mode: -40.0-0.0 LUFS
    pub fn set_target_level(&mut self, level: f32) {
        match self.mode {
            NormalizationMode::Peak => self.peak_target = level.clamp(0.1, 1.0),
            NormalizationMode::Loudness => {
                self.loudness_target_lufs = level.clamp(-40.0, 0.0);
            }
        }
    }

    /// Get the current target level.
    pub fn target_level(&self) -> f32 {
        match self.mode {
            NormalizationMode::Peak => self.peak_target,
            NormalizationMode::Loudness => self.loudness_target_lufs,
        }
    }

    /// Set headroom in dB (only applies to Loudness mode).
    pub fn set_headroom(&mut self, headroom_db: f32) {
        self.headroom_db = headroom_db.clamp(0.0, 12.0);
    }

    pub fn headroom(&self) -> f32 {
        self.headroom_db
    }

    /// Configure attack/release time constants in seconds.
    pub fn set_smoothing(&mut self, attack_seconds: f32, release_seconds: f32) {
        self.attack_seconds = attack_seconds.clamp(0.001, 10.0);
        self.release_seconds = release_seconds.clamp(0.001, 10.0);
    }

    /// Calculate peak normalization gain and return `(gain, measured_peak)`.
    /// Finds the maximum absolute value and calculates gain to reach target level.
    fn calculate_peak_gain(buffer: &[f32], target_level: f32) -> (f32, f32) {
        let peak = buffer.iter().fold(0.0f32, |a, s| a.max(s.abs()));
        let gain = if peak > 1e-6 {
            target_level / peak
        } else {
            1.0
        };
        (gain, peak)
    }

    /// Update the 400 ms BS.1770 momentary window from interleaved audio.
    /// Every channel has independent filter state and channel powers are
    /// summed in the linear domain. Mono and stereo use weight 1.0.
    fn measure_momentary_lufs(&mut self, buffer: &[f32]) -> f32 {
        let channels = self.num_channels as usize;
        for frame in buffer.chunks_exact(channels) {
            let mut frame_energy = 0.0_f64;
            for (channel, (&sample, filter)) in
                frame.iter().zip(self.k_filters.iter_mut()).enumerate()
            {
                let filtered = filter.process(sample);
                frame_energy +=
                    f64::from(filtered * filtered) * f64::from(channel_weight(channel, channels));
            }

            let stored_energy = frame_energy as f32;
            let old = self.window_buf[self.window_pos];
            self.window_sum += f64::from(stored_energy - old);
            self.window_buf[self.window_pos] = stored_energy;

            self.window_pos += 1;
            if self.window_pos == self.window_size {
                self.window_pos = 0;
            }
            if self.window_filled < self.window_size {
                self.window_filled += 1;
            }
        }

        if self.window_filled == 0 {
            return SILENCE_LUFS;
        }
        let mean_square = (self.window_sum / self.window_filled as f64).max(0.0);
        if mean_square < 1e-12 {
            SILENCE_LUFS
        } else {
            (-0.691 + 10.0 * mean_square.log10()) as f32
        }
    }

    fn gain_from_lufs(measured_lufs: f32, target_level_db: f32) -> f32 {
        if measured_lufs <= SILENCE_LUFS {
            return 1.0;
        }
        let gain_db = target_level_db - measured_lufs;
        10.0f32.powf(gain_db / 20.0)
    }

    /// Process a chunk of audio with the current normalization settings.
    pub fn process(&mut self, buffer: &mut [f32]) {
        if buffer.is_empty() {
            return;
        }

        let input_peak = buffer
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        let (raw_gain, measured_level) = match self.mode {
            NormalizationMode::Peak => Self::calculate_peak_gain(buffer, self.peak_target),
            NormalizationMode::Loudness => {
                let lufs = self.measure_momentary_lufs(buffer);
                let gain = Self::gain_from_lufs(lufs, self.loudness_target_lufs);
                (gain, lufs)
            }
        };
        self.measured_level
            .store(f32::to_bits(measured_level), Ordering::Relaxed);

        let frames = (buffer.len() / self.num_channels as usize).max(1);
        let elapsed = frames as f32 / self.sample_rate as f32;
        let time_constant = if raw_gain < self.smoothed_gain {
            self.attack_seconds
        } else {
            self.release_seconds
        };
        let coeff = 1.0 - (-elapsed / time_constant).exp();
        self.smoothed_gain = coeff * raw_gain + (1.0 - coeff) * self.smoothed_gain;

        // Smoothing must never defeat the current buffer's peak ceiling. In
        // Loudness mode headroom is a sample-peak ceiling, not a subtraction
        // from the selected LUFS target.
        let output_ceiling = match self.mode {
            NormalizationMode::Peak => self.peak_target,
            NormalizationMode::Loudness => 10.0_f32.powf(-self.headroom_db / 20.0),
        };
        let peak_safe_gain = if input_peak > 1e-6 {
            output_ceiling / input_peak
        } else {
            MAX_GAIN
        };
        let safe_gain = self
            .smoothed_gain
            .clamp(MIN_GAIN, MAX_GAIN)
            .min(peak_safe_gain);
        self.smoothed_gain = safe_gain;

        self.gain.store(f32::to_bits(safe_gain), Ordering::Relaxed);

        for sample in buffer.iter_mut() {
            *sample *= safe_gain;
        }
    }

    /// Get the current applied gain (for monitoring/UI).
    pub fn current_gain(&self) -> f32 {
        f32::from_bits(self.gain.load(Ordering::Relaxed))
    }

    /// Get current gain in dB (for display purposes).
    pub fn current_gain_db(&self) -> f32 {
        20.0 * self.current_gain().log10()
    }

    /// Linear peak amplitude in Peak mode, momentary LUFS in Loudness mode.
    pub fn last_measured_level(&self) -> f32 {
        f32::from_bits(self.measured_level.load(Ordering::Relaxed))
    }
}

/// Channel weights for layouts Audido can identify from channel count.
/// Mono/stereo are exact. Five-channel and conventional 5.1 ordering use the
/// BS.1770 surround weights and exclude LFE; unknown layouts fall back to 1.0.
fn channel_weight(channel: usize, channels: usize) -> f32 {
    match (channels, channel) {
        (5, 3 | 4) => 1.41,
        (6, 3) => 0.0,
        (6, 4 | 5) => 1.41,
        _ => 1.0,
    }
}

impl Default for Normalizer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_sample(phase: &mut f32, step: f32) -> f32 {
        let sample = phase.sin();
        *phase += step;
        sample
    }

    fn feed_997_hz(normalizer: &mut Normalizer, channels: u16, active_channels: usize) -> f32 {
        let sample_rate = normalizer.sample_rate();
        let step = 2.0 * std::f32::consts::PI * 997.0 / sample_rate as f32;
        let frames_per_chunk = (sample_rate / 100) as usize;
        let mut phase = 0.0;

        for _ in 0..200 {
            let mut buffer = Vec::with_capacity(frames_per_chunk * channels as usize);
            for _ in 0..frames_per_chunk {
                let sample = sine_sample(&mut phase, step);
                for channel in 0..channels as usize {
                    buffer.push(if channel < active_channels {
                        sample
                    } else {
                        0.0
                    });
                }
            }
            normalizer.process(&mut buffer);
        }

        normalizer.last_measured_level()
    }

    #[test]
    fn generated_48k_filter_matches_bs1770_coefficients() {
        let (stage1, stage2) = k_weighting_biquads(48_000.0);

        assert!((stage1.b0 - 1.535_124_9).abs() < 1e-6);
        assert!((stage1.b1 - -2.691_696_2).abs() < 1e-6);
        assert!((stage1.b2 - 1.198_392_8).abs() < 1e-6);
        assert!((stage1.a1 - -1.690_659_3).abs() < 1e-6);
        assert!((stage1.a2 - 0.732_480_76).abs() < 1e-6);
        assert!((stage2.a1 - -1.990_047_5).abs() < 1e-6);
        assert!((stage2.a2 - 0.990_072_25).abs() < 1e-6);
    }

    #[test]
    fn peak_mode_ramps_toward_target_without_snapping() {
        let mut n = Normalizer::new();
        let mut buf = vec![0.45; 480];
        n.process(&mut buf);
        assert!(n.current_gain() < 2.0);
        assert!(n.current_gain() > 1.0);
        for _ in 0..1_000 {
            let mut buf = vec![0.45; 480];
            n.process(&mut buf);
        }
        assert!((n.current_gain() - 2.0).abs() < 0.05);
    }

    #[test]
    fn loudness_mode_does_not_fade_in_from_zero() {
        let mut n = Normalizer::new();
        n.set_mode(NormalizationMode::Loudness);
        let mut buf = vec![0.1f32; 256];
        n.process(&mut buf);
        // With the old bug (smoothed_gain starting at 0.0) the very first
        // buffer would be multiplied by a gain near 0, silencing it.
        assert!(n.current_gain() > 0.5);
    }

    #[test]
    fn silence_does_not_blow_up_gain() {
        let mut n = Normalizer::new();
        n.set_mode(NormalizationMode::Loudness);
        for _ in 0..100 {
            n.process(&mut vec![0.0; 480]);
        }
        assert_eq!(n.current_gain(), 1.0);
    }

    #[test]
    fn mode_switch_preserves_targets_in_their_own_units() {
        let mut n = Normalizer::new();
        n.set_target_level(0.75);
        n.set_mode(NormalizationMode::Loudness);
        n.set_target_level(-23.0);
        n.set_mode(NormalizationMode::Peak);
        assert_eq!(n.target_level(), 0.75);
        n.set_mode(NormalizationMode::Loudness);
        assert_eq!(n.target_level(), -23.0);
        assert_eq!(n.current_gain(), 1.0);
    }

    #[test]
    fn momentary_lufs_matches_known_reference_tone() {
        let mut n = Normalizer::with_sample_rate(48_000);
        n.set_mode(NormalizationMode::Loudness);
        let lufs = feed_997_hz(&mut n, 1, 1);

        assert!((lufs - -3.01).abs() < 0.05, "measured {lufs} LUFS");
    }

    #[test]
    fn stereo_channels_have_independent_filter_state_and_sum_power() {
        let mut left_only = Normalizer::with_format(48_000, 2);
        left_only.set_mode(NormalizationMode::Loudness);
        let left_only_lufs = feed_997_hz(&mut left_only, 2, 1);

        let mut dual_mono = Normalizer::with_format(48_000, 2);
        dual_mono.set_mode(NormalizationMode::Loudness);
        let dual_mono_lufs = feed_997_hz(&mut dual_mono, 2, 2);

        assert!((left_only_lufs - -3.01).abs() < 0.05);
        assert!(dual_mono_lufs.abs() < 0.05);
    }

    #[test]
    fn peak_smoothing_never_allows_current_buffer_to_clip() {
        let mut n = Normalizer::new();
        for _ in 0..1_000 {
            n.process(&mut vec![0.09; 480]);
        }
        assert!(n.current_gain() > 3.0);

        let mut transient = vec![1.0; 480];
        n.process(&mut transient);
        assert!(transient.iter().all(|sample| sample.abs() <= 0.900_001));
    }

    #[test]
    fn loudness_headroom_is_enforced_as_sample_peak_ceiling() {
        let mut n = Normalizer::new();
        n.set_mode(NormalizationMode::Loudness);
        n.set_headroom(3.0);

        let mut transient = vec![1.0; 480];
        n.process(&mut transient);
        let ceiling = 10.0_f32.powf(-3.0 / 20.0);
        assert!(
            transient
                .iter()
                .all(|sample| sample.abs() <= ceiling + 1e-6)
        );
    }

    #[test]
    fn clone_exposes_realtime_gain_and_meter_values() {
        let mut realtime = Normalizer::new();
        let monitor = realtime.clone();
        realtime.process(&mut vec![0.45; 480]);

        assert_eq!(monitor.current_gain(), realtime.current_gain());
        assert_eq!(monitor.last_measured_level(), 0.45);
    }
}
