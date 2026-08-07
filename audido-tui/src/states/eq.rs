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
}
