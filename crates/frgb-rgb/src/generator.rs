use crate::buffer::RgbBuffer;
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// EffectResult
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct EffectResult {
    pub buffer: RgbBuffer,
    pub frame_count: usize,
    pub interval_ms: f32,
}

// ---------------------------------------------------------------------------
// EffectGenerator trait
// ---------------------------------------------------------------------------

pub trait EffectGenerator {
    fn generate(&self, layout: &LedLayout, fan_count: u8, params: &EffectParams, colors: &[Rgb]) -> EffectResult;
    fn interval_base(&self) -> f32;
    fn frame_count(&self, layout: &LedLayout, fan_count: u8) -> usize;
}
