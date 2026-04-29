//! HydroShift II AIO pump control — RPM to PWM scaling and aio_param layout.
//!
//! The pump is controlled over the Lian Li RF protocol, not USB. The master device
//! streams a 240-byte RF frame (opcode 0x12 0x21) carrying a 32-byte `aio_param`
//! buffer that the pump receiver latches onto.
//!
//! Two hardware variants share the protocol but use different RPM→PWM scaling:
//!   - WaterBlock (HydroShift II Circle, dev_type 10, max 2500 RPM)
//!   - WaterBlock2 (HydroShift II Square, dev_type 11, max 3200 RPM)
//!
//! Two RPM→PWM scaling functions handle the Circle and Square variants respectively.

/// Minimum pump RPM accepted by the hardware (both variants).
pub const PUMP_MIN_RPM: u16 = 1600;

/// Maximum pump RPM for Circle / WaterBlock variant.
pub const PUMP_MAX_RPM_CIRCLE: u16 = 2500;

/// Maximum pump RPM for Square / WaterBlock2 variant.
pub const PUMP_MAX_RPM_SQUARE: u16 = 3200;

/// Convert a target pump RPM to the `aio_param[28..30]` PWM value for
/// WaterBlock / HydroShift II Circle (dev_type 10).
///
/// Piecewise linear map from the input RPM (clamped to 1600..=2500)
/// to an inverse PWM-like register value (lower = faster).
pub fn water_block_pump_pwm(rpm: u16) -> u16 {
    let rpm = rpm.clamp(PUMP_MIN_RPM, PUMP_MAX_RPM_CIRCLE);
    if rpm <= 1720 {
        1500u16.saturating_sub(((rpm - 1600) as f32 * 1.667) as u16)
    } else if rpm <= 1870 {
        1300u16.saturating_sub((rpm - 1720) * 2)
    } else if rpm <= 2000 {
        1000u16.saturating_sub(((rpm - 1870) as f32 * 1.23) as u16)
    } else if rpm <= 2300 {
        840u16.saturating_sub((rpm - 2000) * 2)
    } else if rpm <= 2400 {
        240u16.saturating_sub(((rpm - 2300) as f32 * 1.8) as u16)
    } else {
        // 2401..=2500
        60u16.saturating_sub(((rpm - 2400) as f32 * 0.5) as u16)
    }
}

/// Convert a target pump RPM to the `aio_param[28..30]` PWM value for
/// WaterBlock2 / HydroShift II Square (dev_type 11).
///
/// Piecewise linear map from RPM (clamped to 1600..=3200) to PWM register.
pub fn water_block2_pump_pwm(rpm: u16) -> u16 {
    let rpm = rpm.clamp(PUMP_MIN_RPM, PUMP_MAX_RPM_SQUARE);
    if rpm <= 1800 {
        1590u16.saturating_sub(((rpm - 1600) as f32 * 0.95) as u16)
    } else if rpm <= 2000 {
        1400u16.saturating_sub(rpm - 1800)
    } else if rpm <= 2200 {
        1200u16.saturating_sub(rpm - 2000)
    } else if rpm <= 2400 {
        1000u16.saturating_sub(rpm - 2200)
    } else if rpm <= 2600 {
        800u16.saturating_sub(rpm - 2400)
    } else if rpm <= 2800 {
        580u16.saturating_sub(((rpm - 2600) as f32 * 1.11) as u16)
    } else if rpm <= 3000 {
        330u16.saturating_sub(((rpm - 2800) as f32 * 1.2) as u16)
    } else {
        // 3001..=3200
        90u16.saturating_sub(((rpm - 3000) as f32 * 0.45) as u16)
    }
}

/// AIO pump variant — determines the RPM→PWM scaling formula.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PumpVariant {
    /// HydroShift II Circle / WaterBlock (dev_type 10, max 2500 RPM).
    Circle,
    /// HydroShift II Square / WaterBlock2 (dev_type 11, max 3200 RPM).
    Square,
}

impl PumpVariant {
    /// Max RPM for this variant (upper clamp for scaling).
    pub fn max_rpm(self) -> u16 {
        match self {
            Self::Circle => PUMP_MAX_RPM_CIRCLE,
            Self::Square => PUMP_MAX_RPM_SQUARE,
        }
    }

    /// Apply the variant-specific RPM→PWM scaling.
    pub fn rpm_to_pwm(self, rpm: u16) -> u16 {
        match self {
            Self::Circle => water_block_pump_pwm(rpm),
            Self::Square => water_block2_pump_pwm(rpm),
        }
    }
}

/// Map a 0-100% intensity value to the full RPM range (PUMP_MIN_RPM..=max_rpm).
///
/// `pct` is clamped to 0..=100. 0% → PUMP_MIN_RPM, 100% → max_rpm (variant-specific).
pub fn pct_to_rpm(variant: PumpVariant, pct: u8) -> u16 {
    let pct = pct.min(100) as u32;
    let max = variant.max_rpm() as u32;
    let range = max - PUMP_MIN_RPM as u32;
    (PUMP_MIN_RPM as u32 + pct * range / 100) as u16
}

/// Build a 32-byte `aio_param` buffer with defaults + specified pump PWM.
///
/// `aio_param[25] = 80` (default LCD brightness); `array[26] = 1` (fixed).
/// All other fields are zero for this minimal payload:
/// CPU/GPU temp inputs disabled, fan speed not driven from sensors, colors black,
/// pump enabled so the receiver honors our new PWM value.
///
/// Byte layout:
///   [0]     cpu_temp
///   [1]     cpu_load
///   [2]     gpu_temp
///   [3]     gpu_load
///   [4..6]  fan_speed (big-endian)
///   [6]     loop_interval
///   [7]     pump_enable
///   [8]     cpu_temp_enable
///   [9]     cpu_load_enable
///   [10]    gpu_temp_enable
///   [11]    gpu_load_enable
///   [12]    fan_speed_enable
///   [13..17]str_color ARGB
///   [17..21]val_color ARGB
///   [21..25]uint_color ARGB
///   [25]    lcd_brightness
///   [26]    = 1 (fixed)
///   [27]    aio_theme_index
///   [28..30]pump_pwm (big-endian, from rpm_to_pwm)
///   [30]    rotation
///   [31]    = 0
pub fn build_aio_param(pump_pwm: u16) -> [u8; 32] {
    let mut ap = [0u8; 32];
    ap[7] = 1; // pump_enable
    ap[25] = 80; // lcd_brightness default
    ap[26] = 1; // fixed
    ap[28] = (pump_pwm >> 8) as u8;
    ap[29] = (pump_pwm & 0xFF) as u8;
    ap
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- water_block_pump_pwm (Circle) ----

    #[test]
    fn water_block_clamps_below_min() {
        // 1500 < 1600 → clamps to 1600 → 1500 - 0 = 1500
        assert_eq!(water_block_pump_pwm(1500), 1500);
    }

    #[test]
    fn water_block_clamps_above_max() {
        // 3000 > 2500 → clamps to 2500 → 60 - 100*0.5 = 10
        assert_eq!(water_block_pump_pwm(3000), 10);
    }

    #[test]
    fn water_block_at_segment_boundaries() {
        // 1600 → 1500 (slowest)
        assert_eq!(water_block_pump_pwm(1600), 1500);
        // 1720 → first branch: 1500 - 120*1.667 = 1500 - 200 = 1300
        assert_eq!(water_block_pump_pwm(1720), 1300);
        // 1870 → second branch (<=1870): 1300 - 150*2 = 1000
        assert_eq!(water_block_pump_pwm(1870), 1000);
        // 2000 → third branch (<=2000): 1000 - 130*1.23 = 1000 - 159 = 841
        assert_eq!(water_block_pump_pwm(2000), 841);
        // 2500 → last branch: 60 - 100*0.5 = 10
        assert_eq!(water_block_pump_pwm(2500), 10);
    }

    #[test]
    fn water_block_monotonic_non_increasing() {
        // Higher RPM should always produce a lower-or-equal PWM (pump spins faster).
        let mut prev = u16::MAX;
        for rpm in (1600..=2500).step_by(5) {
            let pwm = water_block_pump_pwm(rpm);
            assert!(pwm <= prev, "rpm={rpm} pwm={pwm} prev={prev}");
            prev = pwm;
        }
    }

    // ---- water_block2_pump_pwm (Square) ----

    #[test]
    fn water_block2_clamps_below_min() {
        assert_eq!(water_block2_pump_pwm(1000), 1590);
    }

    #[test]
    fn water_block2_clamps_above_max() {
        // 4000 > 3200 → clamps to 3200 → 90 - 200*0.45 = 90 - 90 = 0
        assert_eq!(water_block2_pump_pwm(4000), 0);
    }

    #[test]
    fn water_block2_at_segment_boundaries() {
        assert_eq!(water_block2_pump_pwm(1600), 1590);
        // 1800: 1590 - 200*0.95 = 1590 - 190 = 1400
        assert_eq!(water_block2_pump_pwm(1800), 1400);
        // 2000: next branch: 1400 - 200 = 1200
        assert_eq!(water_block2_pump_pwm(2000), 1200);
        // 3200: 90 - 200*0.45 = 0
        assert_eq!(water_block2_pump_pwm(3200), 0);
    }

    #[test]
    fn water_block2_monotonic_non_increasing() {
        let mut prev = u16::MAX;
        for rpm in (1600..=3200).step_by(5) {
            let pwm = water_block2_pump_pwm(rpm);
            assert!(pwm <= prev, "rpm={rpm} pwm={pwm} prev={prev}");
            prev = pwm;
        }
    }

    // ---- pct_to_rpm ----

    #[test]
    fn pct_to_rpm_circle_endpoints() {
        assert_eq!(pct_to_rpm(PumpVariant::Circle, 0), 1600);
        assert_eq!(pct_to_rpm(PumpVariant::Circle, 100), 2500);
        assert_eq!(pct_to_rpm(PumpVariant::Circle, 50), 2050);
    }

    #[test]
    fn pct_to_rpm_square_endpoints() {
        assert_eq!(pct_to_rpm(PumpVariant::Square, 0), 1600);
        assert_eq!(pct_to_rpm(PumpVariant::Square, 100), 3200);
        assert_eq!(pct_to_rpm(PumpVariant::Square, 50), 2400);
    }

    #[test]
    fn pct_to_rpm_clamps_over_100() {
        assert_eq!(pct_to_rpm(PumpVariant::Circle, 200), 2500);
    }

    // ---- build_aio_param ----

    #[test]
    fn aio_param_has_defaults_and_pump_bytes() {
        let ap = build_aio_param(0x0841); // 2113 decimal
        assert_eq!(ap[7], 1, "pump_enable must be set");
        assert_eq!(ap[25], 80, "lcd_brightness default");
        assert_eq!(ap[26], 1, "fixed byte");
        assert_eq!(ap[28], 0x08);
        assert_eq!(ap[29], 0x41);
        // All other bytes zero.
        for (i, b) in ap.iter().enumerate() {
            if matches!(i, 7 | 25 | 26 | 28 | 29) {
                continue;
            }
            assert_eq!(*b, 0, "byte {i} should be zero");
        }
    }
}
