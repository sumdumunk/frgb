use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::scale_color;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// PumpEffect — pulsing radial glow simulating pump pressure
// ---------------------------------------------------------------------------
//
// ring pulses outward from center in a radial
// pattern synchronized to a pump heartbeat rhythm. Brightness peaks
// at center and fades toward edges, with rhythmic intensity variation.
//
// frames = 120

pub struct PumpEffect;

impl EffectGenerator for PumpEffect {
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

        let color = colors.first().copied().unwrap_or(Rgb { r: 0, g: 100, b: 254 });
        let c = apply_brightness(color, params.brightness);
        let center = ring_size / 2;

        // Pump rhythm: 30 frames per cycle (quick pulse + slow recovery)
        let cycle = 30;

        for frame in 0..frames {
            let phase = frame % cycle;
            // Pump envelope: sharp rise (0-4), hold (4-8), slow decay (8-30)
            let pump_intensity: u8 = if phase < 4 {
                (phase * 64).min(255) as u8
            } else if phase < 8 {
                255
            } else {
                let decay = phase - 8;
                255u8.saturating_sub((decay * 12) as u8)
            };

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                for led in 0..ring_size {
                    let dist = led.abs_diff(center);
                    // Radial fade from center
                    let radial = if center > 0 {
                        255 - (dist * 200 / center).min(255) as u8
                    } else {
                        255
                    };
                    let combined = ((pump_intensity as u16 * radial as u16) >> 8) as u8;
                    if combined > 0 {
                        buf.set_led(frame, fan_base + led, scale_color(c, combined));
                    }
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
        25.0
    }
    fn frame_count(&self, _layout: &LedLayout, _fan_count: u8) -> usize {
        120
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn frame_count() {
        let gen = PumpEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 120);
    }

    #[test]
    fn pump_has_rhythmic_pulse() {
        let gen = PumpEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 0, g: 100, b: 254 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        // Frame 4-7 (peak) should be brighter than frame 20 (decay)
        let peak = result.buffer.get_led(5, 12).b; // center
        let decay = result.buffer.get_led(20, 12).b;
        assert!(
            peak > decay,
            "pump peak ({peak}) should be brighter than decay ({decay})"
        );
    }

    #[test]
    fn center_brighter_than_edge() {
        let gen = PumpEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 0, g: 100, b: 254 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let center = result.buffer.get_led(5, 12).b;
        let edge = result.buffer.get_led(5, 0).b;
        assert!(center > edge, "center ({center}) should be brighter than edge ({edge})");
    }
}
