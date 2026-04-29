use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectDirection, EffectParams, Rgb};

// ---------------------------------------------------------------------------
// ScanEffect
// ---------------------------------------------------------------------------
//
// A colored bar sweeps across the LEDs and back. Like a scanner/KITT effect.
//
// single color bar moves forward then backward.
// Bar width = ring_size / 4.
//
// frames = total_per_fan * 2 (one pass forward, one backward)

pub struct ScanEffect;

impl EffectGenerator for ScanEffect {
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
        let bar_width = (ring_size / 4).max(2);

        for frame in 0..frames {
            // Ping-pong: 0..ring_size forward, then ring_size..0 backward
            let pos = if frame < ring_size {
                frame
            } else {
                2 * ring_size - frame - 1
            };
            let pos = if ccw { ring_size - 1 - pos } else { pos };

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;
                for i in 0..bar_width {
                    if pos + i >= ring_size {
                        break;
                    }
                    let led = pos + i;
                    // Fade: brightest at center of bar
                    let dist = if i < bar_width / 2 { i } else { bar_width - 1 - i };
                    let fade = ((dist + 1) as u16 * 255 / (bar_width / 2 + 1) as u16) as u8;
                    let fc = Rgb {
                        r: ((c.r as u16 * fade as u16) >> 8) as u8,
                        g: ((c.g as u16 * fade as u16) >> 8) as u8,
                        b: ((c.b as u16 * fade as u16) >> 8) as u8,
                    };
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
        (layout.total_per_fan as usize) * 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn scan_frame_count() {
        let gen = ScanEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 48);
    }

    #[test]
    fn scan_bar_moves() {
        let gen = ScanEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        // Frame 0 and frame 12 should have lit LEDs at different positions
        let lit_0: Vec<usize> = (0..24).filter(|&l| result.buffer.get_led(0, l).r > 0).collect();
        let lit_12: Vec<usize> = (0..24).filter(|&l| result.buffer.get_led(12, l).r > 0).collect();
        assert!(!lit_0.is_empty(), "frame 0 should have lit LEDs");
        assert!(!lit_12.is_empty(), "frame 12 should have lit LEDs");
        assert_ne!(lit_0, lit_12, "bar should move between frames");
    }
}
