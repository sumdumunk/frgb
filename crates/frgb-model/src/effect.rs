use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Effect — single flat enum of all unique animations
// ---------------------------------------------------------------------------
//
// Each variant is a unique animation generator. The same variant is used
// regardless of device family (CL, SL, H2, etc.). Wire mode_id values
// differ per family and are looked up via const tables, not stored on
// the enum.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Effect {
    // --- Shared across most families ---
    Rainbow,
    RainbowMorph,
    StaticColor,
    Breathing,
    Runway,
    Meteor,
    Twinkle,
    ColorCycle,
    Mixing,
    Tide,
    ElectricCurrent,
    Reflect,
    GradientRibbon,
    Disco,
    Warning,
    MopUp,
    Hourglass,
    Voice,

    // --- CL / SL-INF ---
    Taichi,
    MeteorRainbow,
    ColorfulMeteor,
    Lottery,
    Scan,
    DoubleMeteor,
    MeteorContest,
    MeteorMix,
    ReturnArc,
    DoubleArc,
    Door,
    HeartBeat,
    HeartBeatRunway,
    Wing,
    Drumming,
    Boomerang,
    CandyBox,

    // --- SL ---
    Staggered,
    Render,
    PingPong,
    Stack,
    Ripple,
    Collide,
    Endless,
    River,
    Duel,
    Pioneer,
    ShuttleRun,

    // --- H2 (AIO) ---
    Pump,
    Bounce,

    // --- UI (wired hub) ---
    CoverCycle,
    Wave,
    MeteorShower,
    Paint,
    Snooker,
    BlowUp,

    // --- TL (reference project) ---
    TailChasing,
    Racing,
    Intertwine,
    Kaleidoscope,
}

impl Effect {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Rainbow => "Rainbow",
            Self::RainbowMorph => "RainbowMorph",
            Self::StaticColor => "StaticColor",
            Self::Breathing => "Breathing",
            Self::Runway => "Runway",
            Self::Meteor => "Meteor",
            Self::Twinkle => "Twinkle",
            Self::ColorCycle => "ColorCycle",
            Self::Mixing => "Mixing",
            Self::Tide => "Tide",
            Self::ElectricCurrent => "ElectricCurrent",
            Self::Reflect => "Reflect",
            Self::GradientRibbon => "GradientRibbon",
            Self::Disco => "Disco",
            Self::Warning => "Warning",
            Self::MopUp => "MopUp",
            Self::Hourglass => "Hourglass",
            Self::Voice => "Voice",
            Self::Taichi => "Taichi",
            Self::MeteorRainbow => "MeteorRainbow",
            Self::ColorfulMeteor => "ColorfulMeteor",
            Self::Lottery => "Lottery",
            Self::Scan => "Scan",
            Self::DoubleMeteor => "DoubleMeteor",
            Self::MeteorContest => "MeteorContest",
            Self::MeteorMix => "MeteorMix",
            Self::ReturnArc => "ReturnArc",
            Self::DoubleArc => "DoubleArc",
            Self::Door => "Door",
            Self::HeartBeat => "HeartBeat",
            Self::HeartBeatRunway => "HeartBeatRunway",
            Self::Wing => "Wing",
            Self::Drumming => "Drumming",
            Self::Boomerang => "Boomerang",
            Self::CandyBox => "CandyBox",
            Self::Staggered => "Staggered",
            Self::Render => "Render",
            Self::PingPong => "PingPong",
            Self::Stack => "Stack",
            Self::Ripple => "Ripple",
            Self::Collide => "Collide",
            Self::Endless => "Endless",
            Self::River => "River",
            Self::Duel => "Duel",
            Self::Pioneer => "Pioneer",
            Self::ShuttleRun => "ShuttleRun",
            Self::Pump => "Pump",
            Self::Bounce => "Bounce",
            Self::CoverCycle => "CoverCycle",
            Self::Wave => "Wave",
            Self::MeteorShower => "MeteorShower",
            Self::Paint => "Paint",
            Self::Snooker => "Snooker",
            Self::BlowUp => "BlowUp",
            Self::TailChasing => "TailChasing",
            Self::Racing => "Racing",
            Self::Intertwine => "Intertwine",
            Self::Kaleidoscope => "Kaleidoscope",
        }
    }

    /// Whether this effect uses a custom color parameter.
    ///
    /// Effects that generate their own colors (rainbow spectrum, random, etc.)
    /// ignore user-supplied colors.
    pub fn supports_color(&self) -> bool {
        !matches!(
            self,
            Self::Rainbow
                | Self::RainbowMorph
                | Self::Disco
                | Self::MeteorRainbow
                | Self::ColorfulMeteor
                | Self::Lottery
                | Self::CandyBox
                | Self::Voice
        )
    }

    /// Whether this effect responds to direction (Cw/Ccw/In/Out).
    ///
    /// Effects without directional motion ignore the direction parameter.
    pub fn supports_direction(&self) -> bool {
        matches!(
            self,
            Self::Rainbow
                | Self::Meteor
                | Self::ColorCycle
                | Self::Mixing
                | Self::Tide
                | Self::ElectricCurrent
                | Self::GradientRibbon
                | Self::MopUp
                | Self::Taichi
                | Self::MeteorRainbow
                | Self::ColorfulMeteor
                | Self::Scan
                | Self::ReturnArc
                | Self::HeartBeatRunway
                | Self::Boomerang
                | Self::Staggered
                | Self::Render
                | Self::Stack
                | Self::Endless
                | Self::River
                | Self::Pioneer
                | Self::Runway
                | Self::TailChasing
                | Self::Racing
        )
    }

    /// Whether this effect responds to the speed parameter.
    ///
    /// All implemented effects support speed through interval scaling.
    pub fn supports_speed(&self) -> bool {
        true
    }

    /// Case-insensitive name lookup. Accepts PascalCase, lowercase, and kebab-case.
    pub fn from_name(s: &str) -> Option<Self> {
        let norm: String = s
            .to_lowercase()
            .chars()
            .map(|c| if c == '-' || c == ' ' { '_' } else { c })
            .collect();
        ALL_EFFECTS
            .iter()
            .find(|e| norm.eq_ignore_ascii_case(e.name()))
            .copied()
    }

    pub fn all() -> &'static [Self] {
        &ALL_EFFECTS
    }
}

const ALL_EFFECTS: [Effect; 58] = [
    Effect::Rainbow,
    Effect::RainbowMorph,
    Effect::StaticColor,
    Effect::Breathing,
    Effect::Runway,
    Effect::Meteor,
    Effect::Twinkle,
    Effect::ColorCycle,
    Effect::Mixing,
    Effect::Tide,
    Effect::ElectricCurrent,
    Effect::Reflect,
    Effect::GradientRibbon,
    Effect::Disco,
    Effect::Warning,
    Effect::MopUp,
    Effect::Hourglass,
    Effect::Voice,
    Effect::Taichi,
    Effect::MeteorRainbow,
    Effect::ColorfulMeteor,
    Effect::Lottery,
    Effect::Scan,
    Effect::DoubleMeteor,
    Effect::MeteorContest,
    Effect::MeteorMix,
    Effect::ReturnArc,
    Effect::DoubleArc,
    Effect::Door,
    Effect::HeartBeat,
    Effect::HeartBeatRunway,
    Effect::Wing,
    Effect::Drumming,
    Effect::Boomerang,
    Effect::CandyBox,
    Effect::Staggered,
    Effect::Render,
    Effect::PingPong,
    Effect::Stack,
    Effect::Ripple,
    Effect::Collide,
    Effect::Endless,
    Effect::River,
    Effect::Duel,
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

// ---------------------------------------------------------------------------
// Device family — which protocol effect set a device uses
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceFamily {
    Cl,
    Sl,
    H2,
    Ui,
    Tl,
}

// ---------------------------------------------------------------------------
// Mode ID tables — wire protocol values per device family
// ---------------------------------------------------------------------------
//
// These map (DeviceFamily, mode_id) ↔ Effect. The mode_id is not part of
// the Effect identity — it's a wire encoding that differs per family.
// Currently unused in the TUZ pipeline (we send compressed buffers, not
// mode IDs), but needed for reading back firmware state.

pub fn mode_id(effect: Effect, family: DeviceFamily) -> Option<u8> {
    let table = match family {
        DeviceFamily::Cl => &CL_MODE_TABLE[..],
        DeviceFamily::Sl => &SL_MODE_TABLE[..],
        DeviceFamily::H2 => &H2_MODE_TABLE[..],
        DeviceFamily::Ui => &UI_MODE_TABLE[..],
        DeviceFamily::Tl => &TL_MODE_TABLE[..],
    };
    table.iter().find(|(e, _)| *e == effect).map(|(_, id)| *id)
}

pub fn effect_from_mode_id(family: DeviceFamily, id: u8) -> Option<Effect> {
    let table = match family {
        DeviceFamily::Cl => &CL_MODE_TABLE[..],
        DeviceFamily::Sl => &SL_MODE_TABLE[..],
        DeviceFamily::H2 => &H2_MODE_TABLE[..],
        DeviceFamily::Ui => &UI_MODE_TABLE[..],
        DeviceFamily::Tl => &TL_MODE_TABLE[..],
    };
    table.iter().find(|(_, mid)| *mid == id).map(|(e, _)| *e)
}

/// Effects available for a given device family.
pub fn effects_for_family(family: DeviceFamily) -> Vec<Effect> {
    let table = match family {
        DeviceFamily::Cl => &CL_MODE_TABLE[..],
        DeviceFamily::Sl => &SL_MODE_TABLE[..],
        DeviceFamily::H2 => &H2_MODE_TABLE[..],
        DeviceFamily::Ui => &UI_MODE_TABLE[..],
        DeviceFamily::Tl => &TL_MODE_TABLE[..],
    };
    table.iter().map(|(e, _)| *e).collect()
}

const CL_MODE_TABLE: [(Effect, u8); 34] = [
    (Effect::Rainbow, 1),
    (Effect::RainbowMorph, 2),
    (Effect::StaticColor, 3),
    (Effect::Breathing, 4),
    (Effect::Runway, 5),
    (Effect::Meteor, 6),
    (Effect::Twinkle, 7),
    (Effect::Taichi, 8),
    (Effect::ColorCycle, 9),
    (Effect::MopUp, 10),
    (Effect::MeteorRainbow, 11),
    (Effect::ColorfulMeteor, 12),
    (Effect::Lottery, 13),
    (Effect::Warning, 14),
    (Effect::Voice, 15),
    (Effect::Mixing, 16),
    (Effect::Tide, 17),
    (Effect::Scan, 18),
    (Effect::DoubleMeteor, 19),
    (Effect::MeteorContest, 20),
    (Effect::MeteorMix, 21),
    (Effect::ReturnArc, 22),
    (Effect::DoubleArc, 23),
    (Effect::Door, 24),
    (Effect::HeartBeat, 25),
    (Effect::HeartBeatRunway, 26),
    (Effect::Disco, 27),
    (Effect::ElectricCurrent, 28),
    (Effect::Reflect, 29),
    (Effect::GradientRibbon, 30),
    (Effect::Wing, 31),
    (Effect::Drumming, 32),
    (Effect::Boomerang, 33),
    (Effect::CandyBox, 34),
];

const SL_MODE_TABLE: [(Effect, u8); 25] = [
    (Effect::Rainbow, 1),
    (Effect::RainbowMorph, 2),
    (Effect::StaticColor, 3),
    (Effect::Breathing, 4),
    (Effect::Runway, 5),
    (Effect::Meteor, 6),
    (Effect::ColorCycle, 7),
    (Effect::Staggered, 8),
    (Effect::Tide, 9),
    (Effect::Mixing, 10),
    (Effect::Render, 11),
    (Effect::PingPong, 12),
    (Effect::Stack, 13),
    (Effect::Ripple, 14),
    (Effect::Collide, 15),
    (Effect::Reflect, 16),
    (Effect::ElectricCurrent, 17),
    (Effect::Endless, 18),
    (Effect::River, 19),
    (Effect::Duel, 20),
    (Effect::Hourglass, 21),
    (Effect::Pioneer, 22),
    (Effect::ShuttleRun, 23),
    (Effect::GradientRibbon, 24),
    (Effect::Twinkle, 25),
];

const H2_MODE_TABLE: [(Effect, u8); 11] = [
    (Effect::Rainbow, 1),
    (Effect::RainbowMorph, 2),
    (Effect::StaticColor, 3),
    (Effect::Breathing, 4),
    (Effect::Runway, 5),
    (Effect::Meteor, 6),
    (Effect::Taichi, 7),
    (Effect::Twinkle, 8),
    (Effect::Voice, 9),
    (Effect::Pump, 10),
    (Effect::Bounce, 11),
];

const UI_MODE_TABLE: [(Effect, u8); 23] = [
    (Effect::Rainbow, 1),
    (Effect::RainbowMorph, 2),
    (Effect::StaticColor, 3),
    (Effect::Breathing, 4),
    (Effect::Runway, 5),
    (Effect::Meteor, 6),
    (Effect::Stack, 7),
    (Effect::Twinkle, 8),
    (Effect::ColorCycle, 9),
    (Effect::CoverCycle, 10),
    (Effect::Wave, 11),
    (Effect::MeteorShower, 12),
    (Effect::Tide, 13),
    (Effect::ElectricCurrent, 14),
    (Effect::MopUp, 15),
    (Effect::Disco, 16),
    (Effect::Mixing, 17),
    (Effect::Paint, 18),
    (Effect::Snooker, 19),
    (Effect::Voice, 20),
    (Effect::BlowUp, 21),
    (Effect::Warning, 22),
    (Effect::Hourglass, 23),
];

const TL_MODE_TABLE: [(Effect, u8); 28] = [
    (Effect::Rainbow, 1),
    (Effect::RainbowMorph, 2),
    (Effect::StaticColor, 3),
    (Effect::Breathing, 4),
    (Effect::Runway, 5),
    (Effect::Meteor, 6),
    (Effect::ColorCycle, 7),
    (Effect::Staggered, 8),
    (Effect::Tide, 9),
    (Effect::Mixing, 10),
    (Effect::Voice, 11),
    (Effect::Door, 12),
    (Effect::Render, 13),
    (Effect::Ripple, 14),
    (Effect::Reflect, 15),
    (Effect::TailChasing, 16),
    (Effect::Paint, 17),
    (Effect::PingPong, 18),
    (Effect::Stack, 19),
    (Effect::CoverCycle, 20),
    (Effect::Wave, 21),
    (Effect::Racing, 22),
    (Effect::Lottery, 23),
    (Effect::Intertwine, 24),
    (Effect::MeteorShower, 25),
    (Effect::Collide, 26),
    (Effect::ElectricCurrent, 27),
    (Effect::Kaleidoscope, 28),
];

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_effects_count() {
        assert_eq!(Effect::all().len(), 58);
    }

    #[test]
    fn from_name_lowercase() {
        assert_eq!(Effect::from_name("rainbow"), Some(Effect::Rainbow));
        assert_eq!(Effect::from_name("breathing"), Some(Effect::Breathing));
        assert_eq!(Effect::from_name("candybox"), Some(Effect::CandyBox));
    }

    #[test]
    fn from_name_case_insensitive() {
        assert_eq!(Effect::from_name("RAINBOW"), Some(Effect::Rainbow));
        assert_eq!(Effect::from_name("StaticColor"), Some(Effect::StaticColor));
    }

    #[test]
    fn from_name_unknown() {
        assert_eq!(Effect::from_name("notaneffect"), None);
    }

    #[test]
    fn name_roundtrip() {
        for e in Effect::all() {
            let found = Effect::from_name(e.name());
            assert_eq!(found, Some(*e), "roundtrip failed for {:?}", e);
        }
    }

    #[test]
    fn cl_mode_id_rainbow() {
        assert_eq!(mode_id(Effect::Rainbow, DeviceFamily::Cl), Some(1));
    }

    #[test]
    fn sl_mode_id_twinkle() {
        // Twinkle is mode_id 25 on SL, 7 on CL — different IDs, same animation
        assert_eq!(mode_id(Effect::Twinkle, DeviceFamily::Sl), Some(25));
        assert_eq!(mode_id(Effect::Twinkle, DeviceFamily::Cl), Some(7));
    }

    #[test]
    fn mode_id_not_in_family() {
        // Staggered is SL-only
        assert_eq!(mode_id(Effect::Staggered, DeviceFamily::Cl), None);
        assert_eq!(mode_id(Effect::Staggered, DeviceFamily::Sl), Some(8));
    }

    #[test]
    fn effect_from_mode_id_roundtrip() {
        for &(effect, id) in &CL_MODE_TABLE {
            assert_eq!(effect_from_mode_id(DeviceFamily::Cl, id), Some(effect));
        }
        for &(effect, id) in &SL_MODE_TABLE {
            assert_eq!(effect_from_mode_id(DeviceFamily::Sl, id), Some(effect));
        }
    }

    #[test]
    fn effects_for_family_cl_count() {
        assert_eq!(effects_for_family(DeviceFamily::Cl).len(), 34);
    }

    #[test]
    fn effects_for_family_sl_count() {
        assert_eq!(effects_for_family(DeviceFamily::Sl).len(), 25);
    }

    #[test]
    fn effects_for_family_h2_count() {
        assert_eq!(effects_for_family(DeviceFamily::H2).len(), 11);
    }

    #[test]
    fn effects_for_family_ui_count() {
        assert_eq!(effects_for_family(DeviceFamily::Ui).len(), 23);
    }

    // --- Effect metadata ---

    #[test]
    fn rainbow_no_color_yes_direction() {
        assert!(!Effect::Rainbow.supports_color());
        assert!(Effect::Rainbow.supports_direction());
        assert!(Effect::Rainbow.supports_speed());
    }

    #[test]
    fn static_color_yes_color_no_direction() {
        assert!(Effect::StaticColor.supports_color());
        assert!(!Effect::StaticColor.supports_direction());
    }

    #[test]
    fn meteor_supports_both() {
        assert!(Effect::Meteor.supports_color());
        assert!(Effect::Meteor.supports_direction());
    }

    #[test]
    fn disco_no_color_no_direction() {
        assert!(!Effect::Disco.supports_color());
        assert!(!Effect::Disco.supports_direction());
    }

    #[test]
    fn all_effects_support_speed() {
        for e in Effect::all() {
            assert!(e.supports_speed(), "{:?} should support speed", e);
        }
    }

    #[test]
    fn tl_mode_table_basic() {
        assert_eq!(mode_id(Effect::TailChasing, DeviceFamily::Tl), Some(16));
        assert_eq!(mode_id(Effect::Racing, DeviceFamily::Tl), Some(22));
        assert_eq!(mode_id(Effect::Intertwine, DeviceFamily::Tl), Some(24));
        assert_eq!(mode_id(Effect::Kaleidoscope, DeviceFamily::Tl), Some(28));
    }

    #[test]
    fn tl_effects_count() {
        assert_eq!(effects_for_family(DeviceFamily::Tl).len(), 28);
    }

    #[test]
    fn tl_roundtrip() {
        for &(effect, id) in &TL_MODE_TABLE {
            assert_eq!(effect_from_mode_id(DeviceFamily::Tl, id), Some(effect));
        }
    }

    #[test]
    fn new_effects_name_roundtrip() {
        for name in ["TailChasing", "Racing", "Intertwine", "Kaleidoscope"] {
            let e = Effect::from_name(name).unwrap();
            assert_eq!(e.name(), name);
        }
    }

    #[test]
    fn every_effect_in_at_least_one_table_or_exempted() {
        let exempt = [Effect::Voice]; // Needs audio input, intentionally not in every table
        for effect in Effect::all() {
            if exempt.contains(effect) {
                continue;
            }
            let in_any = [
                DeviceFamily::Cl,
                DeviceFamily::Sl,
                DeviceFamily::H2,
                DeviceFamily::Ui,
                DeviceFamily::Tl,
            ]
            .iter()
            .any(|f| mode_id(*effect, *f).is_some());
            assert!(in_any, "Effect::{:?} is not in any mode table and not exempted", effect);
        }
    }

    #[test]
    fn self_generated_color_effects() {
        // Effects that generate their own colors should not support custom color
        let no_color = [
            Effect::Rainbow,
            Effect::RainbowMorph,
            Effect::Disco,
            Effect::MeteorRainbow,
            Effect::ColorfulMeteor,
            Effect::Lottery,
            Effect::CandyBox,
            Effect::Voice,
        ];
        for e in &no_color {
            assert!(!e.supports_color(), "{:?} should not support color", e);
        }
    }
}
