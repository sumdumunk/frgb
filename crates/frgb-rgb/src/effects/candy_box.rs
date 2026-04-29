use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::rainbow_color_at;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// CandyBoxEffect — multi-colored segments rotating like a candy wheel
// ---------------------------------------------------------------------------
//
// ring divided into colorful candy-stripe segments
// that rotate continuously. Each segment gets a different rainbow color.
//
// frames = total_per_fan * 2

pub struct CandyBoxEffect;

impl EffectGenerator for CandyBoxEffect {
    fn generate(&self, layout: &LedLayout, fan_count: u8, params: &EffectParams, _colors: &[Rgb]) -> EffectResult {
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

        let brightness = params.brightness;
        let seg_size = (ring_size / 6).max(2);
        let num_segments = ring_size / seg_size;

        for frame in 0..frames {
            let offset = frame % ring_size;

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                for led in 0..ring_size {
                    let pos = (led + offset) % ring_size;
                    let seg = pos / seg_size;
                    let color = rainbow_color_at(seg, num_segments);
                    let c = apply_brightness(color, brightness);
                    buf.set_led(frame, fan_base + led, c);
                }
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
        layout.total_per_fan as usize * 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn all_leds_lit() {
        let gen = CandyBoxEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let result = gen.generate(&layout, 1, &params, &[]);

        let lit: usize = (0..24)
            .filter(|&l| {
                let c = result.buffer.get_led(0, l);
                c.r > 0 || c.g > 0 || c.b > 0
            })
            .count();
        assert_eq!(lit, 24, "candy box should fill entire ring");
    }

    #[test]
    fn has_multiple_colors() {
        let gen = CandyBoxEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let result = gen.generate(&layout, 1, &params, &[]);

        let mut unique_colors = std::collections::HashSet::new();
        for l in 0..24 {
            let c = result.buffer.get_led(0, l);
            unique_colors.insert((c.r / 30, c.g / 30, c.b / 30));
        }
        assert!(unique_colors.len() >= 3, "should have multiple color segments");
    }
}
