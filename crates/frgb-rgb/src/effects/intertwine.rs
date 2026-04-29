use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::add_color;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// IntertwineEffect — two half-ring arcs rotating in opposite directions
// ---------------------------------------------------------------------------
//
// Arc A: clockwise (user color), Arc B: counter-clockwise (complementary).
// Each arc is half the ring. Where they overlap, colors blend additively.
// TL-family effect.
//
// frames = ring * 2, interval = 40ms

pub struct IntertwineEffect;

impl EffectGenerator for IntertwineEffect {
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
            .unwrap_or(Rgb { r: 254, g: 0, b: 100 });
        // Complementary color
        let color_b = Rgb {
            r: 254u8.saturating_sub(color_a.r),
            g: 254u8.saturating_sub(color_a.g),
            b: 254u8.saturating_sub(color_a.b),
        };
        let half = ring_size / 2;

        for frame in 0..frames {
            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                // Arc A: starts at position `frame`, extends half ring clockwise
                let start_a = frame % ring_size;
                let ca = apply_brightness(color_a, brightness);
                for i in 0..half {
                    let pos = (start_a + i) % ring_size;
                    let existing = buf.get_led(frame, fan_base + pos);
                    buf.set_led(frame, fan_base + pos, add_color(existing, ca));
                }

                // Arc B: starts at opposite side, moves counter-clockwise
                let start_b = (half + ring_size - frame % ring_size) % ring_size;
                let cb = apply_brightness(color_b, brightness);
                for i in 0..half {
                    let pos = (start_b + i) % ring_size;
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
        40.0
    }

    fn frame_count(&self, layout: &LedLayout, _fan_count: u8) -> usize {
        let ring = layout.total_per_fan as usize;
        if ring == 0 {
            1
        } else {
            ring * 2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn frame_count_formula() {
        let gen = IntertwineEffect;
        let layout = LedLayout::for_device(DeviceType::TlWireless);
        assert_eq!(gen.frame_count(&layout, 1), 26 * 2);
    }

    #[test]
    fn all_leds_lit() {
        let gen = IntertwineEffect;
        let layout = LedLayout::for_device(DeviceType::TlWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let result = gen.generate(&layout, 1, &params, &[]);
        // Two half-ring arcs should cover the entire ring
        for led in 0..26 {
            let c = result.buffer.get_led(0, led);
            assert!(c.r > 0 || c.g > 0 || c.b > 0, "LED {} should be lit", led);
        }
    }
}
