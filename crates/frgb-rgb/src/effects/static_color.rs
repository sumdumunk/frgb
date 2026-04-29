use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::fill_fan;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

pub struct StaticColorEffect;

impl EffectGenerator for StaticColorEffect {
    fn generate(&self, layout: &LedLayout, fan_count: u8, params: &EffectParams, colors: &[Rgb]) -> EffectResult {
        let frames = self.frame_count(layout, fan_count);
        let total_leds = layout.total_leds(fan_count);
        let mut buf = RgbBuffer::new(frames, total_leds);
        let color = colors.first().copied().unwrap_or(Rgb { r: 254, g: 0, b: 0 });
        let bright_color = apply_brightness(color, params.brightness);
        for frame in 0..frames {
            for fan in 0..fan_count {
                fill_fan(&mut buf, frame, layout, fan, bright_color);
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
        30
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

    #[test]
    fn static_color_all_leds_same() {
        let gen = StaticColorEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);
        for led in 0..24 {
            let c = result.buffer.get_led(0, led);
            assert_eq!(c.r, 254, "LED {led}");
            assert_eq!(c.g, 0);
            assert_eq!(c.b, 0);
        }
    }

    #[test]
    fn static_color_30_frames() {
        let gen = StaticColorEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 30);
    }

    #[test]
    fn static_color_brightness_applied() {
        let gen = StaticColorEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        // brightness=128 => r = (254 * 128) >> 8 = 127
        let params = EffectParams {
            brightness: Brightness::new(128),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);
        let c = result.buffer.get_led(0, 0);
        assert_eq!(c.r, 127);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn static_color_default_color_when_empty() {
        let gen = StaticColorEffect;
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
}
