use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectDirection, EffectParams, Rgb};

// ---------------------------------------------------------------------------
// RainbowEffect
// ---------------------------------------------------------------------------
//
// Distributes a full rainbow gradient across all LEDs, rotating one position
// per frame. Each LED gets a hue based on its position, creating a moving
// rainbow band across the fan.
//
// uses lookup tables for the gradient. We compute
// the same gradient procedurally via HSV.
//
// frames = total_per_fan (one full rotation)
// Direction CW:  rainbow shifts forward
// Direction CCW: rainbow shifts backward

pub struct RainbowEffect;

impl EffectGenerator for RainbowEffect {
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
        let ccw = params.direction == EffectDirection::Ccw;

        for frame in 0..frames {
            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                for led in 0..ring_size {
                    // Compute hue position: distribute 360° across ring, shift by frame
                    let pos = if ccw {
                        (ring_size + led - frame % ring_size) % ring_size
                    } else {
                        (led + frame) % ring_size
                    };
                    let hue = (pos as u32 * 65535 / ring_size as u32) as u16;
                    let c = crate::color::hue_to_rgb(hue);
                    let c = apply_brightness(c, brightness);
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
        layout.total_per_fan as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn rainbow_frame_count() {
        let gen = RainbowEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 24);
    }

    #[test]
    fn rainbow_all_leds_colored() {
        let gen = RainbowEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let result = gen.generate(&layout, 1, &params, &[]);

        for led in 0..24 {
            let c = result.buffer.get_led(0, led);
            assert!(c.r > 0 || c.g > 0 || c.b > 0, "LED {led} should be colored");
        }
    }

    #[test]
    fn rainbow_adjacent_leds_differ() {
        let gen = RainbowEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let result = gen.generate(&layout, 1, &params, &[]);

        let c0 = result.buffer.get_led(0, 0);
        let c12 = result.buffer.get_led(0, 12);
        assert_ne!(c0, c12, "LEDs 0 and 12 should have different hues");
    }
}
