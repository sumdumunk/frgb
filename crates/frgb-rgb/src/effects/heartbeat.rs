use crate::buffer::RgbBuffer;
use crate::color::apply_brightness;
use crate::effects::common::fill_fan;
use crate::generator::{EffectGenerator, EffectResult};
use crate::layout::LedLayout;
use frgb_model::rgb::{EffectParams, Rgb};
use frgb_model::Brightness;

// ---------------------------------------------------------------------------
// HeartBeatEffect
// ---------------------------------------------------------------------------
//
// Pulsing heartbeat: quick bright pulse, slight pause, second pulse, long pause.
// Mimics a cardiac rhythm.
//
// uses brightness envelope lookup table.
//
// frames = 96

pub struct HeartBeatEffect;

// Brightness envelope: heartbeat double-pulse pattern
// Ramp up → peak → down → brief pause → second pulse → long pause
const HEARTBEAT_ENVELOPE: [u8; 96] = [
    // First beat (frames 0-23): quick pulse
    16, 32, 64, 128, 192, 255, 255, 255, 192, 128, 64, 32, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    // Second beat (frames 24-47): slightly softer pulse
    16, 32, 48, 96, 160, 224, 255, 224, 160, 96, 48, 32, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    // Rest (frames 48-95): low/off
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16,
];

impl EffectGenerator for HeartBeatEffect {
    fn generate(&self, layout: &LedLayout, fan_count: u8, params: &EffectParams, colors: &[Rgb]) -> EffectResult {
        let frames = self.frame_count(layout, fan_count);
        let total_leds = layout.total_leds(fan_count);
        let mut buf = RgbBuffer::new(frames, total_leds);

        let color = colors.first().copied().unwrap_or(Rgb { r: 254, g: 0, b: 0 });
        let brightness = params.brightness;

        for frame in 0..frames {
            let envelope = HEARTBEAT_ENVELOPE[frame % 96];
            // Chain envelope with user brightness
            let eff_brightness = ((envelope as u16 * brightness.value() as u16) >> 8) as u8;
            let c = apply_brightness(color, Brightness::new(eff_brightness));

            for fan in 0..fan_count {
                fill_fan(&mut buf, frame, layout, fan, c);
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

    fn frame_count(&self, _: &LedLayout, _: u8) -> usize {
        96
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::device::DeviceType;
    use frgb_model::Brightness;

    #[test]
    fn heartbeat_frame_count() {
        let gen = HeartBeatEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        assert_eq!(gen.frame_count(&layout, 1), 96);
    }

    #[test]
    fn heartbeat_has_pulse_and_rest() {
        let gen = HeartBeatEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let peak = result.buffer.get_led(5, 0).r; // near first pulse peak
        let rest = result.buffer.get_led(60, 0).r; // during rest phase
        assert!(peak > rest, "pulse peak ({peak}) should be brighter than rest ({rest})");
        assert!(peak > 200, "peak should be bright, got {peak}");
    }

    #[test]
    fn heartbeat_double_pulse() {
        let gen = HeartBeatEffect;
        let layout = LedLayout::for_device(DeviceType::ClWireless);
        let params = EffectParams {
            brightness: Brightness::new(255),
            ..Default::default()
        };
        let colors = vec![Rgb { r: 254, g: 0, b: 0 }];
        let result = gen.generate(&layout, 1, &params, &colors);

        let pulse1 = result.buffer.get_led(6, 0).r; // first peak
        let gap = result.buffer.get_led(18, 0).r; // gap between pulses
        let pulse2 = result.buffer.get_led(30, 0).r; // second peak
        assert!(pulse1 > gap, "first pulse should be brighter than gap");
        assert!(pulse2 > gap, "second pulse should be brighter than gap");
    }
}
