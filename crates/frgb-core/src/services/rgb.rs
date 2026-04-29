//! RGB composition services — pure functions, no IO.
//!
//! Takes device info + mode, produces an EffectResult buffer.
//! The caller (System/Backend) handles compression and transmission.

use frgb_model::device::DeviceType;
use frgb_model::effect::Effect;
use frgb_model::rgb::{FanLedAssignment, FanZoneSpec, Rgb, RgbMode, SubZone, ZoneSource};
use frgb_model::Brightness;
use frgb_rgb::buffer::RgbBuffer;
use frgb_rgb::effects::{
    BlowUpEffect, BoomerangEffect, BounceEffect, BreathingEffect, CandyBoxEffect, CollideEffect, ColorCycleEffect,
    ColorfulMeteorEffect, CoverCycleEffect, DiscoEffect, DoorEffect, DoubleArcEffect, DoubleMeteorEffect,
    DrummingEffect, DuelEffect, ElectricCurrentEffect, EndlessEffect, GradientRibbonEffect, HeartBeatEffect,
    HeartBeatRunwayEffect, HourglassEffect, IntertwineEffect, KaleidoscopeEffect, LotteryEffect, MeteorContestEffect,
    MeteorEffect, MeteorMixEffect, MeteorRainbowEffect, MeteorShowerEffect, MixingEffect, MopUpEffect, PaintEffect,
    PingPongEffect, PioneerEffect, PumpEffect, RacingEffect, RainbowEffect, RainbowMorphEffect, ReflectEffect,
    RenderEffect, ReturnArcEffect, RippleEffect, RiverEffect, RunwayEffect, ScanEffect, ShuttleRunEffect,
    SnookerEffect, StackEffect, StaggeredEffect, StaticColorEffect, TaichiEffect, TailChasingEffect, TideEffect,
    TwinkleEffect, WarningEffect, WaveEffect, WingEffect,
};
use frgb_rgb::generator::{EffectGenerator, EffectResult};
use frgb_rgb::layout::LedLayout;

use crate::error::{CoreError, Result};

/// Compose an RgbMode into an EffectResult buffer for a given device type.
///
/// This is the top-level entry point for all RGB composition. Converts
/// any RgbMode variant into FanZoneSpecs and calls compose_zones.
pub fn compose(device_type: DeviceType, leds_per_fan: usize, fan_count: usize, mode: &RgbMode) -> Result<EffectResult> {
    let specs: Vec<FanZoneSpec> = match mode {
        RgbMode::Off => vec![FanZoneSpec {
            inner: ZoneSource::Off,
            outer: ZoneSource::Off,
        }],
        RgbMode::Static {
            ring,
            color,
            brightness,
        } => {
            let src = ZoneSource::Color {
                color: *color,
                brightness: *brightness,
            };
            vec![FanZoneSpec::from_ring(*ring, src)]
        }
        RgbMode::Effect { effect, params, ring } => {
            let src = ZoneSource::Effect {
                effect: *effect,
                params: params.clone(),
            };
            vec![FanZoneSpec::from_ring(*ring, src)]
        }
        RgbMode::Composed(specs) => specs.clone(),
        RgbMode::PerFan(assignments) => {
            // Lower PerFan to Composed: each FanColorAssignment becomes a FanZoneSpec
            assignments
                .iter()
                .map(|a| FanZoneSpec {
                    inner: match a.inner {
                        Some(c) => ZoneSource::Color {
                            color: c,
                            brightness: frgb_model::Brightness::new(255),
                        },
                        None => ZoneSource::Off,
                    },
                    outer: match a.outer {
                        Some(c) => ZoneSource::Color {
                            color: c,
                            brightness: frgb_model::Brightness::new(255),
                        },
                        None => ZoneSource::Off,
                    },
                })
                .collect()
        }
        RgbMode::PerLed(assignments) => {
            if leds_per_fan == 0 || fan_count == 0 {
                return Err(CoreError::InvalidInput(format!(
                    "device type {:?} has no LED buffer",
                    device_type
                )));
            }
            return Ok(compose_per_led(device_type, leds_per_fan, fan_count, assignments));
        }
        RgbMode::TempRgb(_) => {
            return Err(CoreError::Protocol(
                "TempRgb mode requires sensor readings (Phase 3)".into(),
            ));
        }
        RgbMode::SubZones {
            inner_top,
            inner_middle,
            inner_bottom,
            outer_top,
            outer_middle,
            outer_bottom,
            brightness,
        } => {
            if leds_per_fan == 0 || fan_count == 0 {
                return Err(CoreError::InvalidInput(format!(
                    "device type {:?} has no LED buffer",
                    device_type
                )));
            }
            return compose_sub_zones(
                device_type,
                leds_per_fan,
                fan_count,
                *inner_top,
                *inner_middle,
                *inner_bottom,
                *outer_top,
                *outer_middle,
                *outer_bottom,
                *brightness,
            );
        }
    };

    if specs.is_empty() {
        return Err(CoreError::InvalidInput("composed mode has no fan specs".into()));
    }
    if leds_per_fan == 0 || fan_count == 0 {
        return Err(CoreError::InvalidInput(format!(
            "device type {:?} has no LED buffer",
            device_type
        )));
    }

    let result = compose_zones(device_type, leds_per_fan, fan_count, &specs);
    if result.frame_count == 0 {
        return Err(CoreError::InvalidInput("composition generated 0 frames".into()));
    }
    Ok(result)
}

/// Compose per-fan, per-zone specs into a single EffectResult buffer.
///
/// Each fan gets an independent inner/outer zone source. The zone sources are
/// resolved to their own buffers, then merged using `is_inner_led` to determine
/// which positions belong to which zone.
///
/// Frame counts align to the longest source (shorter sources loop).
/// Interval uses the minimum effect interval (preserves animation speed).
pub fn compose_zones(
    device_type: DeviceType,
    leds_per_fan: usize,
    fan_count: usize,
    specs: &[FanZoneSpec],
) -> EffectResult {
    debug_assert!(
        !specs.is_empty() && leds_per_fan > 0 && fan_count > 0,
        "compose_zones called with empty specs or zero dimensions"
    );

    let phys = LedLayout::for_device(device_type);
    let layout = LedLayout {
        inner_count: phys.inner_count,
        outer_count: (leds_per_fan as u8).saturating_sub(phys.inner_count),
        total_per_fan: leds_per_fan as u8,
    };

    struct ResolvedSpec {
        inner: EffectResult,
        outer: EffectResult,
    }
    let resolved: Vec<ResolvedSpec> = specs
        .iter()
        .map(|spec| ResolvedSpec {
            inner: resolve_zone_source(&spec.inner, &layout, 1),
            outer: resolve_zone_source(&spec.outer, &layout, 1),
        })
        .collect();

    let max_frames = resolved
        .iter()
        .flat_map(|r| [r.inner.frame_count, r.outer.frame_count])
        .max()
        .unwrap_or(1);

    let interval = resolved
        .iter()
        .flat_map(|r| [r.inner.interval_ms, r.outer.interval_ms])
        .fold(f32::MAX, f32::min);
    let interval = if interval == f32::MAX { 20.0 } else { interval };

    let total_leds = leds_per_fan * fan_count;
    let mut buf = RgbBuffer::new(max_frames, total_leds);

    // Precompute inner/outer classification — device-constant, no need to recompute per frame
    let inner_mask: Vec<bool> = (0..leds_per_fan).map(|l| is_inner_led(device_type, l)).collect();

    for frame in 0..max_frames {
        for fan_idx in 0..fan_count {
            let spec_idx = fan_idx.min(resolved.len() - 1);
            let r = &resolved[spec_idx];
            let inner_frame = if r.inner.frame_count > 0 {
                frame % r.inner.frame_count
            } else {
                0
            };
            let outer_frame = if r.outer.frame_count > 0 {
                frame % r.outer.frame_count
            } else {
                0
            };

            for (led, &is_inner) in inner_mask.iter().enumerate() {
                let global_led = fan_idx * leds_per_fan + led;
                let color = if is_inner {
                    r.inner.buffer.get_led(inner_frame, led)
                } else {
                    r.outer.buffer.get_led(outer_frame, led)
                };
                buf.set_led(frame, global_led, color);
            }
        }
    }

    EffectResult {
        buffer: buf,
        frame_count: max_frames,
        interval_ms: interval,
    }
}

/// Compose per-LED color assignments directly into a single-frame buffer.
///
/// Each `FanLedAssignment` provides separate inner and outer color vectors.
/// Colors are mapped to the device's inner/outer LED positions. If fewer
/// colors than LEDs, remaining LEDs are black. If fewer assignments than
/// fans, the last assignment repeats.
fn compose_per_led(
    device_type: DeviceType,
    leds_per_fan: usize,
    fan_count: usize,
    assignments: &[FanLedAssignment],
) -> EffectResult {
    let total_leds = leds_per_fan * fan_count;
    let mut buf = RgbBuffer::new(1, total_leds);

    if assignments.is_empty() {
        return EffectResult {
            buffer: buf,
            frame_count: 1,
            interval_ms: 20.0,
        };
    }

    // Collect inner/outer LED indices for this device type
    let mut inner_indices = Vec::new();
    let mut outer_indices = Vec::new();
    for led in 0..leds_per_fan {
        if is_inner_led(device_type, led) {
            inner_indices.push(led);
        } else {
            outer_indices.push(led);
        }
    }

    for fan_idx in 0..fan_count {
        let a = &assignments[fan_idx.min(assignments.len() - 1)];
        let base = fan_idx * leds_per_fan;

        for (i, &led_offset) in inner_indices.iter().enumerate() {
            if let Some(color) = a.inner.get(i) {
                buf.set_led(0, base + led_offset, *color);
            }
        }
        for (i, &led_offset) in outer_indices.iter().enumerate() {
            if let Some(color) = a.outer.get(i) {
                buf.set_led(0, base + led_offset, *color);
            }
        }
    }

    EffectResult {
        buffer: buf,
        frame_count: 1,
        interval_ms: 20.0,
    }
}

/// Map a per-fan LED index to its sub-zone for devices that support
/// six-zone composition (TL + SL today). Returns None for unsupported
/// device types or for indices outside the device's wire range.
///
/// LED indices are the user-facing addressing space (same as `cmd_led
/// --index N`), not raw virtual-buffer positions. The zone names match
/// physical geometry from a hardware probe per device.
///
/// **TL** (TlWireless, TlLcdWireless) — 21 working indices, 5 dead:
/// - 0..1: inner-top   (top of left wall, bend at 1)
/// - 2..5: inner-middle
/// - 6..7: inner-bottom
/// - 8..9: outer-top   (top of right wall, bend at 9)
/// - 10..13: outer-middle
/// - 14..20: outer-bottom
/// - 21..25: dead (firmware accepts but no LED wired)
///
/// **SL** (SlWireless, SlLcdWireless, SlV2) — 21 working indices, hex
/// inner + bar outer, mirrored across both halves of the fan by firmware:
/// - 0..1: outer-top    (top of bar)
/// - 2..6: outer-middle
/// - 7: outer-bottom
/// - 8..10: inner-top   (top of hex, bend at 10)
/// - 11..16: inner-middle
/// - 17..19: inner-bottom (bend at 17)
/// - 20: inner-top      (left-only top of inner hex; folded into inner-top
///   for symmetry with the right side via firmware mirroring)
pub fn sub_zone(device_type: DeviceType, led_index: usize) -> Option<SubZone> {
    use SubZone::*;
    match device_type {
        DeviceType::TlWireless | DeviceType::TlLcdWireless => match led_index {
            0..=1 => Some(InnerTop),
            2..=5 => Some(InnerMiddle),
            6..=7 => Some(InnerBottom),
            8..=9 => Some(OuterTop),
            10..=13 => Some(OuterMiddle),
            14..=20 => Some(OuterBottom),
            _ => None,
        },
        DeviceType::SlWireless | DeviceType::SlLcdWireless | DeviceType::SlV2 => match led_index {
            0..=1 => Some(OuterTop),
            2..=6 => Some(OuterMiddle),
            7 => Some(OuterBottom),
            8..=10 => Some(InnerTop),
            11..=16 => Some(InnerMiddle),
            17..=19 => Some(InnerBottom),
            20 => Some(InnerTop),
            _ => None,
        },
        _ => None,
    }
}

/// Scale an Rgb color by a brightness value (0-255, 255 = full).
fn scale_brightness(c: Rgb, b: Brightness) -> Rgb {
    let f = b.value() as u32;
    Rgb {
        r: ((c.r as u32 * f) / 255) as u8,
        g: ((c.g as u32 * f) / 255) as u8,
        b: ((c.b as u32 * f) / 255) as u8,
    }
}

/// Compose a six-zone color assignment into an EffectResult, reusing
/// `compose_per_led` for the actual virtual-buffer routing.
///
/// Buffer choice (inner vs outer) is driven entirely by the zone tag returned
/// by `sub_zone()` — not by the LED's positional index. This makes the
/// function correct-by-construction for devices like SL where the inner/outer
/// LED index ranges are non-contiguous from the start of the per-fan buffer.
#[allow(clippy::too_many_arguments)]
fn compose_sub_zones(
    device_type: DeviceType,
    leds_per_fan: usize,
    fan_count: usize,
    inner_top: Option<Rgb>,
    inner_middle: Option<Rgb>,
    inner_bottom: Option<Rgb>,
    outer_top: Option<Rgb>,
    outer_middle: Option<Rgb>,
    outer_bottom: Option<Rgb>,
    brightness: Brightness,
) -> Result<EffectResult> {
    let layout = LedLayout::for_device(device_type);
    if layout.total_per_fan == 0 {
        return Err(CoreError::InvalidInput(format!(
            "SubZones mode not supported for {device_type:?}"
        )));
    }
    if (0..layout.total_per_fan as usize).all(|i| sub_zone(device_type, i).is_none()) {
        return Err(CoreError::InvalidInput(format!(
            "SubZones mode not supported for {device_type:?}"
        )));
    }

    let inner_n = layout.inner_count as usize;
    let outer_n = layout.outer_count as usize;
    let mut inner = vec![Rgb::BLACK; inner_n];
    let mut outer = vec![Rgb::BLACK; outer_n];
    let mut inner_cursor = 0usize;
    let mut outer_cursor = 0usize;

    for led_index in 0..layout.total_per_fan as usize {
        let zone = match sub_zone(device_type, led_index) {
            Some(z) => z,
            None => continue,
        };
        let (color, is_inner_zone) = match zone {
            SubZone::InnerTop    => (inner_top,    true),
            SubZone::InnerMiddle => (inner_middle, true),
            SubZone::InnerBottom => (inner_bottom, true),
            SubZone::OuterTop    => (outer_top,    false),
            SubZone::OuterMiddle => (outer_middle, false),
            SubZone::OuterBottom => (outer_bottom, false),
        };
        let scaled = color.map(|c| scale_brightness(c, brightness)).unwrap_or(Rgb::BLACK);
        if is_inner_zone {
            if inner_cursor < inner_n {
                inner[inner_cursor] = scaled;
            }
            inner_cursor += 1;
        } else {
            if outer_cursor < outer_n {
                outer[outer_cursor] = scaled;
            }
            outer_cursor += 1;
        }
    }

    // Verify cursors match what sub_zone() actually classifies — catches future
    // devices where the sub_zone classifier disagrees with LedLayout. Uses
    // sub_zone() counts directly instead of layout.{inner,outer}_count because
    // some devices (e.g. TL) have dead LED indices that sub_zone() returns None
    // for, making the sub_zone count legitimately smaller than layout counts.
    debug_assert_eq!(
        inner_cursor,
        (0..layout.total_per_fan as usize)
            .filter(|&i| sub_zone(device_type, i).map(|z| z.is_inner()).unwrap_or(false))
            .count(),
        "compose_sub_zones: inner cursor ({inner_cursor}) != sub_zone inner count for {device_type:?}"
    );
    debug_assert_eq!(
        outer_cursor,
        (0..layout.total_per_fan as usize)
            .filter(|&i| sub_zone(device_type, i).map(|z| !z.is_inner()).unwrap_or(false))
            .count(),
        "compose_sub_zones: outer cursor ({outer_cursor}) != sub_zone outer count for {device_type:?}"
    );

    let assignments = vec![FanLedAssignment { inner, outer }; fan_count];
    Ok(compose_per_led(device_type, leds_per_fan, fan_count, &assignments))
}

/// Resolve a ZoneSource to an EffectResult buffer.
pub fn resolve_zone_source(source: &ZoneSource, layout: &LedLayout, fan_count: u8) -> EffectResult {
    use frgb_model::rgb::EffectParams;

    match source {
        ZoneSource::Color { color, brightness } => {
            let params = EffectParams {
                brightness: *brightness,
                ..Default::default()
            };
            StaticColorEffect.generate(layout, fan_count, &params, &[*color])
        }
        ZoneSource::Effect { effect, params } => {
            let gen: Box<dyn EffectGenerator> = match effect_generator(effect) {
                Some(g) => g,
                None => {
                    let total_leds = layout.total_leds(fan_count);
                    let buf = RgbBuffer::new(1, total_leds);
                    return EffectResult {
                        buffer: buf,
                        frame_count: 1,
                        interval_ms: 20.0,
                    };
                }
            };
            let colors: Vec<Rgb> = params.color.into_iter().collect();
            let mut result = gen.generate(layout, fan_count, params, &colors);

            let speed_idx = (params.speed.clamp(1, 5) as usize) - 1;
            let speed_level = frgb_protocol::constants::EFFECT_SPEED_LEVELS[speed_idx];
            result.interval_ms *= speed_level as f32;
            result
        }
        ZoneSource::Off => {
            let total_leds = layout.total_leds(fan_count);
            let buf = RgbBuffer::new(1, total_leds);
            EffectResult {
                buffer: buf,
                frame_count: 1,
                interval_ms: 20.0,
            }
        }
    }
}

/// Map an Effect to its generator. Returns None for unimplemented effects (Voice, etc.).
pub fn effect_generator(effect: &Effect) -> Option<Box<dyn EffectGenerator>> {
    match effect {
        Effect::Rainbow => Some(Box::new(RainbowEffect)),
        Effect::RainbowMorph => Some(Box::new(RainbowMorphEffect)),
        Effect::StaticColor => Some(Box::new(StaticColorEffect)),
        Effect::Breathing => Some(Box::new(BreathingEffect)),
        Effect::Runway => Some(Box::new(RunwayEffect)),
        Effect::Meteor => Some(Box::new(MeteorEffect)),
        Effect::Twinkle => Some(Box::new(TwinkleEffect)),
        Effect::ColorCycle => Some(Box::new(ColorCycleEffect)),
        Effect::Mixing => Some(Box::new(MixingEffect)),
        Effect::Tide => Some(Box::new(TideEffect)),
        Effect::ElectricCurrent => Some(Box::new(ElectricCurrentEffect)),
        Effect::Reflect => Some(Box::new(ReflectEffect)),
        Effect::GradientRibbon => Some(Box::new(GradientRibbonEffect)),
        Effect::Disco => Some(Box::new(DiscoEffect)),
        Effect::Warning => Some(Box::new(WarningEffect)),
        Effect::MopUp => Some(Box::new(MopUpEffect)),
        Effect::Hourglass => Some(Box::new(HourglassEffect)),
        Effect::Taichi => Some(Box::new(TaichiEffect)),
        Effect::MeteorRainbow => Some(Box::new(MeteorRainbowEffect)),
        Effect::ColorfulMeteor => Some(Box::new(ColorfulMeteorEffect)),
        Effect::Lottery => Some(Box::new(LotteryEffect)),
        Effect::Scan => Some(Box::new(ScanEffect)),
        Effect::DoubleMeteor => Some(Box::new(DoubleMeteorEffect)),
        Effect::MeteorContest => Some(Box::new(MeteorContestEffect)),
        Effect::MeteorMix => Some(Box::new(MeteorMixEffect)),
        Effect::ReturnArc => Some(Box::new(ReturnArcEffect)),
        Effect::DoubleArc => Some(Box::new(DoubleArcEffect)),
        Effect::Door => Some(Box::new(DoorEffect)),
        Effect::HeartBeat => Some(Box::new(HeartBeatEffect)),
        Effect::HeartBeatRunway => Some(Box::new(HeartBeatRunwayEffect)),
        Effect::Wing => Some(Box::new(WingEffect)),
        Effect::Drumming => Some(Box::new(DrummingEffect)),
        Effect::Boomerang => Some(Box::new(BoomerangEffect)),
        Effect::CandyBox => Some(Box::new(CandyBoxEffect)),
        Effect::Staggered => Some(Box::new(StaggeredEffect)),
        Effect::Render => Some(Box::new(RenderEffect)),
        Effect::PingPong => Some(Box::new(PingPongEffect)),
        Effect::Stack => Some(Box::new(StackEffect)),
        Effect::Ripple => Some(Box::new(RippleEffect)),
        Effect::Collide => Some(Box::new(CollideEffect)),
        Effect::Endless => Some(Box::new(EndlessEffect)),
        Effect::River => Some(Box::new(RiverEffect)),
        Effect::Duel => Some(Box::new(DuelEffect)),
        Effect::Pioneer => Some(Box::new(PioneerEffect)),
        Effect::ShuttleRun => Some(Box::new(ShuttleRunEffect)),
        Effect::Pump => Some(Box::new(PumpEffect)),
        Effect::Bounce => Some(Box::new(BounceEffect)),
        Effect::CoverCycle => Some(Box::new(CoverCycleEffect)),
        Effect::Wave => Some(Box::new(WaveEffect)),
        Effect::MeteorShower => Some(Box::new(MeteorShowerEffect)),
        Effect::Paint => Some(Box::new(PaintEffect)),
        Effect::Snooker => Some(Box::new(SnookerEffect)),
        Effect::BlowUp => Some(Box::new(BlowUpEffect)),
        Effect::TailChasing => Some(Box::new(TailChasingEffect)),
        Effect::Racing => Some(Box::new(RacingEffect)),
        Effect::Intertwine => Some(Box::new(IntertwineEffect)),
        Effect::Kaleidoscope => Some(Box::new(KaleidoscopeEffect)),
        Effect::Voice => None, // requires audio input
    }
}

/// Determine if a LED position within a fan's virtual buffer is an "inner" LED.
///
/// User-facing LED-index space (matches `frgb led --index N` and the
/// docstring of `sub_zone()`):
///
/// - **SL** (SlWireless, SlLcdWireless, SlV2 — 21 positions/fan): outer bar
///   at 0-7, inner hex at 8-20. Verified by hardware probe (see
///   project_sl_sub_zones_pending.md). Corrects a prior implementation that
///   used a 40-position interleaved layout `[12..=19, 32..=39]`.
/// - **SlInfWireless**: retains the pre-Stage-4 formula `[12..=19, 32..=39]`
///   pending hardware verification of the Infinity fan layout. devices.toml
///   says virtual_leds=44; correct inner partition is unconfirmed.
/// - **CL** (24 positions/fan): inner at 0-7, outer at 8-23.
/// - **TL** (26 positions/fan, 21 working): inner at 0-7, outer at 8-20.
///   "Inner" physically covers the left side of both fan faces; "outer"
///   the right side of both — confirmed by hardware probe.
pub fn is_inner_led(device_type: DeviceType, led_within_fan: usize) -> bool {
    match device_type {
        DeviceType::SlWireless | DeviceType::SlLcdWireless | DeviceType::SlV2 => {
            // SL hardware: 21 positions/fan. Inner hex at indices 8-20, outer bar at 0-7.
            // Verified by hardware probe (see project_sl_sub_zones_pending.md).
            matches!(led_within_fan, 8..=20)
        }
        DeviceType::SlInfWireless => {
            // SL Infinity (58-LED layout) — pre-existing formula retained pending
            // hardware verification of correct partition. devices.toml says
            // virtual_leds=44, inner_leds=8 — neither matches the formula or any
            // current layout. Tracked as a follow-up.
            matches!(led_within_fan, 12..=19 | 32..=39)
        }
        DeviceType::ClWireless => led_within_fan < 8,
        DeviceType::TlWireless | DeviceType::TlLcdWireless => led_within_fan < 8,
        _ => true,
    }
}

/// Compute the `(is_inner, offset)` for a user-facing LED index.
///
/// `is_inner` mirrors `is_inner_led(device_type, led_index)`.
/// `offset` is the count of preceding indices classified the same way,
/// i.e. the slot in the inner-or-outer buffer where the color should land
/// so that `compose_per_led` routes it to physical position `led_index`.
pub fn per_led_zone_offset(device_type: DeviceType, led_index: usize) -> (bool, usize) {
    let is_inner = is_inner_led(device_type, led_index);
    let mut off = 0usize;
    for i in 0..led_index {
        if is_inner_led(device_type, i) == is_inner {
            off += 1;
        }
    }
    (is_inner, off)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::rgb::{EffectParams, FanColorAssignment, FanLedAssignment, Ring};
    use frgb_model::Brightness;

    #[test]
    fn compose_static_red() {
        let result = compose(
            DeviceType::ClWireless,
            24,
            2,
            &RgbMode::Static {
                ring: Ring::Both,
                color: Rgb { r: 254, g: 0, b: 0 },
                brightness: Brightness::new(255),
            },
        )
        .unwrap();
        assert!(result.frame_count > 0);
        let c = result.buffer.get_led(0, 0);
        assert_eq!(c.r, 254);
    }

    #[test]
    fn compose_off() {
        let result = compose(DeviceType::ClWireless, 24, 1, &RgbMode::Off).unwrap();
        let c = result.buffer.get_led(0, 0);
        assert_eq!(c, Rgb::BLACK);
    }

    #[test]
    fn compose_effect() {
        let mode = RgbMode::Effect {
            effect: Effect::Rainbow,
            params: EffectParams::default(),
            ring: Ring::Both,
        };
        let result = compose(DeviceType::ClWireless, 24, 1, &mode).unwrap();
        assert!(result.frame_count > 1);
    }

    #[test]
    fn compose_zero_leds_errors() {
        assert!(compose(DeviceType::Unknown, 0, 0, &RgbMode::Off).is_err());
    }

    #[test]
    fn is_inner_led_cl() {
        for i in 0..8 {
            assert!(is_inner_led(DeviceType::ClWireless, i));
        }
        for i in 8..24 {
            assert!(!is_inner_led(DeviceType::ClWireless, i));
        }
    }

    #[test]
    fn is_inner_led_sl_matches_physical_hardware() {
        // Hardware: indices 0-7 = outer bar; 8-20 = inner hex.
        for i in 0..=7 {
            assert!(!is_inner_led(DeviceType::SlWireless, i), "SL led {i} should be outer");
        }
        for i in 8..=20 {
            assert!(is_inner_led(DeviceType::SlWireless, i), "SL led {i} should be inner");
        }
    }

    /// TL fans: 26 addressable LEDs, inner = [0..8), outer = [8..26).
    /// Physically, "inner" colors the left side of *both* fan faces and "outer"
    /// the right side of both faces (hardware-confirmed). The wire-level split
    /// uses 13/13 but the shipped firmware/codec remaps that to this asymmetric 8/18.
    #[test]
    fn is_inner_led_tl() {
        for dt in [DeviceType::TlWireless, DeviceType::TlLcdWireless] {
            for i in 0..8 {
                assert!(is_inner_led(dt, i), "TL led {i} should be inner for {dt:?}");
            }
            for i in 8..26 {
                assert!(!is_inner_led(dt, i), "TL led {i} should be outer for {dt:?}");
            }
        }
    }

    /// TL: every LED index 0..=20 maps to exactly one zone; 21..=25 dead.
    #[test]
    fn sub_zone_tl_classifier() {
        use SubZone::*;
        let cases = [
            (0..=1, InnerTop),
            (2..=5, InnerMiddle),
            (6..=7, InnerBottom),
            (8..=9, OuterTop),
            (10..=13, OuterMiddle),
            (14..=20, OuterBottom),
        ];
        for (range, expected) in cases {
            for i in range {
                assert_eq!(
                    sub_zone(DeviceType::TlWireless, i),
                    Some(expected),
                    "TL wire index {i} should be {expected:?}",
                );
            }
        }
        for i in 21..=25 {
            assert_eq!(sub_zone(DeviceType::TlWireless, i), None);
        }
    }

    /// SL: 21 working indices; 0..=7 are bar (Outer*), 8..=20 are hex (Inner*).
    /// Index 20 folds into InnerTop for symmetry (left-only top of inner hex).
    #[test]
    fn sub_zone_sl_classifier() {
        use SubZone::*;
        let cases = [
            (0..=1, OuterTop),
            (2..=6, OuterMiddle),
            (7..=7, OuterBottom),
            (8..=10, InnerTop),
            (11..=16, InnerMiddle),
            (17..=19, InnerBottom),
        ];
        for (range, expected) in cases {
            for i in range {
                assert_eq!(
                    sub_zone(DeviceType::SlWireless, i),
                    Some(expected),
                    "SL led index {i} should be {expected:?}",
                );
            }
        }
        // Asymmetric folded position
        assert_eq!(sub_zone(DeviceType::SlWireless, 20), Some(InnerTop));
        // Same map applies to SL-LCD and SL-V2
        assert_eq!(sub_zone(DeviceType::SlLcdWireless, 8), Some(InnerTop));
        assert_eq!(sub_zone(DeviceType::SlV2, 0), Some(OuterTop));
    }

    /// Unsupported device types return None for every index.
    #[test]
    fn sub_zone_rejects_unsupported() {
        for i in 0..=39 {
            assert_eq!(sub_zone(DeviceType::ClWireless, i), None);
            assert_eq!(sub_zone(DeviceType::HydroShift, i), None);
        }
    }

    /// SubZones composition on TL: each zone color lands at the expected
    /// virtual-buffer positions (TL has identity LED→buffer mapping since
    /// `is_inner_led_tl` matches the LedLayout split).
    #[test]
    fn sub_zones_composes_tl() {
        let red = Rgb { r: 254, g: 0, b: 0 };
        let blue = Rgb { r: 0, g: 0, b: 254 };
        let mode = RgbMode::SubZones {
            inner_top: Some(red),
            inner_middle: None,
            inner_bottom: None,
            outer_top: None,
            outer_middle: Some(blue),
            outer_bottom: None,
            brightness: frgb_model::Brightness::new(255),
        };
        let result = compose(DeviceType::TlWireless, 26, 1, &mode).unwrap();
        // inner-top: indices 0..=1 → red
        assert_eq!(result.buffer.get_led(0, 0), red);
        assert_eq!(result.buffer.get_led(0, 1), red);
        // inner-middle (no color set): indices 2..=5 → black
        for i in 2..=5 {
            assert_eq!(result.buffer.get_led(0, i), Rgb::BLACK, "led {i} should be black");
        }
        // outer-middle: indices 10..=13 → blue
        for i in 10..=13 {
            assert_eq!(result.buffer.get_led(0, i), blue, "led {i} should be blue");
        }
        // dead indices 21..=25 stay black
        for i in 21..=25 {
            assert_eq!(result.buffer.get_led(0, i), Rgb::BLACK);
        }
    }

    #[test]
    fn sub_zones_composes_sl_outer_top_lights_bar_top() {
        let orange = Rgb { r: 254, g: 60, b: 0 };
        let mode = RgbMode::SubZones {
            inner_top: None,
            inner_middle: None,
            inner_bottom: None,
            outer_top: Some(orange),
            outer_middle: None,
            outer_bottom: None,
            brightness: frgb_model::Brightness::new(255),
        };
        // SL leds_per_fan = total_per_fan = 21.
        let result = compose(DeviceType::SlWireless, 21, 1, &mode).unwrap();
        // outer-top is at user-facing LED indices 0-1 (per sub_zone() docstring).
        let bar_top_a = result.buffer.get_led(0, 0);
        let bar_top_b = result.buffer.get_led(0, 1);
        assert_eq!(bar_top_a, orange, "led 0 (outer-top, bar) should be orange");
        assert_eq!(bar_top_b, orange, "led 1 (outer-top, bar) should be orange");
        // Inner hex (indices 8-20) should be black.
        for i in 8..=20 {
            assert_eq!(result.buffer.get_led(0, i), Rgb::BLACK, "led {i} (inner hex) should be black");
        }
    }

    #[test]
    fn sub_zones_composes_sl_inner_top_lights_hex_top() {
        let blue = Rgb { r: 0, g: 0, b: 254 };
        let mode = RgbMode::SubZones {
            inner_top: Some(blue),
            inner_middle: None,
            inner_bottom: None,
            outer_top: None,
            outer_middle: None,
            outer_bottom: None,
            brightness: frgb_model::Brightness::new(255),
        };
        let result = compose(DeviceType::SlWireless, 21, 1, &mode).unwrap();
        // inner-top is at user-facing LED indices 8-10 + 20.
        for i in 8..=10 {
            assert_eq!(result.buffer.get_led(0, i), blue, "led {i} (inner-top) should be blue");
        }
        assert_eq!(result.buffer.get_led(0, 20), blue, "led 20 (inner-top) should be blue");
        // Bar (indices 0-7) should be black.
        for i in 0..=7 {
            assert_eq!(result.buffer.get_led(0, i), Rgb::BLACK, "led {i} (outer bar) should be black");
        }
    }

    #[test]
    fn sub_zones_composes_sl_outer_middle_lights_bar_middle() {
        let red = Rgb { r: 254, g: 0, b: 0 };
        let mode = RgbMode::SubZones {
            outer_middle: Some(red),
            inner_top: None, inner_middle: None, inner_bottom: None,
            outer_top: None, outer_bottom: None,
            brightness: frgb_model::Brightness::new(255),
        };
        let result = compose(DeviceType::SlWireless, 21, 1, &mode).unwrap();
        // outer-middle at indices 2-6.
        for i in 2..=6 {
            assert_eq!(result.buffer.get_led(0, i), red, "led {i} should be red");
        }
    }

    #[test]
    fn sub_zones_composes_sl_inner_bottom_lights_hex_bottom() {
        let green = Rgb { r: 0, g: 254, b: 0 };
        let mode = RgbMode::SubZones {
            inner_bottom: Some(green),
            inner_top: None, inner_middle: None,
            outer_top: None, outer_middle: None, outer_bottom: None,
            brightness: frgb_model::Brightness::new(255),
        };
        let result = compose(DeviceType::SlWireless, 21, 1, &mode).unwrap();
        // inner-bottom at indices 17-19.
        for i in 17..=19 {
            assert_eq!(result.buffer.get_led(0, i), green, "led {i} should be green");
        }
    }

    /// Production runtime calls `compose(SlWireless, virtual_leds=40, ...)` per
    /// devices.toml. After the Stage 4 fix, positions 0-7 (outer bar) and 8-20
    /// (inner hex) are written from the corresponding zone; positions 21-39 are
    /// BLACK (not addressed at the user-facing zone level). This matches the
    /// hypothesis that virtual_leds=40 for SL is vestigial — the firmware uses
    /// only positions 0-20. Hardware verification (Task 18) confirms this.
    ///
    /// If user-visible behavior shows dark LEDs in the 21-39 range, this test
    /// will pass but Task 18 will fail; that means virtual_leds=40 is NOT
    /// vestigial and devices.toml needs updating to 21, OR positions 21-39
    /// need mirrored fill.
    #[test]
    fn sub_zones_composes_sl_at_production_leds_per_fan_40() {
        let red = Rgb { r: 200, g: 0, b: 0 };
        let blue = Rgb { r: 0, g: 0, b: 200 };
        let mode = RgbMode::SubZones {
            inner_top: Some(blue),
            inner_middle: None,
            inner_bottom: None,
            outer_top: Some(red),
            outer_middle: None,
            outer_bottom: None,
            brightness: Brightness::new(255),
        };
        // Production runtime uses virtual_leds=40 (from devices.toml) — match it.
        let result = compose(DeviceType::SlWireless, 40, 1, &mode).unwrap();

        // outer-top (user indices 0-1) lights with red.
        assert_eq!(result.buffer.get_led(0, 0), red, "led 0 (outer-top) should be red");
        assert_eq!(result.buffer.get_led(0, 1), red, "led 1 (outer-top) should be red");

        // inner-top (user indices 8-10 + 20) lights with blue.
        assert_eq!(result.buffer.get_led(0, 8), blue, "led 8 (inner-top) should be blue");
        assert_eq!(result.buffer.get_led(0, 9), blue, "led 9 (inner-top) should be blue");
        assert_eq!(result.buffer.get_led(0, 10), blue, "led 10 (inner-top) should be blue");
        assert_eq!(result.buffer.get_led(0, 20), blue, "led 20 (inner-top) should be blue");

        // Positions 21-39 (above user-facing index 20) are BLACK after the fix.
        // This is the documented assumption — firmware presumed to ignore them.
        for i in 21..=39 {
            assert_eq!(result.buffer.get_led(0, i), Rgb::BLACK, "led {i} should be black (vestigial range)");
        }
    }

    /// SubZones rejects devices without a sub_zone classifier with a clear error.
    #[test]
    fn sub_zones_rejects_unsupported_device() {
        let mode = RgbMode::SubZones {
            inner_top: Some(Rgb { r: 254, g: 0, b: 0 }),
            inner_middle: None,
            inner_bottom: None,
            outer_top: None,
            outer_middle: None,
            outer_bottom: None,
            brightness: frgb_model::Brightness::new(255),
        };
        assert!(compose(DeviceType::ClWireless, 24, 1, &mode).is_err());
        // HydroShift: layout exists but no zone classifier yet → reject.
        assert!(compose(DeviceType::HydroShiftII, 24, 1, &mode).is_err());
    }

    /// Brightness scaling: 50% brightness halves each channel.
    #[test]
    fn sub_zones_brightness_scales() {
        let red = Rgb { r: 200, g: 0, b: 0 };
        let mode = RgbMode::SubZones {
            inner_top: Some(red),
            inner_middle: None,
            inner_bottom: None,
            outer_top: None,
            outer_middle: None,
            outer_bottom: None,
            brightness: frgb_model::Brightness::new(128),
        };
        let result = compose(DeviceType::TlWireless, 26, 1, &mode).unwrap();
        let scaled = result.buffer.get_led(0, 0);
        // 200 * 128 / 255 = 100
        assert_eq!(scaled.r, 100);
        assert_eq!(scaled.g, 0);
        assert_eq!(scaled.b, 0);
    }

    /// Inner + outer LED counts must sum to `addressable_leds()` for every
    /// dual-ring fan type — catches off-by-N bugs in the split constant.
    #[test]
    fn inner_plus_outer_covers_addressable_leds() {
        for dt in [
            DeviceType::SlWireless,
            DeviceType::ClWireless,
            DeviceType::TlWireless,
            DeviceType::TlLcdWireless,
        ] {
            let total = dt.addressable_leds() as usize;
            let inner = (0..total).filter(|&i| is_inner_led(dt, i)).count();
            let outer = total - inner;
            assert!(inner > 0, "{dt:?}: inner count must be non-zero");
            assert!(outer > 0, "{dt:?}: outer count must be non-zero");
            assert_eq!(
                inner + outer, total,
                "{dt:?}: inner ({inner}) + outer ({outer}) must equal addressable_leds ({total})",
            );
        }
    }

    #[test]
    fn compose_per_fan() {
        let mode = RgbMode::PerFan(vec![
            FanColorAssignment {
                inner: Some(Rgb { r: 254, g: 0, b: 0 }),
                outer: Some(Rgb { r: 0, g: 0, b: 254 }),
            },
            FanColorAssignment {
                inner: None,
                outer: Some(Rgb { r: 0, g: 254, b: 0 }),
            },
        ]);
        let result = compose(DeviceType::ClWireless, 24, 2, &mode).unwrap();
        // Fan 0 inner (LED 0) should be red
        assert_eq!(result.buffer.get_led(0, 0).r, 254);
        // Fan 0 outer (LED 8) should be blue
        assert_eq!(result.buffer.get_led(0, 8).b, 254);
        // Fan 1 inner (LED 24) should be off (None)
        assert_eq!(result.buffer.get_led(0, 24), Rgb::BLACK);
        // Fan 1 outer (LED 32) should be green
        assert_eq!(result.buffer.get_led(0, 32).g, 254);
    }

    #[test]
    fn compose_per_led_cl() {
        // CL: 8 inner (0-7), 16 outer (8-23)
        let red = Rgb { r: 254, g: 0, b: 0 };
        let blue = Rgb { r: 0, g: 0, b: 254 };
        let mode = RgbMode::PerLed(vec![FanLedAssignment {
            inner: vec![red; 8],
            outer: vec![blue; 16],
        }]);
        let result = compose(DeviceType::ClWireless, 24, 2, &mode).unwrap();
        assert_eq!(result.frame_count, 1);
        // Fan 0 inner LED 0 = red
        assert_eq!(result.buffer.get_led(0, 0).r, 254);
        // Fan 0 outer LED 8 = blue
        assert_eq!(result.buffer.get_led(0, 8).b, 254);
        // Fan 1 repeats last assignment
        assert_eq!(result.buffer.get_led(0, 24).r, 254);
        assert_eq!(result.buffer.get_led(0, 32).b, 254);
    }

    #[test]
    fn compose_per_led_partial_colors() {
        // Only provide 2 inner colors out of 8 — rest should be black
        let green = Rgb { r: 0, g: 254, b: 0 };
        let mode = RgbMode::PerLed(vec![FanLedAssignment {
            inner: vec![green, green],
            outer: vec![],
        }]);
        let result = compose(DeviceType::ClWireless, 24, 1, &mode).unwrap();
        assert_eq!(result.buffer.get_led(0, 0).g, 254);
        assert_eq!(result.buffer.get_led(0, 1).g, 254);
        // LED 2 — no color provided, should be black
        assert_eq!(result.buffer.get_led(0, 2), Rgb::BLACK);
        // Outer LEDs — empty vec, all black
        assert_eq!(result.buffer.get_led(0, 8), Rgb::BLACK);
    }

    #[test]
    fn compose_per_led_empty_assignments() {
        let mode = RgbMode::PerLed(vec![]);
        let result = compose(DeviceType::ClWireless, 24, 1, &mode).unwrap();
        assert_eq!(result.frame_count, 1);
        // All black
        assert_eq!(result.buffer.get_led(0, 0), Rgb::BLACK);
    }

    #[test]
    fn compose_per_led_sl_physical_layout() {
        // SL physical layout: 21 positions/fan.
        // is_inner_led for SL: outer bar at 0-7, inner hex at 8-20.
        let red = Rgb { r: 254, g: 0, b: 0 };
        let blue = Rgb { r: 0, g: 0, b: 254 };
        let mode = RgbMode::PerLed(vec![FanLedAssignment {
            inner: vec![red; 13],
            outer: vec![blue; 8],
        }]);
        let result = compose(DeviceType::SlWireless, 21, 1, &mode).unwrap();
        assert_eq!(result.frame_count, 1);
        // Positions 0-7 are outer → blue
        for i in 0..=7 {
            assert_eq!(result.buffer.get_led(0, i).b, 254, "position {i} should be outer (blue)");
        }
        // Positions 8-20 are inner → red
        for i in 8..=20 {
            assert_eq!(result.buffer.get_led(0, i).r, 254, "position {i} should be inner (red)");
        }
    }

    #[test]
    fn compose_per_led_multi_fan() {
        let red = Rgb { r: 254, g: 0, b: 0 };
        let green = Rgb { r: 0, g: 254, b: 0 };
        let mode = RgbMode::PerLed(vec![
            FanLedAssignment {
                inner: vec![red; 8],
                outer: vec![],
            },
            FanLedAssignment {
                inner: vec![green; 8],
                outer: vec![],
            },
        ]);
        let result = compose(DeviceType::ClWireless, 24, 2, &mode).unwrap();
        // Fan 0 inner = red
        assert_eq!(result.buffer.get_led(0, 0).r, 254);
        assert_eq!(result.buffer.get_led(0, 0).g, 0);
        // Fan 1 inner = green
        assert_eq!(result.buffer.get_led(0, 24).g, 254);
        assert_eq!(result.buffer.get_led(0, 24).r, 0);
    }

    // -----------------------------------------------------------------------
    // cmd_led routing — verify user-facing led_index N lights physical LED N
    // -----------------------------------------------------------------------

    #[test]
    fn per_led_index_routing_sl_lights_physical_index_directly() {
        // Simulate `frgb led red --index 0` on an SL fan.
        // User expects: physical LED 0 (top of outer bar) lights red.
        let red = Rgb { r: 254, g: 0, b: 0 };
        let layout = LedLayout::for_device(DeviceType::SlWireless);
        let inner_n = layout.inner_count as usize;
        let outer_n = layout.outer_count as usize;

        let led_index = 0usize;
        let (is_inner, off) = per_led_zone_offset(DeviceType::SlWireless, led_index);
        let mut inner = vec![Rgb::BLACK; inner_n];
        let mut outer = vec![Rgb::BLACK; outer_n];
        if is_inner { inner[off] = red; } else { outer[off] = red; }

        let assignments = vec![FanLedAssignment { inner, outer }];
        let result = compose_per_led(DeviceType::SlWireless, 21, 1, &assignments);

        // Physical LED 0 (outer bar) should be red.
        assert_eq!(result.buffer.get_led(0, 0), red, "SL led_index=0 should light physical LED 0");
        for i in 1..21 {
            assert_eq!(result.buffer.get_led(0, i), Rgb::BLACK, "led {i} should be black");
        }
    }

    #[test]
    fn per_led_index_routing_sl_index_8_lights_inner_top() {
        // Simulate `frgb led blue --index 8` on an SL fan.
        // Physical LED 8 is the first position of the inner hex.
        let blue = Rgb { r: 0, g: 0, b: 254 };
        let layout = LedLayout::for_device(DeviceType::SlWireless);
        let inner_n = layout.inner_count as usize;
        let outer_n = layout.outer_count as usize;

        let led_index = 8usize;
        let (is_inner, off) = per_led_zone_offset(DeviceType::SlWireless, led_index);
        let mut inner = vec![Rgb::BLACK; inner_n];
        let mut outer = vec![Rgb::BLACK; outer_n];
        if is_inner { inner[off] = blue; } else { outer[off] = blue; }

        let assignments = vec![FanLedAssignment { inner, outer }];
        let result = compose_per_led(DeviceType::SlWireless, 21, 1, &assignments);

        assert_eq!(result.buffer.get_led(0, 8), blue, "SL led_index=8 should light physical LED 8 (inner top)");
        for i in 0..21 {
            if i != 8 {
                assert_eq!(result.buffer.get_led(0, i), Rgb::BLACK, "led {i} should be black");
            }
        }
    }

    #[test]
    fn per_led_zone_offset_sl_correct_for_all_indices() {
        // SL: indices 0-7 are outer (offsets 0-7), indices 8-20 are inner (offsets 0-12).
        for i in 0..=7 {
            let (is_inner, off) = per_led_zone_offset(DeviceType::SlWireless, i);
            assert!(!is_inner, "SL led {i} should be outer");
            assert_eq!(off, i, "SL led {i} should have outer offset {i}");
        }
        for i in 8..=20 {
            let (is_inner, off) = per_led_zone_offset(DeviceType::SlWireless, i);
            assert!(is_inner, "SL led {i} should be inner");
            assert_eq!(off, i - 8, "SL led {i} should have inner offset {}", i - 8);
        }
    }

    #[test]
    fn per_led_zone_offset_tl_correct_for_all_indices() {
        // TL: indices 0-7 are inner (offsets 0-7), indices 8-25 are outer (offsets 0-17).
        for i in 0..=7 {
            let (is_inner, off) = per_led_zone_offset(DeviceType::TlWireless, i);
            assert!(is_inner, "TL led {i} should be inner");
            assert_eq!(off, i);
        }
        for i in 8..=25 {
            let (is_inner, off) = per_led_zone_offset(DeviceType::TlWireless, i);
            assert!(!is_inner, "TL led {i} should be outer");
            assert_eq!(off, i - 8);
        }
    }

    #[test]
    fn effect_generator_all_implemented() {
        let implemented = [
            Effect::Rainbow,
            Effect::RainbowMorph,
            Effect::StaticColor,
            Effect::Breathing,
            Effect::Runway,
            Effect::Meteor,
            Effect::Twinkle,
            Effect::Taichi,
            Effect::ColorCycle,
            Effect::Warning,
            Effect::Mixing,
            Effect::Tide,
            Effect::Scan,
            Effect::DoubleMeteor,
            Effect::HeartBeat,
            Effect::MeteorRainbow,
            Effect::ColorfulMeteor,
            Effect::MeteorContest,
            Effect::MeteorMix,
            Effect::MopUp,
            Effect::ReturnArc,
            Effect::DoubleArc,
            Effect::Door,
            Effect::HeartBeatRunway,
            Effect::Disco,
            Effect::ElectricCurrent,
            Effect::Reflect,
            Effect::GradientRibbon,
            Effect::Wing,
            Effect::Drumming,
            Effect::Boomerang,
            Effect::CandyBox,
            Effect::Lottery,
            Effect::Staggered,
            Effect::Render,
            Effect::PingPong,
            Effect::Stack,
            Effect::Ripple,
            Effect::Collide,
            Effect::Endless,
            Effect::River,
            Effect::Duel,
            Effect::Hourglass,
            Effect::Pioneer,
            Effect::ShuttleRun,
            Effect::Pump,
            Effect::Bounce,
            Effect::CoverCycle,
            Effect::Wave,
            Effect::MeteorShower,
            Effect::Paint,
            Effect::Snooker,
            Effect::BlowUp,
            Effect::TailChasing,
            Effect::Racing,
            Effect::Intertwine,
            Effect::Kaleidoscope,
        ];
        for e in &implemented {
            assert!(effect_generator(e).is_some(), "{:?} not implemented", e);
        }
    }
}
