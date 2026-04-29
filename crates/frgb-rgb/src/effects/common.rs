use crate::buffer::RgbBuffer;
use crate::layout::LedLayout;
use frgb_model::rgb::Rgb;
use frgb_model::Brightness;

pub fn fill_inner(buf: &mut RgbBuffer, frame: usize, layout: &LedLayout, fan_idx: u8, color: Rgb) {
    for led in layout.inner_range(fan_idx) {
        buf.set_led(frame, led, color);
    }
}

pub fn fill_outer(buf: &mut RgbBuffer, frame: usize, layout: &LedLayout, fan_idx: u8, color: Rgb) {
    for led in layout.outer_range(fan_idx) {
        buf.set_led(frame, led, color);
    }
}

pub fn fill_fan(buf: &mut RgbBuffer, frame: usize, layout: &LedLayout, fan_idx: u8, color: Rgb) {
    fill_inner(buf, frame, layout, fan_idx, color);
    fill_outer(buf, frame, layout, fan_idx, color);
}

/// Compute a single color channel for meteor-style exponential decay tail.
/// Formula: `(color_ch * fade * brightness) >> 16`
#[inline]
pub fn meteor_channel(color_ch: u8, fade: u8, brightness: Brightness) -> u8 {
    ((color_ch as u32 * fade as u32 * brightness.value() as u32) >> 16) as u8
}

// ---------------------------------------------------------------------------
// Shared animation primitives
// ---------------------------------------------------------------------------

/// Linearly interpolate between two colors. `t` in 0..=255 (0=a, 255=b).
#[inline]
pub fn lerp_color(a: Rgb, b: Rgb, t: u8) -> Rgb {
    let inv = 255 - t as u16;
    let t16 = t as u16;
    Rgb {
        r: ((a.r as u16 * inv + b.r as u16 * t16) / 255) as u8,
        g: ((a.g as u16 * inv + b.g as u16 * t16) / 255) as u8,
        b: ((a.b as u16 * inv + b.b as u16 * t16) / 255) as u8,
    }
}

/// Scale a color by a factor (0..=255, 255 = identity).
#[inline]
pub fn scale_color(c: Rgb, factor: u8) -> Rgb {
    if factor == 255 {
        return c;
    }
    Rgb {
        r: ((c.r as u16 * factor as u16) >> 8) as u8,
        g: ((c.g as u16 * factor as u16) >> 8) as u8,
        b: ((c.b as u16 * factor as u16) >> 8) as u8,
    }
}

/// Additive blend two colors, clamped to 254 (protocol safe).
#[inline]
pub fn add_color(a: Rgb, b: Rgb) -> Rgb {
    Rgb {
        r: a.r.saturating_add(b.r).min(254),
        g: a.g.saturating_add(b.g).min(254),
        b: a.b.saturating_add(b.b).min(254),
    }
}

/// Max-blend two colors (take brighter channel).
#[inline]
pub fn max_color(a: Rgb, b: Rgb) -> Rgb {
    Rgb {
        r: a.r.max(b.r),
        g: a.g.max(b.g),
        b: a.b.max(b.b),
    }
}

/// Ping-pong position: 0→max→0 over `period` frames.
/// Returns position in 0..max (inclusive).
#[inline]
pub fn bounce_pos(frame: usize, max: usize) -> usize {
    if max == 0 {
        return 0;
    }
    let period = max * 2;
    let t = frame % period;
    if t < max {
        t
    } else {
        period - t
    }
}

/// Rainbow color from position within a ring. Full spectrum over `total` positions.
#[inline]
pub fn rainbow_color_at(position: usize, total: usize) -> Rgb {
    if total == 0 {
        return Rgb::BLACK;
    }
    let hue = (position as u32 * 65535 / total as u32) as u16;
    crate::color::hue_to_rgb(hue)
}

/// Deterministic hash for pseudo-random effects (twinkle, disco, lottery).
/// Returns 0..=255.
#[inline]
pub fn effect_hash(a: usize, b: usize) -> u8 {
    let mut h = (a as u32).wrapping_mul(2654435761);
    h ^= (b as u32).wrapping_mul(2246822519);
    h = h.wrapping_mul(3266489917);
    (h >> 24) as u8
}

/// Tail length for meteor-style effects (head + 4 trailing LEDs).
pub const METEOR_TAIL_LEN: usize = 5;

/// How to blend a meteor pixel onto an existing buffer value.
enum MeteorBlend {
    Replace,
    Max,
    Add,
}

/// Core meteor tail implementation — paint head + exponential-decay trail.
#[allow(clippy::too_many_arguments)]
// Source: CLIPPY — too_many_arguments
// Effect rendering primitive: buf, frame, fan_base, head_pos, ring_size, ccw, color, brightness, blend
// are all independent parameters. A struct would add unnecessary allocation in a hot loop.
#[inline]
fn paint_meteor_tail_inner(
    buf: &mut RgbBuffer,
    frame: usize,
    fan_base: usize,
    head_pos: usize,
    ring_size: usize,
    ccw: bool,
    color: Rgb,
    brightness: Brightness,
    blend: MeteorBlend,
) {
    for tail_pos in 0..METEOR_TAIL_LEN {
        let fade: u8 = 255u8 >> tail_pos;
        let ring_idx = if ccw {
            (head_pos + tail_pos) % ring_size
        } else {
            (head_pos + ring_size - tail_pos) % ring_size
        };
        let led = fan_base + ring_idx;
        let c = Rgb {
            r: meteor_channel(color.r, fade, brightness),
            g: meteor_channel(color.g, fade, brightness),
            b: meteor_channel(color.b, fade, brightness),
        };
        let out = match blend {
            MeteorBlend::Replace => c,
            MeteorBlend::Max => max_color(buf.get_led(frame, led), c),
            MeteorBlend::Add => add_color(buf.get_led(frame, led), c),
        };
        buf.set_led(frame, led, out);
    }
}

/// Paint a meteor tail (direct write, no blending).
#[allow(clippy::too_many_arguments)] // Source: CLIPPY — too_many_arguments
#[inline]
pub fn paint_meteor_tail(
    buf: &mut RgbBuffer,
    frame: usize,
    fan_base: usize,
    head_pos: usize,
    ring_size: usize,
    ccw: bool,
    color: Rgb,
    brightness: Brightness,
) {
    paint_meteor_tail_inner(
        buf,
        frame,
        fan_base,
        head_pos,
        ring_size,
        ccw,
        color,
        brightness,
        MeteorBlend::Replace,
    );
}

/// Paint a meteor tail with additive blend (colors mix when overlapping).
#[allow(clippy::too_many_arguments)] // Source: CLIPPY — too_many_arguments
#[inline]
pub fn paint_meteor_tail_add(
    buf: &mut RgbBuffer,
    frame: usize,
    fan_base: usize,
    head_pos: usize,
    ring_size: usize,
    ccw: bool,
    color: Rgb,
    brightness: Brightness,
) {
    paint_meteor_tail_inner(
        buf,
        frame,
        fan_base,
        head_pos,
        ring_size,
        ccw,
        color,
        brightness,
        MeteorBlend::Add,
    );
}

/// Paint a meteor tail with max-blend (brighter channel wins).
#[allow(clippy::too_many_arguments)] // Source: CLIPPY — too_many_arguments
#[inline]
pub fn paint_meteor_tail_max(
    buf: &mut RgbBuffer,
    frame: usize,
    fan_base: usize,
    head_pos: usize,
    ring_size: usize,
    ccw: bool,
    color: Rgb,
    brightness: Brightness,
) {
    paint_meteor_tail_inner(
        buf,
        frame,
        fan_base,
        head_pos,
        ring_size,
        ccw,
        color,
        brightness,
        MeteorBlend::Max,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lerp_color_endpoints() {
        let a = Rgb { r: 254, g: 0, b: 0 };
        let b = Rgb { r: 0, g: 0, b: 254 };
        let at_a = lerp_color(a, b, 0);
        assert_eq!(at_a.r, 254);
        assert_eq!(at_a.b, 0);
        let at_b = lerp_color(a, b, 255);
        assert_eq!(at_b.r, 0);
        assert_eq!(at_b.b, 254);
    }

    #[test]
    fn lerp_color_midpoint() {
        let a = Rgb { r: 254, g: 0, b: 0 };
        let b = Rgb { r: 0, g: 0, b: 254 };
        let mid = lerp_color(a, b, 128);
        assert!(mid.r > 100 && mid.r < 150);
        assert!(mid.b > 100 && mid.b < 150);
    }

    #[test]
    fn scale_color_identity() {
        let c = Rgb { r: 200, g: 100, b: 50 };
        assert_eq!(scale_color(c, 255), c);
    }

    #[test]
    fn scale_color_half() {
        let c = Rgb { r: 200, g: 100, b: 50 };
        let s = scale_color(c, 128);
        assert_eq!(s.r, 100);
        assert_eq!(s.g, 50);
        assert_eq!(s.b, 25);
    }

    #[test]
    fn add_color_clamps() {
        let a = Rgb { r: 200, g: 200, b: 200 };
        let b = Rgb { r: 200, g: 200, b: 200 };
        let c = add_color(a, b);
        assert_eq!(c.r, 254);
    }

    #[test]
    fn bounce_pos_ping_pong() {
        assert_eq!(bounce_pos(0, 10), 0);
        assert_eq!(bounce_pos(5, 10), 5);
        assert_eq!(bounce_pos(10, 10), 10);
        assert_eq!(bounce_pos(15, 10), 5);
        assert_eq!(bounce_pos(20, 10), 0);
    }

    #[test]
    fn effect_hash_deterministic() {
        let h1 = effect_hash(42, 7);
        let h2 = effect_hash(42, 7);
        assert_eq!(h1, h2);
    }

    #[test]
    fn effect_hash_varies() {
        let vals: Vec<u8> = (0..20).map(|i| effect_hash(i, 0)).collect();
        let unique: std::collections::HashSet<u8> = vals.iter().copied().collect();
        assert!(unique.len() > 10, "hash should produce varied values");
    }
}
