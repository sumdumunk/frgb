//! Engine loop — periodic tasks: discovery refresh, RPM polling, sequence/curve evaluation.
//!
//! The engine ticks on a target interval (50ms). Each tick:
//! 1. Advance sequence playback (every tick, uses elapsed duration for jitter tolerance)
//! 2. Re-discover devices on poll interval (detect connects/disconnects)
//! 3. Poll hwmon sensors and evaluate fan curves (on poll interval)
//! 4. Dispatch events to subscribed IPC clients
//!
//! USB discovery blocks the tick thread (~100-200ms). The show runner uses real
//! elapsed time, not tick count, so animation timing self-corrects after a
//! slow tick. Fan curves and sensors only run on the poll interval (~2s)
//! so they're unaffected by tick jitter.

use std::path::Path;
use std::time::{Duration, Instant};

use frgb_core::backend::SensorReading;
use frgb_core::hwmon::{self, HwmonChip};
use frgb_core::System;
use frgb_model::ipc::Event;
use frgb_model::sensor::SensorCalibration;
use frgb_model::device::FanRole;
use frgb_model::speed::SpeedMode;
use frgb_model::GroupId;
use frgb_model::SpeedPercent;

use std::collections::{HashMap, HashSet, VecDeque};

use crate::alerts::AlertRunner;
use crate::app_profiles::AppProfileRunner;
use crate::curves::CurveRunner;
use crate::lcd_manager::{LcdAction, LcdManager};
use crate::power::PowerRunner;
use crate::scheduler::ScheduleRunner;
use crate::show_runner::ShowRunner;
use crate::temp_rgb::TempRgbRunner;

const RPM_HISTORY_LEN: usize = 60; // ~2 minutes at 2s poll

/// Apply a profile's group snapshots to the system.
///
/// For each snapshot:
/// - If `scene.speed` is Some AND the group is fan-capable (per
///   `System::is_fan_capable`), apply the speed.
/// - Always apply `scene.rgb` — RGB-only groups (AURA motherboard headers)
///   are valid RGB targets even though they have no fan speed.
///
/// Errors are silently ignored — profile-switch is best-effort, matching the
/// existing daemon convention. Used by SwitchProfile (handler.rs) and the four
/// engine profile-application sites (AppProfile/Power/Alert/Schedule).
pub(crate) fn apply_profile_groups(
    system: &mut System,
    snapshots: &[frgb_model::config::GroupSnapshot],
) {
    for snap in snapshots {
        if let Some(speed) = &snap.scene.speed {
            if system.is_fan_capable(snap.group_id) {
                let _ = system.set_speed(snap.group_id, speed);
            }
        }
        let _ = system.set_rgb(snap.group_id, &snap.scene.rgb);
    }
}

/// Engine state — tracks timing for periodic tasks and runs sub-engines.
pub struct Engine {
    /// How often to re-discover devices and poll sensors (ms).
    pub poll_interval: Duration,
    last_poll: Instant,
    last_tick: Instant,

    /// Cached RPMs from last discovery (for change detection).
    last_rpms: Vec<(GroupId, [u16; 4])>,

    /// Per-group RPM history: ring buffer of average RPM readings.
    rpm_history: HashMap<GroupId, VecDeque<u16>>,

    /// Per-group RPM-anomaly suppression — already-alerted groups in this set
    /// don't re-fire until the condition resolves. Cleared when condition
    /// resolves AND ≥10 samples are available; entries persist when data is
    /// insufficient (no new evidence). Garbage-collected against rpm_history
    /// at the end of each check_rpm_anomalies call.
    rpm_anomaly_alerted: HashSet<GroupId>,

    /// Sequence/effect cycle playback.
    pub show_runner: ShowRunner,
    /// Fan curve evaluation.
    pub curves: CurveRunner,
    /// Temperature alert monitoring.
    pub alerts: AlertRunner,
    /// Temperature-reactive RGB.
    pub temp_rgb: TempRgbRunner,
    /// Time-based schedule evaluation.
    pub scheduler: ScheduleRunner,
    /// Application-based profile switching.
    pub app_profiles: AppProfileRunner,
    /// AC/battery power state monitoring.
    pub power: PowerRunner,
    /// LCD content manager — generates frames, tracks per-device state.
    pub lcd_manager: LcdManager,

    /// Hwmon sensor chips (scanned once at startup).
    hwmon_chips: Vec<HwmonChip>,
    /// Sensor calibration offsets from config.
    sensor_calibration: SensorCalibration,
    /// Pending indicate restores: (deadline, group_id, previous_rgb_mode).
    /// Multiple groups can be indicated simultaneously without losing restore state.
    pub indicate_restores: Vec<(Instant, GroupId, frgb_model::rgb::RgbMode)>,

    /// Whether the daemon is actively scanning for unbound devices.
    pub bind_scanning: bool,

    /// Cumulative running seconds per group (RPM > 0 = running).
    wear_counters: HashMap<GroupId, u64>,

    /// Max observed temperature per sensor over the session lifetime.
    sensor_peaks: HashMap<frgb_model::sensor::Sensor, f32>,
    /// When peak tracking started (for age context in future reporting).
    #[allow(dead_code)]
    sensor_peak_start: Instant,
}

impl Engine {
    pub fn new(poll_interval_ms: u32) -> Self {
        Self {
            poll_interval: Duration::from_millis(poll_interval_ms as u64),
            last_poll: Instant::now(),
            last_tick: Instant::now(),
            last_rpms: Vec::new(),
            rpm_history: HashMap::new(),
            rpm_anomaly_alerted: HashSet::new(),
            show_runner: ShowRunner::new(),
            curves: CurveRunner::new(),
            alerts: AlertRunner::new(),
            temp_rgb: TempRgbRunner::new(),
            scheduler: ScheduleRunner::new(),
            app_profiles: AppProfileRunner::new(),
            power: PowerRunner::new(),
            lcd_manager: {
                let mgr = LcdManager::new();
                mgr.init_presets();
                mgr
            },
            hwmon_chips: Vec::new(),
            sensor_calibration: SensorCalibration::default(),
            indicate_restores: Vec::new(),
            bind_scanning: false,
            wear_counters: HashMap::new(),
            sensor_peaks: HashMap::new(),
            sensor_peak_start: Instant::now(),
        }
    }

    /// Scan hwmon chips and load calibration. Call once at startup.
    pub fn init_hwmon(&mut self, calibration: SensorCalibration) {
        self.hwmon_chips = hwmon::scan_chips(Path::new("/sys/class/hwmon"));
        self.sensor_calibration = calibration;
        if !self.hwmon_chips.is_empty() {
            let sensor_count: usize = self.hwmon_chips.iter().map(|c| c.inputs.len()).sum();
            tracing::info!("Hwmon: {} chip(s), {} sensor(s)", self.hwmon_chips.len(), sensor_count);
        }
    }

    /// Get cached hwmon chips (for sensor listing).
    pub fn hwmon_chips(&self) -> &[HwmonChip] {
        &self.hwmon_chips
    }

    /// Get sensor calibration (for on-demand reads in handler).
    pub fn sensor_calibration(&self) -> &SensorCalibration {
        &self.sensor_calibration
    }

    /// Load wear stats from persisted config entries (call once at startup).
    pub fn load_wear_stats(&mut self, entries: &[frgb_model::config::WearEntry]) {
        for entry in entries {
            self.wear_counters.insert(entry.group_id, entry.running_seconds);
        }
    }

    /// Get current wear counters (group → cumulative running seconds).
    pub fn wear_stats(&self) -> &HashMap<GroupId, u64> {
        &self.wear_counters
    }

    /// Export wear counters for config persistence.
    pub fn wear_entries(&self) -> Vec<frgb_model::config::WearEntry> {
        self.wear_counters
            .iter()
            .map(|(&group_id, &running_seconds)| frgb_model::config::WearEntry {
                group_id,
                running_seconds,
            })
            .collect()
    }

    /// Analyse active fan curves against observed thermal peaks.
    /// Returns suggestions where the curve's max-speed temperature is more than
    /// 10 °C above the observed peak, indicating potential to lower the curve.
    pub fn curve_suggestions(
        &self,
        config: &crate::config_cache::ConfigCache,
    ) -> Vec<frgb_model::speed::CurveSuggestion> {
        use frgb_model::speed::CurveSuggestion;
        let cfg = config.config();
        let mut suggestions = Vec::new();

        for (&group, state) in self.curves.active_curves() {
            let curve = &state.curve;
            let sensor = &curve.sensor;

            if let Some(&observed_max) = self.sensor_peaks.get(sensor) {
                if let Some(last_point) = curve.points.last() {
                    let headroom = last_point.temp.celsius() as f32 - observed_max;

                    if headroom > 10.0 {
                        let curve_name = cfg
                            .saved_curves
                            .iter()
                            .find(|nc| nc.curve == *curve)
                            .map(|nc| nc.name.as_str().to_string())
                            .unwrap_or_else(|| "inline".into());

                        suggestions.push(CurveSuggestion {
                            group,
                            curve_name,
                            sensor: sensor.clone(),
                            message: format!(
                                "peak {:?} temp is {:.0}°C but curve reaches max speed at {}°C — {:.0}°C headroom",
                                sensor,
                                observed_max,
                                last_point.temp.celsius(),
                                headroom
                            ),
                            observed_max_temp: observed_max,
                            curve_max_speed_temp: last_point.temp.celsius(),
                        });
                    }
                }
            }
        }

        suggestions
    }

    /// Tick the engine. Returns events generated this tick.
    #[tracing::instrument(skip_all, fields(active_devices = system.devices().len()))]
    pub fn tick(&mut self, system: &mut System, config: &crate::config_cache::ConfigCache) -> Vec<Event> {
        let mut events = Vec::new();
        let now = Instant::now();
        let tick_elapsed = now.duration_since(self.last_tick);
        self.last_tick = now;

        // --- Sequence playback (every tick) ---
        if self.show_runner.is_active() {
            let (actions, seq_events) = self.show_runner.tick(tick_elapsed);
            events.extend(seq_events);

            for action in actions {
                apply_scene(system, action.group, &action.scene);
            }
        }

        // --- Indicate restore (flash timeouts) ---
        self.indicate_restores.retain(|&(deadline, group, ref mode)| {
            if now >= deadline {
                let _ = system.set_rgb(group, mode);
                events.push(Event::RgbChanged {
                    group,
                    mode: mode.clone(),
                });
                false // remove from vec
            } else {
                true // keep pending
            }
        });

        // --- Periodic tasks (discovery interval) ---
        if self.last_poll.elapsed() >= self.poll_interval {
            self.last_poll = Instant::now();

            // Discovery + RPM change detection
            if let Err(e) = system.discover() {
                tracing::warn!("Discovery refresh failed: {e}");
                self.last_rpms.clear();
            } else {
                // Re-apply group configs (role, name) after discovery.
                // Discovery may re-create devices with default state if RF flaps.
                system.registry.apply_group_configs(&config.config().groups);

                events.extend(self.detect_rpm_changes(system));
                events.extend(self.check_rpm_anomalies());

                // Wear counter: accumulate running time for groups with any RPM > 0.
                let poll_secs = self.poll_interval.as_secs();
                for &(group, rpms) in &self.last_rpms {
                    if rpms.iter().any(|&r| r > 0) {
                        *self.wear_counters.entry(group).or_insert(0) += poll_secs;
                    }
                }

                // Bind mode: emit events for newly discovered unbound devices
                if self.bind_scanning && !system.unbound.is_empty() {
                    for dev in &system.unbound {
                        events.push(Event::BindDiscovered(dev.clone()));
                    }
                }

                // Register LCD devices and map them to fan groups.
                // LCD-capable fan groups and LCD USB devices are matched by
                // index: first LCD group → first LCD device, etc.
                // Guard: only re-register when the USB device count changes
                // (avoids unbounded group_map growth on every poll tick).
                let lcd_ids = system.lcd_device_ids();
                if lcd_ids.len() != self.lcd_manager.device_count() {
                    for lcd_id in &lcd_ids {
                        let (w, h) = lcd_resolution(lcd_id);
                        self.lcd_manager.register_device(*lcd_id, w, h);
                    }
                    let lcd_groups: Vec<GroupId> = system
                        .devices()
                        .iter()
                        .filter(|d| d.slots.iter().any(|s| s.has_lcd))
                        .map(|d| d.group)
                        .collect();
                    for (i, &group) in lcd_groups.iter().enumerate() {
                        if let Some(&lcd_id) = lcd_ids.get(i) {
                            self.lcd_manager.map_group(group, lcd_id);
                        }
                    }
                }

                // Pump keepalive: re-assert commanded pump speeds each poll
                // cycle. AIO pump firmware does not latch RF speed commands
                // persistently — the pump decays back to its minimum RPM
                // (~1600) if not continuously re-commanded. Iterates devices
                // with AIO device_type OR Pump role and a cached
                // state.speed_percent (which was either set by a prior
                // Request::SetSpeed, seeded from config on startup, or
                // written by a curve on an earlier tick).
                let pump_keepalive: Vec<(GroupId, SpeedPercent)> = system
                    .devices()
                    .iter()
                    .filter(|d| d.device_type.is_aio() || matches!(d.role, FanRole::Pump))
                    .filter_map(|d| d.state.speed_percent.map(|pct| (d.group, pct)))
                    .collect();
                for (group, pct) in pump_keepalive {
                    if let Err(e) = system.set_speed(group, &SpeedMode::Manual(pct)) {
                        tracing::warn!("Pump keepalive failed for group {group}: {e}");
                    }
                }
            }

            // Sensor polling via hwmon
            let readings = if !self.hwmon_chips.is_empty() {
                let r = hwmon::read_calibrated(&self.hwmon_chips, &self.sensor_calibration);
                let sensor_events = self.curves.ingest_readings(&r);
                // Feed sensor updates to alert + temp_rgb runners
                for event in &sensor_events {
                    if let Event::SensorUpdate { sensor, value } = event {
                        self.alerts.ingest(sensor, *value);
                        self.temp_rgb.ingest(sensor, *value);
                    }
                }
                events.extend(sensor_events);
                // Track peak observed temperature per sensor.
                for reading in &r {
                    if let Some(sensor) = hwmon::classify_sensor(&reading.label) {
                        let peak = self.sensor_peaks.entry(sensor).or_insert(0.0);
                        *peak = peak.max(reading.value as f32);
                    }
                }
                r
            } else {
                Vec::new()
            };

            // LCD frame generation — reuse readings from sensor poll above
            if self.lcd_manager.has_devices() {
                let lcd_readings = build_lcd_readings(&readings);
                let lcd_actions = self.lcd_manager.tick(&lcd_readings);
                for action in lcd_actions {
                    match action {
                        LcdAction::SendFrame { device_id, jpeg } => {
                            if let Err(e) = system.send_lcd_frame(&device_id, &jpeg) {
                                tracing::warn!("LCD frame push failed: {e}");
                            }
                        }
                        LcdAction::SetClock { device_id } => {
                            if let Err(e) = system.set_lcd_clock(&device_id) {
                                tracing::warn!("LCD set clock failed: {e}");
                            }
                        }
                        LcdAction::SetBrightness { device_id, brightness } => {
                            if let Err(e) = system.set_lcd_brightness(&device_id, brightness) {
                                tracing::warn!("LCD set brightness failed: {e}");
                            }
                        }
                        LcdAction::SetRotation { device_id, rotation } => {
                            if let Err(e) = system.set_lcd_rotation(&device_id, rotation) {
                                tracing::warn!("LCD set rotation failed: {e}");
                            }
                        }
                    }
                }
            }

            if self.curves.is_active() {
                let elapsed_secs = self.poll_interval.as_secs_f32();
                let (commands, curve_events) = self.curves.evaluate(elapsed_secs);
                events.extend(curve_events);
                for (group, speed_pct) in commands {
                    if !system.is_fan_capable(group) {
                        continue;
                    }
                    if let Err(e) = system.set_speed(group, &SpeedMode::Manual(speed_pct)) {
                        tracing::warn!("Curve speed set failed for group {group}: {e}");
                    }
                }
            }
            // Temperature-reactive RGB
            if self.temp_rgb.is_active() {
                let (commands, rgb_events) = self.temp_rgb.evaluate();
                events.extend(rgb_events);
                for (group, mode) in commands {
                    if let Err(e) = system.set_rgb(group, &mode) {
                        tracing::warn!("TempRgb set failed for group {group}: {e}");
                    }
                }
            }
            // Alert evaluation (after sensor ingest)
            if self.alerts.is_active() {
                let alert_events = self.alerts.evaluate();
                for event in &alert_events {
                    if let Event::Alert(alert) = event {
                        execute_alert_actions(&self.alerts, alert, system, config);
                    }
                }
                events.extend(alert_events);
            }
            // App profile evaluation (focused window)
            if self.app_profiles.is_active() {
                if let Some(cmd) = self.app_profiles.evaluate() {
                    match cmd {
                        crate::app_profiles::AppProfileCommand::SwitchProfile(name) => {
                            let cfg = config.config();
                            if let Some(profile) = cfg.profiles.iter().find(|p| p.name == name) {
                                apply_profile_groups(system, &profile.groups);
                                events.push(Event::ProfileSwitched { name });
                            }
                        }
                    }
                }
            }
            // Power state monitoring (AC/battery)
            if self.power.is_active() {
                if let Some(cmd) = self.power.evaluate() {
                    match cmd {
                        crate::power::PowerCommand::SwitchProfile(name) => {
                            let cfg = config.config();
                            if let Some(profile) = cfg.profiles.iter().find(|p| p.name == name) {
                                apply_profile_groups(system, &profile.groups);
                                events.push(Event::PowerChanged {
                                    on_ac: self.power.is_on_ac(),
                                });
                                events.push(Event::ProfileSwitched { name });
                            }
                        }
                    }
                }
            }
            // Schedule evaluation (time-based)
            if self.scheduler.is_active() {
                let all_groups = system.group_ids();
                let commands = self.scheduler.evaluate(&all_groups);
                for cmd in commands {
                    execute_schedule_command(cmd, system, config);
                }
            }
        }

        events
    }

    fn detect_rpm_changes(&mut self, system: &System) -> Vec<Event> {
        let mut events = Vec::new();
        let current_rpms: Vec<(GroupId, [u16; 4])> = system.devices().iter().map(|d| (d.group, d.fans_rpm())).collect();

        // O(1) lookup for previous RPMs instead of O(n) linear scan per group
        let old_map: HashMap<GroupId, [u16; 4]> = self.last_rpms.iter().map(|&(g, rpms)| (g, rpms)).collect();

        for &(group, rpms) in &current_rpms {
            match old_map.get(&group) {
                Some(old_rpms) if *old_rpms != rpms => {
                    events.push(Event::RpmUpdate {
                        group,
                        rpms: rpms.to_vec(),
                    });
                }
                None => {
                    events.push(Event::DeviceConnected { group });
                    events.push(Event::RpmUpdate {
                        group,
                        rpms: rpms.to_vec(),
                    });
                }
                _ => {}
            }
        }

        // O(1) lookup for disconnection detection
        let current_groups: std::collections::HashSet<GroupId> = current_rpms.iter().map(|&(g, _)| g).collect();
        for &(group, _) in &self.last_rpms {
            if !current_groups.contains(&group) {
                events.push(Event::DeviceDisconnected { group });
            }
        }

        self.last_rpms = current_rpms;

        // Update RPM history for anomaly detection
        for &(group, rpms) in &self.last_rpms {
            let nonzero: Vec<u16> = rpms.iter().copied().filter(|&r| r > 0).collect();
            let avg = if nonzero.is_empty() {
                0u16
            } else {
                (nonzero.iter().map(|&r| r as u32).sum::<u32>() / nonzero.len() as u32) as u16
            };
            let history = self.rpm_history.entry(group).or_default();
            history.push_back(avg);
            if history.len() > RPM_HISTORY_LEN {
                history.pop_front();
            }
        }

        events
    }

    fn check_rpm_anomalies(&mut self) -> Vec<Event> {
        // (first_avg, second_avg, drop_pct)
        type RpmStats = (u64, u64, u64);
        let mut events = Vec::new();

        // Pass 1: read rpm_history immutably, collect numeric decisions.
        // Some(stats) = anomaly detected this tick (caller will format if firing)
        // None        = condition resolved this tick (clear from alerted set)
        // Group not in decisions = insufficient data, alerted state unchanged
        let mut decisions: Vec<(GroupId, Option<RpmStats>)> = Vec::new();
        for (&group, history) in &self.rpm_history {
            // Insufficient data: leave alerted state unchanged — we have no new
            // evidence either way. Will re-evaluate when more samples arrive.
            if history.len() < 10 {
                continue;
            }
            let mid = history.len() / 2;
            let first_half: Vec<u16> = history.iter().take(mid).copied().collect();
            let second_half: Vec<u16> = history.iter().skip(mid).copied().collect();
            let first_avg = first_half.iter().map(|&r| r as u64).sum::<u64>() / first_half.len().max(1) as u64;
            let second_avg = second_half.iter().map(|&r| r as u64).sum::<u64>() / second_half.len().max(1) as u64;

            if first_avg > 100 && second_avg > 0 && second_avg < first_avg {
                let drop_pct = ((first_avg - second_avg) * 100) / first_avg;
                if drop_pct > 20 {
                    decisions.push((group, Some((first_avg, second_avg, drop_pct))));
                    continue;
                }
            }
            decisions.push((group, None));
        }

        // Pass 2: mutate self.rpm_anomaly_alerted, format messages only on first
        // detection (so steady-state alerted groups don't pay format! cost).
        for (group, stats) in decisions {
            match stats {
                Some((first_avg, second_avg, drop_pct)) => {
                    if self.rpm_anomaly_alerted.insert(group) {
                        events.push(Event::RpmAnomaly {
                            group,
                            message: format!("RPM dropped {}% (avg {} → {})", drop_pct, first_avg, second_avg),
                        });
                    }
                }
                None => {
                    self.rpm_anomaly_alerted.remove(&group);
                }
            }
        }

        // GC alerted entries for groups that no longer have any history (e.g.,
        // device unplugged permanently). Cheap — alerted set is small.
        self.rpm_anomaly_alerted.retain(|g| self.rpm_history.contains_key(g));

        events
    }
}

/// Execute the action associated with a triggered alert.
fn execute_alert_actions(
    runner: &AlertRunner,
    alert: &frgb_model::config::AlertEvent,
    system: &mut System,
    config: &crate::config_cache::ConfigCache,
) {
    use frgb_model::config::AlertAction;

    for pending in runner.pending_actions() {
        if pending.sensor == alert.sensor && pending.threshold == alert.threshold {
            match &pending.action {
                AlertAction::Notify => {
                    tracing::warn!(
                        "ALERT: {:?} reached {:.1}°C (threshold {}°C)",
                        alert.sensor,
                        alert.value,
                        alert.threshold
                    );
                }
                AlertAction::SetSpeed(pct) => {
                    tracing::warn!(
                        "ALERT: {:?} at {:.1}°C — setting all fans to {}%",
                        alert.sensor,
                        alert.value,
                        pct
                    );
                    for group in system.group_ids() {
                        if !system.is_fan_capable(group) {
                            continue;
                        }
                        let _ = system.set_speed(group, &SpeedMode::Manual(*pct));
                    }
                }
                AlertAction::SwitchProfile(name) => {
                    tracing::warn!(
                        "ALERT: {:?} at {:.1}°C — switching to profile '{}'",
                        alert.sensor,
                        alert.value,
                        name
                    );
                    let cfg = config.config();
                    if let Some(profile) = cfg.profiles.iter().find(|p| p.name == *name) {
                        apply_profile_groups(system, &profile.groups);
                    }
                }
            }
        }
    }
}

/// Execute a schedule command.
fn execute_schedule_command(
    cmd: crate::scheduler::ScheduleCommand,
    system: &mut System,
    config: &crate::config_cache::ConfigCache,
) {
    use crate::scheduler::ScheduleCommand;
    let cfg = config.config();
    match cmd {
        ScheduleCommand::SwitchProfile(name) => {
            if let Some(profile) = cfg.profiles.iter().find(|p| p.name == name) {
                apply_profile_groups(system, &profile.groups);
                tracing::info!("Schedule: switched to profile '{}'", name);
            }
        }
        ScheduleCommand::SetSpeed { group, percent } => {
            if !system.is_fan_capable(group) {
                return;
            }
            let _ = system.set_speed(group, &SpeedMode::Manual(percent));
        }
        ScheduleCommand::ApplyCurve { group, curve } => {
            if !system.is_fan_capable(group) {
                return;
            }
            if let Some(_named) = cfg.saved_curves.iter().find(|c| c.name == curve) {
                tracing::info!("Schedule: applying curve '{}' to group {}", curve, group);
                let _ = system.set_speed(group, &SpeedMode::NamedCurve(curve));
            } else {
                tracing::warn!("Schedule: curve '{}' not found", curve);
            }
        }
    }
}

/// Apply a Scene to a device group (RGB + optional speed).
fn apply_scene(system: &mut System, group: GroupId, scene: &frgb_model::show::Scene) {
    if let Err(e) = system.set_rgb(group, &scene.rgb) {
        tracing::warn!("Sequence RGB failed for group {group}: {e}");
    }
    if let Some(ref speed) = scene.speed {
        if system.is_fan_capable(group) {
            if let Err(e) = system.set_speed(group, speed) {
                tracing::warn!("Sequence speed failed for group {group}: {e}");
            }
        }
    }
}

/// Build a label-keyed sensor map for LCD rendering from hwmon data.
///
/// LCD renderer expects keys like "CPU", "GPU", "Water", "MB".
/// Maps Sensor variants to their canonical label strings.
fn build_lcd_readings(readings: &[SensorReading]) -> HashMap<String, f32> {
    use frgb_model::sensor::Sensor;
    let mut map = HashMap::new();
    for reading in readings {
        if let Some(sensor) = hwmon::classify_sensor(&reading.label) {
            let key = match &sensor {
                Sensor::Cpu => "CPU",
                Sensor::Gpu => "GPU",
                Sensor::GpuHotspot => "GPU Hotspot",
                Sensor::GpuVram => "GPU VRAM",
                Sensor::GpuPower => "GPU Power",
                Sensor::GpuUsage => "GPU Usage",
                Sensor::Water => "Water",
                Sensor::Motherboard { .. } => "MB",
                Sensor::Weighted { .. } => continue,
            };
            // First match wins — avoids overwriting with lower-priority chip
            map.entry(key.to_string()).or_insert(reading.value as f32);
        }
    }
    map
}

/// Determine LCD resolution from a DeviceId synthesized via `from_vid_pid`.
///
/// Extracts PID from bytes 2-3 and maps to known LCD resolutions.
fn lcd_resolution(device_id: &frgb_model::device::DeviceId) -> (u32, u32) {
    use frgb_model::usb_ids::*;
    let bytes = device_id.as_bytes();
    let pid = ((bytes[2] as u16) << 8) | (bytes[3] as u16);
    match pid {
        PID_SL_LCD | PID_TLV2_LCD => (400, 400),
        PID_HYDROSHIFT_CIRCLE | PID_HYDROSHIFT_SQUARE => (480, 480),
        PID_UNIVERSAL_88 => (480, 1920),
        _ => (400, 400),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_core::backend::BackendId;
    use frgb_model::device::DeviceId;
    use frgb_model::ipc::Event;
    use frgb_model::sensor::Sensor;
    use frgb_model::spec_loader::load_defaults;
    use frgb_model::speed::{CurvePoint, FanCurve, Interpolation};
    use frgb_model::GroupId;
    use frgb_model::SpeedPercent;
    use frgb_model::Temperature;

    fn test_config() -> crate::config_cache::ConfigCache {
        crate::config_cache::ConfigCache::from(frgb_model::config::Config::default())
    }

    /// Create a System with no backends.
    fn empty_system() -> System {
        System::new(load_defaults())
    }

    /// Create a System with one device in the registry (group 1).
    fn system_with_device() -> System {
        let specs = load_defaults();
        let mut system = System::new(specs);
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        system.registry.refresh(
            BackendId(0),
            vec![frgb_core::DiscoveredDevice {
                id: DeviceId::from([0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]),
                fans_type: [21, 21, 21, 0],
                dev_type: 0,
                group: GroupId::new(1),
                fan_count: 3,
                master: our_mac,
                fans_rpm: [1200, 1200, 1200, 0],
                fans_pwm: [0; 4],
                cmd_seq: 0,
                channel: 0x08,
            }],
            our_mac,
            &system.specs,
        );
        system
    }

    /// Create a System with two devices (groups 1 and 2).
    fn system_with_two_devices() -> System {
        let specs = load_defaults();
        let mut system = System::new(specs);
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        system.registry.refresh(
            BackendId(0),
            vec![
                frgb_core::DiscoveredDevice {
                    id: DeviceId::from([0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]),
                    fans_type: [21, 21, 21, 0],
                    dev_type: 0,
                    group: GroupId::new(1),
                    fan_count: 3,
                    master: our_mac,
                    fans_rpm: [1200, 1200, 1200, 0],
                    fans_pwm: [0; 4],
                    cmd_seq: 0,
                    channel: 0x08,
                },
                frgb_core::DiscoveredDevice {
                    id: DeviceId::from([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]),
                    fans_type: [42, 41, 0, 0],
                    dev_type: 0,
                    group: GroupId::new(2),
                    fan_count: 2,
                    master: our_mac,
                    fans_rpm: [800, 800, 0, 0],
                    fans_pwm: [0; 4],
                    cmd_seq: 0,
                    channel: 0x08,
                },
            ],
            our_mac,
            &system.specs,
        );
        system
    }

    // -----------------------------------------------------------------------
    // Construction
    // -----------------------------------------------------------------------

    /// Engine::new constructs without panic.
    #[test]
    fn engine_new() {
        let engine = Engine::new(2000);
        assert_eq!(engine.poll_interval, Duration::from_millis(2000));
        assert!(!engine.curves.is_active());
        assert!(!engine.show_runner.is_active());
        assert!(engine.scheduler.entries().is_empty());
    }

    /// Source: INFERRED — Engine with zero poll interval is valid (immediate polling).
    #[test]
    fn engine_zero_poll_interval() {
        let engine = Engine::new(0);
        assert_eq!(engine.poll_interval, Duration::ZERO);
    }

    // -----------------------------------------------------------------------
    // RPM change detection
    // -----------------------------------------------------------------------

    /// First tick with devices emits DeviceConnected + RpmUpdate.
    #[test]
    fn rpm_first_detection_emits_connected() {
        let mut engine = Engine::new(2000);
        let sys = system_with_device();
        let events = engine.detect_rpm_changes(&sys);

        // First time seeing group 1 → DeviceConnected + RpmUpdate
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], Event::DeviceConnected { group } if group == GroupId::new(1)));
        assert!(matches!(events[1], Event::RpmUpdate { group, .. } if group == GroupId::new(1)));
    }

    /// Second tick with same RPMs emits no events.
    #[test]
    fn rpm_no_change_no_events() {
        let mut engine = Engine::new(2000);
        let sys = system_with_device();

        engine.detect_rpm_changes(&sys); // first detection
        let events = engine.detect_rpm_changes(&sys); // same RPMs
        assert!(events.is_empty());
    }

    /// RPM change emits RpmUpdate (not DeviceConnected again).
    #[test]
    fn rpm_change_emits_update() {
        let mut engine = Engine::new(2000);
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);

        // First tick: 1200 RPM
        let mut sys = system_with_device();
        engine.detect_rpm_changes(&sys);

        // Second tick: RPMs changed to 1500
        sys.registry.refresh(
            BackendId(0),
            vec![frgb_core::DiscoveredDevice {
                id: DeviceId::from([0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]),
                fans_type: [21, 21, 21, 0],
                dev_type: 0,
                group: GroupId::new(1),
                fan_count: 3,
                master: our_mac,
                fans_rpm: [1500, 1500, 1500, 0],
                fans_pwm: [0; 4],
                cmd_seq: 0,
                channel: 0x08,
            }],
            our_mac,
            &sys.specs,
        );

        let events = engine.detect_rpm_changes(&sys);
        assert_eq!(events.len(), 1);
        if let Event::RpmUpdate { group, rpms } = &events[0] {
            assert_eq!(*group, GroupId::new(1));
            assert_eq!(rpms[0], 1500);
        } else {
            panic!("expected RpmUpdate, got {:?}", events[0]);
        }
    }

    /// Device disappearing emits DeviceDisconnected.
    #[test]
    fn rpm_device_disconnected() {
        let mut engine = Engine::new(2000);
        let mut sys = system_with_two_devices();

        engine.detect_rpm_changes(&sys); // sees both groups

        // Remove group 2 by refreshing with only group 1
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        sys.registry.refresh(
            BackendId(0),
            vec![frgb_core::DiscoveredDevice {
                id: DeviceId::from([0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]),
                fans_type: [21, 21, 21, 0],
                dev_type: 0,
                group: GroupId::new(1),
                fan_count: 3,
                master: our_mac,
                fans_rpm: [1200, 1200, 1200, 0],
                fans_pwm: [0; 4],
                cmd_seq: 0,
                channel: 0x08,
            }],
            our_mac,
            &sys.specs,
        );

        let events = engine.detect_rpm_changes(&sys);
        let disconnected = events
            .iter()
            .any(|e| matches!(e, Event::DeviceDisconnected { group } if *group == GroupId::new(2)));
        assert!(
            disconnected,
            "expected DeviceDisconnected for group 2, got: {:?}",
            events
        );
    }

    /// New device appearing emits DeviceConnected + RpmUpdate.
    #[test]
    fn rpm_new_device_connected() {
        let mut engine = Engine::new(2000);
        let mut sys = system_with_device();

        engine.detect_rpm_changes(&sys); // sees group 1

        // Add group 2
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        sys.registry.refresh(
            BackendId(0),
            vec![
                frgb_core::DiscoveredDevice {
                    id: DeviceId::from([0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]),
                    fans_type: [21, 21, 21, 0],
                    dev_type: 0,
                    group: GroupId::new(1),
                    fan_count: 3,
                    master: our_mac,
                    fans_rpm: [1200, 1200, 1200, 0],
                    fans_pwm: [0; 4],
                    cmd_seq: 0,
                    channel: 0x08,
                },
                frgb_core::DiscoveredDevice {
                    id: DeviceId::from([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]),
                    fans_type: [42, 41, 0, 0],
                    dev_type: 0,
                    group: GroupId::new(2),
                    fan_count: 2,
                    master: our_mac,
                    fans_rpm: [900, 900, 0, 0],
                    fans_pwm: [0; 4],
                    cmd_seq: 0,
                    channel: 0x08,
                },
            ],
            our_mac,
            &sys.specs,
        );

        let events = engine.detect_rpm_changes(&sys);
        let connected = events
            .iter()
            .any(|e| matches!(e, Event::DeviceConnected { group } if *group == GroupId::new(2)));
        let rpm_update = events
            .iter()
            .any(|e| matches!(e, Event::RpmUpdate { group, .. } if *group == GroupId::new(2)));
        assert!(connected, "expected DeviceConnected for group 2");
        assert!(rpm_update, "expected RpmUpdate for group 2");
    }

    // -----------------------------------------------------------------------
    // Tick — basic behavior
    // -----------------------------------------------------------------------

    /// Tick with no devices and no sub-engines returns empty events.
    #[test]
    fn tick_empty_system_no_events() {
        let mut engine = Engine::new(2000);
        let mut sys = empty_system();
        let events = engine.tick(&mut sys, &test_config());
        assert!(events.is_empty());
    }

    /// Source: INFERRED — Tick respects poll_interval: periodic tasks only run
    /// when elapsed time exceeds the interval. With a 2s interval and immediate
    /// successive ticks, only the second tick after the interval triggers discovery.
    #[test]
    fn tick_does_not_poll_before_interval() {
        let mut engine = Engine::new(60_000); // 60 second interval
        let mut sys = system_with_device();

        // First tick: last_poll is "now", so elapsed < 60s → no periodic tasks
        let events = engine.tick(&mut sys, &test_config());
        // Sequencer is inactive, no periodic tasks fire → empty
        assert!(events.is_empty());
    }

    // -----------------------------------------------------------------------
    // Curve integration — curves active during tick
    // -----------------------------------------------------------------------

    /// When a curve is active and sensors are dirty, tick
    /// produces CurveApplied events (even though set_speed fails without a backend).
    #[test]
    fn tick_with_active_curve_evaluates() {
        let mut engine = Engine::new(0); // poll immediately
        let mut sys = system_with_device();

        // Activate a curve for group 1
        let curve = FanCurve {
            points: vec![
                CurvePoint {
                    temp: Temperature::new(30),
                    speed: SpeedPercent::new(30),
                },
                CurvePoint {
                    temp: Temperature::new(70),
                    speed: SpeedPercent::new(80),
                },
            ],
            sensor: Sensor::Cpu,
            interpolation: Interpolation::Linear,
            min_speed: SpeedPercent::new(25),
            stop_below: None,
            ramp_rate: None,
        };
        engine.curves.set_curve(GroupId::new(1), curve);

        // Inject a sensor reading (using the #[cfg(test)] method on CurveRunner)
        engine.curves.inject_sensor(Sensor::Cpu, 50.0);

        // Tick — should evaluate the curve and emit CurveApplied
        // Note: last_poll starts at Instant::now(), so with 0ms interval it fires immediately
        // BUT the tick needs elapsed >= 0, and last_poll was just set. Force it by setting
        // last_poll to the past.
        engine.last_poll = Instant::now() - Duration::from_secs(1);

        let events = engine.tick(&mut sys, &test_config());
        let curve_applied = events
            .iter()
            .any(|e| matches!(e, Event::CurveApplied { group, .. } if *group == GroupId::new(1)));
        assert!(curve_applied, "expected CurveApplied event, got: {:?}", events);
    }

    // -----------------------------------------------------------------------
    // Hwmon / sensors
    // -----------------------------------------------------------------------

    /// hwmon_chips() returns empty by default (no init_hwmon called).
    #[test]
    fn hwmon_chips_empty_by_default() {
        let engine = Engine::new(2000);
        assert!(engine.hwmon_chips().is_empty());
    }

    /// sensor_calibration returns default when not configured.
    #[test]
    fn sensor_calibration_default() {
        let engine = Engine::new(2000);
        let cal = engine.sensor_calibration();
        assert!((cal.cpu_offset - 0.0).abs() < f32::EPSILON);
        assert!((cal.gpu_offset - 0.0).abs() < f32::EPSILON);
    }

    // -----------------------------------------------------------------------
    // Sub-engine state after construction
    // -----------------------------------------------------------------------

    /// bind_scanning defaults to false.
    #[test]
    fn bind_scanning_defaults_false() {
        let engine = Engine::new(2000);
        assert!(!engine.bind_scanning);
    }

    /// Source: INFERRED — All sub-engines start inactive after construction.
    #[test]
    fn sub_engines_inactive_at_start() {
        let engine = Engine::new(2000);
        assert!(!engine.curves.is_active());
        assert!(!engine.show_runner.is_active());
        // alerts/power/scheduler don't have is_active but we verify their
        // config-derived state is empty
        assert!(engine.scheduler.entries().is_empty());
        assert!(engine.hwmon_chips().is_empty());
    }

    // -----------------------------------------------------------------------
    // RPM history tracking
    // -----------------------------------------------------------------------

    /// RPM history grows with each detect_rpm_changes call.
    #[test]
    fn rpm_history_tracks_readings() {
        let mut engine = Engine::new(2000);
        let sys = system_with_device();

        assert!(engine.rpm_history.is_empty());

        engine.detect_rpm_changes(&sys);
        let len1 = engine.rpm_history.get(&GroupId::new(1)).map(|h| h.len()).unwrap_or(0);
        assert_eq!(len1, 1, "history should have 1 entry after first call");

        engine.detect_rpm_changes(&sys);
        let len2 = engine.rpm_history.get(&GroupId::new(1)).map(|h| h.len()).unwrap_or(0);
        assert_eq!(len2, 2, "history should have 2 entries after second call");
    }

    /// Anomaly detector emits RpmAnomaly when RPM drops >20%.
    #[test]
    fn rpm_anomaly_detects_drop() {
        let mut engine = Engine::new(2000);
        let group = GroupId::new(1);

        // Inject history: first half ~1200 RPM, second half ~800 RPM (33% drop)
        let mut history = VecDeque::new();
        for _ in 0..5 {
            history.push_back(1200u16);
        }
        for _ in 0..5 {
            history.push_back(800u16);
        }
        engine.rpm_history.insert(group, history);

        let events = engine.check_rpm_anomalies();
        assert!(!events.is_empty(), "expected at least one RpmAnomaly event");
        let anomaly = events
            .iter()
            .find(|e| matches!(e, Event::RpmAnomaly { group: g, .. } if *g == group));
        assert!(anomaly.is_some(), "expected RpmAnomaly for group 1, got: {:?}", events);
    }

    // -----------------------------------------------------------------------
    // Wear counter tracking
    // -----------------------------------------------------------------------

    /// Wear counters start empty and increment for groups with non-zero RPM.
    #[test]
    fn wear_counter_increments_for_running_fans() {
        let mut engine = Engine::new(2000);
        let sys = system_with_device();

        // Seed last_rpms by running detect_rpm_changes first
        engine.detect_rpm_changes(&sys);
        assert!(engine.wear_counters.is_empty(), "no wear increments before tick");

        // Simulate the periodic wear increment (poll_interval = 2s)
        let poll_secs = engine.poll_interval.as_secs();
        for &(group, rpms) in &engine.last_rpms {
            if rpms.iter().any(|&r| r > 0) {
                *engine.wear_counters.entry(group).or_insert(0) += poll_secs;
            }
        }

        // Group 1 has fans running at 1200 RPM → should be incremented
        let secs = engine.wear_counters.get(&GroupId::new(1)).copied().unwrap_or(0);
        assert_eq!(secs, 2, "expected 2s of wear for group 1 after one poll tick");
    }

    /// load_wear_stats restores counters from persisted config entries.
    #[test]
    fn load_wear_stats_restores_counters() {
        let mut engine = Engine::new(2000);

        let entries = vec![
            frgb_model::config::WearEntry {
                group_id: GroupId::new(1),
                running_seconds: 3600,
            },
            frgb_model::config::WearEntry {
                group_id: GroupId::new(2),
                running_seconds: 7200,
            },
        ];
        engine.load_wear_stats(&entries);

        let stats = engine.wear_stats();
        assert_eq!(stats.get(&GroupId::new(1)).copied(), Some(3600));
        assert_eq!(stats.get(&GroupId::new(2)).copied(), Some(7200));
    }

    /// wear_entries() round-trips the wear counters.
    #[test]
    fn wear_entries_round_trip() {
        let mut engine = Engine::new(2000);
        engine.wear_counters.insert(GroupId::new(3), 100);
        engine.wear_counters.insert(GroupId::new(4), 200);

        let entries = engine.wear_entries();
        assert_eq!(entries.len(), 2);
        let found3 = entries
            .iter()
            .find(|e| e.group_id == GroupId::new(3))
            .map(|e| e.running_seconds);
        let found4 = entries
            .iter()
            .find(|e| e.group_id == GroupId::new(4))
            .map(|e| e.running_seconds);
        assert_eq!(found3, Some(100));
        assert_eq!(found4, Some(200));
    }

    // -----------------------------------------------------------------------
    // Sensor peak tracking
    // -----------------------------------------------------------------------

    /// sensor_peaks tracks the maximum observed temperature per sensor.
    #[test]
    fn sensor_peaks_track_max() {
        use frgb_core::backend::{SensorReading, SensorUnit};

        let mut engine = Engine::new(2000);

        // Simulate three rounds of sensor readings for CPU.
        // Label must match a pattern recognised by classify_sensor (k10temp:Tctl → Sensor::Cpu).
        let readings_a = vec![SensorReading {
            label: "k10temp:Tctl".to_string(),
            value: 55.0,
            unit: SensorUnit::Celsius,
        }];
        let readings_b = vec![SensorReading {
            label: "k10temp:Tctl".to_string(),
            value: 72.0,
            unit: SensorUnit::Celsius,
        }];
        let readings_c = vec![SensorReading {
            label: "k10temp:Tctl".to_string(),
            value: 60.0,
            unit: SensorUnit::Celsius,
        }];

        for reading in &readings_a {
            if let Some(sensor) = frgb_core::hwmon::classify_sensor(&reading.label) {
                let peak = engine.sensor_peaks.entry(sensor).or_insert(0.0);
                *peak = peak.max(reading.value as f32);
            }
        }
        for reading in &readings_b {
            if let Some(sensor) = frgb_core::hwmon::classify_sensor(&reading.label) {
                let peak = engine.sensor_peaks.entry(sensor).or_insert(0.0);
                *peak = peak.max(reading.value as f32);
            }
        }
        for reading in &readings_c {
            if let Some(sensor) = frgb_core::hwmon::classify_sensor(&reading.label) {
                let peak = engine.sensor_peaks.entry(sensor).or_insert(0.0);
                *peak = peak.max(reading.value as f32);
            }
        }

        // Peak must be 72.0 — the highest reading, not the latest.
        let peak = engine.sensor_peaks.get(&Sensor::Cpu).copied();
        assert_eq!(peak, Some(72.0), "expected peak of 72.0°C, got {:?}", peak);
    }

    // -----------------------------------------------------------------------
    // RPM anomaly E2E — detect_rpm_changes feeds history, check_rpm_anomalies fires
    // -----------------------------------------------------------------------

    /// Simulate RPM degradation over multiple detect_rpm_changes cycles, then
    /// verify check_rpm_anomalies emits an event.
    #[test]
    fn rpm_anomaly_e2e_via_detect_and_check() {
        let mut engine = Engine::new(2000);
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        let specs = load_defaults();

        // Phase 1: stable at 1200 RPM for several readings
        for _ in 0..5 {
            let mut sys = System::new(specs.clone());
            sys.registry.refresh(
                BackendId(0),
                vec![frgb_core::DiscoveredDevice {
                    id: DeviceId::from([0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]),
                    fans_type: [21, 21, 21, 0],
                    dev_type: 0,
                    group: GroupId::new(1),
                    fan_count: 3,
                    master: our_mac,
                    fans_rpm: [1200, 1200, 1200, 0],
                    fans_pwm: [0; 4],
                    cmd_seq: 0,
                    channel: 0x08,
                }],
                our_mac,
                &sys.specs,
            );
            engine.detect_rpm_changes(&sys);
        }

        // Phase 2: degraded to 750 RPM (~37% drop) for several readings
        for _ in 0..5 {
            let mut sys = System::new(specs.clone());
            sys.registry.refresh(
                BackendId(0),
                vec![frgb_core::DiscoveredDevice {
                    id: DeviceId::from([0xc8, 0xb4, 0xef, 0x62, 0x32, 0xe1]),
                    fans_type: [21, 21, 21, 0],
                    dev_type: 0,
                    group: GroupId::new(1),
                    fan_count: 3,
                    master: our_mac,
                    fans_rpm: [750, 750, 750, 0],
                    fans_pwm: [0; 4],
                    cmd_seq: 0,
                    channel: 0x08,
                }],
                our_mac,
                &sys.specs,
            );
            engine.detect_rpm_changes(&sys);
        }

        // check_rpm_anomalies should detect the >20% drop
        let events = engine.check_rpm_anomalies();
        let has_anomaly = events
            .iter()
            .any(|e| matches!(e, Event::RpmAnomaly { group, .. } if *group == GroupId::new(1)));
        assert!(
            has_anomaly,
            "expected RpmAnomaly for group 1 after 37% RPM drop, got: {:?}",
            events
        );
    }

    // -----------------------------------------------------------------------
    // Wear counter persistence roundtrip
    // -----------------------------------------------------------------------

    /// Load → increment via detect_rpm_changes + manual tick → export → verify.
    #[test]
    fn wear_counter_persistence_roundtrip() {
        let mut engine = Engine::new(2000);
        let sys = system_with_device();

        // Step 1: Load persisted wear stats (simulating daemon startup)
        let initial_entries = vec![frgb_model::config::WearEntry {
            group_id: GroupId::new(1),
            running_seconds: 3600,
        }];
        engine.load_wear_stats(&initial_entries);
        assert_eq!(
            engine.wear_stats().get(&GroupId::new(1)).copied(),
            Some(3600),
            "initial wear should be 3600s"
        );

        // Step 2: detect_rpm_changes → seeds last_rpms with non-zero RPMs
        engine.detect_rpm_changes(&sys);

        // Step 3: Simulate one poll interval of wear accumulation
        let poll_secs = engine.poll_interval.as_secs();
        for &(group, ref rpms) in &engine.last_rpms.clone() {
            if rpms.iter().any(|&r| r > 0) {
                *engine.wear_counters.entry(group).or_insert(0) += poll_secs;
            }
        }

        // Step 4: Export and verify
        let entries = engine.wear_entries();
        let entry = entries
            .iter()
            .find(|e| e.group_id == GroupId::new(1))
            .expect("should have wear entry for group 1");
        assert_eq!(
            entry.running_seconds,
            3600 + 2,
            "wear should be initial 3600 + one poll interval (2s)"
        );
    }

    // -----------------------------------------------------------------------
    // RPM anomaly debounce — edge-trigger semantics
    // -----------------------------------------------------------------------

    #[test]
    fn rpm_anomaly_fires_once_per_degradation_episode() {
        let mut engine = Engine::new(2000);
        let group = GroupId::new(1);

        // Build history: 5 readings near 1200 RPM, then 5 degraded near 750 RPM.
        let mut history = VecDeque::new();
        for _ in 0..5 { history.push_back(1200u16); }
        for _ in 0..5 { history.push_back(750u16); }
        engine.rpm_history.insert(group, history);

        // First check: anomaly fires.
        let events1 = engine.check_rpm_anomalies();
        let count1 = events1.iter()
            .filter(|e| matches!(e, Event::RpmAnomaly { group: g, .. } if *g == group))
            .count();
        assert_eq!(count1, 1, "expected 1 anomaly on first detection");

        // Second check (history unchanged — still degraded): no new event.
        let events2 = engine.check_rpm_anomalies();
        let count2 = events2.iter()
            .filter(|e| matches!(e, Event::RpmAnomaly { group: g, .. } if *g == group))
            .count();
        assert_eq!(count2, 0, "expected 0 anomalies on re-check (still degraded)");
    }

    #[test]
    fn rpm_anomaly_re_fires_after_recovery() {
        let mut engine = Engine::new(2000);
        let group = GroupId::new(1);

        // First episode.
        let mut history = VecDeque::new();
        for _ in 0..5 { history.push_back(1200u16); }
        for _ in 0..5 { history.push_back(750u16); }
        engine.rpm_history.insert(group, history);
        let events1 = engine.check_rpm_anomalies();
        assert_eq!(
            events1.iter().filter(|e| matches!(e, Event::RpmAnomaly { .. })).count(),
            1
        );

        // Recovery — replace history with healthy readings.
        let mut healthy = VecDeque::new();
        for _ in 0..10 { healthy.push_back(1200u16); }
        engine.rpm_history.insert(group, healthy);
        let _events_recovered = engine.check_rpm_anomalies();

        // Second episode — fresh degradation.
        let mut degraded2 = VecDeque::new();
        for _ in 0..5 { degraded2.push_back(1200u16); }
        for _ in 0..5 { degraded2.push_back(750u16); }
        engine.rpm_history.insert(group, degraded2);
        let events3 = engine.check_rpm_anomalies();
        assert_eq!(
            events3.iter().filter(|e| matches!(e, Event::RpmAnomaly { .. })).count(),
            1,
            "expected anomaly to re-fire after recovery"
        );
    }

    #[test]
    fn rpm_anomaly_groups_independent() {
        let mut engine = Engine::new(2000);
        let g1 = GroupId::new(1);
        let g2 = GroupId::new(2);

        // g1 degraded.
        let mut h1 = VecDeque::new();
        for _ in 0..5 { h1.push_back(1200u16); }
        for _ in 0..5 { h1.push_back(750u16); }
        engine.rpm_history.insert(g1, h1);

        // g2 healthy.
        let mut h2 = VecDeque::new();
        for _ in 0..10 { h2.push_back(1200u16); }
        engine.rpm_history.insert(g2, h2);

        let events = engine.check_rpm_anomalies();
        let g1_events = events.iter()
            .filter(|e| matches!(e, Event::RpmAnomaly { group, .. } if *group == g1))
            .count();
        let g2_events = events.iter()
            .filter(|e| matches!(e, Event::RpmAnomaly { group, .. } if *group == g2))
            .count();
        assert_eq!(g1_events, 1);
        assert_eq!(g2_events, 0);
    }

}
