use crate::device::DeviceType;
use crate::spec::{Capabilities, DeviceKind, DeviceSpec, SpecKey, SpecRegistry};

/// Embedded default devices.toml — compiled into the binary.
const EMBEDDED_TOML: &str = include_str!("../data/devices.toml");

/// Load the default spec registry from the embedded devices.toml.
pub fn load_defaults() -> SpecRegistry {
    parse_toml(EMBEDDED_TOML).expect("embedded devices.toml should be valid")
}

/// Load defaults, then merge user overrides from ~/.config/frgb/devices.toml if it exists.
/// Returns (registry, optional warning) — callers should log warnings if present.
pub fn load_with_overrides() -> SpecRegistry {
    let (registry, warning) = load_with_overrides_verbose();
    if let Some(w) = warning {
        eprintln!("warning: {w}");
    }
    registry
}

/// Load defaults + overrides, returning any warning about override failures.
pub fn load_with_overrides_verbose() -> (SpecRegistry, Option<String>) {
    let mut registry = load_defaults();
    if let Some(path) = user_override_path() {
        match std::fs::read_to_string(&path) {
            Ok(contents) => match parse_toml(&contents) {
                Ok(overrides) => registry.merge(overrides),
                Err(e) => return (registry, Some(format!("failed to parse {}: {e}", path.display()))),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // no override file — normal
            Err(e) => return (registry, Some(format!("failed to read {}: {e}", path.display()))),
        }
    }
    (registry, None)
}

fn user_override_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config/frgb/devices.toml"))
}

/// Parse a TOML string into a SpecRegistry.
fn parse_toml(toml_str: &str) -> Result<SpecRegistry, String> {
    let doc: TomlDoc = toml::from_str(toml_str).map_err(|e| format!("TOML parse error: {e}"))?;

    let mut registry = SpecRegistry::new();
    for raw in doc.device {
        let spec = raw_to_spec(raw)?;
        registry.insert(spec);
    }
    Ok(registry)
}

// ---------------------------------------------------------------------------
// TOML deserialization intermediary — flat structure matching the TOML format
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct TomlDoc {
    device: Vec<RawDevice>,
}

#[derive(serde::Deserialize)]
struct RawDevice {
    // Key fields (exactly one should be set)
    fans_type: Option<u8>,
    hwmon_name: Option<String>,
    usb_vid: Option<u16>,
    usb_pid: Option<u16>,

    // Common
    name: String,
    kind: String,
    #[serde(default)]
    capabilities: Vec<String>,

    // Physical (optional)
    #[serde(default)]
    size_mm: Option<u16>,
    #[serde(default)]
    max_rpm: Option<u16>,
    #[serde(default)]
    cfm: Option<f32>,
    #[serde(default)]
    noise_dba: Option<f32>,

    // LED layout
    #[serde(default)]
    physical_leds: u8,
    #[serde(default)]
    virtual_leds: u8,
    #[serde(default)]
    inner_leds: u8,
    #[serde(default)]
    is_reverse: bool,
    #[serde(default)]
    has_lcd: bool,

    // Protocol
    #[serde(default)]
    device_type: Option<String>,
    /// Non-zero dev_type bytes that identify this device in discovery.
    #[serde(default)]
    dev_types: Vec<u8>,
}

fn raw_to_spec(raw: RawDevice) -> Result<DeviceSpec, String> {
    let key = if let Some(ft) = raw.fans_type {
        SpecKey::FansType(ft)
    } else if let Some(ref name) = raw.hwmon_name {
        SpecKey::Hwmon(name.clone())
    } else if let (Some(vid), Some(pid)) = (raw.usb_vid, raw.usb_pid) {
        SpecKey::UsbId(vid, pid)
    } else {
        return Err(format!(
            "device '{}' has no key (fans_type, hwmon_name, or usb_vid/pid)",
            raw.name
        ));
    };

    let kind = match raw.kind.as_str() {
        "fan" => DeviceKind::Fan,
        "pump" => DeviceKind::Pump,
        "led_strip" => DeviceKind::LedStrip,
        "sensor" => DeviceKind::Sensor,
        "controller" => DeviceKind::Controller,
        other => return Err(format!("unknown device kind '{}' for '{}'", other, raw.name)),
    };

    let device_type = raw.device_type.as_deref().map(parse_device_type).transpose()?;
    let capabilities =
        Capabilities::from_strings(&raw.capabilities).map_err(|e| format!("{} in device '{}'", e, raw.name))?;

    Ok(DeviceSpec {
        key,
        kind,
        name: raw.name,
        capabilities,
        size_mm: raw.size_mm,
        max_rpm: raw.max_rpm,
        cfm: raw.cfm,
        noise_dba: raw.noise_dba,
        physical_leds: raw.physical_leds,
        virtual_leds: raw.virtual_leds,
        inner_leds: raw.inner_leds,
        is_reverse: raw.is_reverse,
        has_lcd: raw.has_lcd,
        device_type,
        dev_types: raw.dev_types,
    })
}

fn parse_device_type(s: &str) -> Result<DeviceType, String> {
    match s {
        "SlWireless" => Ok(DeviceType::SlWireless),
        "SlLcdWireless" => Ok(DeviceType::SlLcdWireless),
        "SlInfWireless" => Ok(DeviceType::SlInfWireless),
        "ClWireless" => Ok(DeviceType::ClWireless),
        "TlWireless" => Ok(DeviceType::TlWireless),
        "TlLcdWireless" => Ok(DeviceType::TlLcdWireless),
        "SlV2" => Ok(DeviceType::SlV2),
        "P28" => Ok(DeviceType::P28),
        "Rl120" => Ok(DeviceType::Rl120),
        "HydroShift" => Ok(DeviceType::HydroShift),
        "HydroShiftII" => Ok(DeviceType::HydroShiftII),
        "GalahadIiLcd" => Ok(DeviceType::GalahadIiLcd),
        "GalahadIiTrinity" => Ok(DeviceType::GalahadIiTrinity),
        "V150" => Ok(DeviceType::V150),
        "StrimerWireless" => Ok(DeviceType::StrimerWireless),
        "StrimerPlusV2" => Ok(DeviceType::StrimerPlusV2),
        "SideArgbKit" => Ok(DeviceType::SideArgbKit),
        "WaterBlock" => Ok(DeviceType::WaterBlock),
        "WaterBlock2" => Ok(DeviceType::WaterBlock2),
        "Led88" => Ok(DeviceType::Led88),
        "Ga2" => Ok(DeviceType::Ga2),
        "Lc217" => Ok(DeviceType::Lc217),
        "OpenRgb" => Ok(DeviceType::OpenRgb),
        other => Err(format!("unknown device_type '{other}'")),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_toml_parses() {
        let registry = load_defaults();
        assert!(!registry.is_empty(), "embedded devices.toml should produce specs");
    }

    #[test]
    fn embedded_has_sl_fan() {
        let reg = load_defaults();
        let spec = reg.lookup_fans_type(21).expect("SL-R (ft=21) should exist");
        assert_eq!(spec.name, "SL-R");
        assert_eq!(spec.kind, DeviceKind::Fan);
        assert_eq!(spec.virtual_leds, 40);
        assert_eq!(spec.inner_leds, 8);
        assert!(spec.is_reverse);
        assert_eq!(spec.device_type, Some(DeviceType::SlWireless));
        assert!(spec.capabilities.contains(Capabilities::RGB));
        assert!(spec.capabilities.contains(Capabilities::RPM_READ));
    }

    #[test]
    fn embedded_has_cl_fan() {
        let reg = load_defaults();
        let spec = reg.lookup_fans_type(42).expect("CL-R (ft=42) should exist");
        assert_eq!(spec.name, "CL-R");
        assert_eq!(spec.virtual_leds, 24);
        assert_eq!(spec.inner_leds, 8);
        assert!(spec.is_reverse);
        assert_eq!(spec.device_type, Some(DeviceType::ClWireless));
    }

    #[test]
    fn embedded_has_tl_fan() {
        let reg = load_defaults();
        let spec = reg.lookup_fans_type(28).expect("TL (ft=28) should exist");
        assert_eq!(spec.virtual_leds, 26);
        assert_eq!(spec.device_type, Some(DeviceType::TlWireless));
    }

    #[test]
    fn embedded_has_sl_inf() {
        let reg = load_defaults();
        let spec = reg.lookup_fans_type(36).expect("SL-INF (ft=36) should exist");
        assert_eq!(spec.virtual_leds, 44);
        assert_eq!(spec.device_type, Some(DeviceType::SlInfWireless));
    }

    #[test]
    fn embedded_has_hwmon_sensors() {
        let reg = load_defaults();
        assert!(reg.lookup_hwmon("k10temp").is_some());
        assert!(reg.lookup_hwmon("amdgpu").is_some());
    }

    #[test]
    fn embedded_has_usb_devices() {
        let reg = load_defaults();
        assert!(reg.lookup_usb(0x1CBE, 0xA021).is_some());
        assert!(reg.lookup_usb(0x0B05, 0x1AA6).is_some());
    }

    #[test]
    fn embedded_has_dev_type_devices() {
        let reg = load_defaults();
        // Strimer (dev_type 1-9)
        for dt in 1..=9 {
            let spec = reg
                .lookup_dev_type(dt)
                .unwrap_or_else(|| panic!("dev_type {dt} missing"));
            assert_eq!(spec.device_type, Some(DeviceType::StrimerWireless));
        }
        // WaterBlock (dev_type 10)
        let wb = reg.lookup_dev_type(10).expect("dev_type 10 missing");
        assert_eq!(wb.device_type, Some(DeviceType::WaterBlock));
        assert_eq!(wb.name, "WaterBlock");
        // WaterBlock 2 (dev_type 11)
        assert_eq!(
            reg.lookup_dev_type(11).unwrap().device_type,
            Some(DeviceType::WaterBlock2)
        );
        // LC217 (dev_type 65)
        assert_eq!(reg.lookup_dev_type(65).unwrap().device_type, Some(DeviceType::Lc217));
        // V150 (dev_type 66)
        assert_eq!(reg.lookup_dev_type(66).unwrap().device_type, Some(DeviceType::V150));
        // Unknown dev_type
        assert!(reg.lookup_dev_type(99).is_none());
    }

    #[test]
    fn all_fan_types_covered() {
        let reg = load_defaults();
        // Verify all fans_type values from the current match arms exist
        for ft in 20..=42 {
            assert!(
                reg.lookup_fans_type(ft).is_some(),
                "fans_type {} missing from devices.toml",
                ft
            );
        }
        // Extended ranges
        for ft in 43..=58 {
            assert!(
                reg.lookup_fans_type(ft).is_some(),
                "fans_type {} missing from devices.toml",
                ft
            );
        }
    }

    #[test]
    fn reverse_flags_match_existing_code() {
        let reg = load_defaults();
        // Verify reverse flags match fans_type_is_reverse() from controller.rs
        let expected_reverse = [
            21, 23, 25, 27, 29, 31, 33, 35, 37, 39, 42, 43, 45, 47, 49, 51, 53, 55, 57,
        ];
        for ft in 20..=58 {
            if let Some(spec) = reg.lookup_fans_type(ft) {
                let expected = expected_reverse.contains(&ft);
                assert_eq!(
                    spec.is_reverse, expected,
                    "fans_type {} reverse flag mismatch: spec={}, expected={}",
                    ft, spec.is_reverse, expected
                );
            }
        }
    }

    #[test]
    fn parse_minimal_toml() {
        let toml = r#"
[[device]]
fans_type = 99
name = "Test"
kind = "fan"
capabilities = ["rgb"]
"#;
        let reg = parse_toml(toml).unwrap();
        let spec = reg.lookup_fans_type(99).unwrap();
        assert_eq!(spec.name, "Test");
        assert!(spec.capabilities.contains(Capabilities::RGB));
        assert_eq!(spec.physical_leds, 0); // default
    }

    #[test]
    fn parse_hwmon_device() {
        let toml = r#"
[[device]]
hwmon_name = "test_sensor"
name = "Test Sensor"
kind = "sensor"
capabilities = ["temp_sensor"]
"#;
        let reg = parse_toml(toml).unwrap();
        let spec = reg.lookup_hwmon("test_sensor").unwrap();
        assert_eq!(spec.kind, DeviceKind::Sensor);
    }

    #[test]
    fn parse_usb_device() {
        let toml = r#"
[[device]]
usb_vid = 0xABCD
usb_pid = 0x1234
name = "Test USB"
kind = "controller"
capabilities = ["lcd"]
"#;
        let reg = parse_toml(toml).unwrap();
        let spec = reg.lookup_usb(0xABCD, 0x1234).unwrap();
        assert_eq!(spec.kind, DeviceKind::Controller);
    }

    /// Verify the 140mm variant mapping for fans_type values.
    /// The fans_type encoding uses a repeating 8-value pattern within each
    /// family block: [120, 120-R, 140, 140-R, 120-LCD, 120-LCD-R, 140-LCD, 140-LCD-R].
    /// SL block: 20-26 (only 7 values, 27 starts TL).
    /// TL block: 27-35 (27 = TL-LCD-R 140, the 8th slot of the SL pattern,
    ///           assigned to TL family by the boundary iType < 27).
    #[test]
    fn fans_type_140mm_and_lcd_match_cs_reference() {
        let reg = load_defaults();

        // fans_type 22: SL 140mm, not reverse, no LCD
        let s22 = reg.lookup_fans_type(22).expect("ft 22");
        assert_eq!(s22.size_mm, Some(140), "ft 22 size");
        assert!(!s22.is_reverse, "ft 22 reverse");
        assert!(!s22.has_lcd, "ft 22 lcd");
        assert_eq!(s22.device_type, Some(DeviceType::SlWireless), "ft 22 device_type");

        // fans_type 23: SL 140mm, reverse, no LCD — NOT an LCD variant
        let s23 = reg.lookup_fans_type(23).expect("ft 23");
        assert_eq!(s23.size_mm, Some(140), "ft 23 size");
        assert!(s23.is_reverse, "ft 23 reverse");
        assert!(!s23.has_lcd, "ft 23 lcd");
        assert_eq!(s23.device_type, Some(DeviceType::SlWireless), "ft 23 device_type");

        // fans_type 26: SL-LCD 140mm
        let s26 = reg.lookup_fans_type(26).expect("ft 26");
        assert_eq!(s26.size_mm, Some(140), "ft 26 size");
        assert!(!s26.is_reverse, "ft 26 reverse");
        assert!(s26.has_lcd, "ft 26 lcd");
        assert_eq!(s26.device_type, Some(DeviceType::SlLcdWireless), "ft 26 device_type");

        // fans_type 27: TL-LCD-R 140mm — first TL fans_type value but has LCD
        let s27 = reg.lookup_fans_type(27).expect("ft 27");
        assert_eq!(s27.size_mm, Some(140), "ft 27 size");
        assert!(s27.is_reverse, "ft 27 reverse");
        assert!(s27.has_lcd, "ft 27 lcd");
        assert_eq!(s27.device_type, Some(DeviceType::TlLcdWireless), "ft 27 device_type");

        // fans_type 30: TL 140mm
        let s30 = reg.lookup_fans_type(30).expect("ft 30");
        assert_eq!(s30.size_mm, Some(140), "ft 30 size");
        assert!(!s30.is_reverse, "ft 30 reverse");
        assert!(!s30.has_lcd, "ft 30 lcd");

        // fans_type 31: TL-R 140mm
        let s31 = reg.lookup_fans_type(31).expect("ft 31");
        assert_eq!(s31.size_mm, Some(140), "ft 31 size");
        assert!(s31.is_reverse, "ft 31 reverse");
        assert!(!s31.has_lcd, "ft 31 lcd");

        // fans_type 34: TL-LCD 140mm
        let s34 = reg.lookup_fans_type(34).expect("ft 34");
        assert_eq!(s34.size_mm, Some(140), "ft 34 size");
        assert!(!s34.is_reverse, "ft 34 reverse");
        assert!(s34.has_lcd, "ft 34 lcd");

        // fans_type 35: TL-LCD-R 140mm
        let s35 = reg.lookup_fans_type(35).expect("ft 35");
        assert_eq!(s35.size_mm, Some(140), "ft 35 size");
        assert!(s35.is_reverse, "ft 35 reverse");
        assert!(s35.has_lcd, "ft 35 lcd");
    }

    #[test]
    fn parse_invalid_kind_errors() {
        let toml = r#"
[[device]]
fans_type = 1
name = "Bad"
kind = "spaceship"
capabilities = []
"#;
        assert!(parse_toml(toml).is_err());
    }

    #[test]
    fn parse_no_key_errors() {
        let toml = r#"
[[device]]
name = "NoKey"
kind = "fan"
capabilities = []
"#;
        assert!(parse_toml(toml).is_err());
    }
}
