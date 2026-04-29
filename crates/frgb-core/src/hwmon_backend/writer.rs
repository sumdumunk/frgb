use crate::error::{CoreError, Result};
use crate::hwmon_backend::fs::HwmonFs;
use std::path::Path;

/// Scale a 0-100 percentage to a 0-255 PWM byte, clamping the input and
/// enforcing a per-channel minimum (spec §5.5).
pub fn pct_to_byte(pct: u8, min_pwm: u8) -> u8 {
    let pct = pct.min(100);
    // round(pct * 255 / 100)
    let byte = ((pct as u16 * 255 + 50) / 100) as u8;
    byte.max(min_pwm)
}

/// Implicit floor applied to pump-role channels when user hasn't set min_pwm.
/// 40% = 102 (rounded).
pub const fn pump_floor_byte() -> u8 {
    102
}

/// Write a PWM byte to `{chip}/pwm{idx}`.
pub fn set_pwm<F: HwmonFs + ?Sized>(fs: &F, chip: &Path, idx: u8, byte: u8) -> Result<()> {
    let path = chip.join(format!("pwm{idx}"));
    fs.write_str(&path, &byte.to_string())
        .map_err(|e| CoreError::Protocol(format!("hwmon write {}: {e}", path.display())))
}

/// Write a `pwmN_enable` value (1 = manual, 5 = Smart Fan IV).
pub fn set_enable<F: HwmonFs + ?Sized>(fs: &F, chip: &Path, idx: u8, value: u8) -> Result<()> {
    let path = chip.join(format!("pwm{idx}_enable"));
    fs.write_str(&path, &value.to_string())
        .map_err(|e| CoreError::Protocol(format!("hwmon write {}: {e}", path.display())))
}

/// Read current `pwmN_enable` value.
pub fn read_enable<F: HwmonFs + ?Sized>(fs: &F, chip: &Path, idx: u8) -> Result<u8> {
    let path = chip.join(format!("pwm{idx}_enable"));
    let s = fs
        .read_to_string(&path)
        .map_err(|e| CoreError::Protocol(format!("hwmon read {}: {e}", path.display())))?;
    s.trim()
        .parse::<u8>()
        .map_err(|e| CoreError::Protocol(format!("hwmon parse {}: {e}", path.display())))
}

/// Read current `pwmN` value (0-255).
pub fn read_pwm<F: HwmonFs + ?Sized>(fs: &F, chip: &Path, idx: u8) -> Result<u8> {
    let path = chip.join(format!("pwm{idx}"));
    let s = fs
        .read_to_string(&path)
        .map_err(|e| CoreError::Protocol(format!("hwmon read {}: {e}", path.display())))?;
    s.trim()
        .parse::<u8>()
        .map_err(|e| CoreError::Protocol(format!("hwmon parse {}: {e}", path.display())))
}

/// Read current `fanN_input` RPM.
pub fn read_fan_rpm<F: HwmonFs + ?Sized>(fs: &F, chip: &Path, idx: u8) -> Result<u16> {
    let path = chip.join(format!("fan{idx}_input"));
    let s = fs
        .read_to_string(&path)
        .map_err(|e| CoreError::Protocol(format!("hwmon read {}: {e}", path.display())))?;
    s.trim()
        .parse::<u16>()
        .map_err(|e| CoreError::Protocol(format!("hwmon parse {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pct_to_byte_clamps_and_scales() {
        // Boundary: 0 → 0
        assert_eq!(pct_to_byte(0, 0), 0);
        // Boundary: 100 → 255
        assert_eq!(pct_to_byte(100, 0), 255);
        // Scaling: 50 → 128 (round((50*255)/100))
        assert_eq!(pct_to_byte(50, 0), 128);
        // Out-of-range clamp: above 100 pins to 255
        assert_eq!(pct_to_byte(150, 0), 255);
    }

    #[test]
    fn pct_to_byte_respects_min_floor() {
        // min_pwm=100, request 10% (=26) → raised to 100
        assert_eq!(pct_to_byte(10, 100), 100);
        // min_pwm=100, request 50% (=128) → unchanged
        assert_eq!(pct_to_byte(50, 100), 128);
    }

    #[test]
    fn pct_to_byte_min_floor_above_100_caps_at_255() {
        assert_eq!(pct_to_byte(0, 255), 255);
    }

    #[test]
    fn pump_floor_byte_is_102() {
        // 40% of 255 ≈ 102 — spec §5.5 implicit pump floor.
        assert_eq!(pump_floor_byte(), 102);
    }

    #[test]
    fn set_pwm_writes_expected_path() {
        // FakeFs::write_str accepts any path — we're asserting on captured writes,
        // not seeded file state.
        let fs = crate::hwmon_backend::fs::tests_only::FakeFs::default();
        set_pwm(&fs, std::path::Path::new("/c"), 2, 128).unwrap();
        assert_eq!(fs.last_write("/c/pwm2"), Some("128".to_string()));
    }

    #[test]
    fn set_enable_writes_expected_path() {
        let fs = crate::hwmon_backend::fs::tests_only::FakeFs::default();
        set_enable(&fs, std::path::Path::new("/c"), 2, 1).unwrap();
        assert_eq!(fs.last_write("/c/pwm2_enable"), Some("1".to_string()));
    }

    #[test]
    fn read_enable_parses_int() {
        let fs = crate::hwmon_backend::fs::tests_only::FakeFs::with_pwm_file("/c/pwm2_enable", "5\n");
        assert_eq!(read_enable(&fs, std::path::Path::new("/c"), 2).unwrap(), 5);
    }

    #[test]
    fn read_pwm_parses_int() {
        let fs = crate::hwmon_backend::fs::tests_only::FakeFs::with_pwm_file("/c/pwm2", "128\n");
        assert_eq!(read_pwm(&fs, std::path::Path::new("/c"), 2).unwrap(), 128);
    }

    #[test]
    fn read_fan_rpm_parses_u16() {
        let fs = crate::hwmon_backend::fs::tests_only::FakeFs::with_pwm_file("/c/fan2_input", "1700\n");
        assert_eq!(read_fan_rpm(&fs, std::path::Path::new("/c"), 2).unwrap(), 1700);
    }
}
