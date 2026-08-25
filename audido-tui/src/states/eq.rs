use audido_core::dsp::eq::{EqPreset, FilterNode};

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum EqMode {
    Casual,
    Advanced,
}

#[derive(Debug, Clone)]
pub struct EqState {
    pub eq_enabled: bool,
    pub eq_mode: EqMode,
    // Local copy of filters for immediate UI feedback before sending to Engine
    pub local_filters: Vec<FilterNode>,
    pub local_preset: EqPreset,
    pub local_master_gain: f32,
    pub local_num_channels: u16,
}

impl EqState {
    const MIN_MASTER_GAIN_DB: f32 = -12.0;
    const MAX_MASTER_GAIN_DB: f32 = 12.0;

    pub fn new() -> Self {
        Self {
            eq_enabled: false,
            eq_mode: EqMode::Casual,

            local_filters: EqPreset::default().set_filters(),
            local_preset: EqPreset::default(),
            local_master_gain: 0.0,
            local_num_channels: 2, // Default to stereo
        }
    }

    /// Toggle EQ enabled state
    pub fn toggle_enabled(&mut self) {
        self.eq_enabled = !self.eq_enabled;
    }

    /// Toggle between Casual and Advanced mode
    pub fn toggle_mode(&mut self) {
        self.eq_mode = match self.eq_mode {
            EqMode::Casual => EqMode::Advanced,
            EqMode::Advanced => EqMode::Casual,
        };
    }

    /// Adjust the master gain while keeping it inside the range supported by the UI.
    pub fn adjust_master_gain(&mut self, delta_db: f32) {
        self.local_master_gain = (self.local_master_gain + delta_db)
            .clamp(Self::MIN_MASTER_GAIN_DB, Self::MAX_MASTER_GAIN_DB);
    }

    /// Format the master gain consistently wherever it is displayed.
    pub fn master_gain_label(&self) -> String {
        format!("{:+.1} dB", self.local_master_gain)
    }
}

#[cfg(test)]
mod tests {
    use super::EqState;

    #[test]
    fn master_gain_is_clamped_and_formatted() {
        let mut state = EqState::new();

        assert_eq!(state.master_gain_label(), "+0.0 dB");

        state.adjust_master_gain(20.0);
        assert_eq!(state.master_gain_label(), "+12.0 dB");

        state.adjust_master_gain(-30.0);
        assert_eq!(state.master_gain_label(), "-12.0 dB");
    }
}
