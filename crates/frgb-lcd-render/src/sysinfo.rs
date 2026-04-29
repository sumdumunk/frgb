use std::collections::HashMap;

use ab_glyph::PxScale;
use image::{DynamicImage, Rgba, RgbaImage};
use imageproc::drawing::draw_text_mut;

use crate::font::font;
use crate::text::measure_text;

// Row definitions: (label, sensor key, color)
const ROWS: &[(&str, &str, Rgba<u8>)] = &[
    ("CPU", "CPU", Rgba([100, 149, 237, 255])),     // cornflower blue
    ("GPU", "GPU", Rgba([80, 200, 120, 255])),      // emerald
    ("Water", "Water", Rgba([100, 180, 230, 255])), // water blue
    ("MB", "MB", Rgba([140, 140, 140, 255])),       // gray
];

fn lookup_mb(sensors: &HashMap<String, f32>) -> Option<f32> {
    sensors.get("MB").or_else(|| sensors.get("Motherboard 0")).copied()
}

fn lookup_row(key: &str, sensors: &HashMap<String, f32>) -> Option<f32> {
    if key == "MB" {
        lookup_mb(sensors)
    } else {
        sensors.get(key).copied()
    }
}

fn format_temp(value: Option<f32>) -> String {
    match value {
        Some(v) => format!("{:.0}°C", v),
        None => "--".to_string(),
    }
}

pub fn render_system_info(sensors: &HashMap<String, f32>, width: u32, height: u32) -> DynamicImage {
    let mut img = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 255]));
    let f = font();

    let w = width as f32;
    let h = height as f32;

    // ── Title ────────────────────────────────────────────────────────────────
    let title = "System Info";
    let title_scale = PxScale::from(h * 0.08);
    let (title_w, _) = measure_text(title, &f, title_scale);
    let title_x = ((w - title_w) / 2.0).round() as i32;
    let title_y = (h * 0.04).round() as i32;
    draw_text_mut(
        &mut img,
        Rgba([255, 255, 255, 255]),
        title_x,
        title_y,
        title_scale,
        &f,
        title,
    );

    // ── Rows ─────────────────────────────────────────────────────────────────
    let row_start = h * 0.20;
    let row_count = ROWS.len() as f32;
    let row_spacing = (h * 0.80 - row_start) / row_count;

    let label_scale = PxScale::from(h * 0.07);
    let value_scale = PxScale::from(h * 0.11);

    let label_x = (w * 0.06).round() as i32;
    let value_right = w * 0.94;

    for (i, &(label, key, color)) in ROWS.iter().enumerate() {
        let row_center_y = row_start + (i as f32 + 0.5) * row_spacing;

        // Label (left-aligned)
        let (_, lbl_h) = measure_text(label, &f, label_scale);
        let lbl_y = (row_center_y - lbl_h / 2.0).round() as i32;
        draw_text_mut(&mut img, color, label_x, lbl_y, label_scale, &f, label);

        // Value (right-aligned)
        let value_str = format_temp(lookup_row(key, sensors));
        let (val_w, val_h) = measure_text(&value_str, &f, value_scale);
        let val_x = (value_right - val_w).round() as i32;
        let val_y = (row_center_y - val_h / 2.0).round() as i32;
        draw_text_mut(
            &mut img,
            Rgba([255, 255, 255, 255]),
            val_x,
            val_y,
            value_scale,
            &f,
            &value_str,
        );
    }

    DynamicImage::ImageRgba8(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_sensors() -> HashMap<String, f32> {
        HashMap::from([("CPU".into(), 55.0), ("GPU".into(), 68.0)])
    }

    fn has_nonblack(img: &DynamicImage) -> bool {
        img.to_rgba8().pixels().any(|p| p[0] > 0 || p[1] > 0 || p[2] > 0)
    }

    #[test]
    fn sysinfo_correct_dimensions() {
        let sensors = sample_sensors();
        let img = render_system_info(&sensors, 400, 400);
        assert_eq!(img.width(), 400);
        assert_eq!(img.height(), 400);
    }

    #[test]
    fn sysinfo_not_all_black() {
        let sensors = sample_sensors();
        let img = render_system_info(&sensors, 400, 400);
        assert!(has_nonblack(&img), "system info should render visible pixels");
    }

    #[test]
    fn sysinfo_empty_sensors_still_renders() {
        let sensors = HashMap::new();
        let img = render_system_info(&sensors, 400, 400);
        // Labels are always rendered, so image must have non-black pixels
        assert!(
            has_nonblack(&img),
            "labels should always be visible even with no sensor data"
        );
    }

    #[test]
    fn sysinfo_480x480() {
        let sensors = sample_sensors();
        let img = render_system_info(&sensors, 480, 480);
        assert_eq!(img.width(), 480);
        assert_eq!(img.height(), 480);
    }
}
