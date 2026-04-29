use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::rainbow_color_at;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// PaintEffect — progressive fill from start
// ---------------------------------------------------------------------------
//
// One LED painted per frame. After a full fill, hold for 20 frames, then
// fill with the next rainbow hue. 4 hue passes.
//
// frames = ring_size * 4 + 80, interval = 40ms

pub struct PaintEffect;

impl EffectGenerator for PaintEffect {
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
        let passes = 4usize;
        let hold = 20usize;
        let pass_len = ring_size + hold;

        for frame in 0..frames {
            let pass = (frame / pass_len).min(passes - 1);
            let pass_frame = frame - pass * pass_len;
            let fill_count = pass_frame.min(ring_size);
            let color = rainbow_color_at(pass, passes);
            let color = apply_brightness(color, brightness);

            // Background: previous pass color
            let bg = if pass > 0 {
                let pc = rainbow_color_at(pass - 1, passes);
                apply_brightness(pc, brightness)
            } else {
                Rgb::BLACK
            };

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;
                for led in 0..ring_size {
                    if led < fill_count {
                        buf.set_led(frame, fan_base + led, color);
                    } else {
                        buf.set_led(frame, fan_base + led, bg);
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
        40.0
    }

    fn frame_count(&self, layout: &LedLayout, _fan_count: u8) -> usize {
        let ring = layout.total_per_fan as usize;
        if ring == 0 {
            1
        } else {
            ring * 4 + 80
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
        let gen = PaintEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 24 * 4 + 80);
    }

    #[test]
    fn progressive_fill() {
        let gen = PaintEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let result = gen.generate(&layout, 1, &params, &[]);
        // Frame 0: 0 LEDs filled (fill_count = min(0, 24) = 0... actually pass_frame=0, fill_count=0)
        // Frame 1: 1 LED filled
        let c = result.buffer.get_led(1, 0);
        assert!(c.r > 0 || c.g > 0 || c.b > 0, "LED 0 at frame 1 should be painted");
        let c2 = result.buffer.get_led(1, 2);
        assert_eq!(c2, Rgb::BLACK, "LED 2 at frame 1 should still be black");
    }
}
