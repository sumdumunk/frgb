use crate::buffer::RgbBuffer;
use crate::effects::common::{paint_meteor_tail_add, paint_meteor_tail_max};
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// MeteorMixEffect — two different-color meteors in opposite directions
// ---------------------------------------------------------------------------
//
// two meteors of different colors chase in opposite
// directions. When they cross, colors blend additively.
//
// frames = total_per_fan * 3

pub struct MeteorMixEffect;

impl EffectGenerator for MeteorMixEffect {
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

        let color1 = colors.first().copied().unwrap_or(Rgb { r: 254, g: 0, b: 0 });
        let color2 = if colors.len() > 1 {
            colors[1]
        } else {
            Rgb { r: 0, g: 0, b: 254 }
        };
        let brightness = params.brightness;

        for frame in 0..frames {
            let offset = frame % ring_size;

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                // Meteor 1: CW, color1 (max-blend)
                paint_meteor_tail_max(&mut buf, frame, fan_base, offset, ring_size, false, color1, brightness);
                // Meteor 2: CCW, color2 (additive blend for color mixing)
                let head = ring_size - 1 - offset;
                paint_meteor_tail_add(&mut buf, frame, fan_base, head, ring_size, true, color2, brightness);
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
    fn frame_count() {
        let gen = MeteorMixEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 72);
    }

    #[test]
    fn has_both_colors() {
        let gen = MeteorMixEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }, Rgb { r: 0, g: 0, b: 254 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let has_red = (0..24).any(|l| result.buffer.get_led(0, l).r > 100);
        let has_blue = (0..24).any(|l| result.buffer.get_led(0, l).b > 100);
        assert!(has_red, "should have red meteor");
        assert!(has_blue, "should have blue meteor");
    }
}
