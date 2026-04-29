use frgb_model::device::{BladeType, DeviceId, DeviceType, FanRole};
use frgb_model::rgb::RgbMode;
use frgb_model::spec::{DeviceKind, SpecRegistry};
use frgb_model::GroupId;
use frgb_model::SpeedPercent;

use crate::backend::{BackendId, DiscoveredDevice};

// ---------------------------------------------------------------------------
// Device — a controllable device group
// ---------------------------------------------------------------------------

/// A controllable device group (Lian Li RF group, hwmon device, AURA header).
#[derive(Debug, Clone)]
pub struct Device {
    /// Primary identifier (first fan MAC for RF, sysfs path for hwmon).
    pub id: DeviceId,
    /// Which backend controls this device.
    pub backend_id: BackendId,
    /// Protocol-assigned group number (1-8 for Lian Li RF).
    pub group: GroupId,
    /// Individual hardware units within this device (fans, pumps).
    pub slots: Vec<DeviceSlot>,
    /// Mutable runtime state.
    pub state: DeviceState,
    /// All MACs in this device group (for RF groups with multiple receivers).
    pub mac_ids: Vec<DeviceId>,
    /// TX/master MAC reference (for Lian Li RF protocol).
    pub tx_ref: DeviceId,
    /// Display name (derived from spec or user-assigned).
    pub name: String,
    /// Primary device type (from first slot's spec).
    pub device_type: DeviceType,
    /// Fan role (intake/exhaust/pump).
    pub role: FanRole,
    /// Blade orientation.
    pub blade: BladeType,
    /// True if device is under motherboard PWM control (fans_pwm all at SPEED_MIN).
    pub mb_sync: bool,
    /// Firmware command sequence counter — needed for state-change commands.
    pub cmd_seq: u8,
}

impl Device {
    /// Number of active slots (non-empty fans_type).
    pub fn fan_count(&self) -> u8 {
        self.slots.iter().filter(|s| s.fans_type != 0).count() as u8
    }

    /// Per-slot RPM readings as a fixed-size array (for protocol compatibility).
    pub fn fans_rpm(&self) -> [u16; 4] {
        let mut rpms = [0u16; 4];
        for (i, slot) in self.slots.iter().enumerate().take(4) {
            rpms[i] = slot.rpm;
        }
        rpms
    }

    /// Per-slot fans_type bytes as a fixed-size array.
    pub fn fans_type(&self) -> [u8; 4] {
        let mut ft = [0u8; 4];
        for (i, slot) in self.slots.iter().enumerate().take(4) {
            ft[i] = slot.fans_type;
        }
        ft
    }

    /// Current visual/speed state as a Scene (for profile snapshots).
    pub fn current_scene(&self) -> frgb_model::show::Scene {
        frgb_model::show::Scene {
            rgb: self.state.rgb_mode.clone().unwrap_or(frgb_model::rgb::RgbMode::Off),
            speed: self.state.speed_percent.map(frgb_model::speed::SpeedMode::Manual),
            lcd: None,
        }
    }
}

// ---------------------------------------------------------------------------
// DeviceSlot — individual hardware unit within a device
// ---------------------------------------------------------------------------

/// Individual hardware unit within a device.
/// The slot's spec (from SpecRegistry) is the source of truth for its
/// capabilities, RPM range, CFM, LED layout, etc.
#[derive(Debug, Clone)]
pub struct DeviceSlot {
    /// Spec lookup key (fans_type byte for Lian Li RF).
    pub fans_type: u8,
    /// Last known RPM reading.
    pub rpm: u16,
    /// Whether this slot has an LCD panel (from spec has_lcd).
    pub has_lcd: bool,
    /// Index (0..=3) in the original discovery packet's fans_rpm/fans_type
    /// arrays that this slot was created from. Required because AIO pumps put
    /// telemetry at discovery slot 3, but filtering can move that slot to a
    /// lower position in Device::slots — subsequent RPM updates must still
    /// read from the original source index to pick up the pump reading.
    pub source_idx: u8,
}

// ---------------------------------------------------------------------------
// DeviceState — mutable runtime state per device
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct DeviceState {
    pub speed_percent: Option<SpeedPercent>,
    pub rgb_mode: Option<RgbMode>,
}

// ---------------------------------------------------------------------------
// DeviceRegistry — manages all known devices
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct DeviceRegistry {
    devices: Vec<Device>,
}

impl DeviceRegistry {
    /// Create an empty registry. Devices are populated by `refresh()`.
    pub fn new() -> Self {
        Self { devices: Vec::new() }
    }

    /// All devices.
    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    /// Find a device by group number.
    pub fn find_by_group(&self, group: GroupId) -> Option<&Device> {
        self.devices.iter().find(|d| d.group == group)
    }

    /// Find a device by group number (mutable).
    pub fn find_by_group_mut(&mut self, group: GroupId) -> Option<&mut Device> {
        self.devices.iter_mut().find(|d| d.group == group)
    }

    /// Find a device by primary ID.
    pub fn find_by_id(&self, id: &DeviceId) -> Option<&Device> {
        self.devices.iter().find(|d| d.id == *id)
    }

    /// All devices from a specific backend.
    pub fn devices_by_backend(&self, backend_id: BackendId) -> Vec<&Device> {
        self.devices.iter().filter(|d| d.backend_id == backend_id).collect()
    }

    /// Upgrade WaterBlock/WaterBlock2 devices to HydroShiftII.
    /// Called when the HydroShift LCD USB device is detected, confirming
    /// the pump block is part of a HydroShift II AIO cooler.
    pub fn upgrade_waterblock_to_hydroshift(&mut self) {
        for dev in &mut self.devices {
            match dev.device_type {
                DeviceType::WaterBlock | DeviceType::WaterBlock2 => {
                    dev.device_type = DeviceType::HydroShiftII;
                    dev.name = "HydroShift II".into();
                    // Mark all slots as LCD-capable now that we know it's a HydroShift
                    for slot in &mut dev.slots {
                        slot.has_lcd = true;
                    }
                }
                _ => {}
            }
        }
    }

    /// Apply user-configured properties (role, name) from config groups.
    /// Called after discovery to overlay user preferences onto hardware-detected devices.
    pub fn apply_group_configs(&mut self, configs: &[frgb_model::config::GroupConfig]) {
        for cfg in configs {
            if let Some(dev) = self.devices.iter_mut().find(|d| d.group == cfg.id) {
                dev.role = cfg.role.clone();
                if !cfg.name.is_empty() {
                    dev.name = cfg.name.clone();
                }
            }
        }
    }

    /// Seed device state from saved config so Status reports match the last-known
    /// hardware state. Called once after discovery on daemon startup.
    pub fn seed_state_from_config(&mut self, groups: &[frgb_model::config::GroupConfig]) {
        for gc in groups {
            self.update_state(gc.id, |state| {
                if let frgb_model::speed::SpeedMode::Manual(pct) = &gc.speed {
                    state.speed_percent = Some(*pct);
                }
                state.rgb_mode = Some(gc.rgb.clone());
            });
        }
    }

    /// Refresh the registry from a new discovery pass.
    ///
    /// Preserves DeviceState (speed, RGB mode) for groups that are still present.
    /// Removes groups that disappeared. Adds newly discovered groups.
    /// Returns unbound devices (not bound to our controller).
    pub fn refresh(
        &mut self,
        backend_id: BackendId,
        discovered: Vec<DiscoveredDevice>,
        our_mac: DeviceId,
        specs: &SpecRegistry,
    ) -> Vec<DiscoveredDevice> {
        let mut unbound = Vec::new();

        // Track which existing groups were seen this pass
        let mut seen_groups = Vec::new();

        for disc in discovered {
            if disc.dev_type == 0xFF {
                continue;
            }

            if disc.master != our_mac {
                unbound.push(disc);
                continue;
            }

            if !seen_groups.contains(&disc.group) {
                seen_groups.push(disc.group);
            }

            if let Some(existing) = self
                .devices
                .iter_mut()
                .find(|d| d.group == disc.group && d.backend_id == backend_id)
            {
                // Update existing device — preserve state, refresh hardware data
                if !existing.mac_ids.contains(&disc.id) {
                    existing.mac_ids.push(disc.id);
                }
                // LCD-only devices (dev_type=0xFE) have 0 fans — don't force a slot
                let new_count = if disc.dev_type == 0xFE {
                    disc.fan_count as usize
                } else {
                    disc.fan_count.max(1) as usize
                };
                if new_count > existing.slots.len() {
                    for i in existing.slots.len()..new_count {
                        let mut ft = disc.fans_type.get(i).copied().unwrap_or(0);
                        if ft == 0 && disc.dev_type > 0 {
                            ft = 100u8.saturating_add(disc.dev_type);
                        }
                        let rpm = disc.fans_rpm.get(i).copied().unwrap_or(0);
                        let has_lcd = specs.lookup_fans_type(ft).is_some_and(|s| s.has_lcd);
                        existing.slots.push(DeviceSlot {
                            fans_type: ft,
                            rpm,
                            has_lcd,
                            source_idx: i as u8,
                        });
                    }
                }
                // Update RPMs from the *original* discovery-slot index, not
                // the position in Device::slots. AIO pumps land in slots[0]
                // after filtering but their telemetry stays at fans_rpm[3].
                for slot in existing.slots.iter_mut() {
                    if let Some(&rpm) = disc.fans_rpm.get(slot.source_idx as usize) {
                        slot.rpm = rpm;
                    }
                }
                // state is intentionally NOT reset — speed/rgb persist
                // mb_sync and cmd_seq refresh from hardware each discovery cycle
                let fan_count = disc.fan_count.max(1) as usize;
                existing.mb_sync =
                    has_hw_mb_sync(&disc.fans_type) && disc.fans_pwm[..fan_count].iter().all(|&p| p == 6);
                existing.cmd_seq = disc.cmd_seq;

                // Backfill speed_percent from hardware PWM if we still don't
                // have a cached value. Only valid for SLV3 fans (dev_type == 0) —
                // other types (CL/TL/pumps) report sentinel/non-PWM bytes that
                // scale to bogus duty values. Pumps notably have a raw
                // fans_type[3]=26 byte that would otherwise pass has_hw_mb_sync.
                // CLI/GUI fall back to RPM/max_rpm when this remains None.
                if existing.state.speed_percent.is_none()
                    && !existing.mb_sync
                    && disc.dev_type == 0
                    && has_hw_mb_sync(&disc.fans_type)
                {
                    if let Some(byte) = disc.fans_pwm[..fan_count].iter().copied().find(|&p| p > 0) {
                        existing.state.speed_percent = Some(frgb_protocol::color::speed_byte_to_percent(byte));
                    }
                }
            } else {
                // New device — default state.
                // For dev_type>0 devices (WaterBlock, Strimer), scan all 4 discovery
                // slots and keep only occupied ones. WaterBlock puts pump data at
                // slot 3: fans_type=[0,0,0,26] rpm=[0,0,0,1822].
                let slots: Vec<DeviceSlot> = (0..4)
                    .filter_map(|i| {
                        let ft = disc.fans_type.get(i).copied().unwrap_or(0);
                        let rpm = disc.fans_rpm.get(i).copied().unwrap_or(0);

                        if disc.dev_type > 0 {
                            // Non-fan device: skip empty slots, always use synthetic
                            // spec key. Raw fans_type bytes have device-specific
                            // semantics and are not fan subtype lookup keys.
                            if ft == 0 && rpm == 0 {
                                return None;
                            }
                            let key = 100u8.saturating_add(disc.dev_type);
                            let has_lcd = specs.lookup_fans_type(key).is_some_and(|s| s.has_lcd);
                            Some(DeviceSlot {
                                fans_type: key,
                                rpm,
                                has_lcd,
                                source_idx: i as u8,
                            })
                        } else {
                            // Fan device: use fan_count to limit slots
                            if i >= disc.fan_count.max(1) as usize {
                                return None;
                            }
                            let has_lcd = specs.lookup_fans_type(ft).is_some_and(|s| s.has_lcd);
                            Some(DeviceSlot {
                                fans_type: ft,
                                rpm,
                                has_lcd,
                                source_idx: i as u8,
                            })
                        }
                    })
                    .collect();

                let device_type = identify_device_type(disc.dev_type, &disc.fans_type, specs);
                let slot_spec = slots.first().and_then(|s| specs.lookup_fans_type(s.fans_type));
                let is_reverse = slot_spec.is_some_and(|s| s.is_reverse);
                let blade = if is_reverse {
                    BladeType::Reverse
                } else {
                    BladeType::Standard
                };
                let name = if disc.dev_type == 0xFE {
                    lcd_name_from_id(&disc.id, disc.group)
                } else {
                    slot_spec.map_or_else(|| format!("Group {}", disc.group), |s| s.name.clone())
                };

                let role = if slot_spec.is_some_and(|s| s.kind == DeviceKind::Pump) {
                    FanRole::Pump
                } else {
                    FanRole::Intake
                };

                // MB sync: only SLV3 fans (20-26) support hardware MB sync.
                // Other types (CL, TL, etc.) always report PWM=6 as minimum speed.
                let fan_count = disc.fan_count.max(1) as usize;
                let mb_sync = has_hw_mb_sync(&disc.fans_type) && disc.fans_pwm[..fan_count].iter().all(|&p| p == 6);

                // Seed state.speed_percent from the hardware's reported PWM so
                // the UI shows the actual running speed immediately on startup
                // instead of "unknown / 0".
                //
                // Restricted to SLV3 fans (fans_type 20..=26) AND dev_type == 0:
                // only SLV3 fans report a real PWM duty in fans_pwm[i]. CL/TL/
                // Strimer/AIO pumps either pin the byte to the 0x06 sentinel or
                // use it for an unrelated purpose (pump RPM register, etc.), so
                // seeding from there produces bogus values. Pumps in particular
                // can have raw fans_type[3]=26 (an SLV3 byte in the last slot
                // per firmware encoding) which would otherwise pass the
                // has_hw_mb_sync check — hence the explicit dev_type == 0 guard.
                // CLI/GUI fall back to RPM/max_rpm in their own status display
                // when this remains None.
                //
                // Also skipped for mb-sync devices — their PWM bytes are the
                // 0x06 sentinel, not a real duty cycle.
                let initial_speed_percent = if mb_sync || disc.dev_type != 0 || !has_hw_mb_sync(&disc.fans_type) {
                    None
                } else {
                    disc.fans_pwm[..fan_count]
                        .iter()
                        .copied()
                        .find(|&p| p > 0)
                        .map(frgb_protocol::color::speed_byte_to_percent)
                };
                let initial_state = DeviceState {
                    speed_percent: initial_speed_percent,
                    rgb_mode: None,
                };

                self.devices.push(Device {
                    id: disc.id,
                    backend_id,
                    group: disc.group,
                    slots,
                    state: initial_state,
                    mac_ids: vec![disc.id],
                    tx_ref: our_mac,
                    name,
                    device_type,
                    role,
                    blade,
                    mb_sync,
                    cmd_seq: disc.cmd_seq,
                });
            }
        }

        // Remove devices from this backend that weren't seen this pass
        self.devices
            .retain(|d| d.backend_id != backend_id || seen_groups.contains(&d.group));

        unbound
    }

    /// Update a device's state by group number.
    pub fn update_state(&mut self, group: GroupId, f: impl FnOnce(&mut DeviceState)) {
        if let Some(device) = self.devices.iter_mut().find(|d| d.group == group) {
            f(&mut device.state);
        }
    }
}

impl Default for DeviceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Device type identification (data-driven via SpecRegistry)
// ---------------------------------------------------------------------------

/// Identify device type from discovery record using SpecRegistry.
///
/// dev_type == 0: wireless fan, identified by fans_type[0] byte.
/// dev_type > 0: non-fan device (Strimer, WaterBlock, etc.), identified by dev_type byte.
/// Both paths are fully data-driven via devices.toml.
/// Whether the device supports hardware motherboard PWM sync.
/// SLV3 fans (20-26) and CL fans (41-42) read the local MB PWM header
/// directly. RL120 (40) shares the CLV1 electrical class but does NOT
/// support mobo sync. TL and SL-INF always report PWM=6 as their
/// minimum speed and require software relay of MB PWM from the RX dongle.
fn has_hw_mb_sync(fans_type: &[u8; 4]) -> bool {
    let first = fans_type.iter().find(|&&ft| ft != 0).copied().unwrap_or(0);
    (20..=26).contains(&first) || (41..=42).contains(&first)
}

pub fn identify_device_type(dev_type: u8, fans_type: &[u8; 4], specs: &SpecRegistry) -> DeviceType {
    // Synthetic dev_type 0xFD identifies AURA RGB channels — they're created
    // by the AuraBackend and never appear in devices.toml.
    if dev_type == 0xFD {
        return DeviceType::Aura;
    }
    if dev_type == 0 {
        specs
            .lookup_fans_type(fans_type[0])
            .and_then(|spec| spec.device_type)
            .unwrap_or(DeviceType::Unknown)
    } else {
        specs
            .lookup_dev_type(dev_type)
            .and_then(|spec| spec.device_type)
            .unwrap_or(DeviceType::Unknown)
    }
}

/// Derive a display name for an LCD device from its DeviceId (which encodes VID:PID).
fn lcd_name_from_id(id: &DeviceId, group: GroupId) -> String {
    use frgb_model::usb_ids::*;
    let bytes = id.as_bytes();
    let pid = u16::from_le_bytes([bytes[2], bytes[3]]);
    match pid {
        PID_SL_LCD => format!("SL-LCD Wireless {}", group.value().saturating_sub(99)),
        PID_TLV2_LCD => format!("TL V2 LCD {}", group.value().saturating_sub(99)),
        PID_HYDROSHIFT_CIRCLE => "HydroShift II Circle".into(),
        PID_HYDROSHIFT_SQUARE => "HydroShift II Square".into(),
        PID_UNIVERSAL_88 => "Universal 8.8\"".into(),
        _ => format!("LCD {group}"),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_model::spec_loader::load_defaults;

    fn mock_discovered(
        group: u8,
        mac: [u8; 6],
        master: [u8; 6],
        fans_type: [u8; 4],
        fan_count: u8,
    ) -> DiscoveredDevice {
        DiscoveredDevice {
            id: DeviceId::from(mac),
            fans_type,
            dev_type: 0,
            group: GroupId::new(group),
            fan_count,
            master: DeviceId::from(master),
            fans_rpm: [1200, 1100, 1000, 0],
            fans_pwm: [0; 4],
            cmd_seq: 0,
            channel: 0x08,
        }
    }

    #[test]
    fn merge_creates_new_device() {
        let specs = load_defaults();
        let mut reg = DeviceRegistry::new();
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);

        let discovered = vec![mock_discovered(
            1,
            [0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1],
            *our_mac.as_bytes(),
            [21, 21, 21, 0],
            3,
        )];

        let unbound = reg.refresh(BackendId(0), discovered, our_mac, &specs);
        assert!(unbound.is_empty());
        assert_eq!(reg.devices().len(), 1);

        let dev = &reg.devices()[0];
        assert_eq!(dev.group, GroupId::new(1));
        assert_eq!(dev.fan_count(), 3);
        assert_eq!(dev.device_type, DeviceType::SlWireless);
        assert_eq!(dev.blade, BladeType::Reverse); // ft=21 is reverse
    }

    #[test]
    fn merge_detects_unbound() {
        let specs = load_defaults();
        let mut reg = DeviceRegistry::new();
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);

        let discovered = vec![mock_discovered(
            254,
            [0xab, 0x1b, 0x1f, 0xe5, 0x66, 0xe1],
            [0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // different master
            [42, 41, 0, 0],
            2,
        )];

        let unbound = reg.refresh(BackendId(0), discovered, our_mac, &specs);
        assert_eq!(unbound.len(), 1);
        assert!(reg.devices().is_empty());
    }

    #[test]
    fn merge_adds_mac_to_existing_group() {
        let specs = load_defaults();
        let mut reg = DeviceRegistry::new();
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);

        let disc1 = mock_discovered(
            1,
            [0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1],
            *our_mac.as_bytes(),
            [21, 21, 21, 0],
            3,
        );
        let disc2 = mock_discovered(
            1,
            [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
            *our_mac.as_bytes(),
            [21, 21, 21, 0],
            3,
        );

        reg.refresh(BackendId(0), vec![disc1], our_mac, &specs);
        reg.refresh(BackendId(0), vec![disc2], our_mac, &specs);

        assert_eq!(reg.devices().len(), 1);
        assert_eq!(reg.devices()[0].mac_ids.len(), 2);
    }

    #[test]
    fn find_by_group() {
        let specs = load_defaults();
        let mut reg = DeviceRegistry::new();
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);

        let disc = mock_discovered(
            3,
            [0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1],
            *our_mac.as_bytes(),
            [42, 42, 0, 0],
            2,
        );
        reg.refresh(BackendId(0), vec![disc], our_mac, &specs);

        assert!(reg.find_by_group(GroupId::new(3)).is_some());
        assert!(reg.find_by_group(GroupId::new(1)).is_none());
    }

    #[test]
    fn identify_device_type_from_spec() {
        let specs = load_defaults();
        assert_eq!(identify_device_type(0, &[21, 21, 0, 0], &specs), DeviceType::SlWireless);
        assert_eq!(identify_device_type(0, &[42, 41, 0, 0], &specs), DeviceType::ClWireless);
        assert_eq!(
            identify_device_type(0, &[36, 0, 0, 0], &specs),
            DeviceType::SlInfWireless
        );
        assert_eq!(identify_device_type(10, &[0, 0, 0, 0], &specs), DeviceType::WaterBlock);
        assert_eq!(identify_device_type(0xFF, &[0, 0, 0, 0], &specs), DeviceType::Unknown);
    }

    #[test]
    fn device_fans_type_array() {
        let dev = Device {
            id: DeviceId::ZERO,
            backend_id: BackendId(0),
            group: GroupId::new(1),
            slots: vec![
                DeviceSlot {
                    fans_type: 21,
                    rpm: 1200,
                    has_lcd: false,
                    source_idx: 0,
                },
                DeviceSlot {
                    fans_type: 21,
                    rpm: 1100,
                    has_lcd: false,
                    source_idx: 1,
                },
                DeviceSlot {
                    fans_type: 21,
                    rpm: 1000,
                    has_lcd: false,
                    source_idx: 2,
                },
            ],
            state: DeviceState::default(),
            mac_ids: vec![],
            tx_ref: DeviceId::ZERO,
            name: "Test".into(),
            device_type: DeviceType::SlWireless,
            role: FanRole::Intake,
            blade: BladeType::Standard,
            mb_sync: false,
            cmd_seq: 0,
        };
        assert_eq!(dev.fans_type(), [21, 21, 21, 0]);
        assert_eq!(dev.fans_rpm(), [1200, 1100, 1000, 0]);
    }

    #[test]
    fn update_state() {
        let specs = load_defaults();
        let mut reg = DeviceRegistry::new();
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);

        let disc = mock_discovered(
            1,
            [0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1],
            *our_mac.as_bytes(),
            [21, 21, 21, 0],
            3,
        );
        reg.refresh(BackendId(0), vec![disc], our_mac, &specs);

        reg.update_state(GroupId::new(1), |s| s.speed_percent = Some(SpeedPercent::new(70)));
        assert_eq!(
            reg.find_by_group(GroupId::new(1)).unwrap().state.speed_percent,
            Some(SpeedPercent::new(70))
        );
    }

    #[test]
    fn refresh_preserves_state() {
        let specs = load_defaults();
        let mut reg = DeviceRegistry::new();
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);

        // First discovery
        let disc = mock_discovered(
            1,
            [0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1],
            *our_mac.as_bytes(),
            [21, 21, 21, 0],
            3,
        );
        reg.refresh(BackendId(0), vec![disc], our_mac, &specs);

        // Set state
        reg.update_state(GroupId::new(1), |s| {
            s.speed_percent = Some(SpeedPercent::new(70));
            s.rgb_mode = Some(RgbMode::Off);
        });

        // Second discovery — same device, updated RPMs
        let disc2 = DiscoveredDevice {
            fans_rpm: [1500, 1400, 1300, 0],
            ..mock_discovered(
                1,
                [0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1],
                *our_mac.as_bytes(),
                [21, 21, 21, 0],
                3,
            )
        };
        reg.refresh(BackendId(0), vec![disc2], our_mac, &specs);

        // State must survive refresh
        let dev = reg.find_by_group(GroupId::new(1)).unwrap();
        assert_eq!(
            dev.state.speed_percent,
            Some(SpeedPercent::new(70)),
            "speed_percent lost across refresh"
        );
        assert_eq!(dev.state.rgb_mode, Some(RgbMode::Off), "rgb_mode lost across refresh");
        // RPMs should be updated
        assert_eq!(dev.slots[0].rpm, 1500);
    }

    #[test]
    fn refresh_removes_disconnected() {
        let specs = load_defaults();
        let mut reg = DeviceRegistry::new();
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);

        let disc1 = mock_discovered(
            1,
            [0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1],
            *our_mac.as_bytes(),
            [21, 21, 21, 0],
            3,
        );
        let disc2 = mock_discovered(
            2,
            [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
            *our_mac.as_bytes(),
            [42, 41, 0, 0],
            2,
        );
        reg.refresh(BackendId(0), vec![disc1.clone(), disc2], our_mac, &specs);
        assert_eq!(reg.devices().len(), 2);

        // Next refresh only sees group 1 — group 2 disconnected
        reg.refresh(BackendId(0), vec![disc1], our_mac, &specs);
        assert_eq!(reg.devices().len(), 1);
        assert!(reg.find_by_group(GroupId::new(1)).is_some());
        assert!(reg.find_by_group(GroupId::new(2)).is_none());
    }

    /// New devices should derive their initial speed_percent from the
    /// hardware-reported fans_pwm bytes so the GUI shows the actual running
    /// speed instead of "unknown".
    #[test]
    fn refresh_seeds_speed_percent_from_fans_pwm() {
        let specs = load_defaults();
        let mut reg = DeviceRegistry::new();
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);

        // SLV3 fan running at ~50% (mid PWM byte). Use 132 which is roughly
        // halfway between SPEED_MIN (6) and SPEED_MAX (255).
        let disc = DiscoveredDevice {
            fans_pwm: [132, 132, 132, 0],
            ..mock_discovered(
                1,
                [0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1],
                *our_mac.as_bytes(),
                [21, 21, 21, 0],
                3,
            )
        };
        reg.refresh(BackendId(0), vec![disc], our_mac, &specs);

        let dev = reg.find_by_group(GroupId::new(1)).expect("group 1");
        let pct = dev
            .state
            .speed_percent
            .expect("speed_percent should be derived from PWM")
            .value();
        // Roughly 50% — allow ±2% slack for integer rounding.
        assert!((45..=55).contains(&pct), "expected ~50%, got {pct}");
        assert!(!dev.mb_sync, "active fan should not be flagged as MB sync");
    }

    /// MB-sync devices report fans_pwm = [6, 6, ...] as a sentinel, not as a
    /// real duty. They must remain "unknown speed" so the UI can show the MB
    /// sync indicator instead of a bogus 0%.
    #[test]
    fn refresh_skips_speed_seed_for_mb_sync() {
        let specs = load_defaults();
        let mut reg = DeviceRegistry::new();
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);

        let disc = DiscoveredDevice {
            fans_pwm: [6, 6, 6, 0], // sentinel for MB-sync mode
            ..mock_discovered(
                1,
                [0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1],
                *our_mac.as_bytes(),
                [21, 21, 21, 0],
                3,
            )
        };
        reg.refresh(BackendId(0), vec![disc], our_mac, &specs);

        let dev = reg.find_by_group(GroupId::new(1)).expect("group 1");
        assert!(dev.mb_sync, "SLV3 with all-PWM=6 should be detected as MB sync");
        assert_eq!(
            dev.state.speed_percent, None,
            "mb-sync devices must not derive a fake speed"
        );
    }

    /// If the very first discovery returned an empty fans_pwm (the radio
    /// hadn't latched yet), the registry should backfill speed_percent from
    /// a later discovery that carries valid PWM bytes.
    #[test]
    fn refresh_backfills_speed_percent_when_first_discovery_was_empty() {
        let specs = load_defaults();
        let mut reg = DeviceRegistry::new();
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);

        // First discovery: pwm all zero (stale RX). state.speed_percent ends up None.
        let mac = [0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1];
        let disc1 = DiscoveredDevice {
            fans_pwm: [0, 0, 0, 0],
            ..mock_discovered(1, mac, *our_mac.as_bytes(), [21, 21, 21, 0], 3)
        };
        reg.refresh(BackendId(0), vec![disc1], our_mac, &specs);
        assert_eq!(reg.find_by_group(GroupId::new(1)).unwrap().state.speed_percent, None);

        // Second discovery: real PWM data arrives.
        let disc2 = DiscoveredDevice {
            fans_pwm: [200, 200, 200, 0],
            ..mock_discovered(1, mac, *our_mac.as_bytes(), [21, 21, 21, 0], 3)
        };
        reg.refresh(BackendId(0), vec![disc2], our_mac, &specs);

        let dev = reg.find_by_group(GroupId::new(1)).unwrap();
        let pct = dev
            .state
            .speed_percent
            .expect("backfill should populate speed_percent")
            .value();
        assert!(pct > 50, "expected high pct from PWM=200, got {pct}");
    }

    /// CL fans with PWM=6 (the SPEED_MIN sentinel) are correctly detected as
    /// MB-sync active. Speed_percent must remain None — the sentinel is not a
    /// real duty cycle. This also verifies CL is now in has_hw_mb_sync.
    #[test]
    fn refresh_cl_fan_with_pwm_sentinel_is_mb_sync() {
        let specs = load_defaults();
        let mut reg = DeviceRegistry::new();
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);

        // CL fan (fans_type 41 = ClWireless), running at MB-sync (PWM=6 sentinel).
        let disc = DiscoveredDevice {
            fans_pwm: [6, 0, 0, 0],
            fans_rpm: [2105, 0, 0, 0],
            ..mock_discovered(
                2,
                [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
                *our_mac.as_bytes(),
                [41, 0, 0, 0],
                1,
            )
        };
        reg.refresh(BackendId(0), vec![disc], our_mac, &specs);

        let dev = reg.find_by_group(GroupId::new(2)).expect("group 2");
        assert!(dev.mb_sync, "CL with PWM=6 should be detected as MB sync");
        assert_eq!(
            dev.state.speed_percent, None,
            "MB-sync devices must not seed speed_percent"
        );
    }

    /// CL fans running at a real duty (not the PWM=6 sentinel) should NOT
    /// seed speed_percent because CL doesn't use the SLV3 PWM byte format.
    /// The fans_type 41-42 range now passes has_hw_mb_sync but the seeding
    /// guard also requires has_hw_mb_sync — which means CL with a non-sentinel
    /// PWM byte would try to seed. Verify this path produces a sensible value
    /// rather than the bogus 0% we had before.
    #[test]
    fn refresh_cl_fan_with_real_pwm_seeds_speed() {
        let specs = load_defaults();
        let mut reg = DeviceRegistry::new();
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);

        // CL fan running at ~80% duty (PWM byte 200, not the sentinel).
        let disc = DiscoveredDevice {
            fans_pwm: [200, 0, 0, 0],
            fans_rpm: [1800, 0, 0, 0],
            ..mock_discovered(
                2,
                [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
                *our_mac.as_bytes(),
                [41, 0, 0, 0],
                1,
            )
        };
        reg.refresh(BackendId(0), vec![disc], our_mac, &specs);

        let dev = reg.find_by_group(GroupId::new(2)).expect("group 2");
        assert!(!dev.mb_sync, "CL with real PWM should not be MB sync");
        let pct = dev.state.speed_percent.expect("CL with real PWM should seed").value();
        assert!(pct > 50, "expected high pct from PWM=200, got {pct}");
    }

    /// AIO pump devices (WaterBlock dev_type=10/11) don't have a fan PWM at
    /// all — pump speed lives in aio_param[28..30] over RF, not in fans_pwm.
    /// Seeding from fans_pwm[0] produces a bogus value (28% in user reports).
    #[test]
    fn refresh_skips_speed_seed_for_aio_pump() {
        let specs = load_defaults();
        let mut reg = DeviceRegistry::new();
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);

        // WaterBlock pump (dev_type=10) with non-zero fans_pwm[0]
        let disc = DiscoveredDevice {
            id: DeviceId::from([0x11, 0x22, 0x33, 0x44, 0x55, 0x66]),
            fans_type: [0, 0, 0, 0],
            dev_type: 10,
            group: GroupId::new(7),
            fan_count: 1,
            master: our_mac,
            fans_rpm: [0, 0, 0, 1650],
            fans_pwm: [76, 0, 0, 0], // bogus byte that scales to ~28%
            cmd_seq: 0,
            channel: 0x08,
        };
        reg.refresh(BackendId(0), vec![disc], our_mac, &specs);

        let dev = reg.find_by_group(GroupId::new(7)).expect("group 7");
        assert_eq!(dev.device_type, DeviceType::WaterBlock);
        assert_eq!(
            dev.state.speed_percent, None,
            "AIO pump must not seed speed_percent from a fan PWM byte"
        );
    }

    /// Real-world HydroShift pump firmware places an SLV3 byte (26) in
    /// `fans_type[3]` — this would trip `has_hw_mb_sync` if the seeding
    /// guard relied on fans_type alone. Guard must check `dev_type == 0`.
    #[test]
    fn refresh_skips_speed_seed_for_pump_with_slv3_byte_in_fans_type() {
        let specs = load_defaults();
        let mut reg = DeviceRegistry::new();
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);

        // WaterBlock pump reporting fans_type=[0,0,0,26] as real firmware does.
        // fans_pwm[3]=36 would seed to 28% if the dev_type guard is missing.
        let disc = DiscoveredDevice {
            id: DeviceId::from([0x11, 0x22, 0x33, 0x44, 0x55, 0x66]),
            fans_type: [0, 0, 0, 26],
            dev_type: 10,
            group: GroupId::new(7),
            fan_count: 1,
            master: our_mac,
            fans_rpm: [0, 0, 0, 1678],
            fans_pwm: [0, 0, 0, 36],
            cmd_seq: 0,
            channel: 0x08,
        };
        reg.refresh(BackendId(0), vec![disc], our_mac, &specs);

        let dev = reg.find_by_group(GroupId::new(7)).expect("group 7");
        assert_eq!(dev.device_type, DeviceType::WaterBlock);
        assert_eq!(
            dev.state.speed_percent, None,
            "pump with fans_type[3]=26 must NOT seed speed_percent — the SLV3 byte \
             is a firmware encoding artifact, not a real PWM duty cycle"
        );
    }

    /// AIO pump RPM persists across multiple discovery cycles.
    ///
    /// The Lian Li RF packet places pump telemetry at fans_rpm[3]. After
    /// filtering empty slots, Device::slots[0] holds the pump data but
    /// originated from discovery index 3. Subsequent discoveries must read
    /// from the original source index (3), not the current slot position (0),
    /// otherwise the RPM silently resets to 0 on every refresh.
    #[test]
    fn pump_rpm_persists_across_refresh() {
        let specs = load_defaults();
        let mut reg = DeviceRegistry::new();
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);

        // First discovery: WaterBlock pump reports 1822 RPM at slot 3.
        let disc1 = DiscoveredDevice {
            id: DeviceId::from([0x11, 0x22, 0x33, 0x44, 0x55, 0x66]),
            fans_type: [0, 0, 0, 0],
            dev_type: 10,
            group: GroupId::new(7),
            fan_count: 1,
            master: our_mac,
            fans_rpm: [0, 0, 0, 1822],
            fans_pwm: [0, 0, 0, 0],
            cmd_seq: 0,
            channel: 0x08,
        };
        reg.refresh(BackendId(0), vec![disc1], our_mac, &specs);

        let dev = reg.find_by_group(GroupId::new(7)).expect("group 7");
        assert_eq!(dev.slots.len(), 1, "pump produces one filtered slot");
        assert_eq!(dev.slots[0].source_idx, 3, "pump data came from discovery index 3");
        assert_eq!(dev.slots[0].rpm, 1822);

        // Second discovery: same pump now reports 1900 RPM at slot 3.
        // The update path must read from source_idx 3, not slot position 0.
        let disc2 = DiscoveredDevice {
            id: DeviceId::from([0x11, 0x22, 0x33, 0x44, 0x55, 0x66]),
            fans_type: [0, 0, 0, 0],
            dev_type: 10,
            group: GroupId::new(7),
            fan_count: 1,
            master: our_mac,
            fans_rpm: [0, 0, 0, 1900],
            fans_pwm: [0, 0, 0, 0],
            cmd_seq: 1,
            channel: 0x08,
        };
        reg.refresh(BackendId(0), vec![disc2], our_mac, &specs);

        let dev = reg.find_by_group(GroupId::new(7)).expect("group 7");
        assert_eq!(
            dev.slots[0].rpm, 1900,
            "pump RPM must update from fans_rpm[source_idx=3], not fans_rpm[0]"
        );
    }

    /// User-set speed values must NOT be clobbered by hardware-derived
    /// backfill on subsequent discoveries.
    #[test]
    fn refresh_does_not_overwrite_user_speed() {
        let specs = load_defaults();
        let mut reg = DeviceRegistry::new();
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);

        let mac = [0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1];
        let disc1 = DiscoveredDevice {
            fans_pwm: [132, 132, 132, 0],
            ..mock_discovered(1, mac, *our_mac.as_bytes(), [21, 21, 21, 0], 3)
        };
        reg.refresh(BackendId(0), vec![disc1], our_mac, &specs);

        // User commands a specific speed.
        reg.update_state(GroupId::new(1), |s| s.speed_percent = Some(SpeedPercent::new(72)));

        // Next discovery still carries the old PWM byte; backfill must not run
        // because state.speed_percent is already Some.
        let disc2 = DiscoveredDevice {
            fans_pwm: [60, 60, 60, 0], // different PWM, but state should win
            ..mock_discovered(1, mac, *our_mac.as_bytes(), [21, 21, 21, 0], 3)
        };
        reg.refresh(BackendId(0), vec![disc2], our_mac, &specs);

        assert_eq!(
            reg.find_by_group(GroupId::new(1)).unwrap().state.speed_percent,
            Some(SpeedPercent::new(72))
        );
    }

    #[test]
    fn lcd_name_includes_group_number() {
        use frgb_model::usb_ids::*;

        // lcd_name_from_id reads pid as u16::from_le_bytes([bytes[2], bytes[3]]).
        // Construct DeviceId with bytes[2..3] encoding the PID in little-endian.
        fn make_lcd_id(pid: u16) -> DeviceId {
            let vid = VID_LCD;
            DeviceId::from([
                (vid & 0xFF) as u8,
                (vid >> 8) as u8,
                (pid & 0xFF) as u8,
                (pid >> 8) as u8,
                0,
                0,
            ])
        }

        // SL-LCD with group 100: name should contain "1" (100 - 99 = 1)
        let name = lcd_name_from_id(&make_lcd_id(PID_SL_LCD), GroupId::new(100));
        assert_eq!(name, "SL-LCD Wireless 1");

        // SL-LCD with group 102: name should contain "3"
        let name = lcd_name_from_id(&make_lcd_id(PID_SL_LCD), GroupId::new(102));
        assert_eq!(name, "SL-LCD Wireless 3");

        // TLV2 LCD with group 99: saturating_sub(99) = 0
        let name = lcd_name_from_id(&make_lcd_id(PID_TLV2_LCD), GroupId::new(99));
        assert_eq!(name, "TL V2 LCD 0");

        // HydroShift Circle: no group number in name
        let name = lcd_name_from_id(&make_lcd_id(PID_HYDROSHIFT_CIRCLE), GroupId::new(100));
        assert_eq!(name, "HydroShift II Circle");

        // Unknown PID: fallback includes group Display
        let name = lcd_name_from_id(&make_lcd_id(0xFFFF), GroupId::new(5));
        assert_eq!(name, "LCD 5");
    }

    #[test]
    fn has_hw_mb_sync_includes_cl_fans() {
        assert!(has_hw_mb_sync(&[21, 21, 21, 0])); // SLV3
        assert!(has_hw_mb_sync(&[41, 0, 0, 0])); // CL
        assert!(has_hw_mb_sync(&[42, 0, 0, 0])); // CL-R
        assert!(!has_hw_mb_sync(&[40, 0, 0, 0])); // RL120 — no mobo sync
        assert!(!has_hw_mb_sync(&[28, 28, 0, 0])); // TLV2 — no mobo sync
        assert!(!has_hw_mb_sync(&[36, 0, 0, 0])); // SL-INF — no mobo sync
    }
}
