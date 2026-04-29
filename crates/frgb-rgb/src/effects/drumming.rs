use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::{fill_fan, scale_color};
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// DrummingEffect — rhythmic pulses like drum beats
// ---------------------------------------------------------------------------
//
// alternating strong/weak pulses, simulating
// a drum rhythm. Strong beat fills ring, weak beat fills partially.
//
// frames = 96

pub struct DrummingEffect;

impl EffectGenerator for DrummingEffect {
    fn generate(&self, layout: &LedLayout, fan_count: u8, params: &EffectParams, colors: &[Rgb]) -> EffectResult {
        let frames = self.frame_count(layout, fan_count);
        let total_leds = layout.total_leds(fan_count);
        let mut buf = RgbBuffer::new(frames, total_leds);

        let color = colors.first().copied().unwrap_or(Rgb { r: 254, g: 0, b: 0 });
        let c = apply_brightness(color, params.brightness);

        // Pattern: strong(8) + decay(8) + weak(4) + decay(4) + rest(8) = 32 frame cycle
        let cycle = 32;

        for frame in 0..frames {
            let phase = frame % cycle;

            let intensity: u8 = if phase < 2 {
                255 // strong hit
            } else if phase < 8 {
                (255 - ((phase - 2) * 42).min(255)) as u8 // strong decay
            } else if phase < 16 {
                0 // rest
            } else if phase < 18 {
                180 // weak hit
            } else if phase < 22 {
                (180 - ((phase - 18) * 45).min(180)) as u8 // weak decay
            } else {
                0 // rest
            };

            if intensity > 0 {
                let fc = scale_color(c, intensity);
                for fan in 0..fan_count {
                    fill_fan(&mut buf, frame, layout, fan, fc);
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
    fn frame_count(&self, _layout: &LedLayout, _fan_count: u8) -> usize {
        96
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn frame_count() {
        let gen = DrummingEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 96);
    }

    #[test]
    fn has_rhythm() {
        let gen = DrummingEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        // Frame 0 (strong beat) should be brighter than frame 10 (rest)
        let bright_0 = result.buffer.get_led(0, 0).r;
        let bright_10 = result.buffer.get_led(10, 0).r;
        assert!(bright_0 > bright_10, "strong beat should be brighter than rest");
    }
}
