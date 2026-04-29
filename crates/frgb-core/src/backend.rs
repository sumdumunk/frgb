use std::any::Any;

use frgb_model::device::DeviceId;
use frgb_model::GroupId;
use frgb_model::SpeedPercent;
use frgb_rgb::generator::EffectResult;

use crate::error::{CoreError, Result};
use crate::registry::Device;

// ---------------------------------------------------------------------------
// BackendId — unique identifier for a backend instance
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BackendId(pub u8);

// ---------------------------------------------------------------------------
// DiscoveredDevice — raw discovery result before registry integration
// ---------------------------------------------------------------------------

/// A device discovered by a backend, before it's integrated into the DeviceRegistry.
#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    /// Unique identifier within this backend (e.g., fan MAC for RF, sysfs path for hwmon).
    pub id: DeviceId,
    /// The fans_type byte for RF devices, or a synthetic key for other backends.
    pub fans_type: [u8; 4],
    /// Raw dev_type byte from the discovery protocol (0 for wireless fans).
    pub dev_type: u8,
    /// Group assignment from the protocol.
    pub group: GroupId,
    /// Number of daisy-chained units.
    pub fan_count: u8,
    /// Master MAC — identifies which controller this device is bound to.
    pub master: DeviceId,
    /// Per-slot RPM readings.
    pub fans_rpm: [u16; 4],
    /// Per-slot PWM bytes from discovery (current speed the fan is running).
    /// All slots at SPEED_MIN (6) indicates motherboard sync mode.
    pub fans_pwm: [u8; 4],
    /// Command sequence number — firmware change-detection counter.
    /// State-change commands (MB sync toggle) require cmd_seq + 1.
    pub cmd_seq: u8,
    /// RF channel (for channel auto-detection).
    pub channel: u8,
}

// ---------------------------------------------------------------------------
// Backend trait — transport + discovery, no domain logic
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// SpeedCommand — what the backend receives for speed control
// ---------------------------------------------------------------------------

/// Protocol-agnostic speed command. System translates SpeedMode → SpeedCommand
/// before passing to the backend. The backend handles wire-level framing.
pub enum SpeedCommand {
    /// Set fan speed to a percentage (0-100).
    Manual(SpeedPercent),
    /// Release fans to motherboard PWM control.
    Pwm,
}

// ---------------------------------------------------------------------------
// Backend trait — transport + discovery, no domain logic
// ---------------------------------------------------------------------------

/// Backend handles discovery and raw command transport.
/// Domain logic (composition, curves) lives in Services, not here.
pub trait Backend {
    fn id(&self) -> BackendId;
    fn name(&self) -> &str;

    /// Discover devices on this backend. Returns raw device info
    /// for the registry to integrate.
    fn discover(&mut self) -> Result<Vec<DiscoveredDevice>>;

    /// Send a speed command. Backend handles protocol framing.
    fn set_speed(&self, device: &Device, cmd: &SpeedCommand) -> Result<()>;

    /// Send a pre-composed RGB buffer. Backend handles compression + framing.
    fn send_rgb(&self, device: &Device, buffer: &EffectResult) -> Result<()>;

    /// Reset/reboot a device. Returns Ok if supported, Err if not.
    fn reset_device(&self, _device: &Device) -> Result<()> {
        Err(CoreError::NotSupported("reset not supported by this backend".into()))
    }

    /// Set hardware merge order for chained effect playback (4 group indices).
    fn set_merge_order(&self, _order: &[u8]) -> Result<()> {
        Err(CoreError::NotSupported(
            "merge order not supported by this backend".into(),
        ))
    }

    /// Lian Li RF extension (bind, lock, unlock). Override if supported.
    fn as_rf_ext(&self) -> Option<&dyn LianLiRfExt> {
        None
    }

    /// LCD extension (frame display). Override if supported.
    fn as_lcd_ext(&self) -> Option<&dyn LcdExt> {
        None
    }

    /// Generic downcast for future extension traits.
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

// ---------------------------------------------------------------------------
// Extension traits — accessed via as_any() downcast
// ---------------------------------------------------------------------------

/// Lian Li RF-specific operations: bind, lock, unlock, MB sync control.
pub trait LianLiRfExt {
    fn bind_device(&self, fan_mac: &DeviceId, target_group: GroupId) -> Result<()>;
    fn unbind_device(&self, fan_mac: &DeviceId, group: GroupId) -> Result<()>;
    fn lock(&self) -> Result<()>;
    fn unlock(&self) -> Result<()>;
    fn channel(&self) -> u8;
    fn tx_id(&self) -> Option<DeviceId>;
    fn tx_firmware_version(&self) -> Option<u16>;

    /// Toggle motherboard PWM sync for a device (command [0x12, 0x24]).
    /// `enable=false` releases the device from MB control so speed commands persist.
    /// `enable=true` hands the device back to motherboard PWM.
    fn set_mb_sync(&self, device: &Device, enable: bool) -> Result<()>;

    /// Set AIO pump speed for a HydroShift II device.
    ///
    /// `pct` is 0-100% of the usable RPM range (PUMP_MIN_RPM to variant max).
    /// Variant (Circle / Square) is detected from the device's `slots[].fans_type`
    /// value (110 = WaterBlock / Circle, 111 = WaterBlock2 / Square).
    ///
    fn set_aio_pump_speed(&self, device: &Device, pct: u8) -> Result<()>;
}

/// LCD-specific operations: content display, brightness, rotation.
pub trait LcdExt {
    /// List all LCD device IDs managed by this backend.
    fn lcd_device_ids(&self) -> Vec<DeviceId>;
    /// List LCD device info (name, resolution) for each managed device.
    fn lcd_device_info(&self) -> Vec<frgb_model::lcd::LcdDeviceInfo>;
    /// Push a raw JPEG frame to the LCD. The backend handles packetizing and encryption.
    fn send_frame(&self, device_id: &DeviceId, jpeg: &[u8]) -> Result<()>;
    /// Set brightness (0-255, mapped to hardware range 0-50 internally).
    fn set_brightness(&self, device_id: &DeviceId, level: frgb_model::Brightness) -> Result<()>;
    /// Set display rotation.
    fn set_rotation(&self, device_id: &DeviceId, rotation: frgb_model::lcd::LcdRotation) -> Result<()>;
    /// Sync the on-device RTC clock to the current local time.
    fn set_clock(&self, device_id: &DeviceId) -> Result<()>;

    /// Upload an H.264 file to device storage and start on-device playback.
    fn upload_h264(&self, _device_id: &DeviceId, _data: &[u8]) -> Result<()> {
        Err(CoreError::NotSupported(
            "H.264 playback not supported by this device".into(),
        ))
    }

    /// Stop on-device H.264 playback.
    fn stop_h264(&self, _device_id: &DeviceId) -> Result<()> {
        Err(CoreError::NotSupported(
            "H.264 stop not supported by this device".into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// SensorReading — generic sensor data from any backend
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SensorReading {
    pub label: String,
    pub value: f64,
    pub unit: SensorUnit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SensorUnit {
    Celsius,
    Rpm,
    Watts,
    Volts,
    Amps,
    /// Dimensionless percentage (0–100), e.g. GPU utilization.
    Percent,
}
