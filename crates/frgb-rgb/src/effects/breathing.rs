use crate::buffer::RgbBuffer;
use crate::color::chained_brightness;
use crate::effects::common::fill_fan;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// BreathingEffect
// ---------------------------------------------------------------------------
//
// 170 frames total:
//   frames  0-84:  ramp goes 0 → 84  (ramp = frame)
//   frames 85-169: ramp goes 84 → 0  (ramp = 169 - frame)
//
// Per LED per frame:
//   ramp_mul = (ramp * 3) & 0xFF
//   channel  = chained_brightness(color_channel, ramp_mul, brightness)

pub struct BreathingEffect;

impl EffectGenerator for BreathingEffect {
    fn generate(&self, layout: &LedLayout, fan_count: u8, params: &EffectParams, colors: &[Rgb]) -> EffectResult {
        let frames = self.frame_count(layout, fan_count);
        let total_leds = layout.total_leds(fan_count);
        let mut buf = RgbBuffer::new(frames, total_leds);
        let color = colors.first().copied().unwrap_or(Rgb { r: 254, g: 0, b: 0 });
        let brightness = params.brightness;

        for frame in 0..frames {
            // Compute ramp value 0..=84
            let ramp: u8 = if frame < 85 { frame as u8 } else { (169 - frame) as u8 };
            let ramp_mul = ((ramp as u16 * 3) & 0xFF) as u8;

            let c = Rgb {
                r: chained_brightness(color.r, ramp_mul, brightness),
                g: chained_brightness(color.g, ramp_mul, brightness),
                b: chained_brightness(color.b, ramp_mul, brightness),
            };

            for fan in 0..fan_count {
                fill_fan(&mut buf, frame, layout, fan, c);
            }
        }

        EffectResult {
            buffer: buf,
            frame_count: frames,
            interval_ms: self.interval_base(),
        }
    }

    fn interval_base(&self) -> f32 {
        11.0
    }

    fn frame_count(&self, _: &LedLayout, _: u8) -> usize {
        170
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::LedLayout;
    use frgb_model::device::DeviceType;
    use frgb_model::rgb::{EffectParams, Rgb};
    use frgb_model::Brightness;

    fn make_result(brightness: Brightness) -> EffectResult {
        let gen = BreathingEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness,
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        gen.generate(&layout, 1, &params, &colors)
    }

    #[test]
    fn breathing_170_frames() {
        let gen = BreathingEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 170);
    }

    #[test]
    fn breathing_frame0_is_dark() {
        // frame 0: ramp=0, ramp_mul=0 → chained_brightness(254, 0, 255) = 0
        let result = make_result(Brightness::new(255));
        let c = result.buffer.get_led(0, 0);
        assert_eq!(c.r, 0, "frame 0 should be dark (ramp=0)");
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn breathing_frame42_mid_brightness() {
        // frame 42: ramp=42, ramp_mul=(42*3)&0xFF=126
        // chained_brightness(254, 126, 255) = (254*126)>>8 = 124, (124*255)>>8 = 123
        let result = make_result(Brightness::new(255));
        let c = result.buffer.get_led(42, 0);
        assert!(c.r > 0 && c.r < 200, "frame 42 should be mid brightness, got r={}", c.r);
    }

    #[test]
    fn breathing_frame85_near_peak() {
        // frame 85: ramp=84, ramp_mul=(84*3)&0xFF=252
        // chained_brightness(254, 252, 255) = (254*252)>>8 = 249, (249*255)>>8 = 248
        let result = make_result(Brightness::new(255));
        let c85 = result.buffer.get_led(85, 0);
        let c0 = result.buffer.get_led(0, 0);
        assert!(c85.r > c0.r, "frame 85 should be brighter than frame 0");
        assert!(c85.r > 200, "frame 85 near peak should be bright, got r={}", c85.r);
    }

    #[test]
    fn breathing_ramp_is_symmetric() {
        // frame k and frame (169-k) should have the same ramp value, hence same color
        let result = make_result(Brightness::new(255));
        for k in 0usize..85 {
            let ca = result.buffer.get_led(k, 0);
            let cb = result.buffer.get_led(169 - k, 0);
            assert_eq!(ca, cb, "frames {k} and {} should match", 169 - k);
        }
    }
}
