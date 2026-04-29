use crate::buffer::RgbBuffer;
use crate::effects::common::paint_meteor_tail_max;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// DoubleMeteorEffect — two meteors chasing in opposite directions
// ---------------------------------------------------------------------------

pub struct DoubleMeteorEffect;

impl EffectGenerator for DoubleMeteorEffect {
    fn generate(&self, layout: &LedLayout, fan_count: u8, params: &EffectParams, colors: &[Rgb]) -> EffectResult {
        let frames = self.frame_count(layout, fan_count);
        let total_leds = layout.total_leds(fan_count);
        let mut buf = RgbBuffer::new(frames, total_leds);

        let ring_size = layout.total_per_fan as usize;
        if ring_size == 0 {
            return EffectResult {
                buffer: buf,
                frame_count: frames,
                interval_ms: self.interval_base(),
            };
        }

        let color = colors.first().copied().unwrap_or(Rgb { r: 254, g: 0, b: 0 });
        let brightness = params.brightness;

        for frame in 0..frames {
            let offset = frame % ring_size;

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                let head_cw = offset;
                let head_ccw = (ring_size - 1 - offset) % ring_size;

                // CW meteor (tail extends backward = ccw=false)
                paint_meteor_tail_max(&mut buf, frame, fan_base, head_cw, ring_size, false, color, brightness);
                // CCW meteor (tail extends forward = ccw=true)
                paint_meteor_tail_max(&mut buf, frame, fan_base, head_ccw, ring_size, true, color, brightness);
            }
        }

        EffectResult {
            buffer: buf,
            frame_count: frames,
            interval_ms: self.interval_base(),
        }
    }

    fn interval_base(&self) -> f32 {
        20.0
    }

    fn frame_count(&self, layout: &LedLayout, _fan_count: u8) -> usize {
        layout.total_per_fan as usize * 3
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn double_meteor_frame_count() {
        let gen = DoubleMeteorEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 72);
    }

    #[test]
    fn double_meteor_has_two_lit_regions() {
        let gen = DoubleMeteorEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let lit: Vec<usize> = (0..24).filter(|&l| result.buffer.get_led(6, l).r > 0).collect();
        assert!(
            lit.len() >= 5,
            "should have at least 5 lit LEDs from two meteors, got {}",
            lit.len()
        );
    }
}
