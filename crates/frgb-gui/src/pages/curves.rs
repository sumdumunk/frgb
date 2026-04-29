//! Wires the Curves page callbacks.

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::bridge::BridgeHandle;
use crate::convert;
use crate::{AppWindow, CurvePointData, NamedCurveData};
use frgb_model::GroupId;
use frgb_model::Temperature;

/// Wire all curve-page callbacks to IPC.
pub fn wire(window: &AppWindow, bridge: &BridgeHandle) {
    // --- save-curve ---
    {
        let w = window.as_weak();
        let bridge = bridge.clone();
        window.on_save_curve(move |name, sensor, interpolation, min_speed| {
            let w = w.clone();
            let name_str = name.to_string();
            if name_str.is_empty() {
                return;
            }

            // Read edit-points from UI
            let points: Vec<frgb_model::speed::CurvePoint> = w
                .upgrade()
                .map(|win| {
                    let model = win.get_edit_points();
                    (0..model.row_count())
                        .filter_map(|i| model.row_data(i))
                        .map(|p| frgb_model::speed::CurvePoint {
                            temp: Temperature::new(p.temp as i32),
                            speed: frgb_model::SpeedPercent::new(p.speed as u8),
                        })
                        .collect()
                })
                .unwrap_or_default();

            let curve = frgb_model::speed::FanCurve {
                points,
                sensor: convert::sensor_from_label(sensor.as_str()),
                interpolation: match interpolation.as_str() {
                    "Step" => frgb_model::speed::Interpolation::Step,
                    _ => frgb_model::speed::Interpolation::Linear,
                },
                min_speed: frgb_model::SpeedPercent::new(min_speed as u8),
                stop_below: None,
                ramp_rate: None,
            };

            // Validate before sending
            if let Err(e) = curve.validate() {
                tracing::warn!("curve validation failed: {e}");
                return;
            }

            let validated_name = match frgb_model::ValidatedName::new(name_str) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("invalid curve name: {e}");
                    return;
                }
            };

            // Use call (not send) so we get error feedback; re-fetch on success
            let bridge2 = bridge.clone();
            bridge.call(
                frgb_ipc::Request::SaveCurve {
                    name: validated_name,
                    curve,
                },
                move |resp| {
                    match resp {
                        frgb_ipc::Response::Ok => {
                            // Re-fetch curve list on success
                            bridge2.call(frgb_ipc::Request::ListCurves, move |resp| {
                                if let frgb_ipc::Response::CurveList(curves) = resp {
                                    let w = w.clone();
                                    slint::invoke_from_event_loop(move || {
                                        if let Some(window) = w.upgrade() {
                                            apply_curve_list(&window, &curves);
                                        }
                                    })
                                    .ok();
                                }
                            });
                        }
                        frgb_ipc::Response::Error(e) => {
                            tracing::warn!("save curve failed: {e}");
                            let w = w.clone();
                            slint::invoke_from_event_loop(move || {
                                if let Some(window) = w.upgrade() {
                                    window
                                        .global::<crate::UiState>()
                                        .set_daemon_status(slint::SharedString::from(format!("Save failed: {e}")));
                                }
                            })
                            .ok();
                        }
                        _ => {}
                    }
                },
            );
        });
    }

    // --- delete-curve ---
    {
        let w = window.as_weak();
        let bridge = bridge.clone();
        window.on_delete_curve(move |name| {
            let w = w.clone();
            let bridge2 = bridge.clone();
            bridge.call(frgb_ipc::Request::DeleteCurve { name: name.to_string() }, move |resp| {
                if matches!(resp, frgb_ipc::Response::Ok) {
                    // Re-fetch curve list and reset selection
                    bridge2.call(frgb_ipc::Request::ListCurves, move |resp| {
                        if let frgb_ipc::Response::CurveList(curves) = resp {
                            let w = w.clone();
                            slint::invoke_from_event_loop(move || {
                                if let Some(window) = w.upgrade() {
                                    apply_curve_list(&window, &curves);
                                    // Reset selection to avoid stale index (I4)
                                    window.set_selected_curve(-1);
                                    window.set_edit_name(SharedString::default());
                                }
                            })
                            .ok();
                        }
                    });
                }
            });
        });
    }

    // --- apply-curve ---
    {
        let bridge = bridge.clone();
        window.on_apply_curve(move |group_id, curve_name| {
            bridge.send(frgb_ipc::Request::SetSpeed {
                group: GroupId::new(group_id as u8),
                mode: frgb_model::speed::SpeedMode::NamedCurve(curve_name.to_string()),
            });
        });
    }

    // --- add-point ---
    {
        let w = window.as_weak();
        window.on_add_point(move || {
            let Some(window) = w.upgrade() else { return };
            let model = window.get_edit_points();
            let mut pts: Vec<CurvePointData> = (0..model.row_count()).filter_map(|i| model.row_data(i)).collect();

            // Add a new point with a sensible default
            let new_temp = if let Some(last) = pts.last() {
                (last.temp + 10.0).min(100.0)
            } else {
                30.0
            };
            let new_speed = if let Some(last) = pts.last() {
                (last.speed + 10.0).min(100.0)
            } else {
                25.0
            };
            pts.push(CurvePointData {
                temp: new_temp,
                speed: new_speed,
            });

            // Sort by temperature
            pts.sort_by(|a, b| a.temp.partial_cmp(&b.temp).unwrap_or(std::cmp::Ordering::Equal));

            window.set_edit_points(ModelRc::new(VecModel::from(pts)));
        });
    }

    // --- remove-point ---
    {
        let w = window.as_weak();
        window.on_remove_point(move |index| {
            let Some(window) = w.upgrade() else { return };
            let model = window.get_edit_points();
            let mut pts: Vec<CurvePointData> = (0..model.row_count()).filter_map(|i| model.row_data(i)).collect();

            let idx = index as usize;
            if idx < pts.len() {
                pts.remove(idx);
            }

            window.set_edit_points(ModelRc::new(VecModel::from(pts)));
        });
    }

    // --- update-point-temp ---
    {
        let w = window.as_weak();
        window.on_update_point_temp(move |index, temp| {
            let Some(window) = w.upgrade() else { return };
            let model = window.get_edit_points();
            let mut pts: Vec<CurvePointData> = (0..model.row_count()).filter_map(|i| model.row_data(i)).collect();

            let idx = index as usize;
            if idx < pts.len() {
                pts[idx].temp = temp;
            }

            // Re-sort by temperature
            pts.sort_by(|a, b| a.temp.partial_cmp(&b.temp).unwrap_or(std::cmp::Ordering::Equal));

            window.set_edit_points(ModelRc::new(VecModel::from(pts)));
        });
    }

    // --- update-point-speed ---
    {
        let w = window.as_weak();
        window.on_update_point_speed(move |index, speed| {
            let Some(window) = w.upgrade() else { return };
            let model = window.get_edit_points();
            let idx = index as usize;
            if idx < model.row_count() {
                if let Some(mut pt) = model.row_data(idx) {
                    pt.speed = speed;
                    model.set_row_data(idx, pt);
                }
            }
        });
    }
}

/// Fetch the curve list from the daemon and populate the UI.
pub fn fetch_curves(window: &AppWindow, bridge: &BridgeHandle) {
    let w = window.as_weak();
    bridge.call(frgb_ipc::Request::ListCurves, move |resp| {
        if let frgb_ipc::Response::CurveList(curves) = resp {
            let w = w.clone();
            slint::invoke_from_event_loop(move || {
                if let Some(window) = w.upgrade() {
                    apply_curve_list(&window, &curves);
                }
            })
            .ok();
        }
    });
}

/// Convert model curves to Slint data and set on window.
fn apply_curve_list(window: &AppWindow, curves: &[frgb_model::config::NamedCurve]) {
    let slint_curves: Vec<NamedCurveData> = curves
        .iter()
        .map(|nc| {
            let pts: Vec<CurvePointData> = nc
                .curve
                .points
                .iter()
                .map(|p| CurvePointData {
                    temp: p.temp.celsius() as f32,
                    speed: p.speed.value() as f32,
                })
                .collect();

            NamedCurveData {
                name: SharedString::from(nc.name.as_str()),
                points: ModelRc::new(VecModel::from(pts)),
                sensor: SharedString::from(convert::sensor_label(&nc.curve.sensor)),
                interpolation: SharedString::from(match nc.curve.interpolation {
                    frgb_model::speed::Interpolation::Linear => "Linear",
                    frgb_model::speed::Interpolation::Step => "Step",
                }),
                min_speed: nc.curve.min_speed.value() as i32,
            }
        })
        .collect();

    window.set_curves(ModelRc::new(VecModel::from(slint_curves)));
}
