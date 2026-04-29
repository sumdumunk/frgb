use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use image::{DynamicImage, Rgba, RgbaImage};
use imageproc::drawing::draw_text_mut;

use crate::font::font;

/// Measure the pixel width and height of `text` rendered at `scale`.
pub fn measure_text(text: &str, font: &FontRef<'_>, scale: PxScale) -> (f32, f32) {
    let scaled = font.as_scaled(scale);
    let width: f32 = text.chars().enumerate().fold(0.0_f32, |acc, (i, ch)| {
        let glyph_id = scaled.glyph_id(ch);
        let advance = scaled.h_advance(glyph_id);
        let kern = if i > 0 {
            let prev = text.chars().nth(i - 1).unwrap();
            scaled.kern(scaled.glyph_id(prev), glyph_id)
        } else {
            0.0
        };
        acc + advance + kern
    });
    let height = scaled.height();
    (width, height)
}

/// Find the largest PxScale where `text` fits within 80% of `max_width`
/// and 30% of `max_height`. Minimum scale is 12.0.
fn auto_scale(text: &str, font: &FontRef<'_>, max_width: u32, max_height: u32) -> PxScale {
    let w_limit = max_width as f32 * 0.80;
    let h_limit = max_height as f32 * 0.30;
    let min_scale = 12.0_f32;

    let mut size = max_height as f32 * 0.20;

    // Scale up until we exceed a limit.
    loop {
        let (w, h) = measure_text(text, font, PxScale::from(size + 4.0));
        if w > w_limit || h > h_limit {
            break;
        }
        size += 4.0;
    }

    // Scale down until we fit within both limits.
    loop {
        let (w, h) = measure_text(text, font, PxScale::from(size));
        if (w <= w_limit && h <= h_limit) || size <= min_scale {
            break;
        }
        size -= 4.0;
    }

    PxScale::from(size.max(min_scale))
}

/// Render `text` centered on a black `width`×`height` image with white glyphs.
/// Returns a pure black frame for empty text.
pub fn render_text(text: &str, width: u32, height: u32) -> DynamicImage {
    let mut img = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 255]));

    if text.is_empty() {
        return DynamicImage::ImageRgba8(img);
    }

    let f = font();
    let scale = auto_scale(text, &f, width, height);
    let (text_w, text_h) = measure_text(text, &f, scale);

    let x = ((width as f32 - text_w) / 2.0).round() as i32;
    let y = ((height as f32 - text_h) / 2.0).round() as i32;

    draw_text_mut(&mut img, Rgba([255, 255, 255, 255]), x, y, scale, &f, text);

    DynamicImage::ImageRgba8(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_text_correct_dimensions() {
        let img = render_text("Hello", 400, 400);
        assert_eq!(img.width(), 400);
        assert_eq!(img.height(), 400);
    }

    #[test]
    fn render_text_not_all_black() {
        let img = render_text("Hello", 400, 400);
        let rgba = img.to_rgba8();
        let has_nonblack = rgba.pixels().any(|p| p[0] > 0 || p[1] > 0 || p[2] > 0);
        assert!(has_nonblack, "text should produce non-black pixels");
    }

    #[test]
    fn render_empty_text_is_black() {
        let img = render_text("", 400, 400);
        let rgba = img.to_rgba8();
        let all_black = rgba.pixels().all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0);
        assert!(all_black, "empty text should be all black");
    }

    #[test]
    fn render_text_480x480() {
        let img = render_text("GPU: 72°C", 480, 480);
        assert_eq!(img.width(), 480);
        assert_eq!(img.height(), 480);
    }
}
