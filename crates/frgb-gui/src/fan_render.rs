#![allow(clippy::field_reassign_with_default, clippy::too_many_arguments)]
//! Fan visual renderer using tiny-skia.
//!
//! Renders physical fan representations matching real hardware appearance:
//! - SL/TL: Square housing, 7 blades, inner octagon LED segments, outer vertical bars
//! - CL: Square housing, 8 blades, inner diffused disc, outer vertical bars
//! - HydroShift: Circular housing, LED ring, LCD center
//!
//! Each render produces a pixel buffer displayable as a Slint Image,
//! plus hit-test data for mapping click coordinates to LED indices.

use frgb_model::device::DeviceType;
use frgb_model::rgb::Rgb;
use frgb_rgb::layout::LedLayout;
use tiny_skia::*;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Identifies a single LED for hit-testing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedHit {
    pub zone: LedZone,
    pub index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedZone {
    Inner,
    Outer,
}

/// Result of rendering a fan: pixel data + hit-test regions.
pub struct FanRenderResult {
    pub width: u32,
    pub height: u32,
    /// RGBA pixel data (premultiplied alpha from tiny-skia).
    pub pixels: Vec<u8>,
    /// Hit regions: (center_x, center_y, radius, led_hit).
    pub hit_regions: Vec<(f32, f32, f32, LedHit)>,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const FAN_SIZE: u32 = 200;
const CX: f32 = 100.0;
const CY: f32 = 100.0;

// Housing
const HOUSING_RADIUS: f32 = 10.0;
const HOUSING_INSET: f32 = 4.0;
const SCREW_RADIUS: f32 = 3.5;
const SCREW_INSET: f32 = 13.0;

// Color constructors (tiny-skia Color::from_rgba8 isn't const)
fn bg_housing() -> Color {
    color_rgb(0x0e, 0x0e, 0x0e)
}
fn border_housing() -> Color {
    color_rgb(0x1a, 0x1a, 0x1a)
}
fn bg_hub() -> Color {
    color_rgb(0x0d, 0x0d, 0x0d)
}
fn border_hub() -> Color {
    color_rgb(0x22, 0x22, 0x22)
}
fn bg_screw() -> Color {
    color_rgb(0x08, 0x08, 0x08)
}
fn blade_color() -> Color {
    color_rgba(0x33, 0x33, 0x33, 0x1a)
}

// Fan geometry
const BLADE_RADIUS: f32 = 70.0;
const HUB_RADIUS: f32 = 16.0;

// LED geometry
const OUTER_BAR_WIDTH: f32 = 6.0;
const OUTER_BAR_INSET_X: f32 = 6.0;
const OUTER_BAR_INSET_Y: f32 = 5.0;
const INNER_SEG_THICKNESS: f32 = 5.0;
const LED_DOT_RADIUS: f32 = 4.0;

// CL specific
const CL_DISC_RADIUS: f32 = 70.0;

// Glow
const GLOW_LAYERS: u32 = 4;

// ---------------------------------------------------------------------------
// Const helpers
// ---------------------------------------------------------------------------

fn color_rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

fn color_rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_rgba8(r, g, b, a)
}

fn rgb_to_color(rgb: &Rgb) -> Color {
    Color::from_rgba8(rgb.r, rgb.g, rgb.b, 255)
}

fn rgb_to_glow(rgb: &Rgb, alpha: u8) -> Color {
    Color::from_rgba8(rgb.r, rgb.g, rgb.b, alpha)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Render a single fan with the given LED colors.
/// `inner_colors` and `outer_colors` provide per-LED RGB values.
/// `selected` is the currently selected LED (if any) — drawn with highlight ring.
pub fn render_fan(
    device_type: DeviceType,
    inner_colors: &[Rgb],
    outer_colors: &[Rgb],
    selected: Option<LedHit>,
) -> FanRenderResult {
    let mut pixmap = Pixmap::new(FAN_SIZE, FAN_SIZE).expect("pixmap alloc");
    let mut hits = Vec::new();

    match device_type {
        DeviceType::ClWireless => {
            draw_square_housing(&mut pixmap);
            draw_cl_disc(&mut pixmap, inner_colors);
            draw_cl_blades(&mut pixmap);
            draw_outer_bars(&mut pixmap, outer_colors, &mut hits);
            draw_hub(&mut pixmap);
            draw_cl_inner_dots(&mut pixmap, inner_colors, &mut hits);
        }
        DeviceType::HydroShift | DeviceType::HydroShiftII => {
            draw_round_housing(&mut pixmap);
            draw_hydro_ring(&mut pixmap, inner_colors, &mut hits);
            draw_hydro_lcd(&mut pixmap);
        }
        // SL, TL, and everything else: SL-style layout
        _ => {
            draw_square_housing(&mut pixmap);
            draw_sl_blades(&mut pixmap);
            draw_inner_octagon(&mut pixmap, inner_colors, &mut hits);
            draw_outer_bars(&mut pixmap, outer_colors, &mut hits);
            draw_hub(&mut pixmap);
        }
    }

    // Draw selection highlight
    if let Some(sel) = selected {
        draw_selection_ring(&mut pixmap, &hits, sel);
    }

    FanRenderResult {
        width: FAN_SIZE,
        height: FAN_SIZE,
        pixels: pixmap.take(),
        hit_regions: hits,
    }
}

/// Hit-test: given pixel coordinates, find which LED was clicked.
pub fn hit_test(result: &FanRenderResult, x: f32, y: f32) -> Option<LedHit> {
    let mut best: Option<(f32, LedHit)> = None;
    for &(cx, cy, r, hit) in &result.hit_regions {
        let dx = x - cx;
        let dy = y - cy;
        let dist = (dx * dx + dy * dy).sqrt();
        // Generous click target: 2x the visual radius
        if dist <= r * 2.5 && (best.is_none() || dist < best.unwrap().0) {
            best = Some((dist, hit));
        }
    }
    best.map(|(_, h)| h)
}

/// Convert a FanRenderResult to a slint::Image.
pub fn to_slint_image(result: &FanRenderResult) -> slint::Image {
    // tiny-skia produces premultiplied RGBA. Slint's Rgba8Pixel expects
    // straight (non-premultiplied) alpha, so we un-premultiply.
    let pixel_count = (result.width * result.height) as usize;
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for chunk in result.pixels.chunks_exact(4) {
        let (pr, pg, pb, a) = (chunk[0], chunk[1], chunk[2], chunk[3]);
        if a == 0 {
            rgba.extend_from_slice(&[0, 0, 0, 0]);
        } else if a == 255 {
            rgba.extend_from_slice(&[pr, pg, pb, 255]);
        } else {
            let af = a as f32 / 255.0;
            let r = (pr as f32 / af).min(255.0) as u8;
            let g = (pg as f32 / af).min(255.0) as u8;
            let b = (pb as f32 / af).min(255.0) as u8;
            rgba.extend_from_slice(&[r, g, b, a]);
        }
    }
    let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&rgba, result.width, result.height);
    slint::Image::from_rgba8(buffer)
}

// ---------------------------------------------------------------------------
// Housing & structural elements
// ---------------------------------------------------------------------------

fn draw_square_housing(pixmap: &mut Pixmap) {
    let size = FAN_SIZE as f32;
    let i = HOUSING_INSET;

    // Housing background
    let rect = rounded_rect(i, i, size - 2.0 * i, size - 2.0 * i, HOUSING_RADIUS);
    fill_path(pixmap, &rect, bg_housing());
    stroke_path(pixmap, &rect, border_housing(), 1.5);

    // Screw holes
    for &(sx, sy) in &[
        (SCREW_INSET, SCREW_INSET),
        (size - SCREW_INSET, SCREW_INSET),
        (SCREW_INSET, size - SCREW_INSET),
        (size - SCREW_INSET, size - SCREW_INSET),
    ] {
        if let Some(circle) = PathBuilder::from_circle(sx, sy, SCREW_RADIUS) {
            fill_path(pixmap, &circle, bg_screw());
            stroke_path(pixmap, &circle, border_hub(), 0.5);
        }
    }

    // Fan area border circle
    if let Some(circle) = PathBuilder::from_circle(CX, CY, BLADE_RADIUS + 8.0) {
        stroke_path(pixmap, &circle, border_housing(), 1.0);
    }
}

fn draw_round_housing(pixmap: &mut Pixmap) {
    // HydroShift: circular housing
    if let Some(outer) = PathBuilder::from_circle(CX, CY, 96.0) {
        fill_path(pixmap, &outer, bg_housing());
        stroke_path(pixmap, &outer, border_housing(), 1.5);
    }
    if let Some(inner) = PathBuilder::from_circle(CX, CY, 90.0) {
        fill_path(pixmap, &inner, color_rgb(0x11, 0x11, 0x11));
        stroke_path(pixmap, &inner, border_housing(), 1.0);
    }
}

fn draw_hub(pixmap: &mut Pixmap) {
    if let Some(hub) = PathBuilder::from_circle(CX, CY, HUB_RADIUS) {
        fill_path(pixmap, &hub, bg_hub());
        stroke_path(pixmap, &hub, border_housing(), 1.0);
    }
    if let Some(inner_hub) = PathBuilder::from_circle(CX, CY, HUB_RADIUS * 0.55) {
        fill_path(pixmap, &inner_hub, color_rgb(0x11, 0x11, 0x11));
        stroke_path(pixmap, &inner_hub, border_hub(), 0.5);
    }
    if let Some(dot) = PathBuilder::from_circle(CX, CY, 3.0) {
        fill_path(pixmap, &dot, color_rgb(0x1a, 0x1a, 0x1a));
    }
}

// ---------------------------------------------------------------------------
// Blades
// ---------------------------------------------------------------------------

fn draw_sl_blades(pixmap: &mut Pixmap) {
    // 7 blades like SL Wireless
    draw_blade_set(pixmap, 7);
}

fn draw_cl_blades(pixmap: &mut Pixmap) {
    // CL blade silhouettes over the disc
    draw_blade_silhouettes(pixmap, 8);
}

fn draw_blade_set(pixmap: &mut Pixmap, count: u32) {
    let angle_step = std::f32::consts::TAU / count as f32;
    let mut paint = Paint::default();
    paint.set_color(blade_color());
    paint.anti_alias = true;

    for i in 0..count {
        let angle = angle_step * i as f32;

        // Blade: elongated ellipse from hub to near edge
        let blade_len = BLADE_RADIUS - 8.0;
        let blade_w = 10.0;
        if let Some(oval) = PathBuilder::from_oval(
            Rect::from_xywh(-blade_w, -blade_len, blade_w * 2.0, blade_len * 1.6)
                .unwrap_or(Rect::from_xywh(0.0, 0.0, 1.0, 1.0).unwrap()),
        ) {
            let ts =
                Transform::from_translate(CX, CY).post_concat(Transform::from_rotate_at(angle.to_degrees(), 0.0, 0.0));
            pixmap.fill_path(&oval, &paint, FillRule::Winding, ts, None);
        }
    }
}

fn draw_blade_silhouettes(pixmap: &mut Pixmap, count: u32) {
    // CL: dark blade shapes that block the disc glow
    let angle_step = std::f32::consts::TAU / count as f32;
    let mut paint = Paint::default();
    paint.set_color(color_rgba(0x11, 0x11, 0x11, 0xDD));
    paint.anti_alias = true;

    for i in 0..count {
        let angle = angle_step * i as f32;
        let half_w = 0.12; // angular half-width

        let a1 = angle - half_w;
        let a2 = angle + half_w;

        let r_outer = BLADE_RADIUS - 2.0;

        let mut pb = PathBuilder::new();
        pb.move_to(CX, CY);
        pb.line_to(CX + r_outer * a1.cos(), CY + r_outer * a1.sin());
        pb.line_to(CX + r_outer * a2.cos(), CY + r_outer * a2.sin());
        pb.close();

        if let Some(path) = pb.finish() {
            pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }
}

// ---------------------------------------------------------------------------
// SL/TL inner octagon segments
// ---------------------------------------------------------------------------

fn draw_inner_octagon(pixmap: &mut Pixmap, colors: &[Rgb], hits: &mut Vec<(f32, f32, f32, LedHit)>) {
    // SL inner: 6 line segments forming a broken octagon at blade perimeter
    // (missing top and bottom horizontal). 8 LEDs total mapped to 6 segments.
    // Segments: TL diagonal, L vertical, BL diagonal, BR diagonal, R vertical, TR diagonal
    let r = BLADE_RADIUS + 4.0;
    let inset = 24.0; // from corner
    let half = FAN_SIZE as f32 / 2.0;

    #[rustfmt::skip]
    let segments: [(f32, f32, f32, f32); 6] = [
        // TL diagonal
        (half - r + inset, half - r,           half - r,           half - r + inset),
        // Left vertical
        (half - r - 2.0,   half - r + inset + 8.0, half - r - 2.0,     half + r - inset - 8.0),
        // BL diagonal
        (half - r,         half + r - inset,   half - r + inset,   half + r),
        // BR diagonal
        (half + r - inset, half + r,           half + r,           half + r - inset),
        // Right vertical
        (half + r + 2.0,   half + r - inset - 8.0, half + r + 2.0,     half - r + inset + 8.0),
        // TR diagonal
        (half + r,         half - r + inset,   half + r - inset,   half - r),
    ];

    // Map 8 inner LEDs to segments: 1-1-2-1-1-2 (corners get singles, sides get doubles)
    let led_to_seg: [usize; 8] = [0, 1, 2, 2, 3, 4, 5, 5];
    let led_to_frac: [f32; 8] = [0.5, 0.5, 0.35, 0.65, 0.5, 0.5, 0.35, 0.65];

    for (led_idx, (&seg_idx, &frac)) in led_to_seg.iter().zip(led_to_frac.iter()).enumerate() {
        let (x1, y1, x2, y2) = segments[seg_idx];
        let px = x1 + (x2 - x1) * frac;
        let py = y1 + (y2 - y1) * frac;

        hits.push((
            px,
            py,
            LED_DOT_RADIUS,
            LedHit {
                zone: LedZone::Inner,
                index: led_idx,
            },
        ));
    }

    // Draw the segment lines with blended color
    for (seg_idx, &(x1, y1, x2, y2)) in segments.iter().enumerate() {
        // Average color of LEDs on this segment
        let leds_on_seg: Vec<usize> = led_to_seg
            .iter()
            .enumerate()
            .filter(|(_, &s)| s == seg_idx)
            .map(|(i, _)| i)
            .collect();

        let avg_color = if leds_on_seg.is_empty() {
            Rgb::BLACK
        } else {
            let (sr, sg, sb) = leds_on_seg.iter().fold((0u32, 0u32, 0u32), |(r, g, b), &i| {
                let c = colors.get(i).copied().unwrap_or(Rgb::BLACK);
                (r + c.r as u32, g + c.g as u32, b + c.b as u32)
            });
            let n = leds_on_seg.len() as u32;
            Rgb {
                r: (sr / n) as u8,
                g: (sg / n) as u8,
                b: (sb / n) as u8,
            }
        };

        // Glow layer
        draw_line_glow(pixmap, x1, y1, x2, y2, INNER_SEG_THICKNESS + 6.0, &avg_color, 40);
        // Main segment
        draw_line_segment(pixmap, x1, y1, x2, y2, INNER_SEG_THICKNESS, &avg_color, 240);
    }

    // Draw LED dots on top
    for (led_idx, &(px, py, _, _)) in hits.iter().enumerate() {
        if hits[led_idx].3.zone == LedZone::Inner {
            let color = colors.get(led_idx).copied().unwrap_or(Rgb::BLACK);
            draw_led_dot(pixmap, px, py, &color);
        }
    }
}

// ---------------------------------------------------------------------------
// CL inner disc
// ---------------------------------------------------------------------------

fn draw_cl_disc(pixmap: &mut Pixmap, colors: &[Rgb]) {
    // CL inner: diffused disc of light behind the blades.
    // Average the inner LED colors for the disc tint.
    let avg = if colors.is_empty() {
        Rgb::BLACK
    } else {
        let (sr, sg, sb) = colors.iter().fold((0u32, 0u32, 0u32), |(r, g, b), c| {
            (r + c.r as u32, g + c.g as u32, b + c.b as u32)
        });
        let n = colors.len() as u32;
        Rgb {
            r: (sr / n) as u8,
            g: (sg / n) as u8,
            b: (sb / n) as u8,
        }
    };

    if avg == Rgb::BLACK {
        return;
    }

    // Draw concentric circles with decreasing opacity for disc glow
    let steps = 6u32;
    for i in 0..steps {
        let frac = 1.0 - (i as f32 / steps as f32);
        let r = CL_DISC_RADIUS * frac;
        let alpha = (60.0 * frac) as u8;
        if let Some(circle) = PathBuilder::from_circle(CX, CY, r) {
            let mut paint = Paint::default();
            paint.set_color(rgb_to_glow(&avg, alpha));
            paint.anti_alias = true;
            paint.blend_mode = BlendMode::Screen;
            pixmap.fill_path(&circle, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    // Bright center glow
    if let Some(center) = PathBuilder::from_circle(CX, CY, 30.0) {
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(255, 255, 255, 40));
        paint.blend_mode = BlendMode::Screen;
        paint.anti_alias = true;
        pixmap.fill_path(&center, &paint, FillRule::Winding, Transform::identity(), None);
    }
}

fn draw_cl_inner_dots(pixmap: &mut Pixmap, colors: &[Rgb], hits: &mut Vec<(f32, f32, f32, LedHit)>) {
    // CL: 8 inner LEDs arranged in a ring around the hub
    let layout = LedLayout::for_device(DeviceType::ClWireless);
    let count = layout.inner_count as usize;
    let ring_r = HUB_RADIUS + 14.0;

    for i in 0..count {
        let angle = std::f32::consts::TAU * i as f32 / count as f32 - std::f32::consts::FRAC_PI_2;
        let px = CX + ring_r * angle.cos();
        let py = CY + ring_r * angle.sin();
        let color = colors.get(i).copied().unwrap_or(Rgb::BLACK);

        draw_led_dot(pixmap, px, py, &color);
        hits.push((
            px,
            py,
            LED_DOT_RADIUS,
            LedHit {
                zone: LedZone::Inner,
                index: i,
            },
        ));
    }
}

// ---------------------------------------------------------------------------
// Outer vertical bars (SL/CL/TL)
// ---------------------------------------------------------------------------

fn draw_outer_bars(pixmap: &mut Pixmap, colors: &[Rgb], hits: &mut Vec<(f32, f32, f32, LedHit)>) {
    let size = FAN_SIZE as f32;
    let bar_h = size - 2.0 * OUTER_BAR_INSET_Y;

    // Split outer LEDs: left bar = first half, right bar = second half
    let total = colors.len().max(1);
    let left_count = total / 2;
    let right_count = total - left_count;

    // Left bar
    {
        let bx = OUTER_BAR_INSET_X;
        let by = OUTER_BAR_INSET_Y;
        draw_led_bar(pixmap, bx, by, OUTER_BAR_WIDTH, bar_h, &colors[..left_count], true);

        // Left bar LED dots
        for i in 0..left_count {
            let frac = if left_count <= 1 {
                0.5
            } else {
                i as f32 / (left_count - 1) as f32
            };
            let py = by + bar_h * frac;
            let px = bx + OUTER_BAR_WIDTH / 2.0;
            let color = colors.get(i).copied().unwrap_or(Rgb::BLACK);
            draw_led_dot(pixmap, px, py, &color);
            hits.push((
                px,
                py,
                LED_DOT_RADIUS,
                LedHit {
                    zone: LedZone::Outer,
                    index: i,
                },
            ));
        }
    }

    // Right bar
    {
        let bx = size - OUTER_BAR_INSET_X - OUTER_BAR_WIDTH;
        let by = OUTER_BAR_INSET_Y;
        draw_led_bar(pixmap, bx, by, OUTER_BAR_WIDTH, bar_h, &colors[left_count..], false);

        for i in 0..right_count {
            let frac = if right_count <= 1 {
                0.5
            } else {
                i as f32 / (right_count - 1) as f32
            };
            let py = by + bar_h * frac;
            let px = bx + OUTER_BAR_WIDTH / 2.0;
            let global_i = left_count + i;
            let color = colors.get(global_i).copied().unwrap_or(Rgb::BLACK);
            draw_led_dot(pixmap, px, py, &color);
            hits.push((
                px,
                py,
                LED_DOT_RADIUS,
                LedHit {
                    zone: LedZone::Outer,
                    index: global_i,
                },
            ));
        }
    }
}

fn draw_led_bar(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, colors: &[Rgb], _left: bool) {
    if colors.is_empty() {
        return;
    }

    // Build gradient stops from LED colors
    let mut stops = Vec::new();
    for (i, c) in colors.iter().enumerate() {
        let frac = if colors.len() <= 1 {
            0.5
        } else {
            i as f32 / (colors.len() - 1) as f32
        };
        let stop = GradientStop::new(frac, rgb_to_color(c));
        stops.push(stop);
    }

    if stops.len() < 2 {
        // Solid color
        let c = rgb_to_color(&colors[0]);
        let rect = rounded_rect(x, y, w, h, w / 2.0);
        fill_path(pixmap, &rect, c);
        return;
    }

    // Glow — build glow-alpha stops from the same color positions
    let glow_rect = rounded_rect(x - 3.0, y - 2.0, w + 6.0, h + 4.0, (w + 6.0) / 2.0);
    let mut glow_stops = Vec::new();
    for (i, c) in colors.iter().enumerate() {
        let frac = if colors.len() <= 1 {
            0.5
        } else {
            i as f32 / (colors.len() - 1) as f32
        };
        glow_stops.push(GradientStop::new(frac, rgb_to_glow(c, 40)));
    }
    if glow_stops.len() >= 2 {
        if let Some(shader) = LinearGradient::new(
            Point::from_xy(0.0, y),
            Point::from_xy(0.0, y + h),
            glow_stops,
            SpreadMode::Pad,
            Transform::identity(),
        ) {
            let mut paint = Paint::default();
            paint.shader = shader;
            paint.anti_alias = true;
            paint.blend_mode = BlendMode::Screen;
            pixmap.fill_path(&glow_rect, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    // Main bar
    let rect = rounded_rect(x, y, w, h, w / 2.0);
    if let Some(shader) = LinearGradient::new(
        Point::from_xy(0.0, y),
        Point::from_xy(0.0, y + h),
        stops,
        SpreadMode::Pad,
        Transform::identity(),
    ) {
        let mut paint = Paint::default();
        paint.shader = shader;
        paint.anti_alias = true;
        pixmap.fill_path(&rect, &paint, FillRule::Winding, Transform::identity(), None);
    }
}

// ---------------------------------------------------------------------------
// HydroShift ring
// ---------------------------------------------------------------------------

fn draw_hydro_ring(pixmap: &mut Pixmap, colors: &[Rgb], hits: &mut Vec<(f32, f32, f32, LedHit)>) {
    // All LEDs are "inner" on HydroShift — arranged in a ring
    let ring_r = 80.0;
    let count = colors.len().max(1);

    // Draw ring glow
    for i in 0..count {
        let angle = std::f32::consts::TAU * i as f32 / count as f32 - std::f32::consts::FRAC_PI_2;
        let color = colors.get(i).copied().unwrap_or(Rgb::BLACK);
        if color == Rgb::BLACK {
            continue;
        }

        // Arc glow: draw a thick arc segment
        let arc_half = std::f32::consts::TAU / count as f32 / 2.0;
        let a1 = angle - arc_half;
        let a2 = angle + arc_half;
        let inner_r = ring_r - 6.0;
        let outer_r = ring_r + 6.0;

        let mut pb = PathBuilder::new();
        pb.move_to(CX + inner_r * a1.cos(), CY + inner_r * a1.sin());
        pb.line_to(CX + outer_r * a1.cos(), CY + outer_r * a1.sin());
        // Arc along outer
        let steps = 4;
        for s in 1..=steps {
            let t = a1 + (a2 - a1) * s as f32 / steps as f32;
            pb.line_to(CX + outer_r * t.cos(), CY + outer_r * t.sin());
        }
        pb.line_to(CX + inner_r * a2.cos(), CY + inner_r * a2.sin());
        // Arc along inner (reverse)
        for s in (0..steps).rev() {
            let t = a1 + (a2 - a1) * s as f32 / steps as f32;
            pb.line_to(CX + inner_r * t.cos(), CY + inner_r * t.sin());
        }
        pb.close();

        if let Some(path) = pb.finish() {
            // Glow
            let mut glow_paint = Paint::default();
            glow_paint.set_color(rgb_to_glow(&color, 50));
            glow_paint.anti_alias = true;
            glow_paint.blend_mode = BlendMode::Screen;
            pixmap.fill_path(&path, &glow_paint, FillRule::Winding, Transform::identity(), None);

            // Solid segment
            let mut paint = Paint::default();
            paint.set_color(rgb_to_color(&color));
            paint.anti_alias = true;
            let mut inner_pb = PathBuilder::new();
            let ir2 = ring_r - 4.0;
            let or2 = ring_r + 4.0;
            inner_pb.move_to(CX + ir2 * a1.cos(), CY + ir2 * a1.sin());
            inner_pb.line_to(CX + or2 * a1.cos(), CY + or2 * a1.sin());
            for s in 1..=steps {
                let t = a1 + (a2 - a1) * s as f32 / steps as f32;
                inner_pb.line_to(CX + or2 * t.cos(), CY + or2 * t.sin());
            }
            inner_pb.line_to(CX + ir2 * a2.cos(), CY + ir2 * a2.sin());
            for s in (0..steps).rev() {
                let t = a1 + (a2 - a1) * s as f32 / steps as f32;
                inner_pb.line_to(CX + ir2 * t.cos(), CY + ir2 * t.sin());
            }
            inner_pb.close();
            if let Some(inner_path) = inner_pb.finish() {
                pixmap.fill_path(&inner_path, &paint, FillRule::Winding, Transform::identity(), None);
            }
        }

        // Hit region at LED center
        let px = CX + ring_r * angle.cos();
        let py = CY + ring_r * angle.sin();
        hits.push((
            px,
            py,
            6.0,
            LedHit {
                zone: LedZone::Inner,
                index: i,
            },
        ));
    }
}

fn draw_hydro_lcd(pixmap: &mut Pixmap) {
    // LCD center circle
    if let Some(lcd) = PathBuilder::from_circle(CX, CY, 48.0) {
        fill_path(pixmap, &lcd, color_rgb(0x06, 0x06, 0x10));
        stroke_path(pixmap, &lcd, border_hub(), 1.5);
    }
}

// ---------------------------------------------------------------------------
// Drawing primitives
// ---------------------------------------------------------------------------

fn draw_led_dot(pixmap: &mut Pixmap, x: f32, y: f32, color: &Rgb) {
    if *color == Rgb::BLACK {
        // Draw dim circle for off LEDs
        if let Some(circle) = PathBuilder::from_circle(x, y, LED_DOT_RADIUS) {
            fill_path(pixmap, &circle, color_rgba(0x22, 0x22, 0x22, 0x80));
            stroke_path(pixmap, &circle, color_rgba(0x33, 0x33, 0x33, 0x60), 0.5);
        }
        return;
    }

    // Glow
    for i in 1..=GLOW_LAYERS {
        let r = LED_DOT_RADIUS + i as f32 * 2.5;
        let alpha = 30u8.saturating_sub(i as u8 * 7);
        if let Some(circle) = PathBuilder::from_circle(x, y, r) {
            let mut paint = Paint::default();
            paint.set_color(rgb_to_glow(color, alpha));
            paint.anti_alias = true;
            paint.blend_mode = BlendMode::Screen;
            pixmap.fill_path(&circle, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }

    // Main dot
    if let Some(circle) = PathBuilder::from_circle(x, y, LED_DOT_RADIUS) {
        let mut paint = Paint::default();
        paint.set_color(rgb_to_color(color));
        paint.anti_alias = true;
        pixmap.fill_path(&circle, &paint, FillRule::Winding, Transform::identity(), None);
    }

    // Highlight specular
    if let Some(spec) = PathBuilder::from_circle(x - 1.0, y - 1.0, LED_DOT_RADIUS * 0.4) {
        let mut paint = Paint::default();
        paint.set_color(Color::from_rgba8(255, 255, 255, 60));
        paint.anti_alias = true;
        pixmap.fill_path(&spec, &paint, FillRule::Winding, Transform::identity(), None);
    }
}

fn draw_selection_ring(pixmap: &mut Pixmap, hits: &[(f32, f32, f32, LedHit)], selected: LedHit) {
    if let Some(&(x, y, r, _)) = hits.iter().find(|(_, _, _, h)| *h == selected) {
        let ring_r = r + 4.0;
        if let Some(circle) = PathBuilder::from_circle(x, y, ring_r) {
            // Pulsing accent ring
            let mut paint = Paint::default();
            paint.anti_alias = true;
            let stroke = Stroke {
                width: 2.0,
                line_cap: LineCap::Round,
                ..Stroke::default()
            };
            paint.set_color(Color::from_rgba8(0x4a, 0x9e, 0xff, 0xCC));
            pixmap.stroke_path(&circle, &paint, &stroke, Transform::identity(), None);
        }
    }
}

fn draw_line_segment(pixmap: &mut Pixmap, x1: f32, y1: f32, x2: f32, y2: f32, thickness: f32, color: &Rgb, alpha: u8) {
    let mut pb = PathBuilder::new();
    pb.move_to(x1, y1);
    pb.line_to(x2, y2);
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color(rgb_to_glow(color, alpha));
        paint.anti_alias = true;
        let stroke = Stroke {
            width: thickness,
            line_cap: LineCap::Round,
            ..Stroke::default()
        };
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

fn draw_line_glow(pixmap: &mut Pixmap, x1: f32, y1: f32, x2: f32, y2: f32, thickness: f32, color: &Rgb, alpha: u8) {
    let mut pb = PathBuilder::new();
    pb.move_to(x1, y1);
    pb.line_to(x2, y2);
    if let Some(path) = pb.finish() {
        let mut paint = Paint::default();
        paint.set_color(rgb_to_glow(color, alpha));
        paint.anti_alias = true;
        paint.blend_mode = BlendMode::Screen;
        let stroke = Stroke {
            width: thickness,
            line_cap: LineCap::Round,
            ..Stroke::default()
        };
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

fn fill_path(pixmap: &mut Pixmap, path: &Path, color: Color) {
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    pixmap.fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);
}

fn stroke_path(pixmap: &mut Pixmap, path: &Path, color: Color, width: f32) {
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    let stroke = Stroke {
        width,
        ..Stroke::default()
    };
    pixmap.stroke_path(path, &paint, &stroke, Transform::identity(), None);
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Path {
    let rect = Rect::from_xywh(x, y, w, h).unwrap();
    let mut pb = PathBuilder::new();
    // Approximate rounded rect with arcs
    let r = r.min(w / 2.0).min(h / 2.0);
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    pb.finish().unwrap_or_else(|| PathBuilder::from_rect(rect))
}
