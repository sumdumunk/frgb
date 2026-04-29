use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::fill_fan;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// RainbowMorphEffect
// ---------------------------------------------------------------------------
//
// All LEDs display the same color. The color smoothly cycles through the
// full RGB spectrum: red → green → blue → red over 255 frames.
//
// 3-phase linear transition:
//   Frames   0- 84: R 255→0,  G 0→255, B=0     (red → green)
//   Frames  85-169: R=0,      G 255→0, B 0→255  (green → blue)
//   Frames 170-254: R 0→255,  G=0,     B 255→0  (blue → red)
//
// frames = 255

pub struct RainbowMorphEffect;

impl EffectGenerator for RainbowMorphEffect {
    fn generate(&self, layout: &LedLayout, fan_count: u8, params: &EffectParams, _colors: &[Rgb]) -> EffectResult {
        let frames = self.frame_count(layout, fan_count);
        let total_leds = layout.total_leds(fan_count);
        let mut buf = RgbBuffer::new(frames, total_leds);

        let brightness = params.brightness;

        // Track RGB channels across frames
        let mut r: u8 = 254;
        let mut g: u8 = 0;
        let mut b: u8 = 0;

        for frame in 0..frames {
            let c = apply_brightness(Rgb { r, g, b }, brightness);
            for fan in 0..fan_count {
                fill_fan(&mut buf, frame, layout, fan, c);
            }

            // Advance color
            if frame < 85 {
                r = r.saturating_sub(3);
                g = g.saturating_add(3);
            } else if frame < 170 {
                g = g.saturating_sub(3);
                b = b.saturating_add(3);
            } else {
                b = b.saturating_sub(3);
                r = r.saturating_add(3);
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

    fn frame_count(&self, _: &LedLayout, _: u8) -> usize {
        255
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn rainbow_morph_frame_count() {
        let gen = RainbowMorphEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 255);
    }

    #[test]
    fn rainbow_morph_starts_red() {
        let gen = RainbowMorphEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let result = gen.generate(&layout, 1, &params, &[]);
        let c = result.buffer.get_led(0, 0);
        assert_eq!(c.r, 254);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn rainbow_morph_all_leds_same_per_frame() {
        let gen = RainbowMorphEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let result = gen.generate(&layout, 1, &params, &[]);

        let c0 = result.buffer.get_led(42, 0);
        let c12 = result.buffer.get_led(42, 12);
        let c23 = result.buffer.get_led(42, 23);
        assert_eq!(c0, c12);
        assert_eq!(c0, c23);
    }

    #[test]
    fn rainbow_morph_transitions_through_green() {
        let gen = RainbowMorphEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let result = gen.generate(&layout, 1, &params, &[]);
        // Around frame 85: should be mostly green
        let c = result.buffer.get_led(84, 0);
        assert!(c.g > c.r, "at frame 84, green ({}) should exceed red ({})", c.g, c.r);
    }
}
