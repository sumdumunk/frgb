use frgb_model::effect::Effect;
use frgb_model::rgb::EffectDirection;
#[cfg(test)]
use frgb_model::rgb::{EffectParams, EffectScope, Rgb, RgbMode, Ring};
#[cfg(test)]
use frgb_model::Brightness;

// RGB mode strings shared between Slint UI and Rust callbacks.
#[cfg(test)]
pub const MODE_OFF: &str = "off";
#[cfg(test)]
pub const MODE_STATIC: &str = "static";
#[cfg(test)]
pub const MODE_EFFECT: &str = "effect";
#[cfg(test)]
pub const MODE_PER_FAN: &str = "per-fan";

// ---------------------------------------------------------------------------
// Effect display name table
// ---------------------------------------------------------------------------
//
// 47 displayable effects. Omits Voice and the 6 UI-only wired hub effects:
// CoverCycle, Wave, MeteorShower, Paint, Snooker, BlowUp.

const EFFECT_TABLE: &[(Effect, &str)] = &[
    // Shared (17)
    (Effect::Rainbow, "Rainbow"),
    (Effect::RainbowMorph, "Rainbow Morph"),
    (Effect::StaticColor, "Static Color"),
    (Effect::Breathing, "Breathing"),
    (Effect::Runway, "Runway"),
    (Effect::Meteor, "Meteor"),
    (Effect::Twinkle, "Twinkle"),
    (Effect::ColorCycle, "Color Cycle"),
    (Effect::Mixing, "Mixing"),
    (Effect::Tide, "Tide"),
    (Effect::ElectricCurrent, "Electric Current"),
    (Effect::Reflect, "Reflect"),
    (Effect::GradientRibbon, "Gradient Ribbon"),
    (Effect::Disco, "Disco"),
    (Effect::Warning, "Warning"),
    (Effect::MopUp, "Mop Up"),
    (Effect::Hourglass, "Hourglass"),
    // CL/SL-INF (17)
    (Effect::Taichi, "Taichi"),
    (Effect::MeteorRainbow, "Meteor Rainbow"),
    (Effect::ColorfulMeteor, "Colorful Meteor"),
    (Effect::Lottery, "Lottery"),
    (Effect::Scan, "Scan"),
    (Effect::DoubleMeteor, "Double Meteor"),
    (Effect::MeteorContest, "Meteor Contest"),
    (Effect::MeteorMix, "Meteor Mix"),
    (Effect::ReturnArc, "Return Arc"),
    (Effect::DoubleArc, "Double Arc"),
    (Effect::Door, "Door"),
    (Effect::HeartBeat, "Heart Beat"),
    (Effect::HeartBeatRunway, "Heart Beat Runway"),
    (Effect::Wing, "Wing"),
    (Effect::Drumming, "Drumming"),
    (Effect::Boomerang, "Boomerang"),
    (Effect::CandyBox, "Candy Box"),
    // SL (11)
    (Effect::Staggered, "Staggered"),
    (Effect::Render, "Render"),
    (Effect::PingPong, "Ping Pong"),
    (Effect::Stack, "Stack"),
    (Effect::Ripple, "Ripple"),
    (Effect::Collide, "Collide"),
    (Effect::Endless, "Endless"),
    (Effect::River, "River"),
    (Effect::Duel, "Duel"),
    (Effect::Pioneer, "Pioneer"),
    (Effect::ShuttleRun, "Shuttle Run"),
    // H2 (2)
    (Effect::Pump, "Pump"),
    (Effect::Bounce, "Bounce"),
];

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Find an effect by its display name (case-insensitive).
pub fn effect_by_name(name: &str) -> Option<Effect> {
    EFFECT_TABLE
        .iter()
        .find(|(_, display)| display.eq_ignore_ascii_case(name))
        .map(|(effect, _)| *effect)
}

/// Get the human-readable display name for an effect.
pub fn effect_display_name(effect: &Effect) -> &'static str {
    EFFECT_TABLE
        .iter()
        .find(|(e, _)| e == effect)
        .map(|(_, name)| *name)
        .unwrap_or_else(|| effect.name())
}

/// Return all displayable effect names as strings (for the `[string]` effect-names property).
pub fn effect_display_names() -> Vec<&'static str> {
    EFFECT_TABLE.iter().map(|(_, name)| *name).collect()
}

/// Reverse lookup: find Effect from display name.
pub fn effect_from_display_name(name: &str) -> Option<Effect> {
    EFFECT_TABLE.iter().find(|(_, n)| *n == name).map(|(e, _)| *e)
}

/// Map a ring index to a Ring value. 0=Inner, 1=Outer, 2=Both.
#[cfg(test)]
fn ring_from_index(idx: i32) -> Ring {
    match idx {
        0 => Ring::Inner,
        1 => Ring::Outer,
        _ => Ring::Both,
    }
}

/// Map a direction index to an EffectDirection. 0=Cw, 1=Ccw, 2=Out, 3=In.
pub fn direction_from_index(idx: i32) -> EffectDirection {
    match idx {
        1 => EffectDirection::Ccw,
        2 => EffectDirection::Out,
        3 => EffectDirection::In,
        _ => EffectDirection::Cw,
    }
}

/// Map an EffectDirection to its index. Inverse of `direction_from_index`.
pub fn direction_to_index(dir: &EffectDirection) -> i32 {
    match dir {
        EffectDirection::Cw => 0,
        EffectDirection::Ccw => 1,
        EffectDirection::Out => 2,
        EffectDirection::In => 3,
    }
}

/// Construct an RgbMode from UI state values.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn build_rgb_mode(
    mode: &str,
    effect_name: &str,
    r: i32,
    g: i32,
    b: i32,
    ring_idx: i32,
    direction_idx: i32,
    speed: i32,
    brightness: i32,
) -> Option<RgbMode> {
    let ring = ring_from_index(ring_idx);
    let color = Rgb {
        r: r.clamp(0, 255) as u8,
        g: g.clamp(0, 255) as u8,
        b: b.clamp(0, 255) as u8,
    };

    match mode {
        MODE_OFF => Some(RgbMode::Off),
        MODE_STATIC => Some(RgbMode::Static {
            ring,
            color,
            brightness: Brightness::new(brightness.clamp(0, 255) as u8),
        }),
        MODE_EFFECT => {
            let effect = effect_by_name(effect_name)?;
            let params = EffectParams {
                speed: speed.clamp(1, 5) as u8,
                direction: direction_from_index(direction_idx),
                brightness: Brightness::new(brightness.clamp(0, 255) as u8),
                color: Some(color),
                scope: EffectScope::All,
            };
            Some(RgbMode::Effect { effect, params, ring })
        }
        MODE_PER_FAN => None,
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_table_count() {
        assert_eq!(EFFECT_TABLE.len(), 47);
    }

    #[test]
    fn no_omitted_effects_in_table() {
        let omitted = [
            Effect::Voice,
            Effect::CoverCycle,
            Effect::Wave,
            Effect::MeteorShower,
            Effect::Paint,
            Effect::Snooker,
            Effect::BlowUp,
        ];
        for (effect, _) in EFFECT_TABLE {
            assert!(
                !omitted.contains(effect),
                "omitted effect {:?} found in display table",
                effect
            );
        }
    }

    #[test]
    fn effect_by_name_roundtrip() {
        for (effect, display) in EFFECT_TABLE {
            let found = effect_by_name(display);
            assert_eq!(found, Some(*effect), "lookup failed for '{}'", display);
        }
    }

    #[test]
    fn effect_by_name_case_insensitive() {
        assert_eq!(effect_by_name("rainbow"), Some(Effect::Rainbow));
        assert_eq!(effect_by_name("CANDY BOX"), Some(Effect::CandyBox));
    }

    #[test]
    fn effect_display_name_known() {
        assert_eq!(effect_display_name(&Effect::Rainbow), "Rainbow");
        assert_eq!(effect_display_name(&Effect::CandyBox), "Candy Box");
        assert_eq!(effect_display_name(&Effect::ElectricCurrent), "Electric Current");
    }

    #[test]
    fn effect_display_names_count() {
        assert_eq!(effect_display_names().len(), 47);
    }

    #[test]
    fn ring_from_index_all() {
        assert_eq!(ring_from_index(0), Ring::Inner);
        assert_eq!(ring_from_index(1), Ring::Outer);
        assert_eq!(ring_from_index(2), Ring::Both);
        assert_eq!(ring_from_index(99), Ring::Both);
    }

    #[test]
    fn direction_from_index_all() {
        assert_eq!(direction_from_index(0), EffectDirection::Cw);
        assert_eq!(direction_from_index(1), EffectDirection::Ccw);
        assert_eq!(direction_from_index(2), EffectDirection::Out);
        assert_eq!(direction_from_index(3), EffectDirection::In);
        assert_eq!(direction_from_index(99), EffectDirection::Cw);
    }

    #[test]
    fn build_rgb_mode_off() {
        let result = build_rgb_mode("off", "", 0, 0, 0, 2, 0, 3, 255);
        assert_eq!(result, Some(RgbMode::Off));
    }

    #[test]
    fn build_rgb_mode_static() {
        let result = build_rgb_mode("static", "", 255, 0, 128, 2, 0, 3, 255);
        assert_eq!(
            result,
            Some(RgbMode::Static {
                ring: Ring::Both,
                color: Rgb { r: 255, g: 0, b: 128 },
                brightness: Brightness::new(255),
            })
        );
    }

    #[test]
    fn build_rgb_mode_effect() {
        let result = build_rgb_mode("effect", "Rainbow", 0, 120, 255, 0, 1, 3, 200);
        match result {
            Some(RgbMode::Effect { effect, params, ring }) => {
                assert_eq!(effect, Effect::Rainbow);
                assert_eq!(ring, Ring::Inner);
                assert_eq!(params.speed, 3);
                assert_eq!(params.direction, EffectDirection::Ccw);
                assert_eq!(params.brightness, Brightness::new(200));
                assert_eq!(params.color, Some(Rgb { r: 0, g: 120, b: 255 }));
                assert_eq!(params.scope, EffectScope::All);
            }
            other => panic!("expected Effect mode, got {:?}", other),
        }
    }

    #[test]
    fn build_rgb_mode_effect_unknown_returns_none() {
        let result = build_rgb_mode("effect", "NotAnEffect", 0, 0, 0, 2, 0, 3, 255);
        assert_eq!(result, None);
    }

    #[test]
    fn build_rgb_mode_per_fan_returns_none() {
        let result = build_rgb_mode("per-fan", "", 0, 0, 0, 2, 0, 3, 255);
        assert_eq!(result, None);
    }
}
