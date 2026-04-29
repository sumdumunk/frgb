use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::scale_color;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectDirection, EffectParams, Rgb};

// ---------------------------------------------------------------------------
// EndlessEffect — continuous flowing stream of color
// ---------------------------------------------------------------------------
//
// smooth flowing color stream that wraps
// continuously around the ring with a brightness gradient.
//
// frames = total_per_fan * 3

pub struct EndlessEffect;

impl EffectGenerator for EndlessEffect {
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
        let ccw = params.direction == EffectDirection::Ccw;
        let stream_len = ring_size * 2 / 3;

        for frame in 0..frames {
            let head = frame % ring_size;

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                for i in 0..stream_len {
                    let ring_idx = if ccw {
                        (ring_size - 1 - head + ring_size - i) % ring_size
                    } else {
                        (head + ring_size - i) % ring_size
                    };
                    // Linear gradient along stream
                    let fade = (255 - i * 255 / stream_len.max(1)).max(0) as u8;
                    let led = fan_base + ring_idx;
                    buf.set_led(frame, led, scale_color(c, fade));
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
        let gen = EndlessEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 72);
    }

    #[test]
    fn stream_moves() {
        let gen = EndlessEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let brightest_0 = (0..24).max_by_key(|&l| result.buffer.get_led(0, l).r).unwrap();
        let brightest_10 = (0..24).max_by_key(|&l| result.buffer.get_led(10, l).r).unwrap();
        assert_ne!(brightest_0, brightest_10, "stream head should move");
    }
}
