use crate::constants::{RGB_PROTOCOL_MAX, SPEED_MAX, SPEED_MIN};
use frgb_model::rgb::Rgb;
use frgb_model::SpeedPercent;

/// Convert an Rgb color to a 16-bit hue value (0-65535 maps to 0°-360°).
///
/// Uses standard HSV normalization (divide by 255.0, not 254.0/protocol max).
/// Floating-point equality comparisons on `max` are safe here because r, g, b
/// are derived from the same integer division path (u8 / 255.0 is exact in f64).
pub fn rgb_to_hue16(color: Rgb) -> u16 {
    let r = color.r as f64 / 255.0;
    let g = color.g as f64 / 255.0;
    let b = color.b as f64 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let diff = max - min;
    if diff == 0.0 {
        return 0;
    }
    let hue_deg = if max == r {
        (60.0 * ((g - b) / diff) + 360.0) % 360.0
    } else if max == g {
        (60.0 * ((b - r) / diff) + 120.0) % 360.0
    } else {
        (60.0 * ((r - g) / diff) + 240.0) % 360.0
    };
    let hue = ((hue_deg * 65535.0) / 360.0) as u16;
    // Firmware treats hue=0 as "no color". Wrap chromatic 0 to 65535 (360° = 0°).
    if hue == 0 {
        65535
    } else {
        hue
    }
}

pub fn hue16_to_rgb(hue: u16) -> Rgb {
    let hue_deg = (hue as f64 / 65535.0) * 360.0;
    let h = (hue_deg / 60.0).min(5.999_999);
    let i = (h as u32 % 6) as u8;
    let f = h - h.floor();
    let max = RGB_PROTOCOL_MAX as f64;
    let q = (max * (1.0 - f)) as u8;
    let t = (max * f) as u8;
    let (r, g, b) = match i {
        0 => (RGB_PROTOCOL_MAX, t, 0),
        1 => (q, RGB_PROTOCOL_MAX, 0),
        2 => (0, RGB_PROTOCOL_MAX, t),
        3 => (0, q, RGB_PROTOCOL_MAX),
        4 => (t, 0, RGB_PROTOCOL_MAX),
        _ => (RGB_PROTOCOL_MAX, 0, q),
    };
    Rgb { r, g, b }
}

pub fn percent_to_speed_byte(percent: SpeedPercent) -> u8 {
    let pct = percent.value();
    let range = (SPEED_MAX - SPEED_MIN) as u16;
    SPEED_MIN + ((pct as u16 * range) / 100) as u8
}

pub fn speed_byte_to_percent(byte: u8) -> SpeedPercent {
    if byte <= SPEED_MIN {
        return SpeedPercent::new(0);
    }
    if byte == SPEED_MAX {
        return SpeedPercent::new(100);
    }
    let range = (SPEED_MAX - SPEED_MIN) as u16;
    SpeedPercent::new((((byte - SPEED_MIN) as u16 * 100) / range) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::rgb::Rgb;
    use frgb_model::SpeedPercent;

    #[test]
    fn red_to_hue16() {
        let hue = rgb_to_hue16(Rgb { r: 254, g: 0, b: 0 });
        assert_eq!(hue, 0xFFFF); // chromatic hue=0 wraps to 65535 (firmware treats 0 as "no color")
    }

    #[test]
    fn green_to_hue16() {
        let hue = rgb_to_hue16(Rgb { r: 0, g: 254, b: 0 });
        assert_eq!(hue, 0x5555);
    }

    #[test]
    fn blue_to_hue16() {
        let hue = rgb_to_hue16(Rgb { r: 0, g: 0, b: 254 });
        assert_eq!(hue, 0xAAAA);
    }

    #[test]
    fn hue16_to_rgb_red() {
        let rgb = hue16_to_rgb(0x0000);
        assert_eq!(rgb, Rgb { r: 254, g: 0, b: 0 });
    }

    #[test]
    fn hue16_to_rgb_green() {
        let rgb = hue16_to_rgb(0x5555);
        assert_eq!(rgb, Rgb { r: 0, g: 254, b: 0 });
    }

    #[test]
    fn hue16_to_rgb_blue() {
        let rgb = hue16_to_rgb(0xAAAA);
        assert_eq!(rgb, Rgb { r: 0, g: 0, b: 254 });
    }

    #[test]
    fn speed_percent_to_byte() {
        assert_eq!(percent_to_speed_byte(SpeedPercent::new(0)), SPEED_MIN);
        assert_eq!(percent_to_speed_byte(SpeedPercent::new(100)), SPEED_MAX);
        let mid = percent_to_speed_byte(SpeedPercent::new(50));
        assert!(mid > SPEED_MIN && mid < SPEED_MAX);
    }

    #[test]
    fn speed_byte_roundtrip() {
        for pct in [0u8, 25, 50, 75, 100] {
            let byte = percent_to_speed_byte(SpeedPercent::new(pct));
            let back = speed_byte_to_percent(byte);
            assert!(
                (back.value() as i16 - pct as i16).unsigned_abs() <= 1,
                "pct={pct} byte={byte} back={back}"
            );
        }
    }

    // --- rgb_to_hue16 edge cases ---

    #[test]
    fn black_to_hue16_is_zero() {
        // Black has no saturation — diff == 0, returns 0
        let hue = rgb_to_hue16(Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(hue, 0);
    }

    #[test]
    fn white_to_hue16_is_zero() {
        // White (254,254,254) also has diff==0, returns 0
        let hue = rgb_to_hue16(Rgb { r: 254, g: 254, b: 254 });
        assert_eq!(hue, 0);
    }

    #[test]
    fn grey_to_hue16_is_zero() {
        // Any grey has equal r/g/b, so diff==0
        let hue = rgb_to_hue16(Rgb { r: 128, g: 128, b: 128 });
        assert_eq!(hue, 0);
    }

    // --- hue16_to_rgb near 360 degrees ---

    #[test]
    fn hue16_to_rgb_near_360_is_red() {
        // 0xFFFF maps to nearly 360°, which wraps to red
        let rgb = hue16_to_rgb(0xFFFF);
        // Should be in the red sector: r high, b component present, g low
        // At ~360° the output is in the case _ => (RGB_PROTOCOL_MAX, 0, q) i.e. red-ish
        assert!(rgb.r > 200, "r={} should be high near 360°", rgb.r);
    }

    #[test]
    fn hue16_to_rgb_mid_range() {
        // 0x8000 is ~180°, cyan territory
        let rgb = hue16_to_rgb(0x8000);
        // At ~180° we're in the blue-to-cyan region: g and b high, r low
        assert!(rgb.g > 0 || rgb.b > 0, "expected non-zero g or b at 180°");
    }

    // --- percent_to_speed_byte edge values ---

    #[test]
    fn speed_percent_edge_1() {
        let byte = percent_to_speed_byte(SpeedPercent::new(1));
        // Should be just above SPEED_MIN
        assert!(byte > SPEED_MIN, "1% should be above SPEED_MIN");
    }

    #[test]
    fn speed_percent_edge_99() {
        let byte = percent_to_speed_byte(SpeedPercent::new(99));
        // Should be just below SPEED_MAX
        assert!(byte < SPEED_MAX, "99% should be strictly below SPEED_MAX");
        assert!(byte > SPEED_MIN, "99% should be above SPEED_MIN");
    }

    #[test]
    fn speed_percent_clamps_above_100() {
        // SpeedPercent::new clamps to 100, so both produce SPEED_MAX
        assert_eq!(
            percent_to_speed_byte(SpeedPercent::new(100)),
            percent_to_speed_byte(SpeedPercent::new(200))
        );
        assert_eq!(percent_to_speed_byte(SpeedPercent::new(255)), SPEED_MAX);
    }

    #[test]
    fn speed_byte_to_percent_at_min() {
        assert_eq!(speed_byte_to_percent(SPEED_MIN), SpeedPercent::new(0));
    }

    #[test]
    fn speed_byte_to_percent_below_min() {
        assert_eq!(speed_byte_to_percent(0), SpeedPercent::new(0));
    }

    #[test]
    fn speed_byte_to_percent_at_max() {
        assert_eq!(speed_byte_to_percent(SPEED_MAX), SpeedPercent::new(100));
    }
}
