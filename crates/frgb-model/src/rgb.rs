use serde::{Deserialize, Serialize};

use crate::effect::Effect;
use crate::sensor::Sensor;
use crate::Brightness;
use crate::Temperature;

// ---------------------------------------------------------------------------
// Rgb
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const BLACK: Self = Self { r: 0, g: 0, b: 0 };
    /// Protocol-safe white (254, 254, 254). The Lian Li protocol reserves 0xFF.
    /// Use `to_protocol()` to clamp arbitrary Rgb values for wire encoding.
    pub const WHITE: Self = Self { r: 254, g: 254, b: 254 };

    /// Parse a 6-char hex string (with or without leading `#`).
    pub fn from_hex(hex: &str) -> Result<Self, String> {
        let hex = hex.trim().trim_start_matches('#');
        if hex.len() != 6 {
            return Err(format!("expected 6 hex chars, got {}", hex.len()));
        }
        let parse = |s: &str| u8::from_str_radix(s, 16).map_err(|e| format!("invalid hex: {e}"));
        Ok(Self {
            r: parse(&hex[0..2])?,
            g: parse(&hex[2..4])?,
            b: parse(&hex[4..6])?,
        })
    }

    /// Lowercase 6-char hex string (no `#`).
    pub fn to_hex(&self) -> String {
        format!("{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Clamp each channel to 0–254 (Lian Li protocol: 0xff is reserved).
    pub fn to_protocol(&self) -> [u8; 3] {
        [self.r.min(0xfe), self.g.min(0xfe), self.b.min(0xfe)]
    }

    /// Look up a color by name (case-insensitive).
    ///
    /// Values are tuned for WS2812B-style ARGB LEDs where the green element
    /// is physically brighter than red at the same PWM. Mixed hues have their
    /// green channel reduced to compensate — without this, "orange" appears
    /// yellow and "yellow" appears lime/green on real hardware.
    pub fn from_name(name: &str) -> Option<Self> {
        let n = name.to_lowercase();
        match n.as_str() {
            "red" => Some(Self { r: 254, g: 0, b: 0 }),
            "orange" => Some(Self { r: 254, g: 60, b: 0 }),
            "yellow" => Some(Self { r: 254, g: 160, b: 0 }),
            "green" => Some(Self { r: 0, g: 254, b: 0 }),
            "cyan" => Some(Self { r: 0, g: 254, b: 254 }),
            "blue" => Some(Self { r: 0, g: 0, b: 254 }),
            "purple" => Some(Self { r: 127, g: 0, b: 254 }),
            "pink" | "magenta" => Some(Self { r: 254, g: 0, b: 254 }),
            "white" => Some(Self { r: 254, g: 254, b: 254 }),
            "black" | "off" => Some(Self { r: 0, g: 0, b: 0 }),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Ring / direction / scope enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Ring {
    Inner,
    Outer,
    Both,
}

/// Per-fan sub-zone — three sections (top/middle/bottom) on each side
/// (inner/outer). Hardware-confirmed wire-index ranges per device type;
/// see `frgb_core::services::rgb::sub_zone` for the mapping. Currently
/// supported devices: TL (TlWireless / TlLcdWireless) and SL
/// (SlWireless / SlLcdWireless / SlV2). Other devices reject SubZones
/// composition with InvalidInput.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SubZone {
    InnerTop,
    InnerMiddle,
    InnerBottom,
    OuterTop,
    OuterMiddle,
    OuterBottom,
}

impl SubZone {
    pub fn is_inner(&self) -> bool {
        matches!(self, Self::InnerTop | Self::InnerMiddle | Self::InnerBottom)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::InnerTop => "inner-top",
            Self::InnerMiddle => "inner-middle",
            Self::InnerBottom => "inner-bottom",
            Self::OuterTop => "outer-top",
            Self::OuterMiddle => "outer-middle",
            Self::OuterBottom => "outer-bottom",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EffectDirection {
    Cw,
    Ccw,
    Out,
    In,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EffectScope {
    Outer,
    Inner,
    All,
    Down,
    Up,
    TlAll,
    Center,
    H2,
    ClOuter,
    ClCenter,
    ClAll,
    Front,
    Behind,
}

// ---------------------------------------------------------------------------
// EffectParams
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectParams {
    pub speed: u8,
    pub direction: EffectDirection,
    /// Brightness scale factor (0-255). Used in apply_brightness, not a wire value.
    /// 255 = identity (no dimming). For protocol transmission, values are clamped to 254.
    pub brightness: Brightness,
    pub color: Option<Rgb>,
    pub scope: EffectScope,
}

impl Default for EffectParams {
    fn default() -> Self {
        Self {
            speed: 3,
            direction: EffectDirection::Cw,
            brightness: Brightness::new(255),
            color: None,
            scope: EffectScope::All,
        }
    }
}

// ---------------------------------------------------------------------------
// Fan color / LED assignments
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FanColorAssignment {
    pub inner: Option<Rgb>,
    pub outer: Option<Rgb>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FanLedAssignment {
    pub inner: Vec<Rgb>,
    pub outer: Vec<Rgb>,
}

// ---------------------------------------------------------------------------
// Temperature-driven RGB
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TempColorPoint {
    pub temp: Temperature,
    pub color: Rgb,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TempRgbConfig {
    pub sensor: Sensor,
    pub gradient: Vec<TempColorPoint>,
    pub ring: Ring,
}

// ---------------------------------------------------------------------------
// Zone-based composition
// ---------------------------------------------------------------------------

/// What to render in a single LED zone (inner or outer ring of one fan).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZoneSource {
    /// Solid color with brightness (0-255, 255 = full).
    Color { color: Rgb, brightness: Brightness },
    /// Animated effect with full parameters (speed, direction, brightness, color).
    Effect { effect: Effect, params: EffectParams },
    /// LEDs off (black).
    Off,
}

/// Per-fan specification: independent inner and outer zone sources.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FanZoneSpec {
    pub inner: ZoneSource,
    pub outer: ZoneSource,
}

impl FanZoneSpec {
    /// Create a spec that applies a single source to the specified ring.
    pub fn from_ring(ring: Ring, source: ZoneSource) -> Self {
        match ring {
            Ring::Both => Self {
                inner: source.clone(),
                outer: source,
            },
            Ring::Inner => Self {
                inner: source,
                outer: ZoneSource::Off,
            },
            Ring::Outer => Self {
                inner: ZoneSource::Off,
                outer: source,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// RgbMode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RgbMode {
    Off,
    /// Single solid color for all LEDs on the specified ring.
    Static {
        ring: Ring,
        color: Rgb,
        brightness: Brightness,
    },
    /// Per-fan color assignment (inner / outer ring independently).
    PerFan(Vec<FanColorAssignment>),
    /// Per-LED color assignment.
    PerLed(Vec<FanLedAssignment>),
    /// Hardware effect with parameters, applied to the specified ring.
    Effect {
        effect: Effect,
        params: EffectParams,
        ring: Ring,
    },
    /// Temperature-reactive gradient.
    TempRgb(TempRgbConfig),
    /// Composed per-fan, per-zone specification.
    /// Each fan gets independent inner/outer zone sources (color, effect, or off).
    /// If fewer specs than fans, the last spec repeats for remaining fans.
    Composed(Vec<FanZoneSpec>),
    /// Six-zone color assignment (inner/outer × top/middle/bottom). Per-device
    /// support: TL (TlWireless, TlLcdWireless) and SL (SlWireless,
    /// SlLcdWireless, SlV2). Other device types reject this mode.
    /// None for a zone leaves it black.
    SubZones {
        inner_top: Option<Rgb>,
        inner_middle: Option<Rgb>,
        inner_bottom: Option<Rgb>,
        outer_top: Option<Rgb>,
        outer_middle: Option<Rgb>,
        outer_bottom: Option<Rgb>,
        brightness: Brightness,
    },
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_from_hex() {
        let c = Rgb::from_hex("ff4400").unwrap();
        assert_eq!(c, Rgb { r: 255, g: 68, b: 0 });
    }

    #[test]
    fn rgb_from_hex_with_hash() {
        let c = Rgb::from_hex("#ff4400").unwrap();
        assert_eq!(c, Rgb { r: 255, g: 68, b: 0 });
    }

    #[test]
    fn rgb_to_hex() {
        let c = Rgb { r: 255, g: 68, b: 0 };
        assert_eq!(c.to_hex(), "ff4400");
    }

    #[test]
    fn rgb_clamp_to_protocol() {
        let c = Rgb { r: 255, g: 255, b: 255 };
        let clamped = c.to_protocol();
        assert_eq!(clamped, [0xfe, 0xfe, 0xfe]);
    }

    #[test]
    fn rgb_from_name() {
        let c = Rgb::from_name("red").unwrap();
        assert_eq!(c, Rgb { r: 254, g: 0, b: 0 });
        let c = Rgb::from_name("blue").unwrap();
        assert_eq!(c, Rgb { r: 0, g: 0, b: 254 });
        assert!(Rgb::from_name("notacolor").is_none());
    }
}
