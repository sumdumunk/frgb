use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::{bounce_pos, scale_color};
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// PingPongEffect — ball bouncing between ring ends with trail
// ---------------------------------------------------------------------------
//
// single lit segment bounces between the two ends
// of the ring, leaving a fading trail.
//
// frames = total_per_fan * 4

pub struct PingPongEffect;

impl EffectGenerator for PingPongEffect {
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

        for frame in 0..frames {
            let pos = bounce_pos(frame, ring_size - 1);

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                // Ball (bright center)
                buf.set_led(frame, fan_base + pos, c);

                // Fading trail (3 LEDs behind the ball)
                for trail in 1..=3usize {
                    let trail_pos_fwd = pos.wrapping_sub(trail);
                    let trail_pos_bwd = pos + trail;
                    let fade = (255 >> trail) as u8;
                    let tc = scale_color(c, fade);

                    if trail_pos_fwd < ring_size {
                        let existing = buf.get_led(frame, fan_base + trail_pos_fwd);
                        if existing == Rgb::BLACK {
                            buf.set_led(frame, fan_base + trail_pos_fwd, tc);
                        }
                    }
                    if trail_pos_bwd < ring_size {
                        let existing = buf.get_led(frame, fan_base + trail_pos_bwd);
                        if existing == Rgb::BLACK {
                            buf.set_led(frame, fan_base + trail_pos_bwd, tc);
                        }
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
        let gen = PingPongEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 96);
    }

    #[test]
    fn ball_bounces() {
        let gen = PingPongEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        // Frame 0: ball near start. Frame 23: ball near end
        let brightest_0 = (0..24).max_by_key(|&l| result.buffer.get_led(0, l).r).unwrap();
        let brightest_23 = (0..24).max_by_key(|&l| result.buffer.get_led(23, l).r).unwrap();
        assert_ne!(brightest_0, brightest_23, "ball should move");
    }
}
