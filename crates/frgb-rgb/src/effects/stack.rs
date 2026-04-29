use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectDirection, EffectParams, Rgb};

// ---------------------------------------------------------------------------
// StackEffect — blocks fall and stack up from one end
// ---------------------------------------------------------------------------
//
// colored blocks "drop" one at a time from the top
// and stack at the bottom. Once full, stack clears and restarts.
//
// frames = total_per_fan * (total_per_fan + 4) / 2

pub struct StackEffect;

impl EffectGenerator for StackEffect {
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
        let ccw = params.direction == EffectDirection::Ccw;

        // Pre-compute: for each "drop" (0..ring_size), how many frames it takes
        // to fall to its resting position, then total frames per cycle.
        let mut frame_idx = 0;
        for drop_num in 0..ring_size {
            // Source: INFERRED — clippy flagged if_same_then_else because both branches were identical.
            // Not a bug: travel distance is symmetric regardless of direction. Directionality is
            // handled by `falling_pos` and stacked `pos` below. Collapsed to remove dead branch.
            let travel = ring_size - 1 - drop_num;

            for step in 0..=travel {
                if frame_idx >= frames {
                    break;
                }

                for fan in 0..fan_count {
                    let fan_base = fan as usize * ring_size;

                    // Draw stacked blocks
                    for stacked in 0..drop_num {
                        let pos = if ccw { stacked } else { ring_size - 1 - stacked };
                        buf.set_led(frame_idx, fan_base + pos, c);
                    }

                    // Draw falling block
                    let falling_pos = if ccw { ring_size - 1 - step } else { step };
                    buf.set_led(frame_idx, fan_base + falling_pos, c);
                }

                frame_idx += 1;
            }
        }

        // Fill remaining frames with full ring
        while frame_idx < frames {
            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;
                for led in 0..ring_size {
                    buf.set_led(frame_idx, fan_base + led, c);
                }
            }
            frame_idx += 1;
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
        let n = layout.total_per_fan as usize;
        // Sum of travel distances: n-1 + n-2 + ... + 0 = n*(n-1)/2, plus n for held frames
        (n * (n + 1)) / 2 + n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn frame_count() {
        let gen = StackEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        // n=24: (24*25)/2 + 24 = 300 + 24 = 324
        assert_eq!(gen.frame_count(&layout, 1), 324);
    }

    #[test]
    fn stacking_increases() {
        let gen = StackEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let lit_early: usize = (0..24).filter(|&l| result.buffer.get_led(0, l).r > 0).count();
        let lit_late: usize = (0..24).filter(|&l| result.buffer.get_led(100, l).r > 0).count();
        assert!(
            lit_late > lit_early,
            "stack should grow: early={lit_early} late={lit_late}"
        );
    }
}
