use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectDirection, EffectParams, Rgb};

// ---------------------------------------------------------------------------
// MopUpEffect — progressive fill sweep that wipes color across the ring
// ---------------------------------------------------------------------------
//
// single color sweeps across the ring, filling
// LEDs one by one, then all blink off and repeat.
//
// frames = total_per_fan * 2 (fill phase + held phase)

pub struct MopUpEffect;

impl EffectGenerator for MopUpEffect {
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
            let fill_count = if frame < ring_size {
                frame + 1 // filling phase
            } else {
                // Drain phase: hold full for a bit then empty
                let drain_frame = frame - ring_size;
                ring_size.saturating_sub(drain_frame + 1)
            };

            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;
                for i in 0..fill_count {
                    let led = if ccw { ring_size - 1 - i } else { i };
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
        let gen = MopUpEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 48);
    }

    #[test]
    fn progressive_fill() {
        let gen = MopUpEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let lit_0: usize = (0..24).filter(|&l| result.buffer.get_led(0, l).r > 0).count();
        let lit_12: usize = (0..24).filter(|&l| result.buffer.get_led(12, l).r > 0).count();
        assert!(lit_12 > lit_0, "more LEDs should be lit in later frames");
    }
}
