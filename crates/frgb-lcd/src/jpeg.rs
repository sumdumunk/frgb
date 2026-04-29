use std::io::Cursor;

use image::imageops::FilterType;
use image::{DynamicImage, ImageReader};

/// Resize an image to the target LCD resolution and encode as JPEG.
///
/// The image is resized to fit `width x height` using Lanczos3 filtering.
/// Non-square inputs are resized to fill (aspect ratio preserved, then center-cropped).
///
/// If the JPEG at the requested quality exceeds `MAX_JPEG_SIZE` (101,888 bytes),
/// quality is reduced in steps of 5 until it fits. Returns an error only if
/// even quality=5 produces oversized output (extremely unlikely for 400×400/480×480).
///
/// Returns the raw JPEG bytes, guaranteed to be ≤ MAX_JPEG_SIZE.
pub fn prepare_jpeg(img: &DynamicImage, width: u32, height: u32, quality: u8) -> Result<Vec<u8>, String> {
    let resized = img.resize_to_fill(width, height, FilterType::Lanczos3);
    let rgb = resized.to_rgb8();

    let mut q = quality;
    loop {
        let jpeg_buf = encode_rgb_jpeg(&rgb, q)?;

        if jpeg_buf.len() <= crate::MAX_JPEG_SIZE {
            return Ok(jpeg_buf);
        }

        if q <= 5 {
            return Err(format!(
                "JPEG output ({} bytes) exceeds MAX_JPEG_SIZE ({}) even at quality {}",
                jpeg_buf.len(),
                crate::MAX_JPEG_SIZE,
                q,
            ));
        }

        q = q.saturating_sub(5).max(5);
    }
}

fn encode_rgb_jpeg(rgb: &image::RgbImage, quality: u8) -> Result<Vec<u8>, String> {
    let mut jpeg_buf = Vec::new();
    {
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_buf, quality);
        encoder
            .encode_image(rgb)
            .map_err(|e| format!("JPEG encode failed: {e}"))?;
    }
    Ok(jpeg_buf)
}

/// Load an image from raw bytes (any supported format), resize, and encode as JPEG.
pub fn prepare_jpeg_from_bytes(data: &[u8], width: u32, height: u32, quality: u8) -> Result<Vec<u8>, String> {
    let reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|e| format!("image format detection failed: {e}"))?;
    let img = reader.decode().map_err(|e| format!("image decode failed: {e}"))?;
    prepare_jpeg(&img, width, height, quality)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_jpeg_correct_size() {
        let mut img = image::RgbImage::new(100, 100);
        for pixel in img.pixels_mut() {
            *pixel = image::Rgb([255, 0, 0]);
        }
        let jpeg = prepare_jpeg(&image::DynamicImage::from(img), 400, 400, 85).unwrap();
        assert_eq!(jpeg[0], 0xFF);
        assert_eq!(jpeg[1], 0xD8);
        assert!(jpeg.len() <= crate::MAX_JPEG_SIZE);
    }

    #[test]
    fn prepare_jpeg_from_bytes_roundtrip() {
        let mut img = image::RgbImage::new(50, 50);
        for pixel in img.pixels_mut() {
            *pixel = image::Rgb([100, 150, 200]);
        }
        let first = prepare_jpeg(&image::DynamicImage::from(img), 50, 50, 90).unwrap();
        let jpeg = prepare_jpeg_from_bytes(&first, 400, 400, 85).unwrap();
        assert_eq!(jpeg[0], 0xFF);
        assert_eq!(jpeg[1], 0xD8);
    }

    #[test]
    fn prepare_jpeg_non_square_input() {
        let img = image::RgbImage::new(800, 200);
        let jpeg = prepare_jpeg(&image::DynamicImage::from(img), 400, 400, 85).unwrap();
        assert_eq!(jpeg[0], 0xFF);
        assert_eq!(jpeg[1], 0xD8);
    }

    #[test]
    fn prepare_jpeg_quality_affects_size() {
        let mut img = image::RgbImage::new(400, 400);
        for pixel in img.pixels_mut() {
            *pixel = image::Rgb([128, 64, 200]);
        }
        let dyn_img = image::DynamicImage::from(img);
        let low_q = prepare_jpeg(&dyn_img, 400, 400, 30).unwrap();
        let high_q = prepare_jpeg(&dyn_img, 400, 400, 95).unwrap();
        assert!(low_q.len() < high_q.len());
    }

    #[test]
    fn prepare_jpeg_hydroshift_resolution() {
        let img = image::RgbImage::new(100, 100);
        let jpeg = prepare_jpeg(&image::DynamicImage::from(img), 480, 480, 85).unwrap();
        assert_eq!(jpeg[0], 0xFF);
        assert_eq!(jpeg[1], 0xD8);
    }

    #[test]
    fn prepare_jpeg_enforces_max_size() {
        // Worst case: random-ish noise image at max quality — hardest to compress
        let mut img = image::RgbImage::new(480, 480);
        for (x, y, pixel) in img.enumerate_pixels_mut() {
            let v = ((x * 7 + y * 13) % 256) as u8;
            *pixel = image::Rgb([v, v.wrapping_mul(3), v.wrapping_mul(7)]);
        }
        let jpeg = prepare_jpeg(&image::DynamicImage::from(img), 480, 480, 100).unwrap();
        assert!(
            jpeg.len() <= crate::MAX_JPEG_SIZE,
            "JPEG size {} exceeds MAX_JPEG_SIZE {}",
            jpeg.len(),
            crate::MAX_JPEG_SIZE,
        );
        // Should still be valid JPEG
        assert_eq!(jpeg[0], 0xFF);
        assert_eq!(jpeg[1], 0xD8);
    }

    #[test]
    fn prepare_jpeg_from_bytes_invalid_input() {
        let garbage = vec![0u8; 100];
        assert!(prepare_jpeg_from_bytes(&garbage, 400, 400, 85).is_err());
    }
}
