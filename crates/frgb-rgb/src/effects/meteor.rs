use crate::buffer::RgbBuffer;
use crate::effects::common::paint_meteor_tail;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectDirection, EffectParams, Rgb};

// ---------------------------------------------------------------------------
// MeteorEffect — chasing head with exponential-decay tail (5 LEDs)
// ---------------------------------------------------------------------------

pub struct MeteorEffect;

impl EffectGenerator for MeteorEffect {
    fn generate(&self, layout: &LedLayout, fan_count: u8, params: &EffectParams, colors: &[Rgb]) -> EffectResult {
        let frames = self.frame_count(layout, fan_count);
        let total_leds = layout.total_leds(fan_count);
        let mut buf = RgbBuffer::new(frames, total_leds);

        let color = colors.first().copied().unwrap_or(Rgb { r: 254, g: 0, b: 0 });
        let brightness = params.brightness;
        let ccw = params.direction == EffectDirection::Ccw;

        // Animate across ALL LEDs per fan as one continuous ring
        let ring_size = layout.total_per_fan as usize;
        if ring_size == 0 {
            return EffectResult {
                buffer: buf,
                frame_count: frames,
                interval_ms: self.interval_base(),
            };
        }

        for frame in 0..frames {
            let head_offset = frame % ring_size;

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                let head_pos = if ccw {
                    (ring_size - 1) - head_offset
                } else {
                    head_offset
                };

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::LedLayout;
    use frgb_model::device::DeviceType;
    use frgb_model::rgb::{EffectDirection, EffectParams, Rgb};
    use frgb_model::Brightness;

    #[test]
    fn meteor_frame_count() {
        let gen = MeteorEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        // total_per_fan = 24, so frames = 24 * 3 = 72
        assert_eq!(gen.frame_count(&layout, 1), 72);
    }

    #[test]
    fn meteor_frame0_exactly_5_lit_leds() {
        let gen = MeteorEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        // All LEDs 0..24 for fan 0. Count how many are non-zero in frame 0.
        let ring_size = layout.total_per_fan as usize;
        let lit: Vec<usize> = (0..ring_size)
            .filter(|&led| {
                let c = result.buffer.get_led(0, led);
                c.r > 0 || c.g > 0 || c.b > 0
            })
            .collect();

        assert_eq!(lit.len(), 5, "expected exactly 5 lit LEDs in frame 0, got {:?}", lit);
    }

    #[test]
    fn meteor_tail_decreasing_brightness() {
        // In frame 0 with CW direction, head is at ring[0].
        // Tail wraps: ring[0]=255, ring[23]=127, ring[22]=63, ring[21]=31, ring[20]=15
        let gen = MeteorEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let ring_size = layout.total_per_fan as usize; // 24
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let r_head = result.buffer.get_led(0, 0).r; // head fade=255
        let r_t1 = result.buffer.get_led(0, ring_size - 1).r; // tail_pos=1 fade=127
        let r_t2 = result.buffer.get_led(0, ring_size - 2).r; // tail_pos=2 fade=63
        let r_t3 = result.buffer.get_led(0, ring_size - 3).r; // tail_pos=3 fade=31
        let r_t4 = result.buffer.get_led(0, ring_size - 4).r; // tail_pos=4 fade=15

        assert!(r_head > r_t1, "head should be brighter than tail[1]");
        assert!(r_t1 > r_t2, "tail[1] should be brighter than tail[2]");
        assert!(r_t2 > r_t3, "tail[2] should be brighter than tail[3]");
        assert!(r_t3 > r_t4, "tail[3] should be brighter than tail[4]");
        assert!(r_t4 > 0, "tail[4] should still be non-zero");
    }

    #[test]
    fn meteor_ccw_has_5_lit_leds() {
        let gen = MeteorEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let ring_size = layout.total_per_fan as usize;
        let params = EffectParams {
            brightness: Brightness::new(255),
            direction: EffectDirection::Ccw,
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let lit: usize = (0..ring_size)
            .filter(|&led| {
                let c = result.buffer.get_led(0, led);
                c.r > 0 || c.g > 0 || c.b > 0
            })
            .count();
        assert_eq!(lit, 5, "CCW frame 0 should have 5 lit LEDs, got {}", lit);
    }

    #[test]
    fn meteor_ccw_head_at_opposite_end() {
        let gen = MeteorEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let ring_size = layout.total_per_fan as usize;

        let cw_params = EffectParams {
            brightness: Brightness::new(255),
            direction: EffectDirection::Cw,
            ..Default::default()
        };
        let ccw_params = EffectParams {
            brightness: Brightness::new(255),
            direction: EffectDirection::Ccw,
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];

        let cw_result = gen.generate(&layout, 1, &cw_params, &colors);
        let ccw_result = gen.generate(&layout, 1, &ccw_params, &colors);

        // CW head at index 0, CCW head at index ring_size-1
        let cw_head = cw_result.buffer.get_led(0, 0);
        let ccw_head = ccw_result.buffer.get_led(0, ring_size - 1);

        assert!(cw_head.r > 0, "CW head should be lit");
        assert!(ccw_head.r > 0, "CCW head should be lit");
    }

    #[test]
    fn meteor_multi_fan() {
        let gen = MeteorEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let ring_size = layout.total_per_fan as usize;
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 3, &params, &colors);

        for fan in 0..3u8 {
            let fan_base = fan as usize * ring_size;
            let lit: usize = (fan_base..fan_base + ring_size)
                .filter(|&led| {
                    let c = result.buffer.get_led(0, led);
                    c.r > 0 || c.g > 0 || c.b > 0
                })
                .count();
            assert_eq!(lit, 5, "fan {} should have 5 lit LEDs in frame 0", fan);
        }
    }

    #[test]
    fn meteor_frame_count_sl() {
        // SL fans: total_per_fan=21, frames = 21 * 3 = 63
        let gen = MeteorEffect;
        let layout = LedLayout::for_device(DeviceType::SlWireless);
        assert_eq!(gen.frame_count(&layout, 1), 63);
    }
}
