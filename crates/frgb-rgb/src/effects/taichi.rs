use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectDirection, EffectParams, Rgb};

// ---------------------------------------------------------------------------
// TaichiEffect
// ---------------------------------------------------------------------------
//
// Rotating yin-yang pattern: half the ring is one color, half is another.
// The boundary rotates one position per frame.
//
// uses 2 user colors with alternating halves.
// We use the user color + its complement (inverted hue).
//
// frames = total_per_fan

pub struct TaichiEffect;

impl EffectGenerator for TaichiEffect {
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
        // Second color: complement (swap R↔B)
        let color2 = if colors.len() > 1 {
            colors[1]
        } else {
            Rgb {
                r: color1.b,
                g: color1.g,
                b: color1.r,
            }
        };
        let c1 = apply_brightness(color1, params.brightness);
        let c2 = apply_brightness(color2, params.brightness);
        let ccw = params.direction == EffectDirection::Ccw;

        for frame in 0..frames {
            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;
                for led in 0..ring_size {
                    let pos = if ccw {
                        (ring_size + led - frame % ring_size) % ring_size
                    } else {
                        (led + frame) % ring_size
                    };
                    let color = if pos < ring_size / 2 { c1 } else { c2 };
                    buf.set_led(frame, fan_base + led, color);
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
        layout.total_per_fan as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn taichi_half_and_half() {
        let gen = TaichiEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        // Frame 0: first half should be color1, second half color2
        let c0 = result.buffer.get_led(0, 0);
        let c12 = result.buffer.get_led(0, 12);
        assert_ne!(c0, c12, "opposite halves should differ");
    }
}
