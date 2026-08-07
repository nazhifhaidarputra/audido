// Implementation for Parametric EQ
// The algorithm is based on RBJ Audio EQ Cookbook

use core::f32;
use std::f32::consts::PI;

use strum::{EnumIter, IntoEnumIterator};

pub const MAX_EQ_FILTERS: usize = 8;

/// Filter type: Use Direct Form II Biquad Filter
#[derive(Default, Debug, Clone, Copy, PartialEq, EnumIter, strum::Display)]
pub enum FilterType {
    #[default]
    Peaking,
    LowPass,
    HighPass,
    LowShelf,
    HighShelf,
    BandPass,
    Notch,
}

impl FilterType {
    pub fn next(&self) -> FilterType {
        let mut modes = FilterType::iter();
        for mode in modes.by_ref() {
            if mode == *self {
                break;
            }
        }
        modes.next().unwrap_or(FilterType::Peaking)
    }

    pub fn prev(&self) -> FilterType {
        let modes: Vec<FilterType> = FilterType::iter().collect();
        let len = modes.len();
        for (i, mode) in modes.iter().enumerate() {
            if *mode == *self {
                if i == 0 {
                    return modes[len - 1];
                } else {
                    return modes[i - 1];
                }
            }
        }
        FilterType::Peaking
    }
}

#[derive(Clone, Debug)]
pub struct FilterNode {
    pub id: i16,
    pub filter_type: FilterType,
    /// cutoff frequence of the filter (in Hz)
    pub freq: f32,
    /// Filter gain in dB
    pub gain: f32,
    /// Filter Q Factor/resonance
    pub q: f32,
    /// Filter order (1 = 6dB/oct, 2 = 12dB/oct, 4 = 24dB/oct, etc)
    pub order: u8,
}

impl FilterNode {
    pub fn new(id: i16, freq: f32) -> Self {
        Self {
            id,
            filter_type: FilterType::Peaking,
            freq,
            gain: 0.0,
            q: 0.707,
            order: 2,
        }
    }

    pub fn magnitude_db(&self, frequency_hz: f32, sample_rate: f32) -> f32 {
        // ensure that frequency is not below zero or greater than nyquist frequency
        if frequency_hz <= 0.0 || frequency_hz >= sample_rate / 2.0 {
            return 0.0;
        }

        let w0 = 2.0 * PI * self.freq / sample_rate;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * self.q);
        let a_linear = 10.0f32.powf(self.gain / 40.0);

        let (b0, b1, b2, a0, a1, a2) =
            Biquad::calculate_coefficients(cos_w0, alpha, a_linear, self.filter_type);

        // Evaluate Transfer Function H(z) at z = e^(jw)
        // w (omega) for the target frequency
        let w = 2.0 * PI * frequency_hz / sample_rate;
        let cos_w = w.cos();
        let cos_2w = (2.0 * w).cos();
        let sin_w = w.sin();
        let sin_2w = (2.0 * w).sin();

        // Numerator (b part) real and imag
        let num_r = b0 + b1 * cos_w + b2 * cos_2w;
        let num_i = b1 * sin_w + b2 * sin_2w;

        // Denominator (a part) real and imag
        let den_r = a0 + a1 * cos_w + a2 * cos_2w;
        let den_i = a1 * sin_w + a2 * sin_2w;

        let mag_sq = (num_r * num_r + num_i * num_i) / (den_r * den_r + den_i * den_i);

        // Convert to dB: 10 * log10(mag_sq) which is 20 * log10(mag)
        let single_biquad_db = 10.0 * mag_sq.log10();

        // Account for cascaded biquads: order N uses ceil(N/2) identical biquad sections.
        // In dB, cascading N sections multiplies the single-section dB by N.
        let num_biquads = (self.order as f32 / 2.0).ceil().max(1.0);
        single_biquad_db * num_biquads
    }

    pub fn set_filter_type(&mut self, filter_type: FilterType) {
        self.filter_type = filter_type;
    }

    pub fn set_freq(&mut self, freq: f32) {
        self.freq = freq.clamp(20.0, 20000.0);
    }

    pub fn set_gain(&mut self, gain: f32) {
        self.gain = gain.clamp(-20.0, 20.0);
    }

    pub fn set_order(&mut self, order: u8) -> u8 {
        let new_order = order.clamp(1, 16);
        self.order = new_order;
        new_order
    }

    pub fn set_q_factor(&mut self, q: f32) {
        self.q = q.clamp(0.1, 10.0);
    }

    /// Reset this filter node to default parameter values, preserving its id
    pub fn reset(&mut self) {
        let id = self.id;
        *self = Self::default();
        self.id = id;
    }
}

impl Default for FilterNode {
    fn default() -> Self {
        Self {
            id: 0,
            filter_type: FilterType::Peaking,
            freq: 1000.0,
            gain: 0.0,
            q: 0.707,
            order: 2,
        }
    }
}

/// implement Biquad Filter (Direct Form II Transposed)
/// $$ y[n] = frac{b0/a0}x[n] + frac{b1/a0}x[n-1] + frac{b2/a0}x[n-2] - frac{a1/a0}y[n-1] - frac{a2/a0}y[n-2] $$
#[derive(Clone, Default, Debug)]
struct Biquad {
    // Coefficients
    a1: f32,
    a2: f32,
    b0: f32,
    b1: f32,
    b2: f32,
    // Previous State
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn process(&mut self, sample: f32) -> f32 {
        // Direct Form II Transposed difference equation
        // y[n] = b0*x[n] + z1[n-1]
        // z1[n] = b1*x[n] - a1*y[n] + z2[n-1]
        // z2[n] = b2*x[n] - a2*y[n]

        let out = self.b0 * sample + self.z1;
        self.z1 = self.b1 * sample - self.a1 * out + self.z2;
        self.z2 = self.b2 * sample - self.a2 * out;

        out
    }

    /// Recalculate coefficients
    fn update(&mut self, filter: &FilterNode, sample_rate: f32) {
        let w0 = 2.0 * PI * filter.freq / sample_rate;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / (2.0 * filter.q);

        // amplitude in linear scale (converted from dB)
        // A = 10^(Adb / 40.0)
        let a = 10.0f32.powf(filter.gain / 40.0);

        let (b0, b1, b2, a0, a1, a2) =
            Self::calculate_coefficients(cos_w0, alpha, a, filter.filter_type);

        // Normalize coefficients by a0
        let inv_a0 = 1.0 / a0;
        self.b0 = b0 * inv_a0;
        self.b1 = b1 * inv_a0;
        self.b2 = b2 * inv_a0;
        self.a1 = a1 * inv_a0;
        self.a2 = a2 * inv_a0;
    }

    /// Helper to calculate (b0, b1, b2, a0, a1, a2)
    /// This code is adapted from RBJ Audio EQ Cookbook
    fn calculate_coefficients(
        cos_w0: f32,
        alpha: f32,
        a: f32,
        filter_type: FilterType,
    ) -> (f32, f32, f32, f32, f32, f32) {
        match filter_type {
            FilterType::Peaking => (
                1.0 + alpha * a,
                -2.0 * cos_w0,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos_w0,
                1.0 - alpha / a,
            ),
            FilterType::LowPass => (
                (1.0 - cos_w0) / 2.0,
                1.0 - cos_w0,
                (1.0 - cos_w0) / 2.0,
                1.0 + alpha,
                -2.0 * cos_w0,
                1.0 - alpha,
            ),
            FilterType::HighPass => (
                (1.0 + cos_w0) / 2.0,
                -(1.0 + cos_w0),
                (1.0 + cos_w0) / 2.0,
                1.0 + alpha,
                -2.0 * cos_w0,
                1.0 - alpha,
            ),
            FilterType::LowShelf => {
                let sqrt_a = a.sqrt();
                (
                    a * ((a + 1.0) - (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0),
                    a * ((a + 1.0) - (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha),
                    (a + 1.0) + (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0),
                    (a + 1.0) + (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha,
                )
            }
            FilterType::HighShelf => {
                let sqrt_a = a.sqrt();
                (
                    a * ((a + 1.0) + (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0),
                    a * ((a + 1.0) + (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha),
                    (a + 1.0) - (a - 1.0) * cos_w0 + 2.0 * sqrt_a * alpha,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos_w0),
                    (a + 1.0) - (a - 1.0) * cos_w0 - 2.0 * sqrt_a * alpha,
                )
            }
            FilterType::BandPass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos_w0, 1.0 - alpha),
            FilterType::Notch => (
                1.0,
                -2.0 * cos_w0,
                1.0,
                1.0 + alpha,
                -2.0 * cos_w0,
                1.0 - alpha,
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum EqPreset {
    #[default]
    Flat,
    Acoustic,
    Dance,
    Electronic,
    BassBoosted,
    Custom,
    // ...
}

fn create_flat_filters() -> Vec<FilterNode> {
    let mut filters = Vec::with_capacity(MAX_EQ_FILTERS);
    let freqs = [40.0, 200.0, 500.0, 1000., 2000., 5000., 10000., 15000.];
    for i in 0..MAX_EQ_FILTERS {
        filters.push(FilterNode {
            id: i as i16,
            filter_type: FilterType::Peaking,
            freq: *freqs.get(i).unwrap_or(&1000.0),
            gain: 0.0,
            q: 0.707,
            order: 2,
        });
    }
    filters
}

impl EqPreset {
    pub fn set_filters(&self) -> Vec<FilterNode> {
        match self {
            EqPreset::Flat => create_flat_filters(),
            EqPreset::Acoustic => create_flat_filters(),
            EqPreset::Dance => create_flat_filters(),
            EqPreset::Electronic => create_flat_filters(),
            EqPreset::BassBoosted => vec![FilterNode {
                id: 1,
                filter_type: FilterType::LowShelf,
                freq: 100.0,
                gain: 6.0,
                q: 0.707,
                order: 2,
            }],
            EqPreset::Custom => create_flat_filters(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Equalizer {
    pub sample_rate: u32,
    pub preset: EqPreset,
    pub filters: Vec<FilterNode>,
    /// Internal DSP state (vector of vector because one node can have multiple biquads for high order)
    processors: Vec<Vec<Vec<Biquad>>>, // [channel][filter][biquad]
    pub master_gain: f32,
    num_channels: u16,
}

impl Equalizer {
    pub fn new(sample_rate: u32, num_channels: u16) -> Self {
        let preset = EqPreset::Flat;
        let mut eq = Self {
            sample_rate,
            preset,
            filters: preset.set_filters(),
            processors: Vec::new(), // Initialized in rebuild
            master_gain: 1.0,
            num_channels,
        };
        // Initialize processors based on initial filters
        eq.rebuild_processors();
        eq
    }

    pub fn process_frame(&mut self, frame: &mut [f32]) {
        if (self.master_gain - 1.0).abs() > f32::EPSILON {
            for sample in frame.iter_mut() {
                *sample *= self.master_gain;
            }
        }

        let num_ch = self.num_channels as usize;
        if num_ch == 0 {
            return;
        }

        for (i, sample) in frame.iter_mut().enumerate() {
            let channel_idx = i % num_ch;

            // Access the processor chain for this specific channel
            if let Some(channel_filters) = self.processors.get_mut(channel_idx) {
                let mut s = *sample;

                // Pass the sample through every filter node in the chain
                for filter_biquads in channel_filters {
                    // Pass through every biquad (for high-order cascades)
                    for biquad in filter_biquads {
                        s = biquad.process(s);
                    }
                }
                *sample = s;
            }
        }
    }

    pub fn update_preset(&mut self, preset: EqPreset) {
        if preset != self.preset {
            self.preset = preset;
            let config = self.preset.set_filters();
            self.filters = config;

            // Rebuild the DSP processors to reflect the new config
            self.rebuild_processors();
        }
    }

    /// Rebuild the DSP processors. called when the parameter is changed
    fn rebuild_processors(&mut self) {
        self.processors.clear();

        for _ in 0..self.num_channels {
            let mut channel_chain = Vec::with_capacity(self.filters.len());
            for filter_node in &self.filters {
                // A standard Biquad is 2nd order (12dB/oct).
                // For order 4 (24dB/oct), we need 2 biquads.
                // For order 1 (6dB/oct), we technically need 0.5 biquads, but we treat it as order 2 with reduced slope logic

                let num_biquads = (filter_node.order as f32 / 2.0).ceil() as usize;
                let count = if num_biquads == 0 { 1 } else { num_biquads };

                let mut biquads = Vec::with_capacity(count);
                for _ in 0..count {
                    let mut bq = Biquad::default();
                    bq.update(filter_node, self.sample_rate as f32);
                    biquads.push(bq);
                }
                channel_chain.push(biquads);
            }
            self.processors.push(channel_chain);
        }
    }

    pub fn parameters_changed(&mut self) {
        // If the channel configuration doesn't match, we must do a full rebuild
        if self.processors.len() != self.num_channels as usize {
            self.rebuild_processors();
            return;
        }

        // Iterate over every channel to update its specific processors
        for channel_filters in &mut self.processors {
            // If the number of filters changed (e.g. added a band), rebuild
            if channel_filters.len() != self.filters.len() {
                self.rebuild_processors();
                return;
            }

            // Update filters inside this channel
            for (i, filter_node) in self.filters.iter().enumerate() {
                let biquad_chain = &mut channel_filters[i];

                // Handle Order Changes (resize chain while keeping state where possible)
                let required_biquads = (filter_node.order as f32 / 2.0).ceil() as usize;
                let count = if required_biquads == 0 {
                    1
                } else {
                    required_biquads
                };

                if biquad_chain.len() < count {
                    // Order increased: append new zero-state biquads
                    biquad_chain.resize_with(count, Biquad::default);
                } else if biquad_chain.len() > count {
                    // Order decreased: truncate but keep state of remaining
                    biquad_chain.truncate(count);
                }

                // Update coefficients for all biquads (preserves z1/z2)
                for biquad in biquad_chain.iter_mut() {
                    biquad.update(filter_node, self.sample_rate as f32);
                }
            }
        }
    }

    pub fn reset_parameters(&mut self) {
        self.filters = self.preset.set_filters();
        self.master_gain = 1.0;
        self.parameters_changed();
    }

    pub fn reset_filter_node_param(&mut self, node_index: usize) -> anyhow::Result<()> {
        let preset_filters = self.preset.set_filters();
        let default_node = preset_filters.get(node_index).cloned().unwrap_or_else(|| {
            let mut node = FilterNode::default();
            node.id = node_index as i16;
            node
        });

        let filter_node = self
            .filters
            .get_mut(node_index)
            .ok_or(anyhow::anyhow!("Filter node not found"))?;
        *filter_node = default_node;

        self.parameters_changed();
        Ok(())
    }

    /// Get the combined frequency response curve for plotting
    /// Returns Vector of (Frequency, Gain_dB) points
    pub fn get_response_curve(&self, width: usize) -> Vec<(f32, f32)> {
        let mut points = Vec::with_capacity(width);

        let start_freq: f32 = 20.0;
        let end_freq: f32 = 20000.0;
        let log_start = start_freq.ln();
        let log_end = end_freq.ln();
        let step = (log_end - log_start) / (width as f32 - 1.0);

        // convert master gain to dB
        let master_gain_db = 20.0 * (self.master_gain).log10();

        for i in 0..width {
            let log_f = log_start + step * i as f32;
            let f = log_f.exp();
            let mut total_db = master_gain_db;
            for filter in &self.filters {
                total_db += filter.magnitude_db(f, self.sample_rate as f32);
            }
            points.push((f, total_db));
        }

        points
    }
}
