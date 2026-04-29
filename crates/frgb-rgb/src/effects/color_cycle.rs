use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectDirection, EffectParams, Rgb};

// ---------------------------------------------------------------------------
// ColorCycleEffect
// ---------------------------------------------------------------------------
//
// Groups of LEDs light up in the user's color, chase around the ring with
// dark gaps between groups. Creates a segmented rotating pattern.
//
// uses Slv3UserColour for 3 color segments with
// spacing. We simplify to a single user color with 3 lit segments
// separated by dark gaps.
//
// frames = total_per_fan * 3

pub struct ColorCycleEffect;

impl EffectGenerator for ColorCycleEffect {
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

        // 3 lit segments, each ~1/6 of ring, with ~1/6 dark gap between
        let segment_len = (ring_size / 6).max(1);
        let gap_len = (ring_size / 6).max(1);
        let cycle_len = segment_len + gap_len;

        for frame in 0..frames {
            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                for led in 0..ring_size {
                    let pos = if ccw {
                        (ring_size + led - frame % ring_size) % ring_size
                    } else {
                        (led + frame) % ring_size
                    };

                    // Lit if within a segment (3 segments evenly spaced)
                    let within_cycle = pos % cycle_len;
                    let lit = within_cycle < segment_len;

                    if lit {
                        let c = apply_brightness(color, brightness);
                        buf.set_led(frame, fan_base + led, c);
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

    fn frame_count(&self, layout: &LedLayout, _fan_count: u8) -> usize {
        (layout.total_per_fan as usize) * 3
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn color_cycle_frame_count() {
        let gen = ColorCycleEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 72);
    }

    #[test]
    fn color_cycle_has_lit_and_dark_leds() {
        let gen = ColorCycleEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let lit: usize = (0..24)
            .filter(|&led| {
                let c = result.buffer.get_led(0, led);
                c.r > 0 || c.g > 0 || c.b > 0
            })
            .count();
        let dark = 24 - lit;
        assert!(lit > 0, "should have some lit LEDs");
        assert!(dark > 0, "should have some dark LEDs");
    }
}
