use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::lerp_color;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectDirection, EffectParams, Rgb};

// ---------------------------------------------------------------------------
// RiverEffect — flowing gradient that fills the entire ring
// ---------------------------------------------------------------------------
//
// smooth flowing gradient that covers all LEDs,
// creating a river-like continuous motion effect.
//
// frames = total_per_fan * 3

pub struct RiverEffect;

impl EffectGenerator for RiverEffect {
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
            Rgb { r: 0, g: 100, b: 254 }
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
                    // Sinusoidal-ish gradient via triangle wave
                    let half = ring_size / 2;
                    let t = if pos < half {
                        (pos * 255 / half.max(1)) as u8
                    } else {
                        ((ring_size - pos) * 255 / half.max(1)) as u8
                    };
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
        25.0
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
    fn all_leds_lit() {
        let gen = RiverEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let lit: usize = (0..24)
            .filter(|&l| {
                let c = result.buffer.get_led(0, l);
                c.r > 0 || c.g > 0 || c.b > 0
            })
            .count();
        assert_eq!(lit, 24, "river should fill entire ring");
    }
}
