//! Push IPC data into Slint UI state.
//!
//! In the new architecture, device data lives as an `in property` on AppWindow
//! rather than in globals like DeviceState. Profile data is also on AppWindow.
//! UiState remains a global for selection model, daemon-connected, etc.

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use frgb_model::config::GroupStatus;
use frgb_model::ipc::Event;
use frgb_model::GroupId;

use crate::convert::{
    avg_nonzero_rpm, group_status_to_slint, rgb_mode_display, speed_curve_name, speed_mode_string, speed_percent,
};
use crate::{AppWindow, DeviceGroupData, UiState};

// ---------------------------------------------------------------------------
// Bulk state updates
// ---------------------------------------------------------------------------

/// Push a full device status snapshot into the UI.
pub fn apply_device_status(window: &AppWindow, groups: &[GroupStatus]) {
    let slint_groups: Vec<DeviceGroupData> = groups.iter().map(group_status_to_slint).collect();
    let total_fans: i32 = slint_groups.iter().map(|g| g.fan_count).sum();
    let lcd_count: i32 = slint_groups.iter().map(|g| g.lcd_count).sum();

    let intake_cfm: f32 = slint_groups
        .iter()
        .filter(|g| g.role.as_str() == "Intake")
        .map(|g| g.cfm)
        .sum();
    let exhaust_cfm: f32 = slint_groups
        .iter()
        .filter(|g| g.role.as_str() == "Exhaust")
        .map(|g| g.cfm)
        .sum();

    window.set_devices(ModelRc::new(VecModel::from(slint_groups)));
    window.set_lcd_count(lcd_count);
    window.global::<UiState>().set_total_groups(groups.len() as i32);
    window.global::<UiState>().set_total_fans(total_fans);
    window.global::<UiState>().set_intake_cfm(intake_cfm.round() as i32);
    window.global::<UiState>().set_exhaust_cfm(exhaust_cfm.round() as i32);
}

/// Push profile list into the UI.
pub fn apply_profile_list(window: &AppWindow, profiles: &[String], active: &str) {
    let slint_profiles: Vec<SharedString> = profiles.iter().map(SharedString::from).collect();
    window.set_profiles(ModelRc::new(VecModel::from(slint_profiles)));
    if !active.is_empty() {
        window.set_active_profile(SharedString::from(active));
    }
}

/// Set daemon connection state and status message.
pub fn set_connected(window: &AppWindow, connected: bool) {
    let ui = window.global::<UiState>();
    ui.set_daemon_connected(connected);
    ui.set_daemon_status(SharedString::from(if connected {
        "Connected"
    } else {
        "Daemon not running"
    }));
}

// ---------------------------------------------------------------------------
// Incremental event updates
// ---------------------------------------------------------------------------

/// Apply a single event to the UI state.
pub fn apply_event(window: &AppWindow, event: &Event) {
    match event {
        Event::RpmUpdate { group, rpms } => {
            update_group_field(window, *group, |g| {
                let rpms_i32: Vec<i32> = rpms.iter().map(|&r| r as i32).collect();
                g.avg_rpm = avg_nonzero_rpm(&rpms_i32);
                g.rpms = ModelRc::new(VecModel::from(rpms_i32));
            });
        }
        Event::SpeedChanged { group, mode } => {
            update_group_field(window, *group, |g| {
                g.speed_mode = SharedString::from(speed_mode_string(mode));
                g.speed_percent = speed_percent(mode);
                g.speed_curve_name = SharedString::from(speed_curve_name(mode));
                g.mb_sync = matches!(mode, frgb_model::speed::SpeedMode::Pwm);
            });
        }
        Event::RgbChanged { group, mode } => {
            update_group_field(window, *group, |g| {
                g.rgb_mode = SharedString::from(rgb_mode_display(mode));
            });
        }
        Event::DeviceConnected { group } => {
            update_group_field(window, *group, |g| {
                g.online = true;
            });
        }
        Event::DeviceDisconnected { group } => {
            update_group_field(window, *group, |g| {
                g.online = false;
            });
        }
        Event::SensorUpdate { .. } => {
            // Sensor data flows through SensorHistory → timer → set_sensors().
            // No per-event UI update needed here.
        }
        Event::ProfileSwitched { name } => {
            window.set_active_profile(SharedString::from(name.as_str()));
        }
        Event::FanStall { group, fan } => {
            tracing::warn!("fan stall detected: group={group}, fan={fan}");
            window
                .global::<UiState>()
                .set_daemon_status(SharedString::from(format!("Fan stall: group {group} fan {fan}")));
        }
        Event::Alert(alert) => {
            tracing::warn!(
                "alert: {:?} = {:.1}°C (threshold {}°C)",
                alert.sensor,
                alert.value,
                alert.threshold
            );
            window.global::<UiState>().set_daemon_status(SharedString::from(format!(
                "Alert: {:?} {:.0}°C",
                alert.sensor, alert.value
            )));
        }
        Event::CurveApplied { group, speed, temp: _ } => {
            update_group_field(window, *group, |g| {
                g.speed_percent = speed.value() as i32;
                g.speed_mode = SharedString::from("Curve");
            });
        }
        Event::SequenceStep { group, step_index } => {
            tracing::debug!("sequence step: group={group} step={step_index}");
            window.global::<UiState>().set_daemon_status(SharedString::from(format!(
                "Sequence: group {group} step {}",
                step_index + 1
            )));
        }
        Event::SequenceEnded { name } => {
            tracing::info!("sequence ended: {name}");
            window
                .global::<UiState>()
                .set_daemon_status(SharedString::from(format!("Sequence \"{name}\" ended")));
            window.set_active_show(SharedString::default());
        }
        Event::PowerChanged { on_ac } => {
            tracing::info!("power changed: on_ac={on_ac}");
            window
                .global::<UiState>()
                .set_daemon_status(SharedString::from(if *on_ac { "Power: AC" } else { "Power: Battery" }));
        }
        Event::BindDiscovered(dev) => {
            tracing::info!("bind discovered: {:?} ({} fans)", dev.mac, dev.fan_count);
            window.global::<UiState>().set_daemon_status(SharedString::from(format!(
                "Unbound device found: {} fans",
                dev.fan_count
            )));
        }
        Event::RpmAnomaly { group, message } => {
            tracing::warn!("RPM anomaly: group={group} — {message}");
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Find a group by ID in the devices model and apply a mutation to it.
fn update_group_field(window: &AppWindow, group_id: GroupId, f: impl FnOnce(&mut DeviceGroupData)) {
    let model = window.get_devices();
    for i in 0..model.row_count() {
        if let Some(mut g) = model.row_data(i) {
            if g.group_id == group_id.value() as i32 {
                f(&mut g);
                model.set_row_data(i, g);
                return;
            }
        }
    }
    tracing::warn!("event for unknown group {group_id} — device list may be stale");
}
