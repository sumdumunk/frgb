use crate::GroupId;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceId([u8; 6]);

impl DeviceId {
    pub const ZERO: Self = Self([0; 6]);
    pub const BROADCAST: Self = Self([0xff; 6]);

    pub fn from_hex(hex: &str) -> Result<Self, String> {
        let hex = hex.trim().replace(' ', "");
        if hex.len() != 12 {
            return Err(format!("expected 12 hex chars, got {}", hex.len()));
        }
        let bytes = (0..6)
            .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("invalid hex: {e}"))?;
        let mut arr = [0u8; 6];
        arr.copy_from_slice(&bytes);
        Ok(Self(arr))
    }

    /// Synthesize a DeviceId from USB VID/PID (for non-RF devices without hardware MACs).
    pub fn from_vid_pid(vid: u16, pid: u16) -> Self {
        Self([
            (vid >> 8) as u8,
            (vid & 0xFF) as u8,
            (pid >> 8) as u8,
            (pid & 0xFF) as u8,
            0,
            0,
        ])
    }

    /// Set the index byte (byte 5) to distinguish multiple devices with same VID:PID.
    pub fn set_index(&mut self, idx: u8) {
        self.0[5] = idx;
    }

    pub fn as_bytes(&self) -> &[u8; 6] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl From<[u8; 6]> for DeviceId {
    fn from(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceType {
    // Fans
    SlWireless,
    SlLcdWireless,
    SlInfWireless,
    ClWireless,
    TlWireless,
    TlLcdWireless,
    SlV2,
    P28,
    Rl120,
    // AIO
    HydroShift,
    HydroShiftII,
    GalahadIiLcd,
    GalahadIiTrinity,
    V150,
    // Cables
    StrimerWireless,
    StrimerPlusV2,
    // Accessories
    SideArgbKit,
    WaterBlock,
    WaterBlock2,
    Led88,
    Ga2,
    Lc217,
    // External
    OpenRgb,
    // Motherboard RGB
    Aura,
    // Unrecognized device type
    Unknown,
}

impl DeviceType {
    /// Protocol identifier byte used in packet headers.
    /// Multiple device types intentionally share protocol IDs when they
    /// use the same packet format (e.g., all AIO devices share ID 90).
    pub fn protocol_id(&self) -> u8 {
        match self {
            Self::SlWireless | Self::SlLcdWireless | Self::SlV2 => 20,
            Self::SlInfWireless => 36,
            Self::ClWireless => 41,
            Self::TlWireless | Self::TlLcdWireless => 28,
            Self::Rl120 => 40,
            Self::V150 => 66,
            Self::Led88 => 88,
            Self::Ga2 | Self::GalahadIiLcd | Self::GalahadIiTrinity => 90,
            Self::StrimerWireless | Self::StrimerPlusV2 => 1,
            Self::WaterBlock => 10,
            Self::WaterBlock2 => 11,
            Self::Lc217 => 65,
            Self::OpenRgb => 99,
            Self::HydroShift | Self::HydroShiftII => 90,
            Self::Aura => 0,
            Self::P28 | Self::SideArgbKit | Self::Unknown => 0,
        }
    }

    pub fn addressable_leds(&self) -> u16 {
        match self {
            Self::SlWireless | Self::SlLcdWireless | Self::SlV2 => 21,
            Self::SlInfWireless => 58,
            Self::ClWireless => 48,
            Self::TlWireless | Self::TlLcdWireless => 26,
            Self::Led88 | Self::V150 => 88,
            Self::SideArgbKit => 10,
            // Pump head RGB ring: 24 base + 24 per fan
            Self::WaterBlock | Self::WaterBlock2 => 48,
            // HydroShift pump head: 24 LEDs (single ring, no per-fan extension)
            Self::HydroShift | Self::HydroShiftII => 24,
            _ => 0,
        }
    }

    pub fn has_lcd(&self) -> bool {
        matches!(
            self,
            Self::SlLcdWireless
                | Self::TlLcdWireless
                | Self::HydroShift
                | Self::HydroShiftII
                | Self::GalahadIiLcd
                | Self::Ga2
        )
    }

    pub fn is_fan(&self) -> bool {
        matches!(
            self,
            Self::SlWireless
                | Self::SlLcdWireless
                | Self::SlInfWireless
                | Self::ClWireless
                | Self::TlWireless
                | Self::TlLcdWireless
                | Self::SlV2
                | Self::P28
                | Self::Rl120
        )
    }

    pub fn is_motherboard(&self) -> bool {
        matches!(self, Self::Aura)
    }

    pub fn is_aio(&self) -> bool {
        matches!(
            self,
            Self::HydroShift | Self::HydroShiftII | Self::GalahadIiLcd | Self::GalahadIiTrinity | Self::V150
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FanRole {
    Intake,
    Exhaust,
    Pump,
    Custom(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BladeType {
    Standard,
    Reverse,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FanGroup {
    pub id: GroupId,
    pub name: String,
    pub device_type: DeviceType,
    pub fan_count: u8,
    pub role: FanRole,
    pub blade: BladeType,
    pub cfm_max: Option<f32>,
    pub fan_ids: Vec<DeviceId>,
    pub tx_ref: DeviceId,
    /// Raw fans_type bytes from device record — per-slot device type encoding.
    pub fans_type: [u8; 4],
    /// Per-slot RPM readings from last discovery (0 = empty slot).
    pub fans_rpm: [u16; 4],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupWearInfo {
    pub group: GroupId,
    pub running_seconds: u64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnboundDevice {
    pub mac: DeviceId,
    pub master: DeviceId,
    pub group: GroupId,
    pub fan_count: u8,
    pub device_type: DeviceType,
    pub fans_type: [u8; 4],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_from_bytes() {
        let id = DeviceId::from([0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]);
        assert_eq!(id.as_bytes(), &[0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]);
    }

    #[test]
    fn device_id_from_hex_string() {
        let id = DeviceId::from_hex("c8b4ef6232e1").unwrap();
        assert_eq!(id.as_bytes(), &[0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]);
    }

    #[test]
    fn device_id_to_hex_string() {
        let id = DeviceId::from([0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]);
        assert_eq!(id.to_hex(), "c8b4ef6232e1");
    }

    #[test]
    fn device_id_invalid_hex() {
        assert!(DeviceId::from_hex("zzzz").is_err());
        assert!(DeviceId::from_hex("c8b4ef").is_err()); // too short
    }

    #[test]
    fn device_type_protocol_id() {
        assert_eq!(DeviceType::SlWireless.protocol_id(), 20);
        assert_eq!(DeviceType::ClWireless.protocol_id(), 41);
        assert_eq!(DeviceType::TlWireless.protocol_id(), 28);
        assert_eq!(DeviceType::Led88.protocol_id(), 88);
    }

    #[test]
    fn device_type_led_count() {
        assert_eq!(DeviceType::SlWireless.addressable_leds(), 21);
        assert_eq!(DeviceType::ClWireless.addressable_leds(), 48);
        assert_eq!(DeviceType::Led88.addressable_leds(), 88);
    }

    // --- has_lcd ---

    #[test]
    fn has_lcd_true_cases() {
        assert!(DeviceType::SlLcdWireless.has_lcd());
        assert!(DeviceType::HydroShift.has_lcd());
        assert!(DeviceType::Ga2.has_lcd());
        assert!(DeviceType::TlLcdWireless.has_lcd());
        assert!(DeviceType::HydroShiftII.has_lcd());
        assert!(DeviceType::GalahadIiLcd.has_lcd());
    }

    #[test]
    fn has_lcd_false_cases() {
        assert!(!DeviceType::SlWireless.has_lcd());
        assert!(!DeviceType::ClWireless.has_lcd());
        assert!(!DeviceType::StrimerWireless.has_lcd());
        assert!(!DeviceType::Led88.has_lcd());
        assert!(!DeviceType::WaterBlock.has_lcd());
        assert!(!DeviceType::OpenRgb.has_lcd());
    }

    // --- is_fan ---

    #[test]
    fn is_fan_true_cases() {
        assert!(DeviceType::SlWireless.is_fan());
        assert!(DeviceType::SlLcdWireless.is_fan());
        assert!(DeviceType::ClWireless.is_fan());
        assert!(DeviceType::TlWireless.is_fan());
        assert!(DeviceType::P28.is_fan());
        assert!(DeviceType::Rl120.is_fan());
        assert!(DeviceType::SlV2.is_fan());
    }

    #[test]
    fn is_fan_false_cases() {
        assert!(!DeviceType::HydroShift.is_fan());
        assert!(!DeviceType::StrimerWireless.is_fan());
        assert!(!DeviceType::Led88.is_fan());
        assert!(!DeviceType::WaterBlock.is_fan());
        assert!(!DeviceType::OpenRgb.is_fan());
        assert!(!DeviceType::V150.is_fan());
    }

    // --- is_aio ---

    #[test]
    fn is_aio_true_cases() {
        assert!(DeviceType::HydroShift.is_aio());
        assert!(DeviceType::HydroShiftII.is_aio());
        assert!(DeviceType::GalahadIiLcd.is_aio());
        assert!(DeviceType::GalahadIiTrinity.is_aio());
        assert!(DeviceType::V150.is_aio());
    }

    #[test]
    fn is_aio_false_cases() {
        assert!(!DeviceType::SlWireless.is_aio());
        assert!(!DeviceType::ClWireless.is_aio());
        assert!(!DeviceType::StrimerWireless.is_aio());
        assert!(!DeviceType::Led88.is_aio());
        assert!(!DeviceType::WaterBlock.is_aio());
    }

    // --- protocol_id extended ---

    #[test]
    fn protocol_id_extended() {
        assert_eq!(DeviceType::HydroShift.protocol_id(), 90);
        assert_eq!(DeviceType::StrimerWireless.protocol_id(), 1);
        assert_eq!(DeviceType::StrimerPlusV2.protocol_id(), 1);
        assert_eq!(DeviceType::WaterBlock.protocol_id(), 10);
        assert_eq!(DeviceType::WaterBlock2.protocol_id(), 11);
        assert_eq!(DeviceType::OpenRgb.protocol_id(), 99);
        assert_eq!(DeviceType::P28.protocol_id(), 0);
        assert_eq!(DeviceType::SideArgbKit.protocol_id(), 0);
        assert_eq!(DeviceType::Ga2.protocol_id(), 90);
        assert_eq!(DeviceType::GalahadIiLcd.protocol_id(), 90);
        assert_eq!(DeviceType::Lc217.protocol_id(), 65);
        assert_eq!(DeviceType::V150.protocol_id(), 66);
        assert_eq!(DeviceType::Rl120.protocol_id(), 40);
    }

    // --- addressable_leds extended ---

    #[test]
    fn addressable_leds_extended() {
        assert_eq!(DeviceType::SlInfWireless.addressable_leds(), 58);
        assert_eq!(DeviceType::TlWireless.addressable_leds(), 26);
        assert_eq!(DeviceType::TlLcdWireless.addressable_leds(), 26);
        assert_eq!(DeviceType::V150.addressable_leds(), 88);
        assert_eq!(DeviceType::SideArgbKit.addressable_leds(), 10);
        assert_eq!(DeviceType::StrimerWireless.addressable_leds(), 0);
        assert_eq!(DeviceType::HydroShift.addressable_leds(), 24);
        assert_eq!(DeviceType::WaterBlock.addressable_leds(), 48);
        assert_eq!(DeviceType::OpenRgb.addressable_leds(), 0);
        assert_eq!(DeviceType::SlV2.addressable_leds(), 21);
    }

    #[test]
    fn fan_group_serialization_roundtrip() {
        let group = FanGroup {
            id: GroupId::new(1),
            name: "Bottom Intake".into(),
            device_type: DeviceType::SlWireless,
            fan_count: 3,
            role: FanRole::Intake,
            blade: BladeType::Standard,
            cfm_max: Some(64.05),
            fan_ids: vec![DeviceId::from([0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1])],
            tx_ref: DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]),
            fans_type: [21, 21, 21, 0],
            fans_rpm: [1400, 1350, 1380, 0],
        };
        let json = serde_json::to_string(&group).unwrap();
        let deser: FanGroup = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.name, "Bottom Intake");
        assert_eq!(deser.fan_count, 3);
    }

    #[test]
    fn unbound_device_serialization_roundtrip() {
        let dev = UnboundDevice {
            mac: DeviceId::from([0xab, 0x1b, 0x1f, 0xe5, 0x66, 0xe1]),
            master: DeviceId::ZERO,
            group: GroupId::new(254),
            fan_count: 2,
            device_type: DeviceType::ClWireless,
            fans_type: [42, 41, 0, 0],
        };
        let json = serde_json::to_string(&dev).unwrap();
        let deser: UnboundDevice = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.mac, dev.mac);
        assert_eq!(deser.group, GroupId::new(254));
        assert_eq!(deser.fans_type, [42, 41, 0, 0]);
    }
}
