use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::{effect_hash, rainbow_color_at};
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// LotteryEffect — random segments light up in sequence like a lottery wheel
// ---------------------------------------------------------------------------
//
// segments of the ring light up randomly in
// rainbow colors, cycling through like a spinning lottery wheel that
// gradually slows down.
//
// frames = 180

pub struct LotteryEffect;

impl EffectGenerator for LotteryEffect {
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
        let segment_size = (ring_size / 6).max(2);
        let num_segments = ring_size / segment_size;

        for frame in 0..frames {
            // Pick which segment lights up (pseudo-random but changes each frame)
            let active = effect_hash(frame, 0) as usize % num_segments;
            let color = rainbow_color_at(effect_hash(frame, 42) as usize, 256);
            let c = apply_brightness(color, brightness);

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;
                let seg_start = active * segment_size;

                for i in 0..segment_size {
                    let led = (seg_start + i) % ring_size;
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
        30.0
    }
    fn frame_count(&self, _layout: &LedLayout, _fan_count: u8) -> usize {
        180
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn frame_count() {
        let gen = LotteryEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 180);
    }

    #[test]
    fn lights_segment_per_frame() {
        let gen = LotteryEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let result = gen.generate(&layout, 1, &params, &[]);

        let lit: usize = (0..24)
            .filter(|&l| {
                let c = result.buffer.get_led(0, l);
                c.r > 0 || c.g > 0 || c.b > 0
            })
            .count();
        // Should light one segment (~4 LEDs) not the whole ring
        assert!(lit > 0 && lit < 12, "should light a segment, got {lit} LEDs");
    }
}
