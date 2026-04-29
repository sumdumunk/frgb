use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::{bounce_pos, scale_color};
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// ReflectEffect — symmetric mirrored pattern expanding from center
// ---------------------------------------------------------------------------
//
// color expands symmetrically from
// the center of the ring outward, then contracts back. Creates a
// mirror/reflection visual.
//
// frames = total_per_fan * 2

pub struct ReflectEffect;

impl EffectGenerator for ReflectEffect {
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
            // Ping-pong: expand then contract
            let spread = bounce_pos(frame, half);

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                for i in 0..=spread {
                    // Gradient: brighter at center, dimmer at edges
                    let fade = if spread > 0 {
                        255 - (i * 180 / spread.max(1)).min(255) as u8
                    } else {
                        255
                    };
                    let fc = scale_color(c, fade);

                    // Mirror from center
                    let led_a = half + i;
                    let led_b = half.wrapping_sub(i);
                    if led_a < ring_size {
                        buf.set_led(frame, fan_base + led_a, fc);
                    }
                    if led_b < ring_size {
                        buf.set_led(frame, fan_base + led_b, fc);
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
        let gen = ReflectEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 48);
    }

    #[test]
    fn symmetric_pattern() {
        let gen = ReflectEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        // At expansion frame, pattern should be symmetric around center
        let frame = 6;
        let center = 12;
        for i in 0..6 {
            let a = result.buffer.get_led(frame, center + i);
            let b = result.buffer.get_led(frame, center - i);
            assert_eq!(a, b, "pattern should be symmetric at offset {i}");
        }
    }
}
