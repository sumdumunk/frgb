use std::collections::HashMap;
use std::f32::consts::PI;

use ab_glyph::PxScale;
use image::{DynamicImage, Rgba, RgbaImage};
use imageproc::drawing::{draw_filled_rect_mut, draw_line_segment_mut, draw_text_mut};
use imageproc::rect::Rect;

use frgb_model::lcd::{
    BarOrientation, LcdTemplate, SensorRange, SensorSourceConfig, TemplateBackground, TextAlign, WidgetKind,
};

use crate::font::font;
use crate::text::measure_text;

// ── public entry point ──────────────────────────────────────────────────────

/// Render a template with sensor data to the target resolution.
pub fn render_template(
    template: &LcdTemplate,
    sensors: &HashMap<String, f32>,
    target_width: u32,
    target_height: u32,
) -> DynamicImage {
    let scale_x = target_width as f32 / template.base_width as f32;
    let scale_y = target_height as f32 / template.base_height as f32;
    let scale = scale_x.min(scale_y);

    let canvas_w = (template.base_width as f32 * scale).round() as u32;
    let canvas_h = (template.base_height as f32 * scale).round() as u32;

    // Render background onto canvas
    let mut canvas = render_background(&template.background, canvas_w, canvas_h);

    // Draw each visible widget
    for widget in &template.widgets {
        if !widget.visible {
            continue;
        }
        let w = (widget.width * scale).round().max(1.0) as u32;
        let h = (widget.height * scale).round().max(1.0) as u32;
        if w == 0 || h == 0 {
            continue;
        }

        let value = resolve_sensor(&widget.kind, sensors);
        let sub = draw_widget(&widget.kind, value, sensors, w, h, scale);

        // Widget position is center-based; convert to top-left
        let dst_x = (widget.x * scale - w as f32 / 2.0).round() as i32;
        let dst_y = (widget.y * scale - h as f32 / 2.0).round() as i32;

        blit_alpha(&mut canvas, &sub, dst_x, dst_y);
    }

    // Letterbox into final target
    if canvas_w == target_width && canvas_h == target_height {
        DynamicImage::ImageRgba8(canvas)
    } else {
        let mut final_img = RgbaImage::from_pixel(target_width, target_height, Rgba([0, 0, 0, 255]));
        let off_x = ((target_width - canvas_w) / 2) as i32;
        let off_y = ((target_height - canvas_h) / 2) as i32;
        blit_alpha(&mut final_img, &canvas, off_x, off_y);
        DynamicImage::ImageRgba8(final_img)
    }
}

// ── background ──────────────────────────────────────────────────────────────

fn render_background(bg: &TemplateBackground, w: u32, h: u32) -> RgbaImage {
    match bg {
        TemplateBackground::Color { rgba } => RgbaImage::from_pixel(w, h, Rgba(*rgba)),
        TemplateBackground::Image { path } => match image::open(path) {
            Ok(img) => img.resize_exact(w, h, image::imageops::FilterType::Lanczos3).to_rgba8(),
            Err(e) => {
                tracing::warn!("template background image failed: {e}");
                RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 255]))
            }
        },
    }
}

// ── sensor resolution ───────────────────────────────────────────────────────

fn resolve_sensor(kind: &WidgetKind, sensors: &HashMap<String, f32>) -> f32 {
    let source = match kind {
        WidgetKind::ValueText { source, .. }
        | WidgetKind::RadialGauge { source, .. }
        | WidgetKind::VerticalBar { source, .. }
        | WidgetKind::HorizontalBar { source, .. }
        | WidgetKind::Speedometer { source, .. } => source,
        _ => return 0.0,
    };
    match source {
        SensorSourceConfig::CpuTemp => sensors.get("CPU").copied().unwrap_or(0.0),
        SensorSourceConfig::GpuTemp => sensors.get("GPU").copied().unwrap_or(0.0),
        SensorSourceConfig::GpuUsage => sensors.get("GPU Usage").copied().unwrap_or(0.0),
        SensorSourceConfig::WaterTemp => sensors.get("Water").copied().unwrap_or(0.0),
        SensorSourceConfig::CpuUsage => sensors.get("CPU Usage").copied().unwrap_or(0.0),
        SensorSourceConfig::MemUsage => sensors.get("Memory Usage").copied().unwrap_or(0.0),
        SensorSourceConfig::Hwmon { label, .. } => sensors.get(label).copied().unwrap_or(0.0),
        SensorSourceConfig::Constant { value } => *value,
        SensorSourceConfig::Command { cmd } => {
            // Execute command and parse stdout as f32.
            // Security: runs as daemon user, config is user-owned (0600).
            match std::process::Command::new("sh").args(["-c", cmd]).output() {
                Ok(output) => String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .parse::<f32>()
                    .unwrap_or(0.0),
                Err(_) => 0.0,
            }
        }
    }
}

// ── range color helper ──────────────────────────────────────────────────────

fn range_color(ranges: &[SensorRange], value: f32) -> Rgba<u8> {
    if ranges.is_empty() {
        return Rgba([100, 200, 255, 255]);
    }
    for range in ranges {
        if let Some(max_val) = range.max {
            if value <= max_val {
                return Rgba([range.color[0], range.color[1], range.color[2], range.alpha]);
            }
        } else {
            return Rgba([range.color[0], range.color[1], range.color[2], range.alpha]);
        }
    }
    let last = ranges.last().unwrap();
    Rgba([last.color[0], last.color[1], last.color[2], last.alpha])
}

// ── alpha blitting ──────────────────────────────────────────────────────────

fn blit_alpha(dst: &mut RgbaImage, src: &RgbaImage, dx: i32, dy: i32) {
    let (dw, dh) = (dst.width() as i32, dst.height() as i32);
    let (sw, sh) = (src.width() as i32, src.height() as i32);

    for sy in 0..sh {
        let ty = dy + sy;
        if ty < 0 || ty >= dh {
            continue;
        }
        for sx in 0..sw {
            let tx = dx + sx;
            if tx < 0 || tx >= dw {
                continue;
            }
            let sp = src.get_pixel(sx as u32, sy as u32);
            let sa = sp[3] as u32;
            if sa == 0 {
                continue;
            }
            if sa == 255 {
                dst.put_pixel(tx as u32, ty as u32, *sp);
            } else {
                let dp = dst.get_pixel(tx as u32, ty as u32);
                let inv = 255 - sa;
                let r = (sp[0] as u32 * sa + dp[0] as u32 * inv) / 255;
                let g = (sp[1] as u32 * sa + dp[1] as u32 * inv) / 255;
                let b = (sp[2] as u32 * sa + dp[2] as u32 * inv) / 255;
                let a = (sa + dp[3] as u32 * inv / 255).min(255);
                dst.put_pixel(tx as u32, ty as u32, Rgba([r as u8, g as u8, b as u8, a as u8]));
            }
        }
    }
}

// ── widget dispatch ─────────────────────────────────────────────────────────

fn draw_widget(kind: &WidgetKind, value: f32, sensors: &HashMap<String, f32>, w: u32, h: u32, scale: f32) -> RgbaImage {
    match kind {
        WidgetKind::Label {
            text,
            font_size,
            color,
            align,
        } => draw_label(text, *font_size * scale, color, *align, w, h),
        WidgetKind::ValueText {
            source: _,
            format,
            unit,
            font_size,
            color,
            align,
            value_min: _,
            value_max: _,
            ranges,
        } => draw_value_text(value, format, unit, *font_size * scale, color, *align, ranges, w, h),
        WidgetKind::RadialGauge {
            source: _,
            value_min,
            value_max,
            start_angle,
            sweep_angle,
            inner_radius_pct,
            background_color,
            ranges,
        } => draw_radial_gauge(
            value,
            *value_min,
            *value_max,
            *start_angle,
            *sweep_angle,
            *inner_radius_pct,
            background_color,
            ranges,
            w,
            h,
        ),
        WidgetKind::VerticalBar {
            source: _,
            value_min,
            value_max,
            background_color,
            corner_radius: _,
            ranges,
        } => draw_vertical_bar(value, *value_min, *value_max, background_color, ranges, w, h),
        WidgetKind::HorizontalBar {
            source: _,
            value_min,
            value_max,
            background_color,
            corner_radius: _,
            ranges,
        } => draw_horizontal_bar(value, *value_min, *value_max, background_color, ranges, w, h),
        WidgetKind::Speedometer {
            source: _,
            value_min,
            value_max,
            start_angle,
            sweep_angle,
            needle_color,
            tick_color,
            tick_count,
            background_color,
            ranges,
        } => draw_speedometer(
            value,
            *value_min,
            *value_max,
            *start_angle,
            *sweep_angle,
            needle_color,
            tick_color,
            *tick_count,
            background_color,
            ranges,
            w,
            h,
        ),
        WidgetKind::CoreBars {
            sources,
            orientation,
            background_color,
            show_labels,
            ranges,
        } => draw_core_bars(
            sources,
            sensors,
            *orientation,
            background_color,
            *show_labels,
            ranges,
            w,
            h,
        ),
        WidgetKind::Image { path, opacity } => draw_image_widget(path, *opacity, w, h),
    }
}

// ── Label ───────────────────────────────────────────────────────────────────

fn draw_label(text: &str, font_size: f32, color: &[u8; 4], align: TextAlign, w: u32, h: u32) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 0]));
    if text.is_empty() || font_size < 1.0 {
        return img;
    }
    let f = font();
    let scale = PxScale::from(font_size.max(4.0));
    let (tw, th) = measure_text(text, &f, scale);
    let x = match align {
        TextAlign::Left => 0,
        TextAlign::Center => ((w as f32 - tw) / 2.0).round() as i32,
        TextAlign::Right => (w as f32 - tw).round() as i32,
    };
    let y = ((h as f32 - th) / 2.0).round() as i32;
    draw_text_mut(&mut img, Rgba(*color), x, y, scale, &f, text);
    img
}

// ── ValueText ───────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_value_text(
    value: f32,
    format: &str,
    unit: &str,
    font_size: f32,
    _default_color: &[u8; 4],
    align: TextAlign,
    ranges: &[SensorRange],
    w: u32,
    h: u32,
) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 0]));

    // Format: single-pass replace — check which specifier is present and apply it
    let formatted = if format.contains("{:.2}") {
        format.replace("{:.2}", &format!("{:.2}", value))
    } else if format.contains("{:.1}") {
        format.replace("{:.1}", &format!("{:.1}", value))
    } else if format.contains("{:.0}") {
        format.replace("{:.0}", &format!("{:.0}", value))
    } else if format.contains("{}") {
        format.replace("{}", &format!("{:.0}", value))
    } else {
        format!("{:.0}", value)
    };

    let display_text = if unit.is_empty() {
        formatted
    } else {
        format!("{}{}", formatted, unit)
    };

    let color = range_color(ranges, value);
    let f = font();
    let scale = PxScale::from(font_size.max(4.0));
    let (tw, th) = measure_text(&display_text, &f, scale);
    let x = match align {
        TextAlign::Left => 0,
        TextAlign::Center => ((w as f32 - tw) / 2.0).round() as i32,
        TextAlign::Right => (w as f32 - tw).round() as i32,
    };
    let y = ((h as f32 - th) / 2.0).round() as i32;
    draw_text_mut(&mut img, color, x, y, scale, &f, &display_text);
    img
}

// ── RadialGauge ─────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_radial_gauge(
    value: f32,
    value_min: f32,
    value_max: f32,
    start_angle: f32,
    sweep_angle: f32,
    inner_radius_pct: f32,
    background_color: &[u8; 4],
    ranges: &[SensorRange],
    w: u32,
    h: u32,
) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 0]));

    let cx = w as f32 / 2.0;
    let cy = h as f32 / 2.0;
    let outer_r = w.min(h) as f32 / 2.0;
    let inner_r = outer_r * inner_radius_pct;

    let range_span = if value_max > value_min {
        value_max - value_min
    } else {
        1.0
    };
    let pct = ((value - value_min) / range_span).clamp(0.0, 1.0);
    let filled_sweep = sweep_angle * pct;

    let bg_color = Rgba(*background_color);
    let fill_color = range_color(ranges, value);

    // Convert angles: start_angle is in degrees, 0 = right, clockwise
    let start_rad = start_angle * PI / 180.0;

    for py in 0..h {
        for px in 0..w {
            let dx = px as f32 - cx;
            let dy = py as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt();

            if dist < inner_r || dist > outer_r {
                continue;
            }

            // Angle from center, clockwise from right
            let mut angle = dy.atan2(dx);
            if angle < 0.0 {
                angle += 2.0 * PI;
            }

            // Normalize angle relative to start
            let mut rel = angle - start_rad;
            if rel < 0.0 {
                rel += 2.0 * PI;
            }

            let sweep_rad = sweep_angle * PI / 180.0;
            let filled_rad = filled_sweep * PI / 180.0;

            if rel <= sweep_rad {
                if rel <= filled_rad {
                    img.put_pixel(px, py, fill_color);
                } else {
                    img.put_pixel(px, py, bg_color);
                }
            }
        }
    }

    img
}

// ── HorizontalBar ───────────────────────────────────────────────────────────

fn draw_horizontal_bar(
    value: f32,
    value_min: f32,
    value_max: f32,
    background_color: &[u8; 4],
    ranges: &[SensorRange],
    w: u32,
    h: u32,
) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(w, h, Rgba(*background_color));

    let range_span = if value_max > value_min {
        value_max - value_min
    } else {
        1.0
    };
    let pct = ((value - value_min) / range_span).clamp(0.0, 1.0);
    let filled_w = (w as f32 * pct).round() as u32;

    if filled_w > 0 {
        let color = range_color(ranges, value);
        draw_filled_rect_mut(&mut img, Rect::at(0, 0).of_size(filled_w, h), color);
    }

    img
}

// ── VerticalBar ─────────────────────────────────────────────────────────────

fn draw_vertical_bar(
    value: f32,
    value_min: f32,
    value_max: f32,
    background_color: &[u8; 4],
    ranges: &[SensorRange],
    w: u32,
    h: u32,
) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(w, h, Rgba(*background_color));

    let range_span = if value_max > value_min {
        value_max - value_min
    } else {
        1.0
    };
    let pct = ((value - value_min) / range_span).clamp(0.0, 1.0);
    let filled_h = (h as f32 * pct).round() as u32;

    if filled_h > 0 {
        let color = range_color(ranges, value);
        let y = h.saturating_sub(filled_h) as i32;
        draw_filled_rect_mut(&mut img, Rect::at(0, y).of_size(w, filled_h), color);
    }

    img
}

// ── Speedometer ─────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_speedometer(
    value: f32,
    value_min: f32,
    value_max: f32,
    start_angle: f32,
    sweep_angle: f32,
    needle_color: &[u8; 4],
    tick_color: &[u8; 4],
    tick_count: u32,
    background_color: &[u8; 4],
    ranges: &[SensorRange],
    w: u32,
    h: u32,
) -> RgbaImage {
    // Draw the radial gauge as the base
    let mut img = draw_radial_gauge(
        value,
        value_min,
        value_max,
        start_angle,
        sweep_angle,
        0.85,
        background_color,
        ranges,
        w,
        h,
    );

    let cx = w as f32 / 2.0;
    let cy = h as f32 / 2.0;
    let radius = w.min(h) as f32 / 2.0;

    // Draw tick marks
    let tick_rgba = Rgba(*tick_color);
    let start_rad = start_angle * PI / 180.0;
    let sweep_rad = sweep_angle * PI / 180.0;

    if tick_count > 1 {
        let tick_step = sweep_rad / (tick_count - 1) as f32;
        let outer_tick = radius * 0.95;
        let inner_tick = radius * 0.82;

        for i in 0..tick_count {
            let angle = start_rad + i as f32 * tick_step;
            let x0 = cx + inner_tick * angle.cos();
            let y0 = cy + inner_tick * angle.sin();
            let x1 = cx + outer_tick * angle.cos();
            let y1 = cy + outer_tick * angle.sin();
            draw_line_segment_mut(&mut img, (x0, y0), (x1, y1), tick_rgba);
        }
    }

    // Draw needle
    let range_span = if value_max > value_min {
        value_max - value_min
    } else {
        1.0
    };
    let pct = ((value - value_min) / range_span).clamp(0.0, 1.0);
    let needle_angle = start_rad + sweep_rad * pct;
    let needle_len = radius * 0.75;
    let needle_rgba = Rgba(*needle_color);

    let nx = cx + needle_len * needle_angle.cos();
    let ny = cy + needle_len * needle_angle.sin();
    draw_line_segment_mut(&mut img, (cx, cy), (nx, ny), needle_rgba);

    // Draw thicker needle by offsetting
    for offset in [-1.0_f32, 1.0] {
        let perp_angle = needle_angle + PI / 2.0;
        let ox = offset * perp_angle.cos();
        let oy = offset * perp_angle.sin();
        draw_line_segment_mut(&mut img, (cx + ox, cy + oy), (nx + ox, ny + oy), needle_rgba);
    }

    img
}

// ── CoreBars ────────────────────────────────────────────────────────────────

fn resolve_source(source: &SensorSourceConfig, sensors: &HashMap<String, f32>) -> f32 {
    match source {
        SensorSourceConfig::CpuTemp => sensors.get("CPU").copied().unwrap_or(0.0),
        SensorSourceConfig::GpuTemp => sensors.get("GPU").copied().unwrap_or(0.0),
        SensorSourceConfig::GpuUsage => sensors.get("GPU Usage").copied().unwrap_or(0.0),
        SensorSourceConfig::WaterTemp => sensors.get("Water").copied().unwrap_or(0.0),
        SensorSourceConfig::CpuUsage => sensors.get("CPU Usage").copied().unwrap_or(0.0),
        SensorSourceConfig::MemUsage => sensors.get("Memory Usage").copied().unwrap_or(0.0),
        SensorSourceConfig::Hwmon { label, .. } => sensors.get(label).copied().unwrap_or(0.0),
        SensorSourceConfig::Constant { value } => *value,
        SensorSourceConfig::Command { cmd } => {
            // Execute command and parse stdout as f32.
            // Security: runs as daemon user, config is user-owned (0600).
            match std::process::Command::new("sh").args(["-c", cmd]).output() {
                Ok(output) => String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .parse::<f32>()
                    .unwrap_or(0.0),
                Err(_) => 0.0,
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_core_bars(
    sources: &[SensorSourceConfig],
    sensors: &HashMap<String, f32>,
    orientation: BarOrientation,
    background_color: &[u8; 4],
    show_labels: bool,
    ranges: &[SensorRange],
    w: u32,
    h: u32,
) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(w, h, Rgba(*background_color));

    // Static labels avoid per-tick format! allocations
    const CORE_LABELS: [&str; 16] = [
        "CPU Core 0",
        "CPU Core 1",
        "CPU Core 2",
        "CPU Core 3",
        "CPU Core 4",
        "CPU Core 5",
        "CPU Core 6",
        "CPU Core 7",
        "CPU Core 8",
        "CPU Core 9",
        "CPU Core 10",
        "CPU Core 11",
        "CPU Core 12",
        "CPU Core 13",
        "CPU Core 14",
        "CPU Core 15",
    ];

    // Resolve per-core values from explicit sources, or fall back to CPU Core N keys
    let core_values: Vec<f32> = if sources.is_empty() {
        // Default: read static core labels from sensors
        CORE_LABELS
            .iter()
            .map(|label| sensors.get(*label).copied().unwrap_or(0.0))
            .collect()
    } else {
        sources.iter().map(|s| resolve_source(s, sensors)).collect()
    };
    let core_count = core_values.len().max(1) as u32;

    let f = font();
    let label_height = if show_labels { (h as f32 * 0.12).max(10.0) } else { 0.0 };
    let label_scale = PxScale::from(label_height.clamp(6.0, 14.0));

    match orientation {
        BarOrientation::Horizontal => {
            let bar_h = ((h as f32 - label_height) / core_count as f32).floor().max(1.0);
            let gap = 1.0_f32;

            for (i, &val) in core_values.iter().enumerate() {
                let by = (i as f32 * (bar_h + gap)).round() as i32;
                let fill_w = (w as f32 * (val / 100.0).clamp(0.0, 1.0)).round() as u32;
                let bar_h_u = bar_h.round() as u32;
                let color = range_color(ranges, val);

                if fill_w > 0 && bar_h_u > 0 {
                    draw_filled_rect_mut(&mut img, Rect::at(0, by).of_size(fill_w, bar_h_u), color);
                }

                if show_labels {
                    let label = format!("C{}", i);
                    let lx = 2;
                    let ly = by + (bar_h * 0.1) as i32;
                    draw_text_mut(&mut img, Rgba([200, 200, 200, 255]), lx, ly, label_scale, &f, &label);
                }
            }
        }
        BarOrientation::Vertical => {
            let bar_w = ((w as f32) / core_count as f32).floor().max(1.0);
            let gap = 1.0_f32;

            for (i, &val) in core_values.iter().enumerate() {
                let bx = (i as f32 * (bar_w + gap)).round() as i32;
                let fill_h = (h as f32 * (val / 100.0).clamp(0.0, 1.0)).round() as u32;
                let bar_w_u = bar_w.round() as u32;
                let color = range_color(ranges, val);

                if fill_h > 0 && bar_w_u > 0 {
                    let by = h.saturating_sub(fill_h) as i32;
                    draw_filled_rect_mut(&mut img, Rect::at(bx, by).of_size(bar_w_u, fill_h), color);
                }

                if show_labels {
                    let label = format!("{}", i);
                    let lx = bx + (bar_w * 0.2) as i32;
                    let ly = (h as f32 - label_height) as i32;
                    draw_text_mut(&mut img, Rgba([200, 200, 200, 255]), lx, ly, label_scale, &f, &label);
                }
            }
        }
    }

    img
}

// ── Image widget ────────────────────────────────────────────────────────────

fn draw_image_widget(path: &str, opacity: f32, w: u32, h: u32) -> RgbaImage {
    match image::open(path) {
        Ok(img) => {
            let resized = img.resize(w, h, image::imageops::FilterType::Lanczos3).to_rgba8();

            if (opacity - 1.0).abs() < 0.01 {
                // Center the image if it's smaller than the widget
                let mut out = RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 0]));
                let ox = (w.saturating_sub(resized.width())) / 2;
                let oy = (h.saturating_sub(resized.height())) / 2;
                for (x, y, px) in resized.enumerate_pixels() {
                    let tx = ox + x;
                    let ty = oy + y;
                    if tx < w && ty < h {
                        out.put_pixel(tx, ty, *px);
                    }
                }
                out
            } else {
                let alpha_mul = (opacity * 255.0).round() as u8;
                let mut out = RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 0]));
                let ox = (w.saturating_sub(resized.width())) / 2;
                let oy = (h.saturating_sub(resized.height())) / 2;
                for (x, y, px) in resized.enumerate_pixels() {
                    let tx = ox + x;
                    let ty = oy + y;
                    if tx < w && ty < h {
                        let a = (px[3] as u32 * alpha_mul as u32 / 255) as u8;
                        out.put_pixel(tx, ty, Rgba([px[0], px[1], px[2], a]));
                    }
                }
                out
            }
        }
        Err(e) => {
            tracing::warn!("image widget load failed: {e}");
            RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 0]))
        }
    }
}

// ── dirty-flag change detection ────────────────────────────────────────────

/// Quantize a sensor value to integer at given precision multiplier.
/// precision=10 gives 0.1 resolution.
fn quantize(value: f32, precision: i32) -> i64 {
    (value * precision as f32).round() as i64
}

/// Tracks quantized sensor state per widget for change detection.
pub struct TemplateState {
    widget_values: Vec<Option<i64>>,
}

impl TemplateState {
    pub fn new(template: &LcdTemplate) -> Self {
        Self {
            widget_values: vec![None; template.widgets.len()],
        }
    }

    /// Returns true if any visible widget's sensor value changed since last render.
    /// Always returns true on first call (before any `mark_rendered`).
    pub fn needs_render(&self, template: &LcdTemplate, sensors: &HashMap<String, f32>) -> bool {
        // First render: no values stored yet → always render
        if self.widget_values.iter().all(|v| v.is_none()) {
            return true;
        }
        for (i, widget) in template.widgets.iter().enumerate() {
            if !widget.visible {
                continue;
            }
            let value = resolve_sensor(&widget.kind, sensors);
            let quantized = quantize(value, 10);
            match self.widget_values.get(i) {
                Some(Some(prev)) if *prev == quantized => continue,
                _ => return true,
            }
        }
        false
    }

    /// Update stored values after a successful render.
    pub fn mark_rendered(&mut self, template: &LcdTemplate, sensors: &HashMap<String, f32>) {
        for (i, widget) in template.widgets.iter().enumerate() {
            let value = resolve_sensor(&widget.kind, sensors);
            let quantized = quantize(value, 10);
            if i < self.widget_values.len() {
                self.widget_values[i] = Some(quantized);
            }
        }
    }
}

// ── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::lcd::*;
    use frgb_model::ValidatedName;

    fn test_template() -> LcdTemplate {
        LcdTemplate {
            id: "test".into(),
            name: ValidatedName::new("Test").unwrap(),
            base_width: 480,
            base_height: 480,
            background: TemplateBackground::Color {
                rgba: [20, 20, 30, 255],
            },
            widgets: vec![
                Widget {
                    id: "label".into(),
                    kind: WidgetKind::Label {
                        text: "CPU".into(),
                        font_size: 24.0,
                        color: [255, 255, 255, 255],
                        align: TextAlign::Center,
                    },
                    x: 240.0,
                    y: 50.0,
                    width: 200.0,
                    height: 40.0,
                    rotation: 0.0,
                    visible: true,
                    update_interval_ms: None,
                },
                Widget {
                    id: "gauge".into(),
                    kind: WidgetKind::RadialGauge {
                        source: SensorSourceConfig::CpuTemp,
                        value_min: 20.0,
                        value_max: 100.0,
                        start_angle: 135.0,
                        sweep_angle: 270.0,
                        inner_radius_pct: 0.78,
                        background_color: [40, 40, 40, 255],
                        ranges: vec![
                            SensorRange {
                                max: Some(60.0),
                                color: [0, 200, 100],
                                alpha: 255,
                            },
                            SensorRange {
                                max: None,
                                color: [255, 50, 50],
                                alpha: 255,
                            },
                        ],
                    },
                    x: 240.0,
                    y: 260.0,
                    width: 300.0,
                    height: 300.0,
                    rotation: 0.0,
                    visible: true,
                    update_interval_ms: Some(1000),
                },
            ],
        }
    }

    #[test]
    fn render_template_correct_dimensions() {
        let tmpl = test_template();
        let sensors = HashMap::from([("CPU".into(), 55.0f32)]);
        let img = render_template(&tmpl, &sensors, 400, 400);
        assert_eq!(img.width(), 400);
        assert_eq!(img.height(), 400);
    }

    #[test]
    fn render_template_has_visible_content() {
        let tmpl = test_template();
        let sensors = HashMap::from([("CPU".into(), 55.0f32)]);
        let img = render_template(&tmpl, &sensors, 400, 400);
        let rgba = img.to_rgba8();
        let non_bg = rgba.pixels().filter(|p| p[0] != 20 || p[1] != 20 || p[2] != 30).count();
        assert!(
            non_bg > 100,
            "should have visible widget content, got {non_bg} non-bg pixels"
        );
    }

    #[test]
    fn render_bar_widget() {
        let tmpl = LcdTemplate {
            id: "bar".into(),
            name: ValidatedName::new("Bar").unwrap(),
            base_width: 200,
            base_height: 200,
            background: TemplateBackground::Color { rgba: [0, 0, 0, 255] },
            widgets: vec![Widget {
                id: "bar".into(),
                kind: WidgetKind::HorizontalBar {
                    source: SensorSourceConfig::Constant { value: 75.0 },
                    value_min: 0.0,
                    value_max: 100.0,
                    background_color: [30, 30, 30, 255],
                    corner_radius: 0.0,
                    ranges: vec![SensorRange {
                        max: None,
                        color: [0, 255, 0],
                        alpha: 255,
                    }],
                },
                x: 100.0,
                y: 100.0,
                width: 160.0,
                height: 20.0,
                rotation: 0.0,
                visible: true,
                update_interval_ms: None,
            }],
        };
        let img = render_template(&tmpl, &HashMap::new(), 200, 200);
        assert_eq!(img.width(), 200);
    }

    #[test]
    fn invisible_widget_skipped() {
        let tmpl = LcdTemplate {
            id: "t".into(),
            name: ValidatedName::new("T").unwrap(),
            base_width: 100,
            base_height: 100,
            background: TemplateBackground::Color { rgba: [0, 0, 0, 255] },
            widgets: vec![Widget {
                id: "hidden".into(),
                kind: WidgetKind::Label {
                    text: "HIDDEN".into(),
                    font_size: 30.0,
                    color: [255, 255, 255, 255],
                    align: TextAlign::Center,
                },
                x: 50.0,
                y: 50.0,
                width: 80.0,
                height: 30.0,
                rotation: 0.0,
                visible: false,
                update_interval_ms: None,
            }],
        };
        let img = render_template(&tmpl, &HashMap::new(), 100, 100);
        let rgba = img.to_rgba8();
        let all_black = rgba.pixels().all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0);
        assert!(all_black, "invisible widget should not produce visible pixels");
    }

    #[test]
    fn render_vertical_bar() {
        let tmpl = LcdTemplate {
            id: "vbar".into(),
            name: ValidatedName::new("VBar").unwrap(),
            base_width: 100,
            base_height: 200,
            background: TemplateBackground::Color { rgba: [0, 0, 0, 255] },
            widgets: vec![Widget {
                id: "vb".into(),
                kind: WidgetKind::VerticalBar {
                    source: SensorSourceConfig::Constant { value: 50.0 },
                    value_min: 0.0,
                    value_max: 100.0,
                    background_color: [20, 20, 20, 255],
                    corner_radius: 0.0,
                    ranges: vec![SensorRange {
                        max: None,
                        color: [0, 150, 255],
                        alpha: 255,
                    }],
                },
                x: 50.0,
                y: 100.0,
                width: 30.0,
                height: 160.0,
                rotation: 0.0,
                visible: true,
                update_interval_ms: None,
            }],
        };
        let img = render_template(&tmpl, &HashMap::new(), 100, 200);
        assert_eq!(img.width(), 100);
        assert_eq!(img.height(), 200);
    }

    #[test]
    fn render_value_text() {
        let tmpl = LcdTemplate {
            id: "vt".into(),
            name: ValidatedName::new("VT").unwrap(),
            base_width: 200,
            base_height: 60,
            background: TemplateBackground::Color { rgba: [0, 0, 0, 255] },
            widgets: vec![Widget {
                id: "val".into(),
                kind: WidgetKind::ValueText {
                    source: SensorSourceConfig::CpuTemp,
                    format: "{:.0}".into(),
                    unit: "°C".into(),
                    font_size: 24.0,
                    color: [255, 255, 255, 255],
                    align: TextAlign::Center,
                    value_min: 0.0,
                    value_max: 100.0,
                    ranges: vec![
                        SensorRange {
                            max: Some(60.0),
                            color: [0, 200, 100],
                            alpha: 255,
                        },
                        SensorRange {
                            max: None,
                            color: [255, 50, 50],
                            alpha: 255,
                        },
                    ],
                },
                x: 100.0,
                y: 30.0,
                width: 180.0,
                height: 40.0,
                rotation: 0.0,
                visible: true,
                update_interval_ms: None,
            }],
        };
        let sensors = HashMap::from([("CPU".into(), 45.0f32)]);
        let img = render_template(&tmpl, &sensors, 200, 60);
        let rgba = img.to_rgba8();
        let has_nonblack = rgba.pixels().any(|p| p[0] > 0 || p[1] > 0 || p[2] > 0);
        assert!(has_nonblack, "value text should produce visible pixels");
    }

    #[test]
    fn render_speedometer() {
        let tmpl = LcdTemplate {
            id: "speedo".into(),
            name: ValidatedName::new("Speedo").unwrap(),
            base_width: 300,
            base_height: 300,
            background: TemplateBackground::Color { rgba: [0, 0, 0, 255] },
            widgets: vec![Widget {
                id: "sp".into(),
                kind: WidgetKind::Speedometer {
                    source: SensorSourceConfig::Constant { value: 65.0 },
                    value_min: 0.0,
                    value_max: 100.0,
                    start_angle: 135.0,
                    sweep_angle: 270.0,
                    needle_color: [255, 255, 255, 255],
                    tick_color: [180, 180, 180, 255],
                    tick_count: 10,
                    background_color: [30, 30, 30, 255],
                    ranges: vec![SensorRange {
                        max: None,
                        color: [100, 200, 255],
                        alpha: 255,
                    }],
                },
                x: 150.0,
                y: 150.0,
                width: 260.0,
                height: 260.0,
                rotation: 0.0,
                visible: true,
                update_interval_ms: None,
            }],
        };
        let img = render_template(&tmpl, &HashMap::new(), 300, 300);
        assert_eq!(img.width(), 300);
    }

    #[test]
    fn range_color_selects_correctly() {
        let ranges = vec![
            SensorRange {
                max: Some(60.0),
                color: [0, 255, 0],
                alpha: 255,
            },
            SensorRange {
                max: None,
                color: [255, 0, 0],
                alpha: 255,
            },
        ];
        assert_eq!(range_color(&ranges, 50.0), Rgba([0, 255, 0, 255]));
        assert_eq!(range_color(&ranges, 70.0), Rgba([255, 0, 0, 255]));
        assert_eq!(range_color(&ranges, 60.0), Rgba([0, 255, 0, 255]));
    }

    #[test]
    fn range_color_empty_returns_default() {
        assert_eq!(range_color(&[], 50.0), Rgba([100, 200, 255, 255]));
    }

    #[test]
    fn letterbox_nonsquare_target() {
        let tmpl = LcdTemplate {
            id: "sq".into(),
            name: ValidatedName::new("Sq").unwrap(),
            base_width: 480,
            base_height: 480,
            background: TemplateBackground::Color {
                rgba: [50, 50, 50, 255],
            },
            widgets: vec![],
        };
        let img = render_template(&tmpl, &HashMap::new(), 640, 480);
        assert_eq!(img.width(), 640);
        assert_eq!(img.height(), 480);
        // The 480x480 base should scale to 480x480 and be centered in 640x480
        // leaving 80px black bars on each side
        let rgba = img.to_rgba8();
        let corner = rgba.get_pixel(0, 0);
        assert_eq!(corner, &Rgba([0, 0, 0, 255]), "letterbox should have black bars");
    }

    // ── TemplateState dirty-flag tests ──────────────────────────────────────

    #[test]
    fn quantize_sensor_value() {
        assert_eq!(quantize(55.32, 10), 553);
        assert_eq!(quantize(55.37, 10), 554);
        assert_eq!(quantize(55.32, 10), quantize(55.32, 10));
        assert_ne!(quantize(55.32, 10), quantize(55.42, 10));
    }

    #[test]
    fn template_state_detects_change() {
        let template = LcdTemplate {
            id: "t1".into(),
            name: ValidatedName::new("Test").unwrap(),
            base_width: 480,
            base_height: 480,
            background: TemplateBackground::Color { rgba: [0, 0, 0, 255] },
            widgets: vec![Widget {
                id: "gauge".into(),
                kind: WidgetKind::RadialGauge {
                    source: SensorSourceConfig::CpuTemp,
                    value_min: 0.0,
                    value_max: 100.0,
                    start_angle: 135.0,
                    sweep_angle: 270.0,
                    inner_radius_pct: 0.78,
                    background_color: [40, 40, 40, 255],
                    ranges: vec![],
                },
                x: 240.0,
                y: 240.0,
                width: 300.0,
                height: 300.0,
                rotation: 0.0,
                visible: true,
                update_interval_ms: Some(1000),
            }],
        };
        let mut state = TemplateState::new(&template);
        let s1 = HashMap::from([("CPU".into(), 55.3f32)]);
        let s2 = HashMap::from([("CPU".into(), 55.31f32)]);
        let s3 = HashMap::from([("CPU".into(), 56.5f32)]);

        assert!(state.needs_render(&template, &s1));
        state.mark_rendered(&template, &s1);
        assert!(!state.needs_render(&template, &s2));
        assert!(state.needs_render(&template, &s3));
    }

    #[test]
    fn template_state_invisible_widget_ignored() {
        let template = LcdTemplate {
            id: "t".into(),
            name: ValidatedName::new("T").unwrap(),
            base_width: 100,
            base_height: 100,
            background: TemplateBackground::Color { rgba: [0, 0, 0, 255] },
            widgets: vec![Widget {
                id: "hidden".into(),
                kind: WidgetKind::ValueText {
                    source: SensorSourceConfig::CpuTemp,
                    format: "{:.0}".into(),
                    unit: "°C".into(),
                    font_size: 24.0,
                    color: [255, 255, 255, 255],
                    align: TextAlign::Center,
                    value_min: 0.0,
                    value_max: 100.0,
                    ranges: vec![],
                },
                x: 50.0,
                y: 50.0,
                width: 80.0,
                height: 30.0,
                rotation: 0.0,
                visible: false,
                update_interval_ms: None,
            }],
        };
        let mut state = TemplateState::new(&template);
        let s1 = HashMap::from([("CPU".into(), 55.0f32)]);
        assert!(state.needs_render(&template, &s1), "first call always true");
        state.mark_rendered(&template, &s1);
        let s2 = HashMap::from([("CPU".into(), 99.0f32)]);
        assert!(
            !state.needs_render(&template, &s2),
            "invisible widget change should not trigger render"
        );
    }
}
