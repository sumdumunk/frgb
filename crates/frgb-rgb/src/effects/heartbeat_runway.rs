use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::scale_color;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectDirection, EffectParams, Rgb};

// ---------------------------------------------------------------------------
// HeartBeatRunwayEffect — heartbeat pulse that travels along the ring
// ---------------------------------------------------------------------------
//
// combines the HeartBeat brightness envelope
// with a runway-style progressive fill. The pulse travels forward.
//
// frames = total_per_fan * 4

pub struct HeartBeatRunwayEffect;

// Cardiac double-pulse envelope (simplified from heartbeat.rs)
const PULSE: [u8; 32] = [
    0, 40, 100, 200, 254, 200, 100, 40, // first beat ramp
    0, 0, 0, 0, // inter-beat pause
    0, 30, 80, 160, 200, 160, 80, 30, // second beat (weaker)
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // recovery
];

impl EffectGenerator for HeartBeatRunwayEffect {
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

        for frame in 0..frames {
            let pulse_idx = frame % PULSE.len();
            let intensity = PULSE[pulse_idx];
            let fill_pos = (frame / 2) % ring_size; // slower travel

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                for i in 0..=fill_pos {
                    let led = if ccw { ring_size - 1 - i } else { i };
                    // Fade: brighter at fill front
                    let pos_fade = if fill_pos > 0 {
                        128 + (i * 127 / fill_pos) as u8
                    } else {
                        255
                    };
                    let combined = ((intensity as u16 * pos_fade as u16) >> 8) as u8;
                    let fc = scale_color(c, combined);
                    buf.set_led(frame, fan_base + led, fc);
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
        layout.total_per_fan as usize * 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn frame_count() {
        let gen = HeartBeatRunwayEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 96);
    }

    #[test]
    fn has_pulsing_fill() {
        let gen = HeartBeatRunwayEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        // Frame 4 (peak pulse) should be brighter than frame 10 (between beats)
        let peak = result.buffer.get_led(4, 0).r;
        let trough = result.buffer.get_led(10, 0).r;
        assert!(peak > trough, "should have pulsing brightness");
    }
}
