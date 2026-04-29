use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::bounce_pos;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// SnookerEffect — two dots bouncing back and forth
// ---------------------------------------------------------------------------
//
// Dot A starts at 0 going forward, dot B at ring-1 going backward.
// Each has a 2-LED fading trail. Dots bounce (ping-pong) at ring edges.
//
// frames = 200, interval = 30ms

pub struct SnookerEffect;

impl EffectGenerator for SnookerEffect {
    fn generate(&self, layout: &LedLayout, fan_count: u8, params: &EffectParams, colors: &[Rgb]) -> EffectResult {
        let frames = self.frame_count(layout, fan_count);
        let total_leds = layout.total_leds(fan_count);
        let mut buf = RgbBuffer::new(frames, total_leds);

        let ring_size = layout.total_per_fan as usize;
        if ring_size < 2 {
            return EffectResult {
                buffer: buf,
                frame_count: frames,
                interval_ms: self.interval_base(),
            };
        }

        let brightness = params.brightness;
        let color_a = colors
            .first()
            .copied()
            .or(params.color)
            .unwrap_or(Rgb { r: 254, g: 0, b: 0 });
        let color_b = Rgb { r: 0, g: 254, b: 0 };
        let tail_len = 2usize;
        let max_pos = ring_size - 1;

        for frame in 0..frames {
            let pos_a = bounce_pos(frame, max_pos);
            // Dot B offset by half period
            let pos_b = bounce_pos(frame + max_pos, max_pos);

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                // Paint dot A + trail
                for t in 0..=tail_len {
                    let fade = 255u8 >> t;
                    if let Some(pos) = pos_a.checked_sub(t) {
                        // Trail behind in forward direction when going forward
                        let c = Rgb {
                            r: ((color_a.r as u16 * fade as u16) >> 8) as u8,
                            g: ((color_a.g as u16 * fade as u16) >> 8) as u8,
                            b: ((color_a.b as u16 * fade as u16) >> 8) as u8,
                        };
                        let c = apply_brightness(c, brightness);
                        buf.set_led(frame, fan_base + pos, c);
                    }
                }

                // Paint dot B + trail
                for t in 0..=tail_len {
                    let fade = 255u8 >> t;
                    let pos = (pos_b + t).min(max_pos);
                    let c = Rgb {
                        r: ((color_b.r as u16 * fade as u16) >> 8) as u8,
                        g: ((color_b.g as u16 * fade as u16) >> 8) as u8,
                        b: ((color_b.b as u16 * fade as u16) >> 8) as u8,
                    };
                    let c = apply_brightness(c, brightness);
                    let existing = buf.get_led(frame, fan_base + pos);
                    buf.set_led(frame, fan_base + pos, crate::effects::common::add_color(existing, c));
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
        30.0
    }
    fn frame_count(&self, _layout: &LedLayout, _fan_count: u8) -> usize {
        200
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn frame_count_is_200() {
        let gen = SnookerEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 200);
    }

    #[test]
    fn dots_are_lit() {
        let gen = SnookerEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let result = gen.generate(&layout, 1, &params, &[]);
        // At frame 0, dot A at pos 0, dot B at pos max_pos (23)
        let c0 = result.buffer.get_led(0, 0);
        assert!(c0.r > 0, "dot A should be red at pos 0");
        let c23 = result.buffer.get_led(0, 23);
        assert!(c23.g > 0, "dot B should be green at pos 23");
    }
}
