use std::collections::HashMap;
use std::f32::consts::PI;

use ab_glyph::PxScale;
use frgb_model::lcd::{LcdSensorDisplay, LcdSensorStyle};
use frgb_model::sensor::TempUnit;
use image::{DynamicImage, Rgba, RgbaImage};
use imageproc::drawing::{draw_filled_rect_mut, draw_hollow_rect_mut, draw_line_segment_mut, draw_text_mut};
use imageproc::rect::Rect;

use crate::color_from_sensor_color;
use crate::font::font;
use crate::text::measure_text;

// ── sensor key lookup ────────────────────────────────────────────────────────

fn sensor_key(display: &LcdSensorDisplay) -> &'static str {
    match display.sensor {
        frgb_model::sensor::Sensor::Cpu => "CPU",
        frgb_model::sensor::Sensor::Gpu => "GPU",
        frgb_model::sensor::Sensor::GpuHotspot => "GPU Hotspot",
        frgb_model::sensor::Sensor::GpuVram => "GPU VRAM",
        frgb_model::sensor::Sensor::GpuPower => "GPU Power",
        frgb_model::sensor::Sensor::GpuUsage => "GPU Usage",
        frgb_model::sensor::Sensor::Water => "Water",
        frgb_model::sensor::Sensor::Motherboard { .. } => "MB",
        frgb_model::sensor::Sensor::Weighted { .. } => "CPU",
    }
}

fn lookup_value(display: &LcdSensorDisplay, sensors: &HashMap<String, f32>) -> Option<f32> {
    match &display.sensor {
        frgb_model::sensor::Sensor::Weighted { cpu_pct, gpu_pct } => {
            let cpu = sensors.get("CPU").copied()?;
            let gpu = sensors.get("GPU").copied()?;
            Some(cpu * (*cpu_pct as f32 / 100.0) + gpu * (*gpu_pct as f32 / 100.0))
        }
        s => sensors.get(sensor_key_for(s)).copied(),
    }
}

/// Source: INFERRED — same as sensor_key but takes a &Sensor instead of &LcdSensorDisplay.
fn sensor_key_for(sensor: &frgb_model::sensor::Sensor) -> &'static str {
    match sensor {
        frgb_model::sensor::Sensor::Cpu => "CPU",
        frgb_model::sensor::Sensor::Gpu => "GPU",
        frgb_model::sensor::Sensor::GpuHotspot => "GPU Hotspot",
        frgb_model::sensor::Sensor::GpuVram => "GPU VRAM",
        frgb_model::sensor::Sensor::GpuPower => "GPU Power",
        frgb_model::sensor::Sensor::GpuUsage => "GPU Usage",
        frgb_model::sensor::Sensor::Water => "Water",
        frgb_model::sensor::Sensor::Motherboard { .. } => "MB",
        frgb_model::sensor::Sensor::Weighted { .. } => "CPU",
    }
}

fn display_label(display: &LcdSensorDisplay) -> String {
    if let Some(ref lbl) = display.label {
        lbl.clone()
    } else {
        sensor_key(display).to_string()
    }
}

// ── value formatting ─────────────────────────────────────────────────────────

fn format_value(value: Option<f32>, unit: &TempUnit) -> String {
    format_value_for_sensor(value, unit, None)
}

fn format_value_for_sensor(value: Option<f32>, unit: &TempUnit, sensor: Option<&frgb_model::sensor::Sensor>) -> String {
    match value {
        None => "--".to_string(),
        Some(v) => {
            // Non-temperature sensors use their own unit regardless of TempUnit
            if let Some(sensor) = sensor {
                match sensor {
                    frgb_model::sensor::Sensor::GpuPower => return format!("{:.0}W", v),
                    frgb_model::sensor::Sensor::GpuUsage => return format!("{:.0}%", v),
                    _ => {}
                }
            }
            match unit {
                TempUnit::Celsius => format!("{:.0}°C", v),
                TempUnit::Fahrenheit => format!("{:.0}°F", v * 9.0 / 5.0 + 32.0),
            }
        }
    }
}

/// Clamp a temperature value to a 0-100 percentage of the range 0–100°C.
fn to_pct(value: Option<f32>) -> f32 {
    match value {
        None => 0.0,
        Some(v) => (v / 100.0).clamp(0.0, 1.0),
    }
}

// ── text helpers ─────────────────────────────────────────────────────────────

fn draw_centered_text(img: &mut RgbaImage, text: &str, color: Rgba<u8>, scale: PxScale, center_x: f32, center_y: f32) {
    let f = font();
    let (tw, th) = measure_text(text, &f, scale);
    let x = (center_x - tw / 2.0).round() as i32;
    let y = (center_y - th / 2.0).round() as i32;
    draw_text_mut(img, color, x, y, scale, &f, text);
}

// ── arc drawing ──────────────────────────────────────────────────────────────

/// Draw a circular arc from `start_deg` to `end_deg` (clockwise) at `radius`.
/// `segments` line segments approximate the arc.
#[allow(clippy::too_many_arguments)]
fn draw_arc(
    img: &mut RgbaImage,
    cx: f32,
    cy: f32,
    radius: f32,
    start_deg: f32,
    end_deg: f32,
    color: Rgba<u8>,
    segments: usize,
) {
    if segments == 0 || (end_deg - start_deg).abs() < 0.01 {
        return;
    }
    let start_rad = start_deg * PI / 180.0;
    let end_rad = end_deg * PI / 180.0;
    let step = (end_rad - start_rad) / segments as f32;

    for i in 0..segments {
        let a0 = start_rad + i as f32 * step;
        let a1 = start_rad + (i + 1) as f32 * step;
        let p0 = (cx + radius * a0.cos(), cy + radius * a0.sin());
        let p1 = (cx + radius * a1.cos(), cy + radius * a1.sin());
        draw_line_segment_mut(img, p0, p1, color);
    }
}

// ── render_gauge ─────────────────────────────────────────────────────────────

fn render_gauge(display: &LcdSensorDisplay, sensors: &HashMap<String, f32>, width: u32, height: u32) -> DynamicImage {
    let mut img = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 255]));

    let cx = width as f32 / 2.0;
    let cy = height as f32 / 2.0;
    let radius = (width.min(height) as f32 * 0.38).round();

    // Arc runs from 135° to 405° (270° sweep), clockwise
    let arc_start = 135.0_f32;
    let arc_sweep = 270.0_f32;
    let segments = 54;

    let value = lookup_value(display, sensors);
    let pct = to_pct(value);
    let filled_sweep = arc_sweep * pct;

    let dim_gray = Rgba([60, 60, 60, 255]);
    let active_color = color_from_sensor_color(display.color);

    // Draw 3 parallel arcs for thickness at offset -2, 0, +2
    for &offset in &[-2.0_f32, 0.0, 2.0] {
        let r = radius + offset;
        // Background arc (full sweep)
        draw_arc(
            &mut img,
            cx,
            cy,
            r,
            arc_start,
            arc_start + arc_sweep,
            dim_gray,
            segments,
        );
        // Filled arc (proportional)
        if filled_sweep > 0.01 {
            draw_arc(
                &mut img,
                cx,
                cy,
                r,
                arc_start,
                arc_start + filled_sweep,
                active_color,
                segments,
            );
        }
    }

    // Value text centered
    let val_text = format_value_for_sensor(value, &display.unit, Some(&display.sensor));
    let val_scale = PxScale::from(height as f32 * 0.15);
    draw_centered_text(&mut img, &val_text, Rgba([255, 255, 255, 255]), val_scale, cx, cy);

    // Label above center
    let label = display_label(display);
    let lbl_scale = PxScale::from(height as f32 * 0.07);
    draw_centered_text(&mut img, &label, active_color, lbl_scale, cx, cy - height as f32 * 0.18);

    DynamicImage::ImageRgba8(img)
}

// ── render_number ─────────────────────────────────────────────────────────────

fn render_number(display: &LcdSensorDisplay, sensors: &HashMap<String, f32>, width: u32, height: u32) -> DynamicImage {
    let mut img = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 255]));

    let cx = width as f32 / 2.0;
    let cy = height as f32 / 2.0;

    let value = lookup_value(display, sensors);
    let active_color = color_from_sensor_color(display.color);

    // Label at top third
    let label = display_label(display);
    let lbl_scale = PxScale::from(height as f32 * 0.10);
    draw_centered_text(&mut img, &label, active_color, lbl_scale, cx, cy - height as f32 * 0.20);

    // Large centered value
    let val_text = format_value_for_sensor(value, &display.unit, Some(&display.sensor));
    let val_scale = PxScale::from(height as f32 * 0.25);
    draw_centered_text(
        &mut img,
        &val_text,
        Rgba([255, 255, 255, 255]),
        val_scale,
        cx,
        cy + height as f32 * 0.05,
    );

    DynamicImage::ImageRgba8(img)
}

// ── render_graph (simple, no history) ────────────────────────────────────────

fn render_graph(display: &LcdSensorDisplay, sensors: &HashMap<String, f32>, width: u32, height: u32) -> DynamicImage {
    let mut img = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 255]));

    let active_color = color_from_sensor_color(display.color);
    let value = lookup_value(display, sensors);
    let pct = to_pct(value);

    let cx = width as f32 / 2.0;

    // Label at top
    let label = display_label(display);
    let lbl_scale = PxScale::from(height as f32 * 0.09);
    draw_centered_text(&mut img, &label, active_color, lbl_scale, cx, height as f32 * 0.12);

    // Bar geometry
    let bar_x = (width as f32 * 0.10) as i32;
    let bar_y = (height as f32 * 0.25) as i32;
    let bar_w = (width as f32 * 0.80) as u32;
    let bar_h = (height as f32 * 0.40) as u32;

    // Dim border
    let border_color = Rgba([80, 80, 80, 255]);
    if bar_w > 0 && bar_h > 0 {
        draw_hollow_rect_mut(
            &mut img,
            Rect::at(bar_x - 1, bar_y - 1).of_size(bar_w + 2, bar_h + 2),
            border_color,
        );
    }

    // Filled portion
    let filled_w = ((bar_w as f32) * pct).round() as u32;
    if filled_w > 0 && bar_h > 0 {
        draw_filled_rect_mut(&mut img, Rect::at(bar_x, bar_y).of_size(filled_w, bar_h), active_color);
    }

    // Percentage text below bar
    let pct_text = format!("{:.0}%", pct * 100.0);
    let pct_scale = PxScale::from(height as f32 * 0.09);
    draw_centered_text(
        &mut img,
        &pct_text,
        Rgba([200, 200, 200, 255]),
        pct_scale,
        cx,
        height as f32 * 0.78,
    );

    DynamicImage::ImageRgba8(img)
}

// ── render_graph_with_history (pub) ──────────────────────────────────────────

pub fn render_graph_with_history(
    label: &str,
    history: &[f32],
    current: Option<f32>,
    unit: &TempUnit,
    color: Rgba<u8>,
    width: u32,
    height: u32,
) -> DynamicImage {
    let mut img = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 255]));

    let cx = width as f32 / 2.0;

    // Label at top
    let lbl_scale = PxScale::from(height as f32 * 0.09);
    draw_centered_text(&mut img, label, color, lbl_scale, cx, height as f32 * 0.08);

    // Current value below label
    let val_text = format_value(current, unit);
    let val_scale = PxScale::from(height as f32 * 0.09);
    draw_centered_text(
        &mut img,
        &val_text,
        Rgba([200, 200, 200, 255]),
        val_scale,
        cx,
        height as f32 * 0.18,
    );

    // Graph area
    let gx = (width as f32 * 0.08) as i32;
    let gy = (height as f32 * 0.28) as i32;
    let gw = (width as f32 * 0.84) as u32;
    let gh = (height as f32 * 0.60) as u32;

    // Dim border
    let border_color = Rgba([80, 80, 80, 255]);
    if gw > 1 && gh > 1 {
        draw_hollow_rect_mut(&mut img, Rect::at(gx - 1, gy - 1).of_size(gw + 2, gh + 2), border_color);
    }

    // Plot history line
    if history.len() >= 2 {
        let n = history.len();
        let x_step = gw as f32 / (n - 1) as f32;

        for i in 0..n - 1 {
            let x0 = gx as f32 + i as f32 * x_step;
            let x1 = gx as f32 + (i + 1) as f32 * x_step;

            let pct0 = (history[i] / 100.0).clamp(0.0, 1.0);
            let pct1 = (history[i + 1] / 100.0).clamp(0.0, 1.0);

            // Y is inverted: top = 100%, bottom = 0%
            let y0 = gy as f32 + gh as f32 * (1.0 - pct0);
            let y1 = gy as f32 + gh as f32 * (1.0 - pct1);

            draw_line_segment_mut(&mut img, (x0, y0), (x1, y1), color);
        }
    }

    DynamicImage::ImageRgba8(img)
}

// ── public entry point ────────────────────────────────────────────────────────

pub fn render_sensor(
    display: &LcdSensorDisplay,
    sensors: &HashMap<String, f32>,
    width: u32,
    height: u32,
) -> DynamicImage {
    match display.style {
        LcdSensorStyle::Gauge => render_gauge(display, sensors, width, height),
        LcdSensorStyle::Number | LcdSensorStyle::Carousel => render_number(display, sensors, width, height),
        LcdSensorStyle::Graph => render_graph(display, sensors, width, height),
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::lcd::{LcdSensorColor, LcdSensorStyle};
    use frgb_model::sensor::{Sensor, TempUnit};

    fn cpu_display(style: LcdSensorStyle) -> LcdSensorDisplay {
        LcdSensorDisplay {
            sensor: Sensor::Cpu,
            label: Some("CPU".to_string()),
            unit: TempUnit::Celsius,
            style,
            color: LcdSensorColor::Blue,
        }
    }

    fn sensors_with_cpu(temp: f32) -> HashMap<String, f32> {
        let mut m = HashMap::new();
        m.insert("CPU".to_string(), temp);
        m
    }

    fn has_nonblack(img: &DynamicImage) -> bool {
        img.to_rgba8().pixels().any(|p| p[0] > 0 || p[1] > 0 || p[2] > 0)
    }

    #[test]
    fn gauge_correct_dimensions() {
        let display = cpu_display(LcdSensorStyle::Gauge);
        let sensors = sensors_with_cpu(55.0);
        let img = render_sensor(&display, &sensors, 400, 400);
        assert_eq!(img.width(), 400);
        assert_eq!(img.height(), 400);
    }

    #[test]
    fn gauge_not_all_black() {
        let display = cpu_display(LcdSensorStyle::Gauge);
        let sensors = sensors_with_cpu(55.0);
        let img = render_sensor(&display, &sensors, 400, 400);
        assert!(has_nonblack(&img), "gauge should render visible pixels");
    }

    #[test]
    fn gauge_different_values_differ() {
        let display_lo = cpu_display(LcdSensorStyle::Gauge);
        let display_hi = cpu_display(LcdSensorStyle::Gauge);
        let img_lo = render_sensor(&display_lo, &sensors_with_cpu(20.0), 400, 400).to_rgba8();
        let img_hi = render_sensor(&display_hi, &sensors_with_cpu(90.0), 400, 400).to_rgba8();
        assert_ne!(
            img_lo.as_raw(),
            img_hi.as_raw(),
            "20% and 90% gauge images should differ"
        );
    }

    #[test]
    fn number_not_all_black() {
        let display = cpu_display(LcdSensorStyle::Number);
        let sensors = sensors_with_cpu(72.0);
        let img = render_sensor(&display, &sensors, 400, 400);
        assert!(has_nonblack(&img), "number should render visible pixels");
    }

    #[test]
    fn graph_not_all_black() {
        let display = cpu_display(LcdSensorStyle::Graph);
        let sensors = sensors_with_cpu(50.0);
        let img = render_sensor(&display, &sensors, 400, 400);
        assert!(has_nonblack(&img), "graph should render visible pixels");
    }

    #[test]
    fn missing_sensor_shows_dash() {
        let display = cpu_display(LcdSensorStyle::Number);
        let sensors = HashMap::new();
        let img = render_sensor(&display, &sensors, 400, 400);
        // "--" text is still rendered so image should be non-black
        assert!(
            has_nonblack(&img),
            "missing sensor should still render '--' (non-black)"
        );
    }

    #[test]
    fn all_colors_render() {
        let colors = [
            LcdSensorColor::Blue,
            LcdSensorColor::Green,
            LcdSensorColor::Purple,
            LcdSensorColor::Red,
        ];
        for color in colors {
            let display = LcdSensorDisplay {
                sensor: Sensor::Cpu,
                label: Some("CPU".to_string()),
                unit: TempUnit::Celsius,
                style: LcdSensorStyle::Gauge,
                color,
            };
            let sensors = sensors_with_cpu(55.0);
            let img = render_sensor(&display, &sensors, 400, 400);
            assert_eq!(img.width(), 400);
            assert_eq!(img.height(), 400);
        }
    }

    #[test]
    fn graph_with_history_renders() {
        let history: Vec<f32> = (0..60).map(|i| 40.0 + (i as f32) * 0.5).collect();
        let color = color_from_sensor_color(LcdSensorColor::Blue);
        let img = render_graph_with_history("CPU", &history, Some(55.0), &TempUnit::Celsius, color, 400, 400);
        assert!(has_nonblack(&img), "history graph should render visible pixels");
    }
}
