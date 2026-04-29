use crate::device::*;
use crate::ipc::Target;
use crate::lcd::*;
use crate::rgb::*;
use crate::sensor::*;
use crate::speed::*;
use crate::GroupId;
use crate::SpeedPercent;
use crate::ValidatedName;
use serde::{Deserialize, Serialize};

// Supporting types referenced in IPC

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedCurve {
    pub name: ValidatedName,
    pub curve: FanCurve,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupStatus {
    pub group: FanGroup,
    pub rpms: Vec<u16>,
    pub speed: SpeedMode,
    pub rgb: RgbMode,
    pub lcd: Option<LcdConfig>,
    pub lcd_count: u8,
    pub mb_sync: bool,
    pub online: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupDeviceIds {
    pub group_id: GroupId,
    pub fan_ids: Vec<DeviceId>,
    pub tx_ref: DeviceId,
    pub fan_type: DeviceType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirmwareInfo {
    pub tx_version: String,
    pub rx_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlertEvent {
    pub sensor: Sensor,
    pub value: f32,
    pub threshold: u8,
}

// EffectSequence replaced by show::EffectCycle + show::Sequence.
// Re-export for config compatibility.
pub use crate::show::{EffectCycle, Sequence};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertConfig {
    pub temp_alerts: Vec<TempAlert>,
    pub fan_stall_detect: bool,
    pub device_disconnect: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TempAlert {
    pub sensor: Sensor,
    pub threshold: u8,
    pub action: AlertAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertAction {
    Notify,
    SwitchProfile(String),
    SetSpeed(SpeedPercent),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupConfig {
    pub id: GroupId,
    pub name: String,
    pub device_type: DeviceType,
    pub fan_count: u8,
    pub role: FanRole,
    pub blade: BladeType,
    pub cfm_max: Option<f32>,
    pub excluded: bool,
    pub speed: SpeedMode,
    pub rgb: RgbMode,
    pub lcd: Option<LcdConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonConfig {
    pub poll_interval_ms: u32,
    pub autostart: bool,
    pub minimize_to_tray: bool,
    pub close_to_tray: bool,
    pub notifications: bool,
    pub default_profile: Option<String>,
    pub default_curve: Option<String>,
    pub motherboard_min_pwm: u8,
    #[serde(default = "default_openrgb_port")]
    pub openrgb_server_port: u16,
    #[serde(default)]
    pub openrgb_server_enabled: bool,
}

fn default_openrgb_port() -> u16 {
    6743
}

fn default_config_version() -> u32 {
    1
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: 2000,
            autostart: false,
            minimize_to_tray: true,
            close_to_tray: true,
            notifications: true,
            default_profile: None,
            default_curve: Some("balanced".into()),
            motherboard_min_pwm: 50,
            openrgb_server_port: default_openrgb_port(),
            openrgb_server_enabled: false,
        }
    }
}

// Keyframe types now in show module.
pub use crate::show::{Blend, KeyFrame, KeyframeEffect, Playback, Scene};

/// Named per-LED color preset for save/load across daemon restarts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedPreset {
    pub name: ValidatedName,
    pub group_device_type: DeviceType,
    pub fan_count: u8,
    pub assignments: Vec<FanLedAssignment>,
}

// ---------------------------------------------------------------------------
// Scheduling (section 3.15)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleEntry {
    pub hour: u8,
    pub minute: u8,
    pub days: Vec<Weekday>,
    pub action: ScheduleAction,
}

impl ScheduleEntry {
    pub fn validate(&self) -> Result<(), String> {
        if self.hour >= 24 {
            return Err(format!("hour must be 0-23, got {}", self.hour));
        }
        if self.minute >= 60 {
            return Err(format!("minute must be 0-59, got {}", self.minute));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Weekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScheduleAction {
    SwitchProfile(String),
    SetSpeed { target: Target, percent: SpeedPercent },
    ApplyCurve { target: Target, curve: String },
}

// ---------------------------------------------------------------------------
// App profiles
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppProfile {
    pub app_name: String,
    pub profile: String,
}

// ---------------------------------------------------------------------------
// Sync config (section 3.17)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncConfig {
    pub enabled: bool,
    pub include_lianli: bool,
    pub include_mobo_rgb: bool,
    pub include_openrgb: bool,
    /// Optional role filter — when non-empty, only groups with a matching role participate.
    #[serde(default)]
    pub include_roles: Vec<FanRole>,
    /// Optional device-type filter — when non-empty, only matching device types participate.
    #[serde(default)]
    pub include_device_types: Vec<DeviceType>,
    /// Explicit group exclusions — these groups never participate in sync regardless of other filters.
    #[serde(default)]
    pub exclude_groups: Vec<GroupId>,
}

// ---------------------------------------------------------------------------
// Power config (section 3.16)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PowerConfig {
    pub on_battery: Option<String>,
    pub on_ac: Option<String>,
}

// ---------------------------------------------------------------------------
// V150 config (section 3.13)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct V150Config {
    pub front_speed: SpeedMode,
    pub rear_speed: SpeedMode,
    pub mb_fan_sync: bool,
    pub mb_light_sync: bool,
}

// ---------------------------------------------------------------------------
// Strimer config (section 3.13)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrimerConfig {
    pub variant: StrimerVariant,
    pub channels: u8,
    pub led_count: u16,
    pub rgb: RgbMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StrimerVariant {
    Wireless24Pin,
    Wireless3x8Pin,
    Wireless2x8Pin,
    WirelessCpu2x8Pin,
    Wireless16Pin12,
    Wireless16Pin8,
    PlusV2_24Pin,
    PlusV2_3x8Pin,
    PlusV2_2x8Pin,
    PlusV2_12Plus4Pin,
}

// ---------------------------------------------------------------------------
// Hotkey config (section 3.19)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyConfig {
    pub enabled: bool,
    pub bindings: Vec<HotkeyBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyBinding {
    pub keys: String,
    pub action: HotkeyAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HotkeyAction {
    SetSpeedAll(u8),
    SwitchProfile(String),
    EmergencyFullSpeed,
    ToggleRgb,
    DimBrightness,
    CycleProfiles,
}

// ---------------------------------------------------------------------------
// OSD config (section 3.19)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OsdConfig {
    pub enabled: bool,
    pub position: OsdPosition,
    pub opacity: f32,
    pub mode: OsdMode,
    pub show_temps: bool,
    pub show_rpm: bool,
    pub show_load: bool,
}

impl OsdConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !self.opacity.is_finite() || self.opacity < 0.0 || self.opacity > 1.0 {
            return Err(format!("opacity must be 0.0-1.0, got {}", self.opacity));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OsdPosition {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OsdMode {
    Compact,
    Detailed,
}

// ---------------------------------------------------------------------------
// Profile (section 3.14)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: ValidatedName,
    pub groups: Vec<GroupSnapshot>,
    pub effect_cycle: Option<EffectCycle>,
    pub sequences: Vec<Sequence>,
}

/// Saved state for one device group within a profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupSnapshot {
    pub group_id: GroupId,
    pub scene: Scene,
}

// ---------------------------------------------------------------------------
// Wireless config (section 3.18)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WirelessConfig {
    pub channel: u8,
}

impl Default for WirelessConfig {
    fn default() -> Self {
        Self { channel: 0x08 }
    }
}

// ---------------------------------------------------------------------------
// AURA config (section 3.19)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuraChannelConfig {
    pub name: String,
    pub leds: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuraConfig {
    pub group_base: u8,
    pub channels: Vec<AuraChannelConfig>,
}

impl Default for AuraConfig {
    fn default() -> Self {
        Self {
            group_base: 50,
            channels: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuraHwEffect {
    Off,
    Static,
    Breathing,
    Flashing,
    SpectrumCycle,
    Rainbow,
    SpectrumCycleBreathing,
    ChaseFade,
    SpectrumCycleChaseFade,
    Chase,
    SpectrumCycleChase,
    SpectrumCycleWave,
    ChaseRainbowPulse,
    RandomFlicker,
}

impl AuraHwEffect {
    pub fn to_byte(self) -> u8 {
        match self {
            Self::Off => 0x00,
            Self::Static => 0x01,
            Self::Breathing => 0x02,
            Self::Flashing => 0x03,
            Self::SpectrumCycle => 0x04,
            Self::Rainbow => 0x05,
            Self::SpectrumCycleBreathing => 0x06,
            Self::ChaseFade => 0x07,
            Self::SpectrumCycleChaseFade => 0x08,
            Self::Chase => 0x09,
            Self::SpectrumCycleChase => 0x0A,
            Self::SpectrumCycleWave => 0x0B,
            Self::ChaseRainbowPulse => 0x0C,
            Self::RandomFlicker => 0x0D,
        }
    }
}

// ---------------------------------------------------------------------------
// Hwmon (motherboard Super I/O) config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HwmonChannelRole {
    Intake,
    Exhaust,
    Pump,
    Fan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum HwmonCurveExecution {
    #[default]
    Auto,
    Hardware,
    Software,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HwmonChannelConfig {
    pub pwm: u8,
    pub name: String,
    pub role: HwmonChannelRole,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub min_pwm: u8,
    #[serde(default)]
    pub curve_execution: HwmonCurveExecution,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HwmonConfig {
    pub group_base: u8,
    #[serde(default)]
    pub state_file: Option<String>,
    #[serde(default)]
    pub channels: Vec<HwmonChannelConfig>,
}

impl Default for HwmonConfig {
    fn default() -> Self {
        Self {
            group_base: 60,
            state_file: None,
            channels: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Wear tracking
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WearEntry {
    pub group_id: GroupId,
    pub running_seconds: u64,
}

// ---------------------------------------------------------------------------
// Top-level Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_config_version")]
    pub version: u32,
    pub groups: Vec<GroupConfig>,
    pub device_ids: Vec<GroupDeviceIds>,
    pub effect_cycle: Option<EffectCycle>,
    pub sequences: Vec<Sequence>,
    pub profiles: Vec<Profile>,
    pub active_profile: Option<String>,
    pub saved_curves: Vec<NamedCurve>,
    pub saved_effects: Vec<KeyframeEffect>,
    #[serde(default)]
    pub saved_led_presets: Vec<LedPreset>,
    #[serde(default)]
    pub lcd_templates: Vec<crate::lcd::LcdTemplate>,
    pub daemon: DaemonConfig,
    pub alerts: Option<AlertConfig>,
    pub schedules: Vec<ScheduleEntry>,
    pub app_profiles: Vec<AppProfile>,
    pub sync: Option<SyncConfig>,
    pub power: Option<PowerConfig>,
    pub wireless: WirelessConfig,
    pub hotkeys: Option<HotkeyConfig>,
    pub osd: Option<OsdConfig>,
    pub v150: Option<V150Config>,
    pub strimers: Vec<StrimerConfig>,
    pub sensor_calibration: SensorCalibration,
    pub temp_rgb: Vec<TempRgbConfig>,
    pub merge_order: Option<Vec<u8>>,
    pub aura: AuraConfig,
    #[serde(default)]
    pub hwmon: HwmonConfig,
    #[serde(default)]
    pub wear_stats: Vec<WearEntry>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            groups: Vec::new(),
            device_ids: Vec::new(),
            effect_cycle: None,
            sequences: Vec::new(),
            profiles: Vec::new(),
            active_profile: None,
            saved_curves: Vec::new(),
            saved_effects: Vec::new(),
            saved_led_presets: Vec::new(),
            lcd_templates: Vec::new(),
            daemon: DaemonConfig::default(),
            alerts: None,
            schedules: Vec::new(),
            app_profiles: Vec::new(),
            sync: None,
            power: None,
            wireless: WirelessConfig::default(),
            hotkeys: None,
            osd: None,
            v150: None,
            strimers: Vec::new(),
            sensor_calibration: SensorCalibration::default(),
            temp_rgb: Vec::new(),
            merge_order: None,
            aura: AuraConfig::default(),
            hwmon: HwmonConfig::default(),
            wear_stats: Vec::new(),
        }
    }
}

impl Config {
    /// Validate and clamp config values. Returns warnings for any values that
    /// were out of range (clamped) or otherwise suspicious. The config is still
    /// usable after validation — no hard errors.
    pub fn validate(&mut self) -> Vec<String> {
        let mut warnings = Vec::new();

        // Clamp poll_interval_ms to [100, 60000]
        if self.daemon.poll_interval_ms < 100 {
            warnings.push(format!(
                "poll_interval_ms {} too low, clamped to 100",
                self.daemon.poll_interval_ms
            ));
            self.daemon.poll_interval_ms = 100;
        }
        if self.daemon.poll_interval_ms > 60000 {
            warnings.push(format!(
                "poll_interval_ms {} too high, clamped to 60000",
                self.daemon.poll_interval_ms
            ));
            self.daemon.poll_interval_ms = 60000;
        }

        // openrgb_server_port must be non-zero
        if self.daemon.openrgb_server_port == 0 {
            warnings.push("openrgb_server_port is 0, reset to default 6743".into());
            self.daemon.openrgb_server_port = default_openrgb_port();
        }

        // Clamp motherboard_min_pwm to [0, 100]
        if self.daemon.motherboard_min_pwm > 100 {
            warnings.push(format!(
                "motherboard_min_pwm {} exceeds 100%, clamped to 100",
                self.daemon.motherboard_min_pwm
            ));
            self.daemon.motherboard_min_pwm = 100;
        }

        // Temp alert thresholds: flag anything above 150°C
        if let Some(ref alerts) = self.alerts {
            for (i, alert) in alerts.temp_alerts.iter().enumerate() {
                if alert.threshold > 150 {
                    warnings.push(format!("temp_alert[{i}].threshold {} exceeds 150°C", alert.threshold));
                }
            }
        }

        // Validate schedule entries
        for (i, entry) in self.schedules.iter().enumerate() {
            if let Err(e) = entry.validate() {
                warnings.push(format!("schedule[{i}]: {e}"));
            }
        }

        // Validate OSD config
        if let Some(osd) = &self.osd {
            if let Err(e) = osd.validate() {
                warnings.push(format!("osd: {e}"));
            }
        }

        // Validate named curves
        for curve in &self.saved_curves {
            if let Err(e) = curve.curve.validate() {
                warnings.push(format!("curve '{}': {e}", curve.name));
            }
        }

        // Validate group configs
        for gc in &self.groups {
            if let Err(e) = gc.speed.validate() {
                warnings.push(format!("group '{}' speed: {e}", gc.name));
            }
        }

        warnings
    }

    /// Insert or update a profile by name. If a profile with the same name
    /// already exists, it is replaced; otherwise the new profile is appended.
    ///
    /// This is the single in-memory entry point for profile mutation —
    /// daemon code should always go through this rather than reading/writing
    /// the config file directly. The on-disk state is updated by the daemon's
    /// `ConfigCache::flush()` after this method returns.
    pub fn upsert_profile(&mut self, profile: Profile) {
        if let Some(existing) = self.profiles.iter_mut().find(|p| p.name == profile.name) {
            *existing = profile;
        } else {
            self.profiles.push(profile);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_entry_validate_ok() {
        let entry = ScheduleEntry {
            hour: 8,
            minute: 30,
            days: vec![Weekday::Mon],
            action: ScheduleAction::SwitchProfile("night".into()),
        };
        assert!(entry.validate().is_ok());
    }

    #[test]
    fn schedule_entry_validate_bad_hour() {
        let entry = ScheduleEntry {
            hour: 24,
            minute: 0,
            days: vec![],
            action: ScheduleAction::SwitchProfile("x".into()),
        };
        let err = entry.validate().unwrap_err();
        assert!(err.contains("hour must be 0-23"));
    }

    #[test]
    fn schedule_entry_validate_bad_minute() {
        let entry = ScheduleEntry {
            hour: 0,
            minute: 60,
            days: vec![],
            action: ScheduleAction::SwitchProfile("x".into()),
        };
        let err = entry.validate().unwrap_err();
        assert!(err.contains("minute must be 0-59"));
    }

    #[test]
    fn osd_config_validate_ok() {
        let osd = OsdConfig {
            enabled: true,
            position: OsdPosition::TopRight,
            opacity: 0.75,
            mode: OsdMode::Compact,
            show_temps: true,
            show_rpm: false,
            show_load: true,
        };
        assert!(osd.validate().is_ok());
    }

    #[test]
    fn osd_config_validate_opacity_out_of_range() {
        let osd = OsdConfig {
            enabled: true,
            position: OsdPosition::TopLeft,
            opacity: 1.5,
            mode: OsdMode::Compact,
            show_temps: false,
            show_rpm: false,
            show_load: false,
        };
        let err = osd.validate().unwrap_err();
        assert!(err.contains("opacity must be 0.0-1.0"));
    }

    #[test]
    fn osd_config_validate_opacity_negative() {
        let osd = OsdConfig {
            enabled: false,
            position: OsdPosition::BottomLeft,
            opacity: -0.1,
            mode: OsdMode::Detailed,
            show_temps: false,
            show_rpm: false,
            show_load: false,
        };
        assert!(osd.validate().is_err());
    }

    #[test]
    fn daemon_config_default_values() {
        let d = DaemonConfig::default();
        assert_eq!(d.poll_interval_ms, 2000);
        assert!(!d.autostart);
        assert!(d.minimize_to_tray);
        assert!(d.close_to_tray);
        assert!(d.notifications);
        assert_eq!(d.default_profile, None);
        assert_eq!(d.default_curve, Some("balanced".into()));
        assert_eq!(d.motherboard_min_pwm, 50);
        assert_eq!(d.openrgb_server_port, 6743);
        assert!(!d.openrgb_server_enabled);
    }

    #[test]
    fn wireless_config_default_channel() {
        let w = WirelessConfig::default();
        assert_eq!(w.channel, 0x08);
    }

    #[test]
    fn config_default_empty_collections() {
        let c = Config::default();
        assert!(c.groups.is_empty());
        assert!(c.device_ids.is_empty());
        assert!(c.profiles.is_empty());
        assert!(c.saved_curves.is_empty());
        assert!(c.saved_effects.is_empty());
        assert!(c.schedules.is_empty());
        assert!(c.app_profiles.is_empty());
        assert!(c.strimers.is_empty());
        assert!(c.temp_rgb.is_empty());
        assert_eq!(c.active_profile, None);
        assert_eq!(c.alerts, None);
        assert_eq!(c.sync, None);
        assert_eq!(c.power, None);
        assert_eq!(c.hotkeys, None);
        assert_eq!(c.osd, None);
        assert_eq!(c.v150, None);
    }

    #[test]
    fn config_default_no_effect_cycle() {
        let c = Config::default();
        assert!(c.effect_cycle.is_none());
        assert!(c.sequences.is_empty());
    }

    #[test]
    fn config_default_wireless_channel() {
        let c = Config::default();
        assert_eq!(c.wireless.channel, 0x08);
    }

    #[test]
    fn config_default_daemon_embedded() {
        let c = Config::default();
        assert_eq!(c.daemon.poll_interval_ms, 2000);
        assert_eq!(c.daemon.default_curve, Some("balanced".into()));
    }

    #[test]
    fn config_serialization_roundtrip() {
        let original = Config::default();
        let json = serde_json::to_string(&original).expect("serialize failed");
        let restored: Config = serde_json::from_str(&json).expect("deserialize failed");
        assert_eq!(original, restored);
    }

    #[test]
    fn config_default_validates() {
        let mut config = Config::default();
        assert!(config.validate().is_empty());
    }

    #[test]
    fn config_serialization_with_groups() {
        use crate::device::{BladeType, DeviceType, FanRole};
        use crate::rgb::RgbMode;
        use crate::speed::SpeedMode;

        let mut c = Config::default();
        c.groups.push(GroupConfig {
            id: GroupId::new(1),
            name: "Front Fans".into(),
            device_type: DeviceType::SlWireless,
            fan_count: 3,
            role: FanRole::Intake,
            blade: BladeType::Standard,
            cfm_max: None,
            excluded: false,
            speed: SpeedMode::Manual(SpeedPercent::new(50)),
            rgb: RgbMode::Off,
            lcd: None,
        });
        let json = serde_json::to_string(&c).expect("serialize failed");
        let restored: Config = serde_json::from_str(&json).expect("deserialize failed");
        assert_eq!(restored.groups.len(), 1);
        assert_eq!(restored.groups[0].name, "Front Fans");
        assert_eq!(restored.groups[0].fan_count, 3);
    }

    #[test]
    fn config_version_roundtrip() {
        let cfg = Config::default();
        assert_eq!(cfg.version, 1);
        let json = serde_json::to_string(&cfg).unwrap();
        let deser: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.version, 1);
    }

    #[test]
    fn config_version_defaults_when_missing() {
        // Serialize a default config, then strip the "version" key to simulate
        // loading a config saved before the version field existed.
        let cfg = Config::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let stripped = json.replace(r#""version":1,"#, "");
        assert!(!stripped.contains("version"));
        let deser: Config = serde_json::from_str(&stripped).unwrap();
        assert_eq!(deser.version, 1);
    }

    #[test]
    fn validate_clamps_poll_interval_low() {
        let mut cfg = Config::default();
        cfg.daemon.poll_interval_ms = 0;
        let warnings = cfg.validate();
        assert!(!warnings.is_empty());
        assert_eq!(cfg.daemon.poll_interval_ms, 100);
    }

    #[test]
    fn validate_clamps_poll_interval_high() {
        let mut cfg = Config::default();
        cfg.daemon.poll_interval_ms = 120_000;
        let warnings = cfg.validate();
        assert!(!warnings.is_empty());
        assert_eq!(cfg.daemon.poll_interval_ms, 60000);
    }

    #[test]
    fn validate_accepts_normal_values() {
        let mut cfg = Config::default();
        cfg.daemon.poll_interval_ms = 2000;
        let warnings = cfg.validate();
        assert!(warnings.is_empty());
    }

    #[test]
    fn validate_resets_zero_openrgb_port() {
        let mut cfg = Config::default();
        cfg.daemon.openrgb_server_port = 0;
        let warnings = cfg.validate();
        assert!(!warnings.is_empty());
        assert_eq!(cfg.daemon.openrgb_server_port, 6743);
    }

    /// Full roundtrip: Config with groups, saved curves, wear entries, and profiles.
    #[test]
    fn config_full_roundtrip_with_all_newtypes() {
        use crate::device::{BladeType, DeviceType, FanRole};
        use crate::rgb::RgbMode;
        use crate::sensor::Sensor;
        use crate::speed::{CurvePoint, FanCurve, Interpolation, SpeedMode};

        let mut cfg = Config::default();

        // GroupConfig with GroupId newtype
        cfg.groups.push(GroupConfig {
            id: GroupId::new(1),
            name: "Top Exhaust".into(),
            device_type: DeviceType::SlWireless,
            fan_count: 3,
            role: FanRole::Exhaust,
            blade: BladeType::Standard,
            cfm_max: Some(62.5),
            excluded: false,
            speed: SpeedMode::Manual(SpeedPercent::new(45)),
            rgb: RgbMode::Off,
            lcd: None,
        });

        // Saved curve with SpeedPercent and Temperature newtypes
        cfg.saved_curves.push(NamedCurve {
            name: ValidatedName::new("balanced").unwrap(),
            curve: FanCurve {
                points: vec![
                    CurvePoint {
                        temp: crate::Temperature::new(30),
                        speed: SpeedPercent::new(25),
                    },
                    CurvePoint {
                        temp: crate::Temperature::new(50),
                        speed: SpeedPercent::new(50),
                    },
                    CurvePoint {
                        temp: crate::Temperature::new(80),
                        speed: SpeedPercent::new(100),
                    },
                ],
                sensor: Sensor::Cpu,
                interpolation: Interpolation::Linear,
                min_speed: SpeedPercent::new(20),
                stop_below: None,
                ramp_rate: None,
            },
        });

        // WearEntry with GroupId
        cfg.wear_stats.push(WearEntry {
            group_id: GroupId::new(1),
            running_seconds: 86400,
        });

        // Profile with ValidatedName
        cfg.profiles.push(Profile {
            name: ValidatedName::new("gaming").unwrap(),
            groups: vec![GroupSnapshot {
                group_id: GroupId::new(1),
                scene: crate::show::Scene {
                    speed: Some(SpeedMode::Manual(SpeedPercent::new(80))),
                    rgb: RgbMode::Off,
                    lcd: None,
                },
            }],
            effect_cycle: None,
            sequences: vec![],
        });

        let json = serde_json::to_string_pretty(&cfg).expect("serialize failed");
        let restored: Config = serde_json::from_str(&json).expect("deserialize failed");

        assert_eq!(restored.groups.len(), 1);
        assert_eq!(restored.groups[0].id, GroupId::new(1));
        assert_eq!(restored.groups[0].name, "Top Exhaust");
        assert_eq!(restored.groups[0].speed, SpeedMode::Manual(SpeedPercent::new(45)));

        assert_eq!(restored.saved_curves.len(), 1);
        assert_eq!(restored.saved_curves[0].curve.points.len(), 3);
        assert_eq!(
            restored.saved_curves[0].curve.points[0].temp,
            crate::Temperature::new(30)
        );
        assert_eq!(restored.saved_curves[0].curve.points[0].speed, SpeedPercent::new(25));

        assert_eq!(restored.wear_stats.len(), 1);
        assert_eq!(restored.wear_stats[0].group_id, GroupId::new(1));
        assert_eq!(restored.wear_stats[0].running_seconds, 86400);

        assert_eq!(restored.profiles.len(), 1);
        assert_eq!(restored.profiles[0].name, ValidatedName::new("gaming").unwrap());
        assert_eq!(restored.profiles[0].groups[0].group_id, GroupId::new(1));
    }

    #[test]
    fn hwmon_config_default_matches_spec() {
        let cfg = HwmonConfig::default();
        assert_eq!(cfg.group_base, 60);
        assert!(cfg.channels.is_empty());
        assert!(cfg.state_file.is_none());
    }

    #[test]
    fn hwmon_channel_role_serde_round_trip() {
        let roles = [
            HwmonChannelRole::Intake,
            HwmonChannelRole::Exhaust,
            HwmonChannelRole::Pump,
            HwmonChannelRole::Fan,
        ];
        for role in roles {
            let json = serde_json::to_string(&role).unwrap();
            let back: HwmonChannelRole = serde_json::from_str(&json).unwrap();
            assert_eq!(back, role);
        }
    }

    #[test]
    fn hwmon_config_serde_round_trip() {
        let cfg = HwmonConfig {
            group_base: 60,
            state_file: Some("/tmp/foo.json".into()),
            channels: vec![
                HwmonChannelConfig {
                    pwm: 2,
                    name: "Rear exhaust".into(),
                    role: HwmonChannelRole::Exhaust,
                    model: Some("Noctua NF-A14".into()),
                    min_pwm: 0,
                    curve_execution: HwmonCurveExecution::Auto,
                },
            ],
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: HwmonConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn config_default_includes_hwmon() {
        let cfg = Config::default();
        assert_eq!(cfg.hwmon.group_base, 60);
        assert!(cfg.hwmon.channels.is_empty());
    }

    #[test]
    fn upsert_profile_inserts_when_missing() {
        let mut config = Config::default();
        let name = ValidatedName::new("testprofile").unwrap();
        let profile = Profile {
            name: name.clone(),
            groups: vec![],
            effect_cycle: None,
            sequences: vec![],
        };
        config.upsert_profile(profile);
        assert_eq!(config.profiles.len(), 1);
        assert_eq!(config.profiles[0].name, name);
    }

    #[test]
    fn upsert_profile_updates_when_name_matches() {
        use crate::rgb::RgbMode;
        use crate::show::Scene;

        let mut config = Config::default();
        let name = ValidatedName::new("testprofile").unwrap();
        config.profiles.push(Profile {
            name: name.clone(),
            groups: vec![],
            effect_cycle: None,
            sequences: vec![],
        });
        let updated = Profile {
            name: name.clone(),
            groups: vec![GroupSnapshot {
                group_id: GroupId::new(1),
                scene: Scene {
                    rgb: RgbMode::Off,
                    speed: None,
                    lcd: None,
                },
            }],
            effect_cycle: None,
            sequences: vec![],
        };
        config.upsert_profile(updated);
        assert_eq!(config.profiles.len(), 1, "should update, not append");
        assert_eq!(config.profiles[0].groups.len(), 1);
    }
}
