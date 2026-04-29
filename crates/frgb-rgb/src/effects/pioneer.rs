use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::scale_color;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectDirection, EffectParams, Rgb};

// ---------------------------------------------------------------------------
// PioneerEffect — explorer LED advances and leaves a dimming trail
// ---------------------------------------------------------------------------
//
// a single bright LED advances around the ring,
// leaving behind a gradually dimming trail that persists longer than
// a standard meteor.
//
// frames = total_per_fan * 3

pub struct PioneerEffect;

impl EffectGenerator for PioneerEffect {
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
        let trail_len = ring_size / 2; // longer trail than meteor

        for frame in 0..frames {
            let head = frame % ring_size;

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                for i in 0..=trail_len {
                    let ring_idx = if ccw {
                        (ring_size - 1 - head + ring_size + i) % ring_size
                    } else {
                        (head + ring_size - i) % ring_size
                    };
                    // Gradual fade along trail
                    let fade = (255 - i * 255 / (trail_len + 1)).max(0) as u8;
                    let fc = scale_color(c, fade);
                    let led = fan_base + ring_idx;
                    let existing = buf.get_led(frame, led);
                    if fc.r > existing.r || fc.g > existing.g || fc.b > existing.b {
                        buf.set_led(frame, led, fc);
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
        layout.total_per_fan as usize * 3
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn trail_longer_than_meteor() {
        let gen = PioneerEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let lit: usize = (0..24).filter(|&l| result.buffer.get_led(12, l).r > 0).count();
        assert!(lit > 5, "pioneer should have longer trail than meteor (5), got {lit}");
    }
}
