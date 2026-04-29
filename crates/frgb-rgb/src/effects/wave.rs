use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};

// ---------------------------------------------------------------------------
// WaveEffect — sinusoidal brightness wave
// ---------------------------------------------------------------------------
//
// Frame f, LED i: brightness = sin(2pi * (i/ring + f/frames)) * 0.5 + 0.5
// Applied to the user's color. Creates a smooth traveling wave of brightness.
//
// frames = 120, interval = 35ms

pub struct WaveEffect;

impl EffectGenerator for WaveEffect {
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

        let base_color = colors
            .first()
            .copied()
            .or(params.color)
            .unwrap_or(Rgb { r: 0, g: 100, b: 254 });
        let brightness = params.brightness;
        let two_pi = std::f64::consts::TAU;

        for frame in 0..frames {
            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;
                for led in 0..ring_size {
                    let phase = two_pi * (led as f64 / ring_size as f64 + frame as f64 / frames as f64);
                    let wave = (phase.sin() * 0.5 + 0.5) as f32;
                    let factor = (wave * 255.0) as u8;
                    let c = Rgb {
                        r: ((base_color.r as u16 * factor as u16) >> 8) as u8,
                        g: ((base_color.g as u16 * factor as u16) >> 8) as u8,
                        b: ((base_color.b as u16 * factor as u16) >> 8) as u8,
                    };
                    let c = apply_brightness(c, brightness);
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
        35.0
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
    fn frame_count_is_120() {
        let gen = WaveEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 120);
    }

    #[test]
    fn brightness_varies_across_ring() {
        let gen = WaveEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let result = gen.generate(&layout, 1, &params, &[Rgb { r: 254, g: 254, b: 254 }]);
        // Sample several LEDs at frame 0 — they should have different brightnesses
        let vals: Vec<u8> = (0..24).map(|i| result.buffer.get_led(0, i).r).collect();
        let min = *vals.iter().min().unwrap();
        let max = *vals.iter().max().unwrap();
        assert!(
            max > min + 50,
            "wave should produce brightness variation, got min={} max={}",
            min,
            max
        );
    }
}
