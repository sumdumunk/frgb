use frgb_model::device::DeviceId;
use frgb_model::rgb::RgbMode;
use frgb_model::spec::SpecRegistry;
use frgb_model::speed::{PumpMode, SpeedMode};
use frgb_model::GroupId;
use frgb_model::SpeedPercent;

use crate::backend::{Backend, DiscoveredDevice, LianLiRfExt, SpeedCommand};
use crate::error::{CoreError, Result};
use crate::registry::{Device, DeviceRegistry};
use crate::services;

/// System is the top-level coordinator.
///
/// Owns backends and device registry, wires services between them.
/// Backends are type-erased (Box<dyn Backend>) so multiple backend
/// types can coexist (RF, wired, AIO, etc.).
pub struct System {
    backends: Vec<Box<dyn Backend>>,
    pub registry: DeviceRegistry,
    pub specs: SpecRegistry,
    pub raw_records: Vec<DiscoveredDevice>,
    pub unbound: Vec<frgb_model::device::UnboundDevice>,
}

impl System {
    /// Create a new System with no backends. Call `add_backend()` then `discover()`.
    pub fn new(specs: SpecRegistry) -> Self {
        Self {
            backends: Vec::new(),
            registry: DeviceRegistry::new(),
            specs,
            raw_records: Vec::new(),
            unbound: Vec::new(),
        }
    }

    /// Add a backend. Returns the BackendId assigned.
    pub fn add_backend(&mut self, backend: Box<dyn Backend>) {
        self.backends.push(backend);
    }

    /// Number of registered backends.
    pub fn backend_count(&self) -> usize {
        self.backends.len()
    }

    /// Discover all devices across all backends.
    ///
    /// Refreshes the registry: updates existing devices (preserving state),
    /// adds new ones, removes disconnected ones.
    #[tracing::instrument(skip(self))]
    pub fn discover(&mut self) -> Result<()> {
        self.raw_records.clear();
        self.unbound.clear();

        for backend in &mut self.backends {
            let discovered = backend.discover()?;
            self.raw_records.extend(discovered.clone());

            let our_mac = backend.as_rf_ext().and_then(|rf| rf.tx_id()).unwrap_or(DeviceId::ZERO);

            let unbound_discovered = self.registry.refresh(backend.id(), discovered, our_mac, &self.specs);

            for disc in unbound_discovered {
                let device_type = crate::registry::identify_device_type(disc.dev_type, &disc.fans_type, &self.specs);
                self.unbound.push(frgb_model::device::UnboundDevice {
                    mac: disc.id,
                    master: disc.master,
                    group: disc.group,
                    fan_count: disc.fan_count,
                    device_type,
                    fans_type: disc.fans_type,
                });
            }
        }

        // If a HydroShift LCD USB device is present on the bus, upgrade any
        // WaterBlock RF device to HydroShiftII. L-Connect identifies HydroShift II
        // by LCD USB device presence, not from the RF record alone.
        // Lightweight scan — no device open, works in CLI direct mode too.
        use frgb_model::usb_ids::{PID_HYDROSHIFT_CIRCLE, PID_HYDROSHIFT_SQUARE, VID_LCD};
        if frgb_usb::is_usb_device_present(VID_LCD, PID_HYDROSHIFT_CIRCLE)
            || frgb_usb::is_usb_device_present(VID_LCD, PID_HYDROSHIFT_SQUARE)
        {
            self.registry.upgrade_waterblock_to_hydroshift();
        }

        Ok(())
    }

    /// All discovered devices.
    pub fn devices(&self) -> &[Device] {
        self.registry.devices()
    }

    /// All group IDs from discovered devices.
    pub fn group_ids(&self) -> Vec<GroupId> {
        self.devices().iter().map(|d| d.group).collect()
    }

    /// Returns true if the device at `group_id` can accept fan-speed commands.
    /// AURA motherboard headers and other RGB-only devices return false.
    /// Returns false for unknown group IDs.
    pub fn is_fan_capable(&self, group_id: GroupId) -> bool {
        self.find_group(group_id)
            .map(|d| !d.device_type.is_motherboard())
            .unwrap_or(false)
    }

    /// Returns the group IDs of all devices that can accept fan-speed commands.
    /// Equivalent to `group_ids()` filtered by `is_fan_capable()`.
    pub fn fan_speed_groups(&self) -> Vec<GroupId> {
        self.group_ids()
            .into_iter()
            .filter(|gid| self.is_fan_capable(*gid))
            .collect()
    }

    /// Get the backend name for a given backend ID (e.g., "lianli-rf", "aura").
    pub fn backend_name(&self, id: crate::backend::BackendId) -> Option<&str> {
        self.backends.iter().find(|b| b.id() == id).map(|b| b.name())
    }

    /// Warn if any `GroupId` is claimed by more than one backend (spec §4.3).
    ///
    /// Call after `discover()` when all backends have populated the registry.
    pub fn warn_group_id_overlaps(&self) {
        use std::collections::HashMap;
        let mut owners: HashMap<GroupId, Vec<&str>> = HashMap::new();
        for dev in self.devices() {
            let backend_name = self.backend_name(dev.backend_id).unwrap_or("?");
            owners.entry(dev.group).or_default().push(backend_name);
        }
        for (group, backends) in &owners {
            if backends.len() > 1 {
                tracing::warn!(
                    "group {} claimed by multiple backends: {:?}. Check config.*.group_base for overlap.",
                    group.value(),
                    backends
                );
            }
        }
    }

    /// Find a device by group number.
    pub fn find_group(&self, group: GroupId) -> Result<&Device> {
        self.registry
            .find_by_group(group)
            .ok_or_else(|| CoreError::NotFound(format!("fan group {group}")))
    }

    /// Set RGB mode for a specific group. No-op for devices without LEDs.
    #[tracing::instrument(skip(self, mode))]
    pub fn set_rgb(&mut self, group_id: GroupId, mode: &RgbMode) -> Result<()> {
        let device = self.find_group(group_id)?;
        let backend_id = device.backend_id;

        // AURA devices carry their LED count in the backend channel config,
        // not in the spec registry. For all other devices, look up the spec.
        let is_aura = self
            .backends
            .iter()
            .find(|b| b.id() == backend_id)
            .is_some_and(|b| b.name() == "aura");

        let (leds_per_fan, fan_count) = if is_aura {
            let led_count = self
                .backends
                .iter()
                .find(|b| b.id() == backend_id)
                .and_then(|b| b.as_any().downcast_ref::<crate::AuraBackend>())
                .map(|aura| aura.led_count_for_group(device.group) as usize)
                .unwrap_or(0);
            if led_count == 0 {
                return Ok(());
            }
            (led_count, 1)
        } else {
            let leds = virtual_leds_per_fan(device, &self.specs);
            if leds == 0 {
                return Ok(());
            }
            (leds as usize, device.fan_count() as usize)
        };

        let buffer = services::rgb::compose(device.device_type, leds_per_fan, fan_count, mode)?;

        let device = self.find_group(group_id)?;
        let backend = self.backend_for(backend_id)?;
        backend.send_rgb(device, &buffer)?;

        self.registry
            .update_state(group_id, |s| s.rgb_mode = Some(mode.clone()));
        Ok(())
    }

    /// Set fan speed for a specific group.
    ///
    /// AIO pump devices (HydroShift II, Galahad, V150) do not respond to RF fan
    /// PWM packets — pump speed is streamed via the RF `aio_param` frame
    /// (command 0x12 0x21). Manual(pct) on a pump group is routed through
    /// `LianLiRfExt::set_aio_pump_speed`; PWM (motherboard sync) is a no-op
    /// on pumps.
    pub fn set_speed(&mut self, group_id: GroupId, mode: &SpeedMode) -> Result<()> {
        let cmd = match mode {
            SpeedMode::Manual(percent) => SpeedCommand::Manual(*percent),
            SpeedMode::Pwm => SpeedCommand::Pwm,
            SpeedMode::Curve(_) | SpeedMode::NamedCurve(_) => {
                return Ok(());
            }
        };

        let (backend_id, is_pump) = {
            let device = self.find_group(group_id)?;
            // Route any pump (AIO-labelled or raw WaterBlock) through the
            // AIO param frame. WaterBlock devices without the LCD upgrade
            // still need the pump-specific RF payload.
            let pump = device.device_type.is_aio()
                || matches!(device.role, frgb_model::device::FanRole::Pump);
            (device.backend_id, pump)
        };

        if is_pump {
            match &cmd {
                SpeedCommand::Manual(pct) => {
                    let device = self.find_group(group_id)?.clone();
                    {
                        let rf = self.rf_ext().ok_or_else(|| CoreError::NotFound("RF backend".into()))?;
                        rf.set_aio_pump_speed(&device, pct.value())?;
                    }
                    self.registry.update_state(group_id, |s| s.speed_percent = Some(*pct));
                }
                SpeedCommand::Pwm => {
                    // Motherboard PWM sync does not apply to AIO pumps.
                }
            }
            return Ok(());
        }

        let device = self.find_group(group_id)?;
        let backend = self.backend_for(backend_id)?;
        backend.set_speed(device, &cmd)?;
        match &cmd {
            SpeedCommand::Manual(pct) => self.registry.update_state(group_id, |s| s.speed_percent = Some(*pct)),
            SpeedCommand::Pwm => self.registry.update_state(group_id, |s| s.speed_percent = None),
        }
        Ok(())
    }

    /// Toggle motherboard PWM sync for a specific group.
    /// When disabling MB sync, also sends an initial speed command so the fan
    /// has a target to run at (uses the provided speed, or 50% default).
    pub fn set_mb_sync(&mut self, group_id: GroupId, enable: bool, speed_after: Option<SpeedPercent>) -> Result<()> {
        {
            let device = self.find_group(group_id)?;
            let rf = self.rf_ext().ok_or_else(|| CoreError::NotFound("RF backend".into()))?;
            rf.set_mb_sync(device, enable)?;
        }

        if !enable {
            // After disabling MB sync, send speed then SaveConfig (lock broadcast)
            // to persist the new state to firmware NVRAM. Order matters: speed
            // first so the fan has a target, then lock to commit.
            let pct = speed_after.unwrap_or(SpeedPercent::new(50));
            let device = self.find_group(group_id)?;
            let backend_id = device.backend_id;
            let backend = self.backend_for(backend_id)?;
            backend.set_speed(device, &SpeedCommand::Manual(pct))?;
            self.registry.update_state(group_id, |s| s.speed_percent = Some(pct));

            // SaveConfig / lock — persists current state to NVRAM
            let rf = self.rf_ext().ok_or_else(|| CoreError::NotFound("RF backend".into()))?;
            rf.lock()?;
        }
        Ok(())
    }

    /// Set LCD config (brightness + rotation) for a specific group.
    pub fn set_lcd(&self, group_id: GroupId, config: &frgb_model::lcd::LcdConfig) -> Result<()> {
        let device = self.find_group(group_id)?;
        let lcd = self
            .lcd_ext()
            .ok_or_else(|| CoreError::NotFound("LCD backend".into()))?;
        lcd.set_brightness(&device.id, config.brightness)?;
        lcd.set_rotation(&device.id, config.rotation)?;
        Ok(())
    }

    /// Set pump mode for an AIO cooler.
    ///
    /// PumpMode variants map to percent-of-range targets that are then scaled
    /// through the variant-specific RPM→PWM formula:
    ///   Quiet    = 25%
    ///   Standard = 50%
    ///   High     = 75%
    ///   Full     = 100%
    ///   Fixed(p) = p%
    ///
    /// The command is streamed via the Lian Li RF protocol (command 0x12 0x21)
    /// — pump hardware does not respond to LCD USB writes.
    pub fn set_pump(&mut self, group_id: GroupId, mode: &PumpMode) -> Result<()> {
        let device = {
            let device = self.find_group(group_id)?;
            if !device.device_type.is_aio() {
                return Err(CoreError::InvalidInput(format!(
                    "group {group_id} is not an AIO cooler"
                )));
            }
            device.clone()
        };

        let pct: u8 = match mode {
            PumpMode::Quiet => 25,
            PumpMode::Standard => 50,
            PumpMode::High => 75,
            PumpMode::Full => 100,
            PumpMode::Fixed(p) => (*p).min(100),
        };

        {
            let rf = self.rf_ext().ok_or_else(|| CoreError::NotFound("RF backend".into()))?;
            rf.set_aio_pump_speed(&device, pct)?;
        }

        // Reflect the commanded percentage in registry state so status
        // displays the set value instead of only the RPM-derived estimate.
        self.registry
            .update_state(group_id, |s| s.speed_percent = Some(SpeedPercent::new(pct)));
        Ok(())
    }

    /// List all LCD device IDs from the LCD backend.
    pub fn lcd_device_ids(&self) -> Vec<DeviceId> {
        self.lcd_ext().map(|lcd| lcd.lcd_device_ids()).unwrap_or_default()
    }

    /// List LCD device info for all LCD screens.
    pub fn lcd_device_info(&self) -> Vec<frgb_model::lcd::LcdDeviceInfo> {
        self.lcd_ext().map(|lcd| lcd.lcd_device_info()).unwrap_or_default()
    }

    /// Push a JPEG frame to an LCD device.
    pub fn send_lcd_frame(&self, device_id: &DeviceId, jpeg: &[u8]) -> Result<()> {
        let lcd = self
            .lcd_ext()
            .ok_or_else(|| CoreError::NotFound("LCD backend".into()))?;
        lcd.send_frame(device_id, jpeg)
    }

    /// Set LCD brightness for a specific device.
    pub fn set_lcd_brightness(&self, device_id: &DeviceId, brightness: frgb_model::Brightness) -> Result<()> {
        let lcd = self
            .lcd_ext()
            .ok_or_else(|| CoreError::NotFound("LCD backend".into()))?;
        lcd.set_brightness(device_id, brightness)
    }

    /// Set LCD rotation for a specific device.
    pub fn set_lcd_rotation(&self, device_id: &DeviceId, rotation: frgb_model::lcd::LcdRotation) -> Result<()> {
        let lcd = self
            .lcd_ext()
            .ok_or_else(|| CoreError::NotFound("LCD backend".into()))?;
        lcd.set_rotation(device_id, rotation)
    }

    /// Sync the on-device RTC clock for an LCD device.
    pub fn set_lcd_clock(&self, device_id: &DeviceId) -> Result<()> {
        let lcd = self
            .lcd_ext()
            .ok_or_else(|| CoreError::NotFound("LCD backend".into()))?;
        lcd.set_clock(device_id)
    }

    /// Access Lian Li RF extension from any backend that supports it.
    pub fn rf_ext(&self) -> Option<&dyn LianLiRfExt> {
        self.backends.iter().find_map(|b| b.as_rf_ext())
    }

    /// Access LCD extension from any backend that supports it.
    pub fn lcd_ext(&self) -> Option<&dyn crate::backend::LcdExt> {
        self.backends.iter().find_map(|b| b.as_lcd_ext())
    }

    /// Reset a device via its backend. Sends the appropriate reboot command.
    pub fn reset_device(&self, group_id: GroupId) -> Result<()> {
        let device = self.find_group(group_id)?;
        let backend = self.backend_for(device.backend_id)?;
        backend.reset_device(device)
    }

    /// Set hardware merge order for chained effect playback.
    /// Sends to all backends that support it.
    pub fn set_merge_order(&self, order: &[u8]) -> Result<()> {
        let mut sent = false;
        for backend in &self.backends {
            if backend.set_merge_order(order).is_ok() {
                sent = true;
            }
        }
        if sent {
            Ok(())
        } else {
            Err(CoreError::NotSupported("no backend supports merge order".into()))
        }
    }

    /// All registered backends as a slice.
    pub fn backends(&self) -> &[Box<dyn Backend>] {
        &self.backends
    }

    /// Find a backend by name.
    pub fn backend_by_name(&self, name: &str) -> Option<&dyn Backend> {
        self.backends.iter().find(|b| b.name() == name).map(|b| &**b)
    }

    /// Find a backend by name (mutable).
    pub fn backend_by_name_mut(&mut self, name: &str) -> Option<&mut dyn Backend> {
        for b in &mut self.backends {
            if b.name() == name {
                return Some(b.as_mut());
            }
        }
        None
    }

    fn backend_for(&self, id: crate::backend::BackendId) -> Result<&dyn Backend> {
        self.backends
            .iter()
            .find(|b| b.id() == id)
            .map(|b| &**b)
            .ok_or_else(|| CoreError::NotFound(format!("backend {:?}", id)))
    }
}

/// Get virtual LEDs per fan for a device from its DeviceSpec.
fn virtual_leds_per_fan(device: &Device, specs: &SpecRegistry) -> u16 {
    device
        .slots
        .first()
        .and_then(|slot| specs.lookup_fans_type(slot.fans_type))
        .map(|spec| spec.virtual_leds as u16)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendId;
    use crate::rf_backend::LianLiRfBackend;
    use crate::transport::mock::MockTransport;
    use frgb_model::rgb::{Rgb, Ring};
    use frgb_model::spec_loader::load_defaults;
    use frgb_model::Brightness;
    use frgb_model::SpeedPercent;

    fn make_test_system() -> System {
        let tx = MockTransport::new();
        let rx = MockTransport::new();
        let mut backend = LianLiRfBackend::new(tx, rx, Some(0x08));
        backend.set_tx_id(DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]));

        let specs = load_defaults();
        let mut system = System::new(specs);
        system.add_backend(Box::new(backend));

        let tx_ref = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        system.registry.refresh(
            BackendId(0),
            vec![crate::backend::DiscoveredDevice {
                id: DeviceId::from([0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]),
                fans_type: [21, 21, 21, 0],
                dev_type: 0,
                group: GroupId::new(1),
                fan_count: 3,
                master: tx_ref,
                fans_rpm: [1400, 1400, 1400, 0],
                fans_pwm: [0; 4],
                cmd_seq: 0,
                channel: 0x08,
            }],
            tx_ref,
            &system.specs,
        );

        system
    }

    #[test]
    fn find_group() {
        let sys = make_test_system();
        assert!(sys.find_group(GroupId::new(1)).is_ok());
        assert!(sys.find_group(GroupId::new(99)).is_err());
    }

    #[test]
    fn set_rgb_static() {
        let mut sys = make_test_system();
        let mode = RgbMode::Static {
            ring: Ring::Both,
            color: Rgb { r: 254, g: 0, b: 0 },
            brightness: Brightness::new(255),
        };
        sys.set_rgb(GroupId::new(1), &mode).unwrap();
    }

    #[test]
    fn set_speed_manual() {
        let mut sys = make_test_system();
        sys.set_speed(GroupId::new(1), &SpeedMode::Manual(SpeedPercent::new(50)))
            .unwrap();
    }

    #[test]
    fn set_rgb_updates_state() {
        let mut sys = make_test_system();
        let mode = RgbMode::Off;
        sys.set_rgb(GroupId::new(1), &mode).unwrap();
        assert_eq!(
            sys.find_group(GroupId::new(1)).unwrap().state.rgb_mode,
            Some(RgbMode::Off)
        );
    }

    #[test]
    fn set_speed_updates_state() {
        let mut sys = make_test_system();
        sys.set_speed(GroupId::new(1), &SpeedMode::Manual(SpeedPercent::new(70)))
            .unwrap();
        assert_eq!(
            sys.find_group(GroupId::new(1)).unwrap().state.speed_percent,
            Some(SpeedPercent::new(70))
        );
    }

    #[test]
    fn rf_ext_accessible() {
        let sys = make_test_system();
        assert!(sys.rf_ext().is_some());
    }

    /// Add a HydroShiftII pump group to a test system. Discovers a WaterBlock
    /// via RF then upgrades the device_type. Used by pump routing tests.
    fn add_pump_group(system: &mut System, group: GroupId) {
        let tx_ref = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        // WaterBlock discovery: dev_type=10, pump slot at index 3 (RPM=1800, fans_type=0).
        system.registry.refresh(
            BackendId(0),
            vec![crate::backend::DiscoveredDevice {
                id: DeviceId::from([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]),
                fans_type: [0, 0, 0, 0],
                dev_type: 10,
                group,
                fan_count: 1,
                master: tx_ref,
                fans_rpm: [0, 0, 0, 1800],
                fans_pwm: [0; 4],
                cmd_seq: 0,
                channel: 0x08,
            }],
            tx_ref,
            &system.specs,
        );
        system.registry.upgrade_waterblock_to_hydroshift();
    }

    /// Scan all packets written to the mock TX and return the first that
    /// contains a given command type in its RF payload header. RF frames are
    /// split 4 × 60 bytes across USB packets; only the first chunk carries
    /// the command bytes at payload offset 0-1 (packet bytes 4-5 after the
    /// `[0x10, seq, channel, rx_type]` header).
    fn find_rf_cmd_packet(packets: &[Vec<u8>], cmd: [u8; 2]) -> Option<&Vec<u8>> {
        packets.iter().find(|pkt| {
            pkt.len() >= 6
                && pkt[0] == 0x10
                && pkt[1] == 0 // seq 0 carries payload bytes 0-59 → RF header
                && pkt[4] == cmd[0]
                && pkt[5] == cmd[1]
        })
    }

    /// `speed` on a pump group must emit an RF AIO info frame
    /// (0x12 0x21), not a fan PWM frame (0x12 0x10). This verifies the routing
    /// through LianLiRfExt::set_aio_pump_speed.
    #[test]
    fn set_speed_on_pump_sends_aio_info_frame() {
        use frgb_protocol::constants::{CMD_TYPE_AIO_INFO, CMD_TYPE_SPEED};

        let mut sys = make_test_system();
        add_pump_group(&mut sys, GroupId::new(7));
        assert_eq!(
            sys.find_group(GroupId::new(7)).unwrap().device_type,
            frgb_model::device::DeviceType::HydroShiftII,
        );

        sys.set_speed(GroupId::new(7), &SpeedMode::Manual(SpeedPercent::new(80)))
            .unwrap();

        // Registry state reflects the commanded percentage.
        assert_eq!(
            sys.find_group(GroupId::new(7)).unwrap().state.speed_percent,
            Some(SpeedPercent::new(80))
        );

        // The RF backend must have written AIO info frames, not fan PWM frames.
        let rf = sys
            .backends
            .iter()
            .find_map(|b| b.as_any().downcast_ref::<LianLiRfBackend<MockTransport>>())
            .expect("rf backend present");
        let packets = rf.tx().written_packets();
        assert!(
            find_rf_cmd_packet(&packets, CMD_TYPE_AIO_INFO).is_some(),
            "expected CMD_TYPE_AIO_INFO (0x12 0x21) in TX packets, got {} packets",
            packets.len(),
        );
        assert!(
            find_rf_cmd_packet(&packets, CMD_TYPE_SPEED).is_none(),
            "pump groups must not emit fan PWM frames",
        );
    }

    /// PWM (motherboard sync) is a no-op on AIO pumps. Must
    /// return Ok without touching the RF bus.
    #[test]
    fn set_speed_pwm_on_pump_is_noop() {
        let mut sys = make_test_system();
        add_pump_group(&mut sys, GroupId::new(7));
        sys.set_speed(GroupId::new(7), &SpeedMode::Pwm).unwrap();

        let rf = sys
            .backends
            .iter()
            .find_map(|b| b.as_any().downcast_ref::<LianLiRfBackend<MockTransport>>())
            .expect("rf backend present");
        assert!(
            rf.tx().written_packets().is_empty(),
            "PWM on a pump must not send any RF packets",
        );
    }

    /// set_pump() rejects non-AIO groups with InvalidInput,
    /// preventing accidental pump commands on fan groups.
    #[test]
    fn set_pump_rejects_non_aio_group() {
        let mut sys = make_test_system();
        let err = sys.set_pump(GroupId::new(1), &PumpMode::Fixed(70)).unwrap_err();
        assert!(
            matches!(&err, CoreError::InvalidInput(msg) if msg.contains("not an AIO")),
            "expected InvalidInput, got {err:?}",
        );
    }

    /// set_pump() with PumpMode::Quiet maps to 25% and emits
    /// an AIO info frame with the pump bytes set.
    #[test]
    fn set_pump_named_mode_sends_aio_info() {
        use frgb_protocol::constants::CMD_TYPE_AIO_INFO;

        let mut sys = make_test_system();
        add_pump_group(&mut sys, GroupId::new(7));
        sys.set_pump(GroupId::new(7), &PumpMode::Quiet).unwrap();

        assert_eq!(
            sys.find_group(GroupId::new(7)).unwrap().state.speed_percent,
            Some(SpeedPercent::new(25))
        );

        let rf = sys
            .backends
            .iter()
            .find_map(|b| b.as_any().downcast_ref::<LianLiRfBackend<MockTransport>>())
            .expect("rf backend present");
        let packets = rf.tx().written_packets();
        let first = find_rf_cmd_packet(&packets, CMD_TYPE_AIO_INFO).expect("CMD_TYPE_AIO_INFO frame");
        // aio_param[7] = 1 (pump_enable) lives at payload offset 18+7 = 25.
        // In the TX packet that's 4 (header) + 25 = byte 29.
        assert_eq!(first[4 + 18 + 7], 1, "pump_enable must be set");
        // aio_param[25] = 80 (lcd brightness default) at payload offset 18+25 = 43.
        assert_eq!(first[4 + 18 + 25], 80, "lcd brightness default");
    }

    /// Replace the registry for BackendId(0) with both the original SL fan group
    /// AND an AURA group. refresh() removes devices not in seen_groups for that
    /// backend, so both must be supplied in a single call.
    fn rebuild_test_system_with_aura(system: &mut System) {
        let tx_ref = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        system.registry.refresh(
            BackendId(0),
            vec![
                // SL fan at group 1 (matches make_test_system).
                crate::backend::DiscoveredDevice {
                    id: DeviceId::from([0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]),
                    fans_type: [21, 21, 21, 0],
                    dev_type: 0,
                    group: GroupId::new(1),
                    fan_count: 3,
                    master: tx_ref,
                    fans_rpm: [1400, 1400, 1400, 0],
                    fans_pwm: [0; 4],
                    cmd_seq: 0,
                    channel: 0x08,
                },
                // AURA group 5 — synthetic dev_type=0xFD maps to DeviceType::Aura.
                crate::backend::DiscoveredDevice {
                    id: DeviceId::from([0xff, 0xff, 0xff, 0xff, 0xff, 0xfd]),
                    fans_type: [0, 0, 0, 0],
                    dev_type: 0xFD,
                    group: GroupId::new(5),
                    fan_count: 0,
                    master: tx_ref,
                    fans_rpm: [0; 4],
                    fans_pwm: [0; 4],
                    cmd_seq: 0,
                    channel: 0x08,
                },
            ],
            tx_ref,
            &system.specs,
        );
    }

    #[test]
    fn is_fan_capable_true_for_rf_fan() {
        let sys = make_test_system();
        // make_test_system() registers a 3-fan SL group at GroupId::new(1).
        assert!(sys.is_fan_capable(GroupId::new(1)));
    }

    #[test]
    fn is_fan_capable_false_for_aura() {
        let mut sys = make_test_system();
        rebuild_test_system_with_aura(&mut sys);
        assert!(!sys.is_fan_capable(GroupId::new(5)));
    }

    #[test]
    fn is_fan_capable_false_for_unknown_group() {
        let sys = make_test_system();
        assert!(!sys.is_fan_capable(GroupId::new(99)));
    }

    #[test]
    fn fan_speed_groups_excludes_aura() {
        let mut sys = make_test_system();
        rebuild_test_system_with_aura(&mut sys);
        let groups = sys.fan_speed_groups();
        assert!(groups.contains(&GroupId::new(1)), "RF group 1 should be in fan_speed_groups");
        assert!(!groups.contains(&GroupId::new(5)), "AURA group 5 should NOT be in fan_speed_groups");
    }
}
