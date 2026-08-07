pub struct NormalizerState {
    pub enabled: bool,   
}

impl NormalizerState {
    pub fn new() -> Self {
        Self {
            enabled: false,
        }
    }
}