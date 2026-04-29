//! Light show layer — keyframe effects, scenes, and sequences.
//!
//! Three distinct concepts:
//! - **KeyframeEffect**: user-created animation as data (not code)
//! - **Scene**: complete visual state for one device group at a point in time
//! - **Sequence**: orchestrated scene changes over time (the "light show")

use serde::{Deserialize, Serialize};

use crate::lcd::LcdConfig;
use crate::rgb::{Rgb, RgbMode};
use crate::speed::SpeedMode;
use crate::ValidatedName;

// ---------------------------------------------------------------------------
// Playback — shared by keyframes and sequences
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Playback {
    /// Play once and stop on last frame/step.
    Once,
    /// Loop from end back to start.
    Loop,
    /// Play forward then backward, repeat.
    PingPong,
    /// Loop N times then stop.
    Count(u16),
}

// ---------------------------------------------------------------------------
// Blend — how one keyframe transitions to the next
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Blend {
    /// Hard cut — no interpolation.
    Cut,
    /// Linear crossfade over the hold duration.
    Fade,
}

// ---------------------------------------------------------------------------
// KeyframeEffect — user-created animation described as data
// ---------------------------------------------------------------------------

/// A user-created animation. Each frame specifies per-LED colors with
/// variable timing and optional crossfade to the next frame.
/// Stored in config, referenced by name from RgbMode::Keyframe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyframeEffect {
    pub name: ValidatedName,
    pub frames: Vec<KeyFrame>,
    pub playback: Playback,
}

/// A single frame in a keyframe animation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyFrame {
    /// Per-LED colors. If shorter than device LED count, remaining LEDs are black.
    pub leds: Vec<Rgb>,
    /// How long this frame is held before advancing (ms).
    pub hold_ms: u16,
    /// Interpolation to the next frame.
    pub blend: Blend,
}

// ---------------------------------------------------------------------------
// Scene — complete visual state for one device group
// ---------------------------------------------------------------------------

/// What a device group looks like at a point in time.
/// This is the atomic unit of state — profiles save these, sequences transition between them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub rgb: RgbMode,
    pub speed: Option<SpeedMode>,
    pub lcd: Option<LcdConfig>,
}

// ---------------------------------------------------------------------------
// Sequence — orchestrated scene changes over time
// ---------------------------------------------------------------------------

/// A timeline of scenes played by the daemon's sequence engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sequence {
    pub name: ValidatedName,
    pub steps: Vec<SequenceStep>,
    pub playback: Playback,
}

/// One step in a sequence timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SequenceStep {
    /// Which groups this step targets. None = all groups.
    pub target: Option<SequenceTarget>,
    /// The scene to apply.
    pub scene: Scene,
    /// How long this step holds before advancing (ms).
    pub duration_ms: u32,
    /// Transition INTO this step from the previous one.
    pub transition: Transition,
}

/// Which device groups a sequence step targets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SequenceTarget {
    Group(u8),
    Groups(Vec<u8>),
}

/// How to transition into a sequence step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Transition {
    /// Instant switch.
    Cut,
    /// Crossfade over given duration (ms). Happens before the hold starts.
    Crossfade { duration_ms: u32 },
}

// ---------------------------------------------------------------------------
// EffectCycle — L-Connect built-in effect cycling
// ---------------------------------------------------------------------------

/// L-Connect's "Effect Sequence" feature: cycle through built-in effects
/// with per-step timing and optional crossfade merge.
/// Daemon-driven — the app re-sends TUZ buffers each cycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectCycle {
    pub steps: Vec<EffectCycleStep>,
    pub playback: Playback,
}

/// One effect in an effect cycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectCycleStep {
    pub effect: crate::effect::Effect,
    pub params: crate::rgb::EffectParams,
    /// How long this effect plays before advancing (ms).
    pub duration_ms: u32,
    /// Crossfade merge with the next effect.
    pub merge: bool,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SpeedPercent;

    #[test]
    fn playback_serde_roundtrip() {
        let p = Playback::Count(5);
        let json = serde_json::to_string(&p).unwrap();
        let deser: Playback = serde_json::from_str(&json).unwrap();
        assert_eq!(deser, Playback::Count(5));
    }

    #[test]
    fn keyframe_effect_serde() {
        let effect = KeyframeEffect {
            name: ValidatedName::new("test").unwrap(),
            frames: vec![KeyFrame {
                leds: vec![Rgb { r: 254, g: 0, b: 0 }],
                hold_ms: 100,
                blend: Blend::Fade,
            }],
            playback: Playback::Loop,
        };
        let json = serde_json::to_string(&effect).unwrap();
        let deser: KeyframeEffect = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.name, "test");
        assert_eq!(deser.frames.len(), 1);
    }

    #[test]
    fn scene_serde() {
        let sp50 = SpeedPercent::new(50);
        let scene = Scene {
            rgb: RgbMode::Off,
            speed: Some(SpeedMode::Manual(sp50)),
            lcd: None,
        };
        let json = serde_json::to_string(&scene).unwrap();
        let deser: Scene = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.speed, Some(SpeedMode::Manual(sp50)));
    }

    #[test]
    fn sequence_step_with_crossfade() {
        let step = SequenceStep {
            target: None,
            scene: Scene {
                rgb: RgbMode::Off,
                speed: None,
                lcd: None,
            },
            duration_ms: 5000,
            transition: Transition::Crossfade { duration_ms: 1000 },
        };
        assert_eq!(step.duration_ms, 5000);
        if let Transition::Crossfade { duration_ms } = step.transition {
            assert_eq!(duration_ms, 1000);
        } else {
            panic!("expected crossfade");
        }
    }

    #[test]
    fn effect_cycle_serde() {
        let cycle = EffectCycle {
            steps: vec![EffectCycleStep {
                effect: crate::effect::Effect::Rainbow,
                params: crate::rgb::EffectParams::default(),
                duration_ms: 10000,
                merge: false,
            }],
            playback: Playback::Loop,
        };
        let json = serde_json::to_string(&cycle).unwrap();
        let deser: EffectCycle = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.steps.len(), 1);
    }
}
