use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::effect_hash;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// TwinkleEffect
// ---------------------------------------------------------------------------
//
// Random LEDs flash on and fade out, creating a twinkling/sparkle effect.
// Uses a deterministic pseudo-random sequence (seeded from position) so
// the pattern is reproducible across sends.
//
// frames = 120

pub struct TwinkleEffect;

const TWINKLE_FRAMES: usize = 120;
const FADE_STEPS: usize = 8;

impl EffectGenerator for TwinkleEffect {
    fn generate(&self, layout: &LedLayout, fan_count: u8, params: &EffectParams, colors: &[Rgb]) -> EffectResult {
        let frames = self.frame_count(layout, fan_count);
        let total_leds = layout.total_leds(fan_count);
        let mut buf = RgbBuffer::new(frames, total_leds);

        let color = colors.first().copied().unwrap_or(Rgb { r: 254, g: 254, b: 254 });
        let brightness = params.brightness;

        let ring_size = layout.total_per_fan as usize;
        if ring_size == 0 {
            return EffectResult {
                buffer: buf,
                frame_count: frames,
                interval_ms: self.interval_base(),
            };
        }

        // For each frame, determine which LEDs are twinkling.
        // A LED starts a twinkle when its hash falls below a threshold.
        // It then fades over FADE_STEPS frames.
        for frame in 0..frames {
            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                for led in 0..ring_size {
                    let global_led = fan_base + led;

                    // Check if any recent frame triggered this LED
                    let mut max_brightness: u8 = 0;
                    for age in 0..FADE_STEPS {
                        if frame < age {
                            break;
                        }
                        let trigger_frame = frame - age;
                        let h = effect_hash(trigger_frame, global_led);
                        if h < 12 {
                            // This LED was triggered `age` frames ago
                            let fade = 255u8 >> age;
                            max_brightness = max_brightness.max(fade);
                        }
                    }

                    if max_brightness > 0 {
                        let c = apply_brightness(color, brightness);
                        let c = Rgb {
                            r: ((c.r as u16 * max_brightness as u16) >> 8) as u8,
                            g: ((c.g as u16 * max_brightness as u16) >> 8) as u8,
                            b: ((c.b as u16 * max_brightness as u16) >> 8) as u8,
                        };
                        buf.set_led(frame, global_led, c);
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
        20.0
    }

    fn frame_count(&self, _: &LedLayout, _: u8) -> usize {
        TWINKLE_FRAMES
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn twinkle_frame_count() {
        let gen = TwinkleEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 120);
    }

    #[test]
    fn twinkle_has_some_lit_leds() {
        let gen = TwinkleEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 254, b: 254 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        // Across all frames, some LEDs should be lit
        let mut any_lit = false;
        for frame in 0..result.frame_count {
            for led in 0..24 {
                let c = result.buffer.get_led(frame, led);
                if c.r > 0 || c.g > 0 || c.b > 0 {
                    any_lit = true;
                    break;
                }
            }
            if any_lit {
                break;
            }
        }
        assert!(any_lit, "twinkle should have some lit LEDs across frames");
    }

    #[test]
    fn twinkle_deterministic() {
        let gen = TwinkleEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];

        let r1 = gen.generate(&layout, 1, &params, &colors);
        let r2 = gen.generate(&layout, 1, &params, &colors);

        for frame in 0..r1.frame_count {
            for led in 0..24 {
                assert_eq!(
                    r1.buffer.get_led(frame, led),
                    r2.buffer.get_led(frame, led),
                    "frame {frame} LED {led} should be deterministic"
                );
            }
        }
    }
}
