use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::bounce_pos;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// DuelEffect — two colors battle for territory across the ring
// ---------------------------------------------------------------------------
//
// two colors occupy opposite halves of the ring.
// The boundary between them oscillates back and forth as if the colors
// are fighting for dominance.
//
// frames = total_per_fan * 4

pub struct DuelEffect;

impl EffectGenerator for DuelEffect {
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
        let c1 = apply_brightness(color1, params.brightness);
        let c2 = apply_brightness(color2, params.brightness);
        let half = ring_size / 2;
        let swing = half / 3; // how far the boundary swings

        for frame in 0..frames {
            // Oscillating boundary
            let offset = bounce_pos(frame, swing);
            let boundary = half - swing / 2 + offset;

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;
                for led in 0..ring_size {
                    let c = if led < boundary { c1 } else { c2 };
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
        layout.total_per_fan as usize * 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn has_two_color_regions() {
        let gen = DuelEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }, Rgb { r: 0, g: 0, b: 254 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let has_red = (0..24).any(|l| result.buffer.get_led(0, l).r > 100);
        let has_blue = (0..24).any(|l| result.buffer.get_led(0, l).b > 100);
        assert!(has_red && has_blue, "should have both color territories");
    }
}
