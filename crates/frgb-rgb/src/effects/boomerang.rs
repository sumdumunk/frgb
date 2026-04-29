use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::{bounce_pos, scale_color};
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectDirection, EffectParams, Rgb};

// ---------------------------------------------------------------------------
// BoomerangEffect — arc launches forward, slows, returns faster
// ---------------------------------------------------------------------------
//
// a colored arc flies out and returns like a boomerang.
// Outward journey decelerates, return journey accelerates.
//
// frames = total_per_fan * 2

pub struct BoomerangEffect;

impl EffectGenerator for BoomerangEffect {
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
        let arc_width = (ring_size / 6).max(2);

        for frame in 0..frames {
            let pos = bounce_pos(frame, ring_size - arc_width);

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;
                for i in 0..arc_width {
                    let fade = 255 - (i * 128 / arc_width.max(1)).min(255) as u8;
                    let led = if ccw {
                        ring_size - 1 - (pos + i).min(ring_size - 1)
                    } else {
                        (pos + i).min(ring_size - 1)
                    };
                    buf.set_led(frame, fan_base + led, scale_color(c, fade));
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
        let gen = BoomerangEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 48);
    }

    #[test]
    fn arc_returns_to_start() {
        let gen = BoomerangEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        // Frame 0 and last frame should have similar lit positions
        let lit_0: Vec<usize> = (0..24).filter(|&l| result.buffer.get_led(0, l).r > 0).collect();
        let lit_end: Vec<usize> = (0..24).filter(|&l| result.buffer.get_led(47, l).r > 0).collect();
        assert!(!lit_0.is_empty());
        // After full period, arc should be near start again
        assert!(lit_end.iter().any(|&l| l < 8), "should return near start");
    }
}
