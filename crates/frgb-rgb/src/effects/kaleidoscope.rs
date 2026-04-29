use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::rainbow_color_at;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// KaleidoscopeEffect — symmetric rainbow pattern mirrored across ring center
// ---------------------------------------------------------------------------
//
// Divide ring into 4 quadrants. Generate rainbow pattern for the first
// quadrant and mirror it to the other three. Rainbow hues cycle over time.
// TL-family effect.
//
// frames = 120, interval = 35ms

pub struct KaleidoscopeEffect;

impl EffectGenerator for KaleidoscopeEffect {
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
        let quadrant = ring_size / 4;
        if quadrant == 0 {
            return EffectResult {
                buffer: buf,
                frame_count: frames,
                interval_ms: self.interval_base(),
            };
        }

        for frame in 0..frames {
            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                for i in 0..quadrant {
                    // Rainbow position shifts with frame for animation
                    let hue_pos = (i + frame) % frames;
                    let color = rainbow_color_at(hue_pos, frames);
                    let color = apply_brightness(color, brightness);

                    // Mirror to all 4 quadrants
                    let positions = [
                        i,                                                  // Q1: forward
                        (quadrant * 2).saturating_sub(1).saturating_sub(i), // Q2: mirrored
                        quadrant * 2 + i,                                   // Q3: forward
                        (quadrant * 4).saturating_sub(1).saturating_sub(i), // Q4: mirrored
                    ];

                    for &pos in &positions {
                        if pos < ring_size {
                            buf.set_led(frame, fan_base + pos, color);
                        }
                    }
                }

                // Fill any remaining LEDs (if ring_size not divisible by 4)
                // with the last quadrant color to avoid black gaps
            }
        }

        EffectResult {
            buffer: buf,
            frame_count: frames,
            interval_ms: self.interval_base(),
        }
    }

    fn interval_base(&self) -> f32 {
        35.0
    }
    fn frame_count(&self, _layout: &LedLayout, _fan_count: u8) -> usize {
        120
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn frame_count_is_120() {
        let gen = KaleidoscopeEffect;
        let layout = LedLayout::for_device(DeviceType::TlWireless);
        assert_eq!(gen.frame_count(&layout, 1), 120);
    }

    #[test]
    fn symmetric_pattern() {
        let gen = KaleidoscopeEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let result = gen.generate(&layout, 1, &params, &[]);
        // Q1 position 0 should match Q3 position 12 (quadrant*2 + 0)
        let c0 = result.buffer.get_led(0, 0);
        let c12 = result.buffer.get_led(0, 12);
        assert_eq!(c0, c12, "Q1 and Q3 should mirror");
    }
}
