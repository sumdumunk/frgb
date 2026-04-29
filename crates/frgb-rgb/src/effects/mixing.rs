use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectDirection, EffectParams, Rgb};

// ---------------------------------------------------------------------------
// MixingEffect
// ---------------------------------------------------------------------------
//
// Two colors approach from opposite ends of the ring, overlap in the middle
// to create a blended color, then continue past each other.
//
// two colors chase with additive blending at overlap.
//
// frames = total_per_fan * 3

pub struct MixingEffect;

impl EffectGenerator for MixingEffect {
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

        let color1 = colors.first().copied().unwrap_or(Rgb { r: 254, g: 0, b: 0 });
        let color2 = if colors.len() > 1 {
            colors[1]
        } else {
            Rgb { r: 0, g: 0, b: 254 }
        };
        let c1 = apply_brightness(color1, params.brightness);
        let c2 = apply_brightness(color2, params.brightness);
        let ccw = params.direction == EffectDirection::Ccw;

        for frame in 0..frames {
            let pos = frame % ring_size;

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                for led in 0..ring_size {
                    let idx = if ccw { ring_size - 1 - led } else { led };

                    // Color 1 fills from the start up to `pos`
                    let has_c1 = idx <= pos;
                    // Color 2 fills from the end down to `ring_size - 1 - pos`
                    let has_c2 = idx >= ring_size.saturating_sub(pos + 1);

                    let color = match (has_c1, has_c2) {
                        (true, true) => {
                            // Overlap: additive blend, clamped
                            Rgb {
                                r: c1.r.saturating_add(c2.r).min(254),
                                g: c1.g.saturating_add(c2.g).min(254),
                                b: c1.b.saturating_add(c2.b).min(254),
                            }
                        }
                        (true, false) => c1,
                        (false, true) => c2,
                        (false, false) => Rgb::BLACK,
                    };
                    buf.set_led(frame, fan_base + led, color);
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
    fn mixing_frame_count() {
        let gen = MixingEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 72);
    }

    #[test]
    fn mixing_has_blended_region() {
        let gen = MixingEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }, Rgb { r: 0, g: 0, b: 254 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        // At frame 12 (halfway), middle LEDs should have both R and B
        let mid = result.buffer.get_led(12, 12);
        assert!(mid.r > 0 && mid.b > 0, "overlap should blend: r={} b={}", mid.r, mid.b);
    }
}
