use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::rainbow_color_at;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// CoverCycleEffect — sequential color fill cycling through hues
// ---------------------------------------------------------------------------
//
// Each frame advances the fill front by 1 LED. After filling the entire ring,
// the next hue begins filling. 6 passes through different rainbow hues.
//
// frames = ring_size * 6, interval = 40ms

pub struct CoverCycleEffect;

impl EffectGenerator for CoverCycleEffect {
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
        let passes = 6usize;

        for frame in 0..frames {
            let pass = frame / ring_size;
            let fill_count = (frame % ring_size) + 1;
            let color = rainbow_color_at(pass, passes);
            let color = apply_brightness(color, brightness);

            // Previous pass color (for already-filled LEDs behind fill front)
            let prev_color = if pass > 0 {
                let pc = rainbow_color_at(pass - 1, passes);
                apply_brightness(pc, brightness)
            } else {
                Rgb::BLACK
            };

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;
                // LEDs behind the fill front show current pass color
                for led in 0..fill_count {
                    buf.set_led(frame, fan_base + led, color);
                }
                // LEDs ahead of fill front show previous pass color
                for led in fill_count..ring_size {
                    buf.set_led(frame, fan_base + led, prev_color);
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
            ring * 6
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn frame_count_matches_ring_times_6() {
        let gen = CoverCycleEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 24 * 6);
    }

    #[test]
    fn first_frame_has_one_lit_led() {
        let gen = CoverCycleEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let result = gen.generate(&layout, 1, &params, &[]);
        let c = result.buffer.get_led(0, 0);
        assert!(c.r > 0 || c.g > 0 || c.b > 0, "first LED should be lit");
    }

    #[test]
    fn fill_progresses() {
        let gen = CoverCycleEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let result = gen.generate(&layout, 1, &params, &[]);
        // At frame 12, 13 LEDs should be the current hue
        let c0 = result.buffer.get_led(12, 0);
        let c12 = result.buffer.get_led(12, 12);
        assert_eq!(c0, c12, "LEDs within fill should match");
    }
}
