//! Wires the Settings page callbacks.
//!
//! All save/delete operations use bridge.call() (not send) so errors are
//! surfaced to the user via the status bar.

use slint::ComponentHandle;

use crate::bridge::BridgeHandle;
use crate::state;
use crate::AppWindow;
use frgb_model::GroupId;

/// Show an error in the daemon status bar.
fn show_status(w: &slint::Weak<AppWindow>, msg: impl Into<slint::SharedString>) {
    let msg = msg.into();
    let w = w.clone();
    slint::invoke_from_event_loop(move || {
        if let Some(window) = w.upgrade() {
            window.global::<crate::UiState>().set_daemon_status(msg);
        }
    })
    .ok();
}

/// Fetch schedules from daemon and populate the UI list.
pub fn fetch_schedules(window: &AppWindow, bridge: &BridgeHandle) {
    fetch_schedules_inner(&window.as_weak(), bridge);
}

fn fetch_schedules_inner(w: &slint::Weak<AppWindow>, bridge: &BridgeHandle) {
    let w = w.clone();
    bridge.call(frgb_ipc::Request::ListSchedule, move |resp| {
        if let frgb_ipc::Response::ScheduleList(entries) = resp {
            let display: Vec<String> = entries
                .iter()
                .map(|e| {
                    let action_str = match &e.action {
                        frgb_model::config::ScheduleAction::SwitchProfile(name) => {
                            format!("profile:{name}")
                        }
                        frgb_model::config::ScheduleAction::SetSpeed { percent, .. } => {
                            format!("speed:{percent}%")
                        }
                        frgb_model::config::ScheduleAction::ApplyCurve { curve, .. } => {
                            format!("curve:{curve}")
                        }
                    };
                    format!("{:02}:{:02} — {action_str}", e.hour, e.minute)
                })
                .collect();
            let w = w.clone();
            slint::invoke_from_event_loop(move || {
                if let Some(window) = w.upgrade() {
                    let slint_list: Vec<slint::SharedString> = display.iter().map(slint::SharedString::from).collect();
                    window.set_schedule_list(slint::ModelRc::new(slint::VecModel::from(slint_list)));
                }
            })
            .ok();
        }
    });
}

/// Fetch sync config from daemon and populate the UI toggles.
pub fn fetch_sync_config(window: &AppWindow, bridge: &BridgeHandle) {
    let w = window.as_weak();
    bridge.call(frgb_ipc::Request::GetSyncConfig, move |resp| {
        if let frgb_ipc::Response::SyncConfig(cfg) = resp {
            slint::invoke_from_event_loop(move || {
                if let Some(window) = w.upgrade() {
                    window.set_sync_enabled(cfg.enabled);
                    window.set_sync_lianli(cfg.include_lianli);
                    window.set_sync_mobo(cfg.include_mobo_rgb);
                    window.set_sync_openrgb(cfg.include_openrgb);
                    // Role filters: if include_roles is empty, all roles are included
                    window.set_sync_filter_intake(
                        cfg.include_roles.is_empty()
                            || cfg.include_roles.contains(&frgb_model::device::FanRole::Intake),
                    );
                    window.set_sync_filter_exhaust(
                        cfg.include_roles.is_empty()
                            || cfg.include_roles.contains(&frgb_model::device::FanRole::Exhaust),
                    );
                    window.set_sync_filter_pump(
                        cfg.include_roles.is_empty() || cfg.include_roles.contains(&frgb_model::device::FanRole::Pump),
                    );
                    let exclude_str: String = cfg
                        .exclude_groups
                        .iter()
                        .map(|g| g.to_string())
                        .collect::<Vec<_>>()
                        .join(",");
                    window.set_sync_exclude_groups(exclude_str.into());
                }
            })
            .ok();
        }
    });
}

pub fn wire(window: &AppWindow, bridge: &BridgeHandle) {
    // Save settings — calibration + daemon config
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        window.on_save_settings(move |cpu_off, gpu_off, poll_ms, fan_stall, disconnect| {
            let cpu: f32 = cpu_off.to_string().parse().unwrap_or(0.0);
            let gpu: f32 = gpu_off.to_string().parse().unwrap_or(0.0);
            let poll = poll_ms as u32;

            let cal = frgb_model::sensor::SensorCalibration {
                cpu_offset: cpu,
                gpu_offset: gpu,
                custom_paths: std::collections::HashMap::new(),
            };

            // Save calibration
            let w2 = w.clone();
            bridge.call(frgb_model::ipc::Request::SetSensorCalibration { cal }, move |resp| {
                if let frgb_model::ipc::Response::Error(e) = resp {
                    show_status(&w2, format!("Calibration save failed: {e}"));
                }
            });

            // Read-modify-write: fetch current alert config, update booleans, send back.
            let w4 = w.clone();
            let b3 = bridge.clone();
            bridge.call(frgb_model::ipc::Request::GetAlertConfig, move |resp| {
                let mut alert_cfg = match resp {
                    frgb_model::ipc::Response::AlertConfig(cfg) => cfg,
                    _ => frgb_model::config::AlertConfig {
                        temp_alerts: Vec::new(),
                        fan_stall_detect: true,
                        device_disconnect: true,
                    },
                };
                alert_cfg.fan_stall_detect = fan_stall;
                alert_cfg.device_disconnect = disconnect;
                b3.call(
                    frgb_model::ipc::Request::SetAlertConfig { config: alert_cfg },
                    move |resp| {
                        if let frgb_model::ipc::Response::Error(e) = resp {
                            show_status(&w4, format!("Alert config save failed: {e}"));
                        }
                    },
                );
            });

            // Update daemon poll interval
            let w3 = w.clone();
            let b2 = bridge.clone();
            bridge.call(frgb_model::ipc::Request::GetDaemonConfig, move |resp| {
                if let frgb_model::ipc::Response::DaemonConfig(mut cfg) = resp {
                    cfg.poll_interval_ms = poll;
                    let w3 = w3.clone();
                    b2.call(
                        frgb_model::ipc::Request::SetDaemonConfig { config: *cfg },
                        move |resp| match resp {
                            frgb_model::ipc::Response::Ok => {
                                show_status(&w3, "Settings saved");
                            }
                            frgb_model::ipc::Response::Error(e) => {
                                show_status(&w3, format!("Config save failed: {e}"));
                            }
                            _ => {}
                        },
                    );
                }
            });
        });
    }

    // Switch profile
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        window.on_switch_profile(move |name| {
            let w = w.clone();
            let name_str = name.to_string();
            let bridge_ref = bridge.clone();
            bridge.call(
                frgb_ipc::Request::SwitchProfile { name: name_str.clone() },
                move |resp| {
                    if matches!(resp, frgb_ipc::Response::Ok) {
                        bridge_ref.call(frgb_ipc::Request::ListProfiles, move |resp| {
                            let w = w.clone();
                            slint::invoke_from_event_loop(move || {
                                if let Some(window) = w.upgrade() {
                                    if let frgb_ipc::Response::ProfileList(ref profiles) = resp {
                                        state::apply_profile_list(&window, profiles, &name_str);
                                    }
                                }
                            })
                            .ok();
                        });
                    } else if let frgb_ipc::Response::Error(e) = resp {
                        show_status(&w, format!("Switch failed: {e}"));
                    }
                },
            );
        });
    }

    // Delete profile
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        window.on_delete_profile(move |name| {
            let w = w.clone();
            let bridge_ref = bridge.clone();
            bridge.call(
                frgb_ipc::Request::DeleteProfile { name: name.to_string() },
                move |resp| {
                    if matches!(resp, frgb_ipc::Response::Ok) {
                        bridge_ref.call(frgb_ipc::Request::ListProfiles, move |resp| {
                            let w = w.clone();
                            slint::invoke_from_event_loop(move || {
                                if let Some(window) = w.upgrade() {
                                    if let frgb_ipc::Response::ProfileList(ref profiles) = resp {
                                        state::apply_profile_list(&window, profiles, "");
                                    }
                                }
                            })
                            .ok();
                        });
                    } else if let frgb_ipc::Response::Error(e) = resp {
                        show_status(&w, format!("Delete failed: {e}"));
                    }
                },
            );
        });
    }

    // --- Sync config ---
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        window.on_save_sync(move |enabled, lianli, mobo, openrgb| {
            // Read filter values from window properties
            let (roles, exclude_groups) = if let Some(w_ref) = w.upgrade() {
                let mut roles = Vec::new();
                if w_ref.get_sync_filter_intake() {
                    roles.push(frgb_model::device::FanRole::Intake);
                }
                if w_ref.get_sync_filter_exhaust() {
                    roles.push(frgb_model::device::FanRole::Exhaust);
                }
                if w_ref.get_sync_filter_pump() {
                    roles.push(frgb_model::device::FanRole::Pump);
                }
                let exclude_text = w_ref.get_sync_exclude_groups().to_string();
                let exclude_groups: Vec<GroupId> = exclude_text
                    .split(',')
                    .filter_map(|s| s.trim().parse::<u8>().ok().map(GroupId::new))
                    .collect();
                (roles, exclude_groups)
            } else {
                (Vec::new(), Vec::new())
            };

            let config = frgb_model::config::SyncConfig {
                enabled,
                include_lianli: lianli,
                include_mobo_rgb: mobo,
                include_openrgb: openrgb,
                include_roles: roles,
                include_device_types: Vec::new(),
                exclude_groups,
            };
            let w2 = w.clone();
            bridge.call(frgb_ipc::Request::SetSyncConfig { config }, move |resp| match resp {
                frgb_ipc::Response::Ok => {
                    show_status(&w2, "Sync config saved");
                }
                frgb_ipc::Response::Error(e) => {
                    show_status(&w2, format!("Sync save failed: {e}"));
                }
                _ => {}
            });
        });
    }

    // --- Schedule management ---

    // Add schedule
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        window.on_add_schedule(move |hour, minute, _action_type, action_value| {
            let action_str = action_value.to_string();
            let action = if let Some(name) = action_str.strip_prefix("profile:") {
                frgb_model::config::ScheduleAction::SwitchProfile(name.to_string())
            } else if let Some(pct) = action_str.strip_prefix("speed:") {
                let percent = frgb_model::SpeedPercent::new(pct.parse::<u8>().unwrap_or(50));
                frgb_model::config::ScheduleAction::SetSpeed {
                    target: frgb_model::ipc::Target::All,
                    percent,
                }
            } else {
                // Default: treat as profile name
                frgb_model::config::ScheduleAction::SwitchProfile(action_str)
            };
            let entry = frgb_model::config::ScheduleEntry {
                hour: hour.clamp(0, 23) as u8,
                minute: minute.clamp(0, 59) as u8,
                days: vec![
                    frgb_model::config::Weekday::Mon,
                    frgb_model::config::Weekday::Tue,
                    frgb_model::config::Weekday::Wed,
                    frgb_model::config::Weekday::Thu,
                    frgb_model::config::Weekday::Fri,
                    frgb_model::config::Weekday::Sat,
                    frgb_model::config::Weekday::Sun,
                ],
                action,
            };
            let w2 = w.clone();
            let b2 = bridge.clone();
            bridge.call(frgb_ipc::Request::AddSchedule { entry }, move |resp| match resp {
                frgb_ipc::Response::Ok => {
                    fetch_schedules_inner(&w2, &b2);
                }
                frgb_ipc::Response::Error(e) => {
                    show_status(&w2, format!("Add schedule failed: {e}"));
                }
                _ => {}
            });
        });
    }

    // Remove schedule
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        window.on_remove_schedule(move |index| {
            let w2 = w.clone();
            let b2 = bridge.clone();
            bridge.call(
                frgb_ipc::Request::DeleteSchedule { index: index as usize },
                move |resp| {
                    if matches!(resp, frgb_ipc::Response::Ok) {
                        fetch_schedules_inner(&w2, &b2);
                    }
                },
            );
        });
    }

    // Clear schedules
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        window.on_clear_schedules(move || {
            let w2 = w.clone();
            let b2 = bridge.clone();
            bridge.call(frgb_ipc::Request::ClearSchedule, move |resp| {
                if matches!(resp, frgb_ipc::Response::Ok) {
                    fetch_schedules_inner(&w2, &b2);
                }
            });
        });
    }

    // Save profile
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        window.on_save_profile(move |name| {
            let name_str = name.to_string();
            if name_str.is_empty() {
                return;
            }
            let w = w.clone();
            let bridge_ref = bridge.clone();
            let validated_name = match frgb_model::ValidatedName::new(name_str.clone()) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("invalid profile name: {e}");
                    return;
                }
            };
            bridge.call(frgb_ipc::Request::SaveProfile { name: validated_name }, move |resp| {
                if matches!(resp, frgb_ipc::Response::Ok) {
                    bridge_ref.call(frgb_ipc::Request::ListProfiles, move |resp| {
                        slint::invoke_from_event_loop(move || {
                            if let Some(window) = w.upgrade() {
                                if let frgb_ipc::Response::ProfileList(ref profiles) = resp {
                                    state::apply_profile_list(&window, profiles, &name_str);
                                }
                            }
                        })
                        .ok();
                    });
                } else if let frgb_ipc::Response::Error(e) = resp {
                    show_status(&w, format!("Save failed: {e}"));
                }
            });
        });
    }
}
