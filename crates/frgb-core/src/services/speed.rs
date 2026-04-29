//! Speed services — pure functions for RPM estimation and curve evaluation.

use frgb_model::spec::DeviceSpec;

/// Estimate speed percentage from RPM using device spec's max_rpm.
/// Returns None if the device has no max_rpm spec.
pub fn estimate_percent(rpm: u16, spec: &DeviceSpec) -> Option<u8> {
    let max = spec.max_rpm?;
    if max == 0 {
        return Some(0);
    }
    Some(((rpm as u32 * 100) / max as u32).min(100) as u8)
}

/// Estimate total CFM for a set of fans at given RPMs.
/// CFM scales linearly with RPM ratio (simplified model).
pub fn estimate_cfm(rpms: &[u16], spec: &DeviceSpec) -> Option<f32> {
    let max_rpm = spec.max_rpm? as f32;
    let cfm_per_fan = spec.cfm?;
    if max_rpm == 0.0 {
        return Some(0.0);
    }
    let total: f32 = rpms
        .iter()
        .filter(|&&rpm| rpm > 0)
        .map(|&rpm| cfm_per_fan * (rpm as f32 / max_rpm))
        .sum();
    Some(total)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::spec_loader::load_defaults;

    #[test]
    fn estimate_percent_sl_fan() {
        let reg = load_defaults();
        let spec = reg.lookup_fans_type(21).unwrap(); // SL-R, max_rpm=2000
        assert_eq!(estimate_percent(1000, spec), Some(50));
        assert_eq!(estimate_percent(2000, spec), Some(100));
        assert_eq!(estimate_percent(0, spec), Some(0));
    }

    #[test]
    fn estimate_percent_over_max() {
        let reg = load_defaults();
        let spec = reg.lookup_fans_type(21).unwrap();
        // Above max should clamp to 100%
        assert_eq!(estimate_percent(3000, spec), Some(100));
    }

    #[test]
    fn estimate_percent_no_max_rpm() {
        let reg = load_defaults();
        let spec = reg.lookup_hwmon("k10temp").unwrap(); // sensor, no max_rpm
        assert_eq!(estimate_percent(1000, spec), None);
    }

    #[test]
    fn estimate_cfm_sl_fans() {
        let reg = load_defaults();
        let spec = reg.lookup_fans_type(21).unwrap(); // SL-R, cfm=64.05, max=2000
        let cfm = estimate_cfm(&[1000, 1000, 1000], spec).unwrap();
        let expected = 64.05 * 0.5 * 3.0; // 3 fans at 50% RPM
        assert!((cfm - expected).abs() < 0.1);
    }

    #[test]
    fn estimate_cfm_with_zero_rpm() {
        let reg = load_defaults();
        let spec = reg.lookup_fans_type(21).unwrap();
        let cfm = estimate_cfm(&[1000, 0, 1000], spec).unwrap();
        let expected = 64.05 * 0.5 * 2.0; // 2 fans at 50%
        assert!((cfm - expected).abs() < 0.1);
    }
}
