use std::any::Any;
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use frgb_model::device::DeviceId;
use frgb_model::GroupId;
use frgb_protocol::color::percent_to_speed_byte;
use frgb_protocol::encode::{self, RgbMetadata, RF_DATA_CHUNK_SIZE};
use frgb_rgb::compression::encoder::tuz_compress;
use frgb_rgb::generator::EffectResult;

use crate::backend::{Backend, BackendId, DiscoveredDevice, LianLiRfExt, SpeedCommand};
use crate::error::{CoreError, Result};
use crate::registry::Device;
use crate::sequencer;
use crate::session;
use crate::transport::Transport;

/// Default wireless channel.
const DEFAULT_CHANNEL: u8 = 0x08;

/// Cooldown between reconnect attempts on TX/RX transports.
const RF_RECOVERY_COOLDOWN: Duration = Duration::from_secs(5);

/// Lian Li wireless RF backend — handles TX/RX USB communication,
/// device discovery, speed commands, RGB buffer transmission, and bind protocol.
pub struct LianLiRfBackend<T: Transport> {
    backend_id: BackendId,
    tx: RefCell<T>,
    rx: RefCell<T>,
    channel: u8,
    channel_override: bool,
    tx_id: Option<DeviceId>,
    tx_firmware: Option<u16>,
    tx_cooldown: Cell<Option<Instant>>,
    rx_cooldown: Cell<Option<Instant>>,
}

impl<T: Transport> LianLiRfBackend<T> {
    pub fn new(tx: T, rx: T, channel_override: Option<u8>) -> Self {
        Self::with_id(BackendId(0), tx, rx, channel_override)
    }

    pub fn with_id(id: BackendId, tx: T, rx: T, channel_override: Option<u8>) -> Self {
        Self {
            backend_id: id,
            tx: RefCell::new(tx),
            rx: RefCell::new(rx),
            channel: channel_override.unwrap_or(DEFAULT_CHANNEL),
            channel_override: channel_override.is_some(),
            tx_id: None,
            tx_firmware: None,
            tx_cooldown: Cell::new(None),
            rx_cooldown: Cell::new(None),
        }
    }

    pub fn tx(&self) -> std::cell::Ref<'_, T> {
        self.tx.borrow()
    }
    pub fn rx(&self) -> std::cell::Ref<'_, T> {
        self.rx.borrow()
    }
    pub fn tx_firmware(&self) -> Option<u16> {
        self.tx_firmware
    }

    /// Set TX device ID (for testing — normally set during discover).
    #[cfg(test)]
    pub fn set_tx_id(&mut self, id: DeviceId) {
        self.tx_id = Some(id);
    }

    /// Run a TX-side logical op; on failure, attempt a transport reconnect
    /// (cooldown-limited to 5s) and retry the whole op once. The op must be
    /// idempotent at the protocol level — any state established by prior
    /// writes inside `op` will be re-sent on retry.
    ///
    /// When the cooldown blocks a reopen, returns `Err(e)` immediately —
    /// matching the semantics of `frgb_usb::recovery::with_recovery` and
    /// `hwmon_backend::recovery::with_recovery`.
    fn with_tx_recovery<F, R>(&self, op: F) -> Result<R>
    where
        F: Fn(&T) -> Result<R>,
    {
        let first = op(&self.tx.borrow());
        match first {
            Ok(v) => Ok(v),
            Err(e) => {
                if let Some(last) = self.tx_cooldown.get() {
                    if last.elapsed() < RF_RECOVERY_COOLDOWN {
                        return Err(e);
                    }
                }
                self.tx_cooldown.set(Some(Instant::now()));
                tracing::warn!("TX op failed ({e}); reconnecting");
                if let Err(re) = self.tx.borrow_mut().reconnect() {
                    tracing::warn!("TX reconnect failed ({re}); returning original error");
                    return Err(e);
                }
                tracing::info!("TX reconnected, retrying");
                op(&self.tx.borrow())
            }
        }
    }

    /// RX-side companion to with_tx_recovery. Same semantics.
    ///
    /// When the cooldown blocks a reopen, returns `Err(e)` immediately —
    /// matching the semantics of `frgb_usb::recovery::with_recovery` and
    /// `hwmon_backend::recovery::with_recovery`.
    fn with_rx_recovery<F, R>(&self, op: F) -> Result<R>
    where
        F: Fn(&T) -> Result<R>,
    {
        let first = op(&self.rx.borrow());
        match first {
            Ok(v) => Ok(v),
            Err(e) => {
                if let Some(last) = self.rx_cooldown.get() {
                    if last.elapsed() < RF_RECOVERY_COOLDOWN {
                        return Err(e);
                    }
                }
                self.rx_cooldown.set(Some(Instant::now()));
                tracing::warn!("RX op failed ({e}); reconnecting");
                if let Err(re) = self.rx.borrow_mut().reconnect() {
                    tracing::warn!("RX reconnect failed ({re}); returning original error");
                    return Err(e);
                }
                tracing::info!("RX reconnected, retrying");
                op(&self.rx.borrow())
            }
        }
    }

    /// Send a pre-composed RGB buffer through the TUZ pipeline for a device.
    fn send_rgb_buffer(&self, device: &Device, effect: &EffectResult) -> Result<()> {
        let fan_id = device
            .mac_ids
            .first()
            .ok_or_else(|| CoreError::InvalidInput(format!("group {} has no fan IDs", device.group)))?;

        let led_count = effect.buffer.led_count();
        let led_num = u8::try_from(led_count)
            .map_err(|_| CoreError::InvalidInput(format!("LED count {led_count} exceeds 255")))?;

        let rgb_data = effect.buffer.flatten();
        let compressed =
            tuz_compress(&rgb_data).map_err(|e| CoreError::Protocol(format!("TUZ compression failed: {e}")))?;

        let effect_index = session::generate_effect_index();
        let data_parts = compressed.len().div_ceil(RF_DATA_CHUNK_SIZE);
        if data_parts + 1 > 255 {
            return Err(CoreError::Protocol("compressed data too large for RF framing".into()));
        }
        let total_parts = (data_parts + 1) as u8;
        let total_frame = effect.frame_count as u16;

        let part0 = encode::encode_rgb_metadata_payload(
            fan_id,
            &device.tx_ref,
            &effect_index,
            &RgbMetadata {
                total_parts,
                compressed_data_len: compressed.len() as u32,
                total_frame,
                led_num,
                interval: effect.interval_ms as f64,
                sub_interval: 0.0,
                is_outer_match_max: 0,
                total_sub_frame: 0,
            },
        );

        // Pre-encode all data chunks outside the closure — encoding is pure/cheap and
        // avoids re-compressing on retry.
        let data_parts: Vec<[u8; 240]> = compressed
            .chunks(RF_DATA_CHUNK_SIZE)
            .enumerate()
            .map(|(i, chunk)| {
                encode::encode_rgb_data_payload(fan_id, &device.tx_ref, &effect_index, (i + 1) as u8, total_parts, chunk)
            })
            .collect();

        let group_raw = device.group.value();
        self.with_tx_recovery(|tx| {
            sequencer::send_rf_data(tx, self.channel, group_raw, &part0)?;
            for _ in 0..3 {
                tx.sleep(sequencer::DELAY_RF_REPEAT);
                sequencer::send_rf_data(tx, self.channel, group_raw, &part0)?;
            }

            for part in &data_parts {
                sequencer::send_rf_data(tx, self.channel, group_raw, part)?;
            }

            tx.sleep(Duration::from_millis(10));
            Ok(())
        })
    }

    /// Build per-slot PWM array: speed_byte for occupied slots, 0 for unoccupied.
    /// Firmware rejects speed commands with non-zero PWM in unoccupied slots.
    fn build_fans_pwm(speed_byte: u8, fan_count: u8) -> [u8; 4] {
        let mut pwm = [0u8; 4];
        for slot in pwm.iter_mut().take(fan_count.min(4) as usize) {
            *slot = speed_byte;
        }
        pwm
    }

    /// Probe all valid RF channels for devices when the default channel returns nothing.
    ///
    /// Iterates `VALID_CHANNELS`, performing a TX sync + device query on each channel
    /// (skipping the one already tried). Updates `self.channel` and `self.tx_id` on
    /// success. Returns `Ok(())` in all cases — a warning is logged if nothing is found.
    pub(crate) fn scan_channels(&mut self) -> Result<()> {
        use frgb_protocol::constants::VALID_CHANNELS;

        let already_tried = self.channel;

        for &ch in VALID_CHANNELS.iter() {
            if ch == already_tried {
                continue;
            }

            let tx = self.tx.borrow();
            let sync = match crate::discovery::discover_tx(&*tx, ch) {
                Ok(s) => s,
                Err(_) => continue,
            };
            drop(tx);

            // TX sync succeeded — query devices on this channel.
            let response = self.with_rx_recovery(|rx| crate::discovery::discover_devices(rx, 1))?;

            if !response.records.is_empty() {
                tracing::info!(
                    "RF scan: found {} device(s) on channel 0x{:02X}",
                    response.records.len(),
                    ch
                );
                self.channel = ch;
                self.tx_id = Some(sync.tx_device_id);
                self.tx_firmware = Some(sync.firmware_version);
                return Ok(());
            }
        }

        tracing::warn!("RF scan: no devices found on any channel");
        Ok(())
    }

    /// Query RX dongle status for protocol sync between RGB send rounds.
    /// Sends a status query to RX, waits, then drains response packets.
    fn query_rx_status(&self) {
        let rx = self.rx.borrow();
        // Build a minimal status query packet
        let mut pkt = [0u8; 64];
        pkt[0] = 0x10;
        pkt[1] = 0x01;
        pkt[2] = self.channel;
        let _ = rx.write(&pkt);
        rx.sleep(sequencer::DELAY_SYNC);
        // Drain responses
        for _ in 0..10 {
            if rx.read(Duration::from_millis(5)).is_err() {
                break;
            }
        }
    }

    /// Wake the RX dongle with a burst of status queries.
    /// Required before first discovery — without this, the RX may not respond.
    /// Pattern: 5 queries, 10ms apart, then drain responses.
    fn wake_rx(&self) {
        let rx = self.rx.borrow();
        let mut pkt = [0u8; 64];
        pkt[0] = 0x10;
        pkt[1] = 0x01;
        pkt[2] = self.channel;
        for _ in 0..5 {
            let _ = rx.write(&pkt);
            rx.sleep(Duration::from_millis(10));
        }
        // Drain responses
        for _ in 0..32 {
            if rx.read(Duration::from_millis(1)).is_err() {
                break;
            }
        }
    }

    fn get_tx_ref(&self) -> Result<DeviceId> {
        self.tx_id
            .ok_or_else(|| CoreError::NotFound("no TX reference ID available (run discover first)".into()))
    }

    fn set_speed_inner(&self, device: &Device, cmd: &SpeedCommand) -> Result<()> {
        let group_raw = device.group.value();
        // Pre-compute payload outside the closure so it is prepared only once.
        let payload: [u8; 240] = match cmd {
            SpeedCommand::Pwm => {
                let fan_id = match device.mac_ids.first() {
                    Some(id) => id,
                    None => return Ok(()),
                };
                let pwm = Self::build_fans_pwm(frgb_protocol::constants::SPEED_MIN, device.fan_count());
                encode::encode_bind_rf_payload(fan_id, &device.tx_ref, group_raw, self.channel, 1, &pwm)
            }
            SpeedCommand::Manual(percent) => {
                let speed_byte = percent_to_speed_byte(*percent);
                let fan_id = device
                    .mac_ids
                    .first()
                    .ok_or_else(|| CoreError::InvalidInput(format!("group {} has no fan IDs", device.group)))?;
                let pwm = Self::build_fans_pwm(speed_byte, device.fan_count());
                encode::encode_bind_rf_payload(fan_id, &device.tx_ref, group_raw, self.channel, 1, &pwm)
            }
        };
        self.with_tx_recovery(|tx| {
            // Drain stale data from TX buffer before sending speed commands.
            for _ in 0..32 {
                if tx.read(Duration::from_millis(1)).is_err() {
                    break;
                }
            }
            sequencer::send_rf_data(tx, self.channel, group_raw, &payload)?;
            for _ in 0..2 {
                tx.sleep(sequencer::DELAY_RF_REPEAT);
                sequencer::send_rf_data(tx, self.channel, group_raw, &payload)?;
            }
            Ok(())
        })
    }
}

impl<T: Transport + 'static> Backend for LianLiRfBackend<T> {
    fn id(&self) -> BackendId {
        self.backend_id
    }
    fn name(&self) -> &str {
        "lianli-rf"
    }

    fn discover(&mut self) -> Result<Vec<DiscoveredDevice>> {
        self.wake_rx();

        // Step 1: TX sync — retry to handle stale endpoint data after USB open.
        // First attempt may read stale data; subsequent attempts get fresh response.
        // with_tx_recovery handles transport-level recovery; the inner loop handles
        // protocol-level retries (stale endpoint data).
        let sync = self.with_tx_recovery(|tx| {
            let mut sync_result = None;
            for attempt in 0..3u8 {
                // Drain any stale data before each attempt
                for _ in 0..32 {
                    if tx.read(Duration::from_millis(1)).is_err() {
                        break;
                    }
                }
                match crate::discovery::discover_tx(tx, self.channel) {
                    Ok(s) => {
                        sync_result = Some(s);
                        break;
                    }
                    Err(_) if attempt < 2 => {
                        continue;
                    }
                    Err(e) => return Err(e),
                }
            }
            sync_result.ok_or_else(|| CoreError::Protocol("TX sync failed after retries".into()))
        })?;
        self.tx_id = Some(sync.tx_device_id);
        self.tx_firmware = Some(sync.firmware_version);

        // Step 2: Device query on RX — retry up to 3 times
        let mut all_discovered = Vec::new();
        let mut seen_macs = std::collections::HashSet::new();

        for attempt in 0..3u8 {
            if attempt > 0 {
                let rx = self.rx.borrow();
                for _ in 0..32 {
                    if rx.read(Duration::from_millis(5)).is_err() {
                        break;
                    }
                }
            }
            let response = self.with_rx_recovery(|rx| crate::discovery::discover_devices(rx, 1))?;
            if response.records.is_empty() {
                continue;
            }

            let mut channel_set = false;
            for record in &response.records {
                if seen_macs.contains(&record.mac_addr) {
                    continue;
                }
                seen_macs.insert(record.mac_addr);

                // Auto-detect channel from first bound device
                if !channel_set && !self.channel_override && record.master_mac_addr == sync.tx_device_id {
                    self.channel = record.channel;
                    channel_set = true;
                }

                all_discovered.push(DiscoveredDevice {
                    id: record.mac_addr,
                    fans_type: record.fans_type,
                    dev_type: record.dev_type,
                    group: GroupId::new(record.group),
                    fan_count: record.fan_num.max(1),
                    master: record.master_mac_addr,
                    fans_rpm: record.fans_speed,
                    fans_pwm: record.fans_pwm,
                    cmd_seq: record.cmd_seq,
                    channel: record.channel,
                });
            }

            if seen_macs.len() >= 10 {
                break;
            }
        }

        // If no devices found and no channel override, scan all channels.
        if all_discovered.is_empty() && !self.channel_override {
            let channel_before = self.channel;
            self.scan_channels()?;

            // If scan_channels found a new channel, do one more device query pass.
            if self.channel != channel_before {
                let response = self.with_rx_recovery(|rx| crate::discovery::discover_devices(rx, 1))?;

                for record in &response.records {
                    if seen_macs.contains(&record.mac_addr) {
                        continue;
                    }
                    seen_macs.insert(record.mac_addr);
                    all_discovered.push(DiscoveredDevice {
                        id: record.mac_addr,
                        fans_type: record.fans_type,
                        dev_type: record.dev_type,
                        group: GroupId::new(record.group),
                        fan_count: record.fan_num.max(1),
                        master: record.master_mac_addr,
                        fans_rpm: record.fans_speed,
                        fans_pwm: record.fans_pwm,
                        cmd_seq: record.cmd_seq,
                        channel: record.channel,
                    });
                }
            }
        }

        Ok(all_discovered)
    }

    fn set_speed(&self, device: &Device, cmd: &SpeedCommand) -> Result<()> {
        with_backoff(|| self.set_speed_inner(device, cmd), "set_speed")
    }

    fn send_rgb(&self, device: &Device, buffer: &EffectResult) -> Result<()> {
        // Round 1
        self.send_rgb_buffer(device, buffer)?;
        // RX sync between rounds (proven reliability pattern from Python impl)
        self.query_rx_status();
        // Round 2
        self.send_rgb_buffer(device, buffer)?;
        Ok(())
    }

    fn reset_device(&self, device: &Device) -> Result<()> {
        let tx_ref = self.get_tx_ref()?;
        let payload = encode::encode_rf_lcd_reset(&device.id, &tx_ref);
        let group_raw = device.group.value();
        self.with_tx_recovery(|tx| {
            sequencer::send_rf_data(tx, self.channel, group_raw, &payload)?;
            tracing::info!("RF reset sent to group {}", device.group);
            Ok(())
        })
    }

    fn set_merge_order(&self, order: &[u8]) -> Result<()> {
        let tx_ref = self.get_tx_ref()?;
        let payload = encode::encode_rf_set_order(&tx_ref, &tx_ref, order);
        self.with_tx_recovery(|tx| {
            sequencer::send_rf_data(tx, self.channel, 0xFF, &payload)?;
            tracing::info!("RF merge order set: {:?}", &order[..order.len().min(4)]);
            Ok(())
        })
    }

    fn as_rf_ext(&self) -> Option<&dyn LianLiRfExt> {
        Some(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl<T: Transport + 'static> LianLiRfExt for LianLiRfBackend<T> {
    fn bind_device(&self, fan_mac: &DeviceId, target_group: GroupId) -> Result<()> {
        let target_group_raw = target_group.value();
        if !(1..=8).contains(&target_group_raw) {
            return Err(CoreError::InvalidInput(format!(
                "target group must be 1-8, got {target_group}"
            )));
        }
        let tx_ref = self.get_tx_ref()?;

        // Pre-compute all payloads outside the closure (prepared once, re-used on retry).
        let tx_init = encode::encode_tx_init(self.channel);
        let tx_sync = encode::encode_tx_sync(self.channel);
        let clock_pkt = encode::encode_master_clock_sync(&tx_ref, self.channel);
        let speed_byte = percent_to_speed_byte(frgb_model::SpeedPercent::new(50));
        let pwm = [speed_byte; 4];
        let payload = encode::encode_bind_rf_payload(fan_mac, &tx_ref, target_group_raw, self.channel, 1, &pwm);
        let lock_pkt = encode::encode_bind(&tx_ref, self.channel);

        self.with_tx_recovery(|tx| {
            // Step 1: TX init ×3
            for _ in 0..3 {
                tx.write(&tx_init)?;
                tx.sleep(sequencer::DELAY_SETUP);
            }

            // Step 2: TX sync + read response
            tx.write(&tx_sync)?;
            tx.sleep(Duration::from_millis(100));
            let _ = tx.read(Duration::from_millis(500));

            // Step 3: Master clock sync broadcast + follow-ups
            sequencer::send_with_followups(tx, &clock_pkt, 0xFF, self.channel)?;

            // Step 4: Send bind RF payload ×20
            // Bind uses all 4 slots — fan_count isn't known until after binding.
            // Send on both 0xFE (standard unbound) and 0xFF (broadcast) to reach
            // devices regardless of their current group assignment.
            for _ in 0..10u8 {
                sequencer::send_rf_data(tx, self.channel, 0xFE, &payload)?;
                tx.sleep(Duration::from_millis(5));
                sequencer::send_rf_data(tx, self.channel, 0xFF, &payload)?;
                tx.sleep(Duration::from_millis(5));
            }

            // Step 5: TX sync finalize
            tx.write(&tx_sync)?;
            tx.sleep(Duration::from_millis(100));

            // Step 6: Lock broadcast ×3
            for _ in 0..3u8 {
                sequencer::send_with_followups(tx, &lock_pkt, 0xFF, self.channel)?;
            }

            Ok(())
        })
    }

    fn unbind_device(&self, fan_mac: &DeviceId, group: GroupId) -> Result<()> {
        let empty_mac = DeviceId::ZERO;
        let payload = encode::encode_bind_rf_payload(fan_mac, &empty_mac, 0xFE, self.channel, 0, &[0; 4]);
        let group_raw = group.value();
        self.with_tx_recovery(|tx| {
            for _ in 0..10 {
                sequencer::send_rf_data(tx, self.channel, group_raw, &payload)?;
                tx.sleep(Duration::from_millis(5));
            }
            Ok(())
        })
    }

    fn lock(&self) -> Result<()> {
        let tx_ref = self.get_tx_ref()?;
        let pkt = encode::encode_bind(&tx_ref, self.channel);
        self.with_tx_recovery(|tx| {
            sequencer::send_with_followups(tx, &pkt, 0xFF, self.channel)?;
            Ok(())
        })
    }

    fn unlock(&self) -> Result<()> {
        let tx_ref = self.get_tx_ref()?;
        let pkt = encode::encode_unlock(&tx_ref, self.channel);
        self.with_tx_recovery(|tx| {
            sequencer::send_with_followups(tx, &pkt, 0xFF, self.channel)?;
            Ok(())
        })
    }

    fn channel(&self) -> u8 {
        self.channel
    }
    fn tx_id(&self) -> Option<DeviceId> {
        self.tx_id
    }
    fn tx_firmware_version(&self) -> Option<u16> {
        self.tx_firmware
    }

    fn set_mb_sync(&self, device: &Device, enable: bool) -> Result<()> {
        let fan_id = device
            .mac_ids
            .first()
            .ok_or_else(|| CoreError::InvalidInput(format!("group {} has no fan IDs", device.group)))?;

        let group_raw = device.group.value();

        // Pre-compute all payloads outside the closure (prepared once, re-used on retry).
        let clock = encode::encode_master_clock_sync(&device.tx_ref, self.channel);
        let send_payload: [u8; 240] = if enable {
            let pwm = Self::build_fans_pwm(frgb_protocol::constants::SPEED_MIN, device.fan_count());
            encode::encode_bind_rf_payload(fan_id, &device.tx_ref, group_raw, self.channel, 1, &pwm)
        } else {
            let new_cmd_seq = device.cmd_seq.wrapping_add(1).max(1);
            encode::encode_mb_sync_payload(fan_id, &device.tx_ref, group_raw, self.channel, 0, new_cmd_seq, false)
        };

        self.with_tx_recovery(|tx| {
            // Always send a master clock sync before speed changes.
            sequencer::send_with_followups(tx, &clock, 0xFF, self.channel)?;

            if enable {
                sequencer::send_rf_data(tx, self.channel, group_raw, &send_payload)?;
            } else {
                for _ in 0..10 {
                    sequencer::send_rf_data(tx, self.channel, group_raw, &send_payload)?;
                    tx.sleep(Duration::from_millis(5));
                }
            }
            Ok(())
        })
    }

    fn set_aio_pump_speed(&self, device: &Device, pct: u8) -> Result<()> {
        // Detect variant from preserved synthetic fans_type (see registry.refresh):
        //   dev_type 10 (WaterBlock)   → fans_type 110 → Circle  (max 2500 RPM)
        //   dev_type 11 (WaterBlock2)  → fans_type 111 → Square  (max 3200 RPM)
        let variant = if device.slots.iter().any(|s| s.fans_type == 111) {
            frgb_protocol::pump::PumpVariant::Square
        } else {
            frgb_protocol::pump::PumpVariant::Circle
        };

        let rpm = frgb_protocol::pump::pct_to_rpm(variant, pct);
        let pwm = variant.rpm_to_pwm(rpm);
        let aio_param = frgb_protocol::pump::build_aio_param(pwm);

        let device_mac = device
            .mac_ids
            .first()
            .ok_or_else(|| CoreError::InvalidInput(format!("group {} has no device MAC", device.group)))?;

        let group_raw = device.group.value();
        let payload = encode::encode_aio_info_payload(device_mac, &device.tx_ref, group_raw, self.channel, &aio_param);

        self.with_tx_recovery(|tx| {
            // Drain stale RX data before sending (matches fan speed path).
            for _ in 0..32 {
                if tx.read(Duration::from_millis(1)).is_err() {
                    break;
                }
            }

            // Send 3x total (1 + 2 retries) with RF repeat delay — matches fan PWM path.
            sequencer::send_rf_data(tx, self.channel, group_raw, &payload)?;
            for _ in 0..2 {
                tx.sleep(sequencer::DELAY_RF_REPEAT);
                sequencer::send_rf_data(tx, self.channel, group_raw, &payload)?;
            }

            tracing::debug!(
                "AIO pump speed: group={} variant={:?} pct={} rpm={} pwm={}",
                device.group,
                variant,
                pct,
                rpm,
                pwm,
            );
            Ok(())
        })
    }
}

/// Execute with exponential backoff retry. 3 attempts, 50ms base delay doubling.
fn with_backoff<F>(mut f: F, description: &str) -> Result<()>
where
    F: FnMut() -> Result<()>,
{
    for attempt in 0..2u32 {
        match f() {
            Ok(()) => return Ok(()),
            Err(e) => {
                let delay_ms = 50u64 * (1 << attempt);
                tracing::warn!(
                    "{description}: attempt {} failed: {e}, retrying in {delay_ms}ms",
                    attempt + 1
                );
                std::thread::sleep(Duration::from_millis(delay_ms));
            }
        }
    }
    f()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{DeviceSlot, DeviceState};
    use crate::transport::mock::MockTransport;
    use frgb_model::device::{BladeType, DeviceType, FanRole};

    fn make_test_device(group: u8) -> Device {
        Device {
            id: DeviceId::from([0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]),
            backend_id: BackendId(0),
            group: GroupId::new(group),
            slots: vec![
                DeviceSlot {
                    fans_type: 21,
                    rpm: 1400,
                    has_lcd: false,
                    source_idx: 0,
                },
                DeviceSlot {
                    fans_type: 21,
                    rpm: 1400,
                    has_lcd: false,
                    source_idx: 1,
                },
                DeviceSlot {
                    fans_type: 21,
                    rpm: 1400,
                    has_lcd: false,
                    source_idx: 2,
                },
            ],
            state: DeviceState::default(),
            mac_ids: vec![DeviceId::from([0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1])],
            tx_ref: DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]),
            name: "Test SL".into(),
            device_type: DeviceType::SlWireless,
            role: FanRole::Intake,
            blade: BladeType::Reverse,
            mb_sync: false,
            cmd_seq: 0,
        }
    }

    fn make_backend() -> LianLiRfBackend<MockTransport> {
        let tx = MockTransport::new();
        let rx = MockTransport::new();
        let mut backend = LianLiRfBackend::new(tx, rx, Some(0x08));
        // Set tx_id so bind/lock/unlock work
        backend.tx_id = Some(DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]));
        backend
    }

    #[test]
    fn set_speed_sends_rf_packets() {
        let backend = make_backend();
        let device = make_test_device(1);

        backend
            .set_speed(&device, &SpeedCommand::Manual(frgb_model::SpeedPercent::new(50)))
            .unwrap();

        let packets = backend.tx().written_packets();
        // 3 sends × 4 RF framing packets = 12
        assert_eq!(packets.len(), 12);
        assert_eq!(packets[0][0], 0x10);
        assert_eq!(packets[0][3], 1); // group
    }

    #[test]
    fn speed_command_contains_correct_group_byte() {
        // Verify the group byte (rf_type / byte[3]) is set correctly for different groups.
        for group_val in [1u8, 3, 5, 8] {
            let backend = make_backend();
            let device = make_test_device(group_val);

            backend
                .set_speed(&device, &SpeedCommand::Manual(frgb_model::SpeedPercent::new(50)))
                .unwrap();

            let packets = backend.tx().written_packets();
            // 3 sends × 4 RF framing packets = 12
            assert_eq!(packets.len(), 12, "group {group_val}: expected 12 packets");
            // Every framing packet should have the group byte at offset 3
            for (i, pkt) in packets.iter().enumerate() {
                assert_eq!(
                    pkt[3], group_val,
                    "group {group_val}, packet {i}: byte[3] (rx_type) should be {group_val}, got {}",
                    pkt[3]
                );
            }
        }
    }

    #[test]
    fn lock_sends_broadcast() {
        let backend = make_backend();
        backend.lock().unwrap();

        let packets = backend.tx().written_packets();
        assert_eq!(packets.len(), 4);
        assert_eq!(packets[0][4..6], [0x12, 0x15]);
    }

    #[test]
    fn unlock_sends_unlock() {
        let backend = make_backend();
        backend.unlock().unwrap();

        let packets = backend.tx().written_packets();
        assert_eq!(packets.len(), 4);
        assert_eq!(packets[0][4..6], [0x12, 0x14]);
    }

    #[test]
    fn bind_rejects_invalid_group() {
        let backend = make_backend();
        let mac = DeviceId::from([0xab, 0x1b, 0x1f, 0xe5, 0x66, 0xe1]);
        assert!(backend.bind_device(&mac, GroupId::new(0)).is_err());
        assert!(backend.bind_device(&mac, GroupId::new(9)).is_err());
    }

    #[test]
    fn bind_full_sequence() {
        let backend = make_backend();
        // Queue a TX sync response for step 2
        let mut sync_resp = [0u8; 64];
        sync_resp[0] = 0x11;
        backend.tx().queue_read(sync_resp);

        let fan_mac = DeviceId::from([0xab, 0x1b, 0x1f, 0xe5, 0x66, 0xe1]);
        backend.bind_device(&fan_mac, GroupId::new(3)).unwrap();

        let packets = backend.tx().written_packets();
        // Preamble: 3 tx_init + 1 tx_sync + 4 pwm_reset = 8
        // Bind: 20 × 4 RF framing = 80
        // Finalize: 1 tx_sync = 1
        // Lock: 3 × 4 = 12
        // Total: 101
        assert_eq!(packets.len(), 101);
    }

    #[test]
    fn backend_name_and_id() {
        let backend = make_backend();
        assert_eq!(backend.name(), "lianli-rf");
        assert_eq!(backend.id(), BackendId(0));
        assert_eq!(LianLiRfExt::channel(&backend), 0x08);
    }

    /// Build a mock multi-packet device query response (7 packets, 1 device record).
    fn build_device_response_packets(mac: [u8; 6], master_mac: [u8; 6], channel: u8, group: u8) -> Vec<[u8; 64]> {
        let mut buf = vec![0u8; 434];
        buf[0] = 0x10; // header
        buf[1] = 1; // num_devices = 1
        let r = 4; // record start offset
        buf[r..r + 6].copy_from_slice(&mac);
        buf[r + 6..r + 12].copy_from_slice(&master_mac);
        buf[r + 12] = channel;
        buf[r + 13] = group;
        buf[r + 18] = 20; // dev_type = SLV3Fan
        buf[r + 19] = 3; // fan_num
        buf[r + 41] = 0x1C; // record delimiter

        let mut packets = Vec::new();
        for chunk in buf.chunks(64) {
            let mut pkt = [0u8; 64];
            pkt[..chunk.len()].copy_from_slice(chunk);
            packets.push(pkt);
        }
        packets
    }

    /// Build a valid TX sync response packet (0x11 header, embedded MAC + firmware).
    fn build_tx_sync_response(tx_mac: [u8; 6], firmware: u16) -> [u8; 64] {
        let mut resp = [0u8; 64];
        resp[0] = 0x11;
        resp[1..7].copy_from_slice(&tx_mac);
        resp[11] = (firmware >> 8) as u8;
        resp[12] = (firmware & 0xFF) as u8;
        resp
    }

    /// Verify that scan_channels() updates the backend channel when devices are found
    /// on a non-default channel.
    ///
    /// Setup:
    ///   - Backend starts on channel 0x08 (already_tried), channel_override=false.
    ///   - TX mock: one valid sync response queued.
    ///   - RX mock: one page of device records queued.
    ///
    /// scan_channels skips 0x08, then probes 0x01 first. discover_tx reads the queued
    /// response (success), discover_devices reads the queued device packets (1 record).
    /// Channel is updated to 0x01 (the first non-skipped channel that returns devices).
    ///
    /// Expects:
    ///   - backend.channel updated from 0x08 to 0x01
    ///   - backend.tx_id set to the TX MAC from the sync response
    #[test]
    fn scan_channels_finds_devices_on_non_default_channel() {
        let tx_mock = MockTransport::new();
        let rx_mock = MockTransport::new();

        let tx_mac = [0x29u8, 0x7a, 0x84, 0xe5, 0x66, 0xe4];
        let fan_mac = [0xc8u8, 0xb4, 0xef, 0x62, 0x32, 0xe1];

        // TX mock: one valid sync response. scan_channels will read it on ch=0x01
        // (first non-skipped channel). No drain between attempts — each discover_tx
        // reads directly from the queue.
        tx_mock.queue_read(build_tx_sync_response(tx_mac, 0x0105));

        // RX mock: device packets for the ch=0x01 query (first successful probe).
        for pkt in build_device_response_packets(fan_mac, tx_mac, 0x01, 1) {
            rx_mock.queue_read(pkt);
        }

        // Create backend: no channel_override → channel=0x08, channel_override=false.
        let mut backend = LianLiRfBackend::new(tx_mock, rx_mock, None);
        backend.tx_id = Some(DeviceId::from(tx_mac));

        // Call scan_channels directly. It skips 0x08 (already_tried), probes 0x01 first:
        // TX sync succeeds, RX has device records → channel updated to 0x01.
        backend.scan_channels().unwrap();

        assert_eq!(
            LianLiRfExt::channel(&backend),
            0x01,
            "scan_channels should update channel from 0x08 to first channel with devices"
        );
        assert_eq!(
            backend.tx_id,
            Some(DeviceId::from(tx_mac)),
            "tx_id should be set from the TX sync response"
        );
    }

    /// Verify that scan_channels() returns Ok and leaves channel unchanged when no devices
    /// respond on any alternative channel.
    #[test]
    fn scan_channels_no_op_when_no_devices_found() {
        let tx_mock = MockTransport::new();
        let rx_mock = MockTransport::new();

        // No packets queued: all TX sync attempts time out → no channel change.
        let mut backend = LianLiRfBackend::new(tx_mock, rx_mock, None);
        backend.tx_id = Some(DeviceId::from([0x29u8, 0x7a, 0x84, 0xe5, 0x66, 0xe4]));

        backend.scan_channels().unwrap();

        assert_eq!(
            LianLiRfExt::channel(&backend),
            0x08,
            "channel should remain 0x08 when no devices found on any alternative channel"
        );
    }

    struct CountingFailMock {
        fails_remaining: std::cell::Cell<u32>,
        calls: std::cell::Cell<u32>,
    }

    impl CountingFailMock {
        fn new(initial_failures: u32) -> Self {
            Self {
                fails_remaining: std::cell::Cell::new(initial_failures),
                calls: std::cell::Cell::new(0),
            }
        }
    }

    impl crate::transport::Transport for CountingFailMock {
        fn write(&self, _data: &[u8]) -> crate::error::Result<()> {
            self.calls.set(self.calls.get() + 1);
            let remaining = self.fails_remaining.get();
            if remaining > 0 {
                self.fails_remaining.set(remaining - 1);
                Err(crate::error::CoreError::Usb(frgb_usb::error::UsbError::Io(
                    "mock-failure".into(),
                )))
            } else {
                Ok(())
            }
        }
        fn read(
            &self,
            _timeout: std::time::Duration,
        ) -> crate::error::Result<[u8; crate::transport::PACKET_SIZE]> {
            Ok([0u8; crate::transport::PACKET_SIZE])
        }
        fn sleep(&self, _duration: std::time::Duration) {}
    }

    #[test]
    fn with_tx_recovery_retries_on_failure() {
        let tx = CountingFailMock::new(1); // fail once, then succeed
        let rx = CountingFailMock::new(0);
        let backend = LianLiRfBackend::<CountingFailMock>::new(tx, rx, None);
        let r = backend.with_tx_recovery(|t| t.write(&[0u8; 64]));
        assert!(r.is_ok(), "expected Ok after retry, got {r:?}");
        assert_eq!(backend.tx.borrow().calls.get(), 2, "op should be invoked twice (1 fail, 1 success)");
    }

    #[test]
    fn with_tx_recovery_propagates_persistent_failure() {
        let tx = CountingFailMock::new(10); // always fails
        let rx = CountingFailMock::new(0);
        let backend = LianLiRfBackend::<CountingFailMock>::new(tx, rx, None);
        let r = backend.with_tx_recovery(|t| t.write(&[0u8; 64]));
        assert!(r.is_err());
    }

    #[test]
    fn with_rx_recovery_retries_on_failure() {
        let tx = CountingFailMock::new(0);
        let rx = CountingFailMock::new(1); // fail once on RX, then succeed
        let backend = LianLiRfBackend::<CountingFailMock>::new(tx, rx, None);
        let r = backend.with_rx_recovery(|t| t.write(&[0u8; 64]));
        assert!(r.is_ok(), "expected Ok after retry, got {r:?}");
        assert_eq!(backend.rx.borrow().calls.get(), 2, "rx op should be invoked twice");
    }

    #[test]
    fn with_backoff_returns_last_error_after_three_attempts() {
        use std::cell::Cell;
        let counter = Cell::new(0u32);
        let result = with_backoff(
            || {
                counter.set(counter.get() + 1);
                Err(CoreError::InvalidInput(format!("attempt {}", counter.get())))
            },
            "test",
        );
        assert_eq!(counter.get(), 3, "expected exactly 3 attempts");
        match result {
            Err(CoreError::InvalidInput(msg)) => assert_eq!(msg, "attempt 3"),
            other => panic!("expected InvalidInput Err, got {other:?}"),
        }
    }
}
