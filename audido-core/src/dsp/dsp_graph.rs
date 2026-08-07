use crate::dsp::eq::{Equalizer, FilterNode};

pub struct DspNode<T> {
    pub on: bool,
    pub instance: T,
}

impl<T> DspNode<T> {
    pub fn new(instance: T) -> Self {
        Self {
            on: false,
            instance,
        }
    }

    /// Create a new DspNode with an initial enabled state
    pub fn new_with_state(instance: T, on: bool) -> Self {
        Self { on, instance }
    }
}

// Specialized methods for DspNode<Equalizer>
impl DspNode<Equalizer> {
    pub fn set_filter(&mut self, idx: usize, node: FilterNode) {
        if idx < self.instance.filters.len() {
            self.instance.filters[idx] = node;
            self.instance.parameters_changed();
        }
    }

    pub fn set_all_filters(&mut self, nodes: Vec<FilterNode>) {
        self.instance.filters = nodes;
        self.instance.parameters_changed();
    }

    pub fn set_master_gain(&mut self, gain: f32) {
        self.instance.master_gain = gain;
    }
}

// pub struct DspGraph<T> {
//     nodes: Vec<DspNode<T>>,
// }
