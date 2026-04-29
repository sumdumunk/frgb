use crate::buffer::RgbBuffer;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectDirection, EffectParams, Rgb};

// ---------------------------------------------------------------------------
// RunwayEffect
// ---------------------------------------------------------------------------
//
// Progressive fill around the inner ring with a linear brightness gradient.
//
// frames = inner_count * 2
// Per frame: fill_count = frame % ring_size LEDs are lit from start position.
// Gradient: LED i (0-indexed within the lit segment) has brightness
//           proportional to (i + 1) / fill_count  →  brightest at the head.
//
// Direction CW:  fill from index 0 forward
// Direction CCW: fill from index (ring_size-1) backward

pub struct RunwayEffect;

impl EffectGenerator for RunwayEffect {
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
            let fill_count = frame % ring_size;
            if fill_count == 0 {
                // Nothing lit this frame — buffer already zeroed
                continue;
            }

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                for i in 0..fill_count {
                    // Linear brightness gradient: oldest LED dimmest, head brightest
                    // i=0 is the start/dimmest, i=fill_count-1 is the head/brightest
                    let grad = (((i + 1) as u32 * brightness.value() as u32) / fill_count as u32) as u8;

                    let ring_idx = if ccw { (ring_size - 1) - i } else { i };

                    let led = fan_base + ring_idx;
                    let c = Rgb {
                        r: ((color.r as u32 * grad as u32 + 128) >> 8) as u8,
                        g: ((color.g as u32 * grad as u32 + 128) >> 8) as u8,
                        b: ((color.b as u32 * grad as u32 + 128) >> 8) as u8,
                    };
                    buf.set_led(frame, led, c);
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
    fn runway_frame_count() {
        let gen = RunwayEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        // total_per_fan = 24, so frames = 24 * 2 = 48
        assert_eq!(gen.frame_count(&layout, 1), 48);
    }

    #[test]
    fn runway_frame0_is_empty() {
        let gen = RunwayEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let ring_size = layout.total_per_fan as usize;
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);
        for led in 0..ring_size {
            let c = result.buffer.get_led(0, led);
            assert_eq!(c, Rgb::BLACK, "frame 0 LED {led} should be dark");
        }
    }

    #[test]
    fn runway_fill_count_correct() {
        // frame 3: fill_count = 3 % 24 = 3, exactly 3 LEDs lit
        let gen = RunwayEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let ring_size = layout.total_per_fan as usize;
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let lit: usize = (0..ring_size)
            .filter(|&led| {
                let c = result.buffer.get_led(3, led);
                c.r > 0 || c.g > 0 || c.b > 0
            })
            .count();
        assert_eq!(lit, 3, "frame 3 should have exactly 3 lit LEDs");
    }

    #[test]
    fn runway_gradient_increases() {
        // frame 4: fill_count=4, LEDs 0..3 lit. LED 3 (head) should be brightest.
        let gen = RunwayEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let r0 = result.buffer.get_led(4, 0).r;
        let r1 = result.buffer.get_led(4, 1).r;
        let r2 = result.buffer.get_led(4, 2).r;
        let r3 = result.buffer.get_led(4, 3).r;

        assert!(r1 > r0, "LED 1 should be brighter than LED 0");
        assert!(r2 > r1, "LED 2 should be brighter than LED 1");
        assert!(r3 > r2, "LED 3 (head) should be brightest");
    }

    #[test]
    fn runway_ccw_fills_from_opposite_end() {
        let gen = RunwayEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let ring_size = layout.total_per_fan as usize; // 24

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

        // Frame 3: fill_count=3; CW fills idx 0,1,2; CCW fills idx 23,22,21
        let cw_head_r = cw_result.buffer.get_led(3, 2).r;
        let ccw_head_r = ccw_result.buffer.get_led(3, ring_size - 1).r;

        assert!(cw_head_r > 0, "CW head LED should be lit in frame 3");
        assert!(ccw_head_r > 0, "CCW head LED should be lit in frame 3");

        // CW: far end should be dark
        let cw_far = cw_result.buffer.get_led(3, ring_size - 1);
        assert_eq!(cw_far, Rgb::BLACK, "CW frame 3: LED at ring_size-1 should be dark");

        // CCW: index 0 should be dark
        let ccw_far = ccw_result.buffer.get_led(3, 0);
        assert_eq!(ccw_far, Rgb::BLACK, "CCW frame 3: LED at index 0 should be dark");
    }

    #[test]
    fn runway_multi_fan() {
        let gen = RunwayEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let ring_size = layout.total_per_fan as usize;
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 3, &params, &colors);

        // Frame 3: fill_count=3, should light LEDs 0,1,2 in each fan
        for fan in 0..3u8 {
            let fan_base = fan as usize * ring_size;
            for i in 0..3 {
                let c = result.buffer.get_led(3, fan_base + i);
                assert!(c.r > 0, "fan {} LED {} should be lit in frame 3", fan, i);
            }
            let dark = result.buffer.get_led(3, fan_base + 3);
            assert_eq!(dark, Rgb::BLACK, "fan {} LED 3 should be dark in frame 3", fan);
        }
    }

    #[test]
    fn runway_frame_count_sl() {
        // SL fans: total_per_fan=21, frames = 21 * 2 = 42
        let gen = RunwayEffect;
        let layout = LedLayout::for_device(DeviceType::SlWireless);
        assert_eq!(gen.frame_count(&layout, 1), 42);
    }
}
