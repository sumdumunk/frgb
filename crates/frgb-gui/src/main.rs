mod bridge;
mod convert;
mod debounce;
pub mod fan_render;
mod pages;
mod sensor_history;
mod state;

mod lcd_convert;
mod rgb_convert;

slint::include_modules!();

use std::sync::{Arc, Mutex};

use frgb_model::GroupId;

/// Daemon startup result — Ok(()) on success, Err(message) with user-facing reason.
type DaemonResult = Result<(), String>;

/// Try to start the daemon as a background process.
/// Looks for `frgbd` next to the GUI binary, then in PATH.
fn try_start_daemon() -> DaemonResult {
    if frgb_ipc::daemon_running() {
        tracing::info!("daemon already running");
        return Ok(());
    }

    let daemon_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("frgbd")))
        .filter(|p| p.exists());

    let cmd = if let Some(ref path) = daemon_path {
        tracing::info!("found daemon at {}", path.display());
        path.clone()
    } else {
        tracing::info!("daemon not found next to GUI, trying PATH");
        std::path::PathBuf::from("frgbd")
    };

    match std::process::Command::new(&cmd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(mut child) => {
            let pid = child.id();
            tracing::info!("spawned daemon pid={pid} cmd={}", cmd.display());

            for _ in 0..30 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if frgb_ipc::daemon_running() {
                    // Drain stderr in a background thread to prevent pipe buffer
                    // from filling and blocking the daemon's log writes.
                    let stderr_pipe = child.stderr.take();
                    std::thread::Builder::new()
                        .name("daemon-reaper".into())
                        .spawn(move || {
                            if let Some(mut pipe) = stderr_pipe {
                                use std::io::Read;
                                let mut buf = [0u8; 4096];
                                while pipe.read(&mut buf).unwrap_or(0) > 0 {}
                            }
                            let _ = child.wait();
                        })
                        .ok();
                    return Ok(());
                }
                // Check if child exited early
                if let Ok(Some(status)) = child.try_wait() {
                    let stderr = child
                        .stderr
                        .take()
                        .and_then(|mut s| {
                            use std::io::Read;
                            let mut buf = String::new();
                            s.read_to_string(&mut buf).ok().map(|_| buf)
                        })
                        .unwrap_or_default();
                    let stderr = stderr.trim().to_string();

                    // Detect common failure modes from stderr
                    let is_permission = stderr.contains("ermission")
                        || stderr.contains("EACCES")
                        || stderr.contains("Operation not permitted")
                        || status.code() == Some(126);
                    let is_busy = stderr.contains("busy") || stderr.contains("EBUSY");

                    let msg = if is_permission {
                        "USB permission denied — install udev rules: sudo cp udev/99-frgb.rules /etc/udev/rules.d/ && sudo udevadm control --reload-rules && sudo udevadm trigger".to_string()
                    } else if is_busy {
                        "USB device busy — kill stale daemon: pkill frgbd".to_string()
                    } else if stderr.is_empty() {
                        format!("Daemon exited with {status}")
                    } else {
                        format!("Daemon failed: {stderr}")
                    };

                    tracing::error!("{msg}");
                    return Err(msg);
                }
            }
            // Still running but no socket
            std::thread::Builder::new()
                .name("daemon-reaper".into())
                .spawn(move || {
                    let _ = child.wait();
                })
                .ok();
            Err("Daemon started but not responding — check logs".into())
        }
        Err(e) => {
            let msg = if e.kind() == std::io::ErrorKind::NotFound {
                "Daemon binary 'frgbd' not found — build with: cargo build -p frgb-daemon".into()
            } else {
                format!("Failed to start daemon: {e}")
            };
            tracing::warn!("{msg}");
            Err(msg)
        }
    }
}

/// Fetch profiles, firmware info, daemon config, and curves from the daemon.
/// Called on initial connect and on every reconnect.
fn fetch_supplementary(
    window: &AppWindow,
    bridge: &bridge::BridgeHandle,
    lcd_presets: &std::sync::Arc<std::sync::Mutex<Vec<frgb_model::lcd::LcdPreset>>>,
    alert_thresholds: &pages::sensors::AlertThresholds,
) {
    // Profiles
    {
        let w = window.as_weak();
        bridge.call(frgb_ipc::Request::ListProfiles, move |resp| {
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
    }

    // Firmware info
    {
        let w = window.as_weak();
        bridge.call(frgb_model::ipc::Request::GetFirmwareInfo, move |resp| {
            if let frgb_model::ipc::Response::FirmwareInfo(info) = resp {
                slint::invoke_from_event_loop(move || {
                    if let Some(window) = w.upgrade() {
                        window.set_tx_firmware(info.tx_version.into());
                        // I9: rx_version is the TX dongle MAC, correctly labeled
                        window.set_tx_mac(info.rx_version.into());
                    }
                })
                .ok();
            }
        });
    }

    // Daemon config
    {
        let w = window.as_weak();
        bridge.call(frgb_model::ipc::Request::GetDaemonConfig, move |resp| {
            if let frgb_model::ipc::Response::DaemonConfig(cfg) = resp {
                slint::invoke_from_event_loop(move || {
                    if let Some(window) = w.upgrade() {
                        window.set_poll_interval_ms(cfg.poll_interval_ms as i32);
                    }
                })
                .ok();
            }
        });
    }

    // Alert config — populate toggle initial state
    {
        let w = window.as_weak();
        let thresh = alert_thresholds.clone();
        bridge.call(frgb_model::ipc::Request::GetAlertConfig, move |resp| {
            if let frgb_model::ipc::Response::AlertConfig(cfg) = resp {
                // Populate alert thresholds for sensor dashboard
                {
                    let mut map = thresh.lock().unwrap();
                    map.clear();
                    for alert in &cfg.temp_alerts {
                        let label = crate::convert::sensor_label(&alert.sensor);
                        map.insert(label, alert.threshold as f32);
                    }
                }
                slint::invoke_from_event_loop(move || {
                    if let Some(window) = w.upgrade() {
                        window.set_fan_stall_detect(cfg.fan_stall_detect);
                        window.set_disconnect_alert(cfg.device_disconnect);
                    }
                })
                .ok();
            }
        });
    }

    // Curves
    pages::curves::fetch_curves(window, bridge);

    // LCD devices and presets
    pages::lcd::fetch_lcd_devices(window, bridge);
    pages::lcd::fetch_presets(window, bridge, lcd_presets);

    // Sequences
    pages::sequence::fetch_sequences(window, bridge);

    // Schedules
    pages::settings::fetch_schedules(window, bridge);

    // Sync config
    pages::settings::fetch_sync_config(window, bridge);

    // Sensor names (dynamic — populated from daemon discovery)
    {
        let w = window.as_weak();
        bridge.call(frgb_ipc::Request::ListSensors, move |resp| {
            if let frgb_ipc::Response::SensorList(sensors) = resp {
                let names: Vec<slint::SharedString> = sensors
                    .iter()
                    .map(|si| slint::SharedString::from(crate::convert::sensor_label(&si.sensor)))
                    .collect();
                // Always include base sensors, then append detected ones
                let base = ["CPU", "GPU", "Water"];
                let mut unique: Vec<slint::SharedString> = base.iter().map(|s| slint::SharedString::from(*s)).collect();
                for n in names {
                    if !unique.contains(&n) {
                        unique.push(n);
                    }
                }
                slint::invoke_from_event_loop(move || {
                    if let Some(window) = w.upgrade() {
                        window.set_sensor_names(slint::ModelRc::new(slint::VecModel::from(unique)));
                    }
                })
                .ok();
            }
        });
    }
}

/// Query primary monitor resolution via xdpyinfo (X11) or xrandr.
fn screen_resolution() -> Option<(i32, i32)> {
    // Try xdpyinfo first (fast, works on X11 and XWayland)
    if let Ok(out) = std::process::Command::new("xdpyinfo")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with("dimensions:") {
                // "dimensions:    3440x1440 pixels ..."
                if let Some(dims) = line.split_whitespace().nth(1) {
                    if let Some((w, h)) = dims.split_once('x') {
                        if let (Ok(w), Ok(h)) = (w.parse::<i32>(), h.parse::<i32>()) {
                            return Some((w, h));
                        }
                    }
                }
            }
        }
    }
    // Fallback: xrandr --current
    if let Ok(out) = std::process::Command::new("xrandr")
        .arg("--current")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            // "3440x1440+0+0" on the connected primary line
            if line.contains(" connected") && line.contains(" primary") {
                for word in line.split_whitespace() {
                    if let Some((res, _offsets)) = word.split_once('+') {
                        if let Some((w, h)) = res.split_once('x') {
                            if let (Ok(w), Ok(h)) = (w.parse::<i32>(), h.parse::<i32>()) {
                                return Some((w, h));
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Auto-start daemon if not running
    let startup_error = if !frgb_ipc::daemon_running() {
        tracing::info!("daemon not running, attempting to start...");
        try_start_daemon().err()
    } else {
        None
    };

    let window = AppWindow::new().expect("failed to create window");

    // Show daemon startup error in the status bar so the user sees it
    if let Some(ref msg) = startup_error {
        window
            .global::<UiState>()
            .set_daemon_status(slint::SharedString::from(msg.as_str()));
    }

    let history = sensor_history::SensorHistory::new();
    let lcd_presets: pages::lcd::PresetList = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let rgb_states = pages::rgb::RgbStateMap::new();
    let alert_thresholds: pages::sensors::AlertThresholds =
        std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    let led_states: pages::led::LedState = std::sync::Arc::new(std::sync::Mutex::new(Default::default()));

    // Deferred bridge handle — set after spawn, used by on_connected callback.
    let deferred_bridge: Arc<Mutex<Option<bridge::BridgeHandle>>> = Arc::new(Mutex::new(None));

    // Spawn IPC bridge with event callbacks
    let bridge = bridge::spawn(bridge::BridgeCallbacks {
        on_event: {
            let w = window.as_weak();
            let h = history.clone();
            let deferred = deferred_bridge.clone();
            let rgb_st = rgb_states.clone();
            let led_st = led_states.clone();
            Box::new(move |event| {
                if let frgb_ipc::Event::SensorUpdate { ref sensor, value } = event {
                    h.record(&crate::convert::sensor_label(sensor), value);
                }
                // Sync RGB state map from daemon events
                if let frgb_ipc::Event::RgbChanged { group, ref mode } = event {
                    pages::rgb::sync_from_event(&rgb_st, group, mode);
                }
                let needs_lcd_refresh = matches!(&event, frgb_ipc::Event::DeviceConnected { .. });
                let w = w.clone();
                let deferred = deferred.clone();
                let led_st = led_st.clone();
                slint::invoke_from_event_loop(move || {
                    if let Some(window) = w.upgrade() {
                        // Sync LED state from RgbChanged (needs UI thread for device lookup)
                        if let frgb_ipc::Event::RgbChanged { group, ref mode } = event {
                            use slint::Model;
                            let model = window.get_devices();
                            if let Some(g) = (0..model.row_count())
                                .find_map(|i| model.row_data(i).filter(|g| g.group_id == group.value() as i32))
                            {
                                let dt = crate::convert::device_type_from_display(&g.device_type);
                                pages::led::sync_from_event(&led_st, group, mode, dt, g.fan_count as u8);
                                pages::led::render_and_push(&window, &led_st, group);
                            }
                        }
                        state::apply_event(&window, &event);
                        if needs_lcd_refresh {
                            if let Some(bridge) = deferred.lock().ok().and_then(|g| g.clone()) {
                                pages::lcd::fetch_lcd_devices(&window, &bridge);
                            }
                        }
                    }
                })
                .ok();
            })
        },
        on_connected: {
            let w = window.as_weak();
            let deferred = deferred_bridge.clone();
            let presets = lcd_presets.clone();
            let thresh = alert_thresholds.clone();
            Box::new(move |connected| {
                let w = w.clone();
                let deferred = deferred.clone();
                let presets = presets.clone();
                let thresh = thresh.clone();
                slint::invoke_from_event_loop(move || {
                    if let Some(window) = w.upgrade() {
                        state::set_connected(&window, connected);
                        if connected {
                            if let Some(bridge) = deferred.lock().ok().and_then(|g| g.clone()) {
                                fetch_supplementary(&window, &bridge, &presets, &thresh);
                            }
                        }
                    }
                })
                .ok();
            })
        },
        on_initial_state: {
            let w = window.as_weak();
            let rgb_st = rgb_states.clone();
            let led_st = led_states.clone();
            Box::new(move |resp| {
                let w = w.clone();
                let rgb_st = rgb_st.clone();
                let led_st = led_st.clone();
                slint::invoke_from_event_loop(move || {
                    if let Some(window) = w.upgrade() {
                        if let frgb_ipc::Response::DeviceStatus(ref groups) = resp {
                            pages::rgb::sync_from_status(&rgb_st, groups);
                            pages::led::sync_from_status(&led_st, groups);
                            state::apply_device_status(&window, groups);
                            pages::rgb::load_selected_group(&window, &rgb_st);
                        } else {
                            tracing::warn!("initial state: expected DeviceStatus, got {:?}", resp);
                        }
                    }
                })
                .ok();
            })
        },
    });

    // Store bridge handle so on_connected callback can use it
    *deferred_bridge.lock().unwrap() = Some(bridge.clone());

    // Populate static data (these don't need the daemon)
    {
        let effect_names: Vec<slint::SharedString> = rgb_convert::effect_display_names()
            .into_iter()
            .map(slint::SharedString::from)
            .collect();
        window.set_effect_names(slint::ModelRc::new(slint::VecModel::from(effect_names)));
    }
    // Default sensor names (replaced by dynamic list from daemon on connect)
    {
        let sensor_names: Vec<slint::SharedString> = ["CPU", "GPU", "Water"]
            .iter()
            .map(|s| slint::SharedString::from(*s))
            .collect();
        window.set_sensor_names(slint::ModelRc::new(slint::VecModel::from(sensor_names)));
    }

    // Wire core callbacks (previously in pages::devices, now inline — these are
    // app-level operations not specific to any tab/page)
    {
        let bridge_c = bridge.clone();
        let w = window.as_weak();
        window.on_refresh(move || {
            let w = w.clone();
            bridge_c.call(frgb_ipc::Request::Status, move |resp| {
                let w = w.clone();
                slint::invoke_from_event_loop(move || {
                    if let Some(window) = w.upgrade() {
                        if let frgb_ipc::Response::DeviceStatus(ref groups) = resp {
                            state::apply_device_status(&window, groups);
                        }
                    }
                })
                .ok();
            });
        });
    }
    {
        let bridge_c = bridge.clone();
        window.on_indicate(move |group_id| {
            bridge_c.send(frgb_ipc::Request::Indicate {
                group: GroupId::new(group_id as u8),
                duration_secs: 3,
            });
        });
    }
    {
        let bridge_c = bridge.clone();
        let w = window.as_weak();
        window.on_set_group_role(move |group_id, role_str| {
            let role = match role_str.as_str() {
                "Intake" => frgb_model::device::FanRole::Intake,
                "Exhaust" => frgb_model::device::FanRole::Exhaust,
                "Pump" => frgb_model::device::FanRole::Pump,
                other => {
                    tracing::warn!("set_group_role: unknown role '{other}'");
                    return;
                }
            };
            let w = w.clone();
            let b = bridge_c.clone();
            bridge_c.call(
                frgb_ipc::Request::SetRole {
                    group: GroupId::new(group_id as u8),
                    role,
                },
                move |resp| {
                    if matches!(resp, frgb_ipc::Response::Ok) {
                        // Status re-fetch reflects the new role in UI state.
                        let w = w.clone();
                        b.call(frgb_ipc::Request::Status, move |resp| {
                            slint::invoke_from_event_loop(move || {
                                if let Some(window) = w.upgrade() {
                                    if let frgb_ipc::Response::DeviceStatus(ref groups) = resp {
                                        state::apply_device_status(&window, groups);
                                    }
                                }
                            })
                            .ok();
                        });
                    } else {
                        tracing::warn!("set_group_role: daemon returned {resp:?}");
                    }
                },
            );
        });
    }
    {
        let bridge_c = bridge.clone();
        let w = window.as_weak();
        window.on_rename_group(move |group_id, new_name| {
            let name = new_name.to_string();
            if name.is_empty() {
                return;
            }
            let w = w.clone();
            let b = bridge_c.clone();
            bridge_c.call(
                frgb_ipc::Request::RenameGroup {
                    group: GroupId::new(group_id as u8),
                    name,
                },
                move |resp| {
                    if matches!(resp, frgb_ipc::Response::Ok) {
                        let w = w.clone();
                        b.call(frgb_ipc::Request::Status, move |resp| {
                            slint::invoke_from_event_loop(move || {
                                if let Some(window) = w.upgrade() {
                                    if let frgb_ipc::Response::DeviceStatus(ref groups) = resp {
                                        state::apply_device_status(&window, groups);
                                    }
                                }
                            })
                            .ok();
                        });
                    }
                },
            );
        });
    }

    // Wire page callbacks
    pages::speed::wire(&window, &bridge);
    pages::rgb::wire(&window, &bridge, &rgb_states);
    pages::settings::wire(&window, &bridge);
    pages::curves::wire(&window, &bridge);
    pages::lcd::wire(&window, &bridge, &lcd_presets);
    pages::led::wire(&window, &bridge, &led_states);
    pages::led::wire_presets(&window, &bridge, &led_states);
    pages::led::fetch_led_presets(&window, &bridge);
    pages::sequence::wire(&window, &bridge);
    pages::show_runner_ui::wire(&window, &bridge);

    {
        let w = window.as_weak();
        let rgb_st = rgb_states.clone();
        let led_st = led_states.clone();
        window.on_on_group_changed(move |group_index| {
            if let Some(window) = w.upgrade() {
                pages::rgb::load_selected_group(&window, &rgb_st);
                // Render fan visuals for the newly selected group
                if group_index >= 0 {
                    use slint::Model;
                    let model = window.get_devices();
                    if let Some(g) = model.row_data(group_index as usize) {
                        pages::led::render_and_push(&window, &led_st, GroupId::new(g.group_id as u8));
                    }
                }
            }
        });
    }

    // Initial supplementary fetch (will retry automatically on_connected if daemon isn't ready)
    fetch_supplementary(&window, &bridge, &lcd_presets, &alert_thresholds);

    // Periodic sensor graph refresh (1 second)
    let _sensor_timer = {
        let h = history.clone();
        let w = window.as_weak();
        let thresh = alert_thresholds.clone();
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_secs(1),
            move || {
                if let Some(window) = w.upgrade() {
                    let graph_data = pages::sensors::build_sensor_graph_data(&h, &thresh);
                    window.set_sensors(slint::ModelRc::new(slint::VecModel::from(graph_data)));
                }
            },
        );
        timer
    };

    // Center window on primary monitor
    if let Some((sw, sh)) = screen_resolution() {
        let scale = window.window().scale_factor();
        let ww = (1100.0 * scale) as i32;
        let wh = (700.0 * scale) as i32;
        let x = (sw - ww) / 2;
        let y = (sh - wh) / 2;
        window.window().set_position(slint::PhysicalPosition::new(x, y));
    }

    window.run().expect("failed to run event loop");

    // Kill daemon on exit if setting is enabled
    if window.get_kill_daemon_on_exit() {
        tracing::info!("killing daemon on exit");
        let _ = std::process::Command::new("pkill").arg("-f").arg("frgbd").status();
    }
}
