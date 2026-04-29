use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::{bounce_pos, scale_color};
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// ShuttleRunEffect — fast back-and-forth sweep filling the ring
// ---------------------------------------------------------------------------
//
// a colored bar rapidly sweeps back and forth
// across the ring, each sweep adding one more LED that stays lit.
//
// frames = total_per_fan * 3

pub struct ShuttleRunEffect;

impl EffectGenerator for ShuttleRunEffect {
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

        let color = colors.first().copied().unwrap_or(Rgb { r: 254, g: 0, b: 0 });
        let c = apply_brightness(color, params.brightness);
        let bar_width = (ring_size / 6).max(2);

        for frame in 0..frames {
            let pos = bounce_pos(frame, ring_size - bar_width);

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                // Moving bar
                for i in 0..bar_width {
                    let led = (pos + i).min(ring_size - 1);
                    buf.set_led(frame, fan_base + led, c);
                }

                // Persistent trail: dimmer version of visited positions
                let trail_end = frame.min(ring_size);
                for i in 0..trail_end {
                    let trail_led = i % ring_size;
                    let existing = buf.get_led(frame, fan_base + trail_led);
                    if existing == Rgb::BLACK {
                        buf.set_led(frame, fan_base + trail_led, scale_color(c, 40));
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
        15.0
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
    fn bar_bounces() {
        let gen = ShuttleRunEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let brightest_0 = (0..24).max_by_key(|&l| result.buffer.get_led(0, l).r).unwrap();
        let brightest_20 = (0..24).max_by_key(|&l| result.buffer.get_led(20, l).r).unwrap();
        assert_ne!(brightest_0, brightest_20, "bar should move");
    }
}
