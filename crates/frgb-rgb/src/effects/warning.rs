use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::fill_fan;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// WarningEffect
// ---------------------------------------------------------------------------
//
// Alternating blink between user color and black, like a warning strobe.
// 64 frames (8 on, 8 off, repeat with color variation).
// We simplify to: 8 frames color on, 8 frames off = 16 frame cycle, repeat ×4.
//
// frames = 64

pub struct WarningEffect;

impl EffectGenerator for WarningEffect {
    fn generate(&self, layout: &LedLayout, fan_count: u8, params: &EffectParams, colors: &[Rgb]) -> EffectResult {
        let frames = self.frame_count(layout, fan_count);
        let total_leds = layout.total_leds(fan_count);
        let mut buf = RgbBuffer::new(frames, total_leds);

        let color = colors.first().copied().unwrap_or(Rgb { r: 254, g: 0, b: 0 });
        let c = apply_brightness(color, params.brightness);

        for frame in 0..frames {
            // 8 frames on, 8 frames off
            let phase = (frame / 8) % 2;
            if phase == 0 {
                for fan in 0..fan_count {
                    fill_fan(&mut buf, frame, layout, fan, c);
                }
            }
            // phase==1: buffer already black
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
        64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn warning_alternates() {
        let gen = WarningEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let on = result.buffer.get_led(0, 0);
        let off = result.buffer.get_led(8, 0);
        assert!(on.r > 0, "frame 0 should be lit");
        assert_eq!(off, Rgb::BLACK, "frame 8 should be dark");
    }
}
