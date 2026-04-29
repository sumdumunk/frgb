use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::lerp_color;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectDirection, EffectParams, Rgb};

// ---------------------------------------------------------------------------
// GradientRibbonEffect — smooth color gradient that rotates around the ring
// ---------------------------------------------------------------------------
//
// a smooth gradient between two colors
// wraps around the ring and rotates continuously.
//
// frames = total_per_fan * 2

pub struct GradientRibbonEffect;

impl EffectGenerator for GradientRibbonEffect {
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
                    // Gradient: 0→half = c1→c2, half→end = c2→c1
                    let half = ring_size / 2;
                    let t = if pos < half {
                        (pos * 255 / half.max(1)) as u8
                    } else {
                        ((ring_size - pos) * 255 / half.max(1)) as u8
                    };
                    let c = lerp_color(c1, c2, t);
                    buf.set_led(frame, fan_base + led, c);
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
        let gen = GradientRibbonEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 48);
    }

    #[test]
    fn gradient_has_both_colors() {
        let gen = GradientRibbonEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }, Rgb { r: 0, g: 0, b: 254 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let has_red = (0..24).any(|l| result.buffer.get_led(0, l).r > 100);
        let has_blue = (0..24).any(|l| result.buffer.get_led(0, l).b > 100);
        assert!(has_red && has_blue, "gradient should contain both colors");
    }

    #[test]
    fn gradient_rotates() {
        let gen = GradientRibbonEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }, Rgb { r: 0, g: 0, b: 254 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let c0 = result.buffer.get_led(0, 0);
        let c12 = result.buffer.get_led(12, 0);
        assert_ne!(c0, c12, "gradient should rotate between frames");
    }
}
