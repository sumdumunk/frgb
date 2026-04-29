use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::rainbow_color_at;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// BlowUpEffect — explosion from center
// ---------------------------------------------------------------------------
//
// LEDs light outward from the middle of the ring. After full expansion
// (ring/2 frames), all LEDs fade out over 20 frames. Repeats 4 times
// with different rainbow hues.
//
// frames = (ring/2 + 20) * 4, interval = 35ms

pub struct BlowUpEffect;

impl EffectGenerator for BlowUpEffect {
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
        let half = ring_size / 2;
        let fade_frames = 20usize;
        let cycle_len = half + fade_frames;
        let repeats = 4usize;
        let center = ring_size / 2;

        for frame in 0..frames {
            let cycle = (frame / cycle_len).min(repeats - 1);
            let cycle_frame = frame - cycle * cycle_len;
            let color = rainbow_color_at(cycle, repeats);

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                if cycle_frame < half {
                    // Expansion phase: light LEDs outward from center
                    let radius = cycle_frame + 1;
                    for r in 0..radius {
                        let idx_a = (center + r) % ring_size;
                        let idx_b = (center + ring_size - r) % ring_size;
                        let c = apply_brightness(color, brightness);
                        buf.set_led(frame, fan_base + idx_a, c);
                        buf.set_led(frame, fan_base + idx_b, c);
                    }
                } else {
                    // Fade phase: all LEDs lit, fading out
                    let fade_step = cycle_frame - half;
                    let fade_factor = if fade_frames > 0 {
                        255 - (fade_step as u16 * 255 / fade_frames as u16).min(255) as u8
                    } else {
                        0
                    };
                    let c = apply_brightness(color, brightness);
                    let c = Rgb {
                        r: ((c.r as u16 * fade_factor as u16) >> 8) as u8,
                        g: ((c.g as u16 * fade_factor as u16) >> 8) as u8,
                        b: ((c.b as u16 * fade_factor as u16) >> 8) as u8,
                    };
                    for led in 0..ring_size {
                        buf.set_led(frame, fan_base + led, c);
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
        35.0
    }

    fn frame_count(&self, layout: &LedLayout, _fan_count: u8) -> usize {
        let ring = layout.total_per_fan as usize;
        if ring == 0 {
            1
        } else {
            (ring / 2 + 20) * 4
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
        let gen = BlowUpEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), (24 / 2 + 20) * 4);
    }

    #[test]
    fn center_lights_first() {
        let gen = BlowUpEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let result = gen.generate(&layout, 1, &params, &[]);
        // Frame 0: only center LED(s) should be lit
        let center = 12; // ring_size/2
        let c = result.buffer.get_led(0, center);
        assert!(c.r > 0 || c.g > 0 || c.b > 0, "center should be lit first");
        // Edge LEDs should be dark at frame 0
        let edge = result.buffer.get_led(0, 0);
        assert_eq!(edge, Rgb::BLACK, "edge should be dark initially");
    }
}
