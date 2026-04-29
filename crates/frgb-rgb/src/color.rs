use frgb_model::rgb::{Rgb, TempColorPoint};
use frgb_model::Brightness;

/// Apply brightness scaling to a color. Uses >>8 division (range 0..=254/256 ≈ 0..=99.2%).
/// `brightness=255` is special-cased as identity (no dimming).
pub fn apply_brightness(color: Rgb, brightness: Brightness) -> Rgb {
    let b = brightness.value();
    if b == 255 {
        return color;
    }
    Rgb {
        r: ((color.r as u16 * b as u16) >> 8) as u8,
        g: ((color.g as u16 * b as u16) >> 8) as u8,
        b: ((color.b as u16 * b as u16) >> 8) as u8,
    }
}

pub fn chained_brightness(color: u8, ramp: u8, brightness: Brightness) -> u8 {
    let step1 = (color as u16 * ramp as u16) >> 8;
    ((step1 * brightness.value() as u16) >> 8) as u8
}

pub fn temp_to_color(temp: f32, gradient: &[TempColorPoint]) -> Rgb {
    if gradient.is_empty() {
        return Rgb::BLACK;
    }
    if gradient.len() == 1 || temp <= gradient[0].temp.as_f32() {
        return gradient[0].color;
    }
    if temp >= gradient[gradient.len() - 1].temp.as_f32() {
        return gradient[gradient.len() - 1].color;
    }
    for w in gradient.windows(2) {
        let t0 = w[0].temp.as_f32();
        let t1 = w[1].temp.as_f32();
        if temp >= t0 && temp <= t1 {
            let range = t1 - t0;
            let frac = if range > 0.0 { (temp - t0) / range } else { 0.0 };
            return Rgb {
                r: lerp_u8(w[0].color.r, w[1].color.r, frac),
                g: lerp_u8(w[0].color.g, w[1].color.g, frac),
                b: lerp_u8(w[0].color.b, w[1].color.b, frac),
            };
        }
    }
    gradient[gradient.len() - 1].color
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round() as u8
}

/// Convert a hue (0..=65535) to an RGB color at full saturation and value.
/// Channel values are clamped to 0-254 (protocol safe).
pub fn hue_to_rgb(hue: u16) -> Rgb {
    let hue_deg = (hue as f64 / 65535.0) * 360.0;
    let h = (hue_deg / 60.0).min(5.999);
    let i = h as u32;
    let f = h - h.floor();
    let max = 254.0;
    let q = (max * (1.0 - f)) as u8;
    let t = (max * f) as u8;
    let (r, g, b) = match i {
        0 => (254, t, 0),
        1 => (q, 254, 0),
        2 => (0, 254, t),
        3 => (0, q, 254),
        4 => (t, 0, 254),
        _ => (254, 0, q),
    };
    Rgb { r, g, b }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::rgb::Rgb;
    use frgb_model::Brightness;

    #[test]
    fn apply_brightness_full() {
        let c = Rgb { r: 254, g: 100, b: 0 };
        let result = apply_brightness(c, Brightness::new(255));
        assert_eq!(result, Rgb { r: 254, g: 100, b: 0 });
    }

    #[test]
    fn apply_brightness_half() {
        let c = Rgb { r: 254, g: 100, b: 0 };
        let result = apply_brightness(c, Brightness::new(128));
        assert_eq!(result.r, 127);
        assert_eq!(result.g, 50);
        assert_eq!(result.b, 0);
    }

    #[test]
    fn apply_brightness_zero() {
        let c = Rgb { r: 254, g: 100, b: 50 };
        let result = apply_brightness(c, Brightness::new(0));
        assert_eq!(result, Rgb::BLACK);
    }

    #[test]
    fn temp_gradient_interpolation() {
        use frgb_model::rgb::TempColorPoint;
        use frgb_model::Temperature;
        let gradient = vec![
            TempColorPoint {
                temp: Temperature::new(30),
                color: Rgb { r: 0, g: 0, b: 254 },
            },
            TempColorPoint {
                temp: Temperature::new(70),
                color: Rgb { r: 254, g: 0, b: 0 },
            },
        ];
        let c = temp_to_color(30.0, &gradient);
        assert_eq!(c, Rgb { r: 0, g: 0, b: 254 });
        let c = temp_to_color(70.0, &gradient);
        assert_eq!(c, Rgb { r: 254, g: 0, b: 0 });
        let c = temp_to_color(50.0, &gradient);
        assert_eq!(c.r, 127);
        assert_eq!(c.b, 127);
    }

    #[test]
    fn chained_brightness_test() {
        let result = chained_brightness(254, 128, Brightness::new(200));
        // (254 * 128) >> 8 = 127, then (127 * 200) >> 8 = 99
        assert_eq!(result, 99);
    }
}
