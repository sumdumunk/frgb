use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::lerp_color;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectDirection, EffectParams, Rgb};

// ---------------------------------------------------------------------------
// RenderEffect — smooth gradient rendering across the ring
// ---------------------------------------------------------------------------
//
// draws a smooth color gradient that shifts
// position each frame, creating a gentle flowing render effect.
//
// frames = total_per_fan * 2

pub struct RenderEffect;

impl EffectGenerator for RenderEffect {
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
            Rgb { r: 254, g: 254, b: 254 }
        };
        let c1 = apply_brightness(color1, params.brightness);
        let c2 = apply_brightness(color2, params.brightness);
        let ccw = params.direction == EffectDirection::Ccw;

        for frame in 0..frames {
            let offset = if ccw {
                ring_size - (frame % ring_size)
            } else {
                frame % ring_size
            };

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                for led in 0..ring_size {
                    let pos = (led + offset) % ring_size;
                    let t = (pos * 255 / ring_size.max(1)) as u8;
                    buf.set_led(frame, fan_base + led, lerp_color(c1, c2, t));
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
    fn smooth_gradient() {
        let gen = RenderEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }, Rgb { r: 0, g: 0, b: 254 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let c_start = result.buffer.get_led(0, 0);
        let c_mid = result.buffer.get_led(0, 12);
        // Should have different colors at different positions
        assert_ne!(c_start, c_mid, "should have gradient across ring");
    }
}
