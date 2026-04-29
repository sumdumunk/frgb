use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::{effect_hash, scale_color};
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectDirection, EffectParams, Rgb};

// ---------------------------------------------------------------------------
// ElectricCurrentEffect — flickering electrical pulse traveling the ring
// ---------------------------------------------------------------------------
//
// a pulse of varying intensity travels
// around the ring with pseudo-random flicker simulating electrical arcing.
//
// frames = total_per_fan * 3

pub struct ElectricCurrentEffect;

impl EffectGenerator for ElectricCurrentEffect {
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
        let pulse_width = (ring_size / 3).max(3);

        for frame in 0..frames {
            let head = frame % ring_size;

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                for i in 0..pulse_width {
                    let ring_idx = if ccw {
                        (ring_size - 1 - head + ring_size - i) % ring_size
                    } else {
                        (head + ring_size - i) % ring_size
                    };

                    // Pseudo-random intensity flicker
                    let flicker = effect_hash(frame.wrapping_add(i), ring_idx);
                    let intensity = if flicker > 80 {
                        let base = 255 - (i * 255 / pulse_width);
                        (base as u16 * (128 + (flicker >> 1) as u16) / 255) as u8
                    } else {
                        0 // gap in the current
                    };

                    let led = fan_base + ring_idx;
                    buf.set_led(frame, led, scale_color(c, intensity));
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
        15.0
    }
    fn frame_count(&self, layout: &LedLayout, _fan_count: u8) -> usize {
        layout.total_per_fan as usize * 3
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn frame_count() {
        let gen = ElectricCurrentEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 72);
    }

    #[test]
    fn has_flickering_lit_leds() {
        let gen = ElectricCurrentEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let lit: usize = (0..24).filter(|&l| result.buffer.get_led(5, l).r > 0).count();
        assert!(lit > 0 && lit < 24, "should have partial illumination, got {lit}");
    }
}
