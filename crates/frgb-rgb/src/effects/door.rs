use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::bounce_pos;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// DoorEffect — two halves open from center then close back
// ---------------------------------------------------------------------------
//
// ring splits at center, each half slides outward
// (door opening), pauses, then slides back (door closing).
//
// frames = total_per_fan * 2

pub struct DoorEffect;

impl EffectGenerator for DoorEffect {
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
        let half = ring_size / 2;

        for frame in 0..frames {
            // Ping-pong: open then close
            let gap = bounce_pos(frame, half);

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                // Left half: fills from 0 up to (half - gap)
                for led in 0..half.saturating_sub(gap) {
                    buf.set_led(frame, fan_base + led, c);
                }
                // Right half: fills from (half + gap) to end
                for led in (half + gap)..ring_size {
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
        let gen = DoorEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 48);
    }

    #[test]
    fn door_opens_and_closes() {
        let gen = DoorEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let lit_0: usize = (0..24).filter(|&l| result.buffer.get_led(0, l).r > 0).count();
        let lit_6: usize = (0..24).filter(|&l| result.buffer.get_led(6, l).r > 0).count();
        // At frame 0 (closed): all lit. At frame 6 (opening): fewer lit.
        assert!(lit_0 > lit_6, "door should open: closed={lit_0} open={lit_6}");
    }
}
