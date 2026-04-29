use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::{bounce_pos, scale_color};
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// WingEffect — two wings spread symmetrically from center then fold back
// ---------------------------------------------------------------------------
//
// two colored arcs expand outward from the center
// simultaneously (like opening wings), then contract back.
//
// frames = total_per_fan * 2

pub struct WingEffect;

impl EffectGenerator for WingEffect {
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
        let half = ring_size / 2;

        for frame in 0..frames {
            let spread = bounce_pos(frame, half);

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                for i in 0..=spread {
                    let fade = 255 - (i * 100 / spread.max(1)).min(255) as u8;
                    let fc = scale_color(c, fade);

                    // Spread from center outward
                    let a = (half + i) % ring_size;
                    let b = if half >= i { half - i } else { half + ring_size - i };
                    buf.set_led(frame, fan_base + a, fc);
                    if b < ring_size {
                        buf.set_led(frame, fan_base + b, fc);
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
        let gen = WingEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 48);
    }

    #[test]
    fn wings_expand() {
        let gen = WingEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let lit_1: usize = (0..24).filter(|&l| result.buffer.get_led(1, l).r > 0).count();
        let lit_10: usize = (0..24).filter(|&l| result.buffer.get_led(10, l).r > 0).count();
        assert!(lit_10 > lit_1, "wings should expand");
    }
}
