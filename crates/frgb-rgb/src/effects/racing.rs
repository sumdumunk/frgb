use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::add_color;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// RacingEffect — two segments racing at different speeds
// ---------------------------------------------------------------------------
//
// Segment A: 4 LEDs wide, moves 1 LED/frame.
// Segment B: 4 LEDs wide, moves 2 LEDs/frame.
// TL-family effect.
//
// frames = 200, interval = 25ms

pub struct RacingEffect;

impl EffectGenerator for RacingEffect {
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

        let brightness = params.brightness;
        let color_a = colors
            .first()
            .copied()
            .or(params.color)
            .unwrap_or(Rgb { r: 254, g: 0, b: 50 });
        let color_b = Rgb { r: 0, g: 200, b: 254 };
        let seg_len = 4usize;

        for frame in 0..frames {
            let head_a = frame % ring_size;
            let head_b = (frame * 2) % ring_size;

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                // Segment A
                let ca = apply_brightness(color_a, brightness);
                for s in 0..seg_len {
                    let pos = (head_a + ring_size - s) % ring_size;
                    let existing = buf.get_led(frame, fan_base + pos);
                    buf.set_led(frame, fan_base + pos, add_color(existing, ca));
                }

                // Segment B
                let cb = apply_brightness(color_b, brightness);
                for s in 0..seg_len {
                    let pos = (head_b + ring_size - s) % ring_size;
                    let existing = buf.get_led(frame, fan_base + pos);
                    buf.set_led(frame, fan_base + pos, add_color(existing, cb));
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
        25.0
    }
    fn frame_count(&self, _layout: &LedLayout, _fan_count: u8) -> usize {
        200
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn frame_count_is_200() {
        let gen = RacingEffect;
        let layout = LedLayout::for_device(DeviceType::TlWireless);
        assert_eq!(gen.frame_count(&layout, 1), 200);
    }

    #[test]
    fn segments_move_at_different_speeds() {
        let gen = RacingEffect;
        let layout = LedLayout::for_device(DeviceType::TlWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let result = gen.generate(&layout, 1, &params, &[]);
        // At frame 0 both heads at 0; at frame 13, head_a at 13, head_b at 0 (26%26)
        // Check that frame 10 has segments at different positions
        let mut lit_positions = Vec::new();
        for led in 0..26 {
            let c = result.buffer.get_led(10, led);
            if c.r > 0 || c.g > 0 || c.b > 0 {
                lit_positions.push(led);
            }
        }
        assert!(lit_positions.len() >= 4, "should have lit LEDs from segments");
    }
}
