use frgb_model::rgb::Rgb;

// ---------------------------------------------------------------------------
// RgbBuffer
// ---------------------------------------------------------------------------
//
// Internal storage layout: channel-major for efficient per-channel operations.
//
//   data[frame * 3 * leds + channel * leds + led]
//
// where channel 0 = R, 1 = G, 2 = B.
//
// `flatten()` outputs LED-major order for wire transmission:
//   [frame0_led0_R, frame0_led0_G, frame0_led0_B, frame0_led1_R, ...]

#[derive(Clone, Debug)]
pub struct RgbBuffer {
    frames: usize,
    leds: usize,
    /// Length = frames * 3 * leds; layout: [frame][channel][led]
    data: Vec<u8>,
}

impl RgbBuffer {
    pub fn new(frames: usize, leds: usize) -> Self {
        Self {
            frames,
            leds,
            data: vec![0u8; frames * 3 * leds],
        }
    }

    /// Total LED count per frame.
    pub fn led_count(&self) -> usize {
        self.leds
    }

    #[inline]
    fn idx(&self, frame: usize, channel: usize, led: usize) -> usize {
        frame * 3 * self.leds + channel * self.leds + led
    }

    /// Set the RGB color for a specific LED in a specific frame.
    ///
    /// # Panics
    /// Panics if `frame >= self.frames` or `led >= self.leds`.
    pub fn set_led(&mut self, frame: usize, led: usize, color: Rgb) {
        assert!(frame < self.frames, "frame {frame} out of bounds (max {})", self.frames);
        assert!(led < self.leds, "led {led} out of bounds (max {})", self.leds);
        let ir = self.idx(frame, 0, led);
        let ig = self.idx(frame, 1, led);
        let ib = self.idx(frame, 2, led);
        self.data[ir] = color.r;
        self.data[ig] = color.g;
        self.data[ib] = color.b;
    }

    /// Get the RGB color for a specific LED in a specific frame.
    ///
    /// # Panics
    /// Panics if `frame >= self.frames` or `led >= self.leds`.
    pub fn get_led(&self, frame: usize, led: usize) -> Rgb {
        assert!(frame < self.frames, "frame {frame} out of bounds (max {})", self.frames);
        assert!(led < self.leds, "led {led} out of bounds (max {})", self.leds);
        let ir = self.idx(frame, 0, led);
        let ig = self.idx(frame, 1, led);
        let ib = self.idx(frame, 2, led);
        Rgb {
            r: self.data[ir],
            g: self.data[ig],
            b: self.data[ib],
        }
    }

    /// Flatten to LED-major wire order:
    /// [frame0_led0_R, frame0_led0_G, frame0_led0_B, frame0_led1_R, ...]
    pub fn flatten(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.frames * self.leds * 3);
        for frame in 0..self.frames {
            for led in 0..self.leds {
                out.push(self.data[self.idx(frame, 0, led)]);
                out.push(self.data[self.idx(frame, 1, led)]);
                out.push(self.data[self.idx(frame, 2, led)]);
            }
        }
        out
    }

    pub fn frames(&self) -> usize {
        self.frames
    }

    pub fn leds(&self) -> usize {
        self.leds
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_set_and_get_led() {
        let mut buf = RgbBuffer::new(2, 24);
        buf.set_led(0, 5, Rgb { r: 254, g: 100, b: 50 });
        let color = buf.get_led(0, 5);
        assert_eq!(color, Rgb { r: 254, g: 100, b: 50 });
    }

    #[test]
    fn buffer_default_is_black() {
        let buf = RgbBuffer::new(1, 24);
        assert_eq!(buf.get_led(0, 0), Rgb::BLACK);
    }

    #[test]
    fn buffer_flatten_led_major() {
        let mut buf = RgbBuffer::new(1, 3);
        buf.set_led(0, 0, Rgb { r: 10, g: 20, b: 30 });
        buf.set_led(0, 1, Rgb { r: 40, g: 50, b: 60 });
        buf.set_led(0, 2, Rgb { r: 70, g: 80, b: 90 });
        let flat = buf.flatten();
        assert_eq!(flat, vec![10, 20, 30, 40, 50, 60, 70, 80, 90]);
    }

    #[test]
    fn buffer_flatten_multi_frame() {
        let mut buf = RgbBuffer::new(2, 2);
        buf.set_led(0, 0, Rgb { r: 1, g: 2, b: 3 });
        buf.set_led(0, 1, Rgb { r: 4, g: 5, b: 6 });
        buf.set_led(1, 0, Rgb { r: 7, g: 8, b: 9 });
        buf.set_led(1, 1, Rgb { r: 10, g: 11, b: 12 });
        let flat = buf.flatten();
        assert_eq!(flat, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    }
}
