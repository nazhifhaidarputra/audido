use audido_core::{
    dsp::normalization::{
        DEFAULT_HEADROOM_DB, DEFAULT_LOUDNESS_TARGET_LUFS, DEFAULT_PEAK_TARGET, NormalizationMode,
    },
    modules::{core::CoreHandle, normalizer},
};

#[derive(Debug, Clone)]
pub struct NormalizerState {
    pub enabled: bool,
    pub mode: NormalizationMode,
    pub peak_target: f32,
    pub loudness_target_lufs: f32,
    pub headroom_db: f32,
    pub current_gain_db: f32,
    pub measured_level: f32,
}

impl NormalizerState {
    pub fn new() -> Self {
        Self {
            enabled: false,
            mode: NormalizationMode::Peak,
            peak_target: DEFAULT_PEAK_TARGET,
            loudness_target_lufs: DEFAULT_LOUDNESS_TARGET_LUFS,
            headroom_db: DEFAULT_HEADROOM_DB,
            current_gain_db: 0.0,
            measured_level: 0.0,
        }
    }

    pub fn toggle_enabled(&mut self) {
        self.enabled = !self.enabled;
    }

    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            NormalizationMode::Peak => NormalizationMode::Loudness,
            NormalizationMode::Loudness => NormalizationMode::Peak,
        };
        self.current_gain_db = 0.0;
        self.measured_level = match self.mode {
            NormalizationMode::Peak => 0.0,
            NormalizationMode::Loudness => -70.0,
        };
    }

    pub fn adjust_target(&mut self, increase: bool) {
        match self.mode {
            NormalizationMode::Peak => {
                let delta = if increase { 0.01 } else { -0.01 };
                self.peak_target = (self.peak_target + delta).clamp(0.1, 1.0);
            }
            NormalizationMode::Loudness => {
                let delta = if increase { 0.5 } else { -0.5 };
                self.loudness_target_lufs = (self.loudness_target_lufs + delta).clamp(-40.0, 0.0);
            }
        }
    }

    pub fn adjust_headroom(&mut self, increase: bool) {
        let delta = if increase { 0.5 } else { -0.5 };
        self.headroom_db = (self.headroom_db + delta).clamp(0.0, 12.0);
    }

    pub fn target_level(&self) -> f32 {
        match self.mode {
            NormalizationMode::Peak => self.peak_target,
            NormalizationMode::Loudness => self.loudness_target_lufs,
        }
    }

    pub fn target_label(&self) -> String {
        match self.mode {
            NormalizationMode::Peak => format!("{:.0}% full scale", self.peak_target * 100.0),
            NormalizationMode::Loudness => format!("{:.1} LUFS", self.loudness_target_lufs),
        }
    }

    pub fn measured_label(&self) -> String {
        match self.mode {
            NormalizationMode::Peak => {
                if self.measured_level <= 1e-6 {
                    "-∞ dBFS".to_string()
                } else {
                    format!("{:+.1} dBFS", 20.0 * self.measured_level.log10())
                }
            }
            NormalizationMode::Loudness => format!("{:.1} LUFS", self.measured_level),
        }
    }

    pub fn refresh_meter(&mut self, handle: &CoreHandle) {
        let meter = normalizer::meter(&handle.ctx);
        self.current_gain_db = meter.current_gain_db;
        self.measured_level = meter.measured_level;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_are_adjusted_and_formatted_in_mode_specific_units() {
        let mut state = NormalizerState::new();
        assert_eq!(state.target_label(), "90% full scale");

        state.toggle_mode();
        assert_eq!(state.target_label(), "-18.0 LUFS");
        state.adjust_target(true);
        assert_eq!(state.target_level(), -17.5);

        state.toggle_mode();
        assert_eq!(state.target_level(), 0.9);
    }
}
