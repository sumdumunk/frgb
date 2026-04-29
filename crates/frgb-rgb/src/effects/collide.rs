use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::{add_color, max_color, scale_color};
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// CollideEffect — two colors rush toward center and flash on collision
// ---------------------------------------------------------------------------
//
// two colored segments approach from opposite
// ends. When they meet in the center, a bright flash fills the ring
// briefly before resetting.
//
// frames = total_per_fan * 2

pub struct CollideEffect;

impl EffectGenerator for CollideEffect {
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
        let half = ring_size / 2;
        let approach_frames = half;
        let flash_frames = 8.min(ring_size);

        for frame in 0..frames {
            let cycle_frame = frame % (approach_frames + flash_frames);

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                if cycle_frame < approach_frames {
                    let pos = cycle_frame;
                    // Color 1 from start
                    for i in 0..=pos.min(ring_size - 1) {
                        buf.set_led(frame, fan_base + i, c1);
                    }
                    // Color 2 from end
                    for i in 0..=pos.min(ring_size - 1) {
                        let led = ring_size - 1 - i;
                        let existing = buf.get_led(frame, fan_base + led);
                        buf.set_led(frame, fan_base + led, max_color(existing, c2));
                    }
                } else {
                    // Flash: full ring with blended color, fading
                    let flash_age = cycle_frame - approach_frames;
                    let fade = 255 - (flash_age * 255 / flash_frames).min(255) as u8;
                    let flash = scale_color(add_color(c1, c2), fade);
                    for led in 0..ring_size {
                        buf.set_led(frame, fan_base + led, flash);
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
        let gen = CollideEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 48);
    }

    #[test]
    fn has_flash_phase() {
        let gen = CollideEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }, Rgb { r: 0, g: 0, b: 254 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        // At collision frame (half = 12), all LEDs should be lit
        let lit_at_flash: usize = (0..24)
            .filter(|&l| {
                let c = result.buffer.get_led(12, l);
                c.r > 0 || c.b > 0
            })
            .count();
        assert_eq!(lit_at_flash, 24, "flash should fill ring");
    }
}
