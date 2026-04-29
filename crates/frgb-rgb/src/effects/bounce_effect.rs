use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::{bounce_pos, scale_color};
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// BounceEffect — ball of light bouncing between ring ends with gravity
// ---------------------------------------------------------------------------
//
// a bright orb bounces back and forth across the
// ring with a "gravity" feel: it decelerates at the top and accelerates
// at the bottom. Leaves a short glowing trail.
//
// frames = total_per_fan * 4

pub struct BounceEffect;

impl EffectGenerator for BounceEffect {
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
        let trail_len = 4;

        for frame in 0..frames {
            let pos = bounce_pos(frame, ring_size - 1);

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                // Bright orb at current position
                buf.set_led(frame, fan_base + pos, c);

                // Trail (dimming behind the orb)
                for t in 1..=trail_len {
                    // Trail extends in the direction the orb came from
                    let trail_pos = if frame % ((ring_size - 1) * 2) < ring_size - 1 {
                        // Moving forward: trail behind
                        pos.wrapping_sub(t)
                    } else {
                        // Moving backward: trail ahead
                        pos + t
                    };
                    if trail_pos < ring_size {
                        let fade = (255 / (t + 1)) as u8;
                        buf.set_led(frame, fan_base + trail_pos, scale_color(c, fade));
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
        layout.total_per_fan as usize * 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn frame_count() {
        let gen = BounceEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 96);
    }

    #[test]
    fn orb_bounces() {
        let gen = BounceEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let brightest_0 = (0..24).max_by_key(|&l| result.buffer.get_led(0, l).r).unwrap();
        let brightest_12 = (0..24).max_by_key(|&l| result.buffer.get_led(12, l).r).unwrap();
        assert_ne!(brightest_0, brightest_12, "orb should move between frames");
    }

    #[test]
    fn has_trail() {
        let gen = BounceEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        // At frame 5, orb is at position 5. Trail should be at 4,3,2,1
        let lit: usize = (0..24).filter(|&l| result.buffer.get_led(5, l).r > 0).count();
        assert!(lit > 1, "should have orb + trail, got {lit} lit LEDs");
    }
}
