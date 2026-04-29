use crate::config::*;
use crate::device::*;
use crate::lcd::*;
use crate::rgb::*;
use crate::sensor::*;
use crate::speed::*;
use crate::GroupId;
use crate::SpeedPercent;
use crate::Temperature;
use crate::ValidatedName;
use serde::{Deserialize, Serialize};

/// Minimum protocol version the daemon will accept from clients.
pub const PROTOCOL_VERSION_MIN: u32 = 1;

/// Current (maximum) protocol version.
///
/// Bump on **breaking** changes to Request/Response/Event (renamed/removed/restructured
/// variants, changed field types). Purely additive variants do *not* require a bump:
/// serde rejects unknown variants, so an old client that never sends the new request
/// will never see the new response. New clients targeting old daemons must guard
/// new requests behind a Hello-negotiated version check.
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    All,
    Group(GroupId),
    Groups(Vec<GroupId>),
    Role(FanRole),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Topic {
    Rpm,
    Temperature,
    DeviceChange,
    Speed,
    Rgb,
    Profile,
    Alert,
    Power,
}

/// Point-in-time snapshot of stale-handle recovery counters across
/// the two recovery subsystems (USB/HID handles and hwmon sysfs chips).
///
/// Snapshots are not atomic across fields — readers should use these for
/// trend analysis (deltas across periodic samples), not for invariant
/// assertions on a single sample.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RecoveryCountersIpc {
    /// USB/HID reopen syscall called (any path: RF, transport stale-read,
    /// LCD/AURA reconnect, ENE recovery).
    pub usb_reopen_attempts: u64,
    /// USB/HID reopen syscall returned Ok.
    pub usb_reopen_successes: u64,
    /// USB/HID reopen syscall returned Err.
    /// Modulo snapshot atomicity: attempts == successes + failures.
    pub usb_reopen_failures: u64,
    /// Soft recovery (e.g. clear_halt_out) and the subsequent retry op
    /// both succeeded — a complete soft-recovery round-trip.
    pub usb_soft_recovery_successes: u64,
    pub hwmon_rescan_attempts: u64,
    pub hwmon_rescan_successes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    // Device
    /// Query current device and group status including RPM, speed mode, RGB state.
    /// Returns: `Response::DeviceStatus(Vec<GroupStatus>)`
    Status,
    /// Query device status with additional firmware, backend, and performance metrics.
    /// Returns: `Response::DeviceStatus(Vec<GroupStatus>)`
    StatusVerbose,
    /// Discover new unbound devices via RF link.
    /// Returns: `Response::Ok` | `Response::Error`
    Discover,
    /// Reset all devices in target group(s), clearing firmware state and profile data.
    /// Returns: `Response::Ok` | `Response::Error`
    Reset { target: Option<Target> },
    /// Flash group white LED for identification, restore after timeout.
    /// Returns: `Response::Ok` | `Response::Error`
    Indicate { group: GroupId, duration_secs: u8 },
    /// Set all groups to 0% manual speed (emergency stop).
    /// Returns: `Response::Ok` | `Response::Error`
    StopFans,
    // Binding
    /// Bind first unbound device to group with optional RF lock.
    /// Returns: `Response::Ok` | `Response::Error`
    Bind { group: GroupId, lock: bool },
    /// Unbind device in group from RF control.
    /// Returns: `Response::Ok` | `Response::Error`
    Unbind { group: GroupId },
    /// Enable/disable motherboard BIOS PWM control for group.
    /// Returns: `Response::Ok` | `Response::Error`
    SetMbSync { group: GroupId, enable: bool },
    /// Lock all RF devices to prevent rebinding.
    /// Returns: `Response::Ok` | `Response::Error`
    Lock,
    /// Unlock all RF devices to allow rebinding.
    /// Returns: `Response::Ok` | `Response::Error`
    Unlock,
    // Group
    /// List all groups with device info, names, and configuration.
    /// Returns: `Response::GroupList(Vec<FanGroup>)`
    ListGroups,
    /// Rename group for display.
    /// Returns: `Response::Ok` | `Response::Error`
    RenameGroup { group: GroupId, name: String },
    /// Assign role (Intake, Exhaust, etc.) to group.
    /// Returns: `Response::Ok` | `Response::Error`
    SetRole { group: GroupId, role: FanRole },
    /// Mark group as excluded from broadcast commands (SetSpeedAll, SetRgbAll, Sync).
    /// Returns: `Response::Ok` | `Response::Error`
    ExcludeGroup { group: GroupId },
    /// Unmark group as excluded, re-enabling broadcast commands.
    /// Returns: `Response::Ok` | `Response::Error`
    IncludeGroup { group: GroupId },
    /// Remove group from config and registry.
    /// Returns: `Response::Ok` | `Response::Error`
    ForgetGroup { group: GroupId },
    // Speed
    /// Set fan speed mode (Manual %, PWM, Curve) for single group.
    /// Returns: `Response::Ok` | `Response::Error`
    SetSpeed { group: GroupId, mode: SpeedMode },
    /// Set fan speed mode across multiple groups (skips MB sync groups).
    /// Returns: `Response::Ok` | `Response::Error`
    SetSpeedAll { target: Target, mode: SpeedMode },
    /// Set pump speed mode (Quiet, Normal, High, Custom).
    /// Returns: `Response::Ok` | `Response::Error`
    SetPumpMode { group: GroupId, mode: PumpMode },
    // RGB
    /// Set RGB mode (Off, Static, Effect, Breathing, etc.) for single group.
    /// Returns: `Response::Ok` | `Response::Error`
    SetRgb { group: GroupId, mode: RgbMode },
    /// Set RGB mode across multiple groups respecting excludes and roles.
    /// Returns: `Response::Ok` | `Response::Error`
    SetRgbAll { target: Target, mode: RgbMode },
    /// Update brightness on current RGB mode across multiple groups.
    /// Returns: `Response::Ok` | `Response::Error`
    SetBrightness { target: Target, level: crate::Brightness },
    /// Start effect cycle animation loop across groups.
    /// Returns: `Response::Ok` | `Response::Error`
    SetEffectCycle { cycle: EffectCycle },
    // LCD
    /// List connected LCD display devices with info and capabilities.
    /// Returns: `Response::LcdDevices(Vec<LcdDeviceInfo>)`
    ListLcdDevices,
    /// Configure LCD content, refresh interval, and display settings.
    /// Returns: `Response::Ok` | `Response::Error`
    SetLcd { lcd_index: u8, config: LcdConfig },
    /// List available LCD preset templates (built-in or user-saved).
    /// Returns: `Response::Presets(Vec<LcdPreset>)`
    ListPresets,
    // Curves
    /// Save temperature-based fan curve under name for reuse.
    /// Returns: `Response::Ok` | `Response::Error`
    SaveCurve { name: ValidatedName, curve: FanCurve },
    /// Delete named curve from config.
    /// Returns: `Response::Ok` | `Response::Error`
    DeleteCurve { name: String },
    /// List all saved fan curves.
    /// Returns: `Response::CurveList(Vec<NamedCurve>)`
    ListCurves,
    // Profiles
    /// List saved profile names.
    /// Returns: `Response::ProfileList(Vec<String>)`
    ListProfiles,
    /// Load and apply profile settings (speed, RGB) across all groups.
    /// Returns: `Response::Ok` | `Response::Error`
    SwitchProfile { name: String },
    /// Save current group settings (speed, RGB) as named profile.
    /// Returns: `Response::Ok` | `Response::Error`
    SaveProfile { name: ValidatedName },
    /// Delete named profile from config.
    /// Returns: `Response::Ok` | `Response::Error`
    DeleteProfile { name: String },
    // Sensors
    /// List available hwmon sensors (CPU temp, GPU temp, etc.) with current readings.
    /// Returns: `Response::SensorList(Vec<SensorInfo>)`
    ListSensors,
    /// Query single sensor's current reading.
    /// Returns: `Response::SensorReading { sensor: Sensor, value: f32 }`
    GetSensorReading { sensor: Sensor },
    // Config
    /// Export current config to JSON file (optionally compressed).
    /// Returns: `Response::Ok` | `Response::Error`
    ExportConfig { path: String, compress: bool },
    /// Import config from file, with merge or replace semantics.
    /// Returns: `Response::Ok` | `Response::Error`
    ImportConfig { path: String, merge: bool },
    /// Fetch daemon configuration (poll interval, backend settings, etc.).
    /// Returns: `Response::DaemonConfig(Box<DaemonConfig>)`
    GetDaemonConfig,
    // Subscriptions
    /// Signal intent to receive event updates (no-op, all events broadcast unconditionally).
    /// Returns: `Response::Ok`
    Subscribe { topics: Vec<Topic> },
    /// Unsubscribe from event stream.
    /// Returns: `Response::Ok`
    Unsubscribe,
    // Firmware (read-only)
    /// Query RF dongle firmware version and device MAC.
    /// Returns: `Response::FirmwareInfo(FirmwareInfo)`
    GetFirmwareInfo,

    // Device (missing)
    /// Poll for device status changes at interval.
    /// Returns: `Response::Ok` | `Response::Error`
    Watch { interval_ms: u32 },
    /// Enter interactive RF binding mode — daemon streams `Event::BindDiscovered` as unbound devices appear.
    /// Returns: `Response::Ok` | `Response::Error`
    EnterBindMode,
    /// Exit interactive RF binding mode — stops streaming bind discovery events.
    /// Returns: `Response::Ok` | `Response::Error`
    ExitBindMode,

    // Group
    /// Set hardware merge order for chained effect propagation (1-4 groups).
    /// Returns: `Response::Ok` | `Response::Error`
    ReorderGroups { order: Vec<u8> },

    // Sequence control
    /// List all saved animation sequences.
    /// Returns: `Response::SequenceList(Vec<Sequence>)`
    ListSequences,
    /// Save named animation sequence (keyframe steps with timing).
    /// Returns: `Response::Ok` | `Response::Error`
    SaveSequence { sequence: Sequence },
    /// Delete named sequence from config.
    /// Returns: `Response::Ok` | `Response::Error`
    DeleteSequence { name: String },
    /// Start playing sequence on target group(s).
    /// Returns: `Response::Ok` | `Response::Error`
    StartSequence { name: String, target: Option<Target> },
    /// Stop sequence playback on target group(s).
    /// Returns: `Response::Ok` | `Response::Error`
    StopSequence { target: Option<Target> },
    /// Stop all sequences on all groups.
    /// Returns: `Response::Ok` | `Response::Error`
    StopAllSequences,

    // Custom effects
    /// Save named keyframe animation effect.
    /// Returns: `Response::Ok` | `Response::Error`
    SaveEffect { effect: KeyframeEffect },
    /// Delete named effect from config.
    /// Returns: `Response::Ok` | `Response::Error`
    DeleteEffect { name: String },
    /// List all saved keyframe effects.
    /// Returns: `Response::EffectList(Vec<KeyframeEffect>)`
    ListEffects,

    // LED presets
    /// Save per-LED color preset (different color per LED in a group).
    /// Returns: `Response::Ok` | `Response::Error`
    SaveLedPreset { preset: LedPreset },
    /// Delete named LED preset from config.
    /// Returns: `Response::Ok` | `Response::Error`
    DeleteLedPreset { name: String },
    /// List all saved per-LED color presets.
    /// Returns: `Response::LedPresets(Vec<LedPreset>)`
    ListLedPresets,

    // LCD templates
    /// List all LCD widget templates (built-in or user-saved).
    /// Returns: `Response::LcdTemplates(Vec<LcdTemplate>)`
    ListLcdTemplates,
    /// Save LCD template with widget definitions.
    /// Returns: `Response::Ok` | `Response::Error`
    SaveLcdTemplate { template: crate::lcd::LcdTemplate },
    /// Delete LCD template by ID.
    /// Returns: `Response::Ok` | `Response::Error`
    DeleteLcdTemplate { id: String },

    // Profiles (missing)
    /// Duplicate profile with new name.
    /// Returns: `Response::Ok` | `Response::Error`
    CopyProfile { from: String, to: String },

    // Schedule
    /// List all scheduled automation entries (cron-like).
    /// Returns: `Response::ScheduleList(Vec<ScheduleEntry>)`
    ListSchedule,
    /// Add time-based automation entry (daily/weekly triggers).
    /// Returns: `Response::Ok` | `Response::Error`
    AddSchedule { entry: ScheduleEntry },
    /// Delete schedule entry by index.
    /// Returns: `Response::Ok` | `Response::Error`
    DeleteSchedule { index: usize },
    /// Clear all schedule entries.
    /// Returns: `Response::Ok` | `Response::Error`
    ClearSchedule,
    /// Set application profile bindings (launch events trigger profiles).
    /// Returns: `Response::Ok` | `Response::Error`
    SetAppProfiles { profiles: Vec<AppProfile> },

    // Sensors
    /// Update sensor calibration offsets (CPU, GPU temp adjustments).
    /// Returns: `Response::Ok` | `Response::Error`
    SetSensorCalibration { cal: SensorCalibration },

    // Motherboard
    /// Detect motherboard RGB backend (ASUS AURA, etc.).
    /// Returns: `Response::Ok` | `Response::Error`
    MoboDetect,
    /// Check if motherboard RGB backend is active.
    /// Returns: `Response::Ok` | `Response::Error`
    MoboStatus,
    /// Set motherboard fan PWM speed manually.
    /// Returns: `Response::Ok` | `Response::Error`
    MoboSetSpeed { channel: u8, percent: SpeedPercent },
    /// Enable BIOS auto fan control for motherboard channel.
    /// Returns: `Response::Ok` | `Response::Error`
    MoboAuto { channel: u8 },
    /// Query motherboard temperature sensors (use ListSensors instead).
    /// Returns: `Response::Error` (deferred)
    MoboTemps,

    // Sync
    /// Broadcast static color across all devices matching SyncConfig filters.
    /// Returns: `Response::Ok` | `Response::Error`
    Sync { color: Rgb, config: SyncConfig },
    /// Fetch current sync configuration (enabled backends, role/type filters).
    /// Returns: `Response::SyncConfig(SyncConfig)`
    GetSyncConfig,
    /// Persist sync configuration (which backends, roles, device types participate).
    /// Returns: `Response::Ok` | `Response::Error`
    SetSyncConfig { config: SyncConfig },

    // Config
    /// Reload config from disk, refresh curves and sequences in engine.
    /// Returns: `Response::Ok` | `Response::Error`
    ReloadConfig,
    /// Update daemon settings (poll interval, backend choices, etc.).
    /// Returns: `Response::Ok` | `Response::Error`
    SetDaemonConfig { config: DaemonConfig },
    /// Fetch alert configuration (temperature thresholds, stall detection).
    /// Returns: `Response::AlertConfig(AlertConfig)`
    GetAlertConfig,
    /// Update power-mode profile (AC vs battery fan behavior).
    /// Returns: `Response::Ok` | `Response::Error`
    SetPowerConfig { config: PowerConfig },
    /// Update alert thresholds and behaviors.
    /// Returns: `Response::Ok` | `Response::Error`
    SetAlertConfig { config: AlertConfig },

    // Handshake
    /// Protocol version negotiation for client/daemon compatibility check.
    /// Returns: `Response::Hello { protocol_version: u32 }` | `Response::Error`
    Hello { protocol_version: u32 },

    // AURA
    /// Apply motherboard RGB effect with color to AURA channel.
    /// Returns: `Response::Ok` | `Response::Error`
    SetAuraEffect {
        group: GroupId,
        effect: crate::config::AuraHwEffect,
        color: [u8; 3],
    },
    /// List available motherboard RGB channels (AURA groups).
    /// Returns: `Response::AuraChannels(Vec<AuraChannelInfo>)`
    ListAuraChannels,

    // LCD preview
    /// Render a template preview at specified resolution. Returns JPEG bytes.
    RenderTemplatePreview {
        template: crate::lcd::LcdTemplate,
        width: u32,
        height: u32,
    },

    // Wear monitoring
    /// Query cumulative running-time statistics per fan group.
    /// Returns: `Response::WearStats(Vec<GroupWearInfo>)`
    GetWearStats,

    // Curve analysis
    /// Analyse active fan curves against observed thermal headroom.
    /// Returns: `Response::CurveSuggestions(Vec<CurveSuggestion>)`
    GetCurveSuggestions,

    // Observability
    /// Query stale-handle recovery counters (USB/HID + hwmon) for observability.
    /// Returns: `Response::RecoveryCounters(RecoveryCountersIpc)`
    GetRecoveryCounters,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensor::Sensor;
    use crate::speed::SpeedMode;
    use crate::GroupId;
    use crate::SpeedPercent;

    #[test]
    fn request_status_roundtrip() {
        let req = Request::Status;
        let json = serde_json::to_string(&req).unwrap();
        let deser: Request = serde_json::from_str(&json).unwrap();
        assert!(matches!(deser, Request::Status));
    }

    #[test]
    fn request_set_speed_roundtrip() {
        let sp75 = SpeedPercent::new(75);
        let req = Request::SetSpeed {
            group: GroupId::new(2),
            mode: SpeedMode::Manual(sp75),
        };
        let json = serde_json::to_string(&req).unwrap();
        let deser: Request = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            deser,
            Request::SetSpeed {
                group,
                mode: SpeedMode::Manual(pct)
            } if group == GroupId::new(2) && pct == sp75
        ));
    }

    #[test]
    fn response_ok_roundtrip() {
        let resp = Response::Ok;
        let json = serde_json::to_string(&resp).unwrap();
        let deser: Response = serde_json::from_str(&json).unwrap();
        assert!(matches!(deser, Response::Ok));
    }

    #[test]
    fn response_error_roundtrip() {
        let resp = Response::Error("something went wrong".into());
        let json = serde_json::to_string(&resp).unwrap();
        let deser: Response = serde_json::from_str(&json).unwrap();
        if let Response::Error(msg) = deser {
            assert_eq!(msg, "something went wrong");
        } else {
            panic!("expected Response::Error");
        }
    }

    #[test]
    fn event_rpm_update_roundtrip() {
        let ev = Event::RpmUpdate {
            group: GroupId::new(1),
            rpms: vec![1200, 1350, 1100],
        };
        let json = serde_json::to_string(&ev).unwrap();
        let deser: Event = serde_json::from_str(&json).unwrap();
        if let Event::RpmUpdate { group, rpms } = deser {
            assert_eq!(group, GroupId::new(1));
            assert_eq!(rpms, vec![1200, 1350, 1100]);
        } else {
            panic!("expected Event::RpmUpdate");
        }
    }

    #[test]
    fn event_sensor_update_roundtrip() {
        let ev = Event::SensorUpdate {
            sensor: Sensor::Cpu,
            value: 62.5,
        };
        let json = serde_json::to_string(&ev).unwrap();
        let deser: Event = serde_json::from_str(&json).unwrap();
        if let Event::SensorUpdate { sensor, value } = deser {
            assert_eq!(sensor, Sensor::Cpu);
            assert!((value - 62.5).abs() < 1e-4);
        } else {
            panic!("expected Event::SensorUpdate");
        }
    }

    #[test]
    fn target_all_roundtrip() {
        let target = Target::All;
        let json = serde_json::to_string(&target).unwrap();
        let deser: Target = serde_json::from_str(&json).unwrap();
        assert!(matches!(deser, Target::All));
    }

    #[test]
    fn target_groups_roundtrip() {
        let target = Target::Groups(vec![GroupId::new(1), GroupId::new(2), GroupId::new(3)]);
        let json = serde_json::to_string(&target).unwrap();
        let deser: Target = serde_json::from_str(&json).unwrap();
        if let Target::Groups(ids) = deser {
            assert_eq!(ids, vec![GroupId::new(1), GroupId::new(2), GroupId::new(3)]);
        } else {
            panic!("expected Target::Groups");
        }
    }

    #[test]
    fn topic_roundtrip() {
        let topic = Topic::Temperature;
        let json = serde_json::to_string(&topic).unwrap();
        let deser: Topic = serde_json::from_str(&json).unwrap();
        assert!(matches!(deser, Topic::Temperature));
    }

    #[test]
    fn mb_sync_request_roundtrip() {
        let req = Request::SetMbSync {
            group: GroupId::new(2),
            enable: true,
        };
        let json = serde_json::to_string(&req).unwrap();
        let deser: Request = serde_json::from_str(&json).unwrap();
        assert!(matches!(deser, Request::SetMbSync { group, enable: true } if group == GroupId::new(2)));
    }

    #[test]
    fn lock_unlock_request_roundtrip() {
        for req in [Request::Lock, Request::Unlock] {
            let json = serde_json::to_string(&req).unwrap();
            let _deser: Request = serde_json::from_str(&json).unwrap();
        }
    }

    #[test]
    fn hello_request_roundtrip() {
        let req = Request::Hello {
            protocol_version: PROTOCOL_VERSION,
        };
        let json = serde_json::to_string(&req).unwrap();
        let deser: Request = serde_json::from_str(&json).unwrap();
        assert!(matches!(deser, Request::Hello { protocol_version } if protocol_version == PROTOCOL_VERSION));
    }

    #[test]
    fn hello_response_roundtrip() {
        let resp = Response::Hello {
            protocol_version: PROTOCOL_VERSION,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let deser: Response = serde_json::from_str(&json).unwrap();
        assert!(matches!(deser, Response::Hello { protocol_version } if protocol_version == PROTOCOL_VERSION));
    }

    #[test]
    fn topic_all_variants_roundtrip() {
        let topics = vec![
            Topic::Rpm,
            Topic::Temperature,
            Topic::DeviceChange,
            Topic::Speed,
            Topic::Rgb,
            Topic::Profile,
            Topic::Alert,
            Topic::Power,
        ];
        for topic in &topics {
            let json = serde_json::to_string(topic).unwrap();
            let deser: Topic = serde_json::from_str(&json).unwrap();
            assert_eq!(serde_json::to_string(&deser).unwrap(), json);
        }
    }

    // -----------------------------------------------------------------------
    // Event roundtrips — Phase 1-4 features
    // -----------------------------------------------------------------------

    #[test]
    fn event_curve_applied_roundtrip() {
        let ev = Event::CurveApplied {
            group: GroupId::new(1),
            speed: SpeedPercent::new(65),
            temp: crate::Temperature::new(55),
        };
        let json = serde_json::to_string(&ev).unwrap();
        let deser: Event = serde_json::from_str(&json).unwrap();
        if let Event::CurveApplied { group, speed, temp } = deser {
            assert_eq!(group, GroupId::new(1));
            assert_eq!(speed, SpeedPercent::new(65));
            assert_eq!(temp, crate::Temperature::new(55));
        } else {
            panic!("expected Event::CurveApplied, got {deser:?}");
        }
    }

    #[test]
    fn event_bind_discovered_roundtrip() {
        use crate::device::{DeviceId, DeviceType, UnboundDevice};
        let dev = UnboundDevice {
            mac: DeviceId::from([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]),
            master: DeviceId::from([0x11, 0x22, 0x33, 0x44, 0x55, 0x66]),
            group: GroupId::new(0),
            fan_count: 3,
            device_type: DeviceType::Unknown,
            fans_type: [21, 21, 21, 0],
        };
        let ev = Event::BindDiscovered(dev.clone());
        let json = serde_json::to_string(&ev).unwrap();
        let deser: Event = serde_json::from_str(&json).unwrap();
        if let Event::BindDiscovered(d) = deser {
            assert_eq!(d.mac, dev.mac);
            assert_eq!(d.fan_count, 3);
            assert_eq!(d.fans_type, [21, 21, 21, 0]);
        } else {
            panic!("expected Event::BindDiscovered, got {deser:?}");
        }
    }

    #[test]
    fn event_rpm_anomaly_roundtrip() {
        let ev = Event::RpmAnomaly {
            group: GroupId::new(2),
            message: "RPM dropped 25%".into(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        let deser: Event = serde_json::from_str(&json).unwrap();
        if let Event::RpmAnomaly { group, message } = deser {
            assert_eq!(group, GroupId::new(2));
            assert_eq!(message, "RPM dropped 25%");
        } else {
            panic!("expected Event::RpmAnomaly, got {deser:?}");
        }
    }

    // -----------------------------------------------------------------------
    // Response roundtrips — Phase 1-4 features
    // -----------------------------------------------------------------------

    #[test]
    fn response_wear_stats_roundtrip() {
        use crate::device::GroupWearInfo;
        let resp = Response::WearStats(vec![GroupWearInfo {
            group: GroupId::new(1),
            running_seconds: 7200,
            name: "SL Fan".into(),
        }]);
        let json = serde_json::to_string(&resp).unwrap();
        let deser: Response = serde_json::from_str(&json).unwrap();
        if let Response::WearStats(stats) = deser {
            assert_eq!(stats.len(), 1);
            assert_eq!(stats[0].group, GroupId::new(1));
            assert_eq!(stats[0].running_seconds, 7200);
            assert_eq!(stats[0].name, "SL Fan");
        } else {
            panic!("expected Response::WearStats, got {deser:?}");
        }
    }

    #[test]
    fn response_curve_suggestions_roundtrip() {
        use crate::speed::CurveSuggestion;
        let resp = Response::CurveSuggestions(vec![CurveSuggestion {
            group: GroupId::new(1),
            curve_name: "balanced".into(),
            sensor: Sensor::Cpu,
            message: "Curve max temp 70°C but peak observed 55°C — headroom ok".into(),
            observed_max_temp: 55.0,
            curve_max_speed_temp: 70,
        }]);
        let json = serde_json::to_string(&resp).unwrap();
        let deser: Response = serde_json::from_str(&json).unwrap();
        if let Response::CurveSuggestions(suggestions) = deser {
            assert_eq!(suggestions.len(), 1);
            assert_eq!(suggestions[0].group, GroupId::new(1));
            assert_eq!(suggestions[0].curve_name, "balanced");
            assert_eq!(suggestions[0].sensor, Sensor::Cpu);
            assert!((suggestions[0].observed_max_temp - 55.0).abs() < f32::EPSILON);
            assert_eq!(suggestions[0].curve_max_speed_temp, 70);
        } else {
            panic!("expected Response::CurveSuggestions, got {deser:?}");
        }
    }

    #[test]
    fn recovery_counters_response_roundtrip() {
        let original = Response::RecoveryCounters(RecoveryCountersIpc {
            usb_reopen_attempts: 42,
            usb_reopen_successes: 40,
            usb_reopen_failures: 2,
            usb_soft_recovery_successes: 5,
            hwmon_rescan_attempts: 3,
            hwmon_rescan_successes: 2,
        });
        let json = serde_json::to_string(&original).expect("serialize");
        let roundtripped: Response = serde_json::from_str(&json).expect("deserialize");
        match roundtripped {
            Response::RecoveryCounters(c) => {
                assert_eq!(c.usb_reopen_attempts, 42);
                assert_eq!(c.usb_reopen_successes, 40);
                assert_eq!(c.usb_reopen_failures, 2);
                assert_eq!(c.usb_soft_recovery_successes, 5);
                assert_eq!(c.hwmon_rescan_attempts, 3);
                assert_eq!(c.hwmon_rescan_successes, 2);
            }
            other => panic!("expected RecoveryCounters, got {other:?}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuraChannelInfo {
    pub group: GroupId,
    pub name: String,
    pub led_count: u8,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    Ok,
    Error(String),
    Hello {
        protocol_version: u32,
    },
    DeviceStatus(Vec<GroupStatus>),
    GroupList(Vec<FanGroup>),
    CurveList(Vec<NamedCurve>),
    ProfileList(Vec<String>),
    SensorList(Vec<SensorInfo>),
    SensorReading {
        sensor: Sensor,
        value: f32,
    },
    FirmwareInfo(FirmwareInfo),
    DaemonConfig(Box<DaemonConfig>),
    AlertConfig(AlertConfig),
    SyncConfig(SyncConfig),
    EffectList(Vec<KeyframeEffect>),
    LedPresets(Vec<LedPreset>),
    Presets(Vec<crate::lcd::LcdPreset>),
    LcdDevices(Vec<crate::lcd::LcdDeviceInfo>),
    ScheduleList(Vec<ScheduleEntry>),
    SequenceList(Vec<Sequence>),
    Event(Event),
    AuraChannels(Vec<AuraChannelInfo>),
    LcdTemplates(Vec<crate::lcd::LcdTemplate>),
    /// Rendered template preview as JPEG bytes.
    TemplatePreview(Vec<u8>),
    /// Cumulative running-time statistics per fan group.
    WearStats(Vec<crate::device::GroupWearInfo>),
    /// Fan curve analysis results based on observed thermal headroom.
    CurveSuggestions(Vec<crate::speed::CurveSuggestion>),
    /// USB/HID stale-handle recovery counters snapshot.
    RecoveryCounters(RecoveryCountersIpc),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    SensorUpdate {
        sensor: Sensor,
        value: f32,
    },
    RpmUpdate {
        group: GroupId,
        rpms: Vec<u16>,
    },
    SpeedChanged {
        group: GroupId,
        mode: SpeedMode,
    },
    RgbChanged {
        group: GroupId,
        mode: RgbMode,
    },
    DeviceConnected {
        group: GroupId,
    },
    DeviceDisconnected {
        group: GroupId,
    },
    ProfileSwitched {
        name: String,
    },
    Alert(AlertEvent),
    PowerChanged {
        on_ac: bool,
    },
    FanStall {
        group: GroupId,
        fan: u8,
    },
    SequenceStep {
        group: GroupId,
        step_index: usize,
    },
    SequenceEnded {
        name: String,
    },
    CurveApplied {
        group: GroupId,
        speed: SpeedPercent,
        temp: Temperature,
    },
    BindDiscovered(crate::device::UnboundDevice),
    RpmAnomaly {
        group: GroupId,
        message: String,
    },
}
