//! Wires the Speed page callbacks.

use std::sync::Arc;
use std::time::Duration;

use slint::ComponentHandle;

use crate::bridge::BridgeHandle;
use crate::debounce::Debouncer;
use crate::AppWindow;
use frgb_model::GroupId;

// Speed preset names — must match the labels in speed-new.slint PresetButton elements.
const PRESET_QUIET: &str = "Quiet";
const PRESET_STANDARD: &str = "Standard";
const PRESET_HIGH: &str = "High";
const PRESET_FULL: &str = "Full";

pub fn wire(window: &AppWindow, bridge: &BridgeHandle) {
    // Speed slider debouncer — collapses rapid drag events into <=10/s IPC calls
    let speed_debouncer = Arc::new(Debouncer::new(Duration::from_millis(100), {
        let bridge = bridge.clone();
        move |(group, pct): (i32, i32)| {
            bridge.send(frgb_ipc::Request::SetSpeed {
                group: GroupId::new(group as u8),
                mode: frgb_model::speed::SpeedMode::Manual(frgb_model::SpeedPercent::new(pct as u8)),
            });
        }
    }));

    // Speed changed — debounced drag updates
    {
        let debouncer = speed_debouncer.clone();
        window.on_speed_changed(move |group, pct| {
            debouncer.update((group, pct));
        });
    }

    // Speed released — ensure the final position is always sent
    {
        let debouncer = speed_debouncer.clone();
        window.on_speed_released(move |group, pct| {
            debouncer.update((group, pct));
            debouncer.flush();
        });
    }

    // Preset clicked — map preset name to speed percentage
    {
        let bridge = bridge.clone();
        window.on_preset_clicked(move |group, name| {
            let mode = match name.as_str() {
                PRESET_QUIET => frgb_model::speed::SpeedMode::Manual(frgb_model::SpeedPercent::new(25)),
                PRESET_STANDARD => frgb_model::speed::SpeedMode::Manual(frgb_model::SpeedPercent::new(50)),
                PRESET_HIGH => frgb_model::speed::SpeedMode::Manual(frgb_model::SpeedPercent::new(75)),
                PRESET_FULL => frgb_model::speed::SpeedMode::Manual(frgb_model::SpeedPercent::new(100)),
                _ => return,
            };
            bridge.send(frgb_ipc::Request::SetSpeed {
                group: GroupId::new(group as u8),
                mode,
            });
        });
    }

    // Preset all — apply speed preset to all groups
    {
        let bridge = bridge.clone();
        window.on_preset_all(move |name| {
            let mode = match name.as_str() {
                PRESET_QUIET => frgb_model::speed::SpeedMode::Manual(frgb_model::SpeedPercent::new(25)),
                PRESET_STANDARD => frgb_model::speed::SpeedMode::Manual(frgb_model::SpeedPercent::new(50)),
                PRESET_HIGH => frgb_model::speed::SpeedMode::Manual(frgb_model::SpeedPercent::new(75)),
                PRESET_FULL => frgb_model::speed::SpeedMode::Manual(frgb_model::SpeedPercent::new(100)),
                _ => return,
            };
            bridge.send(frgb_ipc::Request::SetSpeedAll {
                target: frgb_ipc::Target::All,
                mode,
            });
        });
    }

    // Mode changed — map mode string to SpeedMode
    {
        let bridge = bridge.clone();
        let w = window.as_weak();
        window.on_mode_changed(move |group, mode_str| {
            let mode = match mode_str.as_str() {
                "Manual" => frgb_model::speed::SpeedMode::Manual(frgb_model::SpeedPercent::new(50)),
                "Curve" => {
                    // Preserve the group's existing curve name if it has one
                    let curve_name = w
                        .upgrade()
                        .and_then(|win: AppWindow| {
                            use slint::Model;
                            let model = win.get_devices();
                            (0..model.row_count())
                                .filter_map(|i| model.row_data(i))
                                .find(|g| g.group_id == group)
                                .and_then(|g| {
                                    let name = g.speed_curve_name.to_string();
                                    if name.is_empty() {
                                        None
                                    } else {
                                        Some(name)
                                    }
                                })
                        })
                        .unwrap_or_else(|| "Default".into());
                    frgb_model::speed::SpeedMode::NamedCurve(curve_name)
                }
                "MB Sync" => frgb_model::speed::SpeedMode::Pwm,
                _ => return,
            };
            bridge.send(frgb_ipc::Request::SetSpeed {
                group: GroupId::new(group as u8),
                mode,
            });
        });
    }
}
