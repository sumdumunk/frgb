pub mod font;
pub mod sensor;
pub mod sysinfo;
pub mod template;
pub mod text;

use frgb_model::lcd::{LcdContent, LcdSensorColor};
use image::{DynamicImage, Rgba, RgbaImage};
use std::collections::HashMap;

fn render_image(bytes: &[u8], width: u32, height: u32) -> DynamicImage {
    match image::load_from_memory(bytes) {
        Ok(img) => img.resize_exact(width, height, image::imageops::FilterType::Lanczos3),
        Err(e) => {
            tracing::warn!("render_image: failed to decode image: {e}");
            black_frame(width, height)
        }
    }
}

/// Render LCD content to an image at the given resolution.
pub fn render(content: &LcdContent, sensors: &HashMap<String, f32>, width: u32, height: u32) -> DynamicImage {
    match content {
        LcdContent::Off => black_frame(width, height),
        LcdContent::Text(s) => text::render_text(s, width, height),
        LcdContent::Sensor(display) => sensor::render_sensor(display, sensors, width, height),
        LcdContent::SensorCarousel(_) => black_frame(width, height),
        LcdContent::SystemInfo => sysinfo::render_system_info(sensors, width, height),
        LcdContent::Image(bytes) => render_image(bytes, width, height),
        LcdContent::Template(tmpl) => template::render_template(tmpl, sensors, width, height),
        LcdContent::Gif { .. }
        | LcdContent::Video(_)
        | LcdContent::Preset(_)
        | LcdContent::Clock(_)
        | LcdContent::ScreenCapture { .. } => black_frame(width, height),
    }
}

pub fn black_frame(width: u32, height: u32) -> DynamicImage {
    DynamicImage::ImageRgba8(RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 255])))
}

pub fn color_from_sensor_color(color: LcdSensorColor) -> Rgba<u8> {
    match color {
        LcdSensorColor::Blue => Rgba([100, 149, 237, 255]),
        LcdSensorColor::Green => Rgba([80, 200, 120, 255]),
        LcdSensorColor::Purple => Rgba([167, 130, 227, 255]),
        LcdSensorColor::Red => Rgba([235, 87, 87, 255]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_frame_correct_dimensions() {
        let img = black_frame(400, 400);
        assert_eq!(img.width(), 400);
        assert_eq!(img.height(), 400);
    }

    #[test]
    fn black_frame_all_black() {
        let img = black_frame(10, 10);
        let rgba = img.to_rgba8();
        for pixel in rgba.pixels() {
            assert_eq!(pixel, &Rgba([0, 0, 0, 255]));
        }
    }

    #[test]
    fn font_loads_successfully() {
        let _f = font::font();
    }

    #[test]
    fn render_off_returns_black() {
        let sensors = HashMap::new();
        let img = render(&LcdContent::Off, &sensors, 400, 400);
        assert_eq!(img.width(), 400);
    }

    #[test]
    fn render_text_dispatches() {
        let sensors = HashMap::new();
        let img = render(&LcdContent::Text("Hello".into()), &sensors, 400, 400);
        let has_nonblack = img.to_rgba8().pixels().any(|p| p[0] > 0 || p[1] > 0 || p[2] > 0);
        assert!(has_nonblack, "text content should produce visible pixels");
    }

    #[test]
    fn render_sensor_dispatches() {
        use frgb_model::lcd::{LcdSensorDisplay, LcdSensorStyle};
        use frgb_model::sensor::{Sensor, TempUnit};
        let display = LcdSensorDisplay {
            sensor: Sensor::Cpu,
            label: Some("CPU".into()),
            unit: TempUnit::Celsius,
            style: LcdSensorStyle::Gauge,
            color: LcdSensorColor::Blue,
        };
        let mut sensors = HashMap::new();
        sensors.insert("CPU".into(), 55.0);
        let img = render(&LcdContent::Sensor(display), &sensors, 400, 400);
        let has_nonblack = img.to_rgba8().pixels().any(|p| p[0] > 0 || p[1] > 0 || p[2] > 0);
        assert!(has_nonblack);
    }

    #[test]
    fn render_system_info_dispatches() {
        let mut sensors = HashMap::new();
        sensors.insert("CPU".into(), 50.0);
        let img = render(&LcdContent::SystemInfo, &sensors, 400, 400);
        let has_nonblack = img.to_rgba8().pixels().any(|p| p[0] > 0 || p[1] > 0 || p[2] > 0);
        assert!(has_nonblack);
    }

    #[test]
    fn render_image_decodes_png() {
        let mut buf = Vec::new();
        let img = RgbaImage::from_pixel(2, 2, Rgba([255, 0, 0, 255]));
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        let sensors = HashMap::new();
        let result = render(&LcdContent::Image(buf), &sensors, 400, 400);
        assert_eq!(result.width(), 400);
        assert_eq!(result.height(), 400);
    }
}
