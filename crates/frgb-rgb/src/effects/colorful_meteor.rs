use crate::buffer::RgbBuffer;
use crate::effects::common::{meteor_channel, rainbow_color_at};
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectDirection, EffectParams, Rgb};

// ---------------------------------------------------------------------------
// ColorfulMeteorEffect — multiple rainbow-colored meteors chasing
// ---------------------------------------------------------------------------
//
// 3 meteors evenly spaced around the ring, each
// with a different hue from the rainbow spectrum.
//
// frames = total_per_fan * 3

pub struct ColorfulMeteorEffect;

const NUM_METEORS: usize = 3;

impl EffectGenerator for ColorfulMeteorEffect {
    fn generate(&self, layout: &LedLayout, fan_count: u8, params: &EffectParams, _colors: &[Rgb]) -> EffectResult {
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

        let brightness = params.brightness;
        let ccw = params.direction == EffectDirection::Ccw;
        let spacing = ring_size / NUM_METEORS;

        for frame in 0..frames {
            let base_offset = frame % ring_size;

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                for m in 0..NUM_METEORS {
                    let color = rainbow_color_at(m * spacing, ring_size);
                    let head_offset = (base_offset + m * spacing) % ring_size;
                    let head_pos = if ccw { ring_size - 1 - head_offset } else { head_offset };

                    for tail_pos in 0..4usize {
                        let fade: u8 = 255u8 >> tail_pos;
                        let ring_idx = if ccw {
                            (head_pos + tail_pos) % ring_size
                        } else {
                            (head_pos + ring_size - tail_pos) % ring_size
                        };
                        let led = fan_base + ring_idx;
                        let new = Rgb {
                            r: meteor_channel(color.r, fade, brightness),
                            g: meteor_channel(color.g, fade, brightness),
                            b: meteor_channel(color.b, fade, brightness),
                        };
                        let existing = buf.get_led(frame, led);
                        buf.set_led(
                            frame,
                            led,
                            Rgb {
                                r: existing.r.max(new.r),
                                g: existing.g.max(new.g),
                                b: existing.b.max(new.b),
                            },
                        );
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
    fn frame_count() {
        let gen = ColorfulMeteorEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 72);
    }

    #[test]
    fn multiple_lit_regions() {
        let gen = ColorfulMeteorEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let result = gen.generate(&layout, 1, &params, &[]);

        let lit: usize = (0..24)
            .filter(|&l| {
                let c = result.buffer.get_led(0, l);
                c.r > 0 || c.g > 0 || c.b > 0
            })
            .count();
        // 3 meteors × 4 tail LEDs = ~12, minus overlaps
        assert!(lit >= 8, "expected multiple lit regions, got {lit}");
    }
}
