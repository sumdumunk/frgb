use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::{effect_hash, rainbow_color_at};
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// DiscoEffect — random color flashing across LED segments
// ---------------------------------------------------------------------------
//
// LEDs flash in random colors at high frequency,
// creating a disco ball effect. Each LED gets a pseudo-random color
// per frame from the rainbow spectrum.
//
// frames = 120

pub struct DiscoEffect;

impl EffectGenerator for DiscoEffect {
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

        for frame in 0..frames {
            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                for led in 0..ring_size {
                    let h = effect_hash(frame, led + fan as usize * 100);
                    // Only ~60% of LEDs light per frame (disco sparkle)
                    if h > 100 {
                        let color_idx = effect_hash(frame.wrapping_mul(7), led.wrapping_mul(13));
                        let c = rainbow_color_at(color_idx as usize, 256);
                        let c = apply_brightness(c, brightness);
                        buf.set_led(frame, fan_base + led, c);
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
        30.0
    }
    fn frame_count(&self, _layout: &LedLayout, _fan_count: u8) -> usize {
        120
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn frame_count() {
        let gen = DiscoEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 120);
    }

    #[test]
    fn has_multiple_colors() {
        let gen = DiscoEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let result = gen.generate(&layout, 1, &params, &[]);

        let mut has_r = false;
        let mut has_g = false;
        let mut has_b = false;
        for led in 0..24 {
            let c = result.buffer.get_led(5, led);
            if c.r > 100 {
                has_r = true;
            }
            if c.g > 100 {
                has_g = true;
            }
            if c.b > 100 {
                has_b = true;
            }
        }
        assert!(has_r || has_g || has_b, "should have colored LEDs");
    }
}
