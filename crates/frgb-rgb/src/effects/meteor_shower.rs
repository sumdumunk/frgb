use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::{add_color, rainbow_color_at};
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// MeteorShowerEffect — multiple meteors at different speeds
// ---------------------------------------------------------------------------
//
// 3 meteors at different ring offsets, each with a 4-LED fading tail.
// Speeds: 1, 2, 3 LEDs/frame. Colors from rainbow spectrum.
//
// frames = 180, interval = 25ms

pub struct MeteorShowerEffect;

impl EffectGenerator for MeteorShowerEffect {
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
        let tail_len = 4usize;

        // 3 meteors: (start_offset, speed, color_index)
        let meteors = [
            (0usize, 1usize, 0usize),
            (ring_size / 3, 2usize, 85usize),
            (ring_size * 2 / 3, 3usize, 170usize),
        ];

        for frame in 0..frames {
            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;
                for &(start, speed, color_idx) in &meteors {
                    let head = (start + frame * speed) % ring_size;
                    let color = rainbow_color_at(color_idx, 256);

                    // Head + fading tail
                    for t in 0..=tail_len {
                        let pos = (head + ring_size - t) % ring_size;
                        let fade = if t == 0 { 255u8 } else { 255u8 >> t };
                        let c = Rgb {
                            r: ((color.r as u16 * fade as u16) >> 8) as u8,
                            g: ((color.g as u16 * fade as u16) >> 8) as u8,
                            b: ((color.b as u16 * fade as u16) >> 8) as u8,
                        };
                        let c = apply_brightness(c, brightness);
                        let existing = buf.get_led(frame, fan_base + pos);
                        buf.set_led(frame, fan_base + pos, add_color(existing, c));
                    }
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
        180
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn frame_count_is_180() {
        let gen = MeteorShowerEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 180);
    }

    #[test]
    fn has_lit_pixels() {
        let gen = MeteorShowerEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let result = gen.generate(&layout, 1, &params, &[]);
        let mut lit = 0;
        for led in 0..24 {
            let c = result.buffer.get_led(0, led);
            if c.r > 0 || c.g > 0 || c.b > 0 {
                lit += 1;
            }
        }
        assert!(lit >= 3, "should have at least 3 meteor heads lit, got {}", lit);
    }
}
