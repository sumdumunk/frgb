use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::scale_color;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectDirection, EffectParams, Rgb};

// ---------------------------------------------------------------------------
// StaggeredEffect — offset color chase with staggered timing per segment
// ---------------------------------------------------------------------------
//
// ring divided into segments that light up in
// staggered sequence, creating a wave-like chase pattern.
//
// frames = total_per_fan * 3

pub struct StaggeredEffect;

impl EffectGenerator for StaggeredEffect {
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
        let seg_size = (ring_size / 4).max(2);
        let num_segs = ring_size / seg_size;

        for frame in 0..frames {
            for fan in 0..fan_count {
                let fan_base = fan as usize * ring_size;

                for seg in 0..num_segs {
                    // Each segment has a staggered phase offset
                    let phase = (frame + seg * 3) % (num_segs * 4);
                    let active = phase < num_segs;

                    if active {
                        let seg_start = if ccw {
                            ring_size - seg_size - seg * seg_size
                        } else {
                            seg * seg_size
                        };
                        // Brightness fade based on phase
                        let fade = 255 - (phase * 40).min(255) as u8;
                        let fc = scale_color(c, fade);
                        for i in 0..seg_size {
                            let led = (seg_start + i) % ring_size;
                            buf.set_led(frame, fan_base + led, fc);
                        }
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
        let gen = StaggeredEffect;
        let layout = LedLayout::for_device(DeviceType::SlWireless);
        assert_eq!(gen.frame_count(&layout, 1), 63);
    }

    #[test]
    fn has_segmented_pattern() {
        let gen = StaggeredEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let lit: usize = (0..24).filter(|&l| result.buffer.get_led(0, l).r > 0).count();
        assert!(lit > 0 && lit < 24, "should have partial illumination");
    }
}
