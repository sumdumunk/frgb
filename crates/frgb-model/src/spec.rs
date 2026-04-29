use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::device::DeviceType;

// ---------------------------------------------------------------------------
// DeviceKind — what category of hardware is this?
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Fan,
    Pump,
    LedStrip,
    Sensor,
    Controller,
}

// ---------------------------------------------------------------------------
// Capabilities — bitflags for what a device can do
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Capabilities(u16);

impl Capabilities {
    pub const NONE: Self = Self(0);
    pub const RPM_READ: Self = Self(0x01);
    pub const SPEED_CTRL: Self = Self(0x02);
    pub const RGB: Self = Self(0x04);
    pub const LCD: Self = Self(0x08);
    pub const CFM: Self = Self(0x10);
    pub const TEMP_SENSOR: Self = Self(0x20);

    pub fn contains(self, flag: Self) -> bool {
        self.0 & flag.0 == flag.0
    }

    pub fn from_strings(names: &[String]) -> Result<Self, String> {
        let mut bits = 0u16;
        for name in names {
            bits |= match name.as_str() {
                "rpm_read" => Self::RPM_READ.0,
                "speed_ctrl" => Self::SPEED_CTRL.0,
                "rgb" => Self::RGB.0,
                "lcd" => Self::LCD.0,
                "cfm" => Self::CFM.0,
                "temp_sensor" => Self::TEMP_SENSOR.0,
                _ => return Err(format!("unknown capability: '{name}'")),
            };
        }
        Ok(Self(bits))
    }
}

impl std::ops::BitOr for Capabilities {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for Capabilities {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

// ---------------------------------------------------------------------------
// SpecKey — polymorphic lookup key for device specs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SpecKey {
    /// Lian Li RF: fans_type byte from discovery
    FansType(u8),
    /// hwmon: chip name ("k10temp", "corsairpsu")
    Hwmon(String),
    /// USB: VID/PID
    UsbId(u16, u16),
}

// ---------------------------------------------------------------------------
// DeviceSpec — static hardware characteristics from data file
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceSpec {
    pub key: SpecKey,
    pub kind: DeviceKind,
    pub name: String,
    pub capabilities: Capabilities,

    // Physical characteristics (optional — only relevant fields populated)
    pub size_mm: Option<u16>,
    pub max_rpm: Option<u16>,
    pub cfm: Option<f32>,
    pub noise_dba: Option<f32>,

    // LED layout (for RGB-capable devices)
    pub physical_leds: u8,
    pub virtual_leds: u8,
    pub inner_leds: u8,
    pub is_reverse: bool,
    pub has_lcd: bool,

    // Protocol dispatch (Lian Li-specific, None for other backends)
    pub device_type: Option<DeviceType>,

    /// Non-zero dev_type bytes that map to this spec (for non-fan devices).
    /// Fans use fans_type[0] for identification; Strimers, WaterBlocks, etc.
    /// use dev_type instead. Multiple dev_type values can map to one spec
    /// (e.g., Strimer dev_type 1-9 all map to StrimerWireless).
    pub dev_types: Vec<u8>,
}

impl DeviceSpec {
    /// Outer LED count, derived from virtual total minus inner.
    pub fn outer_leds(&self) -> u8 {
        self.virtual_leds.saturating_sub(self.inner_leds)
    }
}

// ---------------------------------------------------------------------------
// SpecRegistry — lookup table for all known device specs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SpecRegistry {
    by_fans_type: HashMap<u8, DeviceSpec>,
    by_dev_type: HashMap<u8, DeviceSpec>,
    by_usb_id: HashMap<(u16, u16), DeviceSpec>,
    by_hwmon: HashMap<String, DeviceSpec>,
}

impl SpecRegistry {
    pub fn new() -> Self {
        Self {
            by_fans_type: HashMap::new(),
            by_dev_type: HashMap::new(),
            by_usb_id: HashMap::new(),
            by_hwmon: HashMap::new(),
        }
    }

    /// Insert a spec, indexed by its key type and dev_types.
    pub fn insert(&mut self, spec: DeviceSpec) {
        // Index by dev_type values (for non-fan device identification)
        for &dt in &spec.dev_types {
            self.by_dev_type.insert(dt, spec.clone());
        }
        match &spec.key {
            SpecKey::FansType(ft) => {
                self.by_fans_type.insert(*ft, spec);
            }
            SpecKey::UsbId(vid, pid) => {
                self.by_usb_id.insert((*vid, *pid), spec);
            }
            SpecKey::Hwmon(name) => {
                self.by_hwmon.insert(name.clone(), spec);
            }
        }
    }

    pub fn lookup_fans_type(&self, ft: u8) -> Option<&DeviceSpec> {
        self.by_fans_type.get(&ft)
    }

    /// Look up a non-fan device by its dev_type byte from discovery.
    pub fn lookup_dev_type(&self, dt: u8) -> Option<&DeviceSpec> {
        self.by_dev_type.get(&dt)
    }

    pub fn lookup_usb(&self, vid: u16, pid: u16) -> Option<&DeviceSpec> {
        self.by_usb_id.get(&(vid, pid))
    }

    pub fn lookup_hwmon(&self, name: &str) -> Option<&DeviceSpec> {
        self.by_hwmon.get(name)
    }

    /// All specs in the registry (for iteration/display).
    pub fn all_specs(&self) -> Vec<&DeviceSpec> {
        let mut specs: Vec<&DeviceSpec> = Vec::new();
        specs.extend(self.by_fans_type.values());
        specs.extend(self.by_usb_id.values());
        specs.extend(self.by_hwmon.values());
        specs
    }

    /// Number of unique specs (primary keys only, not dev_type aliases).
    pub fn len(&self) -> usize {
        self.by_fans_type.len() + self.by_usb_id.len() + self.by_hwmon.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Merge another registry into this one (user overrides on top of defaults).
    pub fn merge(&mut self, other: SpecRegistry) {
        for (k, v) in other.by_fans_type {
            self.by_fans_type.insert(k, v);
        }
        for (k, v) in other.by_dev_type {
            self.by_dev_type.insert(k, v);
        }
        for (k, v) in other.by_usb_id {
            self.by_usb_id.insert(k, v);
        }
        for (k, v) in other.by_hwmon {
            self.by_hwmon.insert(k, v);
        }
    }
}

impl Default for SpecRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_fan_spec(ft: u8) -> DeviceSpec {
        DeviceSpec {
            key: SpecKey::FansType(ft),
            kind: DeviceKind::Fan,
            name: "Test Fan".into(),
            capabilities: Capabilities::RPM_READ | Capabilities::SPEED_CTRL | Capabilities::RGB,
            size_mm: Some(120),
            max_rpm: Some(2000),
            cfm: Some(64.05),
            noise_dba: Some(28.0),
            physical_leds: 21,
            virtual_leds: 40,
            inner_leds: 8,
            is_reverse: false,
            has_lcd: false,
            device_type: Some(DeviceType::SlWireless),
            dev_types: vec![],
        }
    }

    #[test]
    fn capabilities_contains() {
        let caps = Capabilities::RPM_READ | Capabilities::RGB;
        assert!(caps.contains(Capabilities::RPM_READ));
        assert!(caps.contains(Capabilities::RGB));
        assert!(!caps.contains(Capabilities::LCD));
    }

    #[test]
    fn capabilities_from_strings() {
        let caps =
            Capabilities::from_strings(&["rpm_read".into(), "speed_ctrl".into(), "rgb".into(), "cfm".into()]).unwrap();
        assert!(caps.contains(Capabilities::RPM_READ));
        assert!(caps.contains(Capabilities::SPEED_CTRL));
        assert!(caps.contains(Capabilities::RGB));
        assert!(caps.contains(Capabilities::CFM));
        assert!(!caps.contains(Capabilities::LCD));
    }

    #[test]
    fn capabilities_unknown_string_errors() {
        let result = Capabilities::from_strings(&["unknown_cap".into()]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown capability"));
    }

    #[test]
    fn outer_leds_derived() {
        let spec = sample_fan_spec(21);
        assert_eq!(spec.outer_leds(), 32); // 40 - 8
    }

    #[test]
    fn registry_lookup_fans_type() {
        let mut reg = SpecRegistry::new();
        reg.insert(sample_fan_spec(21));
        assert!(reg.lookup_fans_type(21).is_some());
        assert!(reg.lookup_fans_type(42).is_none());
    }

    #[test]
    fn registry_lookup_usb() {
        let mut reg = SpecRegistry::new();
        reg.insert(DeviceSpec {
            key: SpecKey::UsbId(0x1CBE, 0xA021),
            kind: DeviceKind::Controller,
            name: "HydroShift II".into(),
            capabilities: Capabilities::LCD,
            size_mm: None,
            max_rpm: None,
            cfm: None,
            noise_dba: None,
            physical_leds: 0,
            virtual_leds: 0,
            inner_leds: 0,
            is_reverse: false,
            has_lcd: true,
            device_type: None,
            dev_types: vec![],
        });
        assert!(reg.lookup_usb(0x1CBE, 0xA021).is_some());
        assert!(reg.lookup_usb(0x1CBE, 0x0000).is_none());
    }

    #[test]
    fn registry_lookup_hwmon() {
        let mut reg = SpecRegistry::new();
        reg.insert(DeviceSpec {
            key: SpecKey::Hwmon("k10temp".into()),
            kind: DeviceKind::Sensor,
            name: "AMD CPU".into(),
            capabilities: Capabilities::TEMP_SENSOR,
            size_mm: None,
            max_rpm: None,
            cfm: None,
            noise_dba: None,
            physical_leds: 0,
            virtual_leds: 0,
            inner_leds: 0,
            is_reverse: false,
            has_lcd: false,
            device_type: None,
            dev_types: vec![],
        });
        assert!(reg.lookup_hwmon("k10temp").is_some());
        assert!(reg.lookup_hwmon("amdgpu").is_none());
    }

    #[test]
    fn registry_merge_overrides() {
        let mut base = SpecRegistry::new();
        base.insert(sample_fan_spec(21));

        let mut override_reg = SpecRegistry::new();
        let mut updated = sample_fan_spec(21);
        updated.max_rpm = Some(2200);
        override_reg.insert(updated);

        base.merge(override_reg);
        assert_eq!(base.lookup_fans_type(21).unwrap().max_rpm, Some(2200));
    }

    #[test]
    fn registry_len() {
        let mut reg = SpecRegistry::new();
        assert!(reg.is_empty());
        reg.insert(sample_fan_spec(21));
        reg.insert(sample_fan_spec(42));
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn spec_key_equality() {
        assert_eq!(SpecKey::FansType(21), SpecKey::FansType(21));
        assert_ne!(SpecKey::FansType(21), SpecKey::FansType(42));
        assert_ne!(SpecKey::FansType(21), SpecKey::Hwmon("k10temp".into()));
    }
}
