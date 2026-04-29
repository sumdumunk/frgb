use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::{bounce_pos, max_color};
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// DoubleArcEffect — two arcs bouncing from opposite ends
// ---------------------------------------------------------------------------
//
// two arcs start at opposite sides and sweep
// toward each other, then return. Max-blend at overlap.
//
// frames = total_per_fan * 2

pub struct DoubleArcEffect;

impl EffectGenerator for DoubleArcEffect {
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
        let arc_width = (ring_size / 6).max(2);
        let travel = ring_size - arc_width;

        for frame in 0..frames {
            let pos = bounce_pos(frame, travel);

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                // Arc 1: from start
                for i in 0..arc_width {
                    let led = (pos + i).min(ring_size - 1);
                    let existing = buf.get_led(frame, fan_base + led);
                    buf.set_led(frame, fan_base + led, max_color(existing, c1));
                }

                // Arc 2: from end (opposite direction)
                for i in 0..arc_width {
                    let led = ring_size - 1 - (pos + i).min(ring_size - 1);
                    let existing = buf.get_led(frame, fan_base + led);
                    buf.set_led(frame, fan_base + led, max_color(existing, c2));
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
    fn frame_count() {
        let gen = DoubleArcEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 48);
    }

    #[test]
    fn two_arcs_present() {
        let gen = DoubleArcEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }, Rgb { r: 0, g: 0, b: 254 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let has_red = (0..24).any(|l| result.buffer.get_led(0, l).r > 100);
        let has_blue = (0..24).any(|l| result.buffer.get_led(0, l).b > 100);
        assert!(has_red && has_blue, "should have both arcs");
    }
}
