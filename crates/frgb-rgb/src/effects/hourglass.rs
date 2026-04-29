use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// HourglassEffect — LEDs drain from top half to bottom half
// ---------------------------------------------------------------------------
//
// like sand in an hourglass: LEDs turn off from
// one end while turning on at the other end, one at a time.
//
// frames = total_per_fan * 2

pub struct HourglassEffect;

impl EffectGenerator for HourglassEffect {
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
            let drain = frame % ring_size;

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                // Top half drains (turns off from start)
                for led in drain..ring_size {
                    buf.set_led(frame, fan_base + led, c);
                }
                // Bottom fills (turns on from end)
                for led in (ring_size - drain)..ring_size {
                    buf.set_led(frame, fan_base + led, c);
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
        let gen = HourglassEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 48);
    }

    #[test]
    fn drains_and_fills() {
        let gen = HourglassEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        // Frame 0: all lit. Frame 12: partially drained.
        let lit_0: usize = (0..24).filter(|&l| result.buffer.get_led(0, l).r > 0).count();
        assert_eq!(lit_0, 24, "frame 0 should be fully lit");
    }
}
