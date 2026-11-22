/// Outward camera zoom settings (capped at 1.0x native scale).
///
/// Callers clamp to a hardcoded `1.0` ceiling and keep smoothing enabled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zoom {
    pub enabled: bool,
    pub min: f32,
    pub step: f32,
    pub smooth_rate: f32,
}

impl Default for Zoom {
    fn default() -> Self {
        Self {
            enabled: true,
            min: 0.35,
            step: 1.10,
            smooth_rate: 12.5,
        }
    }
}
