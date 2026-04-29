use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::scale_color;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// RippleEffect — expanding wave from center outward
// ---------------------------------------------------------------------------
//
// a color pulse expands outward from the center
// of the ring, fading as it spreads. New ripples start periodically.
//
// frames = total_per_fan * 4

pub struct RippleEffect;

impl EffectGenerator for RippleEffect {
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
        let center = ring_size / 2;
        let ripple_period = ring_size;

        for frame in 0..frames {
            let ripple_age = frame % ripple_period;

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                for led in 0..ring_size {
                    let dist = led.abs_diff(center);

                    // Ripple expands outward; LED lights when wave front reaches it
                    if dist <= ripple_age && ripple_age - dist < 3 {
                        let fade = 255 - ((ripple_age - dist) as u16 * 85).min(255) as u8;
                        buf.set_led(frame, fan_base + led, scale_color(c, fade));
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
        25.0
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
        let gen = RippleEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 96);
    }

    #[test]
    fn ripple_expands() {
        let gen = RippleEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let lit_1: usize = (0..24).filter(|&l| result.buffer.get_led(1, l).r > 0).count();
        let lit_8: usize = (0..24).filter(|&l| result.buffer.get_led(8, l).r > 0).count();
        assert!(lit_8 > lit_1, "ripple should expand: frame1={lit_1} frame8={lit_8}");
    }
}
