//! Request handler — dispatches IPC Request → System/Engine → Response.

use frgb_core::System;
use frgb_model::config::FirmwareInfo;
use frgb_model::config::{GroupSnapshot, GroupStatus, SyncConfig};
use frgb_model::ipc::{Event, Request, Response};

use crate::engine::Engine;

/// Validate a user-provided file path for config export/import.
///
/// Defense in depth:
/// 1. This function rejects parents that resolve outside the user's home or
///    `/tmp` (`..`-traversal protection).
/// 2. The caller MUST open the file with `libc::O_NOFOLLOW` to defeat symlink
///    TOCTOU at the leaf component. See ExportConfig/ImportConfig arms below.
/// 3. Parent canonicalization is itself non-atomic; the leaf-level
///    `O_NOFOLLOW` is the load-bearing defense if a co-resident user can
///    rewrite paths between the canonicalize and open syscalls.
fn validate_config_path(path: &str) -> Result<(), String> {
    let p = std::path::Path::new(path);
    // Resolve parent to check it exists (full canonicalize requires the file to exist)
    let parent = p.parent().unwrap_or(p);
    let resolved = if parent.exists() {
        match parent.canonicalize() {
            Ok(c) => c,
            Err(e) => return Err(format!("cannot resolve path: {e}")),
        }
    } else {
        return Err(format!("parent directory does not exist: {}", parent.display()));
    };

    let home = dirs::home_dir();
    let tmp = std::path::Path::new("/tmp");
    let allowed = home.as_ref().is_some_and(|h| resolved.starts_with(h)) || resolved.starts_with(tmp);
    if !allowed {
        return Err(format!(
            "path must be within home directory or /tmp, got: {}",
            resolved.display()
        ));
    }
    Ok(())
}

/// Handle a single IPC request. Returns a response and optional events to broadcast.
#[tracing::instrument(skip_all, fields(request = ?std::mem::discriminant(request)))]
pub fn handle(
    system: &mut System,
    engine: &mut Engine,
    config: &mut crate::config_cache::ConfigCache,
    request: &Request,
) -> (Response, Vec<Event>) {
    let mut events = Vec::new();

    let response = match request {
        Request::Status | Request::StatusVerbose => Response::DeviceStatus(build_status_list(system, true)),
        Request::Discover => handle_discover(system),
        Request::Indicate { group, duration_secs } => {
            // Flash group white to identify physical location, restore after timeout.
            use frgb_model::rgb::{Rgb, RgbMode, Ring};
            let prev = system
                .devices()
                .iter()
                .find(|d| d.group == *group)
                .and_then(|d| d.state.rgb_mode.clone())
                .unwrap_or(RgbMode::Off);
            let flash = RgbMode::Static {
                ring: Ring::Both,
                color: Rgb::WHITE,
                brightness: frgb_model::Brightness::new(255),
            };
            match system.set_rgb(*group, &flash) {
                Ok(()) => {
                    let secs = (*duration_secs).clamp(1, 10) as u64;
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
                    // Replace existing entry for same group (re-indicate extends timeout)
                    engine.indicate_restores.retain(|&(_, g, _)| g != *group);
                    engine.indicate_restores.push((deadline, *group, prev));
                    Response::Ok
                }
                Err(e) => Response::Error(e.to_string()),
            }
        }
        Request::ListGroups => Response::GroupList(system.devices().iter().map(fan_group_from).collect()),
        Request::SetSpeed { group, mode } => {
            if let Err(e) = mode.validate() {
                return (Response::Error(format!("invalid speed mode: {e}")), events);
            }
            // When switching to Manual, disable MB sync first so firmware accepts speed commands
            if matches!(mode, frgb_model::speed::SpeedMode::Manual(_)) {
                engine.curves.remove_curve(*group);
                let _ = system.set_mb_sync(*group, false, None);
            }
            // NamedCurve: look up the curve and activate the curve runner
            if let frgb_model::speed::SpeedMode::NamedCurve(name) = mode {
                engine.curves.remove_curve(*group);
                {
                    let cfg = config.config();
                    if let Some(nc) = cfg.saved_curves.iter().find(|c| c.name == *name) {
                        engine.curves.set_curve(*group, nc.curve.clone());
                        tracing::info!("curve '{}' activated for group {}", name, group);
                    } else {
                        return (Response::Error(format!("curve '{}' not found", name)), events);
                    }
                }
                events.push(Event::SpeedChanged {
                    group: *group,
                    mode: mode.clone(),
                });
                Response::Ok
            } else if let frgb_model::speed::SpeedMode::Curve(ref curve) = mode {
                // Inline curve: activate curve runner directly
                engine.curves.remove_curve(*group);
                engine.curves.set_curve(*group, curve.clone());
                events.push(Event::SpeedChanged {
                    group: *group,
                    mode: mode.clone(),
                });
                Response::Ok
            } else {
                engine.curves.remove_curve(*group);
                match system.set_speed(*group, mode) {
                    Ok(()) => {
                        events.push(Event::SpeedChanged {
                            group: *group,
                            mode: mode.clone(),
                        });
                        Response::Ok
                    }
                    Err(e) => Response::Error(e.to_string()),
                }
            }
        }
        Request::SetSpeedAll { target, mode } => {
            // MB sync groups are controlled by motherboard PWM.
            // SetSpeedAll must skip them; only an explicit per-group mode change toggles MB sync.
            if let Err(e) = mode.validate() {
                return (Response::Error(format!("invalid speed mode: {e}")), events);
            }
            let groups = resolve_target_groups(system, Some(target));
            let mut errors = Vec::new();
            let mut skipped_mb_sync = Vec::new();
            for gid in &groups {
                // Skip RGB-only motherboard groups (AURA) — they have no fan speed.
                if !system.is_fan_capable(*gid) {
                    continue;
                }
                // Skip groups under MB sync — their speed is controlled by motherboard PWM
                let is_mb_sync = system.devices().iter().any(|d| d.group == *gid && d.mb_sync);
                if is_mb_sync {
                    skipped_mb_sync.push(*gid);
                    continue;
                }
                engine.curves.remove_curve(*gid);
                match system.set_speed(*gid, mode) {
                    Ok(()) => events.push(Event::SpeedChanged {
                        group: *gid,
                        mode: mode.clone(),
                    }),
                    Err(e) => errors.push(format!("group {gid}: {e}")),
                }
            }
            if !errors.is_empty() {
                Response::Error(errors.join("; "))
            } else if !skipped_mb_sync.is_empty() {
                let ids: Vec<String> = skipped_mb_sync.iter().map(|g| g.to_string()).collect();
                tracing::info!("SetSpeedAll: skipped MB sync groups: {}", ids.join(", "));
                Response::Ok
            } else {
                Response::Ok
            }
        }
        Request::SetPumpMode { group, mode } => {
            if let Err(msg) = mode.validate() {
                return (Response::Error(msg), events);
            }
            match system.set_pump(*group, mode) {
                Ok(()) => {
                    tracing::info!("SetPumpMode: group={group} mode={mode:?}");
                    Response::Ok
                }
                Err(e) => Response::Error(format!("set pump mode: {e}")),
            }
        }
        Request::SetRgb { group, mode } => match system.set_rgb(*group, mode) {
            Ok(()) => {
                events.push(Event::RgbChanged {
                    group: *group,
                    mode: mode.clone(),
                });
                Response::Ok
            }
            Err(e) => Response::Error(e.to_string()),
        },
        Request::SetRgbAll { target, mode } => {
            let groups = resolve_target_groups(system, Some(target));
            let mut errors = Vec::new();
            for gid in &groups {
                match system.set_rgb(*gid, mode) {
                    Ok(()) => events.push(Event::RgbChanged {
                        group: *gid,
                        mode: mode.clone(),
                    }),
                    Err(e) => errors.push(format!("group {gid}: {e}")),
                }
            }
            if errors.is_empty() {
                Response::Ok
            } else {
                Response::Error(errors.join("; "))
            }
        }
        // Source: INFERRED — SetBrightness modifies brightness in the existing RgbMode
        // per target group via read-modify-write. Keeps state consistent — no separate
        // brightness overlay that diverges from the mode's own brightness field.
        Request::SetBrightness { target, level } => {
            let groups = resolve_target_groups(system, Some(target));
            let level = *level;
            let mut errors = Vec::new();
            for gid in &groups {
                let current = system
                    .devices()
                    .iter()
                    .find(|d| d.group == *gid)
                    .and_then(|d| d.state.rgb_mode.clone());
                let updated = match current {
                    Some(mode) => with_brightness(mode, level),
                    None => continue, // No mode set, nothing to dim
                };
                match system.set_rgb(*gid, &updated) {
                    Ok(()) => events.push(Event::RgbChanged {
                        group: *gid,
                        mode: updated,
                    }),
                    Err(e) => errors.push(format!("group {gid}: {e}")),
                }
            }
            if errors.is_empty() {
                Response::Ok
            } else {
                Response::Error(errors.join("; "))
            }
        }
        Request::GetFirmwareInfo => {
            let rf = system.rf_ext();
            let tx_fw = rf.and_then(|rf| rf.tx_firmware_version());
            let tx_mac = rf.and_then(|rf| rf.tx_id());
            let info = FirmwareInfo {
                tx_version: tx_fw
                    .map(|v| format!("{}.{}", v >> 8, v & 0xFF))
                    .unwrap_or_else(|| "unknown".into()),
                // RX firmware not queryable via RF protocol; report TX dongle MAC instead
                rx_version: tx_mac
                    .map(|id| {
                        // Format as colon-separated MAC for readability
                        let hex = id.to_hex();
                        hex.as_bytes()
                            .chunks(2)
                            .map(|c| std::str::from_utf8(c).unwrap_or("??"))
                            .collect::<Vec<_>>()
                            .join(":")
                    })
                    .unwrap_or_else(|| "n/a".into()),
            };
            Response::FirmwareInfo(info)
        }
        Request::Bind { group, lock } => {
            if let Some(rf) = system.rf_ext() {
                if let Some(dev) = system.unbound.first() {
                    let mac = dev.mac;
                    match rf.bind_device(&mac, *group) {
                        Ok(()) => {
                            if *lock {
                                let _ = rf.lock();
                            }
                            Response::Ok
                        }
                        Err(e) => Response::Error(e.to_string()),
                    }
                } else {
                    Response::Error("no unbound devices".into())
                }
            } else {
                Response::Error("backend does not support bind".into())
            }
        }
        Request::SaveProfile { name } => {
            let groups: Vec<GroupSnapshot> = system
                .devices()
                .iter()
                .map(|d| GroupSnapshot {
                    group_id: d.group,
                    scene: d.current_scene(),
                })
                .collect();
            let group_count = groups.len();
            let profile = frgb_model::config::Profile {
                name: name.clone(),
                groups,
                effect_cycle: None,
                sequences: Vec::new(),
            };
            config.config_mut().upsert_profile(profile);
            config.flush();
            tracing::info!("Profile '{}' saved ({} groups)", name, group_count);
            Response::Ok
        }

        // --- Sequence control ---
        Request::ListSequences => {
            let cfg = config.config();
            Response::SequenceList(cfg.sequences.clone())
        }
        Request::SaveSequence { sequence } => {
            {
                let cfg = config.config_mut();
                if let Some(existing) = cfg.sequences.iter_mut().find(|s| s.name == sequence.name) {
                    *existing = sequence.clone();
                } else {
                    cfg.sequences.push(sequence.clone());
                }
            }
            engine.show_runner.load_sequences(config.config());
            config.flush();
            tracing::info!("Sequence '{}' saved ({} steps)", sequence.name, sequence.steps.len());
            Response::Ok
        }
        Request::DeleteSequence { name } => {
            config.config_mut().sequences.retain(|s| s.name != *name);
            engine.show_runner.load_sequences(config.config());
            config.flush();
            Response::Ok
        }
        Request::SetEffectCycle { cycle } => {
            let groups = system.group_ids();
            engine.show_runner.start_cycle(&groups, cycle.clone());
            tracing::info!(
                "Effect cycle started ({} steps, {} groups)",
                cycle.steps.len(),
                groups.len()
            );
            Response::Ok
        }
        Request::StartSequence { name, target } => match engine.show_runner.get_sequence(name) {
            Some(seq) => {
                let seq = std::sync::Arc::clone(seq);
                let groups = resolve_target_groups(system, target.as_ref());
                engine.show_runner.start_sequence(&groups, seq);
                tracing::info!("Sequence '{}' started on {} groups", name, groups.len());
                Response::Ok
            }
            None => Response::Error(format!("sequence '{}' not found in config", name)),
        },
        Request::StopSequence { target } => {
            let groups = resolve_target_groups(system, target.as_ref());
            events.extend(engine.show_runner.stop(&groups));
            Response::Ok
        }
        Request::StopAllSequences => {
            events.extend(engine.show_runner.stop(&[]));
            Response::Ok
        }

        // --- Sensors ---
        Request::ListSensors => {
            let readings = frgb_core::hwmon::read_calibrated(engine.hwmon_chips(), engine.sensor_calibration());
            let infos: Vec<frgb_model::sensor::SensorInfo> = readings
                .iter()
                .filter_map(|r| {
                    frgb_core::hwmon::classify_sensor(&r.label).map(|sensor| frgb_model::sensor::SensorInfo {
                        sensor,
                        name: r.label.clone(),
                        current: r.value as f32,
                        available: true,
                    })
                })
                .collect();
            Response::SensorList(infos)
        }
        Request::GetSensorReading { sensor } => {
            let readings = frgb_core::hwmon::read_calibrated(
                engine.hwmon_chips(),
                &frgb_model::sensor::SensorCalibration::default(),
            );
            let found = readings
                .iter()
                .find(|r| frgb_core::hwmon::classify_sensor(&r.label).as_ref() == Some(sensor));
            match found {
                Some(r) => Response::SensorReading {
                    sensor: sensor.clone(),
                    value: r.value as f32,
                },
                None => Response::Error(format!("sensor {:?} not available", sensor)),
            }
        }

        // --- LCD ---
        Request::ListLcdDevices => Response::LcdDevices(system.lcd_device_info()),
        Request::SetLcd { lcd_index, config } => match engine.lcd_manager.device_id_by_index(*lcd_index) {
            Some(lcd_id) => {
                if let frgb_model::lcd::LcdContent::Preset(ref preset) = config.content {
                    engine.lcd_manager.load_preset(lcd_id, preset);
                }
                engine.lcd_manager.set_config(lcd_id, config.clone());
                Response::Ok
            }
            None => Response::Error(format!("no LCD device at index {lcd_index}")),
        },
        Request::ListPresets => Response::Presets(engine.lcd_manager.list_presets()),

        // --- Config ---
        Request::ReloadConfig => {
            config.reload();
            let cfg = config.config();
            engine.show_runner.load_sequences(cfg);
            engine.curves.sync_from_config(system, cfg);
            tracing::info!("Config reloaded");
            Response::Ok
        }

        // --- Profiles ---
        Request::ListProfiles => {
            let cfg = config.config();
            Response::ProfileList(cfg.profiles.iter().map(|p| p.name.to_string()).collect())
        }
        Request::DeleteProfile { name } => {
            config.config_mut().profiles.retain(|p| p.name != *name);
            config.flush();
            Response::Ok
        }
        Request::SwitchProfile { name } => {
            let cfg = config.config();
            match cfg.profiles.iter().find(|p| p.name == *name) {
                Some(profile) => {
                    crate::engine::apply_profile_groups(system, &profile.groups);
                    events.push(Event::ProfileSwitched { name: name.clone() });
                    Response::Ok
                }
                None => Response::Error(format!("profile '{}' not found", name)),
            }
        }

        // --- Curves ---
        Request::ListCurves => Response::CurveList(config.config().saved_curves.clone()),
        Request::SaveCurve { name, curve } => {
            if let Err(e) = curve.validate() {
                return (Response::Error(format!("invalid curve: {e}")), events);
            }
            {
                let cfg = config.config_mut();
                let nc = frgb_model::config::NamedCurve {
                    name: name.clone(),
                    curve: curve.clone(),
                };
                if let Some(existing) = cfg.saved_curves.iter_mut().find(|c| c.name == *name) {
                    *existing = nc;
                } else {
                    cfg.saved_curves.push(nc);
                }
            }
            config.flush();
            Response::Ok
        }
        Request::DeleteCurve { name } => {
            config.config_mut().saved_curves.retain(|c| c.name != *name);
            config.flush();
            Response::Ok
        }

        // --- Group management ---
        Request::RenameGroup { group, name } => {
            if let Err(e) = frgb_model::ValidatedName::new(name.as_str()) {
                return (Response::Error(e), events);
            }
            if let Some(gc) = config.config_mut().groups.iter_mut().find(|g| g.id == *group) {
                gc.name = name.clone();
            }
            config.flush();
            if let Some(dev) = system.registry.find_by_group_mut(*group) {
                dev.name = name.clone();
            }
            Response::Ok
        }
        Request::SetRole { group, role } => {
            // Update in-memory registry first so the change is immediate.
            if let Some(dev) = system.registry.find_by_group_mut(*group) {
                dev.role = role.clone();
            }
            // Upsert the config entry so the role survives daemon restarts.
            // If a GroupConfig for this group doesn't exist yet (groups are lazily
            // tracked), build a minimal entry from the current device state.
            let cfg = config.config_mut();
            if let Some(gc) = cfg.groups.iter_mut().find(|g| g.id == *group) {
                gc.role = role.clone();
            } else if let Some(dev) = system.registry.find_by_group(*group) {
                cfg.groups.push(frgb_model::config::GroupConfig {
                    id: *group,
                    name: dev.name.clone(),
                    device_type: dev.device_type,
                    fan_count: dev.fan_count(),
                    role: role.clone(),
                    blade: dev.blade,
                    cfm_max: None,
                    excluded: false,
                    speed: dev
                        .state
                        .speed_percent
                        .map(frgb_model::speed::SpeedMode::Manual)
                        .unwrap_or(frgb_model::speed::SpeedMode::Pwm),
                    rgb: dev.state.rgb_mode.clone().unwrap_or(frgb_model::rgb::RgbMode::Off),
                    lcd: None,
                });
            }
            config.flush();
            Response::Ok
        }
        Request::ExcludeGroup { group } => {
            if let Some(gc) = config.config_mut().groups.iter_mut().find(|g| g.id == *group) {
                gc.excluded = true;
            }
            config.flush();
            Response::Ok
        }
        Request::IncludeGroup { group } => {
            if let Some(gc) = config.config_mut().groups.iter_mut().find(|g| g.id == *group) {
                gc.excluded = false;
            }
            config.flush();
            Response::Ok
        }
        Request::ForgetGroup { group } => {
            let cfg = config.config_mut();
            cfg.groups.retain(|g| g.id != *group);
            cfg.device_ids.retain(|d| d.group_id != *group);
            config.flush();
            Response::Ok
        }

        // --- Schedules ---
        Request::ListSchedule => Response::ScheduleList(engine.scheduler.entries().to_vec()),
        Request::AddSchedule { entry } => {
            if let Err(e) = entry.validate() {
                Response::Error(e)
            } else {
                engine.scheduler.add(entry.clone());
                persist_schedules(engine, config);
                tracing::info!("Schedule added: {:02}:{:02} {:?}", entry.hour, entry.minute, entry.days);
                Response::Ok
            }
        }
        Request::DeleteSchedule { index } => {
            if engine.scheduler.remove(*index) {
                persist_schedules(engine, config);
                Response::Ok
            } else {
                Response::Error(format!("schedule index {} out of bounds", index))
            }
        }
        Request::ClearSchedule => {
            engine.scheduler.clear();
            persist_schedules(engine, config);
            Response::Ok
        }

        // --- Alerts ---
        Request::GetAlertConfig => {
            let cfg = config.config();
            let alert_cfg = cfg.alerts.clone().unwrap_or(frgb_model::config::AlertConfig {
                temp_alerts: Vec::new(),
                fan_stall_detect: true,
                device_disconnect: true,
            });
            Response::AlertConfig(alert_cfg)
        }
        Request::SetAlertConfig { config: alert_cfg } => {
            engine.alerts.set_config(alert_cfg.clone());
            config.config_mut().alerts = Some(alert_cfg.clone());
            config.flush();
            tracing::info!(
                "Alert config updated (stall={}, disconnect={}, {} temp alerts)",
                alert_cfg.fan_stall_detect,
                alert_cfg.device_disconnect,
                alert_cfg.temp_alerts.len()
            );
            Response::Ok
        }

        // --- Daemon config ---
        Request::GetDaemonConfig => Response::DaemonConfig(Box::new(config.config().daemon.clone())),
        Request::SetDaemonConfig { config: daemon_cfg } => {
            config.config_mut().daemon = daemon_cfg.clone();
            config.flush();
            engine.poll_interval = std::time::Duration::from_millis(daemon_cfg.poll_interval_ms as u64);
            tracing::info!("Daemon config updated (poll={}ms)", daemon_cfg.poll_interval_ms);
            Response::Ok
        }
        Request::SetSensorCalibration { cal } => {
            {
                let cfg = config.config_mut();
                // Preserve existing custom_paths that the GUI doesn't manage
                let mut merged = cal.clone();
                merged.custom_paths = cfg.sensor_calibration.custom_paths.clone();
                cfg.sensor_calibration = merged;
            }
            config.flush();
            engine.init_hwmon(cal.clone());
            tracing::info!(
                "Sensor calibration updated (cpu={:+.1}, gpu={:+.1})",
                cal.cpu_offset,
                cal.gpu_offset
            );
            Response::Ok
        }

        Request::SetPowerConfig { config: power_cfg } => {
            config.config_mut().power = Some(power_cfg.clone());
            config.flush();
            engine.power.set_config(power_cfg.clone());
            tracing::info!(
                "Power config updated (ac={:?}, battery={:?})",
                power_cfg.on_ac,
                power_cfg.on_battery
            );
            Response::Ok
        }

        // Subscribe is a no-op — daemon broadcasts all events to all clients unconditionally.
        // The client sends this to signal intent; we just acknowledge it.
        Request::Subscribe { .. } => Response::Ok,
        Request::Unsubscribe => Response::Ok,

        // Handshake — version negotiation
        Request::Hello { protocol_version } => {
            let client_ver = *protocol_version;
            if client_ver < frgb_model::ipc::PROTOCOL_VERSION_MIN {
                tracing::warn!(
                    "client protocol v{client_ver} too old (daemon requires v{}-v{})",
                    frgb_model::ipc::PROTOCOL_VERSION_MIN,
                    frgb_model::ipc::PROTOCOL_VERSION,
                );
                Response::Error(format!(
                    "client protocol v{client_ver} too old (daemon requires v{}-v{})",
                    frgb_model::ipc::PROTOCOL_VERSION_MIN,
                    frgb_model::ipc::PROTOCOL_VERSION,
                ))
            } else {
                // Negotiate: use the lower of client and daemon max
                let negotiated = client_ver.min(frgb_model::ipc::PROTOCOL_VERSION);
                tracing::info!(
                    "protocol negotiated v{negotiated} (client v{client_ver}, daemon v{})",
                    frgb_model::ipc::PROTOCOL_VERSION
                );
                Response::Hello {
                    protocol_version: negotiated,
                }
            }
        }

        // --- Newly implemented handlers ---

        Request::StopFans => {
            let groups = system.group_ids();
            let mode = frgb_model::speed::SpeedMode::Manual(frgb_model::SpeedPercent::new(0));
            let mut errors = Vec::new();
            for gid in &groups {
                if !system.is_fan_capable(*gid) { continue; }
                engine.curves.remove_curve(*gid);
                let _ = system.set_mb_sync(*gid, false, None);
                if let Err(e) = system.set_speed(*gid, &mode) {
                    errors.push(format!("group {gid}: {e}"));
                } else {
                    events.push(Event::SpeedChanged {
                        group: *gid,
                        mode: mode.clone(),
                    });
                }
            }
            if errors.is_empty() {
                tracing::info!("StopFans: {} fan-capable groups set to 0%", events.len());
                Response::Ok
            } else {
                Response::Error(errors.join("; "))
            }
        }

        Request::Unbind { group } => {
            if let Some(rf) = system.rf_ext() {
                let mac = system
                    .devices()
                    .iter()
                    .find(|d| d.group == *group)
                    .and_then(|d| d.mac_ids.first().copied());
                match mac {
                    Some(mac) => match rf.unbind_device(&mac, *group) {
                        Ok(()) => {
                            tracing::info!("Unbound device {} from group {}", mac.to_hex(), group);
                            Response::Ok
                        }
                        Err(e) => Response::Error(e.to_string()),
                    },
                    None => Response::Error(format!("no device found in group {group}")),
                }
            } else {
                Response::Error("backend does not support unbind".into())
            }
        }

        Request::CopyProfile { from, to } => {
            let validated_to = match frgb_model::ValidatedName::new(to.clone()) {
                Ok(v) => v,
                Err(e) => return (Response::Error(e), events),
            };
            let cfg = config.config();
            if let Some(src) = cfg.profiles.iter().find(|p| p.name == *from).cloned() {
                if cfg.profiles.iter().any(|p| p.name == *to) {
                    Response::Error(format!("profile '{}' already exists", to))
                } else {
                    let mut copy = src;
                    copy.name = validated_to;
                    config.config_mut().profiles.push(copy);
                    config.flush();
                    tracing::info!("Profile '{}' copied to '{}'", from, to);
                    Response::Ok
                }
            } else {
                Response::Error(format!("profile '{}' not found", from))
            }
        }

        Request::ExportConfig { path, compress: _ } => {
            if let Err(e) = validate_config_path(path) {
                return (Response::Error(e), events);
            }
            // NOTE: compress parameter ignored — JSON output only.
            let cfg = config.config();
            let json = match serde_json::to_string_pretty(cfg) {
                Ok(j) => j,
                Err(e) => return (Response::Error(format!("failed to serialize config: {e}")), events),
            };

            use std::io::Write as _;
            use std::os::unix::fs::OpenOptionsExt as _;
            let open_result = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(path);
            match open_result {
                Ok(mut file) => match file.write_all(json.as_bytes()) {
                    Ok(()) => {
                        tracing::info!("Config exported to {path}");
                        Response::Ok
                    }
                    Err(e) => Response::Error(format!("failed to write {path}: {e}")),
                },
                Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
                    Response::Error(format!("refusing to follow symlink at: {path}"))
                }
                Err(e) => Response::Error(format!("failed to open {path}: {e}")),
            }
        }

        Request::ImportConfig { path, merge } => {
            if let Err(e) = validate_config_path(path) {
                return (Response::Error(e), events);
            }
            use std::io::Read as _;
            use std::os::unix::fs::OpenOptionsExt as _;
            let open_result = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(path);
            let mut file = match open_result {
                Ok(f) => f,
                Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
                    return (Response::Error(format!("refusing to follow symlink at: {path}")), events);
                }
                Err(e) => return (Response::Error(format!("failed to open {path}: {e}")), events),
            };
            let mut json = String::new();
            if let Err(e) = file.read_to_string(&mut json) {
                return (Response::Error(format!("failed to read {path}: {e}")), events);
            }

            match serde_json::from_str::<frgb_model::config::Config>(&json) {
                Ok(imported) => {
                    if *merge {
                        // NOTE: merge strategy is additive — imported profiles/curves/effects
                        // are appended, existing entries with same name are replaced.
                        // Why: full merge semantics (per-field) are ambiguous without a spec.
                        let existing = config.config_mut();
                        for p in imported.profiles {
                            existing.profiles.retain(|e| e.name != p.name);
                            existing.profiles.push(p);
                        }
                        for c in imported.saved_curves {
                            existing.saved_curves.retain(|e| e.name != c.name);
                            existing.saved_curves.push(c);
                        }
                        for e in imported.saved_effects {
                            existing.saved_effects.retain(|x| x.name != e.name);
                            existing.saved_effects.push(e);
                        }
                    } else {
                        *config.config_mut() = imported;
                    };
                    config.flush();
                    tracing::info!("Config imported from {path} (merge={merge})");
                    Response::Ok
                }
                Err(e) => Response::Error(format!("invalid config JSON: {e}")),
            }
        }

        Request::SetAppProfiles { profiles } => {
            engine.app_profiles.load(profiles.clone());
            config.config_mut().app_profiles = profiles.clone();
            config.flush();
            tracing::info!("App profiles updated ({} mappings)", profiles.len());
            Response::Ok
        }

        Request::SaveEffect { effect } => {
            let cfg = config.config_mut();
            cfg.saved_effects.retain(|e| e.name != effect.name);
            cfg.saved_effects.push(effect.clone());
            config.flush();
            tracing::info!("Effect '{}' saved ({} frames)", effect.name, effect.frames.len());
            Response::Ok
        }

        Request::DeleteEffect { name } => {
            let cfg = config.config_mut();
            let before = cfg.saved_effects.len();
            cfg.saved_effects.retain(|e| e.name != *name);
            if cfg.saved_effects.len() == before {
                Response::Error(format!("effect '{}' not found", name))
            } else {
                config.flush();
                tracing::info!("Effect '{}' deleted", name);
                Response::Ok
            }
        }

        Request::ListEffects => Response::EffectList(config.config().saved_effects.clone()),

        Request::SaveLedPreset { preset } => {
            let cfg = config.config_mut();
            cfg.saved_led_presets.retain(|p| p.name != preset.name);
            cfg.saved_led_presets.push(preset.clone());
            config.flush();
            tracing::info!("LED preset '{}' saved ({} fans)", preset.name, preset.fan_count);
            Response::Ok
        }

        Request::DeleteLedPreset { name } => {
            let cfg = config.config_mut();
            let before = cfg.saved_led_presets.len();
            cfg.saved_led_presets.retain(|p| p.name != *name);
            if cfg.saved_led_presets.len() == before {
                Response::Error(format!("LED preset '{}' not found", name))
            } else {
                config.flush();
                tracing::info!("LED preset '{}' deleted", name);
                Response::Ok
            }
        }

        Request::ListLedPresets => Response::LedPresets(config.config().saved_led_presets.clone()),

        // LCD template CRUD
        Request::ListLcdTemplates => Response::LcdTemplates(config.config().lcd_templates.clone()),

        Request::SaveLcdTemplate { template } => {
            let cfg = config.config_mut();
            if let Some(existing) = cfg.lcd_templates.iter_mut().find(|t| t.id == template.id) {
                *existing = template.clone();
            } else {
                cfg.lcd_templates.push(template.clone());
            }
            config.flush();
            tracing::info!(
                "LCD template '{}' saved ({} widgets)",
                template.name,
                template.widgets.len()
            );
            Response::Ok
        }

        Request::DeleteLcdTemplate { id } => {
            let cfg = config.config_mut();
            let before = cfg.lcd_templates.len();
            cfg.lcd_templates.retain(|t| t.id != *id);
            if cfg.lcd_templates.len() < before {
                config.flush();
                tracing::info!("LCD template '{id}' deleted");
                Response::Ok
            } else {
                Response::Error(format!("template '{id}' not found"))
            }
        }

        // --- Reset: dispatch to correct backend based on device ---
        Request::Reset { target } => {
            let groups: Vec<frgb_model::GroupId> = match target {
                Some(frgb_model::ipc::Target::Group(g)) => vec![*g],
                Some(frgb_model::ipc::Target::Groups(gs)) => gs.clone(),
                Some(frgb_model::ipc::Target::All) | None => system.group_ids(),
                Some(frgb_model::ipc::Target::Role(role)) => system
                    .devices()
                    .iter()
                    .filter(|d| d.role == *role)
                    .map(|d| d.group)
                    .collect(),
            };
            let mut errors = Vec::new();
            for gid in &groups {
                if let Err(e) = system.reset_device(*gid) {
                    errors.push(format!("group {gid}: {e}"));
                }
            }
            if errors.is_empty() {
                tracing::info!("Reset sent to {} group(s)", groups.len());
                Response::Ok
            } else {
                Response::Error(errors.join("; "))
            }
        }

        // Watch = Subscribe + immediate status snapshot.
        // Subscription side-effect (client.subscribed = true) is applied in main.rs.
        Request::Watch { .. } => Response::DeviceStatus(build_status_list(system, true)),

        Request::EnterBindMode => {
            engine.bind_scanning = true;
            // Emit any currently known unbound devices immediately
            for dev in &system.unbound {
                events.push(Event::BindDiscovered(dev.clone()));
            }
            tracing::info!("Bind mode: scanning for unbound devices");
            Response::Ok
        }
        Request::ExitBindMode => {
            engine.bind_scanning = false;
            tracing::info!("Bind mode: stopped");
            Response::Ok
        }

        // --- ReorderGroups: set hardware merge order for chained effects ---
        Request::ReorderGroups { order } => {
            if order.is_empty() || order.len() > 4 {
                return (
                    Response::Error(format!("order must be 1-4 groups, got {}", order.len())),
                    events,
                );
            }
            match system.set_merge_order(order) {
                Ok(()) => {
                    tracing::info!("Merge order set: {:?}", order);
                    config.config_mut().merge_order = Some(order.clone());
                    config.flush();
                    Response::Ok
                }
                Err(e) => Response::Error(format!("set merge order: {e}")),
            }
        }

        // --- Mobo* handlers: motherboard info and RGB via AuraBackend ---
        Request::MoboDetect => {
            let has_aura = system.backend_by_name("aura").is_some();
            if has_aura {
                let aura_groups: Vec<frgb_model::GroupId> = system
                    .devices()
                    .iter()
                    .filter(|d| system.backend_name(d.backend_id) == Some("aura"))
                    .map(|d| d.group)
                    .collect();
                tracing::info!("MoboDetect: AURA found, {} group(s)", aura_groups.len());
                Response::Ok
            } else {
                Response::Error("no AURA motherboard RGB device found".into())
            }
        }
        Request::MoboStatus => {
            // AURA device info is included in the main Status response.
            // MoboStatus returns Ok if AURA backend is present.
            let aura_count = system
                .devices()
                .iter()
                .filter(|d| system.backend_name(d.backend_id) == Some("aura"))
                .count();
            if aura_count > 0 {
                tracing::info!("MoboStatus: {} AURA group(s) active", aura_count);
                Response::Ok
            } else {
                Response::Error("no AURA devices in registry".into())
            }
        }
        Request::MoboSetSpeed { channel, percent } => {
            // Motherboard fan speed: route through wired ENE MB sync, not AURA.
            match system.set_speed(
                frgb_model::GroupId::from(*channel),
                &frgb_model::speed::SpeedMode::Manual(*percent),
            ) {
                Ok(()) => {
                    tracing::info!("MoboSetSpeed: channel={channel} percent={percent}");
                    Response::Ok
                }
                Err(e) => Response::Error(format!("mobo speed: {e}")),
            }
        }
        Request::MoboAuto { channel } => {
            // Enable motherboard BIOS auto fan control via PWM mode
            match system.set_speed(frgb_model::GroupId::from(*channel), &frgb_model::speed::SpeedMode::Pwm) {
                Ok(()) => {
                    tracing::info!("MoboAuto: channel={channel} → PWM");
                    Response::Ok
                }
                Err(e) => Response::Error(format!("mobo auto: {e}")),
            }
        }
        Request::MoboTemps => {
            // Motherboard temps come from hwmon, not AURA HID.
            // Use ListSensors for full sensor data; this is a convenience alias.
            Response::Error("use ListSensors for temperature data".into())
        }

        // SetSyncConfig persists which backends participate in synchronized RGB.
        // Sync broadcasts a static color to all groups whose backend is enabled in SyncConfig.
        Request::Sync { color, config } => {
            if !config.enabled {
                return (Response::Error("sync is disabled".into()), events);
            }
            let mode = frgb_model::rgb::RgbMode::Static {
                ring: frgb_model::rgb::Ring::Both,
                color: *color,
                brightness: frgb_model::Brightness::new(255),
            };
            // Collect target groups first to avoid borrow conflict with set_rgb.
            let target_groups: Vec<frgb_model::GroupId> = system
                .devices()
                .iter()
                .filter(|d| {
                    let backend_name = system.backend_name(d.backend_id).unwrap_or("");
                    let backend_ok = match backend_name {
                        "lianli-rf" => config.include_lianli,
                        "aura" => config.include_mobo_rgb,
                        "openrgb" => config.include_openrgb,
                        _ => false,
                    };
                    if !backend_ok {
                        return false;
                    }
                    // Explicit group exclusion
                    if config.exclude_groups.contains(&d.group) {
                        return false;
                    }
                    // Role filter (empty = all roles included)
                    if !config.include_roles.is_empty() && !config.include_roles.contains(&d.role) {
                        return false;
                    }
                    // Device-type filter (empty = all types included)
                    if !config.include_device_types.is_empty() && !config.include_device_types.contains(&d.device_type)
                    {
                        return false;
                    }
                    true
                })
                .map(|d| d.group)
                .collect();
            let mut applied = 0u32;
            let mut errors = Vec::new();
            for gid in &target_groups {
                match system.set_rgb(*gid, &mode) {
                    Ok(()) => {
                        events.push(Event::RgbChanged {
                            group: *gid,
                            mode: mode.clone(),
                        });
                        applied += 1;
                    }
                    Err(e) => errors.push(format!("group {gid}: {e}")),
                }
            }
            if errors.is_empty() {
                tracing::info!("Sync: applied to {applied} groups");
                Response::Ok
            } else {
                Response::Error(errors.join("; "))
            }
        }
        // SyncConfig is purely persisted — no runtime mirror in Engine.
        // The Sync request takes config as a parameter, so disk is the source of truth.
        Request::GetSyncConfig => {
            let cfg = config.config();
            let sync = cfg.sync.clone().unwrap_or(SyncConfig {
                enabled: false,
                include_lianli: true,
                include_mobo_rgb: false,
                include_openrgb: false,
                include_roles: Vec::new(),
                include_device_types: Vec::new(),
                exclude_groups: Vec::new(),
            });
            Response::SyncConfig(sync)
        }
        Request::SetSyncConfig { config: sync_cfg } => {
            config.config_mut().sync = Some(sync_cfg.clone());
            config.flush();
            tracing::info!(
                "Sync config updated (enabled={}, lianli={}, mobo={}, openrgb={})",
                sync_cfg.enabled,
                sync_cfg.include_lianli,
                sync_cfg.include_mobo_rgb,
                sync_cfg.include_openrgb
            );
            Response::Ok
        }

        // --- AURA IPC handlers ---
        Request::ListAuraChannels => {
            let info = system
                .backend_by_name("aura")
                .and_then(|b| b.as_any().downcast_ref::<frgb_core::AuraBackend>())
                .map(|aura| aura.channel_info())
                .unwrap_or_default();
            Response::AuraChannels(info)
        }
        Request::SetAuraEffect { group, effect, color } => {
            let result = system
                .backend_by_name_mut("aura")
                .and_then(|b| b.as_any_mut().downcast_mut::<frgb_core::AuraBackend>())
                .ok_or_else(|| frgb_core::error::CoreError::NotFound("AURA backend".into()))
                .and_then(|aura| aura.set_hw_effect_by_group(*group, effect.to_byte(), (color[0], color[1], color[2])));
            match result {
                Ok(()) => Response::Ok,
                Err(e) => Response::Error(e.to_string()),
            }
        }

        Request::SetMbSync { group, enable } => match system.set_mb_sync(*group, *enable, None) {
            Ok(()) => {
                let state = if *enable { "motherboard" } else { "frgb" };
                tracing::info!("Group {group}: now on {state} control");
                Response::Ok
            }
            Err(e) => Response::Error(format!("MB sync failed: {e}")),
        },

        Request::Lock => match system.rf_ext() {
            Some(rf) => match rf.lock() {
                Ok(()) => {
                    tracing::info!("All devices: locked");
                    Response::Ok
                }
                Err(e) => Response::Error(format!("Lock failed: {e}")),
            },
            None => Response::Error("no RF backend available".into()),
        },

        Request::Unlock => match system.rf_ext() {
            Some(rf) => match rf.unlock() {
                Ok(()) => {
                    tracing::info!("All devices: unlocked");
                    Response::Ok
                }
                Err(e) => Response::Error(format!("Unlock failed: {e}")),
            },
            None => Response::Error("no RF backend available".into()),
        },

        Request::RenderTemplatePreview {
            template,
            width,
            height,
        } => {
            const MAX_PREVIEW_DIM: u32 = 2048;
            let width = (*width).min(MAX_PREVIEW_DIM);
            let height = (*height).min(MAX_PREVIEW_DIM);
            let sensors = std::collections::HashMap::new();
            let img = frgb_lcd_render::template::render_template(template, &sensors, width, height);
            match frgb_lcd::jpeg::prepare_jpeg(&img, width, height, 85) {
                Ok(jpeg) => Response::TemplatePreview(jpeg),
                Err(e) => Response::Error(format!("preview render failed: {e}")),
            }
        }

        Request::GetCurveSuggestions => Response::CurveSuggestions(engine.curve_suggestions(config)),

        Request::GetRecoveryCounters => {
            let usb = frgb_usb::recovery_counters();
            let hwmon = frgb_core::hwmon_backend::recovery_counters();
            Response::RecoveryCounters(frgb_model::ipc::RecoveryCountersIpc {
                usb_reopen_attempts: usb.reopen_attempts,
                usb_reopen_successes: usb.reopen_successes,
                usb_reopen_failures: usb.reopen_failures,
                usb_soft_recovery_successes: usb.soft_recovery_successes,
                hwmon_rescan_attempts: hwmon.rescan_attempts,
                hwmon_rescan_successes: hwmon.rescan_successes,
            })
        }

        Request::GetWearStats => {
            let stats: Vec<frgb_model::device::GroupWearInfo> = engine
                .wear_stats()
                .iter()
                .map(|(&group, &secs)| {
                    let name = system
                        .find_group(group)
                        .map(|d| d.name.clone())
                        .unwrap_or_else(|_| format!("Group {}", group));
                    frgb_model::device::GroupWearInfo {
                        group,
                        running_seconds: secs,
                        name,
                    }
                })
                .collect();
            Response::WearStats(stats)
        }

        #[allow(unreachable_patterns)]
        _ => Response::Error(format!(
            "request not yet implemented: {:?}",
            std::mem::discriminant(request)
        )),
    };

    (response, events)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Clone an RgbMode with all brightness fields set to the given level.
/// Source: INFERRED — brightness is embedded in multiple RgbMode variants
/// and ZoneSource sub-types; this helper walks the structure uniformly.
fn with_brightness(mode: frgb_model::rgb::RgbMode, level: frgb_model::Brightness) -> frgb_model::rgb::RgbMode {
    use frgb_model::rgb::{FanZoneSpec, RgbMode};
    match mode {
        RgbMode::Off => RgbMode::Off,
        RgbMode::Static { ring, color, .. } => RgbMode::Static {
            ring,
            color,
            brightness: level,
        },
        RgbMode::Effect {
            effect,
            mut params,
            ring,
        } => {
            params.brightness = level;
            RgbMode::Effect { effect, params, ring }
        }
        RgbMode::Composed(specs) => {
            let updated = specs
                .into_iter()
                .map(|spec| FanZoneSpec {
                    inner: zone_with_brightness(spec.inner, level),
                    outer: zone_with_brightness(spec.outer, level),
                })
                .collect();
            RgbMode::Composed(updated)
        }
        // PerFan, PerLed, TempRgb — no brightness field to modify, pass through unchanged.
        other => other,
    }
}

fn zone_with_brightness(
    zone: frgb_model::rgb::ZoneSource,
    level: frgb_model::Brightness,
) -> frgb_model::rgb::ZoneSource {
    use frgb_model::rgb::ZoneSource;
    match zone {
        ZoneSource::Color { color, .. } => ZoneSource::Color {
            color,
            brightness: level,
        },
        ZoneSource::Effect { effect, mut params } => {
            params.brightness = level;
            ZoneSource::Effect { effect, params }
        }
        ZoneSource::Off => ZoneSource::Off,
    }
}

fn persist_schedules(engine: &Engine, config: &mut crate::config_cache::ConfigCache) {
    config.config_mut().schedules = engine.scheduler.entries().to_vec();
    config.flush();
}

fn resolve_target_groups(system: &System, target: Option<&frgb_model::ipc::Target>) -> Vec<frgb_model::GroupId> {
    use frgb_model::ipc::Target;
    match target {
        None | Some(Target::All) => system.group_ids(),
        Some(Target::Group(g)) => vec![*g],
        Some(Target::Groups(gs)) => gs.clone(),
        Some(Target::Role(role)) => system
            .devices()
            .iter()
            .filter(|d| d.role == *role)
            .map(|d| d.group)
            .collect(),
    }
}

fn fan_group_from(d: &frgb_core::registry::Device) -> frgb_model::device::FanGroup {
    frgb_model::device::FanGroup {
        id: d.group,
        name: d.name.clone(),
        device_type: d.device_type,
        fan_count: d.fan_count(),
        role: d.role.clone(),
        blade: d.blade,
        cfm_max: None,
        fan_ids: d.mac_ids.clone(),
        tx_ref: d.tx_ref,
        fans_type: d.fans_type(),
        fans_rpm: d.fans_rpm(),
    }
}

/// Build GroupStatus list from current device state (true = use live state, false = defaults).
fn build_status_list(system: &System, use_state: bool) -> Vec<GroupStatus> {
    system
        .devices()
        .iter()
        .map(|d| GroupStatus {
            group: fan_group_from(d),
            rpms: d.slots.iter().map(|s| s.rpm).collect(),
            speed: if use_state {
                d.state
                    .speed_percent
                    .map(frgb_model::speed::SpeedMode::Manual)
                    .unwrap_or(frgb_model::speed::SpeedMode::Pwm)
            } else {
                frgb_model::speed::SpeedMode::Pwm
            },
            rgb: if use_state {
                d.state.rgb_mode.clone().unwrap_or(frgb_model::rgb::RgbMode::Off)
            } else {
                frgb_model::rgb::RgbMode::Off
            },
            lcd: if d.slots.iter().any(|s| s.has_lcd) {
                Some(frgb_model::lcd::LcdConfig {
                    brightness: frgb_model::Brightness::new(200),
                    rotation: frgb_model::lcd::LcdRotation::R0,
                    content: frgb_model::lcd::LcdContent::Off,
                })
            } else {
                None
            },
            lcd_count: d.slots.iter().filter(|s| s.has_lcd).count() as u8,
            mb_sync: d.mb_sync,
            online: true,
        })
        .collect()
}

fn handle_discover(system: &mut System) -> Response {
    match system.discover() {
        Ok(()) => Response::DeviceStatus(build_status_list(system, true)),
        Err(e) => Response::Error(e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use frgb_core::backend::BackendId;
    use frgb_model::config::{LedPreset, ScheduleAction, ScheduleEntry, Weekday};
    use frgb_model::device::{DeviceId, DeviceType, FanRole};
    use frgb_model::ipc::{Event, Request, Response, Target, Topic};
    use frgb_model::lcd::{LcdTemplate, TemplateBackground};
    use frgb_model::rgb::RgbMode;
    use frgb_model::rgb::{FanLedAssignment, Rgb};
    use frgb_model::sensor::Sensor;
    use frgb_model::show::{EffectCycle, Playback};
    use frgb_model::spec_loader::load_defaults;
    use frgb_model::speed::{CurvePoint, FanCurve, Interpolation, SpeedMode};
    use frgb_model::{GroupId, SpeedPercent, Temperature, ValidatedName};

    use crate::engine::Engine;

    fn test_config() -> crate::config_cache::ConfigCache {
        crate::config_cache::ConfigCache::from(frgb_model::config::Config::default())
    }

    /// Create a System with no backends (pure in-memory, no USB).
    fn empty_system() -> System {
        System::new(load_defaults())
    }

    /// Create a System with one device in the registry (group 1, 3× SL fans).
    /// No backend is registered, so set_speed/set_rgb will fail with "backend not found".
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

    fn test_engine() -> Engine {
        Engine::new(2000)
    }

    // -----------------------------------------------------------------------
    // Status / ListGroups — read-only queries
    // -----------------------------------------------------------------------

    /// Status returns DeviceStatus list.
    #[test]
    fn status_empty_system() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let (resp, events) = handle(&mut sys, &mut eng, &mut test_config(), &Request::Status);
        assert!(matches!(resp, Response::DeviceStatus(ref list) if list.is_empty()));
        assert!(events.is_empty());
    }

    /// Status with devices returns populated list.
    #[test]
    fn status_with_device() {
        let mut sys = system_with_device();
        let mut eng = test_engine();
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &Request::Status);
        if let Response::DeviceStatus(list) = resp {
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].group.id, GroupId::new(1));
            assert_eq!(list[0].rpms, vec![1200, 1200, 1200]);
        } else {
            panic!("expected DeviceStatus, got {:?}", resp);
        }
    }

    /// StatusVerbose is handled identically to Status.
    #[test]
    fn status_verbose_same_as_status() {
        let mut sys = system_with_device();
        let mut eng = test_engine();
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &Request::StatusVerbose);
        assert!(matches!(resp, Response::DeviceStatus(_)));
    }

    /// ListGroups returns FanGroup list.
    #[test]
    fn list_groups_empty() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &Request::ListGroups);
        assert!(matches!(resp, Response::GroupList(ref list) if list.is_empty()));
    }

    /// ListGroups with a device returns populated FanGroup.
    #[test]
    fn list_groups_with_device() {
        let mut sys = system_with_device();
        let mut eng = test_engine();
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &Request::ListGroups);
        if let Response::GroupList(list) = resp {
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].id, GroupId::new(1));
            assert_eq!(list[0].fan_count, 3);
        } else {
            panic!("expected GroupList, got {:?}", resp);
        }
    }

    // -----------------------------------------------------------------------
    // SetSpeed — validation
    // -----------------------------------------------------------------------

    /// Manual(101) is clamped to 100 by SpeedPercent::new.
    /// No validation error — the newtype prevents invalid values by construction.
    #[test]
    fn set_speed_manual_over_100_clamped() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let req = Request::SetSpeed {
            group: GroupId::new(1),
            mode: SpeedMode::Manual(SpeedPercent::new(101)),
        };
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &req);
        // SpeedPercent::new(101) clamps to 100 — no validation error.
        // Backend error expected because there's no real backend in empty_system.
        assert!(matches!(resp, Response::Error(_)));
    }

    /// Curve with empty points fails validation.
    #[test]
    fn set_speed_curve_empty_points_rejected() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let bad_curve = FanCurve {
            points: vec![],
            sensor: Sensor::Cpu,
            interpolation: Interpolation::Linear,
            min_speed: SpeedPercent::new(25),
            stop_below: None,
            ramp_rate: None,
        };
        let req = Request::SetSpeed {
            group: GroupId::new(1),
            mode: SpeedMode::Curve(bad_curve),
        };
        let (resp, events) = handle(&mut sys, &mut eng, &mut test_config(), &req);
        assert!(matches!(resp, Response::Error(ref msg) if msg.contains("FanCurve")));
        assert!(events.is_empty());
    }

    /// Valid Manual speed on missing group returns backend error.
    #[test]
    fn set_speed_manual_missing_group() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let req = Request::SetSpeed {
            group: GroupId::new(99),
            mode: SpeedMode::Manual(SpeedPercent::new(50)),
        };
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &req);
        assert!(matches!(resp, Response::Error(_)));
    }

    /// Inline Curve mode activates curve runner and emits SpeedChanged.
    #[test]
    fn set_speed_inline_curve_activates_runner() {
        let mut sys = empty_system();
        let mut eng = test_engine();
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
        let req = Request::SetSpeed {
            group: GroupId::new(1),
            mode: SpeedMode::Curve(curve),
        };
        let (resp, events) = handle(&mut sys, &mut eng, &mut test_config(), &req);
        assert!(matches!(resp, Response::Ok));
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], Event::SpeedChanged { group, .. } if group == GroupId::new(1)));
        assert!(eng.curves.is_active(), "curve runner should be active");
    }

    /// PWM mode on a group with no backend returns backend error.
    #[test]
    fn set_speed_pwm_no_backend() {
        let mut sys = system_with_device();
        let mut eng = test_engine();
        let req = Request::SetSpeed {
            group: GroupId::new(1),
            mode: SpeedMode::Pwm,
        };
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &req);
        // No backend → error from system.set_speed
        assert!(matches!(resp, Response::Error(_)));
    }

    /// Source: INFERRED — Setting a curve then switching to Manual removes the curve.
    /// When handler receives Manual mode, it calls engine.curves.remove_curve first.
    #[test]
    fn set_speed_manual_removes_active_curve() {
        let mut sys = empty_system();
        let mut eng = test_engine();

        // Activate a curve
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
        eng.curves.set_curve(GroupId::new(1), curve);
        assert!(eng.curves.is_active());

        // SetSpeed Manual → removes curve first, then tries backend (which fails, but curve is gone)
        let req = Request::SetSpeed {
            group: GroupId::new(1),
            mode: SpeedMode::Manual(SpeedPercent::new(50)),
        };
        let _ = handle(&mut sys, &mut eng, &mut test_config(), &req);
        assert!(!eng.curves.is_active(), "curve should be removed on Manual switch");
    }

    // -----------------------------------------------------------------------
    // Subscribe / Unsubscribe — no-ops that return Ok
    // -----------------------------------------------------------------------

    /// Subscribe returns Ok (no-op acknowledgment).
    #[test]
    fn subscribe_returns_ok() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let req = Request::Subscribe {
            topics: vec![Topic::Rpm, Topic::Temperature],
        };
        let (resp, events) = handle(&mut sys, &mut eng, &mut test_config(), &req);
        assert!(matches!(resp, Response::Ok));
        assert!(events.is_empty());
    }

    /// Unsubscribe returns Ok.
    #[test]
    fn unsubscribe_returns_ok() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &Request::Unsubscribe);
        assert!(matches!(resp, Response::Ok));
    }

    // -----------------------------------------------------------------------
    // Schedule management — in-memory only (persist_schedules touches config)
    // -----------------------------------------------------------------------

    /// ListSchedule returns engine's schedule entries.
    #[test]
    fn list_schedule_empty() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &Request::ListSchedule);
        assert!(matches!(resp, Response::ScheduleList(ref list) if list.is_empty()));
    }

    /// AddSchedule with invalid hour returns Error.
    #[test]
    fn add_schedule_invalid_hour() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let req = Request::AddSchedule {
            entry: ScheduleEntry {
                hour: 25,
                minute: 0,
                days: vec![],
                action: ScheduleAction::SwitchProfile("test".into()),
            },
        };
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &req);
        assert!(matches!(resp, Response::Error(ref msg) if msg.contains("hour")));
    }

    /// AddSchedule with invalid minute returns Error.
    #[test]
    fn add_schedule_invalid_minute() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let req = Request::AddSchedule {
            entry: ScheduleEntry {
                hour: 12,
                minute: 60,
                days: vec![],
                action: ScheduleAction::SwitchProfile("test".into()),
            },
        };
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &req);
        assert!(matches!(resp, Response::Error(ref msg) if msg.contains("minute")));
    }

    /// AddSchedule with valid entry returns Ok and adds to engine.
    /// Note: persist_schedules will attempt to save to config file (may fail in CI).
    #[test]
    fn add_schedule_valid() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let req = Request::AddSchedule {
            entry: ScheduleEntry {
                hour: 8,
                minute: 30,
                days: vec![Weekday::Mon, Weekday::Fri],
                action: ScheduleAction::SwitchProfile("quiet".into()),
            },
        };
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &req);
        assert!(matches!(resp, Response::Ok));
        assert_eq!(eng.scheduler.entries().len(), 1);
    }

    /// DeleteSchedule with out-of-bounds index returns Error.
    #[test]
    fn remove_schedule_out_of_bounds() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let req = Request::DeleteSchedule { index: 0 };
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &req);
        assert!(matches!(resp, Response::Error(ref msg) if msg.contains("out of bounds")));
    }

    /// ClearSchedule empties the scheduler.
    #[test]
    fn clear_schedule() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        // Add one first
        eng.scheduler.add(ScheduleEntry {
            hour: 12,
            minute: 0,
            days: vec![],
            action: ScheduleAction::SwitchProfile("test".into()),
        });
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &Request::ClearSchedule);
        assert!(matches!(resp, Response::Ok));
        assert!(eng.scheduler.entries().is_empty());
    }

    // -----------------------------------------------------------------------
    // Sequence control — in-memory operations
    // -----------------------------------------------------------------------

    /// SetEffectCycle returns Ok and starts the show runner.
    #[test]
    fn set_effect_cycle_ok() {
        let mut sys = system_with_device();
        let mut eng = test_engine();
        let req = Request::SetEffectCycle {
            cycle: EffectCycle {
                steps: vec![],
                playback: Playback::Loop,
            },
        };
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &req);
        assert!(matches!(resp, Response::Ok));
    }

    /// StopAllSequences returns Ok.
    #[test]
    fn stop_all_sequences_ok() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &Request::StopAllSequences);
        assert!(matches!(resp, Response::Ok));
    }

    /// StopSequence with no target returns Ok (stops all groups).
    #[test]
    fn stop_sequence_no_target() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let req = Request::StopSequence { target: None };
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &req);
        assert!(matches!(resp, Response::Ok));
    }

    /// StartSequence with nonexistent name returns Error.
    #[test]
    fn start_sequence_not_found() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let req = Request::StartSequence {
            name: "nonexistent".into(),
            target: None,
        };
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &req);
        assert!(matches!(resp, Response::Error(ref msg) if msg.contains("not found")));
    }

    // -----------------------------------------------------------------------
    // Firmware info — no RF backend
    // -----------------------------------------------------------------------

    /// GetFirmwareInfo without RF backend returns unknown/n/a.
    #[test]
    fn firmware_info_no_rf() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &Request::GetFirmwareInfo);
        if let Response::FirmwareInfo(info) = resp {
            assert_eq!(info.tx_version, "unknown");
            assert_eq!(info.rx_version, "n/a");
        } else {
            panic!("expected FirmwareInfo, got {:?}", resp);
        }
    }

    // -----------------------------------------------------------------------
    // Bind — no RF backend
    // -----------------------------------------------------------------------

    /// Bind without RF backend returns Error.
    #[test]
    fn bind_no_rf_backend() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let req = Request::Bind {
            group: GroupId::new(1),
            lock: false,
        };
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &req);
        assert!(matches!(resp, Response::Error(ref msg) if msg.contains("backend does not support bind")));
    }

    // -----------------------------------------------------------------------
    // LCD — no LCD backend
    // -----------------------------------------------------------------------

    /// SetLcd on invalid index returns Error.
    #[test]
    fn set_lcd_missing_device() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let req = Request::SetLcd {
            lcd_index: 99,
            config: frgb_model::lcd::LcdConfig {
                brightness: frgb_model::Brightness::new(50),
                rotation: frgb_model::lcd::LcdRotation::R0,
                content: frgb_model::lcd::LcdContent::Off,
            },
        };
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &req);
        assert!(matches!(resp, Response::Error(_)));
    }

    // -----------------------------------------------------------------------
    // Pump — no AIO backend
    // -----------------------------------------------------------------------

    /// SetPumpMode on missing group returns Error.
    #[test]
    fn set_pump_missing_group() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let req = Request::SetPumpMode {
            group: GroupId::new(1),
            mode: frgb_model::speed::PumpMode::Quiet,
        };
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &req);
        assert!(matches!(resp, Response::Error(_)));
    }

    // -----------------------------------------------------------------------
    // SetRole — upsert + persistence
    // -----------------------------------------------------------------------

    /// SetRole on a discovered group must update both the
    /// in-memory device role and create a persistent GroupConfig entry, even
    /// when the config has no prior entry for that group.
    #[test]
    fn set_role_upserts_config_when_group_missing() {
        let mut sys = system_with_device();
        let mut eng = test_engine();
        let mut cfg = test_config();
        // Sanity: starting state has no group entries.
        assert!(cfg.config().groups.is_empty());

        let req = Request::SetRole {
            group: GroupId::new(1),
            role: frgb_model::device::FanRole::Exhaust,
        };
        let (resp, _) = handle(&mut sys, &mut eng, &mut cfg, &req);
        assert!(matches!(resp, Response::Ok));

        // In-memory device role updated.
        let dev = sys.find_group(GroupId::new(1)).expect("group 1");
        assert_eq!(dev.role, frgb_model::device::FanRole::Exhaust);

        // Config now has a GroupConfig entry for group 1 with the new role.
        let groups = &cfg.config().groups;
        assert_eq!(groups.len(), 1, "missing GroupConfig was not upserted");
        assert_eq!(groups[0].id, GroupId::new(1));
        assert_eq!(groups[0].role, frgb_model::device::FanRole::Exhaust);
    }

    /// SetRole called twice updates the existing entry
    /// rather than appending duplicates.
    #[test]
    fn set_role_updates_existing_entry() {
        let mut sys = system_with_device();
        let mut eng = test_engine();
        let mut cfg = test_config();

        let _ = handle(
            &mut sys,
            &mut eng,
            &mut cfg,
            &Request::SetRole {
                group: GroupId::new(1),
                role: frgb_model::device::FanRole::Intake,
            },
        );
        let _ = handle(
            &mut sys,
            &mut eng,
            &mut cfg,
            &Request::SetRole {
                group: GroupId::new(1),
                role: frgb_model::device::FanRole::Exhaust,
            },
        );

        let groups = &cfg.config().groups;
        assert_eq!(groups.len(), 1, "duplicate GroupConfig was created");
        assert_eq!(groups[0].role, frgb_model::device::FanRole::Exhaust);
        assert_eq!(
            sys.find_group(GroupId::new(1)).unwrap().role,
            frgb_model::device::FanRole::Exhaust
        );
    }

    // -----------------------------------------------------------------------
    // SetRgb — missing group
    // -----------------------------------------------------------------------

    /// SetRgb on missing group returns Error.
    #[test]
    fn set_rgb_missing_group() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let req = Request::SetRgb {
            group: GroupId::new(99),
            mode: RgbMode::Off,
        };
        let (resp, events) = handle(&mut sys, &mut eng, &mut test_config(), &req);
        assert!(matches!(resp, Response::Error(_)));
        assert!(events.is_empty());
    }

    // -----------------------------------------------------------------------
    // Bind mode
    // -----------------------------------------------------------------------

    /// EnterBindMode returns Ok and sets bind_scanning.
    #[test]
    fn enter_bind_mode_returns_ok() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        assert!(!eng.bind_scanning);
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &Request::EnterBindMode);
        assert!(matches!(resp, Response::Ok));
        assert!(eng.bind_scanning);
    }

    /// ExitBindMode returns Ok and clears bind_scanning.
    #[test]
    fn exit_bind_mode_returns_ok() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        eng.bind_scanning = true;
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &Request::ExitBindMode);
        assert!(matches!(resp, Response::Ok));
        assert!(!eng.bind_scanning);
    }

    /// EnterBindMode emits BindDiscovered for existing unbound devices.
    #[test]
    fn enter_bind_mode_emits_existing_unbound() {
        let mut sys = empty_system();
        // Inject an unbound device
        sys.unbound.push(frgb_model::device::UnboundDevice {
            mac: DeviceId::from([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]),
            master: DeviceId::from([0x11, 0x22, 0x33, 0x44, 0x55, 0x66]),
            group: GroupId::new(0),
            fan_count: 3,
            device_type: frgb_model::device::DeviceType::Unknown,
            fans_type: [21, 21, 21, 0],
        });
        let mut eng = test_engine();
        let (resp, events) = handle(&mut sys, &mut eng, &mut test_config(), &Request::EnterBindMode);
        assert!(matches!(resp, Response::Ok));
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], Event::BindDiscovered(dev) if dev.fan_count == 3));
    }

    /// Watch returns DeviceStatus (not Error).
    #[test]
    fn watch_returns_device_status() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let req = Request::Watch { interval_ms: 1000 };
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &req);
        assert!(
            matches!(resp, Response::DeviceStatus(_)),
            "expected DeviceStatus, got {:?}",
            resp
        );
    }

    /// Watch with a registered device returns populated status list.
    #[test]
    fn watch_with_device_returns_populated_status() {
        let mut sys = system_with_device();
        let mut eng = test_engine();
        let req = Request::Watch { interval_ms: 500 };
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &req);
        match resp {
            Response::DeviceStatus(groups) => assert!(!groups.is_empty(), "expected non-empty status list"),
            other => panic!("expected DeviceStatus, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Source: INFERRED — resolve_target_groups with None/All returns all group IDs.
    /// The handler uses this for StopSequence, StartSequence, etc.
    #[test]
    fn resolve_target_all() {
        let sys = system_with_device();
        let groups = resolve_target_groups(&sys, None);
        assert_eq!(groups, vec![GroupId::new(1)]);
        let groups = resolve_target_groups(&sys, Some(&Target::All));
        assert_eq!(groups, vec![GroupId::new(1)]);
    }

    /// Source: INFERRED — resolve_target_groups with Group(n) returns just that group.
    #[test]
    fn resolve_target_single_group() {
        let sys = system_with_device();
        let groups = resolve_target_groups(&sys, Some(&Target::Group(GroupId::new(1))));
        assert_eq!(groups, vec![GroupId::new(1)]);
    }

    /// Source: INFERRED — resolve_target_groups with Role filters by device role.
    #[test]
    fn resolve_target_role() {
        let sys = system_with_device();
        // Default role for SL fans is Intake
        let groups = resolve_target_groups(&sys, Some(&Target::Role(FanRole::Intake)));
        assert_eq!(groups, vec![GroupId::new(1)]);
        let groups = resolve_target_groups(&sys, Some(&Target::Role(FanRole::Exhaust)));
        assert!(groups.is_empty());
    }

    /// Source: INFERRED — build_status_list with use_state=true shows live speed/rgb.
    #[test]
    fn build_status_list_live_state() {
        let mut sys = system_with_device();
        sys.registry.update_state(GroupId::new(1), |s| {
            s.speed_percent = Some(SpeedPercent::new(70));
            s.rgb_mode = Some(RgbMode::Off);
        });
        let list = build_status_list(&sys, true);
        assert_eq!(list.len(), 1);
        assert!(matches!(list[0].speed, SpeedMode::Manual(p) if p == SpeedPercent::new(70)));
        assert!(matches!(list[0].rgb, RgbMode::Off));
    }

    /// Source: INFERRED — build_status_list with use_state=false shows defaults (Pwm/Off).
    #[test]
    fn build_status_list_defaults() {
        let mut sys = system_with_device();
        sys.registry.update_state(GroupId::new(1), |s| {
            s.speed_percent = Some(SpeedPercent::new(70));
            s.rgb_mode = Some(RgbMode::Off);
        });
        let list = build_status_list(&sys, false);
        assert_eq!(list.len(), 1);
        assert!(matches!(list[0].speed, SpeedMode::Pwm));
        assert!(matches!(list[0].rgb, RgbMode::Off));
    }

    // -----------------------------------------------------------------------
    // Sync filtering — role, device-type, group exclusion
    // -----------------------------------------------------------------------

    /// Minimal mock backend that claims to be "lianli-rf" and always fails send_rgb.
    /// Used to exercise the Sync handler's filtering logic without real hardware.
    struct MockLianLiBackend;

    impl frgb_core::backend::Backend for MockLianLiBackend {
        fn id(&self) -> BackendId {
            BackendId(0)
        }
        fn name(&self) -> &str {
            "lianli-rf"
        }
        fn discover(&mut self) -> frgb_core::error::Result<Vec<frgb_core::DiscoveredDevice>> {
            Ok(Vec::new())
        }
        fn set_speed(
            &self,
            _device: &frgb_core::registry::Device,
            _cmd: &frgb_core::backend::SpeedCommand,
        ) -> frgb_core::error::Result<()> {
            Err(frgb_core::error::CoreError::NotSupported("mock".into()))
        }
        fn send_rgb(
            &self,
            _device: &frgb_core::registry::Device,
            _buf: &frgb_rgb::generator::EffectResult,
        ) -> frgb_core::error::Result<()> {
            Err(frgb_core::error::CoreError::NotSupported("mock: no hardware".into()))
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    /// Create a System with two device groups backed by MockLianLiBackend:
    ///   group 1 — Intake (default role after discovery)
    ///   group 2 — Exhaust (mutated after discovery)
    /// set_rgb will fail at the backend level; we test the filtering logic.
    fn two_device_system() -> System {
        let specs = load_defaults();
        let mut system = System::new(specs);
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        // Register the mock backend so backend_name() returns "lianli-rf".
        system.add_backend(Box::new(MockLianLiBackend));
        // Add both devices in one refresh call.
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
                    id: DeviceId::from([0xd0, 0x11, 0x22, 0x33, 0x44, 0x55]),
                    fans_type: [21, 21, 21, 0],
                    dev_type: 0,
                    group: GroupId::new(2),
                    fan_count: 3,
                    master: our_mac,
                    fans_rpm: [1100, 1100, 1100, 0],
                    fans_pwm: [0; 4],
                    cmd_seq: 0,
                    channel: 0x08,
                },
            ],
            our_mac,
            &system.specs,
        );
        // Group 2 is Intake by default; mutate it to Exhaust.
        if let Some(dev) = system.registry.find_by_group_mut(GroupId::new(2)) {
            dev.role = FanRole::Exhaust;
        }
        system
    }

    /// Sync with include_roles=[Intake] must attempt only group 1, not group 2.
    /// Because there is no backend, both set_rgb calls would fail — we verify
    /// that the error list mentions group 1 but NOT group 2 (i.e., group 2 was
    /// filtered out before set_rgb was attempted).
    #[test]
    fn sync_role_filter_includes_only_intake() {
        let mut sys = two_device_system();
        let mut eng = test_engine();
        let req = Request::Sync {
            color: frgb_model::rgb::Rgb { r: 255, g: 0, b: 0 },
            config: frgb_model::config::SyncConfig {
                enabled: true,
                include_lianli: true,
                include_mobo_rgb: false,
                include_openrgb: false,
                include_roles: vec![FanRole::Intake],
                include_device_types: Vec::new(),
                exclude_groups: Vec::new(),
            },
        };
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &req);
        // No backend → set_rgb fails for group 1.
        // Group 2 (Exhaust) must not be mentioned — it was filtered out.
        if let Response::Error(msg) = resp {
            assert!(msg.contains("group 1"), "group 1 should be attempted: {msg}");
            assert!(
                !msg.contains("group 2"),
                "group 2 should be excluded by role filter: {msg}"
            );
        } else {
            panic!("expected Error response (no backend), got {:?}", resp);
        }
    }

    /// Sync with exclude_groups=[1] must skip group 1 entirely.
    #[test]
    fn sync_exclude_groups_skips_group() {
        let mut sys = two_device_system();
        let mut eng = test_engine();
        let req = Request::Sync {
            color: frgb_model::rgb::Rgb { r: 0, g: 255, b: 0 },
            config: frgb_model::config::SyncConfig {
                enabled: true,
                include_lianli: true,
                include_mobo_rgb: false,
                include_openrgb: false,
                include_roles: Vec::new(),
                include_device_types: Vec::new(),
                exclude_groups: vec![GroupId::new(1)],
            },
        };
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &req);
        // Group 2 attempted but fails (no backend); group 1 excluded.
        if let Response::Error(msg) = resp {
            assert!(msg.contains("group 2"), "group 2 should be attempted: {msg}");
            assert!(!msg.contains("group 1"), "group 1 should be excluded: {msg}");
        } else {
            panic!("expected Error response (no backend), got {:?}", resp);
        }
    }

    /// Sync with both filters inactive (empty Vecs) passes all groups through.
    #[test]
    fn sync_no_filters_passes_all_groups() {
        let mut sys = two_device_system();
        let mut eng = test_engine();
        let req = Request::Sync {
            color: frgb_model::rgb::Rgb { r: 0, g: 0, b: 255 },
            config: frgb_model::config::SyncConfig {
                enabled: true,
                include_lianli: true,
                include_mobo_rgb: false,
                include_openrgb: false,
                include_roles: Vec::new(),
                include_device_types: Vec::new(),
                exclude_groups: Vec::new(),
            },
        };
        let (resp, _) = handle(&mut sys, &mut eng, &mut test_config(), &req);
        // Both groups attempted → error mentions both.
        if let Response::Error(msg) = resp {
            assert!(msg.contains("group 1"), "group 1 should be attempted: {msg}");
            assert!(msg.contains("group 2"), "group 2 should be attempted: {msg}");
        } else {
            panic!("expected Error response (no backend), got {:?}", resp);
        }
    }

    // -----------------------------------------------------------------------
    // LED preset management
    // -----------------------------------------------------------------------

    fn test_led_preset() -> LedPreset {
        LedPreset {
            name: frgb_model::ValidatedName::new("Test Layout").unwrap(),
            group_device_type: DeviceType::SlWireless,
            fan_count: 3,
            assignments: vec![FanLedAssignment {
                inner: vec![Rgb { r: 255, g: 0, b: 0 }],
                outer: vec![Rgb { r: 0, g: 0, b: 255 }],
            }],
        }
    }

    /// SaveLedPreset persists the preset; ListLedPresets returns it.
    #[test]
    fn save_led_preset_persists() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let mut cfg = test_config();
        let preset = test_led_preset();

        let (resp, _) = handle(
            &mut sys,
            &mut eng,
            &mut cfg,
            &Request::SaveLedPreset { preset: preset.clone() },
        );
        assert!(matches!(resp, Response::Ok));

        let (resp, _) = handle(&mut sys, &mut eng, &mut cfg, &Request::ListLedPresets);
        if let Response::LedPresets(list) = resp {
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].name, "Test Layout");
            assert_eq!(list[0].fan_count, 3);
        } else {
            panic!("expected LedPresets, got {:?}", resp);
        }
    }

    /// DeleteLedPreset removes the preset; ListLedPresets returns empty.
    #[test]
    fn delete_led_preset_removes() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let mut cfg = test_config();
        let preset = test_led_preset();

        handle(
            &mut sys,
            &mut eng,
            &mut cfg,
            &Request::SaveLedPreset { preset: preset.clone() },
        );

        let (resp, _) = handle(
            &mut sys,
            &mut eng,
            &mut cfg,
            &Request::DeleteLedPreset {
                name: "Test Layout".into(),
            },
        );
        assert!(matches!(resp, Response::Ok));

        let (resp, _) = handle(&mut sys, &mut eng, &mut cfg, &Request::ListLedPresets);
        if let Response::LedPresets(list) = resp {
            assert!(list.is_empty());
        } else {
            panic!("expected LedPresets, got {:?}", resp);
        }
    }

    // -----------------------------------------------------------------------
    // Name validation
    // -----------------------------------------------------------------------

    #[test]
    fn save_profile_rejects_empty_name_at_serde() {
        // ValidatedName rejects empty names at construction / deserialization.
        let json = r#"{"SaveProfile":{"name":""}}"#;
        let result: Result<Request, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn save_profile_rejects_long_name_at_serde() {
        let long = "x".repeat(65);
        let json = format!(r#"{{"SaveProfile":{{"name":"{}"}}}}"#, long);
        let result: Result<Request, _> = serde_json::from_str(&json);
        assert!(result.is_err());
    }

    #[test]
    fn rename_group_rejects_empty_name() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let (resp, _) = handle(
            &mut sys,
            &mut eng,
            &mut test_config(),
            &Request::RenameGroup {
                group: GroupId::new(1),
                name: String::new(),
            },
        );
        assert!(matches!(resp, Response::Error(ref e) if e.contains("empty")));
    }

    #[test]
    fn save_curve_rejects_empty_name_at_serde() {
        // ValidatedName rejects empty names at deserialization.
        let json = r#"{"SaveCurve":{"name":"","curve":{"sensor":"Cpu","points":[{"temp":30,"speed":30}],"interpolation":"Linear","min_speed":0}}}"#;
        let result: Result<Request, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    /// Saving a preset with the same name twice upserts — only one copy is stored.
    #[test]
    fn save_led_preset_upserts() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let mut cfg = test_config();
        let preset = test_led_preset();

        handle(
            &mut sys,
            &mut eng,
            &mut cfg,
            &Request::SaveLedPreset { preset: preset.clone() },
        );

        // Save again with same name but different fan_count
        let updated = LedPreset {
            fan_count: 5,
            ..preset.clone()
        };
        let (resp, _) = handle(
            &mut sys,
            &mut eng,
            &mut cfg,
            &Request::SaveLedPreset { preset: updated },
        );
        assert!(matches!(resp, Response::Ok));

        let (resp, _) = handle(&mut sys, &mut eng, &mut cfg, &Request::ListLedPresets);
        if let Response::LedPresets(list) = resp {
            assert_eq!(list.len(), 1, "expected exactly one preset after upsert");
            assert_eq!(list[0].fan_count, 5, "expected updated fan_count");
        } else {
            panic!("expected LedPresets, got {:?}", resp);
        }
    }

    // -----------------------------------------------------------------------
    // Hello / protocol negotiation
    // -----------------------------------------------------------------------

    #[test]
    fn hello_negotiation_same_version() {
        let mut sys = empty_system();
        let mut eng = Engine::new(2000);
        let mut cfg = test_config();
        let (resp, _) = handle(
            &mut sys,
            &mut eng,
            &mut cfg,
            &Request::Hello {
                protocol_version: frgb_model::ipc::PROTOCOL_VERSION,
            },
        );
        assert!(
            matches!(resp, Response::Hello { protocol_version } if protocol_version == frgb_model::ipc::PROTOCOL_VERSION),
            "expected negotiated version {}, got {:?}",
            frgb_model::ipc::PROTOCOL_VERSION,
            resp,
        );
    }

    #[test]
    fn hello_negotiation_client_newer() {
        let mut sys = empty_system();
        let mut eng = Engine::new(2000);
        let mut cfg = test_config();
        // Client claims v2, daemon max is v1 → negotiated to daemon max
        let (resp, _) = handle(
            &mut sys,
            &mut eng,
            &mut cfg,
            &Request::Hello {
                protocol_version: frgb_model::ipc::PROTOCOL_VERSION + 1,
            },
        );
        assert!(
            matches!(resp, Response::Hello { protocol_version } if protocol_version == frgb_model::ipc::PROTOCOL_VERSION),
            "expected daemon max v{}, got {:?}",
            frgb_model::ipc::PROTOCOL_VERSION,
            resp,
        );
    }

    #[test]
    fn hello_negotiation_client_too_old() {
        let mut sys = empty_system();
        let mut eng = Engine::new(2000);
        let mut cfg = test_config();
        // Client sends v0, daemon min is v1 → error
        let (resp, _) = handle(&mut sys, &mut eng, &mut cfg, &Request::Hello { protocol_version: 0 });
        assert!(
            matches!(resp, Response::Error(ref msg) if msg.contains("too old")),
            "expected 'too old' error, got {:?}",
            resp,
        );
    }

    // -----------------------------------------------------------------------
    // Wear stats
    // -----------------------------------------------------------------------

    /// GetWearStats returns empty WearStats when no wear data has been recorded.
    #[test]
    fn get_wear_stats_empty() {
        let mut sys = empty_system();
        let mut eng = Engine::new(2000);
        let mut cfg = test_config();
        let (resp, events) = handle(&mut sys, &mut eng, &mut cfg, &Request::GetWearStats);
        assert!(events.is_empty());
        match resp {
            Response::WearStats(stats) => assert!(stats.is_empty(), "expected empty stats, got {:?}", stats),
            other => panic!("expected WearStats, got {:?}", other),
        }
    }

    /// GetWearStats returns loaded wear entries after load_wear_stats is called.
    #[test]
    fn get_wear_stats_after_load() {
        let mut sys = system_with_device();
        let mut eng = Engine::new(2000);
        let mut cfg = test_config();

        // Pre-load wear data for group 1
        eng.load_wear_stats(&[frgb_model::config::WearEntry {
            group_id: GroupId::new(1),
            running_seconds: 3600,
        }]);

        let (resp, _) = handle(&mut sys, &mut eng, &mut cfg, &Request::GetWearStats);
        match resp {
            Response::WearStats(stats) => {
                assert_eq!(stats.len(), 1);
                assert_eq!(stats[0].group, GroupId::new(1));
                assert_eq!(stats[0].running_seconds, 3600);
                // Name must be populated (non-empty — group 1 is in the system)
                assert!(!stats[0].name.is_empty(), "expected non-empty name");
            }
            other => panic!("expected WearStats, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // Curve suggestions
    // -----------------------------------------------------------------------

    /// GetCurveSuggestions returns empty list when no curves are active.
    #[test]
    fn get_curve_suggestions_empty() {
        let mut sys = empty_system();
        let mut eng = Engine::new(2000);
        let mut cfg = test_config();
        let (resp, events) = handle(&mut sys, &mut eng, &mut cfg, &Request::GetCurveSuggestions);
        assert!(events.is_empty());
        match resp {
            Response::CurveSuggestions(suggestions) => {
                assert!(
                    suggestions.is_empty(),
                    "expected no suggestions with no active curves, got {:?}",
                    suggestions
                )
            }
            other => panic!("expected CurveSuggestions, got {:?}", other),
        }
    }

    /// GetRecoveryCounters returns a snapshot (no events emitted).
    #[test]
    fn get_recovery_counters_returns_snapshot() {
        let mut sys = empty_system();
        let mut eng = Engine::new(2000);
        let mut cfg = test_config();
        let (resp, events) = handle(&mut sys, &mut eng, &mut cfg, &Request::GetRecoveryCounters);
        assert!(events.is_empty(), "GetRecoveryCounters should emit no events");
        assert!(matches!(resp, Response::RecoveryCounters(_)), "expected Response::RecoveryCounters, got {resp:?}");
    }

    /// End-to-end wiring: incrementing the global USB counters must be visible
    /// through the handler's snapshot. Catches regressions where the handler
    /// reads the wrong counter source or maps fields incorrectly.
    #[test]
    fn get_recovery_counters_reflects_global_state() {
        let mut sys = empty_system();
        let mut eng = Engine::new(2000);
        let mut cfg = test_config();

        let snap = |sys: &mut System, eng: &mut Engine, cfg: &mut crate::config_cache::ConfigCache| {
            let (resp, _) = handle(sys, eng, cfg, &Request::GetRecoveryCounters);
            match resp {
                Response::RecoveryCounters(c) => c,
                other => panic!("expected RecoveryCounters, got {other:?}"),
            }
        };

        let before = snap(&mut sys, &mut eng, &mut cfg);

        // Drive each USB counter via the public record_* API. These are the
        // same statics that UsbDevice::reopen / HidDevice::reopen / with_recovery
        // increment in production — proving the handler reads from the right place.
        frgb_usb::counters::record_reopen_attempt();
        frgb_usb::counters::record_reopen_success();
        frgb_usb::counters::record_reopen_failure();

        let after = snap(&mut sys, &mut eng, &mut cfg);

        // Strict-greater (not equal-plus-1) tolerates concurrent test increments
        // — these counters are process-wide statics shared across the test binary.
        assert!(
            after.usb_reopen_attempts > before.usb_reopen_attempts,
            "usb_reopen_attempts must increase: before={} after={}",
            before.usb_reopen_attempts, after.usb_reopen_attempts,
        );
        assert!(
            after.usb_reopen_successes > before.usb_reopen_successes,
            "usb_reopen_successes must increase: before={} after={}",
            before.usb_reopen_successes, after.usb_reopen_successes,
        );
        assert!(
            after.usb_reopen_failures > before.usb_reopen_failures,
            "usb_reopen_failures must increase: before={} after={}",
            before.usb_reopen_failures, after.usb_reopen_failures,
        );
    }

    /// Enter → Exit → Re-enter bind mode cycle works correctly.
    #[test]
    fn bind_mode_enter_exit_reenter_cycle() {
        let mut sys = empty_system();
        let mut eng = test_engine();
        let mut cfg = test_config();

        // Enter
        let (resp, _) = handle(&mut sys, &mut eng, &mut cfg, &Request::EnterBindMode);
        assert!(matches!(resp, Response::Ok));
        assert!(eng.bind_scanning);

        // Exit
        let (resp, _) = handle(&mut sys, &mut eng, &mut cfg, &Request::ExitBindMode);
        assert!(matches!(resp, Response::Ok));
        assert!(!eng.bind_scanning);

        // Re-enter should work
        let (resp, _) = handle(&mut sys, &mut eng, &mut cfg, &Request::EnterBindMode);
        assert!(matches!(resp, Response::Ok));
        assert!(eng.bind_scanning);
    }

    #[test]
    fn render_template_preview_passes_through_normal_size() {
        let mut system = system_with_device();
        let mut engine = test_engine();
        let mut config = test_config();

        let template = LcdTemplate {
            id: "test_template".to_string(),
            name: ValidatedName::new("test").unwrap(),
            base_width: 480,
            base_height: 480,
            background: TemplateBackground::Color {
                rgba: [0, 0, 0, 255],
            },
            widgets: vec![],
        };

        let request = Request::RenderTemplatePreview {
            template,
            width: 400,
            height: 400,
        };
        let (response, _events) = handle(&mut system, &mut engine, &mut config, &request);
        // Acceptable: TemplatePreview(bytes), or Error (e.g. template-rendering issue) — both
        // demonstrate the request was processed without OOM.
        match response {
            Response::TemplatePreview(_) | Response::Error(_) => {}
            other => panic!("unexpected response variant: {other:?}"),
        }
    }

    #[test]
    fn render_template_preview_clamps_huge_dimensions() {
        let mut system = system_with_device();
        let mut engine = test_engine();
        let mut config = test_config();

        let template = LcdTemplate {
            id: "test_template".to_string(),
            name: ValidatedName::new("test").unwrap(),
            base_width: 480,
            base_height: 480,
            background: TemplateBackground::Color {
                rgba: [0, 0, 0, 255],
            },
            widgets: vec![],
        };

        let request = Request::RenderTemplatePreview {
            template,
            width: 65535,
            height: 65535,
        };
        // Without the clamp this would attempt a ~17 GB allocation. With the
        // clamp at 2048, allocation is bounded at ~16 MiB.
        let (response, _events) = handle(&mut system, &mut engine, &mut config, &request);
        match response {
            Response::TemplatePreview(_) | Response::Error(_) => {}
            other => panic!("unexpected response variant: {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Symlink TOCTOU — O_NOFOLLOW tests
    // -----------------------------------------------------------------------

    #[test]
    fn import_config_rejects_symlink_to_outside_path() {
        use std::io::Write as _;

        let dir = std::env::temp_dir()
            .join(format!("frgb_test_symlink_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let real_target = dir.join("real_target.json");
        let mut f = std::fs::File::create(&real_target).unwrap();
        f.write_all(br#"{"version":1}"#).unwrap();

        let link = dir.join("link.json");
        std::os::unix::fs::symlink(&real_target, &link).unwrap();

        let path_str = link.to_string_lossy().to_string();
        let mut system = empty_system();
        let mut engine = test_engine();
        let mut config = test_config();

        let request = Request::ImportConfig { path: path_str.clone(), merge: false };
        let (response, _) = handle(&mut system, &mut engine, &mut config, &request);
        match response {
            Response::Error(msg) => {
                assert!(
                    msg.contains("symlink") || msg.contains("ELOOP") || msg.to_lowercase().contains("refusing"),
                    "expected symlink-refusal error, got: {msg}"
                );
            }
            other => panic!("expected Error, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_config_accepts_real_file_in_tmp() {
        use std::io::Write as _;

        let dir = std::env::temp_dir()
            .join(format!("frgb_test_safe_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let path = dir.join("safe.json");
        let mut f = std::fs::File::create(&path).unwrap();
        let cfg_json = serde_json::to_string(&frgb_model::config::Config::default()).unwrap();
        f.write_all(cfg_json.as_bytes()).unwrap();

        let path_str = path.to_string_lossy().to_string();
        let mut system = empty_system();
        let mut engine = test_engine();
        let mut config = test_config();

        let request = Request::ImportConfig { path: path_str.clone(), merge: false };
        let (response, _) = handle(&mut system, &mut engine, &mut config, &request);

        assert!(
            matches!(response, Response::Ok),
            "real file should import successfully, got: {response:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_config_rejects_dotdot_traversal_outside_safe_paths() {
        // /tmp/../etc/passwd canonicalizes to /etc/passwd, which is outside home/tmp.
        let mut system = empty_system();
        let mut engine = test_engine();
        let mut config = test_config();

        let request = Request::ImportConfig {
            path: "/tmp/../etc/passwd".to_string(),
            merge: false,
        };
        let (response, _) = handle(&mut system, &mut engine, &mut config, &request);
        match response {
            Response::Error(msg) => {
                assert!(
                    msg.contains("home directory") || msg.contains("/tmp") || msg.contains("path must be"),
                    "expected path-validation error, got: {msg}"
                );
            }
            other => panic!("expected Error for ..-traversal, got {other:?}"),
        }
    }

    #[test]
    fn export_config_rejects_symlink_at_target() {
        use std::io::Write as _;

        let dir = std::env::temp_dir()
            .join(format!("frgb_test_export_symlink_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let real = dir.join("real.json");
        let mut f = std::fs::File::create(&real).unwrap();
        f.write_all(b"{}").unwrap();
        drop(f);

        let link = dir.join("export_link.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let path_str = link.to_string_lossy().to_string();
        let mut system = empty_system();
        let mut engine = test_engine();
        let mut config = test_config();

        let request = Request::ExportConfig { path: path_str.clone(), compress: false };
        let (response, _) = handle(&mut system, &mut engine, &mut config, &request);
        match response {
            Response::Error(msg) => {
                assert!(
                    msg.to_lowercase().contains("refusing") || msg.contains("symlink"),
                    "expected symlink-refusal error on export, got: {msg}"
                );
            }
            Response::Ok => panic!("export should have refused to follow the symlink"),
            other => panic!("unexpected response: {other:?}"),
        }

        // Verify real target was NOT modified (i.e., still contains "{}")
        let after = std::fs::read_to_string(&real).unwrap_or_default();
        assert_eq!(after, "{}", "real target should not have been written through the symlink");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // -----------------------------------------------------------------------
    // AURA-filter regression harness
    //
    // Every daemon entry point that calls set_speed and is reachable for AURA
    // groups must be exercised here. Assert: no SpeedChanged event for AURA,
    // no DeviceState.speed_percent mutation on AURA.
    //
    // New path that touches speed? Add a test here. If you think your path
    // is exempt, add it anyway — the test will pass cheaply.
    // -----------------------------------------------------------------------

    /// Stub backend whose set_speed and send_rgb return Ok — needed for the
    /// AURA-filter harness to make registry-state assertions load-bearing.
    /// Without this, set_speed errors at backend_for() and never mutates state,
    /// making the tests pass regardless of whether the AURA guard is in place.
    struct OkBackend;
    impl frgb_core::backend::Backend for OkBackend {
        fn id(&self) -> BackendId {
            BackendId(0)
        }
        fn name(&self) -> &str {
            "ok-mock"
        }
        fn discover(&mut self) -> frgb_core::error::Result<Vec<frgb_core::DiscoveredDevice>> {
            Ok(Vec::new())
        }
        fn set_speed(
            &self,
            _device: &frgb_core::registry::Device,
            _cmd: &frgb_core::backend::SpeedCommand,
        ) -> frgb_core::error::Result<()> {
            Ok(())
        }
        fn send_rgb(
            &self,
            _device: &frgb_core::registry::Device,
            _buf: &frgb_rgb::generator::EffectResult,
        ) -> frgb_core::error::Result<()> {
            Ok(())
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    /// Build a System with one RF SL fan at group 1 AND one AURA group at group 5.
    /// AURA is injected via the synthetic `dev_type = 0xFD` (see
    /// `frgb_core::registry::identify_device_type`).
    ///
    /// OkBackend is registered at BackendId(0) so that set_speed calls for the RF
    /// group actually reach registry.update_state — making speed_percent assertions
    /// load-bearing. Without this, backend_for() returns Err(NotFound) before any
    /// state mutation occurs, and the AURA guard tests pass vacuously.
    fn system_with_rf_and_aura() -> System {
        let specs = load_defaults();
        let mut system = System::new(specs);
        system.add_backend(Box::new(OkBackend));
        let our_mac = DeviceId::from([0x29, 0x7a, 0x84, 0xe5, 0x66, 0xe4]);
        system.registry.refresh(
            BackendId(0),
            vec![
                // RF SL fan at group 1.
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
                // AURA group 5 — synthetic dev_type=0xFD.
                frgb_core::DiscoveredDevice {
                    id: DeviceId::from([0xff, 0xff, 0xff, 0xff, 0xff, 0xfd]),
                    fans_type: [0, 0, 0, 0],
                    dev_type: 0xFD,
                    group: GroupId::new(5),
                    fan_count: 0,
                    master: our_mac,
                    fans_rpm: [0; 4],
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

    fn aura_group() -> GroupId {
        GroupId::new(5)
    }

    fn rf_group() -> GroupId {
        GroupId::new(1)
    }

    fn assert_aura_untouched(system: &System) {
        let dev = system.find_group(aura_group()).expect("aura device exists");
        assert!(
            dev.state.speed_percent.is_none(),
            "AURA group must not have speed_percent set; got {:?}",
            dev.state.speed_percent
        );
    }

    fn assert_no_aura_speed_event(events: &[Event]) {
        for ev in events {
            if let Event::SpeedChanged { group, .. } = ev {
                assert_ne!(*group, aura_group(), "no SpeedChanged event should fire for AURA group");
            }
        }
    }

    #[test]
    fn aura_filter_stop_fans_skips_aura() {
        let mut system = system_with_rf_and_aura();
        let mut engine = test_engine();
        let mut config = test_config();
        let (response, events) = handle(&mut system, &mut engine, &mut config, &Request::StopFans);
        // Dispatch must succeed — if it errors at the backend level the guard
        // assertion below would pass vacuously (state was never mutated).
        assert!(matches!(response, Response::Ok), "expected Ok, got {:?}", response);
        assert_no_aura_speed_event(&events);
        assert_aura_untouched(&system);
        // Positive check: RF group must have speed_percent set, proving OkBackend
        // was reached and registry.update_state ran.
        let rf_dev = system.find_group(rf_group()).expect("rf device exists");
        assert_eq!(
            rf_dev.state.speed_percent.map(|p| p.value()),
            Some(0),
            "RF group must have speed_percent=0 after StopFans"
        );
    }

    #[test]
    fn aura_filter_set_speed_all_skips_aura() {
        let mut system = system_with_rf_and_aura();
        let mut engine = test_engine();
        let mut config = test_config();
        let req = Request::SetSpeedAll {
            target: Target::All,
            mode: SpeedMode::Manual(SpeedPercent::new(50)),
        };
        let (response, events) = handle(&mut system, &mut engine, &mut config, &req);
        // Dispatch must succeed — a backend error would make AURA assertions vacuous.
        assert!(matches!(response, Response::Ok), "expected Ok, got {:?}", response);
        assert_no_aura_speed_event(&events);
        assert_aura_untouched(&system);
        // Positive check: RF group must have speed_percent set, proving OkBackend
        // was reached and registry.update_state ran.
        let rf_dev = system.find_group(rf_group()).expect("rf device exists");
        assert_eq!(
            rf_dev.state.speed_percent.map(|p| p.value()),
            Some(50),
            "RF group must have speed_percent=50 after SetSpeedAll"
        );
    }

    #[test]
    fn aura_filter_switch_profile_skips_aura_speed() {
        use frgb_model::config::{GroupSnapshot, Profile, Scene};
        use frgb_model::rgb::{Rgb, RgbMode, Ring};
        use frgb_model::Brightness;

        let mut system = system_with_rf_and_aura();
        let mut engine = test_engine();
        let mut config = test_config();

        let name = ValidatedName::new("aura_test").unwrap();
        let profile = Profile {
            name: name.clone(),
            groups: vec![
                GroupSnapshot {
                    group_id: rf_group(),
                    scene: Scene {
                        speed: Some(SpeedMode::Manual(SpeedPercent::new(50))),
                        rgb: RgbMode::Static {
                            ring: Ring::Both,
                            color: Rgb { r: 200, g: 0, b: 0 },
                            brightness: Brightness::new(255),
                        },
                        lcd: None,
                    },
                },
                GroupSnapshot {
                    group_id: aura_group(),
                    scene: Scene {
                        speed: Some(SpeedMode::Manual(SpeedPercent::new(75))),
                        rgb: RgbMode::Static {
                            ring: Ring::Both,
                            color: Rgb { r: 0, g: 200, b: 0 },
                            brightness: Brightness::new(255),
                        },
                        lcd: None,
                    },
                },
            ],
            effect_cycle: None,
            sequences: vec![],
        };
        config.config_mut().upsert_profile(profile);

        let req = Request::SwitchProfile { name: name.to_string() };
        let (response, events) = handle(&mut system, &mut engine, &mut config, &req);
        assert!(matches!(response, Response::Ok), "expected Ok, got {:?}", response);
        assert_no_aura_speed_event(&events);
        assert_aura_untouched(&system);
        // Positive check: RF group must have speed_percent set via OkBackend.
        let rf_dev = system.find_group(rf_group()).expect("rf device exists");
        assert_eq!(
            rf_dev.state.speed_percent.map(|p| p.value()),
            Some(50),
            "RF group must have speed_percent=50 after SwitchProfile"
        );
    }

    #[test]
    fn aura_filter_apply_profile_groups_helper_skips_aura() {
        use frgb_model::config::{GroupSnapshot, Scene};
        use frgb_model::rgb::{Rgb, RgbMode, Ring};
        use frgb_model::Brightness;

        let mut system = system_with_rf_and_aura();
        let snaps = vec![
            GroupSnapshot {
                group_id: rf_group(),
                scene: Scene {
                    speed: Some(SpeedMode::Manual(SpeedPercent::new(50))),
                    rgb: RgbMode::Static {
                        ring: Ring::Both,
                        color: Rgb { r: 200, g: 0, b: 0 },
                        brightness: Brightness::new(255),
                    },
                    lcd: None,
                },
            },
            GroupSnapshot {
                group_id: aura_group(),
                scene: Scene {
                    speed: Some(SpeedMode::Manual(SpeedPercent::new(75))),
                    rgb: RgbMode::Static {
                        ring: Ring::Both,
                        color: Rgb { r: 0, g: 200, b: 0 },
                        brightness: Brightness::new(255),
                    },
                    lcd: None,
                },
            },
        ];
        crate::engine::apply_profile_groups(&mut system, &snaps);
        assert_aura_untouched(&system);
        // Positive check: RF group must have speed_percent set via OkBackend.
        let rf_dev = system.find_group(rf_group()).expect("rf device exists");
        assert_eq!(
            rf_dev.state.speed_percent.map(|p| p.value()),
            Some(50),
            "RF group must have speed_percent=50 after apply_profile_groups"
        );
    }
}
