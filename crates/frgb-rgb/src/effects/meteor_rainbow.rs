use crate::buffer::RgbBuffer;
use crate::effects::common::{paint_meteor_tail, rainbow_color_at};
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectDirection, EffectParams, Rgb};

// ---------------------------------------------------------------------------
// MeteorRainbowEffect — meteor with rainbow-cycling head color
// ---------------------------------------------------------------------------
//
// single meteor where the head color cycles through
// the rainbow spectrum as it moves around the ring. Each frame, the head
// hue advances proportionally to its ring position.
//
// frames = total_per_fan * 3

pub struct MeteorRainbowEffect;

impl EffectGenerator for MeteorRainbowEffect {
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

        for frame in 0..frames {
            let head_offset = frame % ring_size;
            let color = rainbow_color_at(head_offset, ring_size);

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;
                let head_pos = if ccw { ring_size - 1 - head_offset } else { head_offset };

                paint_meteor_tail(&mut buf, frame, fan_base, head_pos, ring_size, ccw, color, brightness);
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
        let gen = MeteorRainbowEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 72);
    }

    #[test]
    fn head_color_changes_across_frames() {
        let gen = MeteorRainbowEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let result = gen.generate(&layout, 1, &params, &[]);

        let c0 = result.buffer.get_led(0, 0);
        let c12 = result.buffer.get_led(12, 12);
        assert_ne!(c0, c12, "head color should vary across frames");
    }
}
