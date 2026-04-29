use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectDirection, EffectParams, Rgb};
use frgb_model::Brightness;

// ---------------------------------------------------------------------------
// TideEffect
// ---------------------------------------------------------------------------
//
// A color wave sweeps across the LEDs, cycling through hues. Each position
// gets a hue offset based on its index, and the whole gradient shifts per frame.
// Like rainbow but with a smooth tide-like rolling motion.
//
// 4-color wave. We generate a smooth hue sweep.
//
// frames = total_per_fan * 4

pub struct TideEffect;

impl EffectGenerator for TideEffect {
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
        let brightness = params.brightness;
        let ccw = params.direction == EffectDirection::Ccw;

        for frame in 0..frames {
            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                for led in 0..ring_size {
                    // Sine-like brightness wave
                    let pos = if ccw {
                        (ring_size + led - frame % ring_size) % ring_size
                    } else {
                        (led + frame) % ring_size
                    };

                    // Create a smooth wave: brightness varies sinusoidally across position
                    let phase = (pos as f32 / ring_size as f32) * std::f32::consts::PI * 2.0;
                    let wave = ((phase.sin() + 1.0) / 2.0 * 255.0) as u8;
                    let eff = ((wave as u16 * brightness.value() as u16) >> 8) as u8;
                    let c = apply_brightness(color, Brightness::new(eff));
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
        layout.total_per_fan as usize * 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn tide_frame_count() {
        let gen = TideEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 96);
    }

    #[test]
    fn tide_has_brightness_variation() {
        let gen = TideEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        // Adjacent LEDs should have different brightness (wave pattern)
        let values: Vec<u8> = (0..24).map(|l| result.buffer.get_led(0, l).r).collect();
        let unique: std::collections::HashSet<u8> = values.iter().copied().collect();
        assert!(
            unique.len() > 3,
            "should have brightness variation, got {} unique values",
            unique.len()
        );
    }
}
