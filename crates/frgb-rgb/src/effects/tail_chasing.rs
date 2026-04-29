use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::add_color;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// TailChasingEffect — two colored dots chasing around the ring
// ---------------------------------------------------------------------------
//
// Dot A leads, dot B at ring/3 offset behind. Both have 3-LED fading tails.
// TL-family effect.
//
// frames = ring * 3, interval = 35ms

pub struct TailChasingEffect;

impl EffectGenerator for TailChasingEffect {
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
            .unwrap_or(Rgb { r: 254, g: 50, b: 0 });
        let color_b = Rgb { r: 0, g: 100, b: 254 };
        let tail_len = 3usize;
        let offset = ring_size / 3;

        for frame in 0..frames {
            let head_a = frame % ring_size;
            let head_b = (frame + ring_size - offset) % ring_size;

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                // Dot A + tail
                for t in 0..=tail_len {
                    let pos = (head_a + ring_size - t) % ring_size;
                    let fade = 255u8 >> t;
                    let c = Rgb {
                        r: ((color_a.r as u16 * fade as u16) >> 8) as u8,
                        g: ((color_a.g as u16 * fade as u16) >> 8) as u8,
                        b: ((color_a.b as u16 * fade as u16) >> 8) as u8,
                    };
                    let c = apply_brightness(c, brightness);
                    let existing = buf.get_led(frame, fan_base + pos);
                    buf.set_led(frame, fan_base + pos, add_color(existing, c));
                }

                // Dot B + tail
                for t in 0..=tail_len {
                    let pos = (head_b + ring_size - t) % ring_size;
                    let fade = 255u8 >> t;
                    let c = Rgb {
                        r: ((color_b.r as u16 * fade as u16) >> 8) as u8,
                        g: ((color_b.g as u16 * fade as u16) >> 8) as u8,
                        b: ((color_b.b as u16 * fade as u16) >> 8) as u8,
                    };
                    let c = apply_brightness(c, brightness);
                    let existing = buf.get_led(frame, fan_base + pos);
                    buf.set_led(frame, fan_base + pos, add_color(existing, c));
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
        35.0
    }

    fn frame_count(&self, layout: &LedLayout, _fan_count: u8) -> usize {
        let ring = layout.total_per_fan as usize;
        if ring == 0 {
            1
        } else {
            ring * 3
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
        let gen = TailChasingEffect;
        let layout = LedLayout::for_device(DeviceType::TlWireless);
        assert_eq!(gen.frame_count(&layout, 1), 26 * 3);
    }

    #[test]
    fn two_dots_visible() {
        let gen = TailChasingEffect;
        let layout = LedLayout::for_device(DeviceType::TlWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let result = gen.generate(&layout, 1, &params, &[]);
        let mut lit = 0;
        for led in 0..26 {
            let c = result.buffer.get_led(0, led);
            if c.r > 0 || c.g > 0 || c.b > 0 {
                lit += 1;
            }
        }
        // 2 dots with 3-LED tails = up to 8 lit LEDs
        assert!((2..=10).contains(&lit), "expected 2-10 lit LEDs, got {}", lit);
    }
}
